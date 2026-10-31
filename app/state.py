import time


class DownloadState:
    def __init__(self):
        self.progress_state = {"percent": "0%", "speed": "0KB/s", "status": "idle", "playlist_info": ""}
        self.cancel_requested = False
        self.active_ffmpeg_process = None
        self.last_heartbeat_time = time.time() + 30.0
        self.heartbeat_received = False
        # Set when the UI window sends a shutdown beacon; cleared by the next heartbeat so a
        # page reload (which also fires the beacon) doesn't kill the app.
        self.shutdown_requested_at = None
        self.download_dir: str = ""
        self.setup_state = {"status": "pending", "progress": 0, "message": "", "ffmpeg_available": False}
        self.update_state = {"status": "checking", "current": "", "latest": ""}
        self.setup_complete = False
        self.setup_in_progress = False

    def reset_for_download(self):
        self.cancel_requested = False
        self.progress_state = {"percent": "0%", "speed": "0KB/s", "status": "starting", "playlist_info": ""}


state = DownloadState()
