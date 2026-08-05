#!/usr/bin/env python3
"""Lane CC: the retained-pin prize, with the confound removed.

WHY THE OBVIOUS ESTIMATE IS WRONG. The tempting measurement is "a post-own-gap
record costs 8.96 MB and a steady-state record costs 1.94 MB, so retaining the
pin saves the difference". It does not, for two reasons, and both were found by
measuring rather than by argument.

1. THE OBSERVED POST-GAP SIZE IS CONFOUNDED. A post-own-gap record is large
   BECAUSE the producer dropped its pin and was forced to emit whole. Its size
   is not evidence of how much changed; it is evidence of the rule under study.
   Nothing about a retained pin can be read off it.

2. THE STEADY-STATE MEAN IS OFTEN A SIZE NO RECORD HAS. d53 emits a 72-byte
   record on the 78% of hours nothing changed and an 8.96 MB whole snapshot
   otherwise; d47 and d51 emit a ~0.1 KB record most hours and a ~931 KB /
   ~572 KB payload on the rest. Averaging those two populations produces a
   number that describes neither, and a prize computed from it is meaningless.

So the substitute a retained pin would actually have produced is estimated one
of two ways, measurement first:

* MEASURED. Six domains already emit patches ACROSS their own gaps (d81, d47,
  d51, d42, d46, d70), so for them the post-gap patch size is observed, not
  modelled -- and it is flat in the number of hours spanned (d81 1026.6 KB at
  gap-run 0 versus 1068.5 KB at gap-run 6+). Where this observation exists it
  is used and no model is applied.

* MODELLED, where no such observation exists. Let `whole` be the domain's
  whole-payload size and `q = steady_mean / whole` the fraction of the payload
  its changed-only path emits in one hour. Treating the changing units as
  independent across hours, a record spanning k hours carries
  `whole * (1 - (1-q)^k)`. At k = 1 this returns `steady_mean` exactly, so the
  model is anchored on the measurement rather than fitted to it, and it
  saturates at `whole`, which is the correct ceiling.

  The model is an ESTIMATE and is labelled as one. Its error has a KNOWN SIGN.
  Changes that cluster in time -- the same units moving again rather than new
  ones -- make the union over k hours SMALLER than independence predicts, so the
  substitute is smaller and the true prize is LARGER: the model is PESSIMISTIC
  under clustering. Anti-correlated change makes it optimistic. Since telemetry
  of this kind clusters far more often than it anti-correlates, the modelled
  prize should be read as a lower estimate rather than an upper one. The naive
  prize is reported alongside it and is not a bound of any kind either -- it is
  simply what the earlier hand tabulations computed.

Read-only with respect to the corpus.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from collections import defaultdict

H = 32
HDR = struct.Struct("<4sHHQiHHII")
RT_DOMAIN_DATA = 3
RT_DOMAIN_GAP = 5
STATUS_DELTA_DATA = 1 << 10
NON_SHARED_HEADER_DOMAINS = {48, 53, 83, 91, 93}


def collect(path):
    """domain -> list of (gap_run_before, payload_len, is_delta, is_first)."""
    gap_run = defaultdict(int)
    seen = defaultdict(bool)
    rows = defaultdict(list)
    with open(path, "rb") as handle:
        while True:
            header = handle.read(H)
            if not header:
                break
            (_m, _v, _h, _s, _hour, domain, rtype, plen, _f) = HDR.unpack_from(header, 0)
            if rtype == RT_DOMAIN_GAP:
                handle.seek(plen, 1)
                gap_run[domain] += 1
                continue
            if rtype != RT_DOMAIN_DATA:
                handle.seek(plen, 1)
                continue
            payload = handle.read(plen)
            if domain in NON_SHARED_HEADER_DOMAINS or plen < 20:
                is_delta = None
            else:
                is_delta = bool(struct.unpack_from("<I", payload, 16)[0] & STATUS_DELTA_DATA)
            rows[domain].append((gap_run[domain], plen, is_delta, not seen[domain]))
            seen[domain] = True
            gap_run[domain] = 0
    return rows


def analyse(domain, records):
    steady = [(k, n, d) for k, n, d, first in records if k == 0 and not first]
    postgap = [(k, n, d) for k, n, d, first in records if k > 0 and not first]
    if not steady or not postgap:
        return None
    total = sum(r[1] for r in records)

    whole = max(r[1] for r in records)
    steady_mean = sum(n for _k, n, _d in steady) / len(steady)
    q = steady_mean / whole if whole else 0.0

    # MEASURED arm: post-gap records that already carry a patch today.
    measured_delta = [n for k, n, d in postgap if d]
    steady_delta = [n for k, n, d in steady if d]

    postgap_bytes = sum(n for _k, n, _d in postgap)
    naive_sub = steady_mean * len(postgap)
    naive_prize = max(0.0, postgap_bytes - naive_sub)

    modelled_sub = 0.0
    measured_records = 0
    for k, n, d in postgap:
        if d and steady_delta:
            # already a patch across its own gap: measured, unchanged.
            modelled_sub += n
            measured_records += 1
        else:
            hours = k + 1
            modelled_sub += whole * (1.0 - (1.0 - q) ** hours)
    modelled_prize = max(0.0, postgap_bytes - modelled_sub)

    return {
        "domain": domain,
        "records": len(records),
        "payload_bytes": total,
        "whole_payload_bytes": whole,
        "steady_records": len(steady),
        "steady_mean_bytes": steady_mean,
        "changed_fraction_per_hour_q": round(q, 6),
        "postgap_records": len(postgap),
        "postgap_bytes": postgap_bytes,
        "postgap_records_already_patched": measured_records,
        "mean_gap_run": round(sum(k for k, _n, _d in postgap) / len(postgap), 3),
        "naive_prize_bytes": int(round(naive_prize)),
        "modelled_prize_bytes": int(round(modelled_prize)),
        "status_word_readable": domain not in NON_SHARED_HEADER_DOMAINS,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("stream")
    ap.add_argument("--registry", default="extractor/native/domain_registry.json")
    args = ap.parse_args()
    names = {}
    try:
        with open(args.registry, encoding="utf-8") as handle:
            names = {e["id"]: e["name"] for e in json.load(handle)["domains"]}
    except OSError:
        pass

    rows = collect(args.stream)
    out = []
    for domain in sorted(rows):
        entry = analyse(domain, rows[domain])
        if entry is None:
            continue
        entry["name"] = names.get(domain, "?")
        out.append(entry)
    out.sort(key=lambda e: -e["modelled_prize_bytes"])
    print(json.dumps({"domains": out}, indent=2))


if __name__ == "__main__":
    sys.exit(main())
