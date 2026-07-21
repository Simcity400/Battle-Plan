#!/usr/bin/env python3
"""Fail-closed verifier for the H-A29 observed research-ahead fixture."""

from __future__ import annotations

import argparse
from decimal import Decimal, InvalidOperation
import hashlib
import json
from pathlib import Path
import struct
import zipfile


EXPECTED_EXECUTABLE_SHA256 = (
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
)
EXPECTED_AUTHORITY = "a729_observed_research_ahead_producer_v2"
EXPECTED_ROOT_JOB_ID = "20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0"
EXPECTED_ROOT_MANIFEST_SHA256 = (
    "82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2"
)
EXPECTED_SENTINEL_JOB_ID = (
    "20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012"
)
EXPECTED_SENTINEL_DUMP_SHA256 = (
    "3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2"
)
EXPECTED_UI_YEAR_PRODUCER_RVA = 0x01BCEA60
EXPECTED_EFFECTIVE_VALUE_PRODUCER_RVA = 0x00ABE4C0
EXPECTED_LOCALIZATION_CONSUMER_RVA = 0x02230F20
EXPECTED_FIXED_SCALE = 100_000
MAX_SNAPSHOT_COUNT = 10_000
MAX_SNAPSHOT_BYTES = 128 * 1024 * 1024

VALUE_FIELDS = (
    ("ahead_of_time_years_raw", "ahead_of_time_years"),
    ("ahead_of_time_penalty_raw", "ahead_of_time_penalty_pct"),
    ("ahead_of_time_reduction_years_raw", "ahead_of_time_reduction_years"),
    ("ahead_of_time_reduced_penalty_raw", "ahead_of_time_reduced_penalty_pct"),
)
A29_FIELDS = {
    "ahead_of_time_fixed_scale",
    "ahead_of_time_provenance",
    *(field for pair in VALUE_FIELDS for field in pair),
}


def exact_int(value, minimum, maximum):
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and minimum <= value <= maximum
    )


def lower_hex_sha256(value):
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def nonzero_lower_hex_sha256(value):
    return lower_hex_sha256(value) and any(character != "0" for character in value)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_json_bytes(raw: bytes, label: str):
    try:
        value = json.loads(raw.decode("utf-8-sig"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("%s is not valid UTF-8 JSON: %s" % (label, error.msg)) from None
    if not isinstance(value, dict):
        raise ValueError("%s JSON root is not an object" % label)
    return value


def validate_manifest(manifest):
    if manifest.get("schema") != "native_collector_session_v1":
        raise ValueError("capture manifest schema mismatch")
    if manifest.get("scheduled_domains") != ["tech_live"]:
        raise ValueError("capture manifest is not exact tech_live-only acquisition")
    if manifest.get("stream_format") != "compact":
        raise ValueError("capture manifest is not a compact stream")
    if manifest.get("collector_control_errors") != [] or manifest.get("parse_errors") != []:
        raise ValueError("capture manifest reports collector-control or parse errors")
    modset = manifest.get("modset")
    if (
        not isinstance(modset, dict)
        or modset.get("resolved") is not True
        or modset.get("errors") != []
    ):
        raise ValueError("capture manifest modset/executable preflight is unresolved")
    game = modset.get("game")
    if (
        not isinstance(game, dict)
        or game.get("exe_sha256") != EXPECTED_EXECUTABLE_SHA256
    ):
        raise ValueError("capture manifest executable is not exact a729")
    compact = manifest.get("compact_stream")
    if not isinstance(compact, dict) or compact.get("decodable") is not True:
        raise ValueError("capture manifest compact stream is not decodable")
    if compact.get("expected_domains") != ["tech_live"]:
        raise ValueError("compact-stream expected-domain roster is not exact tech_live")
    domains_seen = compact.get("domains_seen")
    if not isinstance(domains_seen, list) or "tech_live" not in domains_seen:
        raise ValueError("compact stream did not observe tech_live")
    per_domain = compact.get("per_domain")
    tech = per_domain.get("tech_live") if isinstance(per_domain, dict) else None
    if not isinstance(tech, dict) or not exact_int(
        tech.get("data_record_count"), 1, (1 << 64) - 1
    ):
        raise ValueError("compact stream has no tech_live data record")


def snapshot_documents_from_replay(path: Path, external_manifest):
    try:
        archive = zipfile.ZipFile(path, "r")
    except (OSError, zipfile.BadZipFile):
        raise ValueError("replay is not a readable .battleplan ZIP container") from None
    with archive:
        embedded_infos = [
            info
            for info in archive.infolist()
            if not info.is_dir()
            and info.filename == "recorder/native_session_manifest.json"
        ]
        if len(embedded_infos) != 1:
            raise ValueError("replay does not contain exactly one recorder manifest")
        try:
            embedded_raw = archive.read(embedded_infos[0])
        except (OSError, RuntimeError, zipfile.BadZipFile):
            raise ValueError("embedded recorder manifest is unreadable") from None
        embedded_manifest = load_json_bytes(
            embedded_raw, "recorder/native_session_manifest.json"
        )
        validate_manifest(embedded_manifest)
        if not exact_int(external_manifest.get("pid"), 1, (1 << 32) - 1):
            raise ValueError("external capture manifest pid is malformed")
        if (
            not isinstance(external_manifest.get("session_id"), str)
            or not external_manifest["session_id"]
        ):
            raise ValueError("external capture manifest session_id is absent")
        for field in (
            "schema",
            "session_id",
            "pid",
            "replay_game_id",
            "dll",
            "duration_ms",
            "sample_ms",
            "stream_format",
            "scheduled_domains",
            "raw_stream",
        ):
            if embedded_manifest.get(field) != external_manifest.get(field):
                raise ValueError(
                    "embedded and external capture manifests differ at %s" % field
                )
        infos = [
            info
            for info in archive.infolist()
            if not info.is_dir() and info.filename.endswith(".snapshot.json")
        ]
        names = [info.filename for info in infos]
        if len(names) != len(set(names)):
            raise ValueError("replay contains duplicate snapshot member names")
        if not infos:
            raise ValueError("replay contains no folded snapshot JSON members")
        if len(infos) > MAX_SNAPSHOT_COUNT:
            raise ValueError("replay exceeds the bounded snapshot-count limit")
        documents = []
        for info in sorted(infos, key=lambda item: item.filename):
            if info.file_size > MAX_SNAPSHOT_BYTES:
                raise ValueError("replay snapshot exceeds the bounded size limit")
            try:
                raw = archive.read(info)
            except (OSError, RuntimeError, zipfile.BadZipFile):
                raise ValueError(
                    "replay snapshot member is unreadable: %s" % info.filename
                ) from None
            documents.append(
                {
                    "source_kind": "replay_snapshot",
                    "source_name": info.filename,
                    "sha256": sha256_bytes(raw),
                    "document": load_json_bytes(raw, info.filename),
                }
            )
        return documents, sha256_bytes(embedded_raw)


def snapshot_document_from_file(path: Path):
    try:
        raw = path.read_bytes()
    except OSError:
        raise ValueError("snapshot input is unreadable") from None
    if len(raw) > MAX_SNAPSHOT_BYTES:
        raise ValueError("snapshot input exceeds the bounded size limit")
    return {
        "source_kind": "snapshot",
        "source_name": path.name,
        "sha256": sha256_bytes(raw),
        "document": load_json_bytes(raw, path.name),
    }


def decimal_matches_raw(value, raw):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    try:
        decimal = Decimal(str(value))
    except InvalidOperation:
        return False
    return decimal.is_finite() and decimal * EXPECTED_FIXED_SCALE == raw


def expected_source_record_sha256(
    provenance, research_slot_index, tech_def_index_raw, raw_values
):
    canonical = bytearray(b"the game:a729:a29-observed-producer:v2\0")
    canonical.extend(EXPECTED_EXECUTABLE_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_ROOT_MANIFEST_SHA256.encode("ascii"))
    canonical.extend(EXPECTED_SENTINEL_DUMP_SHA256.encode("ascii"))
    canonical.extend(struct.pack("<Q", provenance["callback_sequence"]))
    canonical.extend(struct.pack("<I", provenance["callback_thread_id"]))
    canonical.extend(struct.pack("<Q", provenance["callback_qpc_ticks"]))
    canonical.extend(struct.pack("<Q", provenance["qpc_frequency_hz"]))
    canonical.extend(struct.pack("<i", provenance["producer_game_hour_raw"]))
    canonical.extend(struct.pack("<I", research_slot_index))
    canonical.extend(struct.pack("<I", tech_def_index_raw))
    canonical.extend(struct.pack("<I", provenance["localization_branch"]))
    for field, _ in VALUE_FIELDS:
        canonical.extend(struct.pack("<q", raw_values[field]))
    return hashlib.sha256(canonical).hexdigest()


def validate_provenance(
    provenance, snapshot, research_slot_index, tech_def_index_raw, raw_values
):
    if not isinstance(provenance, dict):
        raise ValueError("ahead_of_time_provenance is absent or not an object")
    exact_text = {
        "authority": EXPECTED_AUTHORITY,
        "executable_sha256": EXPECTED_EXECUTABLE_SHA256,
        "exact_root_job_id": EXPECTED_ROOT_JOB_ID,
        "exact_root_manifest_sha256": EXPECTED_ROOT_MANIFEST_SHA256,
        "sentinel_job_id": EXPECTED_SENTINEL_JOB_ID,
        "sentinel_dump_sha256": EXPECTED_SENTINEL_DUMP_SHA256,
    }
    for field, expected in exact_text.items():
        if provenance.get(field) != expected:
            raise ValueError("A29 provenance %s mismatch" % field)
    exact_rvas = {
        "ui_year_producer_rva": EXPECTED_UI_YEAR_PRODUCER_RVA,
        "effective_value_producer_rva": EXPECTED_EFFECTIVE_VALUE_PRODUCER_RVA,
        "localization_consumer_rva": EXPECTED_LOCALIZATION_CONSUMER_RVA,
    }
    for field, expected in exact_rvas.items():
        if provenance.get(field) != expected:
            raise ValueError("A29 provenance %s mismatch" % field)
    for field, maximum in (
        ("callback_sequence", (1 << 64) - 1),
        ("callback_thread_id", (1 << 32) - 1),
        ("callback_qpc_ticks", (1 << 64) - 1),
        ("qpc_frequency_hz", (1 << 64) - 1),
        ("producer_game_hour_raw", (1 << 31) - 1),
    ):
        if not exact_int(provenance.get(field), 1, maximum):
            raise ValueError("A29 provenance %s is not a positive exact integer" % field)
    if not exact_int(provenance.get("localization_branch"), 0, 2):
        raise ValueError("A29 provenance localization_branch is outside 0..2")
    source_sha = provenance.get("source_record_sha256")
    if not nonzero_lower_hex_sha256(source_sha):
        raise ValueError("A29 source-record SHA-256 is absent, non-lowerhex, or zero")
    if source_sha != expected_source_record_sha256(
        provenance, research_slot_index, tech_def_index_raw, raw_values
    ):
        raise ValueError("A29 source-record SHA-256 does not bind the portable producer row")
    snapshot_hour = snapshot.get("game_hour_raw")
    if snapshot_hour is not None:
        if not exact_int(snapshot_hour, 1, (1 << 31) - 1):
            raise ValueError("snapshot game_hour_raw is not a positive signed i32")
        if provenance["producer_game_hour_raw"] != snapshot_hour:
            raise ValueError("A29 producer hour differs from snapshot game_hour_raw")
    return {
        "authority": provenance["authority"],
        "callback_sequence": provenance["callback_sequence"],
        "callback_thread_id": provenance["callback_thread_id"],
        "producer_game_hour_raw": provenance["producer_game_hour_raw"],
        "localization_branch": provenance.get("localization_branch"),
        "source_record_sha256": source_sha,
    }


def validate_a29_row(row, snapshot):
    slot_index = row.get("slot_index")
    tech_handle_raw = row.get("tech_handle_raw")
    tech_def_index_raw = row.get("tech_def_index_raw")
    tech_name = row.get("tech_name")
    if not exact_int(slot_index, 0, (1 << 32) - 1):
        raise ValueError("slot_index is not a u32")
    if (
        not exact_int(tech_handle_raw, 0, (1 << 32) - 1)
        or not exact_int(tech_def_index_raw, 0, (1 << 32) - 1)
        or tech_handle_raw != tech_def_index_raw
    ):
        raise ValueError("active technology handle/definition identity is malformed")
    if tech_name is not None and (
        not isinstance(tech_name, str) or not tech_name.strip()
    ):
        raise ValueError("active technology name is present but malformed")
    if row.get("ahead_of_time_fixed_scale") != EXPECTED_FIXED_SCALE:
        raise ValueError("ahead-of-time fixed scale is not 100000")
    raw_values = {}
    values = {}
    for raw_field, value_field in VALUE_FIELDS:
        raw = row.get(raw_field)
        value = row.get(value_field)
        if not exact_int(raw, 0, 10_000_000_000):
            raise ValueError("%s is outside the admitted fixed-five range" % raw_field)
        if not decimal_matches_raw(value, raw):
            raise ValueError("%s is not exactly %s / 100000" % (value_field, raw_field))
        raw_values[raw_field] = raw
        values[value_field] = value
    provenance = validate_provenance(
        row.get("ahead_of_time_provenance"),
        snapshot,
        slot_index,
        tech_def_index_raw,
        raw_values,
    )
    branch = provenance["localization_branch"]
    penalty = raw_values["ahead_of_time_penalty_raw"]
    reduction = raw_values["ahead_of_time_reduction_years_raw"]
    reduced = raw_values["ahead_of_time_reduced_penalty_raw"]
    if branch == 1:
        if reduction != 0 or reduced != penalty:
            raise ValueError(
                "unreduced localization branch does not carry zero reduction and VALUE equality"
            )
    elif branch == 2:
        if reduction <= 0:
            raise ValueError(
                "reduced localization branch does not carry a positive observed reduction"
            )
    else:
        raise ValueError("A29 localization branch is not exact branch 1 or 2")
    return {
        "slot_index": slot_index,
        "tech_handle_raw": tech_handle_raw,
        "tech_def_index_raw": tech_def_index_raw,
        "tech_name": tech_name,
        "raw_values": raw_values,
        "values": values,
        "provenance": provenance,
    }


def inspect_snapshots(snapshots):
    accepted = []
    rejected = []
    snapshot_count = 0
    for source in snapshots:
        snapshot_count += 1
        snapshot = source["document"]
        countries = snapshot.get("countries")
        if countries is None:
            continue
        if not isinstance(countries, dict):
            rejected.append("%s: countries is not an object" % source["source_name"])
            continue
        for country_key in sorted(countries):
            country = countries[country_key]
            research = country.get("research") if isinstance(country, dict) else None
            rows = research.get("in_progress_live") if isinstance(research, dict) else None
            if rows is None:
                continue
            if not isinstance(rows, list):
                rejected.append(
                    "%s:%s: research.in_progress_live is not a list"
                    % (source["source_name"], country_key)
                )
                continue
            for index, row in enumerate(rows):
                if not isinstance(row, dict):
                    rejected.append(
                        "%s:%s:%d: active research row is not an object"
                        % (source["source_name"], country_key, index)
                    )
                    continue
                if not any(field in row for field in A29_FIELDS):
                    continue
                label = "%s:%s:%d" % (source["source_name"], country_key, index)
                try:
                    witness = validate_a29_row(row, snapshot)
                    witness.update(
                        {
                            "country_key": country_key,
                            "snapshot_member": source["source_name"],
                            "snapshot_sha256": source["sha256"],
                        }
                    )
                    accepted.append(witness)
                except ValueError as error:
                    rejected.append("%s: %s" % (label, error))
    if rejected:
        raise ValueError("malformed A29-bearing row: %s" % rejected[0])
    if not accepted:
        raise ValueError(
            "no authenticated A29 row found; open an active research technology "
            "year/ahead tooltip and keep it visible through a same-hour tech_live poll"
        )
    accepted.sort(
        key=lambda row: (
            row["snapshot_member"],
            row["country_key"],
            row["slot_index"],
            row["provenance"]["callback_sequence"],
        )
    )
    return snapshot_count, accepted


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-manifest", required=True)
    parser.add_argument("--replay")
    parser.add_argument("--snapshot", action="append", default=[])
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    out_path = Path(args.out).resolve()
    manifest_path = Path(args.capture_manifest).resolve()
    replay_path = Path(args.replay).resolve() if args.replay else None
    snapshot_paths = [Path(value).resolve() for value in args.snapshot]
    errors = []
    details = {}
    inputs = []
    try:
        if replay_path is None and not snapshot_paths:
            raise ValueError("at least one --replay or --snapshot input is required")
        try:
            manifest_raw = manifest_path.read_bytes()
        except OSError:
            raise ValueError("capture manifest is unreadable") from None
        inputs.append(
            {
                "kind": "capture_manifest",
                "sha256": sha256_bytes(manifest_raw),
            }
        )
        manifest = load_json_bytes(manifest_raw, "capture manifest")
        validate_manifest(manifest)
        snapshots = []
        if replay_path is not None:
            if not replay_path.is_file():
                raise ValueError("replay input is absent or not a file")
            inputs.append({"kind": "replay", "sha256": sha256_file(replay_path)})
            replay_snapshots, embedded_manifest_sha256 = (
                snapshot_documents_from_replay(replay_path, manifest)
            )
            snapshots.extend(replay_snapshots)
            inputs.append(
                {
                    "kind": "embedded_capture_manifest",
                    "sha256": embedded_manifest_sha256,
                }
            )
        for path in snapshot_paths:
            snapshots.append(snapshot_document_from_file(path))
            inputs.append({"kind": "snapshot", "sha256": sha256_file(path)})
        snapshot_count, witnesses = inspect_snapshots(snapshots)
        details = {
            "snapshot_count_inspected": snapshot_count,
            "authenticated_witness_count": len(witnesses),
            "witnesses": witnesses,
        }
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        errors.append(str(error))
    except Exception as error:
        # A malformed or hostile evidence bundle must still produce a stable
        # FAIL receipt instead of accidentally becoming an ungraded artifact.
        errors.append("verifier rejected input: %s" % type(error).__name__)

    accepted = not errors
    receipt = {
        "schema": "hoi4.a29.research_ahead_fixture_verification.v1",
        "status": "PASS" if accepted else "FAIL",
        "production_admission_ready": accepted,
        "checks": {
            "exact_a729_tech_live_capture": accepted,
            "observed_localization_provenance": accepted,
            "same_hour_producer_join": accepted,
            "fixed_five_raw_value_equality": accepted,
            "localization_branch_semantics": accepted,
            "pointer_free_source_hash": accepted,
        },
        "details": details,
        "errors": errors,
        "inputs": inputs,
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print("A29 fixture %s: %s" % (receipt["status"], out_path))
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
