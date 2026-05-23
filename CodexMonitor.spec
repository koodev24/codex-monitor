# -*- mode: python ; coding: utf-8 -*-

import os
import sys


a = Analysis(
    ['codex_monitor.py'],
    pathex=[],
    binaries=[],
    datas=[
        (
            'codex_monitor_app/assets/fonts',
            'codex_monitor_app/assets/fonts',
        ),
    ],
    hiddenimports=[],
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
