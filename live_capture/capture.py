"""capture.py -- the live capture loop.

Orchestrates one or many captures end to end:

    build-gate -> attach -> resolve live layers (tag table / bridge / province
    control) -> run reader plugs -> assemble snapshot -> continuity-tag ->
    derive overlay -> DURABLE write -> persist continuity state.

Two modes: single-shot (``capture_once``) and interval (``run``). Every snapshot
is written IMMEDIATELY after it is assembled+derived, so a crash loses at most the
in-flight read (Codex E durability).

DATE / HOUR sourcing (Codex G): the in-game date is the snapshot's filename, sort
key and diff key, so it must be TRUSTED. The live clock read costs ~150s and the
weather package is in flux, so by default we accept an externally-supplied date
ONLY if the caller marks it trusted (e.g. it came from the in-game mod log at the
same tick). A date from a stale save meta is NOT trusted: such a capture is marked
non-canonical (it still records the snapshot, but to ``_noncanonical/`` and with
degraded continuity confidence). If no trusted date is available the capture is
unknown-time (hour_index null) -> non-canonical by default.

Build gate (Codex / locate.py contract): before ANY live read we compare the live
exe sha256 to the pinned build; refuse on mismatch unless ``force=True``.

This module imports the readers only through the registry, so it stays importable
on non-Windows / without a game (the dry-run test relies on that).
"""
from __future__ import annotations

import os
import sys
import time
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.normpath(os.path.join(_HERE, os.pardir))
_MR = os.path.join(_REPO, "memory_reader")
for _p in (_REPO, _MR):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from live_capture import registry as _registry          # noqa: E402
from live_capture import assembler as _assembler        # noqa: E402
from live_capture import continuity as _continuity      # noqa: E402
from live_capture import process_identity as _pi         # noqa: E402
from live_capture.durable_store import (                 # noqa: E402
    DurableCaptureStore, CaptureLock,
)

try:
    from derive import derive_layers as _derive_layers   # noqa: E402
except Exception:  # pragma: no cover - derive should always import
    _derive_layers = None


@dataclass
class DateInfo:
    """A date for a capture and whether it is trustworthy as the canonical key."""

    date: Optional[str]
    trusted: bool = False
    source: str = "none"       # "clock" | "mod_log" | "user" | "save_meta" | "none"


@dataclass
class CaptureResult:
    written_path: Optional[str]
    snapshot: Dict[str, Any]
    domains_captured: List[str]
    tags: Dict[str, Any]
    canonical: bool
    error: Optional[str] = None


# --------------------------------------------------------------------------
# Live-layer resolution (each is independent + fail-soft: a layer that won't
# resolve becomes None, and the plugs that need it then honestly omit their
# domains -- no fabrication).
# --------------------------------------------------------------------------
def _resolve_live_layers(proc) -> Dict[str, Any]:
    out: Dict[str, Any] = {"tag_table": None, "bridge": None,
                           "province_control": None}
    try:
        from live_layers import tag_table as _tt
        out["tag_table"] = _tt.TagTable.locate(proc)
    except Exception:
        out["tag_table"] = None
    try:
        from live_layers import country_index_bridge as _cib
        out["bridge"] = _cib.CountryIndexBridge.resolve(proc, tbl=out["tag_table"])
    except Exception:
        out["bridge"] = None
    try:
        from live_layers import province_control as _pc
        out["province_control"] = _pc.ProvinceControl.resolve(
            proc, tbl=out["tag_table"])
    except Exception:
        out["province_control"] = None
    return out


def _load_economy_offsets() -> Dict[str, Any]:
    path = os.path.join(_MR, "economy_offsets.draft.json")
    try:
        import json
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return {}


# --------------------------------------------------------------------------
# CaptureLoop
# --------------------------------------------------------------------------
class CaptureLoop:
    """A configured capture loop bound to a store + game/campaign.

    ``open_proc`` is a zero-arg callable returning an open winmem.Process-like
    handle (defaults to ``winmem.open_process('hoi4.exe')``). It is parameterized
    so the dry-run test can inject a fake process + fake registry with no game.
    """

    def __init__(self, *, game_id: str, store_root: Optional[str] = None,
                 plugs=None,
                 open_proc: Optional[Callable[[], Any]] = None,
                 save_seed: Optional[str] = None,
                 user_campaign_id: Optional[str] = None,
                 force_build: bool = False):
        self.game_id = game_id
        self.store = DurableCaptureStore(store_root)
        self.plugs = plugs
        self.open_proc = open_proc or _default_open_proc
        self.force_build = force_build
        self._eco_offsets = _load_economy_offsets()
        st = _continuity.ContinuityState.from_dict(
            self.store.load_continuity_state(game_id))
        self.tracker = _continuity.ContinuityTracker(
            st, save_seed=save_seed, user_campaign_id=user_campaign_id)

    # -- one capture ----------------------------------------------------
    def capture_once(self, *, date_info: Optional[DateInfo] = None,
                     prev_snapshot: Optional[Dict[str, Any]] = None,
                     proc: Optional[Any] = None) -> CaptureResult:
        """Capture a single snapshot end to end and write it durably.

        ``proc`` lets a caller (or the interval loop) reuse one open handle for
        many captures; if None we open + close one for this capture.
        """
        own_proc = proc is None
        if own_proc:
            proc = self.open_proc()
        try:
            identity = _pi.identify(proc)
            if not identity.build_ok() and not self.force_build:
                raise RuntimeError(
                    "build gate: live exe sha256 %s != pinned %s. Refusing to "
                    "read an unknown build (pass force_build=True to override; "
                    "results would be UNVERIFIED)."
                    % (identity.exe_sha256, _pi.PINNED_EXE_SHA256))

            layers = _resolve_live_layers(proc)
            ctx = _registry.CaptureContext(
                proc=proc, identity=identity,
                tag_table=layers["tag_table"], bridge=layers["bridge"],
                province_control=layers["province_control"],
                extras={"economy_offsets": self._eco_offsets},
            )
            results = _registry.read_all(ctx, self.plugs)
            return self._finish_capture(results, identity, date_info,
                                        prev_snapshot)
        finally:
            if own_proc:
                _safe_close(proc)

    def capture_from_results(self, results: Dict[str, Any], *,
                             identity: Any,
                             date_info: Optional[DateInfo] = None,
                             prev_snapshot: Optional[Dict[str, Any]] = None
                             ) -> CaptureResult:
        """Assemble+tag+derive+write from PRE-RUN plug results (test/offline seam).

        Lets the dry-run test feed canned DomainResults without a live process.
        """
        return self._finish_capture(results, identity, date_info, prev_snapshot)

    def _finish_capture(self, results, identity, date_info, prev_snapshot):
        date_info = date_info or DateInfo(None, trusted=False, source="none")
        canonical_date = date_info.date if date_info.trusted else None

        snap = _assembler.assemble_snapshot(
            results, game_id=self.game_id, date=canonical_date,
            exe_sha256=getattr(identity, "exe_sha256", None),
            identity=identity,
            extra_notes=([] if date_info.trusted else
                         (["date %r from %s is UNTRUSTED -> capture is "
                           "non-canonical (hour_index withheld)" % (
                               date_info.date, _safe_source(date_info.source))]
                          if date_info.date else [])),
        )

        # continuity BEFORE derive (Codex G). COMPUTE the tags without advancing
        # the live state (Codex code-review #2): we only commit after the durable
        # write succeeds, so a failed write never advances continuity for a
        # snapshot that does not exist on disk.
        proposal = self.tracker.compute(
            process_instance_id=getattr(identity, "instance_id", None),
            pid=getattr(identity, "pid", None),
            hour_index=snap.get("hour_index"),
        )
        tags = proposal.tags
        self.tracker.stamp(snap, tags)

        # derive overlay -- only pass prev when it is the SAME segment and strictly
        # earlier in-game time (never derive a delta across a rollback / pid change
        # / unknown time / same hour). Otherwise overlay single-snapshot layers.
        safe_prev = self._safe_prev(snap, tags, prev_snapshot)
        if _derive_layers is not None:
            snap = _derive_layers(snap, prev_snapshot=safe_prev)
            # re-stamp: derive_layers deep-copies, but it preserves unknown keys,
            # so _capture survives; assert it for safety.
            if "_capture" not in snap:
                snap["_capture"] = tags.to_dict()
            # derive's deployed_manpower pass re-nulls division_count for any tag
            # not seen in divisions[] -- which clobbers the assembler's measured
            # ZERO for a known country that simply has no division (Codex reconcile
            # #4). Re-apply the measured zero AFTER derive, but ONLY when the
            # divisions domain was actually captured this snapshot.
            _restore_measured_zero_division_count(snap)

        canonical = bool(date_info.trusted and snap.get("hour_index") is not None)
        # write the snapshot FIRST; only on success commit + persist continuity.
        path = self.store.write_snapshot(snap, allow_unknown_hour=False)
        self.tracker.commit(proposal)
        self.store.save_continuity_state(self.game_id,
                                         self.tracker.state.to_dict())

        domains = (snap.get("provenance", {}).get("memory", {}) or {}).get(
            "domains", [])
        return CaptureResult(written_path=path, snapshot=snap,
                             domains_captured=list(domains),
                             tags=tags.to_dict(), canonical=canonical)

    def _safe_prev(self, snap, tags, prev_snapshot):
        if prev_snapshot is None:
            return None
        prev_cap = prev_snapshot.get("_capture") or {}
        # different campaign or segment -> not comparable.
        if prev_cap.get("campaign_id") != tags.campaign_id:
            return None
        if prev_cap.get("segment_id") != tags.segment_id:
            return None
        # a continuity break on THIS capture means the prev is across the break.
        if tags.continuity_break in ("time_rollback", "time_jump_forward",
                                     "process_changed", "campaign_changed"):
            return None
        ph = prev_snapshot.get("hour_index")
        ch = snap.get("hour_index")
        if ph is None or ch is None or not (ch > ph):
            return None
        return prev_snapshot

    # -- interval loop --------------------------------------------------
    def run(self, *, interval_s: float, max_captures: Optional[int] = None,
            date_provider: Optional[Callable[[Any], DateInfo]] = None,
            steal_lock: bool = False,
            on_capture: Optional[Callable[[CaptureResult], None]] = None
            ) -> List[CaptureResult]:
        """Capture every ``interval_s`` seconds until ``max_captures`` (or forever).

        Holds the single-writer lock for the whole run (Codex F). Reuses one open
        process handle across captures. ``date_provider(proc)->DateInfo`` lets the
        caller supply a trusted live/log date per tick (default: unknown time)."""
        lock = CaptureLock(self.store.lock_path(self.game_id))
        lock.acquire(steal=steal_lock)
        results: List[CaptureResult] = []
        prev: Optional[Dict[str, Any]] = None
        n = 0
        try:
            while max_captures is None or n < max_captures:
                proc = None
                try:
                    proc = self.open_proc()
                    di = date_provider(proc) if date_provider else DateInfo(
                        None, trusted=False, source="none")
                    res = self.capture_once(date_info=di, prev_snapshot=prev,
                                            proc=proc)
                    results.append(res)
                    prev = res.snapshot
                    if on_capture:
                        on_capture(res)
                except Exception as exc:  # one bad capture must not kill the loop
                    results.append(CaptureResult(
                        written_path=None, snapshot={}, domains_captured=[],
                        tags={}, canonical=False, error=str(exc)))
                finally:
                    _safe_close(proc)
                n += 1
                if max_captures is None or n < max_captures:
                    time.sleep(interval_s)
        finally:
            lock.release()
        return results


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------
def _restore_measured_zero_division_count(snap):
    """If the divisions domain was captured, a known country with no division has a
    MEASURED division_count of 0 (not null). derive's overlay re-nulls it for tags
    absent from divisions[]; restore the zero here (Codex reconcile #4). Counts
    derived directly from divisions[] so it agrees with the (post-derive) data."""
    prov = (snap.get("provenance", {}).get("memory") or {})
    if "divisions" not in (prov.get("domains") or []):
        return
    counts = {}
    for d in snap.get("divisions", []):
        owner = d.get("owner")
        if isinstance(owner, str):
            counts[owner] = counts.get(owner, 0) + 1
    for tag, c in snap.get("countries", {}).items():
        mil = c.setdefault("military", {})
        mil["division_count"] = counts.get(tag, 0)


_KNOWN_DATE_SOURCES = frozenset(
    {"clock", "mod_log", "user", "save_meta", "none"})


def _safe_source(source: Any) -> str:
    """Constrain a DateInfo.source to a known-safe label before it enters a
    persisted note (Codex follow-up): a caller could pass an arbitrary string
    (even a file path) as the source, which must never leak into tracked output.
    An unrecognized source is reported as 'other' (its raw value is dropped)."""
    return source if source in _KNOWN_DATE_SOURCES else "other"


def _default_open_proc():
    import winmem
    p = winmem.open_process("hoi4.exe")
    if p is None:
        raise RuntimeError("hoi4.exe not running")
    return p


def _safe_close(proc) -> None:
    if proc is None:
        return
    close = getattr(proc, "close", None)
    if callable(close):
        try:
            close()
        except Exception:
            pass


__all__ = ["CaptureLoop", "CaptureResult", "DateInfo"]
