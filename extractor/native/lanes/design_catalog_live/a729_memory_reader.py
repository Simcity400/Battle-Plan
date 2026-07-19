#!/usr/bin/env python3
"""Compatibility entry point for the exact a729 A27 native catalog reader.

The former implementation built a ``SparseMemory`` image from the save oracle
and marked absent definition payloads complete.  That synthetic path has been
removed.  This module now exposes only pinned layout constants and delegates to
the real process-memory producer in :mod:`live_a729_acceptance`.
"""

from __future__ import annotations

from a729_layout import *  # noqa: F401,F403 - stable compatibility surface


def capture(process) -> dict:
    """Capture a full native catalog from an authenticated live process."""
    from live_a729_acceptance import capture as capture_live

    return capture_live(process)


def main(argv=None) -> int:
    from live_a729_acceptance import main as live_main

    return live_main(argv)


if __name__ == "__main__":
    raise SystemExit(main())
