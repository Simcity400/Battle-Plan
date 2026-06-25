"""continuity.py -- MP-continuity tagging for live snapshots.

Every snapshot the writer emits is stamped with the tags a future cross-segment
"stitch" needs. The writer only EMITS them; stitching is a follow-up. The tags
live in a non-schema sidecar (``snap["_capture"]``) so ``validate_snapshot``
still passes (the schema validator ignores unknown top-level keys).

Tags emitted (per Codex review of the original design):

  * ``campaign_id``        -- the grouping key. From a save game_unique_seed when
                              one is supplied (``campaign_id_source="save_seed"``)
                              or a user-supplied id (``"user"``); otherwise the
                              process-instance id (``"process_instance"``), which
                              is STABLE for the life of one hoi4.exe and does NOT
                              change if the capture tool restarts mid-game. We do
                              NOT mint a campaign_id from wallclock (that would
                              invent a new campaign for the same game on restart).
  * ``campaign_id_source`` -- one of save_seed | user | process_instance.
  * ``process_instance_id``-- sha256(exe_sha+pid+create_time)[:16]; PID-reuse safe.
  * ``capture_run_id``     -- this capture-tool invocation (wallclock-seeded);
                              groups everything one ``CaptureLoop`` wrote.
  * ``segment_id``         -- monotonic within a campaign. A NEW segment begins on
                              a process-instance change OR a confirmed time
                              rollback (a KNOWN hour_index strictly less than the
                              last KNOWN hour for this campaign).
  * ``capture_seq``        -- monotonic counter across the whole run; never resets.
  * ``hour_index``         -- the in-game hour (mirrors the snapshot key).
  * ``continuity_break``   -- null, or a reason string: "process_changed",
                              "time_rollback", "time_jump_forward",
                              "unknown_time". Diagnostic + confidence signal.
  * ``confidence``         -- "exact" | "degraded". Degraded when the time is
                              unknown or a forward jump could not be distinguished
                              from a new-campaign-same-process load.

Segment-detection rules (the carefully-reasoned part):

  * A ``None`` (unknown) hour_index NEVER starts a new segment and NEVER replaces
    the last-known hour. It is recorded as ``continuity_break="unknown_time"`` and
    ``confidence="degraded"`` so the rollback check still compares the next KNOWN
    hour against the last KNOWN hour across the unknown gap (Codex A).
  * Same KNOWN hour as last (equal, not less) is NOT a rollback. It is a re-read
    at the same tick (paused game / fast loop).
  * A KNOWN hour strictly LESS than the last known hour for this campaign is a
    rollback -> new segment (someone loaded an earlier save).
  * A process-instance change is always a new segment.
  * A large forward jump within the same process CANNOT be distinguished from a
    brand-new campaign loaded into the same process (no live game_unique_seed),
    so we DO NOT silently treat it as a new campaign. We flag
    ``continuity_break="time_jump_forward"`` + degraded confidence and start a new
    SEGMENT (conservative: a new save was loaded), but keep the campaign_id (the
    process is the only stable key we have). The stitch pass resolves it later.

The continuity STATE (last process instance, last known hour, current campaign,
current segment, capture_seq) is persisted next to the store and updated each
capture under a lock so two loops can't clobber it.
"""
from __future__ import annotations

import hashlib
import json
import os
import time
from dataclasses import dataclass, field
from typing import Any, Dict, Optional


# A forward jump bigger than this many in-game hours, within the same process,
# is "suspiciously large" -> flagged as a possible new-campaign-same-process load
# (degraded confidence). ~120 days. A normal interval capture advances << this.
SUSPICIOUS_FORWARD_JUMP_HOURS = 120 * 24


@dataclass
class ContinuityState:
    """Persisted cross-capture state (one file per store)."""

    last_process_instance: Optional[str] = None
    last_pid: Optional[int] = None
    last_known_hour: Optional[int] = None      # last NON-null hour seen
    campaign_id: Optional[str] = None
    campaign_id_source: Optional[str] = None
    capture_run_id: Optional[str] = None
    segment_id: int = 0
    capture_seq: int = 0
    updated_wallclock: Optional[float] = None

    @classmethod
    def from_dict(cls, d: Dict[str, Any]) -> "ContinuityState":
        st = cls()
        for f in st.__dataclass_fields__:  # type: ignore[attr-defined]
            if f in d:
                setattr(st, f, d[f])
        return st

    def to_dict(self) -> Dict[str, Any]:
        return {f: getattr(self, f)
                for f in self.__dataclass_fields__}  # type: ignore[attr-defined]


@dataclass
class CaptureTags:
    """The per-snapshot continuity stamp written to ``snap["_capture"]``."""

    campaign_id: Optional[str] = None
    campaign_id_source: Optional[str] = None
    process_instance_id: Optional[str] = None
    pid: Optional[int] = None
    capture_run_id: Optional[str] = None
    segment_id: int = 0
    capture_seq: int = 0
    hour_index: Optional[int] = None
    continuity_break: Optional[str] = None
    confidence: str = "exact"
    wallclock: float = field(default_factory=time.time)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "campaign_id": self.campaign_id,
            "campaign_id_source": self.campaign_id_source,
            "process_instance_id": self.process_instance_id,
            "pid": self.pid,
            "capture_run_id": self.capture_run_id,
            "segment_id": self.segment_id,
            "capture_seq": self.capture_seq,
            "hour_index": self.hour_index,
            "continuity_break": self.continuity_break,
            "confidence": self.confidence,
            "wallclock": self.wallclock,
        }


@dataclass
class ContinuityProposal:
    """A computed-but-not-yet-committed continuity result (Codex code-review #2).

    ``tags`` is the per-snapshot stamp; ``next_state`` is the state to promote
    once the snapshot is durably written. The live tracker state is NOT advanced
    until ``ContinuityTracker.commit(proposal)``.
    """

    tags: "CaptureTags"
    next_state: "ContinuityState"


def new_capture_run_id() -> str:
    """A fresh id for one capture-tool invocation (wallclock + entropy)."""
    seed = "%f|%d" % (time.time(), os.getpid())
    return hashlib.sha256(seed.encode("utf-8")).hexdigest()[:16]


class ContinuityTracker:
    """Stateful tagger. One per CaptureLoop; persists its state to disk.

    The persisted-state file is read once at construction and rewritten (durably)
    after every ``tag()``. ``tag()`` is pure-ish: it mutates the in-memory state
    and returns the stamp for this capture; the loop is responsible for asking the
    durable store to persist the state alongside the snapshot.
    """

    def __init__(self, state: ContinuityState,
                 *, capture_run_id: Optional[str] = None,
                 save_seed: Optional[str] = None,
                 user_campaign_id: Optional[str] = None):
        self.state = state
        self.capture_run_id = capture_run_id or new_capture_run_id()
        self.save_seed = save_seed
        self.user_campaign_id = user_campaign_id
        # capture_run_id is per-invocation; always refresh it on the state.
        self.state.capture_run_id = self.capture_run_id

    # -- campaign id resolution ----------------------------------------
    def _resolve_campaign_id(self, process_instance_id: Optional[str]):
        """Decide (campaign_id, source) for the CURRENT capture's process.

        Priority: explicit save seed > user id > process-instance id. The
        process-instance fallback is stable across a capture-tool restart (it does
        NOT use wallclock), so the SAME ongoing game keeps the SAME campaign_id
        even if this tool is restarted mid-game (Codex C)."""
        if self.save_seed:
            return ("seed-%s" % self.save_seed, "save_seed")
        if self.user_campaign_id:
            return (self.user_campaign_id, "user")
        # memory-only stable fallback: the process instance. Honest about what it
        # is (a process, not a proven campaign) via campaign_id_source.
        return (("proc-%s" % process_instance_id) if process_instance_id
                else "proc-unknown", "process_instance")

    # -- the core decision (pure: does NOT mutate self.state) -----------
    def compute(self, *, process_instance_id: Optional[str],
                pid: Optional[int],
                hour_index: Optional[int]) -> "ContinuityProposal":
        """Compute the continuity stamp + the NEXT state for one capture WITHOUT
        mutating the live state (Codex code-review #2). The loop calls ``commit``
        with the returned proposal ONLY after the snapshot + state are durably
        written, so a failed write never advances continuity for a snapshot that
        does not exist."""
        cur = self.state
        # work on a copy; the live state advances only at commit().
        st = ContinuityState.from_dict(cur.to_dict())
        st.capture_seq += 1

        campaign_id, source = self._resolve_campaign_id(process_instance_id)

        break_reason: Optional[str] = None
        confidence = "exact"

        process_changed = (st.last_process_instance is not None
                           and process_instance_id != st.last_process_instance)
        campaign_changed = (st.campaign_id is not None
                            and campaign_id != st.campaign_id)

        # First capture EVER for this state = no prior process AND no campaign yet.
        # Robust to a state reloaded from disk: a persisted state always has a
        # campaign_id and last_process_instance set, so this is False on resume.
        first_capture = (st.last_process_instance is None
                         and st.campaign_id is None)

        if first_capture:
            st.segment_id = max(st.segment_id, 0) + 1
            # A first capture with an UNKNOWN in-game time must still be stamped
            # unknown_time/degraded (adversarial review #2): the old code fell
            # through with confidence="exact" + no break, mislabeling a no-date
            # first read as a clean exact capture. The segment still opens (it is
            # the first), but the time is honestly flagged as unknown and
            # last_known_hour is NOT set below (guarded by `hour_index is not None`).
            if hour_index is None:
                break_reason = "unknown_time"
                confidence = "degraded"
        elif process_changed or campaign_changed:
            # a different process instance (or an explicit new campaign id) ->
            # unambiguously a new segment. The prior process/campaign's clock is
            # not comparable to this one, so DROP last_known_hour first (Codex
            # code-review #3) -- otherwise the next known hour could false-trigger
            # a rollback against the OLD process's last hour.
            st.segment_id += 1
            st.last_known_hour = None
            break_reason = "process_changed" if process_changed else "campaign_changed"
        else:
            # same process + same campaign id. Inspect the in-game time.
            if hour_index is None:
                # unknown time: never segments, never overwrites last_known_hour.
                break_reason = "unknown_time"
                confidence = "degraded"
            else:
                last = st.last_known_hour
                if last is None:
                    pass  # first known hour in this segment; nothing to compare
                elif hour_index < last:
                    # confirmed rollback across (possibly) an unknown gap.
                    st.segment_id += 1
                    break_reason = "time_rollback"
                elif hour_index == last:
                    pass  # same tick re-read (paused / fast loop): NOT a rollback
                elif (hour_index - last) > SUSPICIOUS_FORWARD_JUMP_HOURS:
                    # cannot tell "long time passed" from "new game same process"
                    # without a campaign seed -> conservative: new segment, but
                    # keep the campaign and flag degraded confidence (Codex B).
                    st.segment_id += 1
                    break_reason = "time_jump_forward"
                    confidence = "degraded"
                # else: a normal forward step -> same segment, exact.

        # fold campaign + process into the PROPOSED state.
        if st.campaign_id is None or campaign_changed:
            st.campaign_id = campaign_id
            st.campaign_id_source = source
        st.last_process_instance = process_instance_id
        st.last_pid = pid
        # only a KNOWN hour updates last_known_hour (Codex A).
        if hour_index is not None:
            st.last_known_hour = hour_index
        st.updated_wallclock = time.time()

        tags = CaptureTags(
            campaign_id=st.campaign_id,
            campaign_id_source=st.campaign_id_source,
            process_instance_id=process_instance_id,
            pid=pid,
            capture_run_id=self.capture_run_id,
            segment_id=st.segment_id,
            capture_seq=st.capture_seq,
            hour_index=hour_index,
            continuity_break=break_reason,
            confidence=confidence,
        )
        return ContinuityProposal(tags=tags, next_state=st)

    def commit(self, proposal: "ContinuityProposal") -> None:
        """Promote a computed proposal to the live state (call ONLY after the
        snapshot is durably written)."""
        self.state = proposal.next_state

    def tag(self, *, process_instance_id: Optional[str], pid: Optional[int],
            hour_index: Optional[int]) -> CaptureTags:
        """Convenience: compute AND immediately commit (used by tests and any
        caller that does not need the failed-write rollback guarantee)."""
        prop = self.compute(process_instance_id=process_instance_id, pid=pid,
                            hour_index=hour_index)
        self.commit(prop)
        return prop.tags

    def stamp(self, snap: Dict[str, Any], tags: CaptureTags) -> Dict[str, Any]:
        """Write the continuity tags into a snapshot's ``_capture`` sidecar."""
        snap["_capture"] = tags.to_dict()
        return snap


__all__ = [
    "ContinuityState", "ContinuityTracker", "ContinuityProposal", "CaptureTags",
    "new_capture_run_id", "SUSPICIOUS_FORWARD_JUMP_HOURS",
]
