import multiprocessing
import socket
import sys
import threading
import time
import urllib.request

# Must run before anything imports yt_dlp: prefers a user-updated yt-dlp over the bundled one.
import ytdlp_override
ytdlp_override.activate()

from app import app as app_instance
from app.config import redirect_stdio_if_frozen, setup_logging, FFMPEG_DIR
from app.state import state
from app.ffmpeg_manager import check_ffmpeg_async, cleanup_interrupted_downloads, monitor_heartbeat, open_browser_app, start_server
from app.ytdlp_updater import start_update_check


def wait_for_port_free(port, timeout=10):
    # After an update restart the old process may still hold the port for a moment.
    start = time.time()
    while time.time() - start < timeout:
        with socket.socket() as s:
            if s.connect_ex(("127.0.0.1", port)) != 0:
                return
        time.sleep(0.3)


def wait_for_server(url, timeout=15):
    start = time.time()
    while time.time() - start < timeout:
        try:
            urllib.request.urlopen(url, timeout=1)
            return True
        except Exception:
            time.sleep(0.3)
    return False


if __name__ == '__main__':
    multiprocessing.freeze_support()

    redirect_stdio_if_frozen()
    setup_logging()

    restarted = '--restarted' in sys.argv
    if restarted:
        wait_for_port_free(4321)

    start_update_check()

    check_ffmpeg_async(FFMPEG_DIR)

    cleanup_interrupted_downloads(state.download_dir, kill_processes=False)

    monitor_thread = threading.Thread(target=monitor_heartbeat, daemon=True)
    monitor_thread.start()

    server_thread = threading.Thread(target=start_server)
    server_thread.start()

    wait_for_server("http://127.0.0.1:4321")
    if not restarted:
        open_browser_app("http://127.0.0.1:4321")

    server_thread.join()
