"""In-game-hour cadence accounting for the GUI recorder.

The recorder should sample the replay stream by the game's canonical hour, not
blind wall-clock sleeps. This module is deliberately small and pure so the edge
cases are unit-testable without the game or Win32 memory access.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

from replay import schema


@dataclass
class CadenceDecision:
    """The result of observing one live-clock value."""

    should_capture: bool
    reason: str
    hour_index: Optional[int]
    date: Optional[str]
    base_hour: Optional[int] = None
    skipped_delta: int = 0
    duplicate_delta: int = 0
    committed: bool = False


@dataclass
class RecorderCadence:
    """Track observed in-game hours and decide whether to capture.

    The policy is conservative:
      * capture the first trusted hour and every configured cadence step after it;
      * skip duplicate observations of the current hour;
      * count missed scheduled cadence hours after the new observed hour is
        durably captured;
      * on a rollback, start a new local cadence branch and capture immediately.

    Unknown/untrusted dates are never captured by this scheduler. They are
    counted loudly so the GUI says the recorder is waiting for a trusted clock
    instead of silently producing a replay with no canonical hours.
    """

    target_step_hours: int = 1
    max_skip_ranges: int = 64
    observations: int = 0
    trusted_observations: int = 0
    untrusted_observations: int = 0
    captures: int = 0
    duplicate_observations: int = 0
    cadence_wait_observations: int = 0
    skipped_hours: int = 0
    skip_events: int = 0
    rollback_observations: int = 0
    startup_untrusted_observations: int = 0
    capture_clock_checks: int = 0
    capture_clock_crossings: int = 0
    capture_clock_end_untrusted: int = 0
    last_observed_hour: Optional[int] = None
    last_captured_hour: Optional[int] = None
    last_observed_date: Optional[str] = None
    last_capture_reason: Optional[str] = None
    last_capture_clock_check: Optional[Dict[str, Any]] = None
    skip_ranges: List[Dict[str, int]] = field(default_factory=list)

    def __post_init__(self) -> None:
        self.target_step_hours = max(1, int(self.target_step_hours))

    def plan(self, date_info: Any) -> CadenceDecision:
        """Observe a clock value and decide whether a capture should run.

        This intentionally does not commit capture state. The recorder calls
        ``commit`` only after the durable canonical write succeeds, so a failed
        capture for hour H is retried as H instead of being treated as a
        duplicate observation.
        """
        self.observations += 1
        date = getattr(date_info, "date", None)
        trusted = bool(getattr(date_info, "trusted", False))
        hour = schema.date_to_hour_index(date) if trusted else None

        if hour is None:
            self.untrusted_observations += 1
            if self.last_captured_hour is None:
                self.startup_untrusted_observations += 1
            self.last_capture_reason = "untrusted_time"
            return CadenceDecision(False, "untrusted_time", None, date)

        self.trusted_observations += 1
        self.last_observed_hour = hour
        self.last_observed_date = date

        if self.last_captured_hour is None:
            return CadenceDecision(True, "first_hour", hour, date, base_hour=None)

        if hour == self.last_captured_hour:
            self.duplicate_observations += 1
            return CadenceDecision(False, "duplicate_hour", hour, date,
                                   base_hour=self.last_captured_hour,
                                   duplicate_delta=1)

        if hour < self.last_captured_hour:
            self.rollback_observations += 1
            return CadenceDecision(True, "rollback_hour", hour, date,
                                   base_hour=self.last_captured_hour)

        gap = hour - self.last_captured_hour
        if gap < self.target_step_hours:
            self.cadence_wait_observations += 1
            return CadenceDecision(False, "cadence_not_due", hour, date,
                                   base_hour=self.last_captured_hour)

        skipped = max(0, (gap - 1) // self.target_step_hours)
        reason = "sequential_hour"
        if skipped:
            reason = "skipped_hours"

        return CadenceDecision(True, reason, hour, date,
                               base_hour=self.last_captured_hour,
                               skipped_delta=skipped)

    def commit(self, decision: CadenceDecision) -> CadenceDecision:
        """Commit a planned capture after its canonical snapshot is durable."""
        if decision.committed or not decision.should_capture:
            return decision
        hour = decision.hour_index
        if hour is None:
            return decision
        if self.last_captured_hour != decision.base_hour:
            raise ValueError(
                "stale cadence decision for hour %r: base %r != current %r"
                % (hour, decision.base_hour, self.last_captured_hour))

        if decision.skipped_delta:
            self.skipped_hours += decision.skipped_delta
            self.skip_events += 1
            start = decision.base_hour + self.target_step_hours
            end = decision.base_hour + (
                self.target_step_hours * decision.skipped_delta)
            self._append_skip_range(
                start, end,
                count=decision.skipped_delta,
                step_hours=self.target_step_hours)

        self.last_captured_hour = hour
        self.captures += 1
        self.last_capture_reason = decision.reason
        decision.committed = True
        return decision

    def note_capture_clock_sentinel(self, decision: CadenceDecision,
                                    end_date_info: Any) -> bool:
        """Record whether the live clock changed while a capture was running.

        The recorder reads a pre-capture clock and a pre-write clock within its
        own scope. In recorder mode this is called from CaptureLoop's
        ``pre_write_check`` hook, so a changed hour rejects before durable write.
        """
        if not decision.should_capture or decision.hour_index is None:
            return False
        self.capture_clock_checks += 1
        date = getattr(end_date_info, "date", None)
        trusted = bool(getattr(end_date_info, "trusted", False))
        end_hour = schema.date_to_hour_index(date) if trusted else None
        changed = False
        status = "stable"

        if end_hour is None:
            self.capture_clock_end_untrusted += 1
            status = "end_untrusted"
            changed = True
        elif end_hour != decision.hour_index:
            self.capture_clock_crossings += 1
            status = "hour_crossed"
            changed = True

        self.last_capture_clock_check = {
            "status": status,
            "start_hour": int(decision.hour_index),
            "end_hour": int(end_hour) if end_hour is not None else None,
            "start_date": decision.date,
            "end_date": date,
        }
        return changed

    def observe(self, date_info: Any) -> CadenceDecision:
        """Compatibility helper for pure cadence callers/tests."""
        decision = self.plan(date_info)
        if decision.should_capture:
            self.commit(decision)
        return decision

    def _append_skip_range(self, start_hour: int, end_hour: int, *,
                           count: Optional[int] = None,
                           step_hours: int = 1) -> None:
        if end_hour < start_hour:
            return
        if count is None:
            count = ((end_hour - start_hour) // max(1, int(step_hours))) + 1
        entry = {
            "start_hour": int(start_hour),
            "end_hour": int(end_hour),
            "count": int(count),
        }
        if int(step_hours) != 1:
            entry["step_hours"] = int(step_hours)
        self.skip_ranges.append(entry)
        if len(self.skip_ranges) > self.max_skip_ranges:
            self.skip_ranges = self.skip_ranges[-self.max_skip_ranges:]

    def status_label(self) -> str:
        if self.skipped_hours:
            return "lagging"
        if self.untrusted_observations and not self.trusted_observations:
            return "clock_untrusted"
        if self.startup_untrusted_observations:
            return "coverage_warning"
        if self.capture_clock_crossings or self.capture_clock_end_untrusted:
            return "coverage_warning"
        return "ok"

    def to_manifest(self) -> Dict[str, Any]:
        """JSON-safe diagnostics for the replay manifest.

        Keep this int/string/list-only: the manifest digest path rejects floats
        to stay byte-compatible with the Rust canonical JSON implementation.
        """
        coverage_warnings = []
        if self.startup_untrusted_observations:
            coverage_warnings.append({
                "code": "startup_clock_untrusted",
                "untrusted_observations": int(
                    self.startup_untrusted_observations),
            })
        if self.capture_clock_crossings:
            coverage_warnings.append({
                "code": "capture_clock_hour_crossed",
                "count": int(self.capture_clock_crossings),
            })
        if self.capture_clock_end_untrusted:
            coverage_warnings.append({
                "code": "capture_clock_end_untrusted",
                "count": int(self.capture_clock_end_untrusted),
            })

        return {
            "cadence": {
                "mode": "ingame_hour_dedup_v1",
                "target_step_hours": int(self.target_step_hours),
                "status": self.status_label(),
                "observations": int(self.observations),
                "trusted_observations": int(self.trusted_observations),
                "untrusted_observations": int(self.untrusted_observations),
                "captures": int(self.captures),
                "duplicate_observations": int(self.duplicate_observations),
                "cadence_wait_observations": int(
                    self.cadence_wait_observations),
                "skipped_hours": int(self.skipped_hours),
                "skip_events": int(self.skip_events),
                "rollback_observations": int(self.rollback_observations),
                "startup_untrusted_observations": int(
                    self.startup_untrusted_observations),
                "last_observed_hour": self.last_observed_hour,
                "last_captured_hour": self.last_captured_hour,
                "last_capture_reason": self.last_capture_reason,
                "skip_ranges": list(self.skip_ranges),
                "coverage_warnings": coverage_warnings,
                "capture_clock_sentinel": {
                    "mode": "pre_write_check_v1",
                    "checks": int(self.capture_clock_checks),
                    "hour_crossings": int(self.capture_clock_crossings),
                    "end_untrusted_observations": int(
                        self.capture_clock_end_untrusted),
                    "last": self.last_capture_clock_check,
                    "action": "reject_before_durable_write",
                },
            }
        }


__all__ = ["CadenceDecision", "RecorderCadence"]
