# -*- mode: python ; coding: utf-8 -*-


from PyInstaller.utils.hooks import collect_data_files, collect_submodules

a = Analysis(
    ['main.py'],
    pathex=[],
    binaries=[],
    datas=[('static', 'static'), ('ffmpeg', 'ffmpeg')] + collect_data_files('yt_dlp_ejs'),
    hiddenimports=['uvicorn.logging', 'uvicorn.loops', 'uvicorn.loops.auto', 'uvicorn.protocols', 'uvicorn.protocols.http', 'uvicorn.protocols.http.auto', 'uvicorn.protocols.websockets', 'uvicorn.lifespan', 'uvicorn.lifespan.on', 'yt_dlp'] + collect_submodules('yt_dlp_ejs'),
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='VideoIndiren_Debug',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
