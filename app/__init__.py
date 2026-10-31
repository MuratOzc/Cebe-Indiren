import os
from fastapi import FastAPI
from fastapi.staticfiles import StaticFiles

from .utils import resource_path
from .config import load_config, initialize_download_dir
from .state import state
from . import routes

from urllib.parse import urlparse
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.responses import Response

app = FastAPI()

ALLOWED_HOSTS = {"127.0.0.1:4321", "localhost:4321", "127.0.0.1", "localhost"}


class LocalhostSecurityMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        host = request.headers.get("host", "").lower()
        if host and host not in ALLOWED_HOSTS:
            return Response("Forbidden Host", status_code=403)

        origin = request.headers.get("origin")
        if origin:
            parsed = urlparse(origin)
            if parsed.netloc.lower() not in ALLOWED_HOSTS:
                return Response("Forbidden Origin", status_code=403)

        referer = request.headers.get("referer")
        if referer:
            parsed = urlparse(referer)
            if parsed.netloc.lower() not in ALLOWED_HOSTS:
                return Response("Forbidden Referer", status_code=403)

        return await call_next(request)


app.add_middleware(LocalhostSecurityMiddleware)

static_dir = resource_path("static")
if not os.path.exists(static_dir):
    static_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "static")
app.mount("/static", StaticFiles(directory=static_dir), name="static")

saved_dir, _ = load_config()
state.download_dir = initialize_download_dir(saved_dir)

app.include_router(routes.router)
