#!/usr/bin/env python3
"""Tiny offline static server for the game MP-stats map viewer.

Serves the *repository root* (battle-plan/) so the viewer at
/map_view/index.html can fetch sibling assets with root-relative paths:

    /map_pipeline/assets/...
    /save_oracle/output/...

This avoids '..' path-escaping and keeps NO absolute personal path in the
tracked source (the root is resolved at runtime, relative to this file).

Bind is hard-locked to 127.0.0.1 (loopback only) -- nothing is exposed off
the machine. No network access, no external dependencies (stdlib only).

Run:
    python map_view/serve.py            # from repo root, or from anywhere
    python serve.py                     # from inside map_view/
then open  http://127.0.0.1:8777/map_view/  in a browser.

Optional CLI args:
    --port N        listen on port N (default 8777)
    --root PATH     serve a different directory as web root
"""
import argparse
import functools
import http.server
import os
import sys

# Repo root = parent of this file's directory (map_view/ -> battle-plan/).
DEFAULT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    """SimpleHTTPRequestHandler with correct MIME for our assets + brief logs."""

    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".json": "application/json",
        ".js": "text/javascript",
        ".png": "image/png",
        ".html": "text/html",
    }

    def log_message(self, fmt, *args):  # terse one-line logging
        sys.stderr.write("  %s - %s\n" % (self.address_string(), fmt % args))


def main():
    ap = argparse.ArgumentParser(description="Offline map-viewer server (loopback only).")
    ap.add_argument("--port", type=int, default=8777)
    ap.add_argument("--root", default=DEFAULT_ROOT,
                    help="directory to serve as web root (default: repo root)")
    args = ap.parse_args()

    root = os.path.abspath(args.root)
    if not os.path.isdir(root):
        sys.exit("error: root directory does not exist: %s" % root)

    handler = functools.partial(QuietHandler, directory=root)
    addr = ("127.0.0.1", args.port)  # loopback only -- never 0.0.0.0
    httpd = http.server.ThreadingHTTPServer(addr, handler)

    url = "http://127.0.0.1:%d/map_view/" % args.port
    print("Serving %s" % root)
    print("Map viewer:  %s" % url)
    print("(loopback only; Ctrl-C to stop)")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("\nstopped.")
        httpd.server_close()


if __name__ == "__main__":
    main()
