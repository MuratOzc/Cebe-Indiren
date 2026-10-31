"""Loads a user-updated yt-dlp zip (if present) ahead of the bundled copy.

Must stay free of imports from the `app` package: it has to run before anything imports yt_dlp.
PyInstaller's importer sits in sys.meta_path ahead of sys.path, so a meta_path hook is required.
"""
import importlib.machinery
import os
import sys

if sys.platform == 'win32':
    _base = os.environ.get('APPDATA', os.path.expanduser('~\\AppData\\Roaming'))
else:
    _base = os.path.expanduser('~/.config')

UPDATE_DIR = os.path.join(_base, 'VideoIndiren', 'yt_dlp_update')
ZIP_PATH = os.path.join(UPDATE_DIR, 'yt-dlp.zip')
STAMP_PATH = os.path.join(UPDATE_DIR, 'exe.stamp')


def exe_stamp():
    """Identifies the running exe build (None when not frozen) so an update made for an older exe can be dropped."""
    if not getattr(sys, 'frozen', False):
        return None
    st = os.stat(sys.executable)
    return f'{st.st_size}-{st.st_mtime_ns}'


def _discard_update():
    for path in (ZIP_PATH, STAMP_PATH):
        try:
            os.remove(path)
        except OSError:
            pass


class _ZipFinder:
    def find_spec(self, fullname, path=None, target=None):
        if fullname == 'yt_dlp':
            return importlib.machinery.PathFinder.find_spec(fullname, [ZIP_PATH])
        if fullname.startswith('yt_dlp.') and path and any(str(p).startswith(ZIP_PATH) for p in path):
            return importlib.machinery.PathFinder.find_spec(fullname, path)
        return None


def activate():
    if not os.path.isfile(ZIP_PATH):
        return
    stamp = exe_stamp()
    if stamp is not None:
        try:
            with open(STAMP_PATH) as f:
                saved = f.read().strip()
        except OSError:
            saved = None
        if saved != stamp:
            # The exe was replaced (new build) since this update was installed; its bundled yt-dlp may be newer.
            _discard_update()
            return
    finder = _ZipFinder()
    sys.meta_path.insert(0, finder)
    try:
        import yt_dlp  # noqa: F401
    except Exception:
        # Broken update: fall back to the bundled yt-dlp and discard the bad zip.
        sys.meta_path.remove(finder)
        for name in [m for m in sys.modules if m == 'yt_dlp' or m.startswith('yt_dlp.')]:
            del sys.modules[name]
        _discard_update()
