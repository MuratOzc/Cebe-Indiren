import os
import sys
import json
import logging
from logging.handlers import RotatingFileHandler


def get_app_data_dir():
    if sys.platform == 'win32':
        base = os.environ.get('APPDATA', os.path.expanduser('~\\AppData\\Roaming'))
    else:
        base = os.path.expanduser('~/.config')

    app_dir = os.path.join(base, 'VideoIndiren')
    if not os.path.exists(app_dir):
        os.makedirs(app_dir)
    return app_dir


APP_DATA_DIR = get_app_data_dir()
LOG_FILE = os.path.join(APP_DATA_DIR, 'app.log')
CONFIG_FILE = os.path.join(APP_DATA_DIR, 'config.json')


def setup_logging():
    handler = RotatingFileHandler(
        LOG_FILE,
        maxBytes=1_000_000,
        backupCount=2,
        encoding='utf-8'
    )
    logging.basicConfig(
        handlers=[handler],
        level=logging.INFO,
        format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
    )


def redirect_stdio_if_frozen():
    if getattr(sys, 'frozen', False):
        f = open(os.devnull, 'w')
        sys.stdout = f
        sys.stderr = f
        sys.stdin = open(os.devnull, 'r')


def load_config():
    try:
        if os.path.exists(CONFIG_FILE):
            with open(CONFIG_FILE, 'r') as f:
                data = json.load(f)
                return data.get("download_dir"), data.get("quality", "best")
    except Exception as e:
        logging.getLogger(__name__).error(f"Failed to load config: {e}")
    return None, "best"


def save_config(download_dir=None, quality=None):
    try:
        current_dir, current_quality = load_config()
        new_dir = download_dir if download_dir is not None else current_dir
        new_quality = quality if quality is not None else current_quality
        with open(CONFIG_FILE, 'w') as f:
            json.dump({"download_dir": new_dir, "quality": new_quality}, f)
    except Exception as e:
        logging.getLogger(__name__).error(f"Failed to save config: {e}")


def initialize_download_dir(saved_dir):
    if saved_dir and os.path.exists(saved_dir):
        return saved_dir

    downloads = os.path.join(os.path.expanduser("~"), "Downloads")
    if os.path.exists(downloads):
        return downloads

    local = os.path.join(os.getcwd(), "downloads")
    os.makedirs(local, exist_ok=True)
    return local
