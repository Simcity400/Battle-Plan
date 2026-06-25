#!/usr/bin/env python3
"""Offline launcher + loopback server for the Battle Plan REPLAY UI.

Self-contained, stdlib only, no network. Serves the replay UI and the JSON store
over ``http://127.0.0.1`` (a real HTTP origin is required — opening the app via
``file://`` would have ``fetch()`` blocked by the browser's local-file CORS rules,
Codex review #1/#2). Bound to loopback only.

What it serves (rooted at the REPO so the embedded map scrubber's absolute asset
paths like ``/map_pipeline/assets/province_ids_half.png`` resolve unchanged —
replay_render is NOT modified, Codex #3):

    /                         -> replay_ui/app.html (the 5-screen UI)
    /replay_ui/...            -> static UI assets (app.js, css), output series json
    /replay/snapshots/...     -> the real per-game store (read-only)
    /replay_render/output/... -> the generated map scrubber HTML (iframe screen 1)
    /map_pipeline/assets/...  -> province raster the scrubber fetches
    /dashboard/chart.umd...   -> the vendored Chart.js (reused, not copied)
    /api/games                -> JSON list of discovered games (real + demo)

A small REST-ish JSON API on top of the static files:
    /api/games                       discovered games {real:[...], demo:[...]}
    /api/series/<root>/<game_id>     compact per-game series (built on the fly,
                                     cached) — keeps the UI from fetching 5 MB
                                     snapshots just to draw a chart.

``<root>`` is ``store`` (replay/snapshots) or ``demo`` (replay_ui/demo_snapshots);
the UI never sees a filesystem path.

Usage:
    python serve.py [--port 8078] [--no-open] [--no-build-demo]
"""
from __future__ import annotations

import argparse
import json
import os
import posixpath
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
    "demo": os.path.join(HERE, "demo_snapshots"),
}

_series_cache: dict = {}


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


def _build_series(root_key: str, game_id: str) -> dict:
    key = (root_key, game_id)
    if key in _series_cache:
        return _series_cache[key]
    from replay_ui.series import build_series
    root = STORE_ROOTS.get(root_key)
    data = build_series(game_id, store_root=root)
    _series_cache[key] = data
    return data


def _safe_join(root: str, url_path: str):
    """Resolve url_path under root, refusing traversal (Codex #2)."""
    rel = posixpath.normpath(unquote(url_path)).lstrip("/")
    full = os.path.normpath(os.path.join(root, rel.replace("/", os.sep)))
    root_abs = os.path.normpath(root)
    if full != root_abs and not full.startswith(root_abs + os.sep):
        return None
    return full


class Handler(SimpleHTTPRequestHandler):
    # Serve the whole repo (read-only) so scrubber asset paths resolve.
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

    def do_GET(self):
        path = urlsplit(self.path).path
        if path == "/" or path == "":
            self.path = "/replay_ui/app.html"
            return super().do_GET()
        if path == "/api/games":
            return self._send_json(_discover_games())
        if path.startswith("/api/series/"):
            parts = path[len("/api/series/"):].split("/")
            if len(parts) >= 2 and parts[0] in STORE_ROOTS:
                root_key, game_id = parts[0], parts[1]
                try:
                    return self._send_json(_build_series(root_key, game_id))
                except Exception as e:  # never 500 silently
                    return self._send_json({"error": str(e)}, code=500)
            return self._send_json({"error": "bad series path"}, code=400)
        # static — SimpleHTTPRequestHandler already normalizes & is rooted at REPO,
        # but double-check for traversal defensively.
        if _safe_join(REPO, path) is None:
            self.send_error(403, "forbidden")
            return
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
    return preferred  # let bind fail loudly if truly nothing free


def main(argv=None):
    ap = argparse.ArgumentParser(description="Battle Plan replay UI server (offline, loopback).")
    ap.add_argument("--port", type=int, default=8078)
    ap.add_argument("--no-open", action="store_true")
    ap.add_argument("--no-build-demo", action="store_true",
                    help="skip (re)generating the synthetic demo store on launch")
    args = ap.parse_args(argv)

    if not args.no_build_demo:
        try:
            from replay_ui.demo import generate, DEMO_GAME_ID
            generate()
            print(f"[demo] synthetic store ready ({DEMO_GAME_ID})")
        except Exception as e:
            print(f"[demo] skipped: {e}")

    port = _free_port(args.port)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    url = f"http://127.0.0.1:{port}/"
    print("=" * 56)
    print("  Battle Plan - REPLAY UI  (offline, 127.0.0.1 only)")
    print("=" * 56)
    print(f"  Open:  {url}")
    print("  Keep this window open while you use the UI. Ctrl+C to stop.")
    print("=" * 56)
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
