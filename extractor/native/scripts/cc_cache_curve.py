#!/usr/bin/env python3
"""Lane CC: the checkpoint-density curve of a binary replay cache, from its index.

`hour_index.h4ix` carries, for every game hour, the byte offset and LENGTH of
that hour's record in `checkpoints.h4cp` plus the position of the checkpoint it
seeks from. A checkpoint record is one whose `checkpoint_position` names itself;
every other record is a delta.

WHAT MAKES THE CURVE ALMOST EXACT, AND WHERE IT IS NOT. `verify_every_hour`
applies each delta to the value reconstructed for the PREVIOUS hour, not to the
checkpoint, so **a delta record's size does not depend on the checkpoint
interval**. Changing the interval only decides which hours are checkpoints
instead of deltas; every hour that stays a delta keeps the exact record this
build already produced, and those are summed here rather than modelled.

TWO quantities are modelled, and both are reported per row so a reader can see
which rows rest on them:

* an hour that becomes a CHECKPOINT but was a delta here has no checkpoint
  encoding in this build, so it is priced at the mean of the four observed
  checkpoints. Those four span 1,209 to 27,357,593 bytes, which is why the
  tight-interval rows are indicative only.
* an hour that WAS a checkpoint here and becomes a delta at a longer interval
  has no delta encoding, so it is priced at the mean delta. At interval 644 the
  entire projected saving rests on three such records.

The agreement at the build's own interval is true by construction and validates
nothing else.

This is therefore a PROJECTION, not a rebuild. Measuring another interval means
building another cache.

The index format is `ReplayCacheIndex::to_bytes`
(portable_replay/native/snapshot/src/replay_cache.rs). Little-endian
throughout; strings are u32 length + bytes.

Read-only.
"""

from __future__ import annotations

import argparse
import json
import math
import struct
import sys

INDEX_MAGIC = b"H4IX"
FORMAT_VERSION = 2


class Reader:
    def __init__(self, data):
        self.data = data
        self.pos = 0

    def take(self, n):
        if self.pos + n > len(self.data):
            raise SystemExit(f"hour index truncated at {self.pos}")
        out = self.data[self.pos:self.pos + n]
        self.pos += n
        return out

    def u8(self):
        return self.take(1)[0]

    def u16(self):
        return struct.unpack("<H", self.take(2))[0]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def u64(self):
        return struct.unpack("<Q", self.take(8))[0]

    def i64(self):
        return struct.unpack("<q", self.take(8))[0]

    def string(self):
        return self.take(self.u32()).decode("utf-8")


def parse_index(path):
    with open(path, "rb") as handle:
        reader = Reader(handle.read())
    if reader.take(4) != INDEX_MAGIC:
        raise SystemExit("not an H4IX hour index")
    version = reader.u16()
    flags = reader.u16()
    if version != FORMAT_VERSION or flags != 0:
        raise SystemExit(f"unsupported hour index version {version}/{flags}")
    identity_mode = reader.u8()
    if reader.take(3) != b"\0\0\0":
        raise SystemExit("reserved identity bytes are nonzero")
    header = {
        "identity_mode": identity_mode,
        "checkpoint_interval_hours": reader.u32(),
        "source_len": reader.u64(),
        "cache_len": reader.u64(),
        "source_sha256": reader.take(32).hex(),
        "cache_sha256": reader.take(32).hex(),
    }
    header["game_id"] = reader.string()
    header["schema_version"] = reader.string()
    hour_count = reader.u64()
    hours = []
    for position in range(hour_count):
        entry = {
            "hour_index": reader.i64(),
            "h4cr_start_offset": reader.u64(),
            "h4cr_end_offset": reader.u64(),
            "cache_record_offset": reader.u64(),
            "cache_record_len": reader.u64(),
            "checkpoint_position": reader.u32(),
            "delta_steps": reader.u32(),
        }
        reader.take(32)
        entry["date"] = reader.string()
        entry["position"] = position
        entry["is_checkpoint"] = entry["checkpoint_position"] == position
        hours.append(entry)
    if reader.pos != len(reader.data):
        raise SystemExit("trailing bytes after the hour index")
    return header, hours


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("hour_index")
    ap.add_argument("--intervals", default="1,6,12,24,48,168,336,644")
    args = ap.parse_args()
    header, hours = parse_index(args.hour_index)

    checkpoints = [h for h in hours if h["is_checkpoint"]]
    deltas = [h for h in hours if not h["is_checkpoint"]]
    ckpt_bytes = sum(h["cache_record_len"] for h in checkpoints)
    delta_bytes = sum(h["cache_record_len"] for h in deltas)
    ckpt_mean = ckpt_bytes / len(checkpoints) if checkpoints else 0
    delta_mean = delta_bytes / len(deltas) if deltas else 0

    curve = []
    total_hours = len(hours)
    for interval in [int(v) for v in args.intervals.split(",") if v.strip()]:
        checkpoint_positions = set(range(0, total_hours, interval))
        measured_delta_bytes = sum(
            h["cache_record_len"] for h in hours
            if h["position"] not in checkpoint_positions and not h["is_checkpoint"]
        )
        modelled = [h for h in hours
                    if h["position"] in checkpoint_positions and not h["is_checkpoint"]]
        kept_checkpoints = [h for h in hours
                            if h["position"] in checkpoint_positions and h["is_checkpoint"]]
        demoted = [h for h in hours
                   if h["is_checkpoint"] and h["position"] not in checkpoint_positions]
        projected = (
            measured_delta_bytes
            + sum(h["cache_record_len"] for h in kept_checkpoints)
            + len(modelled) * ckpt_mean
            + len(demoted) * delta_mean
        )
        curve.append({
            "checkpoint_interval_hours": interval,
            "checkpoints": len(checkpoint_positions),
            "deltas": total_hours - len(checkpoint_positions),
            "max_seek_deltas": min(interval, total_hours) - 1,
            "measured_delta_bytes": measured_delta_bytes,
            "checkpoint_hours_modelled_at_the_mean": len(modelled),
            "checkpoint_hours_demoted_to_a_modelled_delta": len(demoted),
            "projected_cache_bytes": int(round(projected)),
        })

    print(json.dumps({
        "header": header,
        "hours": total_hours,
        "checkpoint_records": len(checkpoints),
        "checkpoint_bytes": ckpt_bytes,
        "checkpoint_mean_bytes": ckpt_mean,
        "checkpoint_record_lengths": [h["cache_record_len"] for h in checkpoints],
        "checkpoint_min_bytes": min((h["cache_record_len"] for h in checkpoints), default=0),
        "checkpoint_max_bytes": max((h["cache_record_len"] for h in checkpoints), default=0),
        "delta_records": len(deltas),
        "delta_bytes": delta_bytes,
        "delta_mean_bytes": delta_mean,
        "delta_share_of_cache": round(delta_bytes / (ckpt_bytes + delta_bytes), 4)
        if (ckpt_bytes + delta_bytes) else None,
        "max_observed_delta_steps": max(h["delta_steps"] for h in hours),
        "curve": curve,
    }, indent=2))


if __name__ == "__main__":
    sys.exit(main())
