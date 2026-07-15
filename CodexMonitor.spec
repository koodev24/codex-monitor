# -*- mode: python ; coding: utf-8 -*-

import os
import sys

try:
    from PyInstaller.utils.hooks import collect_data_files, collect_dynamic_libs
except Exception:
    codex_sdk_datas = []
    codex_sdk_binaries = []
else:
    try:
        codex_sdk_datas = (
            collect_data_files('openai_codex')
            + collect_data_files('codex_cli_bin')
        )
        codex_sdk_binaries = (
            collect_dynamic_libs('openai_codex')
            + collect_dynamic_libs('codex_cli_bin')
        )
    except Exception:
        codex_sdk_datas = []
        codex_sdk_binaries = []


a = Analysis(
    ['codex_monitor.py'],
    pathex=[],
    binaries=codex_sdk_binaries,
    datas=[
        (
            'codex_monitor_app/assets/fonts',
            'codex_monitor_app/assets/fonts',
        ),
        *codex_sdk_datas,
    ],
    hiddenimports=['openai_codex', 'codex_cli_bin'],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='CodexMonitor',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
if sys.platform == 'darwin':
    app = BUNDLE(
        exe,
        name='CodexMonitor.app',
        icon=None,
        bundle_identifier=None,
        info_plist={
            'CFBundleShortVersionString': os.environ.get('CODEX_MONITOR_VERSION', '0.0.0-dev'),
            'CFBundleVersion': os.environ.get('CODEX_MONITOR_VERSION', '0.0.0-dev'),
        },
    )
