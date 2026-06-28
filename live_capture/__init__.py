"""live_capture -- the live snapshot WRITER.

Turns live collector or read-only memory-reader results into schema-conformant
SNAPSHOTS (replay/SCHEMA.md) written durably to the replay store, with
MP-continuity tagging built in from the start. Pure stdlib Python; the new
collector adapter consumes already-decoded frames and does not read saves.

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
from .collector_frames import (
    COLLECTOR_ABI_VERSION, COLLECTOR_BACKEND,
    make_heartbeat_frame, snapshot_from_collector_frame, validate_collector_frame,
)
from .durable_store import DurableCaptureStore, CaptureLock
from .process_identity import ProcessIdentity, identify, PINNED_EXE_SHA256

__all__ = [
    "CaptureLoop", "CaptureResult", "DateInfo",
    "CaptureContext", "DomainResult", "ReaderPlug", "default_registry",
    "read_all", "ContinuityState", "ContinuityTracker", "CaptureTags",
    "assemble_snapshot", "DurableCaptureStore", "CaptureLock",
    "ProcessIdentity", "identify", "PINNED_EXE_SHA256",
    "COLLECTOR_ABI_VERSION", "COLLECTOR_BACKEND",
    "make_heartbeat_frame", "snapshot_from_collector_frame",
    "validate_collector_frame",
]
