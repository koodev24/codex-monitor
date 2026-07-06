import json
import os
import re
import shutil
import tempfile
import time
from datetime import datetime, timezone
from typing import Dict, List, Optional, Tuple

from .api import AuthRefreshClient
from .config import (
    AUTH_ACCOUNTS_DIR,
    AUTH_FILE_PATH,
    AUTH_REFRESH_INTERVAL_SECONDS,
    AUTO_FETCH_INTERVALS,
    AUTO_FETCH_OPTIONS,
    LEGACY_AUTH_ACCOUNTS_DIRS,
)
from .models import (
    AuthFileSnapshot,
    RateLimitWindow,
    ResetCreditsPayload,
    UsageMap,
    UsageResponse,
)
from .storage import UsageStorage


class MonitorStateService:
    def __init__(self, storage: UsageStorage):
        self.storage = storage
        self.usage_map: UsageMap = storage.load()
        self.session_tokens: Dict[str, str] = {}
        self.current_account_email: Optional[str] = self._restore_current_account_email()
        self.auto_fetch_interval: str = self._restore_auto_fetch_value()
        self.latest_auth_jwt: Optional[str] = None
        self.sort_column: Optional[str] = self.storage.get_meta_value("sort_column")
        sort_asc_raw = self.storage.get_meta_value("sort_asc")
        self.sort_asc: bool = True if sort_asc_raw is None else bool(sort_asc_raw)
        self.show_archived: bool = bool(self.storage.get_meta_value("show_archived"))
        show_5h_columns_raw = self.storage.get_meta_value("show_5h_columns")
        self.show_5h_columns: bool = (
            True if show_5h_columns_raw is None else bool(show_5h_columns_raw)
        )
        self.logs_expanded: bool = bool(self.storage.get_meta_value("logs_expanded"))

    def save_sort_preference(self, column: Optional[str], asc: bool) -> None:
        self.sort_column = column
        self.sort_asc = asc
        self.storage.set_meta_value("sort_column", column)
        self.storage.set_meta_value("sort_asc", asc)
        self.save_data()

    def save_show_archived_preference(self, show_archived: bool) -> None:
        self.show_archived = show_archived
        self.storage.set_meta_value("show_archived", show_archived)
        self.save_data()

    def save_show_5h_columns_preference(self, show_5h_columns: bool) -> None:
        self.show_5h_columns = show_5h_columns
        self.storage.set_meta_value("show_5h_columns", show_5h_columns)
        self.save_data()

    def save_logs_expanded_preference(self, logs_expanded: bool) -> None:
        self.logs_expanded = logs_expanded
        self.storage.set_meta_value("logs_expanded", logs_expanded)
        self.save_data()

    def save_data(self) -> None:
        try:
            self.storage.save(self.usage_map)
        except Exception as error:
            print(f"Failed to save data: {error}")

    def clear_session_credentials(self) -> bool:
        had_current_account = (
            self.current_account_email is not None
            or bool(self.session_tokens)
            or bool(self.latest_auth_jwt)
        )
        self.session_tokens.clear()
        self.current_account_email = None
        self.latest_auth_jwt = None
        self.storage.set_meta_value("current_account_email", None)
        self.save_data()
        return had_current_account

    def remember_auth_jwt(self, jwt: Optional[str]) -> None:
        self.latest_auth_jwt = jwt

    def remember_account_jwt(self, email: str, jwt: str) -> None:
        if not email or not jwt:
            return
        self.session_tokens[email] = jwt
        if email == self.current_account_email:
            self.latest_auth_jwt = jwt

    def set_current_account(self, email: str, jwt: str) -> None:
        self.current_account_email = email
        self.session_tokens = {email: jwt}
        self.latest_auth_jwt = jwt
        self.storage.set_meta_value("current_account_email", email)

    def get_display_auto_fetch(self, email: str) -> str:
        if email != self.current_account_email:
            return "-"
        return self.auto_fetch_interval

    def get_auto_fetch_value(self) -> str:
        return self.auto_fetch_interval

    def save_auto_fetch_value(self, email: str, new_value: str) -> bool:
        if email in self.usage_map and email == self.current_account_email:
            self.auto_fetch_interval = (
                new_value if new_value in AUTO_FETCH_OPTIONS else "None"
            )
            self.storage.set_meta_value("auto_fetch", self.auto_fetch_interval)
            self.save_data()
            return True
        return False

    def remove_account(self, email: str) -> bool:
        if email not in self.usage_map:
            return False

        self.usage_map.pop(email, None)
        self.session_tokens.pop(email, None)
        if email == self.current_account_email:
            self.current_account_email = None
            self.latest_auth_jwt = None
            self.storage.set_meta_value("current_account_email", None)

        self.save_data()
        return True

    def archive_account(self, email: str) -> bool:
        if email not in self.usage_map:
            return False

        self.usage_map[email]["archived"] = True
        self.save_data()
        return True

    def unarchive_account(self, email: str) -> bool:
        if email not in self.usage_map:
            return False

        self.usage_map[email].pop("archived", None)
        self.save_data()
        return True

    def import_data(self, payload: object) -> None:
        self.usage_map = self.storage.import_data(payload)
        self.current_account_email = self._restore_current_account_email()
        self.auto_fetch_interval = self._restore_auto_fetch_value()
        self.show_archived = bool(self.storage.get_meta_value("show_archived"))
        show_5h_columns_raw = self.storage.get_meta_value("show_5h_columns")
        self.show_5h_columns = (
            True if show_5h_columns_raw is None else bool(show_5h_columns_raw)
        )
        self.logs_expanded = bool(self.storage.get_meta_value("logs_expanded"))
        self.session_tokens.clear()
        self.latest_auth_jwt = None

    def apply_usage_response(
        self,
        response_data: UsageResponse,
        jwt: str,
        activate: bool = True,
    ) -> Optional[str]:
        email = response_data.get("email")
        rate_limit = response_data.get("rate_limit", {})
        primary_window = self._sanitize_rate_limit_window(
            rate_limit.get("primary_window") if rate_limit else None
        )
        secondary_window = self._sanitize_rate_limit_window(
            rate_limit.get("secondary_window") if rate_limit else None
        )
        short_window, weekly_window = self._classify_rate_limit_windows(
            primary_window,
            secondary_window,
        )

        reset_at = weekly_window.get("reset_at") or primary_window.get("reset_at")
        used_percent = weekly_window.get("used_percent", primary_window.get("used_percent", 0))

        if not email or not (primary_window or secondary_window):
            return None

        if email not in self.usage_map:
            self.usage_map[email] = {}

        next_usage = {
            "reset_ts": reset_at or 0,
            "used_percent": used_percent,
            "last_fetched": time.time(),
        }
        if primary_window:
            next_usage["primary_window"] = primary_window
        if secondary_window:
            next_usage["secondary_window"] = secondary_window
        if short_window:
            next_usage["short_window"] = short_window
        if weekly_window:
            next_usage["weekly_window"] = weekly_window

        self.usage_map[email].update(next_usage)
        if activate:
            self.set_current_account(email, jwt)
        else:
            self.session_tokens[email] = jwt
        self.save_data()
        return email

    def apply_reset_credits(
        self,
        email: str,
        payload: ResetCreditsPayload,
    ) -> None:
        if not email or email not in self.usage_map:
            return
        self.usage_map[email]["resets"] = payload
        self.save_data()

    def get_account_resets(
        self, email: Optional[str]
    ) -> Optional[ResetCreditsPayload]:
        if not email or email not in self.usage_map:
            return None
        return self.usage_map[email].get("resets")

    def _sanitize_rate_limit_window(self, value: object) -> RateLimitWindow:
        if not isinstance(value, dict):
            return {}

        clean_window: RateLimitWindow = {}
        for field in ("used_percent", "limit_window_seconds", "reset_after_seconds", "reset_at"):
            raw_value = value.get(field)
            if raw_value in (None, ""):
                continue
            if isinstance(raw_value, bool):
                continue
            if isinstance(raw_value, (int, float)):
                clean_window[field] = raw_value

        if not clean_window.get("reset_at"):
            return {}
        return clean_window

    def _classify_rate_limit_windows(
        self,
        primary_window: RateLimitWindow,
        secondary_window: RateLimitWindow,
    ) -> Tuple[RateLimitWindow, RateLimitWindow]:
        windows = [window for window in (primary_window, secondary_window) if window]
        if not windows:
            return {}, {}

        weekly_candidates = [
            window
            for window in windows
            if (window.get("limit_window_seconds") or 0) >= 6 * 24 * 60 * 60
        ]
        weekly_window = weekly_candidates[0] if weekly_candidates else max(
            windows,
            key=lambda window: window.get("limit_window_seconds") or 0,
        )

        short_candidates = [window for window in windows if window is not weekly_window]
        short_window = short_candidates[0] if short_candidates else {}
        return short_window, weekly_window

    def get_due_auto_fetch_jwt(self, now: float) -> Optional[str]:
        current_email = self.current_account_email
        if not current_email:
            return None

        data = self.usage_map.get(current_email)
        if not data:
            return None

        interval_label = self.auto_fetch_interval
        if interval_label == "None":
            return None

        interval_seconds = AUTO_FETCH_INTERVALS.get(interval_label, 0)
        if interval_seconds <= 0:
            return None

        if (now - data.get("last_fetched", 0)) < interval_seconds:
            return None

        data["last_fetched"] = now
        return self.latest_auth_jwt or self.session_tokens.get(current_email)

    def get_latest_jwt_for_fetch(self, email: Optional[str] = None) -> Optional[str]:
        if self.latest_auth_jwt:
            return self.latest_auth_jwt
        if email:
            return self.session_tokens.get(email)
        current_email = self.current_account_email
        if not current_email:
            return None
        return self.session_tokens.get(current_email)

    def sorted_usage_items(self) -> List[Tuple[str, dict]]:
        return sorted(
            self.usage_map.items(),
            key=lambda item: (
                0 if item[0] == self.current_account_email else 1,
                self._account_weekly_reset_ts(item[1]),
            ),
        )

    def _account_weekly_reset_ts(self, account_data: dict) -> float:
        weekly_window = account_data.get("weekly_window")
        if isinstance(weekly_window, dict):
            reset_at = weekly_window.get("reset_at")
            if isinstance(reset_at, (int, float)) and not isinstance(reset_at, bool):
                return reset_at
        return account_data.get("reset_ts", 0) or 0

    def _restore_current_account_email(self) -> Optional[str]:
        saved_current_email = self.storage.get_meta_value("current_account_email")
        if saved_current_email in self.usage_map:
            return saved_current_email

        if len(self.usage_map) == 1:
            return next(iter(self.usage_map))

        return None

    def _restore_auto_fetch_value(self) -> str:
        saved_auto_fetch = self.storage.get_meta_value("auto_fetch")
        if isinstance(saved_auto_fetch, str) and saved_auto_fetch in AUTO_FETCH_OPTIONS:
            return saved_auto_fetch
        return "None"


class AuthFileService:
    def __init__(
        self,
        auth_file_path: str = AUTH_FILE_PATH,
        accounts_dir: str = AUTH_ACCOUNTS_DIR,
    ):
        self.auth_file_path = auth_file_path
        self.accounts_dir = accounts_dir
        if accounts_dir == AUTH_ACCOUNTS_DIR:
            self._migrate_legacy_account_backups()

    def auth_file_exists(self) -> bool:
        return os.path.exists(self.auth_file_path)

    def load_snapshot(self) -> AuthFileSnapshot:
        return self.load_snapshot_from_path(self.auth_file_path)

    def load_snapshot_from_path(self, auth_file_path: str) -> AuthFileSnapshot:
        with open(auth_file_path, "r", encoding="utf-8") as file:
            auth_data = json.load(file)
        if not isinstance(auth_data, dict):
            raise ValueError("auth.json root must be an object")
        return auth_data

    def load_access_token(self) -> Optional[str]:
        return self.load_snapshot().get("tokens", {}).get("access_token")

    def backup_path_for_email(self, email: str) -> str:
        return os.path.join(
            self.accounts_dir,
            f"auth-{self._safe_account_file_key(email)}.json",
        )

    def backup_exists(self, email: str) -> bool:
        return os.path.exists(self.backup_path_for_email(email))

    def list_backup_emails(self) -> List[str]:
        if not os.path.isdir(self.accounts_dir):
            return []

        emails: List[str] = []
        for filename in os.listdir(self.accounts_dir):
            if not filename.startswith("auth-") or not filename.endswith(".json"):
                continue
            path = os.path.join(self.accounts_dir, filename)
            if not os.path.isfile(path):
                continue
            try:
                snapshot = self.load_snapshot_from_path(path)
            except (json.JSONDecodeError, OSError, ValueError, TypeError):
                continue
            email = snapshot.get("email")
            if isinstance(email, str) and email:
                emails.append(email)
                continue
            tokens = snapshot.get("tokens", {})
            account_id = tokens.get("account_id") if isinstance(tokens, dict) else None
            if isinstance(account_id, str) and account_id:
                emails.append(filename[len("auth-") : -len(".json")])
        return sorted(set(emails))

    def backup_current_auth(self, email: str) -> str:
        if not self.auth_file_exists():
            raise FileNotFoundError(self.auth_file_path)

        return self.backup_auth_from_path(email, self.auth_file_path)

    def backup_auth_from_path(self, email: str, source_path: str) -> str:
        return self._copy_auth_file(
            source_path=source_path,
            target_path=self.backup_path_for_email(email),
        )

    def remove_backup(self, email: Optional[str]) -> bool:
        if not email:
            return False

        try:
            os.remove(self.backup_path_for_email(email))
            return True
        except FileNotFoundError:
            return False

    def remove_backup_if_access_token_matches(self, email: str, access_token: str) -> bool:
        if not email or not access_token:
            return False

        try:
            current_backup_token = self.load_backup_access_token(email)
        except (FileNotFoundError, json.JSONDecodeError, OSError, ValueError, TypeError):
            return False

        if current_backup_token != access_token:
            return False

        return self.remove_backup(email)

    def load_backup_snapshot(self, email: str) -> AuthFileSnapshot:
        with open(self.backup_path_for_email(email), "r", encoding="utf-8") as file:
            auth_data = json.load(file)
        if not isinstance(auth_data, dict):
            raise ValueError("backup auth root must be an object")
        return auth_data

    def export_backups(self, emails: List[str]) -> Dict[str, AuthFileSnapshot]:
        exported: Dict[str, AuthFileSnapshot] = {}
        for email in sorted(set([*emails, *self.list_backup_emails()])):
            if not email or not self.backup_exists(email):
                continue
            try:
                exported[email] = self.load_backup_snapshot(email)
            except (json.JSONDecodeError, OSError, ValueError, TypeError):
                continue
        return exported

    def import_backups(self, backups: object) -> int:
        if not isinstance(backups, dict):
            return 0

        imported_count = 0
        for email, snapshot in backups.items():
            if not isinstance(email, str) or not email:
                continue
            if not isinstance(snapshot, dict):
                continue
            tokens = snapshot.get("tokens")
            if not isinstance(tokens, dict) or not tokens.get("access_token"):
                continue
            self._write_backup_snapshot(email, snapshot)
            imported_count += 1
        return imported_count

    def load_backup_access_token(self, email: str) -> Optional[str]:
        return self.load_backup_snapshot(email).get("tokens", {}).get("access_token")

    def refresh_backup_if_due(
        self,
        email: str,
        refresh_client: AuthRefreshClient,
        force: bool = False,
        now: Optional[float] = None,
    ) -> AuthFileSnapshot:
        snapshot = self.load_backup_snapshot(email)
        if not force and not self._snapshot_needs_refresh(snapshot, now):
            return snapshot

        refreshed_snapshot = self._refresh_snapshot_tokens(snapshot, refresh_client)
        self._write_backup_snapshot(email, refreshed_snapshot)
        return refreshed_snapshot

    def switch_to_account_backup(
        self,
        target_email: str,
        current_email: Optional[str],
    ) -> Optional[str]:
        if not self.backup_exists(target_email):
            raise FileNotFoundError(self.backup_path_for_email(target_email))

        current_backup_path = None
        if current_email and self.auth_file_exists():
            current_backup_path = self.backup_current_auth(current_email)

        self._copy_auth_file(
            source_path=self.backup_path_for_email(target_email),
            target_path=self.auth_file_path,
        )
        return current_backup_path

    def create_login_codex_home(self) -> str:
        os.makedirs(self.accounts_dir, exist_ok=True)
        login_home = tempfile.mkdtemp(prefix="login-", dir=self.accounts_dir)
        try:
            os.chmod(login_home, 0o700)
        except OSError:
            pass
        return login_home

    def remove_login_codex_home(self, login_codex_home: Optional[str]) -> None:
        if not login_codex_home:
            return

        accounts_dir = os.path.realpath(self.accounts_dir)
        target = os.path.realpath(login_codex_home)
        if not target.startswith(accounts_dir + os.sep):
            return
        if os.path.basename(target).startswith("login-"):
            shutil.rmtree(target, ignore_errors=True)

    def active_auth_path_for_home(self, codex_home: str) -> str:
        return os.path.join(codex_home, "auth.json")

    def activate_auth_from_path(self, source_path: str) -> str:
        return self._copy_auth_file(
            source_path=source_path,
            target_path=self.auth_file_path,
        )

    def _copy_auth_file(self, source_path: str, target_path: str) -> str:
        os.makedirs(os.path.dirname(target_path), exist_ok=True)
        temp_fd, temp_path = tempfile.mkstemp(
            prefix=f".{os.path.basename(target_path)}.",
            suffix=".tmp",
            dir=os.path.dirname(target_path),
        )
        os.close(temp_fd)
        try:
            shutil.copyfile(source_path, temp_path)
            try:
                os.chmod(temp_path, 0o600)
            except OSError:
                pass
            os.replace(temp_path, target_path)
        except Exception:
            try:
                os.remove(temp_path)
            except OSError:
                pass
            raise
        return target_path

    def _write_backup_snapshot(self, email: str, snapshot: AuthFileSnapshot) -> str:
        target_path = self.backup_path_for_email(email)
        os.makedirs(os.path.dirname(target_path), exist_ok=True)
        temp_fd, temp_path = tempfile.mkstemp(
            prefix=f".{os.path.basename(target_path)}.",
            suffix=".tmp",
            dir=os.path.dirname(target_path),
        )
        os.close(temp_fd)
        try:
            with open(temp_path, "w", encoding="utf-8") as file:
                json.dump(snapshot, file, indent=2)
            try:
                os.chmod(temp_path, 0o600)
            except OSError:
                pass
            os.replace(temp_path, target_path)
        except Exception:
            try:
                os.remove(temp_path)
            except OSError:
                pass
            raise
        return target_path

    def _refresh_snapshot_tokens(
        self,
        snapshot: AuthFileSnapshot,
        refresh_client: AuthRefreshClient,
    ) -> AuthFileSnapshot:
        tokens = snapshot.get("tokens")
        if not isinstance(tokens, dict):
            raise ValueError("auth backup has no tokens object")

        refresh_token = tokens.get("refresh_token")
        if not isinstance(refresh_token, str) or not refresh_token:
            raise ValueError("auth backup has no refresh_token")

        refreshed = refresh_client.refresh_tokens(refresh_token)
        access_token = refreshed.get("access_token")
        if not isinstance(access_token, str) or not access_token:
            raise ValueError("token refresh response has no access_token")

        next_snapshot: AuthFileSnapshot = dict(snapshot)
        next_tokens = dict(tokens)
        for field in ("id_token", "access_token", "refresh_token"):
            value = refreshed.get(field)
            if isinstance(value, str) and value:
                next_tokens[field] = value
        next_snapshot["tokens"] = next_tokens
        next_snapshot["last_refresh"] = self._current_refresh_timestamp()
        return next_snapshot

    def _snapshot_needs_refresh(
        self,
        snapshot: AuthFileSnapshot,
        now: Optional[float] = None,
    ) -> bool:
        tokens = snapshot.get("tokens")
        if not isinstance(tokens, dict):
            return False
        refresh_token = tokens.get("refresh_token")
        if not isinstance(refresh_token, str) or not refresh_token:
            return False

        last_refresh_ts = self._parse_refresh_timestamp(snapshot.get("last_refresh"))
        if last_refresh_ts is None:
            return True

        now_ts = time.time() if now is None else now
        return (now_ts - last_refresh_ts) >= AUTH_REFRESH_INTERVAL_SECONDS

    def _parse_refresh_timestamp(self, value: object) -> Optional[float]:
        if not isinstance(value, str) or not value:
            return None

        normalized = value
        if normalized.endswith("Z"):
            normalized = normalized[:-1] + "+00:00"
        try:
            parsed = datetime.fromisoformat(normalized)
        except ValueError:
            return None
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone.utc)
        return parsed.timestamp()

    def _current_refresh_timestamp(self) -> str:
        return datetime.now(timezone.utc).isoformat(timespec="microseconds").replace(
            "+00:00",
            "Z",
        )

    def _migrate_legacy_account_backups(self) -> None:
        for legacy_dir in LEGACY_AUTH_ACCOUNTS_DIRS:
            if legacy_dir == self.accounts_dir or not os.path.isdir(legacy_dir):
                continue
            try:
                os.makedirs(self.accounts_dir, exist_ok=True)
                for filename in os.listdir(legacy_dir):
                    if not filename.startswith("auth-") or not filename.endswith(".json"):
                        continue
                    source_path = os.path.join(legacy_dir, filename)
                    target_path = os.path.join(self.accounts_dir, filename)
                    if not os.path.isfile(source_path) or os.path.exists(target_path):
                        continue
                    shutil.move(source_path, target_path)
                try:
                    os.rmdir(legacy_dir)
                except OSError:
                    pass
            except OSError:
                continue

    def _safe_account_file_key(self, email: str) -> str:
        safe_email = re.sub(r"[^A-Za-z0-9._@+-]+", "_", email.strip())
        safe_email = safe_email.strip("._-")
        return safe_email or "unknown"
