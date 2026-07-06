import os
import sys

from .version import get_app_version


APP_TITLE = "Codex Account Monitor"
APP_NAME = "CodexMonitor"
APP_VERSION = get_app_version()
WINDOW_GEOMETRY = "980x460"
WINDOW_MIN_SIZE = (540, 320)
UPDATE_CHECK_INTERVAL_SECONDS = 21600

GITHUB_REPOSITORY = "koodev24/codex-monitor"
RELEASE_ASSET_NAME = "CodexMonitor-macOS.zip"
RELEASES_API_URL = f"https://api.github.com/repos/{GITHUB_REPOSITORY}/releases/latest"
RELEASES_PAGE_URL = f"https://github.com/{GITHUB_REPOSITORY}/releases/latest"
DEFAULT_INSTALL_DIR = os.path.expanduser("~/Applications")
DEFAULT_APP_INSTALL_PATH = os.path.join(DEFAULT_INSTALL_DIR, f"{APP_NAME}.app")
HTTP_USER_AGENT = f"{APP_NAME}/{APP_VERSION}"

# Automatically resolves to /Users/<your_username>/.codex/auth.json
AUTH_FILE_PATH = os.path.expanduser("~/.codex/auth.json")
AUTH_DIR = os.path.dirname(AUTH_FILE_PATH)

def _default_app_data_dir() -> str:
    if sys.platform == "darwin":
        return os.path.expanduser(f"~/Library/Application Support/{APP_NAME}")

    xdg_data_home = os.environ.get("XDG_DATA_HOME")
    if xdg_data_home:
        return os.path.join(os.path.expanduser(xdg_data_home), APP_NAME)
    return os.path.expanduser(f"~/.local/share/{APP_NAME}")


# App-owned storage. The active Codex auth file remains Codex-owned at ~/.codex/auth.json.
APP_DATA_DIR = _default_app_data_dir()
LOCAL_STORAGE_FILE = os.path.join(APP_DATA_DIR, "usage.json")
LOCAL_STORAGE_META_FILE = os.path.join(APP_DATA_DIR, "usage.meta.json")
LOCAL_LOG_FILE = os.path.join(APP_DATA_DIR, "activity.log")
AUTH_ACCOUNTS_DIR = os.path.join(APP_DATA_DIR, "accounts")

# Legacy paths used by older versions. They are migrated into APP_DATA_DIR at startup.
LEGACY_LOCAL_STORAGE_FILE = os.path.expanduser("~/.codex_usage_store.json")
LEGACY_LOCAL_STORAGE_META_FILE = os.path.expanduser("~/.codex_usage_store.meta.json")
LEGACY_LOCAL_LOG_FILE = os.path.expanduser("~/.codex_usage_store.log")
LEGACY_AUTH_ACCOUNTS_DIRS = (
    os.path.expanduser("~/.codex_usage_store.accounts"),
    os.path.expanduser("~/.codex/accounts"),
    os.path.expanduser("~/.codex/codex_monitor/accounts"),
)

# API URL
USAGE_API_URL = "https://chatgpt.com/backend-api/wham/usage"
RESET_CREDITS_API_URL = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits"
AUTH_REFRESH_URL = "https://auth.openai.com/oauth/token"
AUTH_REFRESH_CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
AUTH_REFRESH_INTERVAL_SECONDS = 8 * 24 * 3600

# Auto-fetch intervals
SEC_IN_MIN = 60
SEC_IN_HOUR = 3600

# Structured interval definitions for better maintainability
# (Value, Unit Seconds, Label Unit)
_INTERVAL_DEFS = [
    (15, SEC_IN_MIN, "Mins"),
    (1, SEC_IN_HOUR, "Hr"),
    (3, SEC_IN_HOUR, "Hrs"),
    (12, SEC_IN_HOUR, "Hrs"),
    (24, SEC_IN_HOUR, "Hrs"),
]

AUTO_FETCH_INTERVALS = {f"{v} {u}": v * s for v, s, u in _INTERVAL_DEFS}
AUTO_FETCH_OPTIONS = ["None", *AUTO_FETCH_INTERVALS.keys()]
