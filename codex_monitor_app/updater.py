import json
import os
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Tuple

import certifi

from .config import (
    APP_VERSION,
    DEFAULT_APP_INSTALL_PATH,
    DEFAULT_INSTALL_DIR,
    GITHUB_REPOSITORY,
    HTTP_USER_AGENT,
    RELEASE_ASSET_NAME,
    RELEASE_ASSET_NAMES,
    RELEASES_API_URL,
    RELEASES_PAGE_URL,
)


@dataclass(frozen=True)
class ReleaseInfo:
    tag_name: str
    version: str
    asset_name: str
    asset_url: str
    html_url: str


class UpdateError(RuntimeError):
    """Raised when the application update workflow cannot continue."""


def _platform_key() -> str:
    if sys.platform.startswith("win"):
        return "win32"
    return sys.platform


def release_asset_name() -> str:
    asset_name = RELEASE_ASSET_NAMES.get(_platform_key())
    if asset_name:
        return asset_name
    raise UpdateError("In-app updates are only supported on macOS and Windows.")


def _ssl_context() -> ssl.SSLContext:
    return ssl.create_default_context(cafile=certifi.where())


def version_tuple(version: str) -> Tuple[int, ...]:
    normalized = version.strip().lstrip("vV")
    parts = [int(piece) for piece in re.findall(r"\d+", normalized)]
    return tuple(parts or [0])


def is_newer_version(candidate: str, current: str) -> bool:
    left = version_tuple(candidate)
    right = version_tuple(current)
    max_len = max(len(left), len(right))
    padded_left = left + (0,) * (max_len - len(left))
    padded_right = right + (0,) * (max_len - len(right))
    return padded_left > padded_right


def _latest_release_from_page() -> ReleaseInfo:
    asset_name = release_asset_name()
    request = urllib.request.Request(
        RELEASES_PAGE_URL,
        headers={"User-Agent": HTTP_USER_AGENT},
    )

    with urllib.request.urlopen(request, context=_ssl_context(), timeout=15) as response:
        html_url = response.geturl()

    match = re.search(r"/releases/tag/([^/?#]+)", html_url)
    if not match:
        raise UpdateError("Could not resolve the latest release tag.")

    tag_name = urllib.parse.unquote(match.group(1))
    encoded_tag = urllib.parse.quote(tag_name, safe="")
    encoded_asset = urllib.parse.quote(asset_name, safe="")
    asset_url = (
        f"https://github.com/{GITHUB_REPOSITORY}/releases/download/"
        f"{encoded_tag}/{encoded_asset}"
    )

    return ReleaseInfo(
        tag_name=tag_name,
        version=tag_name.lstrip("vV") or APP_VERSION,
        asset_name=asset_name,
        asset_url=asset_url,
        html_url=html_url,
    )


def _latest_release_from_api() -> ReleaseInfo:
    asset_name = release_asset_name()
    request = urllib.request.Request(
        RELEASES_API_URL,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": HTTP_USER_AGENT,
        },
    )

    with urllib.request.urlopen(request, context=_ssl_context(), timeout=15) as response:
        payload = json.load(response)

    assets = payload.get("assets", [])
    asset = next(
        (item for item in assets if item.get("name") == asset_name),
        None,
    )
    if not asset:
        raise UpdateError(f"Release asset '{asset_name}' was not found.")

    return ReleaseInfo(
        tag_name=payload.get("tag_name", ""),
        version=payload.get("tag_name", "").lstrip("vV") or APP_VERSION,
        asset_name=asset["name"],
        asset_url=asset["browser_download_url"],
        html_url=payload.get("html_url", ""),
    )


def fetch_latest_release() -> ReleaseInfo:
    try:
        return _latest_release_from_api()
    except urllib.error.HTTPError as error:
        if error.code in (403, 429):
            return _latest_release_from_page()
        raise


def download_release_asset(release: ReleaseInfo, target_dir: str) -> str:
    os.makedirs(target_dir, exist_ok=True)
    archive_path = os.path.join(target_dir, release.asset_name)
    request = urllib.request.Request(
        release.asset_url,
        headers={"User-Agent": HTTP_USER_AGENT},
    )

    with urllib.request.urlopen(
        request,
        context=_ssl_context(),
        timeout=60,
    ) as response, open(
        archive_path,
        "wb",
    ) as archive_file:
        shutil.copyfileobj(response, archive_file)

    return archive_path


def _extract_zip(archive_path: str, target_dir: str) -> None:
    os.makedirs(target_dir, exist_ok=True)
    if sys.platform == "darwin":
        subprocess.run(
            ["/usr/bin/ditto", "-x", "-k", archive_path, target_dir],
            check=True,
        )
        return

    with zipfile.ZipFile(archive_path) as archive:
        archive.extractall(target_dir)


def extract_app_bundle(archive_path: str, target_dir: str) -> str:
    _extract_zip(archive_path, target_dir)

    for root, dirnames, _filenames in os.walk(target_dir):
        for dirname in dirnames:
            if dirname.endswith(".app"):
                return os.path.join(root, dirname)

    raise UpdateError("Downloaded archive did not contain a macOS app bundle.")


def extract_windows_executable(archive_path: str, target_dir: str) -> str:
    _extract_zip(archive_path, target_dir)

    expected_name = Path(DEFAULT_APP_INSTALL_PATH).name.lower()
    fallback_executable = None
    for root, _dirnames, filenames in os.walk(target_dir):
        for filename in filenames:
            if not filename.lower().endswith(".exe"):
                continue

            executable_path = os.path.join(root, filename)
            if filename.lower() == expected_name:
                return executable_path
            if fallback_executable is None:
                fallback_executable = executable_path

    if fallback_executable:
        return fallback_executable

    raise UpdateError("Downloaded archive did not contain a Windows executable.")


def extract_release_artifact(archive_path: str, target_dir: str) -> str:
    if sys.platform == "darwin":
        return extract_app_bundle(archive_path, target_dir)
    if sys.platform.startswith("win"):
        return extract_windows_executable(archive_path, target_dir)

    raise UpdateError("In-app updates are only supported on macOS and Windows.")


def _current_windows_executable_path() -> str:
    if getattr(sys, "frozen", False):
        return str(Path(sys.executable).resolve())
    return DEFAULT_APP_INSTALL_PATH


def _current_macos_bundle_path() -> str:
    executable_path = Path(sys.executable).resolve()
    for parent in executable_path.parents:
        if parent.suffix == ".app":
            return str(parent)
    return DEFAULT_APP_INSTALL_PATH


def current_bundle_path() -> str:
    if sys.platform.startswith("win"):
        return _current_windows_executable_path()

    return _current_macos_bundle_path()


def resolve_install_target() -> str:
    install_path = current_bundle_path()
    parent_dir = os.path.dirname(install_path)
    if os.path.isdir(parent_dir) and os.access(parent_dir, os.W_OK):
        return install_path

    os.makedirs(DEFAULT_INSTALL_DIR, exist_ok=True)
    return DEFAULT_APP_INSTALL_PATH


def prepare_update(release: ReleaseInfo) -> Tuple[str, str, str]:
    temp_root = tempfile.mkdtemp(prefix="codex-monitor-update-")
    archive_path = download_release_asset(release, temp_root)
    extracted_root = os.path.join(temp_root, "expanded")
    source_app = extract_release_artifact(archive_path, extracted_root)
    target_app = resolve_install_target()
    return source_app, target_app, temp_root


def _install_macos_update_and_restart(
    source_app: str,
    target_app: str,
    temp_root: str,
) -> None:
    script = """
while kill -0 "$1" >/dev/null 2>&1; do
  sleep 1
done
mkdir -p "$(dirname "$2")"
rm -rf "$2"
/usr/bin/ditto "$3" "$2"
/usr/bin/xattr -dr com.apple.quarantine "$2" >/dev/null 2>&1 || true
/usr/bin/open "$2"
rm -rf "$4"
"""
    subprocess.Popen(
        ["/bin/sh", "-c", script, "updater", str(os.getpid()), target_app, source_app, temp_root],
        start_new_session=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def _install_windows_update_and_restart(
    source_app: str,
    target_app: str,
    temp_root: str,
) -> None:
    script_path = os.path.join(temp_root, "install-update.cmd")
    script = r"""@echo off
setlocal
:wait
tasklist /FI "PID eq %~1" 2>NUL | find "%~1" >NUL
if not errorlevel 1 (
  timeout /T 1 /NOBREAK >NUL
  goto wait
)
if not exist "%~dp2" mkdir "%~dp2"
copy /Y "%~3" "%~2" >NUL
start "" "%~2"
rmdir /S /Q "%~4"
"""
    Path(script_path).write_text(script, encoding="utf-8")
    creationflags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) | getattr(
        subprocess,
        "DETACHED_PROCESS",
        0,
    )
    subprocess.Popen(
        [
            "cmd.exe",
            "/c",
            script_path,
            str(os.getpid()),
            target_app,
            source_app,
            temp_root,
        ],
        creationflags=creationflags,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def install_update_and_restart(source_app: str, target_app: str, temp_root: str) -> None:
    if sys.platform.startswith("win"):
        _install_windows_update_and_restart(source_app, target_app, temp_root)
        return

    if sys.platform == "darwin":
        _install_macos_update_and_restart(source_app, target_app, temp_root)
        return

    raise UpdateError("In-app updates are only supported on macOS and Windows.")
