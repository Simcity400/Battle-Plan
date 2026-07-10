"""Three-phase report: per-domain acquisition vs materialise split from a run manifest.

Companion to lane/three-phase-timing (see extractor/documents/lane_reports/
three-phase-timing.md and CODEX_DISPATCH_QUEUE.md Job 1/2). Run it against the
native_session_manifest.json of a PERF-GATED capture made with a collector that
includes the phase-1 probe. On older manifests the guarded units are simply
absent and the script says so.

Estimator (per domain d):
  guarded_ns(d) ~= guarded_spl_ns(d) * (reads(d) / guarded_spl_n(d)) + scan_guarded_ns(d)
  materialise(d) ~= wall(d) - guarded(d)     [wall: lw_d{id}_crit_us is CRITICAL-PATH
                                              attribution only; full per-domain walls
                                              live in the debug sidecar]
Phase 3 (encode/emit) is frame-level: rc_*_phase_encode_append_us.

The sampled estimate's resolution is reported (spl_n per domain); domains with
spl_n < ~30 have a noisy estimate — say so, don't over-read them.

SHARDED domains (nested worker threads: the RcDomainScope tag does not
propagate to spawned shards, so child guarded time lands global-only — a
pre-existing attribution limit that equally affects rc_d{id}_reads) get NO
materialise number: lane_wall - guarded would overstate materialise there.
Frame-level totals are unaffected.

Usage: python three_phase_report.py <native_session_manifest.json>
"""

import json
import re
import sys
from collections import defaultdict


UNIT_CONTAINERS = ("read_ab_probe", "data_units_total")


def load_units(path):
    """Merge every probe-unit name->number mapping in the manifest.

    The host lands the domain-250 diagnostic units under
    compact_stream/read_ab_probe (name->sum); data_units_total covered for
    manifests that aggregate there instead.
    """
    raw = open(path, encoding="utf-8-sig", errors="replace").read()
    i = raw.find("{")
    doc = json.loads(raw[i:])
    units = defaultdict(int)

    def walk(o):
        if isinstance(o, dict):
            for k, v in o.items():
                if k in UNIT_CONTAINERS and isinstance(v, dict):
                    for name, val in v.items():
                        if isinstance(val, (int, float)):
                            units[name] += int(val)
                else:
                    walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)

    walk(doc)
    return dict(units)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    units = load_units(sys.argv[1])
    if not units:
        print("no data_units_total found — is this a native_session_manifest.json?")
        return 1

    frames = sum(units.get(f"rc_{p}_frames", 0) for p in ("seh", "rpm"))
    if frames == 0:
        print("no rc_*_frames counter — per-frame numbers below are TOTALS, not rates")
        frames = 1
    else:
        print(f"frames with rc probe: {frames}")

    def fsum(suffix):
        return sum(units.get(f"rc_{p}_{suffix}", 0) for p in ("seh", "rpm"))
    print(
        "frame-level phases (ms/frame): prewarm %.2f  worker_read %.2f  encode_append %.2f"
        % (
            fsum("phase_prewarm_us") / frames / 1e3,
            fsum("phase_worker_read_us") / frames / 1e3,
            fsum("phase_encode_append_us") / frames / 1e3,
        )
    )
    g_spl_ns, g_spl_n = fsum("guarded_spl_ns"), fsum("guarded_spl_n")
    g_scan_ns, g_reads = fsum("scan_guarded_ns"), fsum("reads")
    if g_spl_n == 0:
        print(
            "\nNO guarded-time units in this manifest — capture predates the "
            "three-phase probe or the perf gate was off. Frame-level phases above "
            "are still valid."
        )
        return 0
    frame_guard_ms = (g_spl_ns * (g_reads / g_spl_n) + g_scan_ns) / frames / 1e6
    print(
        "frame-level guarded estimate: %.2f ms/frame  (samples/frame %.0f, "
        "reads/frame %.0f, sample cost ~%.2f ms/frame at ~100ns/QPC-pair)"
        % (frame_guard_ms, g_spl_n / frames, g_reads / frames, g_spl_n / frames * 100 / 1e6)
    )

    # Nested-shard domains: the domain tag does not propagate to spawned shard
    # threads, so their guarded time is under-attributed per-domain (1R review
    # finding 1). No materialise number is printed for them.
    SHARDED = {200, 20, 23, 26, 44}

    ids = sorted(
        {int(m.group(1)) for k in units if (m := re.match(r"rc_d(\d+)_", k))}
    )
    print(
        "\n%6s %12s %10s %12s %12s %12s %8s"
        % ("domain", "reads/frame", "spl_n", "guarded ms", "crit ms", "materialise*", "note")
    )
    for d in ids:
        u = lambda s: units.get(f"rc_d{d}_{s}", 0)
        reads, spl_n = u("reads"), u("guarded_spl_n")
        if reads == 0:
            continue
        guarded_ns = (u("guarded_spl_ns") * (reads / spl_n) if spl_n else 0) + u(
            "scan_guarded_ns"
        )
        crit_us = units.get(f"lw_d{d}_crit_us", 0)
        guarded_ms = guarded_ns / frames / 1e6
        crit_ms = crit_us / frames / 1e3
        # materialise vs CRIT wall only — full wall needs the debug sidecar.
        if d in SHARDED:
            mat_s, note = "   sharded", "shard!"
        else:
            mat = crit_ms - guarded_ms if crit_us else float("nan")
            mat_s = "%12.3f" % mat
            note = "noisy" if spl_n / max(frames, 1) < 30 else ""
        print(
            "%6d %12.0f %10.0f %12.3f %12.3f %s %8s"
            % (d, reads / frames, spl_n / frames, guarded_ms, crit_ms, mat_s, note)
        )
    print(
        "\n* materialise uses lw_d{id}_crit_us (critical-path attribution), not the full\n"
        "  per-domain wall — use the debug sidecar for the definitive split. Negative values\n"
        "  mean the domain was rarely on the critical path; ignore them there. 'sharded' =\n"
        "  nested worker threads whose guarded time is under-attributed per-domain (their\n"
        "  guarded ms above is a floor, main-thread portion only); 'noisy' = spl_n/frame < 30."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
