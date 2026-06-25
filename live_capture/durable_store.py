"""durable_store.py -- atomic, crash-safe writes for live capture.

The replay ``SnapshotStore`` writes snapshots and its index with a plain
``json.dump`` (a crash mid-write truncates the file, and the snapshot + index are
two separate writes). For a live capture loop that writes continuously, a torn
write would corrupt the store. We cannot edit ``replay/store.py`` (read-only
boundary), so this module wraps it and replaces the write path with a durable one
(Codex E): write to a temp file in the SAME directory, ``fsync``, then
``os.replace`` (atomic on Windows + POSIX).

It also owns:

  * the continuity-state file (read/write, atomic) next to the store, and
  * a single-writer LOCK (``O_CREAT | O_EXCL``) covering the whole capture run so
    two loops cannot clobber the continuity state or interleave writes (Codex F).

Unknown-hour snapshots all collide on ``unknown.snapshot.json`` in the canonical
store, so by default we do NOT write them to the canonical store: they go to a
sibling ``_noncanonical/`` directory with a sequence-numbered filename and are
left out of the index. Pass ``allow_unknown_hour=True`` to force them into the
canonical store anyway.
"""
from __future__ import annotations

import binascii
import itertools
import json
import os
import re
import sys
import time
from typing import Any, Dict, Optional

# Process-wide monotonic tiebreaker for non-canonical filenames. time.time_ns()
# has ~16 ms granularity on Windows, so back-to-back captures share a timestamp;
# this counter guarantees uniqueness within a process even at that resolution.
# Combined with os.urandom (below) it is also collision-proof ACROSS a
# crash/restart (the counter alone resets on restart -- adversarial review #3).
_NONCANON_SEQ = itertools.count()

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
if _REPO not in sys.path:
    sys.path.insert(0, _REPO)

from replay import schema  # noqa: E402
from replay.store import SnapshotStore  # noqa: E402


# --------------------------------------------------------------------------
# Atomic JSON write (temp in same dir -> fsync -> os.replace).
# --------------------------------------------------------------------------
def atomic_write_json(path: str, obj: Any, *, compact: bool = True) -> None:
    """Durably write ``obj`` as JSON to ``path`` (atomic replace)."""
    d = os.path.dirname(os.path.abspath(path))
    os.makedirs(d, exist_ok=True)
    tmp = "%s.tmp.%d.%d" % (path, os.getpid(), time.time_ns())
    sep = (",", ":") if compact else (", ", ": ")
    try:
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(obj, fh, separators=sep)
            fh.flush()
            os.fsync(fh.fileno())
        os.replace(tmp, path)
        _fsync_dir(d)   # persist the rename itself (Codex code-review #7)
    finally:
        if os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def _fsync_dir(d: str) -> None:
    """fsync a directory so a rename into it survives a crash. Best-effort:
    O_DIRECTORY/dir-fsync is a no-op or unsupported on some Windows filesystems,
    so failures are swallowed (the os.replace itself is already atomic)."""
    try:
        fd = os.open(d, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


def read_json(path: str) -> Optional[Any]:
    if not os.path.isfile(path):
        return None
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return None


# --------------------------------------------------------------------------
# Single-writer lock.
# --------------------------------------------------------------------------
class CaptureLock:
    """Exclusive ``O_CREAT | O_EXCL`` lock file for a capture run.

    Holds the lock for the life of the loop. A stale lock (a dead PID) can be
    force-broken with ``steal=True``; otherwise an existing lock raises so two
    loops never run against the same store at once (Codex F)."""

    def __init__(self, lock_path: str):
        self.lock_path = lock_path
        self._fd: Optional[int] = None

    def acquire(self, *, steal: bool = False) -> "CaptureLock":
        os.makedirs(os.path.dirname(os.path.abspath(self.lock_path)),
                    exist_ok=True)
        try:
            self._open_excl()
            return self
        except FileExistsError:
            if not steal:
                holder = read_json(self.lock_path) or {}
                raise RuntimeError(
                    "capture lock held by pid=%s since %s (%s). Another capture "
                    "loop is running, or it crashed -- pass steal=True to break a "
                    "stale lock." % (holder.get("pid"), holder.get("acquired"),
                                     self.lock_path))
            # steal: remove and retry once.
            try:
                os.remove(self.lock_path)
            except OSError:
                pass
            self._open_excl()
            return self

    def _open_excl(self) -> None:
        fd = os.open(self.lock_path,
                     os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        payload = json.dumps({"pid": os.getpid(), "acquired": time.time()})
        os.write(fd, payload.encode("utf-8"))
        self._fd = fd

    def release(self) -> None:
        if self._fd is not None:
            try:
                os.close(self._fd)
            except OSError:
                pass
            self._fd = None
        if os.path.exists(self.lock_path):
            try:
                os.remove(self.lock_path)
            except OSError:
                pass

    def __enter__(self) -> "CaptureLock":
        return self

    def __exit__(self, *exc) -> None:
        self.release()


# --------------------------------------------------------------------------
# The durable capture store.
# --------------------------------------------------------------------------
class DurableCaptureStore:
    """Wraps a replay SnapshotStore with atomic writes + continuity state.

    Layout (under ``root``):
        <root>/<game_id>/<hour>.snapshot.json     canonical snapshots (atomic)
        <root>/<game_id>/index.json               index (atomic, rebuilt safely)
        <root>/<game_id>/_noncanonical/...        unknown-hour captures
        <root>/_continuity/<game_id>.state.json   continuity state (atomic)
        <root>/_continuity/<game_id>.lock         single-writer lock
    """

    def __init__(self, root: Optional[str] = None):
        self.store = SnapshotStore(root)
        self.root = self.store.root

    # -- continuity state ----------------------------------------------
    def _continuity_dir(self) -> str:
        return os.path.join(self.root, "_continuity")

    def continuity_state_path(self, game_id: str) -> str:
        return os.path.join(self._continuity_dir(),
                            "%s.state.json" % _safe_game_id(game_id))

    def lock_path(self, game_id: str) -> str:
        return os.path.join(self._continuity_dir(),
                            "%s.lock" % _safe_game_id(game_id))

    def load_continuity_state(self, game_id: str) -> Dict[str, Any]:
        return read_json(self.continuity_state_path(game_id)) or {}

    def save_continuity_state(self, game_id: str, state: Dict[str, Any]) -> None:
        atomic_write_json(self.continuity_state_path(game_id), state,
                          compact=False)

    # -- snapshot write (atomic) ---------------------------------------
    def write_snapshot(self, snap: Dict[str, Any], *,
                       allow_unknown_hour: bool = False) -> str:
        """Validate + durably persist one snapshot and update the index.

        Mirrors SnapshotStore.write_snapshot's layout/index entry but writes both
        the snapshot and the index atomically. Unknown-hour snapshots go to
        ``_noncanonical/`` unless ``allow_unknown_hour`` (Codex E)."""
        problems = schema.validate_snapshot(snap)
        if problems:
            raise ValueError("refusing to write invalid snapshot:\n  "
                             + "\n  ".join(problems[:20]))
        game_id = _safe_game_id(snap["game_id"])
        hour = snap.get("hour_index")
        if hour is None and not allow_unknown_hour:
            return self._write_noncanonical(snap)
        gdir = self.store.game_dir(game_id)
        os.makedirs(gdir, exist_ok=True)
        # Filename carries the SEGMENT (Codex code-review #1): a rollback that
        # revisits an already-captured hour in a NEW segment must not overwrite
        # the prior segment's snapshot/index entry. Same (segment, hour) is a
        # legitimate same-tick re-read and overwrites in place.
        seg = (snap.get("_capture") or {}).get("segment_id")
        fname = "%s.snapshot.json" % _canon_key(seg, hour)
        path = os.path.join(gdir, fname)
        atomic_write_json(path, snap, compact=True)
        self._update_index(game_id, snap, fname)
        return path

    def _write_noncanonical(self, snap: Dict[str, Any]) -> str:
        game_id = _safe_game_id(snap["game_id"])
        ncdir = os.path.join(self.store.game_dir(game_id), "_noncanonical")
        os.makedirs(ncdir, exist_ok=True)
        cap = snap.get("_capture") or {}
        # include segment + seq so a non-canonical capture is uniquely named.
        seg = cap.get("segment_id")
        seq = cap.get("capture_seq")
        # ALWAYS append a monotonic time_ns tiebreaker (adversarial review #1): a
        # crash between the snapshot write and the continuity-state save leaves the
        # persisted capture_seq stale, so a restart can recompute the SAME (seg,seq)
        # and -- without a unique suffix -- silently OVERWRITE a kept non-canonical
        # snapshot. Non-canonical captures are "kept, never deleted" (DESIGN 11.4),
        # so the filename must be collision-proof EVEN across a crash/restart.
        # time_ns (~16 ms on Windows) + a process-local counter both reset/collide
        # across restarts (adversarial review #3); os.urandom is the real guarantee,
        # and we additionally CREATE the file exclusively so an (astronomically
        # unlikely) random collision raises instead of clobbering.
        # TRUE exclusive create (adversarial review #3, refined): O_CREAT|O_EXCL
        # atomically RESERVES the path -- if it somehow already exists we pick a
        # fresh random name and retry, so a kept non-canonical snapshot can NEVER be
        # clobbered (os.path.exists alone is a TOCTOU race; os.replace can stomp).
        path = None
        for _ in range(16):
            rnd = binascii.hexlify(os.urandom(8)).decode("ascii")
            stamp = "s%s.seq%s.%d.p%d.%d.%s" % (
                seg if seg is not None else 0,
                seq if seq is not None else "NA",
                time.time_ns(), os.getpid(), next(_NONCANON_SEQ), rnd)
            fname = "unknown.%s.snapshot.json" % stamp
            cand = os.path.join(ncdir, fname)
            try:
                fd = os.open(cand, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            except FileExistsError:
                continue  # astronomically unlikely; try a new random name
            os.close(fd)   # reservation made; atomic_write_json replaces it durably
            # CAVEAT: a crash in the tiny window between this reservation and the
            # atomic_write_json below leaves a ZERO-BYTE orphan at `cand`. That is
            # benign -- it never clobbers a kept snapshot, and read_json() returns
            # None on the empty/invalid JSON -- so a reader skips it.
            path = cand
            break
        if path is None:
            raise RuntimeError("could not reserve a unique non-canonical filename "
                               "after 16 attempts (this should be impossible)")
        atomic_write_json(path, snap, compact=True)
        return path

    def _update_index(self, game_id: str, snap: Dict[str, Any],
                      fname: str) -> None:
        idx_path = self.store.index_path(game_id)
        idx = read_json(idx_path) or {"game_id": game_id,
                                      "schema_version": schema.SCHEMA_VERSION,
                                      "snapshots": []}
        cap = snap.get("_capture") or {}
        entry = {
            "hour_index": snap.get("hour_index"),
            "date": snap.get("date"),
            "file": fname,
            "save_version": snap.get("save_version"),
            # continuity tags surfaced in the index for the stitch/UI to group on.
            "campaign_id": cap.get("campaign_id"),
            "segment_id": cap.get("segment_id"),
            "capture_seq": cap.get("capture_seq"),
            "counts": {
                "countries": len(snap.get("countries", {})),
                "states": len(snap.get("states", {})),
                "provinces": len(snap.get("provinces", {})),
                "divisions": len(snap.get("divisions", [])),
            },
        }
        # Index uniqueness is FILE-based (Codex code-review #1): a (segment, hour)
        # snapshot has a unique filename, so two different segments at the same
        # hour are distinct entries. Re-writing the SAME file replaces its entry.
        idx["snapshots"] = [e for e in idx["snapshots"]
                            if e.get("file") != fname]
        idx["snapshots"].append(entry)
        idx["snapshots"].sort(key=lambda e: (e.get("hour_index") is None,
                                             e.get("hour_index") or 0,
                                             e.get("segment_id") or 0))
        atomic_write_json(idx_path, idx, compact=False)


def _hour_key(hour_index: Optional[int]) -> str:
    if hour_index is None:
        return "unknown"
    return "%012d" % hour_index


def _canon_key(segment_id: Optional[int], hour_index: Optional[int]) -> str:
    """Canonical snapshot filename stem: ``s<segment>.<zeropad-hour>``. The
    segment prefix keeps a rollback's revisited hour from overwriting the earlier
    segment (Codex code-review #1). Sorts so segment then hour order is stable."""
    seg = segment_id if isinstance(segment_id, int) and segment_id >= 0 else 0
    return "s%04d.%s" % (seg, _hour_key(hour_index))


_GAME_ID_OK = re.compile(r"^[A-Za-z0-9._-]{1,128}$")


def _safe_game_id(game_id: str) -> str:
    """Reject a game_id that could escape the store root (``..``, separators,
    absolute paths) -- Codex code-review #8. We fail loudly rather than sanitize
    silently, so a typo can't quietly scatter files."""
    if not isinstance(game_id, str) or not _GAME_ID_OK.match(game_id) \
            or game_id in (".", ".."):
        raise ValueError("unsafe game_id %r: must match %s and not be a path"
                         % (game_id, _GAME_ID_OK.pattern))
    return game_id


__all__ = [
    "DurableCaptureStore", "CaptureLock", "atomic_write_json", "read_json",
]
