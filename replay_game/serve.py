#!/usr/bin/env python3
"""Offline launcher + loopback server for the Battle Plan GAME-AUTHENTIC replay UI (Layer 2).

Self-contained, stdlib only, no network. Serves the game UI and the JSON store
over ``http://127.0.0.1`` (a real HTTP origin is required so ``fetch()`` works;
``file://`` would be blocked by local-file CORS). Bound to loopback only.

Rooted at the REPO so every asset path resolves unchanged (read-only):

    /                                  -> replay_game/static/index.html (the game UI)
    /replay_game/static/...            -> UI assets (app.js/css, map.js, sprites.js, ...)
    /replay_game/output/...            -> generated centroids cache (if built)
    /game_assets/sprites.json          -> the sprite manifest
    /game_assets/assets/...            -> the converted art PNGs (gitignored on disk)
    /game_assets/gui/...               -> parsed real window layouts (geometry)
    /map_pipeline/assets/...           -> province raster + province_to_state + colors
    /replay/snapshots/...              -> the real per-game snapshot store (read-only)
    /replay_ui/...                     -> Layer-1 stats UI (iframed by the layer toggle)
    /replay_render/output/...          -> existing map scrubber (optional base reuse)

NOTE: Layer 2 has NO Chart.js / no external dependency of its own. When the user
flips to the Layer-1 iframe (/replay_ui/app.html), THAT page pulls its own assets
against this same loopback origin; we don't serve or load anything for Layer 2 from
outside replay_game/ + the read-only asset dirs above. (The repo-rooted static
handler will of course serve any file the iframe requests, same as replay_ui's own
server does — that's the iframe's dependency, not ours.)

A small REST-ish JSON API (namespaced under /replay_game/api/ to avoid colliding
with replay_ui's /api/*; we also alias /api/games for convenience):

    /replay_game/api/games                      discovered games {store:[...], demo:[...]}
    /replay_game/api/snapshots/<root>/<game>    the game index (ordered snapshot manifest)
    /replay_game/api/snapshot/<root>/<game>/<file>  one full snapshot json (sanitized)
    /replay_game/api/timeline/<root>/<game>     the event timeline (or {events:[]} if none)

``<root>`` is ``store`` (replay/snapshots) or ``demo`` (replay_ui/demo_snapshots).
The UI never sees a filesystem path.

Usage:
    python serve.py [--port 8090] [--no-open]
"""
from __future__ import annotations

import argparse
import json
import os
import posixpath
import re
import sys
import threading
import webbrowser
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit, unquote

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
if REPO not in sys.path:
    sys.path.insert(0, REPO)

STORE_ROOTS = {
    "store": os.path.join(REPO, "replay", "snapshots"),
    "demo": os.path.join(REPO, "replay_ui", "demo_snapshots"),
}

# game_id / file tokens we will accept in API paths (no traversal, no separators).
_SAFE_TOKEN = re.compile(r"^[A-Za-z0-9_.\-]+$")


def _discover_games() -> dict:
    out = {"store": [], "demo": []}
    for root_key, root in STORE_ROOTS.items():
        if not os.path.isdir(root):
            continue
        for name in sorted(os.listdir(root)):
            gdir = os.path.join(root, name)
            idx = os.path.join(gdir, "index.json")
            if os.path.isfile(idx):
                try:
                    with open(idx, encoding="utf-8") as fh:
                        index = json.load(fh)
                except (OSError, ValueError):
                    index = {}
                snaps = index.get("snapshots", [])
                out[root_key].append({
                    "game_id": name,
                    "root": root_key,
                    "n_snapshots": len(snaps),
                    "first_date": snaps[0]["date"] if snaps else None,
                    "last_date": snaps[-1]["date"] if snaps else None,
                    "has_timeline": os.path.isfile(os.path.join(gdir, "timeline.json")),
                })
    return out


def _game_dir(root_key: str, game_id: str):
    root = STORE_ROOTS.get(root_key)
    if root is None or not _SAFE_TOKEN.match(game_id):
        return None
    gdir = os.path.normpath(os.path.join(root, game_id))
    root_abs = os.path.normpath(root)
    if gdir != root_abs and not gdir.startswith(root_abs + os.sep):
        return None
    return gdir if os.path.isdir(gdir) else None


def _safe_join(root: str, url_path: str):
    """Resolve url_path under root, refusing traversal."""
    rel = posixpath.normpath(unquote(url_path)).lstrip("/")
    full = os.path.normpath(os.path.join(root, rel.replace("/", os.sep)))
    root_abs = os.path.normpath(root)
    if full != root_abs and not full.startswith(root_abs + os.sep):
        return None
    return full


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=REPO, **k)

    def log_message(self, *a):  # quieter console
        pass

    def _send_json(self, obj, code=200):
        body = json.dumps(obj).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    # --- API ---------------------------------------------------------------
    def _api(self, path: str) -> bool:
        """Handle /replay_game/api/* (and the /api/games alias). Returns True if handled."""
        if path == "/api/games" or path == "/replay_game/api/games":
            self._send_json(_discover_games()); return True
        if not path.startswith("/replay_game/api/"):
            return False
        parts = path[len("/replay_game/api/"):].split("/")
        kind = parts[0] if parts else ""

        if kind == "snapshots" and len(parts) >= 3:
            gdir = _game_dir(parts[1], parts[2])
            if gdir is None:
                self._send_json({"error": "bad game"}, 404); return True
            idx = os.path.join(gdir, "index.json")
            self._send_json(self._read_json(idx) or {"snapshots": []}); return True

        if kind == "snapshot" and len(parts) >= 4:
            gdir = _game_dir(parts[1], parts[2])
            fname = parts[3]
            if gdir is None or not _SAFE_TOKEN.match(fname) or not fname.endswith(".snapshot.json"):
                self._send_json({"error": "bad snapshot path"}, 404); return True
            fpath = os.path.join(gdir, fname)
            data = self._read_json(fpath)
            if data is None:
                self._send_json({"error": "not found"}, 404); return True
            self._send_json(data); return True

        if kind == "timeline" and len(parts) >= 3:
            gdir = _game_dir(parts[1], parts[2])
            if gdir is None:
                self._send_json({"error": "bad game"}, 404); return True
            tl = self._read_json(os.path.join(gdir, "timeline.json"))
            self._send_json(tl or {"events": [], "event_count": 0}); return True

        self._send_json({"error": "unknown api"}, 404); return True

    @staticmethod
    def _read_json(path):
        if not os.path.isfile(path):
            return None
        try:
            with open(path, encoding="utf-8") as fh:
                return json.load(fh)
        except (OSError, ValueError):
            return None

    def do_GET(self):
        path = urlsplit(self.path).path
        if path in ("/", ""):
            self.path = "/replay_game/static/index.html"
            return super().do_GET()
        if path == "/api/games" or path.startswith("/replay_game/api/"):
            if self._api(path):
                return
        # static — defensive traversal check (SimpleHTTPRequestHandler also normalizes)
        if _safe_join(REPO, path) is None:
            self.send_error(403, "forbidden"); return
        return super().do_GET()


def _free_port(preferred: int) -> int:
    import socket
    for port in [preferred] + list(range(preferred + 1, preferred + 20)):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            try:
                s.bind(("127.0.0.1", port))
                return port
            except OSError:
                continue
    return preferred


def _assets_present() -> bool:
    return os.path.isdir(os.path.join(REPO, "game_assets", "assets", "icons"))


def main(argv=None):
    ap = argparse.ArgumentParser(description="Battle Plan game-authentic replay UI (offline, loopback).")
    ap.add_argument("--port", type=int, default=8090)
    ap.add_argument("--no-open", action="store_true")
    args = ap.parse_args(argv)

    if not _assets_present():
        print("[warn] game_assets/assets/ not found on disk — sprites will be placeholders.")
        print("       Run:  python game_assets/extract.py   (from the repo root) to convert the art.")

    port = _free_port(args.port)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    url = f"http://127.0.0.1:{port}/"
    print("=" * 60)
    print("  Battle Plan - GAME-AUTHENTIC REPLAY UI (Layer 2)")
    print("  offline, 127.0.0.1 only")
    print("=" * 60)
    print(f"  Open:  {url}")
    print("  Keep this window open while you use the UI. Ctrl+C to stop.")
    print("=" * 60)
    if not args.no_open:
        threading.Timer(0.6, lambda: webbrowser.open(url)).start()
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n(server stopped)")
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
