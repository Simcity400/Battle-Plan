#!/usr/bin/env python3
"""Unified offline launcher for the Battle Plan suite (one localhost server).

Serves BOTH layers on a single 127.0.0.1 origin and port:

    /                       -> landing page (links the two layers)
    /stats                  -> Layer 1: the live stats / graphs dashboard
    /data.json              -> Layer 1: parsed game.log payload (JSON)
    /replay                 -> Layer 2: the game-authentic replay UI
    /replay/                -> (same)
    /replay_game/...        -> Layer 2 UI assets + its /replay_game/api/*
    /api/games              -> Layer 2 game-discovery alias
    /game_assets/...        -> converted art (read-only)
    /map_pipeline/...       -> province raster + colors (read-only)
    /replay/snapshots/...   -> snapshot store (read-only)
    /replay_ui/...          -> Layer-1 stats UI assets (iframed by the replay toggle)
    /replay_render/...      -> existing map scrubber (optional base reuse)

This is a thin COMPOSITION layer: it reuses the two existing servers' own logic
verbatim rather than re-implementing them, so each layer still behaves exactly as
it does standalone. ``replay_game/serve.py`` and ``dashboard/battle_plan.py``
remain runnable on their own (the existing ``Open Game Replay.bat`` /
``Open Battle Plan Dashboard.bat`` paths are untouched).

Stdlib only, loopback only, no network, no live process memory.

Usage:
    python launcher.py [path/to/game.log] [--port 8080] [--no-open]
"""
from __future__ import annotations

import argparse
import importlib.util
import os
import sys
import threading
import webbrowser
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)


def _load_module(name: str, path: str):
    """Import a sibling tool by file path (neither dir is a package)."""
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {name} from {path}")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


# Layer 1 (stats dashboard) and Layer 2 (game-authentic replay).
stats = _load_module("battle-plan", os.path.join(HERE, "battle_plan.py"))
replay = _load_module("replay_game_serve", os.path.join(REPO, "replay_game", "serve.py"))


LANDING = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Battle Plan</title>
<style>
  body{font-family:Segoe UI,Arial,sans-serif;margin:0;min-height:100vh;
       background:#10141c;color:#dfe6f0;display:flex;flex-direction:column;
       align-items:center;justify-content:center}
  h1{font-size:26px;margin:0 0 6px}
  p.sub{opacity:.6;margin:0 0 30px;font-size:14px}
  .cards{display:flex;gap:22px;flex-wrap:wrap;justify-content:center;max-width:760px}
  a.card{display:block;width:320px;background:#161b26;border:1px solid #2a3344;
         border-radius:10px;padding:22px;text-decoration:none;color:#dfe6f0;
         transition:border-color .15s,transform .15s}
  a.card:hover{border-color:#4363d8;transform:translateY(-2px)}
  a.card h2{margin:0 0 8px;font-size:19px}
  a.card span{opacity:.7;font-size:13px;line-height:1.5}
  footer{margin-top:34px;opacity:.4;font-size:12px}
</style></head>
<body>
  <h1>Battle Plan</h1>
  <p class="sub">offline, 127.0.0.1 only</p>
  <div class="cards">
    <a class="card" href="/stats">
      <h2>&#128202; Stats Dashboard</h2>
      <span>Layer 1 &mdash; live graphs of IC output, production mix and the
      latest-tick snapshot, parsed from <code>game.log</code>. Auto-refreshes.</span>
    </a>
    <a class="card" href="/replay">
      <h2>&#127758; Game Replay</h2>
      <span>Layer 2 &mdash; the game-authentic the game interface; scrub/play across a
      campaign's snapshot series and watch the world evolve.</span>
    </a>
  </div>
  <footer>One local server &middot; both layers share this origin.</footer>
</body></html>
"""


def make_handler(log_path):
    # Layer 2's handler already roots itself at REPO and serves every absolute
    # asset/API path the replay UI uses (/replay_game/*, /api/games, /game_assets/*,
    # ...). We subclass it and intercept ONLY the new landing + Layer-1 routes,
    # then defer everything else to its proven do_GET.
    class Unified(replay.Handler):
        def _send_html(self, page: str, code: int = 200):
            body = page.encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            route = urlsplit(self.path).path

            # --- landing -----------------------------------------------------
            if route in ("", "/"):
                self._send_html(LANDING)
                return

            # --- Layer 1: stats dashboard -----------------------------------
            if route == "/stats":
                page = (stats.HTML
                        .replace("__CHARTJS__", stats.chartjs_tag())
                        .replace("__DATA__", "null"))
                self._send_html(page)
                return
            if route == "/data.json":
                try:
                    body = stats.dump_json(
                        stats.cached_payload(log_path)).encode("utf-8")
                except Exception:
                    body = b'{"series":{},"ticks":[],"types":[],"scalars":{}}'
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)
                return

            # --- Layer 2: replay UI entry -----------------------------------
            if route in ("/replay", "/replay/"):
                # Serve the replay shell; all its assets are absolute under
                # /replay_game/* and resolve via the parent handler. Rewrite the
                # path and defer to the parent (keeps its traversal check).
                self.path = "/replay_game/static/index.html"
                return super().do_GET()

            # --- everything else: Layer 2's own logic -----------------------
            # (/replay_game/*, /api/games, /game_assets/*, /map_pipeline/*,
            #  /replay/snapshots/*, /replay_ui/*, /replay_render/*, ...)
            return super().do_GET()

    return Unified


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


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Unified offline launcher for the Battle Plan suite "
                    "(stats dashboard + game replay on one localhost server).")
    ap.add_argument("log", nargs="?",
                    help="path to game.log for the stats layer "
                         "(default: the game logs folder)")
    ap.add_argument("--port", type=int, default=8080)
    ap.add_argument("--no-open", action="store_true")
    args = ap.parse_args(argv)

    log_path = Path(args.log) if args.log else stats.default_log_path()

    if not replay._assets_present():
        print("[warn] game_assets/assets/ not found on disk - replay sprites "
              "will be placeholders.")
        print("       Run:  python game_assets/extract.py   (from the repo root).")

    port = _free_port(args.port)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_handler(log_path))
    base = f"http://127.0.0.1:{port}/"
    print("=" * 60)
    print("  Battle Plan - UNIFIED LAUNCHER (offline, 127.0.0.1 only)")
    print("=" * 60)
    print(f"  Landing : {base}")
    print(f"  Stats   : {base}stats")
    print(f"  Replay  : {base}replay")
    print(f"  Reading : {log_path}"
          + ("" if log_path.exists() else "   (not created yet - fills in live)"))
    print("  Keep this window open while you use the UI. Ctrl+C to stop.")
    print("=" * 60)
    if not args.no_open:
        threading.Timer(0.6, lambda: webbrowser.open(base)).start()
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\n(server stopped)")
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
