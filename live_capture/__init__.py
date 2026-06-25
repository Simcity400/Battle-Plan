"""live_capture -- the live snapshot WRITER.

Turns the cracked, read-only memory readers into schema-conformant SNAPSHOTS
(replay/SCHEMA.md) written durably to the replay store, with MP-continuity tagging
built in from the start. Pure stdlib Python; the only memory access is through the
existing READ-ONLY readers under ``memory_reader/`` (never edited here).

Public surface:

    from live_capture import CaptureLoop, DateInfo
    loop = CaptureLoop(game_id="ger_live", store_root="replay/snapshots")
    res = loop.capture_once()                 # single shot
    loop.run(interval_s=30, max_captures=10)  # interval

No fabrication: a domain is captured (and listed in provenance) iff its reader plug
ran to completion; otherwise its fields stay null. See README.md.
"""
from __future__ import annotations

from .capture import CaptureLoop, CaptureResult, DateInfo
from .registry import (
    CaptureContext, DomainResult, ReaderPlug, default_registry, read_all,
)
from .continuity import ContinuityState, ContinuityTracker, CaptureTags
from .assembler import assemble_snapshot
from .durable_store import DurableCaptureStore, CaptureLock
from .process_identity import ProcessIdentity, identify, PINNED_EXE_SHA256

__all__ = [
    "CaptureLoop", "CaptureResult", "DateInfo",
    "CaptureContext", "DomainResult", "ReaderPlug", "default_registry",
    "read_all", "ContinuityState", "ContinuityTracker", "CaptureTags",
    "assemble_snapshot", "DurableCaptureStore", "CaptureLock",
    "ProcessIdentity", "identify", "PINNED_EXE_SHA256",
]
