import json
import os
import shutil
from typing import Any, Dict, Optional

from .config import (
    AUTO_FETCH_OPTIONS,
    LEGACY_LOCAL_STORAGE_FILE,
    LEGACY_LOCAL_STORAGE_META_FILE,
    LOCAL_STORAGE_FILE,
    LOCAL_STORAGE_META_FILE,
)
from .models import AccountUsage, UsageMap


class UsageStorage:
    ACCOUNTS_KEY = "accounts"
    META_KEY = "__meta__"

    def __init__(
        self,
        storage_path: str = LOCAL_STORAGE_FILE,
        meta_path: str = LOCAL_STORAGE_META_FILE,
    ):
        self.storage_path = storage_path
        self.meta_path = meta_path
        self.meta: Dict[str, Any] = {}
        if (
            storage_path == LOCAL_STORAGE_FILE
            and meta_path == LOCAL_STORAGE_META_FILE
        ):
            self._migrate_legacy_files()

    def load(self) -> UsageMap:
        data: UsageMap = {}
        legacy_auto_fetch_by_email: Dict[str, str] = {}
        needs_resave = False
        needs_meta_save = False
        self.meta = self._load_meta_file()

        if os.path.exists(self.storage_path):
            try:
                with open(self.storage_path, "r", encoding="utf-8") as file:
                    raw = json.load(file)
            except Exception:
                raw = {}

            if isinstance(raw, dict) and isinstance(raw.get(self.ACCOUNTS_KEY), dict):
                raw_accounts = dict(raw.get(self.ACCOUNTS_KEY, {}))
                legacy_meta = self._sanitize_meta(raw.get(self.META_KEY, {}))
                if legacy_meta:
                    self.meta.update(legacy_meta)
                    needs_meta_save = True

                for key, value in raw.items():
                    if key in (self.ACCOUNTS_KEY, self.META_KEY):
                        continue
                    if self._looks_like_account_payload(value):
                        raw_accounts[key] = self._merge_account_payloads(
                            raw_accounts.get(key),
                            value,
                        )

                needs_resave = True
            elif isinstance(raw, dict):
                raw_accounts = raw
            else:
                raw_accounts = {}

            for email, value in raw_accounts.items():
                if email in (self.ACCOUNTS_KEY, self.META_KEY):
                    needs_resave = True
                    continue

                if isinstance(value, (int, float)):
                    data[email] = {
                        "reset_ts": value,
                        "used_percent": 0,
                        "auto_fetch": "None",
                        "last_fetched": 0,
                    }
                    needs_resave = True
                    continue

                if not self._looks_like_account_payload(value):
                    needs_resave = True
                    continue

                auto_fetch_value = self._sanitize_auto_fetch_value(value.get("auto_fetch"))
                if auto_fetch_value != "None":
                    legacy_auto_fetch_by_email[email] = auto_fetch_value

                sanitized = self._sanitize_account_data(value)
                if "jwt" in value or "auto_fetch" in value:
                    needs_resave = True
                data[email] = sanitized

        if "auto_fetch" not in self.meta:
            migrated_auto_fetch = self._select_migrated_auto_fetch(
                legacy_auto_fetch_by_email
            )
            if migrated_auto_fetch != "None":
                self.meta["auto_fetch"] = migrated_auto_fetch
                needs_meta_save = True

        if "current_account_email" not in self.meta:
            legacy_active_emails = [
                email for email in legacy_auto_fetch_by_email if email in data
            ]
            if len(legacy_active_emails) == 1:
                self.meta["current_account_email"] = legacy_active_emails[0]
                needs_meta_save = True

        if needs_resave:
            self.save(data)
        elif needs_meta_save:
            self._save_meta_file()

        return data

    def save(self, data: UsageMap) -> None:
        self._ensure_parent_dir(self.storage_path)
        with open(self.storage_path, "w", encoding="utf-8") as file:
            json.dump(self._sanitize_usage_map(data), file)
        self._save_meta_file()

    def get_meta_value(self, key: str) -> Optional[Any]:
        return self.meta.get(key)

    def set_meta_value(self, key: str, value: Optional[Any]) -> None:
        if value in (None, ""):
            self.meta.pop(key, None)
            return
        self.meta[key] = value

    def export_data(self, data: UsageMap) -> Dict[str, Any]:
        return {
            "schema_version": 2,
            "accounts": self._sanitize_usage_map(data),
            "config": self._sanitize_meta(self.meta),
        }

    def import_data(self, payload: object) -> UsageMap:
        if not isinstance(payload, dict):
            raise ValueError("Import file root must be a JSON object.")

        raw_accounts = payload.get(self.ACCOUNTS_KEY)
        if not isinstance(raw_accounts, dict):
            raw_accounts = {
                key: value
                for key, value in payload.items()
                if key not in (self.META_KEY, "config", "schema_version")
            }

        imported_accounts: UsageMap = {}
        legacy_auto_fetch_by_email: Dict[str, str] = {}
        for email, value in raw_accounts.items():
            if not isinstance(email, str) or not email:
                continue
            if not self._looks_like_account_payload(value):
                continue

            auto_fetch_value = self._sanitize_auto_fetch_value(value.get("auto_fetch"))
            if auto_fetch_value != "None":
                legacy_auto_fetch_by_email[email] = auto_fetch_value
            imported_accounts[email] = self._sanitize_account_data(value)

        if not imported_accounts:
            raise ValueError("Import file does not contain account data.")

        current_accounts = self.load()
        merged_accounts: UsageMap = dict(current_accounts)
        for email, value in imported_accounts.items():
            merged_accounts[email] = self._sanitize_account_data(
                self._merge_account_payloads(merged_accounts.get(email), value)
            )

        raw_config = payload.get("config", payload.get(self.META_KEY, {}))
        imported_meta = self._sanitize_meta(raw_config)
        if "auto_fetch" not in imported_meta:
            imported_meta["auto_fetch"] = self._select_migrated_auto_fetch(
                legacy_auto_fetch_by_email
            )

        self.meta.update(imported_meta)
        current_account_email = self.meta.get("current_account_email")
        if current_account_email not in merged_accounts:
            self.meta.pop("current_account_email", None)

        self.save(merged_accounts)
        return merged_accounts

    def _sanitize_usage_map(self, data: UsageMap) -> UsageMap:
        return {
            email: self._sanitize_account_data(account_data)
            for email, account_data in data.items()
        }

    def _sanitize_meta(self, meta: dict) -> Dict[str, Any]:
        if not isinstance(meta, dict):
            return {}

        sanitized: Dict[str, Any] = {}
        current_account_email = meta.get("current_account_email")
        if isinstance(current_account_email, str) and current_account_email:
            sanitized["current_account_email"] = current_account_email

        if "auto_fetch" in meta:
            auto_fetch = self._sanitize_auto_fetch_value(meta.get("auto_fetch"))
            if auto_fetch in AUTO_FETCH_OPTIONS:
                sanitized["auto_fetch"] = auto_fetch

        if "sort_column" in meta:
            sort_column = meta.get("sort_column")
            if isinstance(sort_column, str) or sort_column is None:
                sanitized["sort_column"] = sort_column

        if "sort_asc" in meta:
            sanitized["sort_asc"] = bool(meta.get("sort_asc"))

        if "show_archived" in meta:
            sanitized["show_archived"] = bool(meta.get("show_archived"))

        if "show_5h_columns" in meta:
            sanitized["show_5h_columns"] = bool(meta.get("show_5h_columns"))

        if "logs_expanded" in meta:
            sanitized["logs_expanded"] = bool(meta.get("logs_expanded"))

        return sanitized

    def _sanitize_account_data(self, account_data: dict) -> AccountUsage:
        clean_account: AccountUsage = {}
        for field in ("reset_ts", "used_percent", "last_fetched"):
            value = account_data.get(field)
            if isinstance(value, (int, float)) and not isinstance(value, bool):
                clean_account[field] = value

        for field in (
            "primary_window",
            "secondary_window",
            "short_window",
            "weekly_window",
        ):
            window = self._sanitize_rate_limit_window(account_data.get(field))
            if window:
                clean_account[field] = window

        if account_data.get("archived") is True:
            clean_account["archived"] = True

        if "resets" in account_data and isinstance(account_data["resets"], dict):
            clean_account["resets"] = account_data["resets"]

        return clean_account

    def _sanitize_rate_limit_window(self, value: object) -> Dict[str, float]:
        if not isinstance(value, dict):
            return {}

        clean_window: Dict[str, float] = {}
        for field in ("used_percent", "limit_window_seconds", "reset_after_seconds", "reset_at"):
            raw_value = value.get(field)
            if isinstance(raw_value, (int, float)) and not isinstance(raw_value, bool):
                clean_window[field] = raw_value
        return clean_window

    def _load_meta_file(self) -> Dict[str, Any]:
        if not os.path.exists(self.meta_path):
            return {}

        try:
            with open(self.meta_path, "r", encoding="utf-8") as file:
                raw = json.load(file)
        except Exception:
            return {}

        return self._sanitize_meta(raw)

    def _save_meta_file(self) -> None:
        self._ensure_parent_dir(self.meta_path)
        with open(self.meta_path, "w", encoding="utf-8") as file:
            json.dump(self._sanitize_meta(self.meta), file)

    def _ensure_parent_dir(self, path: str) -> None:
        parent_dir = os.path.dirname(path)
        if parent_dir:
            os.makedirs(parent_dir, exist_ok=True)

    def _migrate_legacy_files(self) -> None:
        self._migrate_legacy_file(LEGACY_LOCAL_STORAGE_FILE, self.storage_path)
        self._migrate_legacy_file(LEGACY_LOCAL_STORAGE_META_FILE, self.meta_path)

    def _migrate_legacy_file(self, legacy_path: str, target_path: str) -> None:
        if legacy_path == target_path:
            return
        if not os.path.exists(legacy_path) or os.path.exists(target_path):
            return

        self._ensure_parent_dir(target_path)
        try:
            shutil.move(legacy_path, target_path)
        except OSError:
            try:
                shutil.copyfile(legacy_path, target_path)
            except OSError:
                pass

    def _sanitize_auto_fetch_value(self, value: object) -> str:
        if isinstance(value, str) and value in AUTO_FETCH_OPTIONS:
            return value
        return "None"

    def _select_migrated_auto_fetch(
        self,
        legacy_auto_fetch_by_email: Dict[str, str],
    ) -> str:
        saved_current_email = self.meta.get("current_account_email")
        if isinstance(saved_current_email, str):
            current_value = legacy_auto_fetch_by_email.get(saved_current_email)
            if current_value:
                return current_value

        distinct_values = {
            value for value in legacy_auto_fetch_by_email.values() if value != "None"
        }
        if len(distinct_values) == 1:
            return next(iter(distinct_values))
        return "None"

    def _looks_like_account_payload(self, value: object) -> bool:
        if not isinstance(value, dict):
            return False

        known_fields = {
            "reset_ts",
            "used_percent",
            "primary_window",
            "secondary_window",
            "short_window",
            "weekly_window",
            "auto_fetch",
            "last_fetched",
            "jwt",
            "archived",
            "resets",
        }
        return any(field in value for field in known_fields)

    def _merge_account_payloads(
        self,
        existing_value: Optional[dict],
        new_value: object,
    ) -> dict:
        if not isinstance(existing_value, dict):
            existing_value = {}
        if not isinstance(new_value, dict):
            return dict(existing_value)

        merged = dict(existing_value)

        existing_last_fetched = existing_value.get("last_fetched", 0) or 0
        new_last_fetched = new_value.get("last_fetched", 0) or 0
        prefer_newer_usage = new_last_fetched >= existing_last_fetched

        if prefer_newer_usage:
            for field in (
                "reset_ts",
                "used_percent",
                "primary_window",
                "secondary_window",
                "short_window",
                "weekly_window",
                "last_fetched",
            ):
                if field in new_value:
                    merged[field] = new_value[field]

        new_auto_fetch = new_value.get("auto_fetch")
        existing_auto_fetch = existing_value.get("auto_fetch")
        if new_auto_fetch not in (None, "", "None"):
            merged["auto_fetch"] = new_auto_fetch
        elif existing_auto_fetch not in (None, ""):
            merged["auto_fetch"] = existing_auto_fetch

        if "jwt" in new_value:
            merged["jwt"] = new_value["jwt"]

        if "archived" in new_value:
            merged["archived"] = new_value["archived"]
        elif "archived" in existing_value:
            merged["archived"] = existing_value["archived"]

        return merged
