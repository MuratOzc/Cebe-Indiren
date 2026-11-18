import os
import sys
import time
import shutil
import subprocess
import logging

from .state import state
from .utils import resource_path

logger = logging.getLogger(__name__)


def _no_window_flags():
    return subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0


def trim_video_ffmpeg(input_path, start_sec, end_sec):
    try:
        if state.cancel_requested:
            return input_path

        base, ext = os.path.splitext(input_path)
        trimmed_path = f"{base}_trimmed{ext}"

        cmd = [shutil.which('ffmpeg') or 'ffmpeg', '-y']

        # Fast seeking before input is frame-accurate when re-encoding and avoids decoding preceding video.
        if start_sec and start_sec > 0:
            cmd += ['-ss', str(start_sec)]

        cmd += ['-i', input_path]

        if end_sec is not None:
            duration = end_sec - (start_sec or 0)
            if duration > 0:
                cmd += ['-t', str(duration)]

        is_audio = ext.lower() in ['.mp3', '.m4a', '.wav', '.aac', '.ogg', '.flac', '.wma']

        cmd += ['-map', '0:v?', '-map', '0:a?', '-map_metadata', '0']

        if is_audio:
            cmd += ['-c:v', 'copy']
            if ext.lower() == '.mp3':
                cmd += ['-c:a', 'libmp3lame', '-b:a', '192k']
            else:
                cmd += ['-c:a', 'aac', '-b:a', '192k']
        else:
            cmd += [
                '-c:v', 'libx264',
                '-preset', 'veryfast',
                '-crf', '18',
                '-pix_fmt', 'yuv420p',
                '-c:a', 'aac',
                '-b:a', '192k',
                '-movflags', '+faststart',
            ]

        cmd += [
            '-reset_timestamps', '1',
            '-avoid_negative_ts', 'make_zero',
            trimmed_path,
        ]

        logger.info(f"Trimming video: {' '.join(cmd)}")
        state.progress_state.update({"status": "trimming", "speed": "N/A", "percent": "100%", "size_info": "Video kesiliyor..."})

        state.active_ffmpeg_process = subprocess.Popen(
            cmd,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            creationflags=_no_window_flags()
        )

        _, stderr_data = state.active_ffmpeg_process.communicate()
        returncode = state.active_ffmpeg_process.returncode
        state.active_ffmpeg_process = None

        if returncode == 0 and os.path.exists(trimmed_path):
            try:
                os.replace(trimmed_path, input_path)
                logger.info("Trim successful, replaced original file.")
                return input_path
            except Exception as e:
                logger.warning(f"Could not replace original file with trimmed file: {e}")
                return trimmed_path
        else:
            stderr_text = stderr_data.decode(errors='ignore') if stderr_data else ''
            logger.error(f"FFmpeg trim failed (code {returncode}): {stderr_text}")
            if os.path.exists(trimmed_path):
                try:
                    os.remove(trimmed_path)
                except Exception:
                    pass
            return input_path
    except Exception as e:
        logger.error(f"Trim error: {e}")
        return input_path
    finally:
        state.active_ffmpeg_process = None


def kill_active_ffmpeg_processes():
    """Terminate ffmpeg processes started by this application without killing other system processes."""
    if state.active_ffmpeg_process and state.active_ffmpeg_process.poll() is None:
        try:
            state.active_ffmpeg_process.kill()
            state.active_ffmpeg_process.wait(timeout=3)
            logger.info("Killed active ffmpeg process.")
        except Exception:
            pass
        finally:
            state.active_ffmpeg_process = None

    if sys.platform == "win32":
        try:
            my_pid = os.getpid()
            cmd = f'Get-CimInstance Win32_Process | Where-Object {{ $_.ParentProcessId -eq {my_pid} -and $_.Name -like "*ffmpeg*" }} | Stop-Process -Force'
            subprocess.run(
                ["powershell", "-NoProfile", "-Command", cmd],
                capture_output=True,
                check=False,
                creationflags=_no_window_flags(),
            )
        except Exception:
            pass


def cleanup_interrupted_downloads(download_dir, kill_processes=True, since_time=None):
    try:
        if os.path.exists(download_dir):
            logger.info(f"Cleaning up interrupted download temp files in {download_dir}...")

            if kill_processes:
                kill_active_ffmpeg_processes()
                time.sleep(0.5)

            for root, dirs, files in os.walk(download_dir):
                for file in files:
                    name = file.lower()
                    if name.endswith(('.part', '.ytdl', '.temp', '.tmp')) or '.part-frag' in name or '.temp.' in name:
                        file_path = os.path.join(root, file)
                        if since_time is not None:
                            try:
                                if os.path.getmtime(file_path) < since_time - 2:
                                    continue
                            except OSError:
                                continue
                        for attempt in range(5):
                            try:
                                if os.path.exists(file_path):
                                    os.remove(file_path)
                                    if not os.path.exists(file_path):
                                        logger.info(f"Successfully deleted: {file}")
                                        break
                                    else:
                                        raise Exception("File still exists")
                            except Exception:
                                if attempt < 4:
                                    time.sleep(0.5)
    except Exception as e:
        logger.error(f"Cleanup failed: {e}")


def ensure_bundled_ffmpeg_in_path():
    if shutil.which("ffmpeg"):
        logger.info("FFmpeg found in system PATH.")
        return True

    bundled_ffmpeg_bin = resource_path(os.path.join("ffmpeg", "bin"))
    bundled_ffmpeg = os.path.join(bundled_ffmpeg_bin, "ffmpeg.exe" if sys.platform == "win32" else "ffmpeg")
    if os.path.exists(bundled_ffmpeg):
        logger.info(f"Found bundled FFmpeg: {bundled_ffmpeg_bin}. Adding to PATH...")
        os.environ["PATH"] += os.pathsep + bundled_ffmpeg_bin
        if shutil.which("ffmpeg"):
            logger.info("FFmpeg successfully added to PATH from bundled resources.")
            return True

    logger.warning("FFmpeg NOT found! Video merging will fail or result in lower quality/no audio.")
    return False


def monitor_heartbeat():
    while True:
        time.sleep(1)

        # Check if shutdown beacon was sent and not cancelled by a new heartbeat within 5 seconds
        if state.shutdown_requested_at is not None:
            if time.time() - state.shutdown_requested_at >= 5.0:
                logger.info("Explicit shutdown requested and grace period elapsed. Shutting down...")
                if state.download_active:
                    state.cancel_requested = True
                    time.sleep(1)
                    cleanup_interrupted_downloads(state.download_dir, since_time=state.download_started_at)
                os._exit(0)
                break

        if not state.heartbeat_received:
            continue

        current_status = state.progress_state.get("status")
        if current_status in {"starting", "downloading", "merging", "trimming"}:
            timeout = 180
        else:
            timeout = 60

        if time.time() - state.last_heartbeat_time > timeout:
            logger.info(f"No heartbeat received for {timeout} seconds. Shutting down...")
            if state.download_active:
                state.cancel_requested = True
                time.sleep(3)
                cleanup_interrupted_downloads(state.download_dir, since_time=state.download_started_at)
            os._exit(0)
            break


def open_browser_app(url):
    edge_paths = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\Edge\Application\msedge.exe")
    ]

    edge_exe = None
    for path in edge_paths:
        if os.path.exists(path):
            edge_exe = path
            break

    if edge_exe:
        try:
            logger.info(f"Found Edge at: {edge_exe}")
            subprocess.Popen([edge_exe, f"--app={url}"], creationflags=_no_window_flags())
            return True
        except Exception as e:
            logger.error(f"Failed to launch Edge exe: {e}")

    try:
        import webbrowser
        webbrowser.open(url)
        return True
    except Exception as e:
        logger.error(f"All browser launch attempts failed: {e}")
        return False


def start_server():
    import uvicorn
    try:
        from app import app
        config = uvicorn.Config(app, host="127.0.0.1", port=4321, log_level="info")
        server = uvicorn.Server(config)
        server.run()
    except Exception as e:
        logger.error(f"Uvicorn failed to start: {e}")
