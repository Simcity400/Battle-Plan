"""combat_sampler.py - high-frequency sampling loop that reconstructs a battle log from
the appear / persist / disappear lifecycle of live land-combat objects.

READ-ONLY. Polls CombatReader.enumerate_land_combats() at a chosen cadence, keyed on
combat_id, and emits lifecycle events:
  * BATTLE_START  (combat_id first seen)  - province, attacker set, start age
  * BATTLE_UPDATE (participants/age changed between samples)
  * BATTLE_END    (combat_id no longer present) - last-seen province, age, duration

Both the ATTACKER and the DEFENDER exact division sets are read from live memory (solved
2026-06-24; 13/13 vs the save). Honest residual baked in (see combat_offsets.draft.json):
  * Micro-engagements that begin AND end between two samples are unobservable.
  * combat_id reuse cannot be distinguished without a generation/age discontinuity; this
    sampler splits an engagement if a reappearing id shows a LOWER age than last seen.
  * A side that fails the fail-closed validators surfaces as None (not []) for that side.

This is a reference/demo loop, not the production scheduler (which lives in the reader
productionization phase and shares the snapshot store). It prints events to stdout.
ASCII only.
"""

import time
import argparse

from combat_enum import CombatReader


class BattleTracker(object):
    """Diffs successive combat snapshots into a lifecycle event stream."""

    def __init__(self):
        # combat_id -> last observed record (+ bookkeeping)
        self.live = {}
        self.events = []

    def _emit(self, kind, cid, rec, extra=None):
        ev = {"event": kind, "combat_id": cid,
              "province": rec.get("location"), "age_days": rec.get("day"),
              "attacker": rec.get("attacker_units"),
              "defender": rec.get("defender_units")}
        if extra:
            ev.update(extra)
        self.events.append(ev)
        return ev

    def ingest(self, sample_time, combats):
        """combats: list of dicts from enumerate_land_combats(). Returns events for this tick."""
        seen = {c["combat_id"]: c for c in combats}
        new_events = []

        # appear / update
        for cid, rec in seen.items():
            prev = self.live.get(cid)
            if prev is None:
                new_events.append(self._emit("BATTLE_START", cid, rec,
                                              {"t": sample_time, "start_age": rec.get("day")}))
            else:
                # age regression => id reuse / new engagement on a recycled id
                pday = prev.get("day"); cday = rec.get("day")
                if pday is not None and cday is not None and cday < pday:
                    new_events.append(self._emit("BATTLE_END", cid, prev,
                                                  {"t": sample_time, "reason": "age_regression(id_reuse)"}))
                    new_events.append(self._emit("BATTLE_START", cid, rec,
                                                  {"t": sample_time, "start_age": cday}))
                elif (rec.get("location") != prev.get("location") or
                      set(rec.get("attacker_units") or []) != set(prev.get("attacker_units") or []) or
                      set(rec.get("defender_units") or []) != set(prev.get("defender_units") or [])):
                    new_events.append(self._emit("BATTLE_UPDATE", cid, rec, {"t": sample_time}))
            self.live[cid] = rec

        # disappear
        for cid in list(self.live.keys()):
            if cid not in seen:
                prev = self.live.pop(cid)
                new_events.append(self._emit("BATTLE_END", cid, prev,
                                             {"t": sample_time, "reason": "resolved",
                                              "final_duration": prev.get("duration")}))
        return new_events


def run(interval_s=2.0, max_samples=None, with_defender=True):
    r = CombatReader()
    ok, sha = r.verify_build()
    if not ok:
        print("BUILD GATE FAILED (sha=%s) - refusing to sample." % sha)
        r.close()
        return
    print("sampling land combats every %.1fs (build %s OK). Ctrl-C to stop." % (interval_s, sha[:8]))
    tracker = BattleTracker()
    n = 0
    try:
        while max_samples is None or n < max_samples:
            t = time.time()
            # NOTE: production should bracket this with a date/tick sentinel (read date,
            # snapshot, re-read date; discard on change) per RESEARCH section 4. Omitted
            # here for the demo; combats are stable enough at speed<=1 / paused.
            combats = r.enumerate_land_combats(with_defenders=with_defender)
            evs = tracker.ingest(t, combats)
            if evs:
                for e in evs:
                    print("  %-13s cid=%s prov=%s age=%s atk=%s def=%s %s" % (
                        e["event"], e["combat_id"], e["province"], e["age_days"],
                        e["attacker"], e.get("defender"), e.get("reason", "")))
            else:
                print("  (tick %d: %d active, no changes)" % (n, len(combats)))
            n += 1
            if max_samples is None or n < max_samples:
                time.sleep(interval_s)
    except KeyboardInterrupt:
        print("\nstopped after %d samples; %d lifecycle events recorded." % (n, len(tracker.events)))
    r.close()


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="the game land-combat lifecycle sampler (read-only)")
    ap.add_argument("--interval", type=float, default=2.0, help="seconds between samples")
    ap.add_argument("--samples", type=int, default=None, help="stop after N samples (default: run forever)")
    ap.add_argument("--no-defenders", action="store_true", help="skip reading the (exact) defender set")
    args = ap.parse_args()
    run(interval_s=args.interval, max_samples=args.samples, with_defender=not args.no_defenders)
