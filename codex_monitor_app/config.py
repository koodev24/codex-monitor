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
RELEASE_ASSET_NAMES = {
    "darwin": "CodexMonitor-macOS.zip",
    "win32": "CodexMonitor-Windows.zip",
}
_RELEASE_PLATFORM_KEY = "win32" if sys.platform.startswith("win") else sys.platform
RELEASE_ASSET_NAME = RELEASE_ASSET_NAMES.get(
    _RELEASE_PLATFORM_KEY,
    RELEASE_ASSET_NAMES["darwin"],
)
RELEASES_API_URL = f"https://api.github.com/repos/{GITHUB_REPOSITORY}/releases/latest"
RELEASES_PAGE_URL = f"https://github.com/{GITHUB_REPOSITORY}/releases/latest"
DEFAULT_INSTALL_DIR = (
    os.path.join(os.environ.get("LOCALAPPDATA", os.path.expanduser("~")), APP_NAME)
    if os.name == "nt"
    else os.path.expanduser("~/Applications")
)
DEFAULT_APP_INSTALL_PATH = os.path.join(
    DEFAULT_INSTALL_DIR,
    f"{APP_NAME}.exe" if os.name == "nt" else f"{APP_NAME}.app",
)
HTTP_USER_AGENT = f"{APP_NAME}/{APP_VERSION}"

# Automatically resolves to /Users/<your_username>/.codex/auth.json
AUTH_FILE_PATH = os.path.expanduser("~/.codex/auth.json")
AUTH_DIR = os.path.dirname(AUTH_FILE_PATH)

# Lightweight local storage equivalent
LOCAL_STORAGE_FILE = os.path.expanduser("~/.codex_usage_store.json")
LOCAL_STORAGE_META_FILE = os.path.expanduser("~/.codex_usage_store.meta.json")
LOCAL_LOG_FILE = os.path.expanduser("~/.codex_usage_store.log")

# API URL
USAGE_API_URL = "https://chatgpt.com/backend-api/wham/usage"

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
