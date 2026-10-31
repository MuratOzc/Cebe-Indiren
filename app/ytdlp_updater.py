import hashlib
import json
import logging
import os
import re
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import zipfile

from yt_dlp.version import __version__ as CURRENT_VERSION

from ytdlp_override import UPDATE_DIR, ZIP_PATH, STAMP_PATH, exe_stamp
from .state import state

logger = logging.getLogger(__name__)

RELEASES_URL = "https://github.com/yt-dlp/yt-dlp/releases"
API_LATEST_URL = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest"
HEADERS = {"User-Agent": "VideoIndiren-updater"}
TAG_RE = re.compile(r"^\d{4}\.\d{2}\.\d{2}(\.\d+)?$")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _latest_from_redirect():
    # The web "latest" URL redirects to /tag/<version>; unlike the API it has no rate limit.
    opener = urllib.request.build_opener(_NoRedirect)
    try:
        opener.open(urllib.request.Request(f"{RELEASES_URL}/latest", headers=HEADERS), timeout=8)
    except urllib.error.HTTPError as e:
        location = e.headers.get("Location", "")
        if e.code in (301, 302, 303, 307, 308) and "/tag/" in location:
            return location.rsplit("/tag/", 1)[1]
        raise
    raise RuntimeError("Expected a redirect from releases/latest")


def _latest_from_api():
    req = urllib.request.Request(API_LATEST_URL, headers={**HEADERS, "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(req, timeout=8) as resp:
        return json.load(resp)["tag_name"]


def _version_tuple(version):
    return tuple(int(part) for part in version.split("."))


def _check():
    state.update_state = {"status": "checking", "current": CURRENT_VERSION, "latest": ""}
    for fetch in (_latest_from_redirect, _latest_from_api):
        for attempt in range(3):
            try:
                tag = fetch().strip()
                if not TAG_RE.match(tag):
                    raise ValueError(f"Unexpected version tag: {tag!r}")
                newer = _version_tuple(tag) > _version_tuple(CURRENT_VERSION)
                state.update_state = {
                    "status": "available" if newer else "up_to_date",
                    "current": CURRENT_VERSION,
                    "latest": tag,
                }
                logger.info(f"yt-dlp {CURRENT_VERSION} active, latest stable {tag} ({state.update_state['status']}).")
                return
            except Exception as e:
                logger.warning(f"yt-dlp update check via {fetch.__name__} failed (attempt {attempt + 1}): {e}")
                if attempt < 2:
                    time.sleep(attempt + 1)
    state.update_state = {"status": "failed", "current": CURRENT_VERSION, "latest": ""}


def start_update_check():
    threading.Thread(target=_check, daemon=True).start()


def install_update(tag):
    if not TAG_RE.match(tag):
        raise ValueError("Invalid version tag")
    base = f"{RELEASES_URL}/download/{tag}"
    os.makedirs(UPDATE_DIR, exist_ok=True)
    tmp_path = ZIP_PATH + ".tmp"

    with urllib.request.urlopen(urllib.request.Request(f"{base}/SHA2-256SUMS", headers=HEADERS), timeout=15) as resp:
        sums = resp.read().decode("utf-8", "replace")
    expected = None
    for line in sums.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[1].lstrip("*") == "yt-dlp":
            expected = parts[0].lower()
            break
    if not expected:
        raise ValueError("yt-dlp checksum not found")

    try:
        digest = hashlib.sha256()
        with urllib.request.urlopen(urllib.request.Request(f"{base}/yt-dlp", headers=HEADERS), timeout=30) as resp, \
                open(tmp_path, "wb") as f:
            while True:
                chunk = resp.read(65536)
                if not chunk:
                    break
                digest.update(chunk)
                f.write(chunk)
        if digest.hexdigest() != expected:
            raise ValueError("Checksum mismatch")
        with zipfile.ZipFile(tmp_path) as z:
            if "yt_dlp/version.py" not in z.namelist():
                raise ValueError("Downloaded file is not a yt-dlp package")
        os.replace(tmp_path, ZIP_PATH)
        stamp = exe_stamp()
        if stamp is not None:
            with open(STAMP_PATH, "w") as f:
                f.write(stamp)
    finally:
        if os.path.exists(tmp_path):
            os.remove(tmp_path)
    logger.info(f"Installed yt-dlp {tag} to {ZIP_PATH}")


def restart_app():
    if getattr(sys, "frozen", False):
        args = [sys.executable]
    else:
        args = [sys.executable, os.path.abspath(sys.argv[0])]
    # Without this, a PyInstaller onefile child would reuse this process's temp dir, which is deleted on exit.
    env = dict(os.environ, PYINSTALLER_RESET_ENVIRONMENT="1")
    flags = getattr(subprocess, "DETACHED_PROCESS", 0)
    subprocess.Popen(args + ["--restarted"], env=env, creationflags=flags, close_fds=True)
    os._exit(0)
