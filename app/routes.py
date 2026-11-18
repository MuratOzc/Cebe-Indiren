import os
import time
import logging
import threading
import tkinter as tk
from tkinter import filedialog

import anyio
from fastapi import APIRouter, HTTPException
from fastapi.responses import FileResponse

from .state import state
from .config import load_config, save_config
from .models import DownloadRequest, InfoRequest, QualityRequest, OpenFolderRequest
from .downloader import run_download, get_video_info
from .ffmpeg_manager import kill_active_ffmpeg_processes
from .ytdlp_updater import install_update, restart_app

logger = logging.getLogger(__name__)

router = APIRouter()


@router.get("/api/progress")
async def get_progress():
    return state.progress_state


@router.post("/api/cancel")
def cancel_download():
    state.cancel_requested = True
    kill_active_ffmpeg_processes()
    return {"status": "cancel_requested"}


@router.post("/api/download")
async def download_video(request: DownloadRequest):
    if state.download_active:
        raise HTTPException(status_code=409, detail="Zaten aktif bir indirme var.")

    download_dir = state.download_dir
    if request.download_dir and os.path.exists(request.download_dir):
        download_dir = request.download_dir

    state.download_active = True
    try:
        result = await run_download(request.url, request, download_dir)
        return result
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))
    finally:
        state.download_active = False


@router.get("/api/update_status")
async def get_update_status():
    return state.update_state


@router.post("/api/update_ytdlp")
async def update_ytdlp():
    if state.progress_state.get("status") in {"starting", "downloading", "merging", "trimming"}:
        raise HTTPException(status_code=409, detail="İndirme sürerken güncelleme yapılamaz.")
    latest = state.update_state.get("latest")
    if state.update_state.get("status") != "available" or not latest:
        raise HTTPException(status_code=400, detail="Güncelleme yok.")

    try:
        await anyio.to_thread.run_sync(lambda: install_update(latest))
    except Exception as e:
        logger.error(f"yt-dlp update failed: {e}")
        raise HTTPException(status_code=500, detail=f"Güncelleme başarısız: {e}")

    threading.Timer(1.0, restart_app).start()
    return {"status": "restarting"}


@router.get("/api/heartbeat")
async def heartbeat():
    state.last_heartbeat_time = time.time()
    state.heartbeat_received = True
    # Reset any pending shutdown requested via beforeunload beacon (e.g. from page reload)
    state.shutdown_requested_at = None
    return {"status": "ok"}


@router.post("/api/shutdown")
async def shutdown_app():
    state.shutdown_requested_at = time.time()
    return {"status": "shutting_down"}


@router.get("/api/config")
async def get_config():
    _, quality = load_config()
    return {"default_dir": state.download_dir, "quality": quality}


@router.post("/api/info")
async def get_video_info_route(request: InfoRequest):
    return await anyio.to_thread.run_sync(lambda: get_video_info(request.url))


@router.post("/api/set_quality")
async def set_quality(request: QualityRequest):
    save_config(quality=request.quality)
    return {"status": "ok"}


@router.get("/api/select_folder")
def select_folder():
    root = tk.Tk()
    root.withdraw()
    root.attributes('-topmost', True)
    folder_path = filedialog.askdirectory(initialdir=state.download_dir)
    root.destroy()
    if folder_path:
        state.download_dir = folder_path
        save_config(folder_path)
    return {"path": folder_path}


def _is_allowed_open_path(path):
    norm_path = os.path.normcase(os.path.normpath(os.path.abspath(path)))
    if state.last_downloaded_path:
        norm_last = os.path.normcase(os.path.normpath(os.path.abspath(state.last_downloaded_path)))
        if norm_path == norm_last:
            return True
    if state.download_dir:
        norm_dir = os.path.normcase(os.path.normpath(os.path.abspath(state.download_dir)))
        try:
            if os.path.commonpath([norm_path, norm_dir]) == norm_dir:
                return True
        except ValueError:
            pass
    return False


@router.post("/api/open_folder")
async def open_folder(request: OpenFolderRequest):
    try:
        path = os.path.normpath(os.path.abspath(request.file_path))
        if not _is_allowed_open_path(path):
            raise HTTPException(status_code=403, detail="Access denied")

        if os.path.exists(path):
            if os.path.isfile(path) and os.name == "nt":
                import subprocess
                subprocess.Popen(["explorer", "/select,", path])
            else:
                target_dir = path if os.path.isdir(path) else os.path.dirname(path)
                os.startfile(target_dir)
            return {"status": "success"}
            
        # Fallback to parent directory (handles playlists with dummy file paths)
        target_dir = os.path.dirname(path)
        if target_dir and os.path.exists(target_dir) and _is_allowed_open_path(target_dir):
            os.startfile(target_dir)
            return {"status": "success"}
            
        return {"status": "error", "message": "File not found"}
    except HTTPException:
        raise
    except Exception as e:
        logger.error(f"Failed to open folder: {e}")
        return {"status": "error", "message": str(e)}


@router.get("/")
async def read_root():
    from .utils import resource_path
    static_dir = resource_path("static")
    if not os.path.exists(static_dir):
        static_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "static")
    logger.info("Root page accessed")
    return FileResponse(os.path.join(static_dir, 'index.html'))
