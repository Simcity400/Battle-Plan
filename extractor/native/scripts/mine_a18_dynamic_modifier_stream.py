#!/usr/bin/env python3
"""Mine domain-67 modifier-set transitions from a retained H4CR stream.

This is an evidence miner, not a promotion verifier.  Domain 67 v1 carries
only the owning country database id and modifier definition ids.  Therefore
the output deliberately labels removals as observed set transitions; it does
not infer expiry cause, remaining days, enabled state, or value vectors.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path

import zstandard


HEADER = struct.Struct("<4sHHQiHHII")
DOMAIN = 67
DOMAIN_DATA = 3


def _u32(buf: bytes, offset: int) -> int:
    return struct.unpack_from("<I", buf, offset)[0]


def _parse_domain67(payload: bytes) -> tuple[dict[int, tuple[str, ...]], int]:
    if len(payload) < 20:
        raise ValueError("domain-67 payload shorter than 20-byte header")
    country_count = _u32(payload, 8)
    total_id_count = _u32(payload, 12)
    status_flags = _u32(payload, 16)
    table_end = 20 + country_count * 12
    if table_end > len(payload):
        raise ValueError("domain-67 row table is truncated")

    ids: list[str] = []
    cursor = table_end
    for _ in range(total_id_count):
        if cursor + 2 > len(payload):
            raise ValueError("domain-67 id length is truncated")
        length = struct.unpack_from("<H", payload, cursor)[0]
        cursor += 2
        if length == 0 or cursor + length > len(payload):
            raise ValueError("domain-67 id bytes are invalid or truncated")
        ids.append(payload[cursor : cursor + length].decode("utf-8"))
        cursor += length
    if cursor != len(payload):
        raise ValueError("domain-67 payload has trailing bytes")

    rows: dict[int, tuple[str, ...]] = {}
    expected_offset = 0
    for index in range(country_count):
        base = 20 + index * 12
        owner = _u32(payload, base)
        count = _u32(payload, base + 4)
        offset = _u32(payload, base + 8)
        if owner == 0 or count == 0 or offset != expected_offset:
            raise ValueError("domain-67 owner row violates v1 contiguous-window contract")
        end = offset + count
        if end > len(ids) or owner in rows:
            raise ValueError("domain-67 owner row window is invalid")
        rows[owner] = tuple(ids[offset:end])
        expected_offset = end
    if expected_offset != len(ids):
        raise ValueError("domain-67 owner rows do not cover the id blob")
    return rows, status_flags


def _stored_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _read_exact(reader, length: int) -> bytes:
    parts = []
    remaining = length
    while remaining:
        part = reader.read(remaining)
        if not part:
            break
        parts.append(part)
        remaining -= len(part)
    return b"".join(parts)


def mine(stream_path: Path, tolerate_truncated_final_record: bool = False) -> dict:
    decoded_digest = hashlib.sha256()
    decoded_bytes = 0
    total_records = 0
    d67_records = 0
    first_hour = None
    last_hour = None
    prior: dict[int, tuple[str, ...]] | None = None
    first_d67_pairs: set[tuple[int, str]] = set()
    seen_first_admitted_frame = False
    final_d67_pairs: set[tuple[int, str]] = set()
    # pair -> (preceding absent full frame, first present full frame,
    #          left-censored at the start of this trustworthy segment)
    open_since: dict[tuple[int, str], tuple[int | None, int, bool]] = {}
    additions: list[dict] = []
    removals: list[dict] = []
    closed_lifetimes: list[dict] = []
    status_flags_or = 0
    envelope_flags_or = 0
    prior_sequence = None
    prior_d67_hour = None
    prior_admitted_hour = None
    d67_max_hour_gap = 0
    d67_nonconsecutive_hour_intervals = 0
    admitted_full_replace_records = 0
    partial_status_records = 0
    censored_segments = 0
    censored_open_pair_windows = 0
    terminal_error = None

    with stream_path.open("rb") as stored:
        leading = stored.read(4)
        stored.seek(0)
        if leading == b"\x28\xb5\x2f\xfd":
            storage = "zstd"
            reader = zstandard.ZstdDecompressor().stream_reader(stored)
        elif leading == b"H4CR":
            storage = "raw"
            reader = stored
        else:
            raise ValueError(f"unknown stream storage magic {leading.hex()}")
        try:
            while True:
                header = _read_exact(reader, HEADER.size)
                if not header:
                    break
                if len(header) != HEADER.size:
                    if tolerate_truncated_final_record:
                        terminal_error = "truncated H4CR header"
                        break
                    raise ValueError("truncated H4CR header")
                (magic, version, header_len, sequence, game_hour, domain,
                 record_type, payload_len, record_flags) = HEADER.unpack(header)
                if magic != b"H4CR" or version != 1 or header_len != HEADER.size:
                    raise ValueError("unsupported H4CR framing")
                envelope_flags_or |= record_flags
                if prior_sequence is not None and sequence != prior_sequence + 1:
                    raise ValueError("non-contiguous H4CR sequence")
                prior_sequence = sequence
                payload = _read_exact(reader, payload_len)
                if len(payload) != payload_len:
                    if tolerate_truncated_final_record:
                        terminal_error = (
                            f"truncated H4CR payload: expected {payload_len}, "
                            f"read {len(payload)}"
                        )
                        break
                    raise ValueError("truncated H4CR payload")
                # Hash only complete records.  In truncated-prefix mode the
                # terminal header/partial payload is deliberately excluded,
                # so source.decoded_* identifies the admitted record prefix.
                decoded_digest.update(header)
                decoded_digest.update(payload)
                decoded_bytes += len(header) + len(payload)
                total_records += 1

                if domain != DOMAIN or record_type != DOMAIN_DATA:
                    continue
                rows, flags = _parse_domain67(payload)
                status_flags_or |= flags
                d67_records += 1
                first_hour = game_hour if first_hour is None else first_hour
                last_hour = game_hour
                if prior_d67_hour is not None:
                    gap = game_hour - prior_d67_hour
                    d67_max_hour_gap = max(d67_max_hour_gap, gap)
                    if gap != 1:
                        d67_nonconsecutive_hour_intervals += 1
                prior_d67_hour = game_hour
                if flags != 0:
                    # d67 is full-replace only when status is zero.  Production
                    # fold retains the previous set on PARTIAL_DATA/failure, so
                    # treating these rows as a replacement would fabricate
                    # removals.  Break the trustworthy transition segment and
                    # left-censor the next successful baseline.
                    partial_status_records += 1
                    censored_segments += 1
                    censored_open_pair_windows += len(open_since)
                    open_since.clear()
                    prior = None
                    prior_admitted_hour = None
                    final_d67_pairs = set()
                    continue
                admitted_full_replace_records += 1
                current = {
                    (owner, modifier)
                    for owner, modifiers in rows.items()
                    for modifier in modifiers
                }
                if prior is None:
                    if not seen_first_admitted_frame:
                        first_d67_pairs = set(current)
                        seen_first_admitted_frame = True
                    for pair in current:
                        open_since[pair] = (None, game_hour, True)
                else:
                    previous = {
                        (owner, modifier)
                        for owner, modifiers in prior.items()
                        for modifier in modifiers
                    }
                    for owner, modifier in sorted(current - previous):
                        additions.append({
                            "game_hour_raw": game_hour,
                            "preceding_absent_frame_game_hour_raw": prior_admitted_hour,
                            "owner_tag_db_id": owner,
                            "modifier_id": modifier,
                        })
                        open_since[(owner, modifier)] = (
                            prior_admitted_hour, game_hour, False
                        )
                    for owner, modifier in sorted(previous - current):
                        opened = open_since.pop((owner, modifier), None)
                        preceding_absent = None if opened is None else opened[0]
                        started = None if opened is None else opened[1]
                        left_censored = True if opened is None else opened[2]
                        removals.append({
                            "game_hour_raw": game_hour,
                            "last_present_frame_game_hour_raw": prior_admitted_hour,
                            "owner_tag_db_id": owner,
                            "modifier_id": modifier,
                            "observed_since_game_hour_raw": started,
                            "observed_lifetime_hours": (
                                None if started is None else game_hour - started
                            ),
                            "left_censored": left_censored,
                        })
                        if started is not None:
                            closed_lifetimes.append({
                                "owner_tag_db_id": owner,
                                "modifier_id": modifier,
                                "first_observed_game_hour_raw": started,
                                "preceding_absent_frame_game_hour_raw": preceding_absent,
                                "last_present_frame_game_hour_raw": prior_admitted_hour,
                                "removed_at_game_hour_raw": game_hour,
                                "observed_lifetime_hours": game_hour - started,
                                "lifetime_lower_bound_hours": (
                                    None if prior_admitted_hour is None
                                    else prior_admitted_hour - started
                                ),
                                "lifetime_upper_bound_hours": (
                                    None if preceding_absent is None
                                    else game_hour - preceding_absent
                                ),
                                "left_censored": left_censored,
                            })
                final_d67_pairs = current
                prior = rows
                prior_admitted_hour = game_hour
        finally:
            if reader is not stored:
                reader.close()

    uncensored = [row for row in closed_lifetimes if not row["left_censored"]]
    return {
        "schema": "hoi4.a18.domain67_retained_transition_mine.v1",
        "authority": "retained_domain67_v1_id_set_observation_only",
        "source": {
            "basename": stream_path.name,
            "storage": storage,
            "stored_bytes": stream_path.stat().st_size,
            "stored_sha256": _stored_sha256(stream_path),
            "decoded_bytes": decoded_bytes,
            "decoded_sha256": decoded_digest.hexdigest(),
        },
        "framing": {
            "records": total_records,
            "sequences_contiguous": True,
            "domain67_data_records": d67_records,
            "first_game_hour_raw": first_hour,
            "last_game_hour_raw": last_hour,
            "status_flags_or": status_flags_or,
            "admitted_full_replace_records": admitted_full_replace_records,
            "partial_status_records": partial_status_records,
            "censored_segments": censored_segments,
            "censored_open_pair_windows": censored_open_pair_windows,
            "envelope_flags_or": envelope_flags_or,
            "max_domain67_hour_gap": d67_max_hour_gap,
            "nonconsecutive_domain67_hour_intervals": d67_nonconsecutive_hour_intervals,
            "physical_eof_complete": terminal_error is None,
            "terminal_error": terminal_error,
        },
        "observations": {
            "first_frame_pairs": len(first_d67_pairs),
            "final_frame_pairs": len(final_d67_pairs),
            "additions": len(additions),
            "removals": len(removals),
            "closed_lifetimes": len(closed_lifetimes),
            "uncensored_closed_lifetimes": len(uncensored),
        },
        "additions": additions,
        "removals": removals,
        "closed_lifetimes": closed_lifetimes,
        "right_censored_final_pairs": [
            {
                "owner_tag_db_id": owner,
                "modifier_id": modifier,
                "preceding_absent_frame_game_hour_raw": (
                    None if open_since.get((owner, modifier)) is None
                    else open_since[(owner, modifier)][0]
                ),
                "observed_since_game_hour_raw": (
                    None if open_since.get((owner, modifier)) is None
                    else open_since[(owner, modifier)][1]
                ),
                "left_censored": (
                    True if open_since.get((owner, modifier)) is None
                    else open_since[(owner, modifier)][2]
                ),
            }
            for owner, modifier in sorted(final_d67_pairs)
        ],
        "prohibited_inference": [
            "a set removal does not by itself prove timed expiry",
            "domain 67 v1 does not carry remaining days, enabled state, value vectors, or source tag",
            "numeric owner database ids are not portable country tags",
            "left- or right-censored observations are not exact lifetimes",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("stream", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--allow-truncated-prefix", action="store_true")
    args = parser.parse_args()
    report = mine(args.stream.resolve(), args.allow_truncated_prefix)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report["framing"], sort_keys=True))
    print(json.dumps(report["observations"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
