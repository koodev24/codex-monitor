import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import tkinter as tk
import urllib.error
import webbrowser
from datetime import datetime
from tkinter import filedialog, messagebox
from typing import Dict, List, Optional, Tuple

import customtkinter as ctk

from .api import AuthRefreshClient, UsageApiClient
from .config import (
    APP_VERSION,
    APP_TITLE,
    AUTH_DIR,
    AUTH_FILE_PATH,
    AUTO_FETCH_INTERVALS,
    AUTO_FETCH_OPTIONS,
    LEGACY_LOCAL_LOG_FILE,
    LOCAL_LOG_FILE,
    UPDATE_CHECK_INTERVAL_SECONDS,
    WINDOW_GEOMETRY,
    WINDOW_MIN_SIZE,
)
from .formatters import format_quota_left, format_reset_display
from .icon_font import (
    MATERIAL_SYMBOLS_FAMILY,
    material_symbol,
    register_material_symbols_font,
)
from .models import AuthFileSnapshot
from .services import AuthFileService, MonitorStateService
from .storage import UsageStorage
from .updater import (
    ReleaseInfo,
    UpdateError,
    fetch_latest_release,
    install_update_and_restart,
    is_newer_version,
    prepare_update,
)
from .watcher import AuthFileWatcher


class ToolTip:
    def __init__(
        self,
        widget: tk.Misc,
        text: str,
        bg_color: str,
        fg_color: str,
        border_color: str,
    ):
        self.widget = widget
        self.text = text
        self.bg_color = bg_color
        self.fg_color = fg_color
        self.border_color = border_color
        self._after_id: Optional[str] = None
        self._window: Optional[tk.Toplevel] = None

        widget.bind("<Enter>", self._schedule, add="+")
        widget.bind("<Leave>", self._hide, add="+")
        widget.bind("<ButtonPress>", self._hide, add="+")

    def _schedule(self, _event: tk.Event) -> None:
        self._cancel()
        self._after_id = self.widget.after(450, self._show)

    def _cancel(self) -> None:
        if self._after_id:
            self.widget.after_cancel(self._after_id)
            self._after_id = None

    def _show(self) -> None:
        self._after_id = None
        if self._window:
            return

        x = self.widget.winfo_rootx() + self.widget.winfo_width() // 2
        y = self.widget.winfo_rooty() + self.widget.winfo_height() + 8
        window = tk.Toplevel(self.widget)
        window.wm_overrideredirect(True)
        window.configure(background=self.border_color)
        label = tk.Label(
            window,
            text=self.text,
            background=self.bg_color,
            foreground=self.fg_color,
            borderwidth=0,
            padx=8,
            pady=4,
            font=("TkDefaultFont", 10),
        )
        label.pack(padx=1, pady=1)
        window.update_idletasks()
        x -= window.winfo_width() // 2
        window.wm_geometry(f"+{x}+{y}")
        self._window = window

    def _hide(self, _event: Optional[tk.Event] = None) -> None:
        self._cancel()
        if self._window:
            self._window.destroy()
            self._window = None


class CodexMonitorApp:
    TABLE_HEADER_PAD_Y = 6
    TABLE_ROW_PAD_Y = 5
    TABLE_ROW_GAP_Y = 4
    TABLE_SCROLLBAR_WIDTH = 8
    TABLE_SCROLLBAR_PAD_X = 4
    TOOLBAR_BUTTON_SIZE = 32
    TOOLBAR_BUTTON_RADIUS = 8
    TOOLBAR_ICON_SIZE = 18
    ROW_BUTTON_SIZE = 26
    ROW_BUTTON_RADIUS = 6
    ROW_ICON_SIZE = 16
    AUTH_SIGNATURE_POLL_MS = 5000
    AUTH_EVENT_SETTLE_MS = 250
    AUTH_PARSE_RETRY_MS = 300
    MAX_AUTH_PARSE_RETRIES = 4
    MISSING_TOKEN_RETRY_MS = 350
    MAX_MISSING_TOKEN_RETRIES = 6

    def __init__(self, root: ctk.CTk):
        self.root = root
        self.root.title(APP_TITLE)
        self.root.geometry(WINDOW_GEOMETRY)
        self.root.minsize(*WINDOW_MIN_SIZE)

        self.storage = UsageStorage()
        self.api_client = UsageApiClient()
        self.auth_refresh_client = AuthRefreshClient()
        self.state = MonitorStateService(self.storage)
        self.auth_file_service = AuthFileService()
        self.auth_watcher = AuthFileWatcher(
            watch_dir=AUTH_DIR,
            target_file=AUTH_FILE_PATH,
            callback=self.on_file_changed,
        )

        self._timer_id: Optional[str] = None
        self._pending_fetches = 0
        self._accounts_window_id: Optional[int] = None
        self._update_check_timer_id: Optional[str] = None
        self._auth_poll_timer_id: Optional[str] = None
        self._auth_change_job: Optional[str] = None
        self._auth_retry_job: Optional[str] = None
        self._auth_retry_attempts = 0
        self._missing_token_retry_job: Optional[str] = None
        self._missing_token_retry_attempts = 0
        self._last_seen_auth_signature = self._get_auth_file_signature()
        self._last_auth_refresh_marker: Optional[str] = None
        self._last_seen_access_token: Optional[str] = None
        self._suppressed_auth_signature: Optional[Tuple[int, int]] = None
        self._account_confirm_result = False
        self.manual_button: Optional[ctk.CTkButton] = None
        self.copy_status_button: Optional[ctk.CTkButton] = None
        self.export_button: Optional[ctk.CTkButton] = None
        self.import_button: Optional[ctk.CTkButton] = None
        self.show_archived_button: Optional[ctk.CTkFrame] = None
        self.login_button: Optional[ctk.CTkButton] = None
        self.auto_fetch_label: Optional[ctk.CTkLabel] = None
        self.auto_fetch_menu: Optional[ctk.CTkOptionMenu] = None
        self.status_textbox: Optional[tk.Text] = None
        self.log_toggle_button: Optional[ctk.CTkFrame] = None
        self.clear_logs_button: Optional[ctk.CTkButton] = None
        self.log_textbox: Optional[tk.Text] = None
        self.check_update_button: Optional[ctk.CTkButton] = None
        self.update_button: Optional[ctk.CTkButton] = None
        self.theme_button: Optional[ctk.CTkButton] = None
        self.header_frame: Optional[ctk.CTkFrame] = None
        self.accounts_body_frame: Optional[ctk.CTkFrame] = None
        self.status_frame: Optional[ctk.CTkFrame] = None
        self.accounts_scrollbar: Optional[ctk.CTkScrollbar] = None
        self.header_labels: Dict[str, Tuple[ctk.CTkLabel, str]] = {}
        self._available_release: Optional[ReleaseInfo] = None
        self._prepared_update: Optional[Tuple[ReleaseInfo, str, str, str]] = None
        self._update_check_in_progress = False
        self._manual_update_check_requested = False
        self._update_prepare_in_progress = False
        self._update_in_progress = False
        self._login_in_progress = False
        self._login_dialog: Optional[ctk.CTkToplevel] = None
        self._login_output_textbox: Optional[tk.Text] = None
        self._login_opened_url: Optional[str] = None
        self._login_process: Optional[subprocess.Popen] = None
        self._login_added_email: Optional[str] = None
        self._backup_auto_fetch_cursor = 0
        self._tooltips = []
        self._row_tooltips = []
        self._material_symbols_available = register_material_symbols_font()
        self._migrate_legacy_log_file()

        self._sort_save_timer: Optional[str] = None
        self._spinner_timer_id: Optional[str] = None
        self._spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        self._spinner_index = 0
        self.sort_column: Optional[str] = self._normalized_sort_column(self.state.sort_column)
        self.sort_asc: bool = self.state.sort_asc
        self.show_archived: bool = self.state.show_archived
        self.logs_expanded: bool = self.state.logs_expanded
        self._last_logged_status: Optional[str] = None

        self.setup_ui()
        self.refresh_ui()

        self.auth_watcher.start()
        self._schedule_next_auth_poll()
        self.root.after(0, self.initial_fetch_on_startup)
        self.root.after(1200, self.check_for_updates_silently)
        self.root.protocol("WM_DELETE_WINDOW", self.on_closing)

    def _normalized_sort_column(self, sort_column: Optional[str]) -> Optional[str]:
        if sort_column in ("quota", "short_quota"):
            return "weekly_quota"
        if sort_column == "short_reset":
            return "weekly_reset"
        return sort_column

    def _theme_tokens(self) -> Dict[str, str]:
        if ctk.get_appearance_mode().lower() == "dark":
            return {
                "app_bg": "#0E141B",
                "card": "#16202A",
                "text": "#ECF3F8",
                "muted": "#97A6B3",
                "border": "#263341",
                "table_shell": "#121A23",
                "table_fg": "#ECF3F8",
                "header_bg": "#1A2633",
                "header_fg": "#F5FAFF",
                "selection_bg": "#234C84",
                "control_bg": "#3B82F6",
                "control_hover": "#60A5FA",
                "control_fg": "#F8FBFF",
                "scrollbar_thumb": "#344352",
                "scrollbar_thumb_hover": "#516273",
                "row_even": "#131D27",
                "row_odd": "#1A2530",
                "row_border": "#263341",
                "current_bg": "#123126",
                "current_fg": "#E4FAEC",
                "current_border": "#2D8A66",
                "empty_fg": "#91A0AE",
            }

        return {
            "app_bg": "#F3F7FB",
            "card": "#FFFFFF",
            "text": "#10202F",
            "muted": "#5F7283",
            "border": "#D7E1EB",
            "table_shell": "#FFFFFF",
            "table_fg": "#10202F",
            "header_bg": "#E8F0F7",
            "header_fg": "#10202F",
            "selection_bg": "#C9DAFF",
            "control_bg": "#2563EB",
            "control_hover": "#1D4ED8",
            "control_fg": "#FFFFFF",
            "scrollbar_thumb": "#B5C2CF",
            "scrollbar_thumb_hover": "#7D90A3",
            "row_even": "#FFFFFF",
            "row_odd": "#F1F6FB",
            "row_border": "#DCE5EE",
            "current_bg": "#E4F6EC",
            "current_fg": "#12613F",
            "current_border": "#87C8A8",
            "empty_fg": "#748697",
        }

    def setup_ui(self) -> None:
        tokens = self._theme_tokens()
        self.root.configure(fg_color=tokens["app_bg"])
        self.root.unbind_all("<MouseWheel>")
        self.root.unbind_all("<Button-4>")
        self.root.unbind_all("<Button-5>")

        self.root.grid_columnconfigure(0, weight=1)
        self.root.grid_rowconfigure(0, weight=1)

        accounts_shell = ctk.CTkFrame(
            self.root,
            corner_radius=18,
            fg_color=tokens["table_shell"],
            border_width=1,
            border_color=tokens["border"],
        )
        accounts_shell.grid(row=0, column=0, sticky="nsew", padx=10, pady=(10, 6))
        accounts_shell.grid_columnconfigure(0, weight=1)
        accounts_shell.grid_rowconfigure(1, weight=1)
        self.accounts_shell = accounts_shell

        header_frame = ctk.CTkFrame(
            accounts_shell,
            corner_radius=12,
            fg_color=tokens["header_bg"],
            border_width=1,
            border_color=tokens["border"],
        )
        header_frame.grid(row=0, column=0, sticky="ew", padx=5, pady=(5, 3))
        self.header_frame = header_frame
        self._build_account_headers()

        body_frame = ctk.CTkFrame(
            accounts_shell,
            corner_radius=10,
            fg_color=tokens["table_shell"],
        )
        body_frame.grid(row=1, column=0, sticky="nsew", padx=5, pady=(0, 5))
        body_frame.grid_columnconfigure(0, weight=1)
        body_frame.grid_rowconfigure(0, weight=1)
        self.accounts_body_frame = body_frame

        self.accounts_canvas = tk.Canvas(
            body_frame,
            highlightthickness=0,
            bd=0,
            relief="flat",
            background=tokens["table_shell"],
        )
        self.accounts_canvas.grid(row=0, column=0, sticky="nsew")
        self.accounts_canvas.configure(yscrollincrement=6)

        accounts_scrollbar = ctk.CTkScrollbar(
            body_frame,
            orientation="vertical",
            command=self.accounts_canvas.yview,
            width=self.TABLE_SCROLLBAR_WIDTH,
            corner_radius=999,
            border_spacing=0,
            minimum_pixel_length=36,
            fg_color=tokens["table_shell"],
            button_color=tokens["scrollbar_thumb"],
            button_hover_color=tokens["scrollbar_thumb_hover"],
        )
        accounts_scrollbar.grid(
            row=0,
            column=1,
            sticky="ns",
            padx=(self.TABLE_SCROLLBAR_PAD_X, 0),
        )
        self.accounts_scrollbar = accounts_scrollbar
        self.accounts_canvas.configure(yscrollcommand=accounts_scrollbar.set)

        self.accounts_rows_frame = ctk.CTkFrame(
            self.accounts_canvas,
            fg_color=tokens["table_shell"],
        )
        self.accounts_rows_frame.grid_columnconfigure(0, weight=1)
        self._accounts_window_id = self.accounts_canvas.create_window(
            (0, 0),
            window=self.accounts_rows_frame,
            anchor="nw",
        )
        self.accounts_rows_frame.bind(
            "<Configure>",
            lambda event: self._update_accounts_scrollregion(),
        )
        self.accounts_canvas.bind("<Configure>", self._resize_accounts_window)

        self.root.bind_all("<MouseWheel>", self._on_global_mousewheel, add="+")
        self.root.bind_all("<Button-4>", self._on_global_mousewheel, add="+")
        self.root.bind_all("<Button-5>", self._on_global_mousewheel, add="+")

        status_frame = ctk.CTkFrame(
            self.root,
            corner_radius=16,
            fg_color=tokens["card"],
            border_width=1,
            border_color=tokens["border"],
        )
        status_frame.grid(row=1, column=0, sticky="ew", padx=10, pady=(0, 10))
        status_frame.grid_columnconfigure(0, weight=1)
        status_frame.grid_columnconfigure(1, weight=0)
        self.status_frame = status_frame

        self.status_var = tk.StringVar(value=f"Watching: {AUTH_FILE_PATH} (watchdog)")
        self.status_var.trace_add("write", self._sync_status_textbox)
        self.status_textbox = tk.Text(
            status_frame,
            height=2,
            wrap="word",
            borderwidth=0,
            highlightthickness=0,
            relief="flat",
            background=tokens["card"],
            foreground=tokens["muted"],
            insertbackground=tokens["text"],
            selectbackground=tokens["selection_bg"],
            font=("TkDefaultFont", 11),
        )
        self.status_textbox.grid(row=0, column=0, sticky="ew", padx=(12, 0), pady=9)

        controls_frame = ctk.CTkFrame(status_frame, fg_color="transparent")
        controls_frame.grid(row=0, column=1, sticky="e", padx=(8, 10), pady=7)
        self.status_controls_frame = controls_frame

        copy_text = self._material_icon_text("copy")
        self.copy_status_button = ctk.CTkButton(
            controls_frame,
            text=copy_text or "⧉",
            command=self.copy_status_message,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.copy_status_button.grid(row=0, column=0, sticky="e", padx=(0, 6), pady=0)
        self._force_square_button(self.copy_status_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.copy_status_button, "Copy status")

        self.log_toggle_button = self._create_labeled_icon_button(
            controls_frame,
            icon=self._material_icon_text("visibility") or "●",
            label="Logs",
            command=self.toggle_logs,
            tooltip="Show logs",
        )
        self.log_toggle_button.grid(row=0, column=1, sticky="e", padx=(0, 6), pady=0)

        clear_logs_text = self._material_icon_text("delete_sweep")
        self.clear_logs_button = ctk.CTkButton(
            controls_frame,
            text=clear_logs_text or "×",
            command=self.clear_logs,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.clear_logs_button.grid(row=0, column=2, sticky="e", padx=(0, 10), pady=0)
        self._force_square_button(self.clear_logs_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.clear_logs_button, "Clear logs")
        self.clear_logs_button.grid_remove()

        export_text = self._material_icon_text("download")
        self.export_button = ctk.CTkButton(
            controls_frame,
            text=export_text or "↓",
            command=self.export_data,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.export_button.grid(row=0, column=4, sticky="e", padx=(0, 6), pady=0)
        self._force_square_button(self.export_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.export_button, "Export data")

        import_text = self._material_icon_text("upload")
        self.import_button = ctk.CTkButton(
            controls_frame,
            text=import_text or "↑",
            command=self.import_data,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.import_button.grid(row=0, column=5, sticky="e", padx=(0, 10), pady=0)
        self._force_square_button(self.import_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.import_button, "Import data")

        login_text = self._material_icon_text("person_add")
        self.login_button = ctk.CTkButton(
            controls_frame,
            text=login_text or "+",
            command=self.start_codex_login,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.login_button.grid(row=0, column=6, sticky="e", padx=(0, 10), pady=0)
        self._force_square_button(self.login_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.login_button, "Add account")

        self.auto_fetch_label = ctk.CTkLabel(
            controls_frame,
            text="Auto Fetch",
            font=ctk.CTkFont(size=10, weight="bold"),
            text_color=tokens["muted"],
        )
        self.auto_fetch_label.grid(row=0, column=7, sticky="e", padx=(0, 5), pady=0)

        self.auto_fetch_menu = ctk.CTkOptionMenu(
            controls_frame,
            values=AUTO_FETCH_OPTIONS,
            command=self.save_current_auto_fetch_value,
            width=88,
            height=32,
            corner_radius=10,
            dynamic_resizing=False,
            font=ctk.CTkFont(size=11, weight="bold"),
            dropdown_font=ctk.CTkFont(size=11),
            fg_color=tokens["control_bg"],
            button_color=tokens["control_bg"],
            button_hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
            anchor="w",
        )
        self.auto_fetch_menu.grid(row=0, column=8, sticky="e", padx=(0, 6), pady=0)

        refresh_text = self._material_icon_text("refresh")
        self.manual_button = ctk.CTkButton(
            controls_frame,
            text=refresh_text or "⟳",
            command=self.manual_fetch,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.manual_button.grid(row=0, column=9, sticky="e", padx=(0, 6), pady=0)
        self._force_square_button(self.manual_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.manual_button, "Fetch quota")

        self.show_archived_button = self._create_labeled_icon_button(
            controls_frame,
            icon=self._show_archived_icon(),
            label="Arch",
            command=self.toggle_show_archived,
            tooltip="Hide archived accounts" if self.show_archived else "Show archived accounts",
        )
        self.show_archived_button.grid(row=0, column=10, sticky="e", padx=(0, 6), pady=0)

        check_update_text = self._material_icon_text("update")
        self.check_update_button = ctk.CTkButton(
            controls_frame,
            text=check_update_text or "↻",
            command=self.check_for_updates_manually,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.check_update_button.grid(row=0, column=11, sticky="e", padx=(0, 6), pady=0)
        self._force_square_button(self.check_update_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.check_update_button, "Check for updates")

        self.update_button = ctk.CTkButton(
            controls_frame,
            text="Update",
            command=self.update_application,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=64,
            border_spacing=0,
            anchor="center",
            font=ctk.CTkFont(size=11, weight="bold"),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.update_button.grid(row=0, column=12, sticky="e", padx=(0, 6), pady=0)

        theme_text = self._appearance_toggle_icon()
        self.theme_button = ctk.CTkButton(
            controls_frame,
            text=theme_text,
            command=self.toggle_appearance_mode,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            height=self.TOOLBAR_BUTTON_SIZE,
            width=self.TOOLBAR_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(18),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        self.theme_button.grid(row=0, column=13, sticky="e", padx=(0, 0), pady=0)
        self._force_square_button(self.theme_button, self.TOOLBAR_BUTTON_SIZE)
        self._attach_tooltip(self.theme_button, "Toggle theme")

        self.log_textbox = tk.Text(
            status_frame,
            height=6,
            wrap="word",
            borderwidth=0,
            highlightthickness=1,
            highlightbackground=tokens["border"],
            highlightcolor=tokens["border"],
            relief="flat",
            background=tokens["table_shell"],
            foreground=tokens["muted"],
            insertbackground=tokens["text"],
            selectbackground=tokens["selection_bg"],
            font=("TkDefaultFont", 10),
        )
        self.log_textbox.grid(row=1, column=0, columnspan=2, sticky="ew", padx=12, pady=(0, 10))
        self.log_textbox.grid_remove()

        self._sync_status_textbox()
        self._sync_logs_visibility()
        self._update_manual_button_state()

    def _account_column_layout(self) -> Dict[str, int]:
        return {
            "email": 0,
            "quota": 1,
            "reset": 2,
            "action": 3,
            "gutter": 4,
        }

    def _configure_account_columns(self, frame: ctk.CTkFrame) -> None:
        for column in range(7):
            frame.grid_columnconfigure(column, weight=0, uniform="", minsize=0)

        columns = self._account_column_layout()
        frame.grid_columnconfigure(columns["email"], weight=5, uniform="account-cols")
        frame.grid_columnconfigure(columns["quota"], weight=2, uniform="account-cols", minsize=92)
        frame.grid_columnconfigure(columns["reset"], weight=5, uniform="account-cols")
        frame.grid_columnconfigure(columns["action"], weight=3, uniform="account-cols", minsize=152)

    def _build_account_headers(self) -> None:
        if not self.header_frame:
            return

        for widget in self.header_frame.winfo_children():
            widget.destroy()

        self.header_labels = {}
        self._configure_account_columns(self.header_frame)
        columns = self._account_column_layout()
        self.header_frame.grid_columnconfigure(
            columns["gutter"],
            minsize=self._table_scrollbar_gutter_width(),
        )
        self._build_header_cell(self.header_frame, "Account Email", columns["email"], "w", sort_id="email")
        self._build_header_cell(self.header_frame, "Quota", columns["quota"], "w", sort_id="weekly_quota")
        self._build_header_cell(self.header_frame, "Reset", columns["reset"], "w", sort_id="weekly_reset")
        self._build_header_cell(self.header_frame, "Action", columns["action"], "e")

    def _table_scrollbar_gutter_width(self) -> int:
        return self.TABLE_SCROLLBAR_WIDTH + self.TABLE_SCROLLBAR_PAD_X

    def _material_icon_text(self, name: str) -> str:
        codepoints = {
            "copy": "e14d",
            "download": "f090",
            "refresh": "e5d5",
            "update": "e923",
            "delete": "e92e",
            "archive": "e149",
            "unarchive": "e169",
            "visibility": "e8f4",
            "visibility_off": "e8f5",
            "history": "e889",
            "view_column": "e8ec",
            "light_mode": "e518",
            "dark_mode": "e51c",
            "upload": "f09b",
            "sync": "e863",
            "delete_sweep": "e16c",
            "swap_horiz": "e8d4",
            "person_add": "e7fe",
        }
        if not self._material_symbols_available:
            return ""
        symbol = material_symbol(codepoints.get(name, ""))
        return symbol or ""

    def _material_icon_font(self, size: int) -> ctk.CTkFont:
        if self._material_symbols_available:
            return ctk.CTkFont(family=MATERIAL_SYMBOLS_FAMILY, size=size)
        return ctk.CTkFont(size=size, weight="bold")

    def _attach_tooltip(self, widget: tk.Misc, text: str, row_tooltip: bool = False) -> None:
        tokens = self._theme_tokens()
        tooltip = ToolTip(
            widget,
            text,
            bg_color=tokens["card"],
            fg_color=tokens["text"],
            border_color=tokens["border"],
        )
        if row_tooltip:
            self._row_tooltips.append(tooltip)
        else:
            self._tooltips.append(tooltip)

    def _create_labeled_icon_button(
        self,
        parent: tk.Misc,
        *,
        icon: str,
        label: str,
        command,
        tooltip: str,
    ) -> ctk.CTkFrame:
        tokens = self._theme_tokens()
        button = ctk.CTkFrame(
            parent,
            width=self.TOOLBAR_BUTTON_SIZE,
            height=self.TOOLBAR_BUTTON_SIZE,
            corner_radius=self.TOOLBAR_BUTTON_RADIUS,
            fg_color=tokens["control_bg"],
        )
        button.grid_propagate(False)
        button.pack_propagate(False)
        button._icon_label = tk.Label(
            button,
            text=icon,
            borderwidth=0,
            highlightthickness=0,
            padx=0,
            pady=0,
            background=tokens["control_bg"],
            foreground=tokens["control_fg"],
            font=(MATERIAL_SYMBOLS_FAMILY, 16) if self._material_symbols_available else ("TkDefaultFont", 12, "bold"),
        )
        button._icon_label.place(relx=0.5, y=10, anchor="n")
        button._text_label = tk.Label(
            button,
            text=label,
            borderwidth=0,
            highlightthickness=0,
            padx=0,
            pady=0,
            background=tokens["control_bg"],
            foreground=tokens["control_fg"],
            font=("TkDefaultFont", 6, "bold"),
        )
        button._text_label.place(relx=0.5, y=2, anchor="n")

        def apply_color(color: str) -> None:
            button.configure(fg_color=color)
            button._icon_label.configure(background=color)
            button._text_label.configure(background=color)

        def on_enter(_event: tk.Event) -> None:
            apply_color(self._theme_tokens()["control_hover"])

        def on_leave(_event: tk.Event) -> None:
            apply_color(self._theme_tokens()["control_bg"])

        def on_click(_event: tk.Event) -> str:
            command()
            return "break"

        for widget in (button, button._icon_label, button._text_label):
            widget.bind("<Enter>", on_enter, add="+")
            widget.bind("<Leave>", on_leave, add="+")
            widget.bind("<Button-1>", on_click, add="+")

        self._attach_tooltip(button, tooltip)
        return button

    def _configure_labeled_icon_button(
        self,
        button: Optional[ctk.CTkFrame],
        *,
        icon: Optional[str] = None,
        label: Optional[str] = None,
    ) -> None:
        if not button:
            return

        tokens = self._theme_tokens()
        button.configure(fg_color=tokens["control_bg"])
        icon_label = getattr(button, "_icon_label", None)
        if icon_label is not None:
            if icon is not None:
                icon_label.configure(text=icon)
            icon_label.configure(
                background=tokens["control_bg"],
                foreground=tokens["control_fg"],
            )
        text_label = getattr(button, "_text_label", None)
        if text_label is not None:
            if label is not None:
                text_label.configure(text=label)
            text_label.configure(
                background=tokens["control_bg"],
                foreground=tokens["control_fg"],
            )

    def _center_icon_button(self, button: ctk.CTkButton) -> None:
        image_label = getattr(button, "_image_label", None)
        if image_label is not None:
            image_label.grid_configure(row=2, column=2, sticky="nsew")

    def _force_square_button(self, button: ctk.CTkButton, size: int) -> None:
        button.configure(width=size, height=size)
        button.grid_propagate(False)
        button.pack_propagate(False)
        self._center_icon_button(button)

    def _appearance_toggle_icon(self) -> str:
        if ctk.get_appearance_mode().lower() == "dark":
            return self._material_icon_text("light_mode") or "☀"
        return self._material_icon_text("dark_mode") or "☾"

    def _show_archived_icon(self) -> str:
        if self.show_archived:
            return self._material_icon_text("visibility_off") or "○"
        return self._material_icon_text("visibility") or "●"

    def _update_show_archived_button(self) -> None:
        if not self.show_archived_button:
            return

        self._configure_labeled_icon_button(
            self.show_archived_button,
            icon=self._show_archived_icon(),
            label="Arch",
        )
        self._update_tooltip_text(
            self.show_archived_button,
            "Hide archived accounts" if self.show_archived else "Show archived accounts",
        )

    def rebuild_ui(self) -> None:
        status_message = self.status_var.get() if hasattr(self, "status_var") else ""
        for widget in self.root.winfo_children():
            widget.destroy()
        self.manual_button = None
        self.copy_status_button = None
        self.export_button = None
        self.import_button = None
        self.login_button = None
        self.show_archived_button = None
        self.auto_fetch_label = None
        self.auto_fetch_menu = None
        self.status_textbox = None
        self.log_toggle_button = None
        self.clear_logs_button = None
        self.log_textbox = None
        self.update_button = None
        self.theme_button = None
        self.header_frame = None
        self.accounts_body_frame = None
        self.status_frame = None
        self.accounts_scrollbar = None
        self.header_labels = {}
        self._accounts_window_id = None
        self._tooltips = []
        self._row_tooltips = []
        self.setup_ui()
        if status_message:
            self.status_var.set(status_message)
        self._update_manual_button_state()
        self.refresh_ui(skip_auto_fetch=True)

    def toggle_appearance_mode(self) -> None:
        next_mode = (
            "light" if ctk.get_appearance_mode().lower() == "dark" else "dark"
        )
        ctk.set_appearance_mode(next_mode)
        self._apply_theme_to_static_ui()
        self.refresh_ui(skip_auto_fetch=True)
        self._update_manual_button_state()

    def _apply_theme_to_static_ui(self) -> None:
        tokens = self._theme_tokens()
        self.root.configure(fg_color=tokens["app_bg"])

        if getattr(self, "accounts_shell", None):
            self.accounts_shell.configure(
                fg_color=tokens["table_shell"],
                border_color=tokens["border"],
            )
        if self.header_frame:
            self.header_frame.configure(
                fg_color=tokens["header_bg"],
                border_color=tokens["border"],
            )
        if self.accounts_body_frame:
            self.accounts_body_frame.configure(
                fg_color=tokens["table_shell"],
                bg_color=tokens["table_shell"],
            )
        if getattr(self, "accounts_canvas", None):
            self.accounts_canvas.configure(background=tokens["table_shell"])
        if getattr(self, "accounts_rows_frame", None):
            self.accounts_rows_frame.configure(
                fg_color=tokens["table_shell"],
                bg_color=tokens["table_shell"],
            )
        if self.accounts_scrollbar:
            self.accounts_scrollbar.configure(
                fg_color=tokens["table_shell"],
                bg_color=tokens["table_shell"],
                button_color=tokens["scrollbar_thumb"],
                button_hover_color=tokens["scrollbar_thumb_hover"],
            )
        if self.status_frame:
            self.status_frame.configure(
                fg_color=tokens["card"],
                border_color=tokens["border"],
            )
        if self.status_textbox:
            self.status_textbox.configure(
                background=tokens["card"],
                foreground=tokens["muted"],
                insertbackground=tokens["text"],
                selectbackground=tokens["selection_bg"],
            )
        if self.log_textbox:
            self.log_textbox.configure(
                background=tokens["table_shell"],
                foreground=tokens["muted"],
                insertbackground=tokens["text"],
                selectbackground=tokens["selection_bg"],
                highlightbackground=tokens["border"],
                highlightcolor=tokens["border"],
            )
        if self.auto_fetch_label:
            self.auto_fetch_label.configure(text_color=tokens["muted"])
        if self.auto_fetch_menu:
            self.auto_fetch_menu.configure(
                fg_color=tokens["control_bg"],
                button_color=tokens["control_bg"],
                button_hover_color=tokens["control_hover"],
                text_color=tokens["control_fg"],
            )

        for button in (
            self.copy_status_button,
            self.clear_logs_button,
            self.export_button,
            self.import_button,
            self.login_button,
            self.manual_button,
            self.check_update_button,
            self.update_button,
            self.theme_button,
        ):
            if button:
                button.configure(
                    fg_color=tokens["control_bg"],
                    hover_color=tokens["control_hover"],
                    text_color=tokens["control_fg"],
                )

        self._configure_labeled_icon_button(self.log_toggle_button)
        self._configure_labeled_icon_button(self.show_archived_button)
        self._update_header_sort_labels()
        for tooltip in self._tooltips + self._row_tooltips:
            tooltip.bg_color = tokens["card"]
            tooltip.fg_color = tokens["text"]
            tooltip.border_color = tokens["border"]

    def _build_header_cell(
        self,
        parent: ctk.CTkFrame,
        text: str,
        column: int,
        anchor: str,
        sort_id: Optional[str] = None,
    ) -> None:
        tokens = self._theme_tokens()
        
        display_text = text
        if sort_id:
            if sort_id == self.sort_column:
                display_text += " ▲" if self.sort_asc else " ▼"
            else:
                display_text += " ↕"

        label = ctk.CTkLabel(
            parent,
            text=display_text,
            anchor=anchor,
            font=ctk.CTkFont(size=11, weight="bold"),
            text_color=tokens["header_fg"],
            cursor="hand2" if sort_id else None,
        )
        if sort_id:
            label.bind("<Button-1>", lambda event, sid=sort_id: self._on_header_click(sid))
            label.bind("<Enter>", lambda event, l=label: l.configure(text_color=tokens["control_bg"]))
            label.bind("<Leave>", lambda event, l=label: l.configure(text_color=tokens["header_fg"]))
            self.header_labels[sort_id] = (label, text)
        else:
            self.header_labels[f"static-{column}"] = (label, text)
            
        label.grid(
            row=0,
            column=column,
            sticky="ew",
            padx=10,
            pady=self.TABLE_HEADER_PAD_Y,
        )

    def _on_header_click(self, sort_id: str) -> None:
        if self.sort_column == sort_id:
            if self.sort_asc:
                self.sort_asc = False
            else:
                self.sort_column = None
                self.sort_asc = True
        else:
            self.sort_column = sort_id
            self.sort_asc = True
            
        if self._sort_save_timer:
            self.root.after_cancel(self._sort_save_timer)
        self._sort_save_timer = self.root.after(
            1000, lambda: self.state.save_sort_preference(self.sort_column, self.sort_asc)
        )
            
        self._update_header_sort_labels()
        self.refresh_ui(skip_auto_fetch=True)

    def _update_header_sort_labels(self) -> None:
        tokens = self._theme_tokens()
        for sort_id, (label, base_text) in self.header_labels.items():
            display_text = base_text
            if sort_id in ("email", "quota", "weekly_quota", "short_quota", "short_reset", "weekly_reset"):
                if sort_id == self.sort_column:
                    display_text += " ▲" if self.sort_asc else " ▼"
                else:
                    display_text += " ↕"
                label.bind("<Enter>", lambda event, l=label: l.configure(text_color=tokens["control_bg"]))
                label.bind("<Leave>", lambda event, l=label: l.configure(text_color=tokens["header_fg"]))

            label.configure(text=display_text, text_color=tokens["header_fg"])

    def _build_stacked_value_label(
        self,
        parent: ctk.CTkFrame,
        lines: List[str],
        text_color: str,
        column: int,
        anchor: str = "w",
        bold: bool = False,
    ) -> None:
        label = ctk.CTkLabel(
            parent,
            text="\n".join(lines),
            anchor=anchor,
            justify="left" if anchor == "w" else "right",
            font=ctk.CTkFont(size=11, weight="bold" if bold else "normal"),
            text_color=text_color,
        )
        label.grid(
            row=0,
            column=column,
            sticky="ew",
            padx=10,
            pady=(3, 3),
        )

    def _window_reset_ts(self, data: dict, field: str) -> Optional[float]:
        window = data.get(field)
        if not isinstance(window, dict):
            return None

        reset_at = window.get("reset_at")
        if isinstance(reset_at, (int, float)) and not isinstance(reset_at, bool):
            return reset_at
        return None

    def _window_used_percent(self, data: dict, field: str) -> Optional[float]:
        window = data.get(field)
        if not isinstance(window, dict):
            return None

        used_percent = window.get("used_percent")
        if isinstance(used_percent, (int, float)) and not isinstance(used_percent, bool):
            return used_percent
        return None

    def _has_window_data(self, data: dict, field: str) -> bool:
        return (
            self._window_used_percent(data, field) is not None
            or self._window_reset_ts(data, field) is not None
        )

    def _labeled_value(self, value: str, label: str) -> str:
        return f"{value} ({label})"

    def _clear_account_rows(self) -> None:
        for tooltip in self._row_tooltips:
            tooltip._hide()
        self._row_tooltips = []
        for widget in self.accounts_rows_frame.winfo_children():
            widget.destroy()

    def _is_widget_in_accounts_area(self, widget: Optional[tk.Misc]) -> bool:
        while widget is not None:
            if widget in (
                self.accounts_canvas,
                self.accounts_rows_frame,
                self.accounts_shell,
            ):
                return True
            widget = getattr(widget, "master", None)
        return False

    def _reset_credits_tooltip_text(self, email: str) -> str:
        from .formatters import soonest_expiring_credit, format_reset_time_remaining

        payload = self.state.get_account_resets(email)
        if not payload:
            return "Fetch quota to load reset credits"
        credits = payload.get("credits") if isinstance(payload, dict) else None
        if not isinstance(credits, list) or not credits:
            return "No reset credits"
        soonest = soonest_expiring_credit(payload)
        if not soonest:
            return "No available reset credits"
        remaining = format_reset_time_remaining(soonest.get("expires_at"), time.time())
        return f"Reset credit expires in {remaining}"

    def _show_reset_credits_modal(self, email: str) -> None:
        tokens = self._theme_tokens()
        payload = self.state.get_account_resets(email)
        from .formatters import (
            format_reset_credit_expires,
            format_reset_time_remaining,
            format_reset_granted_at,
        )
        now_ts = datetime.now().timestamp()

        dialog = ctk.CTkToplevel(self.root)
        dialog.title(f"Reset Credits — {email}")
        dialog.resizable(True, True)
        dialog.transient(self.root)
        dialog.configure(fg_color=tokens["card"])
        dialog.minsize(520, 240)
        dialog.grid_columnconfigure(0, weight=1)
        dialog.grid_rowconfigure(0, weight=1)

        container = ctk.CTkFrame(
            dialog,
            corner_radius=14,
            fg_color=tokens["card"],
            border_width=1,
            border_color=tokens["border"],
        )
        container.grid(row=0, column=0, sticky="nsew", padx=12, pady=12)
        container.grid_columnconfigure(0, weight=1)
        container.grid_rowconfigure(1, weight=1)

        title_label = ctk.CTkLabel(
            container,
            text=f"Reset Credits — {email}",
            anchor="w",
            font=ctk.CTkFont(size=15, weight="bold"),
            text_color=tokens["text"],
        )
        title_label.grid(row=0, column=0, sticky="ew", padx=14, pady=(14, 6))

        if payload and isinstance(payload.get("credits"), list) and payload["credits"]:
            credits = payload["credits"]
            rows = []
            for credit in credits:
                expires_text = format_reset_credit_expires(credit.get("expires_at"), now_ts)
                remaining_text = format_reset_time_remaining(credit.get("expires_at"), now_ts)
                granted_text = format_reset_granted_at(credit.get("granted_at"))
                status_text = credit.get("status", "unknown")
                rows.append([expires_text, remaining_text, granted_text, status_text])

            table_frame = ctk.CTkScrollableFrame(
                container,
                fg_color=tokens["table_shell"],
                corner_radius=8,
            )
            table_frame.grid(row=1, column=0, sticky="nsew", padx=14, pady=(0, 12))
            for col in range(4):
                table_frame.grid_columnconfigure(col, weight=1)

            headers = ["Expires", "Time Remaining", "Granted", "Status"]
            for col, header in enumerate(headers):
                header_label = ctk.CTkLabel(
                    table_frame,
                    text=header,
                    font=ctk.CTkFont(size=11, weight="bold"),
                    text_color=tokens["header_fg"],
                )
                header_label.grid(row=0, column=col, sticky="ew", padx=6, pady=4)

            for row_idx, row_data in enumerate(rows):
                for col_idx, cell in enumerate(row_data):
                    cell_label = ctk.CTkLabel(
                        table_frame,
                        text=cell,
                        font=ctk.CTkFont(size=11),
                        text_color=tokens["text"],
                        anchor="w",
                    )
                    cell_label.grid(row=row_idx + 1, column=col_idx, sticky="ew", padx=6, pady=2)
        else:
            if payload is None:
                msg = "No reset credits data. Fetch quota first."
            elif not isinstance(payload.get("credits"), list):
                msg = "Error fetching reset credits."
            else:
                msg = "No reset credits found."
            msg_label = ctk.CTkLabel(
                container,
                text=msg,
                font=ctk.CTkFont(size=12),
                text_color=tokens["muted"],
            )
            msg_label.grid(row=1, column=0, sticky="nsew", padx=14, pady=(0, 12))

        buttons = ctk.CTkFrame(container, fg_color="transparent")
        buttons.grid(row=2, column=0, sticky="e", padx=14, pady=(0, 14))

        close_button = ctk.CTkButton(
            buttons,
            text="Close",
            command=dialog.destroy,
            corner_radius=8,
            height=30,
            width=82,
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        close_button.grid(row=0, column=0)

        dialog.bind("<Escape>", lambda _e: dialog.destroy())
        dialog.protocol("WM_DELETE_WINDOW", dialog.destroy)
        dialog.update_idletasks()

        root_x = self.root.winfo_rootx()
        root_y = self.root.winfo_rooty()
        root_width = self.root.winfo_width()
        root_height = self.root.winfo_height()
        dlg_width = dialog.winfo_reqwidth()
        dlg_height = dialog.winfo_reqheight()
        x = root_x + max((root_width - dlg_width) // 2, 0)
        y = root_y + max((root_height - dlg_height) // 2, 0)
        dialog.geometry(f"+{x}+{y}")
        dialog.grab_set()
        dialog.lift()

    def _on_global_mousewheel(self, event: tk.Event) -> Optional[str]:
        hovered_widget = self.root.winfo_containing(
            self.root.winfo_pointerx(),
            self.root.winfo_pointery(),
        )
        if not self._is_widget_in_accounts_area(hovered_widget):
            return None

        pixel_delta = 0.0
        event_num = getattr(event, "num", None)

        if event_num == 4:
            pixel_delta = -24.0
        elif event_num == 5:
            pixel_delta = 24.0
        else:
            delta = getattr(event, "delta", 0)
            if delta == 0:
                return "break"
            if abs(delta) >= 120:
                pixel_delta = -(delta / 120.0) * 48.0
            else:
                pixel_delta = -delta * 2.5

        self._scroll_accounts_by_pixels(pixel_delta)
        return "break"

    def _scroll_accounts_by_pixels(self, pixel_delta: float) -> None:
        bbox = self.accounts_canvas.bbox("all")
        if not bbox:
            return

        widget_height = max(self.accounts_canvas.winfo_height(), 1)
        content_height = max(bbox[3] - bbox[1], widget_height)
        max_first = max(1.0 - (widget_height / content_height), 0.0)
        if max_first <= 0:
            return

        first, _ = self.accounts_canvas.yview()
        next_first = min(
            max(first + (pixel_delta / content_height), 0.0),
            max_first,
        )
        self.accounts_canvas.yview_moveto(next_first)

    def _resize_accounts_window(self, event: tk.Event) -> None:
        if self._accounts_window_id is not None:
            self.accounts_canvas.itemconfigure(self._accounts_window_id, width=event.width)

    def _update_accounts_scrollregion(self) -> None:
        bbox = self.accounts_canvas.bbox("all")
        self.accounts_canvas.configure(scrollregion=bbox or (0, 0, 0, 0))

    def _sync_status_textbox(self, *_args: object) -> None:
        if not self.status_textbox or not hasattr(self, "status_var"):
            return

        message = self.status_var.get()
        display_message = self._timestamped_log_line(message) if message else ""
        self.status_textbox.configure(state="normal")
        self.status_textbox.delete("1.0", "end")
        self.status_textbox.insert("1.0", display_message)
        self.status_textbox.configure(state="disabled")
        self._append_log_message(message)

    def _timestamped_log_line(self, message: str) -> str:
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        return f"[{timestamp}] {message}"

    def _append_log_message(self, message: str) -> None:
        if not message or message == self._last_logged_status:
            return

        self._last_logged_status = message
        line = self._timestamped_log_line(message)
        try:
            os.makedirs(os.path.dirname(LOCAL_LOG_FILE), exist_ok=True)
            with open(LOCAL_LOG_FILE, "a", encoding="utf-8") as file:
                file.write(f"{line}\n")
        except OSError:
            return

        if self.logs_expanded:
            self._refresh_log_textbox()

    def _migrate_legacy_log_file(self) -> None:
        if LEGACY_LOCAL_LOG_FILE == LOCAL_LOG_FILE:
            return
        if not os.path.exists(LEGACY_LOCAL_LOG_FILE) or os.path.exists(LOCAL_LOG_FILE):
            return

        try:
            os.makedirs(os.path.dirname(LOCAL_LOG_FILE), exist_ok=True)
            shutil.move(LEGACY_LOCAL_LOG_FILE, LOCAL_LOG_FILE)
        except OSError:
            try:
                shutil.copyfile(LEGACY_LOCAL_LOG_FILE, LOCAL_LOG_FILE)
            except OSError:
                pass

    def _read_log_text(self) -> str:
        try:
            with open(LOCAL_LOG_FILE, "r", encoding="utf-8") as file:
                return file.read().rstrip()
        except OSError:
            return ""

    def _refresh_log_textbox(self) -> None:
        if not self.log_textbox:
            return

        log_text = self._read_log_text() or "No logs yet."
        self.log_textbox.configure(state="normal")
        self.log_textbox.delete("1.0", "end")
        self.log_textbox.insert("1.0", log_text)
        self.log_textbox.see("end")
        self.log_textbox.configure(state="disabled")

    def toggle_logs(self) -> None:
        self.logs_expanded = not self.logs_expanded
        self.state.save_logs_expanded_preference(self.logs_expanded)
        self._sync_logs_visibility()

    def _sync_logs_visibility(self) -> None:
        if not self.log_textbox:
            return

        if self.logs_expanded:
            self._refresh_log_textbox()
            self.log_textbox.grid()
            if self.clear_logs_button:
                self.clear_logs_button.grid()
            if self.log_toggle_button:
                self._configure_labeled_icon_button(
                    self.log_toggle_button,
                    icon=self._material_icon_text("visibility_off") or "-",
                    label="Logs",
                )
                self._update_tooltip_text(self.log_toggle_button, "Hide logs")
            return

        self.log_textbox.grid_remove()
        if self.clear_logs_button:
            self.clear_logs_button.grid_remove()
        if self.log_toggle_button:
            self._configure_labeled_icon_button(
                self.log_toggle_button,
                icon=self._material_icon_text("visibility") or "●",
                label="Logs",
            )
            self._update_tooltip_text(self.log_toggle_button, "Show logs")

    def clear_logs(self) -> None:
        try:
            os.makedirs(os.path.dirname(LOCAL_LOG_FILE), exist_ok=True)
            with open(LOCAL_LOG_FILE, "w", encoding="utf-8"):
                pass
        except OSError as error:
            self.status_var.set(f"Clear logs failed: {error}")
            return

        self._last_logged_status = "Logs cleared."
        self._refresh_log_textbox()
        self.status_var.set("Logs cleared.")

    def _update_tooltip_text(self, widget: tk.Misc, text: str) -> None:
        for tooltip in self._tooltips + self._row_tooltips:
            if tooltip.widget == widget:
                tooltip.text = text

    def copy_status_message(self) -> None:
        message = self.status_var.get() if hasattr(self, "status_var") else ""
        if not message:
            return

        self.root.clipboard_clear()
        self.root.clipboard_append(message)

    def copy_account_email(self, email: str) -> None:
        self.root.clipboard_clear()
        self.root.clipboard_append(email)
        self.status_var.set(f"Copied {email}.")

    def _confirm_account_action(
        self,
        *,
        title: str,
        heading: str,
        message: str,
        confirm_text: str,
    ) -> bool:
        tokens = self._theme_tokens()
        self._account_confirm_result = False

        dialog = ctk.CTkToplevel(self.root)
        dialog.title(title)
        dialog.resizable(False, False)
        dialog.transient(self.root)
        dialog.configure(fg_color=tokens["card"])

        container = ctk.CTkFrame(
            dialog,
            corner_radius=14,
            fg_color=tokens["card"],
            border_width=1,
            border_color=tokens["border"],
        )
        container.grid(row=0, column=0, sticky="nsew", padx=12, pady=12)
        container.grid_columnconfigure(0, weight=1)

        title_label = ctk.CTkLabel(
            container,
            text=heading,
            anchor="w",
            font=ctk.CTkFont(size=15, weight="bold"),
            text_color=tokens["text"],
        )
        title_label.grid(row=0, column=0, sticky="ew", padx=14, pady=(14, 4))

        message_label = ctk.CTkLabel(
            container,
            text=message,
            anchor="w",
            justify="left",
            font=ctk.CTkFont(size=12),
            text_color=tokens["muted"],
        )
        message_label.grid(row=1, column=0, sticky="ew", padx=14, pady=(0, 14))

        buttons = ctk.CTkFrame(container, fg_color="transparent")
        buttons.grid(row=2, column=0, sticky="e", padx=14, pady=(0, 14))

        closed = False

        def close_with(result: bool) -> None:
            nonlocal closed
            if closed:
                return
            closed = True
            self._account_confirm_result = result
            try:
                dialog.grab_release()
            except tk.TclError:
                pass
            try:
                dialog.destroy()
            except tk.TclError:
                pass

        cancel_button = ctk.CTkButton(
            buttons,
            text="Cancel",
            command=lambda: close_with(False),
            corner_radius=8,
            height=30,
            width=82,
            fg_color=tokens["row_border"],
            hover_color=tokens["scrollbar_thumb_hover"],
            text_color=tokens["text"],
        )
        cancel_button.grid(row=0, column=0, padx=(0, 8))

        confirm_button = ctk.CTkButton(
            buttons,
            text=confirm_text,
            command=lambda: close_with(True),
            corner_radius=8,
            height=30,
            width=86,
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        confirm_button.grid(row=0, column=1)

        dialog.bind("<Return>", lambda _event: close_with(True))
        dialog.bind("<Escape>", lambda _event: close_with(False))
        dialog.protocol("WM_DELETE_WINDOW", lambda: close_with(False))

        dialog.update_idletasks()
        root_x = self.root.winfo_rootx()
        root_y = self.root.winfo_rooty()
        root_width = self.root.winfo_width()
        root_height = self.root.winfo_height()
        dialog_width = dialog.winfo_reqwidth()
        dialog_height = dialog.winfo_reqheight()
        x = root_x + max((root_width - dialog_width) // 2, 0)
        y = root_y + max((root_height - dialog_height) // 2, 0)
        dialog.geometry(f"+{x}+{y}")
        dialog.grab_set()
        dialog.lift()
        confirm_button.focus_set()
        self.root.wait_window(dialog)
        return self._account_confirm_result

    def _confirm_remove_account(self, email: str) -> bool:
        return self._confirm_account_action(
            title="Remove account",
            heading="Remove account?",
            message=f"This will remove stored account info for:\n{email}",
            confirm_text="Confirm",
        )

    def _confirm_archive_account(self, email: str) -> bool:
        return self._confirm_account_action(
            title="Archive account",
            heading="Archive account?",
            message=f"This will hide the account from the default list:\n{email}",
            confirm_text="Archive",
        )

    def _confirm_switch_account(self, email: str) -> bool:
        return self._confirm_account_action(
            title="Switch account",
            heading="Switch account?",
            message=(
                "This will replace ~/.codex/auth.json with the saved auth backup for:\n"
                f"{email}"
            ),
            confirm_text="Switch",
        )

    def _show_switch_success_dialog(self, email: str) -> None:
        tokens = self._theme_tokens()
        dialog = ctk.CTkToplevel(self.root)
        dialog.title("Account switched")
        dialog.resizable(False, False)
        dialog.transient(self.root)
        dialog.configure(fg_color=tokens["card"])

        container = ctk.CTkFrame(
            dialog,
            corner_radius=14,
            fg_color=tokens["card"],
            border_width=1,
            border_color=tokens["border"],
        )
        container.grid(row=0, column=0, sticky="nsew", padx=12, pady=12)
        container.grid_columnconfigure(0, weight=1)

        title_label = ctk.CTkLabel(
            container,
            text="Account switched",
            anchor="w",
            font=ctk.CTkFont(size=15, weight="bold"),
            text_color=tokens["text"],
        )
        title_label.grid(row=0, column=0, sticky="ew", padx=14, pady=(14, 4))

        message_label = ctk.CTkLabel(
            container,
            text=(
                f"auth.json is now using:\n{email}\n\n"
                "Restart Codex for the desktop app to pick up the new session."
            ),
            anchor="w",
            justify="left",
            font=ctk.CTkFont(size=12),
            text_color=tokens["muted"],
        )
        message_label.grid(row=1, column=0, sticky="ew", padx=14, pady=(0, 14))

        buttons = ctk.CTkFrame(container, fg_color="transparent")
        buttons.grid(row=2, column=0, sticky="e", padx=14, pady=(0, 14))

        closed = False

        def close_dialog() -> None:
            nonlocal closed
            if closed:
                return
            closed = True
            try:
                dialog.grab_release()
            except tk.TclError:
                pass
            try:
                dialog.destroy()
            except tk.TclError:
                pass

        def restart_and_close() -> None:
            close_dialog()
            self.restart_codex_app()

        later_button = ctk.CTkButton(
            buttons,
            text="Later",
            command=close_dialog,
            corner_radius=8,
            height=30,
            width=82,
            fg_color=tokens["row_border"],
            hover_color=tokens["scrollbar_thumb_hover"],
            text_color=tokens["text"],
        )
        later_button.grid(row=0, column=0, padx=(0, 8))

        restart_button = ctk.CTkButton(
            buttons,
            text="Restart Codex",
            command=restart_and_close,
            corner_radius=8,
            height=30,
            width=116,
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        restart_button.grid(row=0, column=1)

        dialog.bind("<Escape>", lambda _event: close_dialog())
        dialog.protocol("WM_DELETE_WINDOW", close_dialog)
        dialog.update_idletasks()
        root_x = self.root.winfo_rootx()
        root_y = self.root.winfo_rooty()
        root_width = self.root.winfo_width()
        root_height = self.root.winfo_height()
        dialog_width = dialog.winfo_reqwidth()
        dialog_height = dialog.winfo_reqheight()
        x = root_x + max((root_width - dialog_width) // 2, 0)
        y = root_y + max((root_height - dialog_height) // 2, 0)
        dialog.geometry(f"+{x}+{y}")
        dialog.grab_set()
        dialog.lift()
        restart_button.focus_set()

    def remove_account(self, email: str) -> None:
        if not self._confirm_remove_account(email):
            return

        self.auth_file_service.remove_backup(email)
        if self.state.remove_account(email):
            self.refresh_ui(skip_auto_fetch=True)
            self._update_manual_button_state()
            self.status_var.set(f"Removed {email}.")

    def toggle_archive_account(self, email: str, archived: bool) -> None:
        if archived:
            if self.state.unarchive_account(email):
                self.refresh_ui(skip_auto_fetch=True)
                self.status_var.set(f"Unarchived {email}.")
            return

        if not self._confirm_archive_account(email):
            return

        if self.state.archive_account(email):
            self.refresh_ui(skip_auto_fetch=True)
            self.status_var.set(f"Archived {email}.")

    def switch_account(self, email: str) -> None:
        if email == self.state.current_account_email:
            self.status_var.set(f"{email} is already active.")
            return

        if not self.auth_file_service.backup_exists(email):
            self.status_var.set(f"No auth backup available for {email}.")
            return

        if not self._confirm_switch_account(email):
            return

        try:
            snapshot = self.auth_file_service.load_backup_snapshot(email)
            jwt = snapshot.get("tokens", {}).get("access_token")
            if not jwt:
                self.status_var.set(f"Auth backup for {email} has no access_token.")
                return

            self.auth_file_service.switch_to_account_backup(
                target_email=email,
                current_email=self.state.current_account_email,
            )
            self._last_seen_auth_signature = self._get_auth_file_signature()
            self._suppressed_auth_signature = self._last_seen_auth_signature
            self._remember_auth_snapshot(snapshot)
            self.state.set_current_account(email, jwt)
            self.refresh_ui(skip_auto_fetch=True)
            self._begin_fetch(f"Switched to {email}. Fetching latest quota...")
            threading.Thread(
                target=self._bg_fetch_single,
                args=(email, jwt),
                daemon=True,
            ).start()
            self._show_switch_success_dialog(email)
        except Exception as error:
            self.status_var.set(f"Switch account failed: {error}")

    def fetch_backup_account_quota(self, email: str) -> None:
        if not self.auth_file_service.backup_exists(email):
            self.status_var.set(f"No auth backup available for {email}.")
            self.refresh_ui(skip_auto_fetch=True)
            return

        self._begin_fetch(f"Refreshing saved auth backup and fetching quota for {email}...")
        threading.Thread(
            target=self._bg_fetch_backup_account,
            args=(email,),
            daemon=True,
        ).start()

    def restart_codex_app(self) -> None:
        if sys.platform != "darwin":
            self.status_var.set(
                "Please restart Codex manually for this account switch to take effect."
            )
            return

        self.status_var.set("Restarting Codex...")
        threading.Thread(target=self._bg_restart_codex_app, daemon=True).start()

    def _bg_restart_codex_app(self) -> None:
        try:
            subprocess.run(
                [
                    "/usr/bin/osascript",
                    "-e",
                    'tell application "Codex" to quit',
                ],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            self._wait_for_codex_to_exit()
            if self._open_codex_app():
                message = "Requested Codex restart."
            else:
                message = "Requested Codex quit, but reopening Codex failed."
        except Exception as error:
            message = f"Failed to restart Codex: {error}"

        self.root.after(0, lambda m=message: self.status_var.set(m))

    def _wait_for_codex_to_exit(self) -> None:
        deadline = time.time() + 8
        while time.time() < deadline:
            if not self._is_codex_app_running():
                return
            time.sleep(0.25)

    def _is_codex_app_running(self) -> bool:
        try:
            result = subprocess.run(
                [
                    "/usr/bin/osascript",
                    "-e",
                    'application "Codex" is running',
                ],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
            )
        except Exception:
            return False
        return result.stdout.strip().lower() == "true"

    def _open_codex_app(self) -> bool:
        commands = [
            ["/usr/bin/open", "-b", "com.openai.codex"],
            ["/usr/bin/open", "/Applications/Codex.app"],
            ["/usr/bin/open", "-a", "Codex"],
        ]
        for command in commands:
            try:
                result = subprocess.run(
                    command,
                    check=False,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            except Exception:
                continue
            if result.returncode == 0:
                return True
        return False

    def toggle_show_archived(self) -> None:
        self.show_archived = not self.show_archived
        self.state.save_show_archived_preference(self.show_archived)
        self._update_show_archived_button()
        self.refresh_ui(skip_auto_fetch=True)
        self.status_var.set(
            "Showing archived accounts." if self.show_archived else "Hiding archived accounts."
        )

    def export_data(self) -> None:
        default_name = f"codex-monitor-data-{datetime.now().strftime('%Y%m%d-%H%M%S')}.json"
        path = filedialog.asksaveasfilename(
            title="Export Codex Monitor data",
            defaultextension=".json",
            initialfile=default_name,
            filetypes=(("JSON files", "*.json"), ("All files", "*.*")),
        )
        if not path:
            return

        should_export = messagebox.askyesno(
            APP_TITLE,
            "Export complete app data including saved auth tokens?",
        )
        if not should_export:
            return

        try:
            current_email = self.state.current_account_email
            if current_email and self.auth_file_service.auth_file_exists():
                self._sync_current_auth_backup(current_email)

            payload = self.storage.export_data(self.state.usage_map)
            payload["auth_backups"] = self.auth_file_service.export_backups(
                list(self.state.usage_map.keys())
            )
            if self.auth_file_service.auth_file_exists():
                try:
                    active_snapshot = self.auth_file_service.load_snapshot()
                    if active_snapshot.get("tokens", {}).get("access_token"):
                        payload["active_auth"] = active_snapshot
                except Exception:
                    pass
            with open(path, "w", encoding="utf-8") as file:
                json.dump(payload, file, indent=2)
            backup_count = len(payload.get("auth_backups", {}))
            self.status_var.set(
                f"Exported accounts, config, and {backup_count} auth backup(s) to {path}."
            )
        except Exception as error:
            self.status_var.set(f"Export failed: {error}")

    def import_data(self) -> None:
        path = filedialog.askopenfilename(
            title="Import Codex Monitor data",
            filetypes=(("JSON files", "*.json"), ("All files", "*.*")),
        )
        if not path:
            return

        should_import = messagebox.askyesno(
            APP_TITLE,
            (
                "Import accounts, app config, and auth backups from this JSON file? "
                "Existing accounts are merged. The live ~/.codex/auth.json file is not replaced."
            ),
        )
        if not should_import:
            return

        try:
            with open(path, "r", encoding="utf-8") as file:
                payload = json.load(file)
            self.state.import_data(payload)
            backup_payload = payload.get("auth_backups", {})
            active_snapshot = payload.get("active_auth")
            current_email = self.state.current_account_email
            if (
                current_email
                and isinstance(active_snapshot, dict)
                and active_snapshot.get("tokens", {}).get("access_token")
            ):
                backup_payload = dict(backup_payload) if isinstance(backup_payload, dict) else {}
                backup_payload.setdefault(current_email, active_snapshot)

            imported_backup_count = self.auth_file_service.import_backups(backup_payload)
            for email in self.state.usage_map:
                try:
                    jwt = self.auth_file_service.load_backup_access_token(email)
                except Exception:
                    continue
                if jwt:
                    self.state.remember_account_jwt(email, jwt)

            self.sort_column = self._normalized_sort_column(self.state.sort_column)
            self.sort_asc = self.state.sort_asc
            self.show_archived = self.state.show_archived
            self.logs_expanded = self.state.logs_expanded
            self._build_account_headers()
            self._update_show_archived_button()
            self._sync_logs_visibility()
            self.refresh_ui(skip_auto_fetch=True)
            self._update_manual_button_state()
            self.status_var.set(
                f"Imported accounts, config, and {imported_backup_count} auth backup(s) from {path}."
            )
        except Exception as error:
            self.status_var.set(f"Import failed: {error}")

    def start_codex_login(self) -> None:
        if self._login_in_progress:
            self.status_var.set("Codex login is already in progress.")
            return

        self._snapshot_active_auth_before_login()
        self._login_in_progress = True
        self._login_opened_url = None
        self._login_added_email = None
        self._show_codex_login_dialog()
        self._append_login_output("Starting: codex login\n")
        self.status_var.set("Starting Codex login...")
        self._update_manual_button_state()
        threading.Thread(target=self._bg_codex_login, daemon=True).start()

    def _snapshot_active_auth_before_login(self) -> None:
        email = self.state.current_account_email
        if not email:
            return

        try:
            snapshot = self.auth_file_service.load_snapshot()
            if not snapshot.get("tokens", {}).get("access_token"):
                return
            self.auth_file_service.backup_current_auth(email)
        except Exception as error:
            print(
                "[Safe Error Log] Failed to snapshot active auth before login for "
                f"{email}: {error}"
            )

    def _codex_login_command(self) -> Optional[List[str]]:
        codex_bin = self._find_codex_binary()
        if not codex_bin:
            return None

        if sys.platform == "win32":
            _, extension = os.path.splitext(codex_bin)
            if extension.lower() in (".bat", ".cmd"):
                return ["cmd", "/c", codex_bin, "login"]
        return [codex_bin, "login"]

    def _find_codex_binary(self) -> Optional[str]:
        configured_bin = os.environ.get("CODEX_MONITOR_CODEX_BIN")
        if configured_bin and os.path.isfile(configured_bin):
            return configured_bin

        path_bin = shutil.which("codex")
        if path_bin:
            return path_bin

        binary_names = ["codex.exe"] if sys.platform == "win32" else ["codex"]
        search_roots: List[str] = []
        frozen_root = getattr(sys, "_MEIPASS", None)
        if isinstance(frozen_root, str):
            search_roots.append(frozen_root)
        if getattr(sys, "frozen", False):
            search_roots.append(os.path.dirname(sys.executable))

        try:
            import openai_codex
        except Exception:
            openai_codex = None

        if openai_codex is not None:
            package_file = getattr(openai_codex, "__file__", None)
            if package_file:
                package_dir = os.path.dirname(package_file)
                search_roots.append(package_dir)
                search_roots.append(os.path.dirname(package_dir))

        try:
            from importlib import metadata
            codex_cli_dist = metadata.distribution("openai-codex-cli-bin")
        except Exception:
            codex_cli_dist = None

        if codex_cli_dist is not None:
            for dist_file in codex_cli_dist.files or []:
                dist_path = str(dist_file)
                if dist_path.replace("\\", "/").endswith(("/bin/codex", "/bin/codex.exe")):
                    candidate = str(codex_cli_dist.locate_file(dist_file))
                    if os.path.isfile(candidate):
                        self._ensure_executable(candidate)
                        return candidate

        seen_roots = set()
        for root in search_roots:
            if not root or root in seen_roots or not os.path.isdir(root):
                continue
            seen_roots.add(root)
            for current_root, _dirs, files in os.walk(root):
                for binary_name in binary_names:
                    if binary_name not in files:
                        continue
                    candidate = os.path.join(current_root, binary_name)
                    if not os.path.isfile(candidate):
                        continue
                    self._ensure_executable(candidate)
                    return candidate

        return None

    def _ensure_executable(self, path: str) -> None:
        if sys.platform == "win32" or os.access(path, os.X_OK):
            return

        try:
            current_mode = os.stat(path).st_mode
            os.chmod(path, current_mode | 0o755)
        except OSError:
            pass

    def _show_codex_login_dialog(self) -> None:
        if self._login_dialog and self._login_dialog.winfo_exists():
            self._login_dialog.lift()
            return

        tokens = self._theme_tokens()
        dialog = ctk.CTkToplevel(self.root)
        dialog.title("Add account")
        dialog.resizable(True, True)
        dialog.transient(self.root)
        dialog.configure(fg_color=tokens["card"])
        dialog.minsize(440, 280)
        self._login_dialog = dialog

        container = ctk.CTkFrame(
            dialog,
            corner_radius=14,
            fg_color=tokens["card"],
            border_width=1,
            border_color=tokens["border"],
        )
        container.grid(row=0, column=0, sticky="nsew", padx=12, pady=12)
        container.grid_columnconfigure(0, weight=1)
        container.grid_rowconfigure(1, weight=1)
        dialog.grid_columnconfigure(0, weight=1)
        dialog.grid_rowconfigure(0, weight=1)

        title_label = ctk.CTkLabel(
            container,
            text="Login with Codex",
            anchor="w",
            font=ctk.CTkFont(size=15, weight="bold"),
            text_color=tokens["text"],
        )
        title_label.grid(row=0, column=0, sticky="ew", padx=14, pady=(14, 6))

        output_textbox = tk.Text(
            container,
            height=9,
            wrap="word",
            borderwidth=0,
            highlightthickness=1,
            highlightbackground=tokens["border"],
            highlightcolor=tokens["border"],
            relief="flat",
            background=tokens["table_shell"],
            foreground=tokens["text"],
            insertbackground=tokens["text"],
            selectbackground=tokens["selection_bg"],
            font=("TkDefaultFont", 11),
        )
        output_textbox.grid(row=1, column=0, sticky="nsew", padx=14, pady=(0, 12))
        output_textbox.configure(state="disabled")
        self._login_output_textbox = output_textbox

        buttons = ctk.CTkFrame(container, fg_color="transparent")
        buttons.grid(row=2, column=0, sticky="e", padx=14, pady=(0, 14))

        cancel_button = ctk.CTkButton(
            buttons,
            text="Cancel",
            command=self.cancel_codex_login,
            corner_radius=8,
            height=30,
            width=82,
            fg_color=tokens["row_border"],
            hover_color=tokens["scrollbar_thumb_hover"],
            text_color=tokens["text"],
        )
        cancel_button.grid(row=0, column=0, padx=(0, 8))

        close_button = ctk.CTkButton(
            buttons,
            text="Close",
            command=self._close_codex_login_dialog,
            corner_radius=8,
            height=30,
            width=82,
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        close_button.grid(row=0, column=1)

        dialog.protocol("WM_DELETE_WINDOW", self._close_codex_login_dialog)
        dialog.update_idletasks()
        root_x = self.root.winfo_rootx()
        root_y = self.root.winfo_rooty()
        root_width = self.root.winfo_width()
        root_height = self.root.winfo_height()
        dialog_width = dialog.winfo_reqwidth()
        dialog_height = dialog.winfo_reqheight()
        x = root_x + max((root_width - dialog_width) // 2, 0)
        y = root_y + max((root_height - dialog_height) // 2, 0)
        dialog.geometry(f"+{x}+{y}")
        dialog.lift()

    def _close_codex_login_dialog(self) -> None:
        if self._login_in_progress:
            self._stop_codex_login_process()
            self._login_in_progress = False
            self._login_process = None
            self.status_var.set("Codex login closed.")
            self._update_manual_button_state()

        if self._login_dialog:
            try:
                self._login_dialog.destroy()
            except tk.TclError:
                pass
        self._login_dialog = None
        self._login_output_textbox = None

    def _stop_codex_login_process(self) -> bool:
        process = self._login_process
        if not process or process.poll() is not None:
            return False

        try:
            process.terminate()
            return True
        except OSError:
            return False

    def cancel_codex_login(self) -> None:
        if self._stop_codex_login_process():
            self.status_var.set("Codex login cancelled.")
            self._append_login_output("\nLogin cancelled.\n")
        self._login_in_progress = False
        self._login_process = None
        self._update_manual_button_state()

    def _append_login_output(self, text: str) -> None:
        textbox = self._login_output_textbox
        if not textbox:
            return

        clean_text = self._strip_terminal_sequences(text)
        try:
            textbox.configure(state="normal")
            textbox.insert("end", clean_text)
            textbox.see("end")
            textbox.configure(state="disabled")
        except tk.TclError:
            pass

    def _strip_terminal_sequences(self, text: str) -> str:
        text = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)
        text = re.sub(r"\x1b\][^\x07]*(?:\x07|\x1b\\)", "", text)
        return text

    def _handle_codex_login_output_line(self, line: str) -> None:
        clean_line = self._strip_terminal_sequences(line)
        self._append_login_output(clean_line)
        match = re.search(r"https?://[^\s]+", clean_line)
        if not match:
            return

        url = match.group(0).rstrip(".,);]")
        if not url or url == self._login_opened_url:
            return

        self._login_opened_url = url
        try:
            webbrowser.open(url)
            self.status_var.set("Opened Codex login URL. Complete login in the browser.")
        except Exception as error:
            self._append_login_output(f"Failed to open browser: {error}\n")

    def _bg_codex_login(self) -> None:
        command = self._codex_login_command()
        if not command:
            if sys.version_info < (3, 10):
                message = (
                    "Bundled Codex login requires Python 3.10+. Recreate the "
                    "venv with Python 3.10 or newer, or set CODEX_MONITOR_CODEX_BIN."
                )
            else:
                message = (
                    "Codex runtime was not found. Rebuild the app with the "
                    "openai-codex dependency, or set CODEX_MONITOR_CODEX_BIN."
                )
            self.root.after(0, lambda m=message: self._finish_codex_login(m))
            return

        login_codex_home: Optional[str] = None
        popen_kwargs = {}
        if sys.platform == "win32" and hasattr(subprocess, "CREATE_NO_WINDOW"):
            popen_kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW

        try:
            login_codex_home = self.auth_file_service.create_login_codex_home()
            login_auth_path = self.auth_file_service.active_auth_path_for_home(
                login_codex_home,
            )
            self.root.after(
                0,
                lambda: self._append_login_output(
                    "Using isolated Codex login session.\n",
                ),
            )

            process_env = os.environ.copy()
            process_env["CODEX_HOME"] = login_codex_home
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                stdin=subprocess.DEVNULL,
                text=True,
                bufsize=1,
                env=process_env,
                **popen_kwargs,
            )
            self._login_process = process
            if process.stdout:
                for line in process.stdout:
                    self.root.after(
                        0,
                        lambda captured_line=line: self._handle_codex_login_output_line(
                            captured_line,
                        ),
                    )
            return_code = process.wait()
            if return_code == 0:
                message = self._import_isolated_codex_login(login_auth_path)
            else:
                message = f"Codex login exited with code {return_code}."
        except FileNotFoundError:
            message = "Codex CLI was not found on PATH."
        except Exception as error:
            message = f"Codex login failed: {error}"
        finally:
            self.auth_file_service.remove_login_codex_home(login_codex_home)

        self.root.after(0, lambda m=message: self._finish_codex_login(m))

    def _import_isolated_codex_login(self, login_auth_path: str) -> str:
        if not os.path.exists(login_auth_path):
            return (
                "Codex login finished, but isolated auth.json was not created. "
                "Set Codex credential storage to file-based auth and try again."
            )

        snapshot = self.auth_file_service.load_snapshot_from_path(login_auth_path)
        jwt = snapshot.get("tokens", {}).get("access_token")
        if not jwt:
            return "Codex login finished, but isolated auth.json has no access_token."

        try:
            response = self._fetch_usage(jwt)
            email = self.state.apply_usage_response(response, jwt, activate=False)
        except Exception as error:
            self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
            return f"Codex login finished, but quota fetch failed: {error}"

        if not email:
            self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
            return "Codex login finished, but quota response did not include an email."

        try:
            self.auth_file_service.backup_auth_from_path(email, login_auth_path)
        except Exception as backup_error:
            print(
                "[Safe Error Log] Failed to save isolated login backup for "
                f"{email}: {backup_error}"
            )

        self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
        self._login_added_email = email
        return f"Codex login finished. Added {email}; active Codex account was not changed."

    def _finish_codex_login(self, message: str) -> None:
        if not self._login_in_progress and self._login_process is None:
            return

        self._login_in_progress = False
        self._login_process = None
        self.status_var.set(message)
        self._append_login_output(f"\n{message}\n")
        added_email = self._login_added_email
        self._login_added_email = None
        if added_email:
            self._close_codex_login_dialog()
        self._update_manual_button_state()

    def _schedule_next_update_check(self) -> None:
        if self._update_check_timer_id:
            self.root.after_cancel(self._update_check_timer_id)

        self._update_check_timer_id = self.root.after(
            UPDATE_CHECK_INTERVAL_SECONDS * 1000,
            self.check_for_updates_silently,
        )

    def _set_update_button_visibility(self) -> None:
        if not self.update_button or not self.theme_button:
            return

        show_button = (
            self._available_release is not None
            or self._update_prepare_in_progress
            or self._update_in_progress
        )
        if show_button:
            self.update_button.grid()
            self.theme_button.grid_configure(column=13)
        else:
            self.update_button.grid_remove()
            self.theme_button.grid_configure(column=12)

    def _animate_spinner(self) -> None:
        if not self.manual_button or self._pending_fetches == 0:
            self._spinner_timer_id = None
            return
            
        frame = self._spinner_frames[self._spinner_index]
        self._spinner_index = (self._spinner_index + 1) % len(self._spinner_frames)
        
        self.manual_button.configure(
            state="disabled",
            text=frame,
            font=ctk.CTkFont(size=14, weight="bold"),
        )
        self._force_square_button(self.manual_button, self.TOOLBAR_BUTTON_SIZE)
        
        self._spinner_timer_id = self.root.after(80, self._animate_spinner)

    def _update_manual_button_state(self) -> None:
        if not self.manual_button:
            return

        if self._pending_fetches > 0:
            if self._spinner_timer_id is None:
                self._spinner_index = 0
                self._animate_spinner()
        else:
            if self._spinner_timer_id is not None:
                self.root.after_cancel(self._spinner_timer_id)
                self._spinner_timer_id = None
            refresh_text = self._material_icon_text("refresh")
            self.manual_button.configure(
                state="normal",
                text=refresh_text or "⟳",
                font=self._material_icon_font(18),
            )
            self._force_square_button(self.manual_button, self.TOOLBAR_BUTTON_SIZE)

        if self.update_button:
            if self._update_in_progress:
                self.update_button.configure(
                    state="disabled",
                    text="Updating...",
                )
            elif self._update_prepare_in_progress:
                self.update_button.configure(
                    state="disabled",
                    text="Preparing...",
                )
            elif self._available_release:
                self.update_button.configure(
                    state="normal",
                    text="Update",
                )
            else:
                self.update_button.configure(
                    state="normal",
                    text="Update",
                )

        check_update_button = getattr(self, "check_update_button", None)
        if check_update_button:
            check_update_text = self._material_icon_text("update")
            if self._update_check_in_progress or self._update_in_progress:
                check_update_button.configure(
                    state="disabled",
                    text=check_update_text or "↻",
                    font=self._material_icon_font(18),
                )
            else:
                check_update_button.configure(
                    state="normal",
                    text=check_update_text or "↻",
                    font=self._material_icon_font(18),
                )
            self._force_square_button(check_update_button, self.TOOLBAR_BUTTON_SIZE)

        if self.login_button:
            login_text = self._material_icon_text("person_add")
            self.login_button.configure(
                state="disabled" if self._login_in_progress else "normal",
                text=login_text or "+",
                font=self._material_icon_font(18),
            )
            self._force_square_button(self.login_button, self.TOOLBAR_BUTTON_SIZE)

        current_email = self.state.current_account_email
        if self.auto_fetch_menu:
            if current_email:
                self.auto_fetch_menu.configure(state="normal")
                self.auto_fetch_menu.set(self.state.get_display_auto_fetch(current_email))
            else:
                self.auto_fetch_menu.configure(state="disabled")
                self.auto_fetch_menu.set("None")

        if self.auto_fetch_label:
            self.auto_fetch_label.configure(
                text_color=self._theme_tokens()["muted"],
            )

        self._set_update_button_visibility()

        if self.theme_button:
            self.theme_button.configure(
                text=self._appearance_toggle_icon(),
                font=self._material_icon_font(18),
            )
            self._force_square_button(self.theme_button, self.TOOLBAR_BUTTON_SIZE)

        self._update_show_archived_button()

    def _begin_fetch(self, status_message: str) -> None:
        self._pending_fetches += 1
        self.status_var.set(status_message)
        self._update_manual_button_state()

    def _finish_fetch(self, status_message: str) -> None:
        self._pending_fetches = max(self._pending_fetches - 1, 0)
        self.status_var.set(status_message)
        self._update_manual_button_state()

    def save_auto_fetch_value(self, email: str, new_value: str) -> None:
        if self.state.save_auto_fetch_value(email, new_value):
            self.refresh_ui()
            self.status_var.set(f"Auto-fetch set to {new_value}.")
            self._notify_auto_fetch_setting_changed(email, new_value)

    def save_current_auto_fetch_value(self, new_value: str) -> None:
        email = self.state.current_account_email
        if not email:
            if self.auto_fetch_menu:
                self.auto_fetch_menu.set("None")
            self.status_var.set("No active account for auto-fetch.")
            return

        self.save_auto_fetch_value(email, new_value)

    def _build_account_row(
        self,
        email: str,
        quota_lines: List[str],
        reset_lines: List[str],
        is_current: bool,
        is_archived: bool,
        has_auth_backup: bool,
        index: int,
    ) -> None:
        tokens = self._theme_tokens()
        row_bg = tokens["current_bg"] if is_current and not is_archived else (
            tokens["row_even"] if index % 2 == 0 else tokens["row_odd"]
        )
        row_text = tokens["muted"] if is_archived else (
            tokens["current_fg"] if is_current else tokens["table_fg"]
        )
        if is_current and is_archived:
            email_display = f"{email}   ACTIVE, ARCHIVED"
        elif is_current:
            email_display = f"{email}   ACTIVE"
        elif is_archived:
            email_display = f"{email}   ARCHIVED"
        else:
            email_display = email

        row = ctk.CTkFrame(
            self.accounts_rows_frame,
            fg_color=row_bg,
            corner_radius=12,
            border_width=1,
            border_color=tokens["current_border"] if is_current and not is_archived else tokens["row_border"],
        )
        row.grid(
            row=index,
            column=0,
            sticky="ew",
            padx=1,
            pady=self.TABLE_ROW_GAP_Y,
        )
        self._configure_account_columns(row)
        columns = self._account_column_layout()

        email_cell = ctk.CTkFrame(row, fg_color="transparent")
        email_cell.grid(
            row=0,
            column=columns["email"],
            sticky="ew",
            padx=10,
            pady=self.TABLE_ROW_PAD_Y,
        )
        email_cell.grid_columnconfigure(0, weight=1)
        email_label = ctk.CTkLabel(
            email_cell,
            text=email_display,
            anchor="w",
            font=ctk.CTkFont(size=11, weight="bold" if is_current and not is_archived else "normal"),
            text_color=row_text,
        )
        email_label.grid(row=0, column=0, sticky="ew")

        copy_text = self._material_icon_text("copy")
        copy_button = ctk.CTkButton(
            email_cell,
            text=copy_text or "⧉",
            command=lambda account_email=email: self.copy_account_email(account_email),
            corner_radius=self.ROW_BUTTON_RADIUS,
            height=self.ROW_BUTTON_SIZE,
            width=self.ROW_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(16),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        copy_button.grid(row=0, column=1, sticky="e", padx=(6, 0))
        self._force_square_button(copy_button, self.ROW_BUTTON_SIZE)
        self._attach_tooltip(copy_button, "Copy email", row_tooltip=True)

        self._build_stacked_value_label(
            row,
            quota_lines,
            row_text,
            columns["quota"],
            anchor="w",
            bold=is_current,
        )
        self._build_stacked_value_label(
            row,
            reset_lines,
            row_text,
            columns["reset"],
            anchor="w",
        )

        actions_cell = ctk.CTkFrame(row, fg_color="transparent")
        actions_cell.grid(
            row=0,
            column=columns["action"],
            sticky="e",
            padx=10,
            pady=self.TABLE_ROW_PAD_Y,
        )
        action_column = 0

        if has_auth_backup and not is_current:
            fetch_text = self._material_icon_text("refresh")
            fetch_button = ctk.CTkButton(
                actions_cell,
                text=fetch_text or "⟳",
                command=lambda account_email=email: self.fetch_backup_account_quota(
                    account_email
                ),
                corner_radius=self.ROW_BUTTON_RADIUS,
                height=self.ROW_BUTTON_SIZE,
                width=self.ROW_BUTTON_SIZE,
                border_spacing=0,
                anchor="center",
                font=self._material_icon_font(16),
                fg_color=tokens["control_bg"],
                hover_color=tokens["control_hover"],
                text_color=tokens["control_fg"],
            )
            fetch_button.grid(row=0, column=action_column, sticky="e", padx=(0, 5))
            self._force_square_button(fetch_button, self.ROW_BUTTON_SIZE)
            self._attach_tooltip(fetch_button, "Fetch quota", row_tooltip=True)
            action_column += 1

        if has_auth_backup and not is_current:
            switch_text = self._material_icon_text("swap_horiz")
            switch_button = ctk.CTkButton(
                actions_cell,
                text=switch_text or "⇄",
                command=lambda account_email=email: self.switch_account(account_email),
                corner_radius=self.ROW_BUTTON_RADIUS,
                height=self.ROW_BUTTON_SIZE,
                width=self.ROW_BUTTON_SIZE,
                border_spacing=0,
                anchor="center",
                font=self._material_icon_font(16),
                fg_color=tokens["control_bg"],
                hover_color=tokens["control_hover"],
                text_color=tokens["control_fg"],
            )
            switch_button.grid(row=0, column=action_column, sticky="e", padx=(0, 5))
            self._force_square_button(switch_button, self.ROW_BUTTON_SIZE)
            self._attach_tooltip(
                switch_button,
                "Switch Account",
                row_tooltip=True,
            )
            action_column += 1

        archive_text = self._material_icon_text("unarchive" if is_archived else "archive")
        archive_button = ctk.CTkButton(
            actions_cell,
            text=archive_text or ("↑" if is_archived else "↓"),
            command=lambda account_email=email, account_archived=is_archived: self.toggle_archive_account(
                account_email,
                account_archived,
            ),
            corner_radius=self.ROW_BUTTON_RADIUS,
            height=self.ROW_BUTTON_SIZE,
            width=self.ROW_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(16),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        archive_button.grid(row=0, column=action_column, sticky="e", padx=(0, 5))
        self._force_square_button(archive_button, self.ROW_BUTTON_SIZE)
        self._attach_tooltip(
            archive_button,
            "Unarchive account" if is_archived else "Archive account",
            row_tooltip=True,
        )
        action_column += 1

        reset_text = self._material_icon_text("history")
        reset_button = ctk.CTkButton(
            actions_cell,
            text=reset_text or "⏱",
            command=lambda account_email=email: self._show_reset_credits_modal(account_email),
            corner_radius=self.ROW_BUTTON_RADIUS,
            height=self.ROW_BUTTON_SIZE,
            width=self.ROW_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(16),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        reset_button.grid(row=0, column=action_column, sticky="e", padx=(0, 5))
        self._force_square_button(reset_button, self.ROW_BUTTON_SIZE)
        self._attach_tooltip(
            reset_button,
            self._reset_credits_tooltip_text(email),
            row_tooltip=True,
        )
        action_column += 1

        remove_text = self._material_icon_text("delete")
        remove_button = ctk.CTkButton(
            actions_cell,
            text=remove_text or "×",
            command=lambda account_email=email: self.remove_account(account_email),
            corner_radius=self.ROW_BUTTON_RADIUS,
            height=self.ROW_BUTTON_SIZE,
            width=self.ROW_BUTTON_SIZE,
            border_spacing=0,
            anchor="center",
            font=self._material_icon_font(16),
            fg_color=tokens["control_bg"],
            hover_color=tokens["control_hover"],
            text_color=tokens["control_fg"],
        )
        remove_button.grid(row=0, column=action_column, sticky="e")
        self._force_square_button(remove_button, self.ROW_BUTTON_SIZE)
        self._attach_tooltip(remove_button, "Remove account", row_tooltip=True)

    def initial_fetch_on_startup(self) -> None:
        self.status_var.set("Checking current auth.json on startup...")
        self.process_auth_file()

    def on_file_changed(self) -> None:
        self.root.after(0, self._schedule_auth_refresh)

    def _get_auth_file_signature(self) -> Optional[Tuple[int, int]]:
        try:
            stat_result = os.stat(self.auth_file_service.auth_file_path)
        except (FileNotFoundError, NotADirectoryError):
            return None
        except OSError:
            return None

        return (stat_result.st_mtime_ns, stat_result.st_size)

    def _schedule_next_auth_poll(self) -> None:
        if self._auth_poll_timer_id:
            self.root.after_cancel(self._auth_poll_timer_id)
        self._auth_poll_timer_id = self.root.after(
            self.AUTH_SIGNATURE_POLL_MS,
            self._poll_auth_file_state,
        )

    def _poll_auth_file_state(self) -> None:
        self._auth_poll_timer_id = None
        current_signature = self._get_auth_file_signature()
        if current_signature != self._last_seen_auth_signature:
            self._last_seen_auth_signature = current_signature
            self._schedule_auth_refresh()
        self._schedule_next_auth_poll()

    def _schedule_auth_refresh(self) -> None:
        if self._auth_retry_job:
            self.root.after_cancel(self._auth_retry_job)
            self._auth_retry_job = None

        self._reset_missing_token_retry_state()

        if self._auth_change_job:
            self.root.after_cancel(self._auth_change_job)

        self._auth_change_job = self.root.after(
            self.AUTH_EVENT_SETTLE_MS,
            self._handle_file_changed,
        )

    def _handle_file_changed(self) -> None:
        self._auth_change_job = None
        self._auth_retry_attempts = 0
        current_signature = self._get_auth_file_signature()
        self._last_seen_auth_signature = current_signature
        if (
            self._suppressed_auth_signature is not None
            and self._suppressed_auth_signature == current_signature
        ):
            self._suppressed_auth_signature = None
            self.status_var.set("auth.json switched. Quota refresh already started.")
            return
        self._suppressed_auth_signature = None

        self.status_var.set("Auth file changed. Fetching new quota...")
        self.process_auth_file()

    def _schedule_auth_parse_retry(self) -> None:
        if self._auth_retry_attempts >= self.MAX_AUTH_PARSE_RETRIES:
            self.status_var.set(
                "auth.json is still invalid. Waiting for the next file change."
            )
            self._auth_retry_job = None
            return

        self._auth_retry_attempts += 1
        self.status_var.set(
            f"auth.json is mid-write. Retrying read ({self._auth_retry_attempts}/{self.MAX_AUTH_PARSE_RETRIES})..."
        )
        self._auth_retry_job = self.root.after(
            self.AUTH_PARSE_RETRY_MS,
            self._retry_process_auth_file,
        )

    def _retry_process_auth_file(self) -> None:
        self._auth_retry_job = None
        self.process_auth_file()

    def _reset_auth_retry_state(self) -> None:
        self._auth_retry_attempts = 0
        if self._auth_retry_job:
            self.root.after_cancel(self._auth_retry_job)
            self._auth_retry_job = None

    def _schedule_missing_token_retry(self) -> bool:
        if self._missing_token_retry_attempts >= self.MAX_MISSING_TOKEN_RETRIES:
            self._missing_token_retry_job = None
            return False

        self._missing_token_retry_attempts += 1
        self.status_var.set(
            "auth.json has no access_token yet. "
            f"Retrying read ({self._missing_token_retry_attempts}/{self.MAX_MISSING_TOKEN_RETRIES})..."
        )
        self._missing_token_retry_job = self.root.after(
            self.MISSING_TOKEN_RETRY_MS,
            self._retry_missing_token_state,
        )
        return True

    def _retry_missing_token_state(self) -> None:
        self._missing_token_retry_job = None
        self.process_auth_file()

    def _reset_missing_token_retry_state(self) -> None:
        self._missing_token_retry_attempts = 0
        if self._missing_token_retry_job:
            self.root.after_cancel(self._missing_token_retry_job)
            self._missing_token_retry_job = None

    def _auth_file_has_access_token(self) -> bool:
        if not self.auth_file_service.auth_file_exists():
            return False

        try:
            return bool(self.auth_file_service.load_access_token())
        except (json.JSONDecodeError, OSError, ValueError, TypeError):
            return False

    def _snapshot_refresh_changed(self, snapshot: AuthFileSnapshot) -> bool:
        current_access_token = snapshot.get("tokens", {}).get("access_token")
        current_refresh_marker = snapshot.get("last_refresh")
        token_changed = (
            self._last_seen_access_token is not None
            and bool(current_access_token)
            and current_access_token != self._last_seen_access_token
        )
        refresh_marker_changed = (
            self._last_auth_refresh_marker is not None
            and bool(current_refresh_marker)
            and current_refresh_marker != self._last_auth_refresh_marker
        )
        return token_changed or refresh_marker_changed

    def _remember_auth_snapshot(self, snapshot: AuthFileSnapshot) -> None:
        self._last_seen_access_token = snapshot.get("tokens", {}).get("access_token")
        self._last_auth_refresh_marker = snapshot.get("last_refresh")

    def _notify_user(self, title: str, message: str) -> None:
        sanitized_title = title.replace('"', "'")
        sanitized_message = message.replace('"', "'")

        try:
            subprocess.run(
                [
                    "osascript",
                    "-e",
                    (
                        f'display notification "{sanitized_message}" '
                        f'with title "{sanitized_title}"'
                    ),
                ],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except Exception:
            pass

    def _notify_auth_refresh_detected(self, snapshot: AuthFileSnapshot) -> None:
        refresh_marker = snapshot.get("last_refresh")
        subtitle = (
            f"last_refresh={refresh_marker}" if refresh_marker else "auth.json changed"
        )
        self._notify_user(
            APP_TITLE,
            f"Detected Codex auth refresh: {subtitle}",
        )

    def _notify_auto_fetch_setting_changed(self, email: str, interval_label: str) -> None:
        if interval_label == "None":
            message = "Auto-fetch turned off."
        else:
            message = f"Auto-fetch enabled for the active account: every {interval_label}."
        self._notify_user(APP_TITLE, message)

    def _notify_auto_fetch_triggered(self, email: str, interval_label: str) -> None:
        if interval_label and interval_label != "None":
            message = f"Auto-fetch triggered for {email} ({interval_label})."
        else:
            message = f"Auto-fetch triggered for {email}."
        self._notify_user(APP_TITLE, message)

    def manual_fetch(self) -> None:
        self.status_var.set("Manual fetch initiated...")
        self.process_auth_file()

    def check_for_updates_silently(self) -> None:
        if self._update_check_in_progress or self._update_in_progress:
            return

        self._manual_update_check_requested = False
        self._update_check_in_progress = True
        self._update_manual_button_state()
        threading.Thread(target=self._bg_check_for_updates, daemon=True).start()

    def check_for_updates_manually(self) -> None:
        if self._update_in_progress:
            self.status_var.set("Update installation is already in progress.")
            return

        self._manual_update_check_requested = True
        self.status_var.set("Checking for updates...")
        if self._update_check_in_progress:
            self._update_manual_button_state()
            return

        self._update_check_in_progress = True
        self._update_manual_button_state()
        threading.Thread(target=self._bg_check_for_updates, daemon=True).start()

    def _bg_check_for_updates(self) -> None:
        release: Optional[ReleaseInfo] = None
        error_message: Optional[str] = None

        try:
            latest_release = fetch_latest_release()
            if is_newer_version(latest_release.version, APP_VERSION):
                release = latest_release
        except urllib.error.HTTPError as error:
            error_message = f"Update check failed with HTTP {error.code}."
        except (urllib.error.URLError, TimeoutError) as error:
            error_message = f"Network Error while checking for updates: {getattr(error, 'reason', str(error))}"
        except UpdateError as error:
            error_message = f"Update check error: {error}"
        except Exception as error:
            error_message = f"Unexpected update check error: {error}"

        self.root.after(
            0,
            lambda r=release, e=error_message: self._finish_update_check(r, e),
        )

    def _finish_update_check(
        self,
        release: Optional[ReleaseInfo],
        error_message: Optional[str],
    ) -> None:
        was_manual_check = getattr(self, "_manual_update_check_requested", False)
        self._manual_update_check_requested = False
        self._update_check_in_progress = False
        if error_message:
            resolved_release = self._available_release
        else:
            resolved_release = release

        self._available_release = resolved_release

        if was_manual_check and error_message:
            self.status_var.set(error_message)
        elif was_manual_check and resolved_release:
            self.status_var.set(
                f"Update available: v{resolved_release.version}. Preparing download..."
            )
            self._notify_user(
                APP_TITLE,
                f"Update v{resolved_release.version} is available.",
            )
        elif was_manual_check:
            self.status_var.set(f"You're already on the latest version (v{APP_VERSION}).")
        elif resolved_release:
            current_message = self.status_var.get()
            if (
                "Update available:" in current_message
                or current_message.startswith("Watching: ")
            ):
                self.status_var.set(
                    f"Update available: v{resolved_release.version}. Preparing download..."
                )
        elif error_message and self.status_var.get().startswith("Watching: "):
            self.status_var.set(error_message)

        self._update_manual_button_state()
        if resolved_release:
            self._start_update_prepare(resolved_release)
        elif not error_message:
            self._prepared_update = None
        self._schedule_next_update_check()

    def _prepared_update_matches(self, release: ReleaseInfo) -> bool:
        return (
            self._prepared_update is not None
            and self._prepared_update[0].tag_name == release.tag_name
        )

    def _start_update_prepare(self, release: ReleaseInfo) -> None:
        if (
            self._update_prepare_in_progress
            or self._update_in_progress
            or self._prepared_update_matches(release)
        ):
            return

        self._update_prepare_in_progress = True
        self._prepared_update = None
        self._update_manual_button_state()
        threading.Thread(
            target=self._bg_prepare_update,
            args=(release,),
            daemon=True,
        ).start()

    def _bg_prepare_update(self, release: ReleaseInfo) -> None:
        prepared: Optional[Tuple[ReleaseInfo, str, str, str]] = None
        error_message: Optional[str] = None

        try:
            source_app, target_app, temp_root = prepare_update(release)
            prepared = (release, source_app, target_app, temp_root)
        except urllib.error.HTTPError as error:
            error_message = f"Update check failed with HTTP {error.code}."
        except (urllib.error.URLError, TimeoutError) as error:
            error_message = f"Network Error while preparing update: {getattr(error, 'reason', str(error))}"
        except subprocess.CalledProcessError:
            error_message = "Update download succeeded, but extracting the app failed."
        except UpdateError as error:
            error_message = f"Update error: {error}"
        except Exception as error:
            error_message = f"Unexpected update prepare error: {error}"

        self.root.after(
            0,
            lambda p=prepared, e=error_message: self._finish_update_prepare(p, e),
        )

    def _finish_update_prepare(
        self,
        prepared: Optional[Tuple[ReleaseInfo, str, str, str]],
        error_message: Optional[str],
    ) -> None:
        self._update_prepare_in_progress = False
        if prepared and self._available_release:
            release = prepared[0]
            if release.tag_name == self._available_release.tag_name:
                self._prepared_update = prepared
                self.status_var.set(
                    f"Update v{release.version} is downloaded and ready to install."
                )
        elif error_message and self._available_release:
            self.status_var.set(error_message)

        self._update_manual_button_state()

    def update_application(self) -> None:
        if (
            self._update_in_progress
            or self._update_prepare_in_progress
            or not self._available_release
        ):
            return

        self._update_in_progress = True
        self.status_var.set(
            f"Installing v{self._available_release.version} from v{APP_VERSION}..."
        )
        self._update_manual_button_state()
        threading.Thread(target=self._bg_update_application, daemon=True).start()

    def _bg_update_application(self) -> None:
        should_close = False

        try:
            if not self._available_release:
                message = f"You're already on the latest version (v{APP_VERSION})."
            else:
                release = self._available_release
                if self._prepared_update_matches(release) and self._prepared_update:
                    _release, source_app, target_app, temp_root = self._prepared_update
                else:
                    source_app, target_app, temp_root = prepare_update(release)
                install_update_and_restart(source_app, target_app, temp_root)
                message = (
                    f"Installing v{release.version} and reopening from {target_app}..."
                )
                should_close = True
        except urllib.error.HTTPError as error:
            message = f"Update check failed with HTTP {error.code}."
        except (urllib.error.URLError, TimeoutError) as error:
            message = f"Network Error while updating: {getattr(error, 'reason', str(error))}"
        except subprocess.CalledProcessError:
            message = "Update download succeeded, but extracting the app failed."
        except UpdateError as error:
            message = f"Update error: {error}"
        except Exception as error:
            message = f"Unexpected update error: {error}"

        self.root.after(
            0,
            lambda m=message, close_after=should_close: self._finish_update(
                m,
                close_after,
            ),
        )

    def _finish_update(self, status_message: str, close_after: bool = False) -> None:
        self._update_in_progress = False
        if close_after:
            self._available_release = None
            self._prepared_update = None
        self.status_var.set(status_message)
        self._update_manual_button_state()
        if close_after:
            self.root.after(500, self.on_closing)

    def process_auth_file(self) -> None:
        if not self.auth_file_service.auth_file_exists():
            self._reset_auth_retry_state()
            self._reset_missing_token_retry_state()
            self._last_seen_auth_signature = None
            self._last_auth_refresh_marker = None
            self._last_seen_access_token = None
            email = self.state.current_account_email
            jwt = self.state.get_latest_jwt_for_fetch(email)

            if jwt:
                account_label = email or "current account"
                self._begin_fetch(
                    f"Logout detected. Taking final snapshot for {account_label}..."
                )
                threading.Thread(
                    target=self._final_snapshot_and_clear,
                    args=(email, jwt),
                    daemon=True,
                ).start()
            else:
                if self.state.clear_session_credentials():
                    self.root.after_idle(self.refresh_ui)
                self.status_var.set("auth.json was removed. Logged out from Codex.")
            return

        try:
            snapshot = self.auth_file_service.load_snapshot()
            self._latest_account_id = snapshot.get("tokens", {}).get("account_id")
            jwt = snapshot.get("tokens", {}).get("access_token")
            if jwt:
                self._reset_auth_retry_state()
                self._reset_missing_token_retry_state()
                refresh_detected = self._snapshot_refresh_changed(snapshot)
                self._remember_auth_snapshot(snapshot)
                self.state.remember_auth_jwt(jwt)
                if refresh_detected:
                    refresh_marker = snapshot.get("last_refresh")
                    status_message = (
                        f"Detected auth refresh at {refresh_marker}. Fetching new quota..."
                        if refresh_marker
                        else "Detected auth token rotation. Fetching new quota..."
                    )
                    self._notify_auth_refresh_detected(snapshot)
                else:
                    status_message = "Fetching quota from current auth.json..."
                self._begin_fetch(status_message)
                threading.Thread(
                    target=self._bg_fetch_single,
                    args=(None, jwt),
                    daemon=True,
                ).start()
            else:
                self._reset_auth_retry_state()
                if self._schedule_missing_token_retry():
                    return

                self._reset_missing_token_retry_state()
                self._reset_auth_retry_state()
                email = self.state.current_account_email
                latest_jwt = self.state.get_latest_jwt_for_fetch(email)
                if latest_jwt:
                    account_label = email or "current account"
                    self._begin_fetch(
                        f"Logout detected. Taking final snapshot for {account_label}..."
                    )
                    threading.Thread(
                        target=self._final_snapshot_and_clear,
                        args=(email, latest_jwt),
                        daemon=True,
                    ).start()
                else:
                    if self.state.clear_session_credentials():
                        self.root.after_idle(self.refresh_ui)
                    self.status_var.set(
                        "No access_token found in auth.json. Logged out from Codex."
                    )
        except json.JSONDecodeError:
            self._schedule_auth_parse_retry()
        except Exception as error:
            self._reset_auth_retry_state()
            self._reset_missing_token_retry_state()
            self.status_var.set(f"Failed to parse auth.json: {error}")

    def _fetch_usage(self, jwt: str) -> dict:
        return self.api_client.fetch_usage(jwt)

    def _sync_current_auth_backup(self, email: str) -> None:
        try:
            self.auth_file_service.backup_current_auth(email)
        except Exception as backup_error:
            print(
                "[Safe Error Log] Failed to sync auth backup for "
                f"{email}: {backup_error}"
            )

    def _bg_fetch_single(self, expected_email: Optional[str], jwt: str) -> None:
        if not jwt:
            self.root.after(0, lambda: self._finish_fetch("No token available for fetch."))
            return

        try:
            response = self._fetch_usage(jwt)
            email = self.state.apply_usage_response(response, jwt)
            if email:
                if expected_email and email != expected_email:
                    print(
                        "[Safe Error Log] Expected quota for "
                        f"{expected_email}, but token belongs to {email}."
                    )
                self._sync_current_auth_backup(email)
                try:
                    account_id = response.get("account_id") or getattr(self, "_latest_account_id", None)
                    if account_id:
                        reset_payload = self.api_client.fetch_reset_credits(jwt, account_id)
                        self.state.apply_reset_credits(email, reset_payload)
                except Exception as exc:
                    print(f"[Safe Error Log] Failed to fetch reset credits for {email}: {exc}")
                self.root.after(0, self.refresh_ui)
                message = f"Successfully updated quota for {email}."
            else:
                message = "Warning: Could not find email or reset_at in API response."
                print(f"[Safe Error Log] {message}")

        except urllib.error.HTTPError as error:
            message = f"HTTP Error {error.code}. Token may be expired."
            print(f"[Safe Error Log] {message}")

        except (urllib.error.URLError, TimeoutError) as error:
            message = f"Network Error: {getattr(error, 'reason', str(error))}"
            if "CERTIFICATE_VERIFY_FAILED" in str(getattr(error, "reason", "")):
                message = (
                    "SSL Error: Run 'Install Certificates.command' in Mac Python "
                    "folder, or run 'pip install certifi'."
                )

            print(f"[Safe Error Log] {message}")

        except Exception as error:
            message = f"Unknown error: {str(error)}"
            print(f"[Safe Error Log] Exception triggered: {message}")

        self.root.after(0, lambda m=message: self._finish_fetch(m))

    def _load_refreshed_backup_access_token(
        self,
        email: str,
        *,
        force: bool = False,
        now: Optional[float] = None,
    ) -> Optional[str]:
        snapshot = self.auth_file_service.refresh_backup_if_due(
            email,
            self.auth_refresh_client,
            force=force,
            now=now,
        )
        jwt = snapshot.get("tokens", {}).get("access_token")
        if isinstance(jwt, str) and jwt:
            self.state.remember_account_jwt(email, jwt)
            return jwt
        return None

    def _bg_fetch_backup_account(self, expected_email: str) -> None:
        jwt: Optional[str] = None
        try:
            jwt = self._load_refreshed_backup_access_token(expected_email)
            if not jwt:
                message = f"Auth backup for {expected_email} has no access_token."
                self.root.after(0, lambda m=message: self._finish_fetch(m))
                return

            response = self._fetch_usage(jwt)
            email = self.state.apply_usage_response(
                response,
                jwt,
                activate=False,
            )
            if email:
                self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
                try:
                    account_id = response.get("account_id") or getattr(self, "_latest_account_id", None)
                    if account_id:
                        reset_payload = self.api_client.fetch_reset_credits(jwt, account_id)
                        self.state.apply_reset_credits(email, reset_payload)
                except Exception as exc:
                    print(f"[Safe Error Log] Failed to fetch reset credits for {email}: {exc}")
                if email == expected_email:
                    message = f"Successfully updated quota for {email}."
                else:
                    message = (
                        f"Updated quota for {email}; backup was selected from "
                        f"{expected_email}."
                    )
            else:
                message = "Warning: Could not find email or reset_at in API response."
                print(f"[Safe Error Log] {message}")

        except urllib.error.HTTPError as error:
            if jwt is None:
                self.auth_file_service.remove_backup(expected_email)
                self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
                message = (
                    f"HTTP Error {error.code}. Removed auth backup for "
                    f"{expected_email} because token refresh failed."
                )
            elif error.code == 401:
                message = self._retry_backup_fetch_after_401(expected_email, jwt)
            else:
                message = f"HTTP Error {error.code}. Backup fetch failed."
            print(f"[Safe Error Log] {message}")

        except (urllib.error.URLError, TimeoutError) as error:
            message = f"Network Error: {getattr(error, 'reason', str(error))}"
            if "CERTIFICATE_VERIFY_FAILED" in str(getattr(error, "reason", "")):
                message = (
                    "SSL Error: Run 'Install Certificates.command' in Mac Python "
                    "folder, or run 'pip install certifi'."
                )
            print(f"[Safe Error Log] {message}")

        except Exception as error:
            if jwt is None:
                self.auth_file_service.remove_backup(expected_email)
                self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
                message = (
                    f"Removed auth backup for {expected_email} because token "
                    f"refresh failed: {str(error)}"
                )
            else:
                message = f"Unknown error: {str(error)}"
            print(f"[Safe Error Log] Exception triggered: {message}")

        self.root.after(0, lambda m=message: self._finish_fetch(m))

    def _retry_backup_fetch_after_401(
        self,
        expected_email: str,
        failed_jwt: Optional[str],
    ) -> str:
        try:
            refreshed_jwt = self._load_refreshed_backup_access_token(
                expected_email,
                force=True,
            )
            if not refreshed_jwt:
                raise ValueError("refresh did not return an access_token")

            response = self._fetch_usage(refreshed_jwt)
            email = self.state.apply_usage_response(
                response,
                refreshed_jwt,
                activate=False,
            )
            self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
            if email == expected_email:
                return f"Refreshed auth backup and updated quota for {email}."
            if email:
                return (
                    f"Refreshed auth backup and updated quota for {email}; "
                    f"backup was selected from {expected_email}."
                )
            return (
                "Refreshed auth backup after HTTP 401, but could not parse "
                "the quota response."
            )
        except Exception:
            removed = self.auth_file_service.remove_backup(expected_email)
            self.root.after(0, lambda: self.refresh_ui(skip_auto_fetch=True))
            if removed:
                return (
                    "HTTP Error 401. Backup token failed and refresh did not "
                    f"succeed; removed auth backup for {expected_email}."
                )
            return (
                "HTTP Error 401. Backup token failed and refresh did not succeed; "
                f"no auth backup remained for {expected_email}."
            )

    def _final_snapshot_and_clear(self, email: Optional[str], jwt: str) -> None:
        account_label = email or "current account"
        try:
            response = self._fetch_usage(jwt)
            updated_email = self.state.apply_usage_response(response, jwt)
            if updated_email:
                message = (
                    f"Logged out. Final snapshot successfully saved for {updated_email}."
                )
            else:
                message = (
                    f"Logged out. Could not parse final snapshot for {account_label}."
                )
        except urllib.error.HTTPError as error:
            if error.code == 401:
                message = (
                    "Logged out. Token instantly revoked, kept last known quota for "
                    f"{account_label}."
                )
            else:
                message = f"Logged out. HTTP {error.code} during final fetch."
        except Exception as error:
            message = f"Logged out. Error during final fetch: {str(error)}"

        self.root.after(0, lambda m=message: self._finalize_logout(m))

    def _finalize_logout(self, message: str) -> None:
        if not self._auth_file_has_access_token():
            self.state.clear_session_credentials()
            self.refresh_ui()
        self._finish_fetch(message)

    def check_auto_fetch(self) -> None:
        now = time.time()
        jwt = self.state.get_due_auto_fetch_jwt(now)
        if jwt:
            email = self.state.current_account_email or "current account"
            interval_label = self.state.get_auto_fetch_value()
            self._notify_auto_fetch_triggered(email, interval_label)
            self._begin_fetch(f"Auto-fetching quota for {email}...")
            threading.Thread(
                target=self._bg_fetch_single,
                args=(self.state.current_account_email, jwt),
                daemon=True,
            ).start()
        self._auto_fetch_next_backup_account(now)

    def _auto_fetch_next_backup_account(self, now: float) -> None:
        interval_label = self.state.get_auto_fetch_value()
        interval_seconds = AUTO_FETCH_INTERVALS.get(interval_label, 0)
        if interval_seconds <= 0:
            return

        active_email = self.state.current_account_email
        candidates = [
            email
            for email, data in self._visible_sorted_account_items()
            if email != active_email
            and not self._is_archived_account(data)
            and self.auth_file_service.backup_exists(email)
            and (now - data.get("last_fetched", 0)) >= interval_seconds
        ]
        if not candidates:
            return

        self._backup_auto_fetch_cursor %= len(candidates)
        email = candidates[self._backup_auto_fetch_cursor]
        self._backup_auto_fetch_cursor = (
            self._backup_auto_fetch_cursor + 1
        ) % len(candidates)

        self._begin_fetch(f"Refreshing saved auth backup and auto-fetching quota for {email}...")
        threading.Thread(
            target=self._bg_fetch_backup_account,
            args=(email,),
            daemon=True,
        ).start()

    def _is_archived_account(self, data: dict) -> bool:
        return data.get("archived") is True

    def _get_account_sort_key(self, item: Tuple[str, dict]):
        email, data = item
        if self.sort_column == "email":
            return email.lower()
        if self.sort_column in ("quota", "weekly_quota"):
            value = self._window_used_percent(data, "weekly_window")
            return value if value is not None else data.get("used_percent", 0)
        if self.sort_column == "short_quota":
            value = self._window_used_percent(data, "short_window")
            return value if value is not None else 0
        if self.sort_column == "short_reset":
            return self._window_reset_ts(data, "short_window") or 0
        if self.sort_column == "weekly_reset":
            value = self._window_reset_ts(data, "weekly_window")
            return value if value is not None else data.get("reset_ts", 0)
        return email.lower()

    def _visible_sorted_account_items(self) -> List[Tuple[str, dict]]:
        items = [
            item for item in self.state.usage_map.items()
            if self.show_archived or not self._is_archived_account(item[1])
        ]
        items.sort(
            key=self._get_account_sort_key,
            reverse=bool(self.sort_column and not self.sort_asc),
        )

        active_email = self.state.current_account_email
        items.sort(
            key=lambda item: (
                1 if self._is_archived_account(item[1]) else 0,
                0 if item[0] == active_email and not self._is_archived_account(item[1]) else 1,
                0 if self.auth_file_service.backup_exists(item[0]) else 1,
            )
        )
        return items

    def refresh_ui(self, skip_auto_fetch: bool = False) -> None:
        if self._timer_id:
            self.root.after_cancel(self._timer_id)

        if not skip_auto_fetch:
            self.check_auto_fetch()
        self._clear_account_rows()

        items = self._visible_sorted_account_items()

        if not items:
            empty_label = ctk.CTkLabel(
                self.accounts_rows_frame,
                text="No tracked accounts yet. Fetch current auth.json to load one.",
                font=ctk.CTkFont(size=12),
                text_color=self._theme_tokens()["empty_fg"],
            )
            empty_label.grid(row=0, column=0, sticky="ew", padx=10, pady=20)
        else:
            now_ts = datetime.now().timestamp()
            for index, (email, data) in enumerate(items):
                try:
                    short_reset_ts = self._window_reset_ts(data, "short_window")
                    weekly_reset_ts = self._window_reset_ts(data, "weekly_window")
                    if weekly_reset_ts is None:
                        weekly_reset_ts = data.get("reset_ts", 0)
                    weekly_used_percent = self._window_used_percent(
                        data,
                        "weekly_window",
                    )
                    if weekly_used_percent is None:
                        weekly_used_percent = data.get("used_percent", 0)
                    short_used_percent = self._window_used_percent(
                        data,
                        "short_window",
                    )
                    is_current = email == self.state.current_account_email
                    is_archived = self._is_archived_account(data)
                    has_auth_backup = self.auth_file_service.backup_exists(email)
                    short_reset_display = format_reset_display(short_reset_ts, now_ts)
                    weekly_reset_display = format_reset_display(weekly_reset_ts, now_ts)
                    weekly_quota_left = format_quota_left(weekly_used_percent)
                    short_quota_left = (
                        format_quota_left(short_used_percent)
                        if short_used_percent is not None
                        else "-"
                    )
                    has_short_window = self._has_window_data(data, "short_window")
                    quota_lines = []
                    reset_lines = []
                    if has_short_window:
                        quota_lines.append(self._labeled_value(short_quota_left, "5h"))
                        reset_lines.append(self._labeled_value(short_reset_display, "5h"))
                        quota_lines.append(self._labeled_value(weekly_quota_left, "weekly"))
                        reset_lines.append(self._labeled_value(weekly_reset_display, "weekly"))
                    else:
                        quota_lines.append(weekly_quota_left)
                        reset_lines.append(weekly_reset_display)
                except Exception:
                    quota_lines = ["Error"]
                    reset_lines = ["Error"]
                    is_current = email == self.state.current_account_email
                    is_archived = self._is_archived_account(data)
                    has_auth_backup = self.auth_file_service.backup_exists(email)

                self._build_account_row(
                    email=email,
                    quota_lines=quota_lines,
                    reset_lines=reset_lines,
                    is_current=is_current,
                    is_archived=is_archived,
                    has_auth_backup=has_auth_backup,
                    index=index,
                )

        self.root.after_idle(self._update_accounts_scrollregion)
        self._timer_id = self.root.after(60000, self.refresh_ui)

    def on_closing(self) -> None:
        if self._timer_id:
            self.root.after_cancel(self._timer_id)
            self._timer_id = None
        if self._update_check_timer_id:
            self.root.after_cancel(self._update_check_timer_id)
            self._update_check_timer_id = None
        if self._auth_poll_timer_id:
            self.root.after_cancel(self._auth_poll_timer_id)
            self._auth_poll_timer_id = None

        process = self._login_process
        if process and process.poll() is None:
            try:
                process.terminate()
            except OSError:
                pass

        self.auth_watcher.stop()
        self.root.destroy()
