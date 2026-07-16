#!/usr/bin/env python3
"""D57 criterion-4 harness: compare the collector's opt-in game-speed (domain 57)
EMISSION against the raw memory words recorded by the read-only D57 speed probe.

This is the last tool the D57 acceptance gate (I1_D57_SPEED_EVIDENCE.md §7,
criterion 4) depends on: it asserts that "the collector opt-in emission is
compared with the raw words, with no coercion and no accepted out-of-range value".
It emits a single fail-closed verdict artifact; it reads NO process memory and
changes NO game/collector state (pure-Python, artifact-consuming only).

WHAT IT PROVES (all grounded, nothing invented)
------------------------------------------------
The raw-canonical ruling (I1_D57_SPEED_EVIDENCE.md §7, OWNER DECISION 2026-07-16):
the wire/canonical datum is the RAW slot `0..4` straight out of engine memory
(`CGameState+0x4BC`); the displayed `1..5` is a derived consumer view
(`displayed = raw + 1`) that is NEVER stored. The collector honours this: it
emits `SessionEventRow{kind: GameSpeed, value_i32: <raw 0..4>}`
(collector/src/lib.rs:26929-26942), where the value comes from
`read_domain57_game_speed()` which fail-closes any word outside
`0..=DOMAIN57_MAX_GAME_SPEED (=4)` (collector/src/lib.rs:27210-27222;
compact_contract/src/build_profile.rs:94,102).

This harness re-derives the ground-truth raw word for each of the five UI speed
states from the SEALED, VERIFIED probe transcript (the probe recorded
`speed_raw = i32[CGameState+0x4BC]` for UI speeds 1..5, i.e. raw 0..4), then
decodes the collector's opt-in emission stream byte-for-byte and asserts, for
every operator-attested alignment, that the emitted `value_i32` EQUALS that
raw word exactly. No coercion (a displayed `1..5` datum makes UI-speed-1's frame
carry `1 != 0` -> FAIL; a value `5` is out of range -> FAIL). No out-of-range
value is accepted ANYWHERE in the stream. Absent emission (opt-in off / no
GameSpeed rows) is a clear refusal, never a pass.

WIRE FORMAT PROVENANCE (byte-exact; NOT invented)
-------------------------------------------------
Emission is the domain-57 compact record stream. Framing constants and layout:
  * Compact record header (H4CR, 32 bytes) -- append_compact_record() in
    lanes/session_multiplayer_metadata_live/src/lib.rs:587-605, parsed by
    compact_contract/src/record_header.rs:60-89:
        [0..4]  magic "H4CR"          [16..20] game_hour_raw i32
        [4..6]  version u16 (=1)      [20..22] domain_id u16 (=57)
        [6..8]  header_len u16 (=32)  [22..24] record_type u16 (DOMAIN_DATA=3)
        [8..16] sequence u64          [24..28] payload_len u32
                                      [28..32] record_flags u32
  * Domain-57 payload (S57M, 64-byte header) -- encode_payload()/gate_payload()
    in the same file :226-585:
        [0..4] magic "S57M"  [4..6] version u16(=1)  [6..8] header_len u16(=64)
        [8..10] domain_id u16(=57)  [10..12] player_row_len u16(=40)
        [12..14] event_row_len u16(=64)  [16..20] game_hour_raw i32
        [24..28] source_flags u32  [28..32] player_count u32
        [32..36] event_count u32  [36..40] string_bytes u32
  * Session event row (64 bytes) -- encode_payload() :289-306:
        [0..2] kind u16 (GameSpeed=3, Pause=4)  [4..8] flags u32
        [32..36] value_i32 (the emitted datum)  [48..52] event_hour_raw i32
CANONICAL STREAM GATE (fail-closed precondition -- never trust a rejected row)
------------------------------------------------------------------------------
Before any value is compared, the whole emission stream is validated by a faithful
Python port of the CANONICAL gate -- gate_compact_stream / gate_payload /
validate_compact_record_semantics (session...live/src/lib.rs:657-897 & 226-585 and
compact_contract/src/record_semantics.rs). This reproduces, byte-for-byte, every
check: H4CR magic/version/header-len, payload-length bound, strictly-+1 sequence
continuity, canonical record flags, the exact DOMAIN_PERF base/PDU1 and FRAME_END
domain/length grammar, exactly-one-DATA-and-one-PERF-per-frame cardinality,
consecutive (zero-skipped) frame hours, the payload<->header game-hour agreement,
the S57M magic/version/domain/row-length/declared-length checks, known
status/source/event/player flag masks, root-evidence presence, row-count and
string-blob bounds, and the fatal-status projection. The harness consumes a row
ONLY from a stream this gate accepts -- exactly the way the transcript side is
only trusted after verify_speed_root_evidence_v2.py passes. (No CLI gates a single
domain-57 .bin stream today: the only stream-gate binary,
compact_replay_verification_gates, validates a whole recorder --run-dir, not a
bytes stream, so the checks are reproduced here rather than shelled out.)

Activation: `DOMAIN57_GAME_SPEED_READER_DEFAULT_ENABLED = false`
(build_profile.rs:98). The emission exists ONLY under the collector's EXPLICIT
OPT-IN (Domain57GameSpeedMode != Disabled; collector/src/lib.rs:3044-3193,26925).
This harness does NOT change that flag; it consumes an opt-in capture's records.
Every emission frame that carries a GameSpeed row must therefore also carry
SOURCE_FLAG_SESSION_ROOT_VALIDATED and must NOT be a proof fixture -- a genuine
opt-in production emission, not a hand-built stream.

CROSS-CAPTURE ALIGNMENT (operator-attested; see the OWED note below)
--------------------------------------------------------------------
The probe transcript is timestamped in Windows FILETIME; the emission stream is
keyed by in-game `game_hour_raw`. There is NO shared tick between the two
independent captures in-tree, so the operator (who drives the SAME 1->2->3->4->5
speed cycle for both, per LIVE_ACCEPTANCE_RUNBOOK.md Sequence 1 launch 1.1)
attests the correspondence with a small alignment manifest:
`battle-plan.d57_emission_alignment.v1`. The manifest CANNOT fake the raw word
-- that is re-derived from the hash-pinned sealed transcript. The manifest only
says which emission record (by sequence + game_hour_raw) was captured in which
probe speed state; the harness enforces byte-exact equality, full 1..5 coverage
(so raw 0..4 is proven, catching a 1-based datum), and range on every value.

  OWED (precise): a HARD, machine-checkable cross-capture binding does not exist
  in-tree -- the probe and the collector emission share no clock/tick/nonce. The
  alignment is therefore operator-attested. A follow-up that lands a shared
  wall-clock/tick stamp on BOTH the probe reads and the emission frames would let
  this manifest be replaced by an automatic join. The EMISSION FORMAT itself is
  fully defined (above) and is NOT owed.

OWNER CLI (Sequence 1 launch 1.1 D57 gate; run AFTER the probe transcript is
sealed+verified per the runbook)
-----------------------------------------------------------------------------
  python extractor/native/scripts/verify_d57_emission_matches_raw.py \
    --transcript                   <run>/transcript.json \
    --nonce-ledger-root            <run>/nonce_ledger \
    --expected-transcript-sha256   <seal-time transcript_sha256 field> \
    --expected-session-id          d57_speed_<date> \
    --expected-evidence-nonce      <nonce> \
    --expected-exe-sha256          870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad \
    --expected-probe-tool-sha256   <sha256 of collect_speed_root_evidence_v2.py> \
    --expected-static-disasm-sha256 789cd1d7d6f743d5699c1bb286100076b935a4cdcec41054886cfd4a3aaa6ebc \
    --emission-stream              <capture>/domain57_stream.bin \
    --emission-stream-sha256       <sha256 of the emission stream> \
    --alignment-manifest           <run>/d57_emission_alignment.json \
    --alignment-manifest-sha256    <sha256 of the alignment manifest> \
    --out                          <run>/d57_emission_match_gate.v1.json

The first eight flags are IDENTICAL to verify_speed_root_evidence_v2.py's inputs:
this harness first re-runs that verifier end-to-end (so it only ever compares
against a fully sealed+verified raw-words transcript), then performs the emission
comparison. PASS = it writes a `battle-plan.d57_emission_match_gate.v1` report
with `"verdict": "passed"`; any anomaly prints `FAIL: <reason>` and exits 1.

Self-test: `python verify_d57_emission_matches_raw.py --self-test`.
"""

from __future__ import annotations

import argparse
import copy
import json
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

# Reuse (do NOT modify) the sibling raw-words verifier: its verify() is the
# precondition, and its deny-extra / duplicate-key / pin discipline is shared.
_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))
import verify_speed_root_evidence_v2 as probe_verifier  # noqa: E402

GateError = probe_verifier.GateError
load_object = probe_verifier.load_object
exact_object = probe_verifier.exact_object
sha256_file = probe_verifier.sha256_file
file_identity = probe_verifier.file_identity
transcript_digest = probe_verifier.transcript_digest
HEX64 = probe_verifier.HEX64

GATE_SCHEMA = "battle-plan.d57_emission_match_gate.v1"
ALIGNMENT_SCHEMA = "battle-plan.d57_emission_alignment.v1"

# Wire constants (byte-exact; see the module docstring for file:line provenance).
# The stream validator below is a faithful port of the CANONICAL gate --
# gate_compact_stream / gate_payload / validate_compact_record_semantics in
# lanes/session_multiplayer_metadata_live/src/lib.rs:657-897 & 226-585 and
# compact_contract/src/record_semantics.rs -- so the harness never trusts a row
# from a stream the canonical gate would reject.
DOMAIN_ID = 57
DOMAIN_FRAME = 1000
RECORD_MAGIC = b"H4CR"
RECORD_VERSION = 1
RECORD_HEADER_LEN = 32
RECORD_TYPE_DOMAIN_PERF = 1
RECORD_TYPE_FRAME_END = 2
RECORD_TYPE_DOMAIN_DATA = 3
COMPACT_MAX_PAYLOAD_LEN = 1 << 28
COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER = 1 << 30
COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 = 1 << 31
COMPACT_DOMAIN_PERF_BASE_LEN = 40
COMPACT_FRAME_END_PAYLOAD_LEN = 32
COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC = b"PDU1"
COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION = 1
PAYLOAD_MAGIC = b"S57M"
PAYLOAD_VERSION = 1
PAYLOAD_HEADER_LEN = 64
PLAYER_COUNTRY_ROW_LEN = 40
SESSION_EVENT_ROW_LEN = 64
KIND_GAME_SPEED = 3
KIND_PAUSE = 4
KIND_CHAT, KIND_SYSTEM_MESSAGE, KIND_ALERT, KIND_NOTIFICATION = 5, 6, 7, 8
KIND_MIN, KIND_MAX = 1, 8
TEXT_BEARING_KINDS = {KIND_CHAT, KIND_SYSTEM_MESSAGE, KIND_ALERT, KIND_NOTIFICATION}
MAX_GAME_SPEED = 4  # DOMAIN57_MAX_GAME_SPEED: raw canonical range 0..=4
MAX_TAG_DB_ID = 4096
MAX_STRING_BLOB_BYTES = 256 * 1024
MAX_PLAYER_ROWS = 4096
MAX_EVENT_ROWS = 100_000
# Flag / status masks (numeric values of the Rust *_KNOWN_MASK / FAILURE_MASK).
SOURCE_FLAG_DLL_NATIVE = 1 << 0
SOURCE_FLAG_SESSION_ROOT_VALIDATED = 1 << 1
SOURCE_FLAG_PROOF_FIXTURE = 1 << 31
SOURCE_FLAG_KNOWN_MASK = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 31)
PLAYER_FLAG_KNOWN_MASK = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3)
EVENT_FLAG_KNOWN_MASK = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3)
STATUS_WORKER_PANIC = 1 << 6
STATUS_BRIDGE_UNAVAILABLE = 1 << 7
STATUS_SOURCE_UNAVAILABLE = 1 << 8
STATUS_PARTIAL_DATA = 1 << 9
# STATUS_KNOWN_MASK = bits 0..13 (compact_contract status_flags_base.rs 0..13).
STATUS_KNOWN_MASK = (1 << 14) - 1
# COMPACT_STATUS_FAILURE_MASK = bits {0,1,2,3,6,7,8,9,12}. Domain 57 has no
# lane-local status mask and no expected-partial tolerance, so the failure
# projection is simply status & FAILURE_MASK.
COMPACT_STATUS_FAILURE_MASK = (
    (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 9) | (1 << 12)
)
MAX_STREAM_BYTES = 512 * 1024 * 1024

ALIGNMENT_ROOT_KEYS = {
    "schema",
    "emission_stream_path",
    "emission_stream_sha256",
    "transcript_sha256",
    "session_id",
    "evidence_nonce_hex",
    "alignments",
}
ALIGNMENT_ENTRY_KEYS = {
    "emission_record_sequence",
    "emission_game_hour_raw",
    "probe_speed_sample_sequence",
}


def _u16(b: bytes, o: int) -> int:
    return int.from_bytes(b[o : o + 2], "little")


def _u32(b: bytes, o: int) -> int:
    return int.from_bytes(b[o : o + 4], "little")


def _u64(b: bytes, o: int) -> int:
    return int.from_bytes(b[o : o + 8], "little")


def _i32(b: bytes, o: int) -> int:
    return int.from_bytes(b[o : o + 4], "little", signed=True)


def _strict_int(value, label: str, minimum: int | None = None, maximum: int | None = None) -> int:
    # bool is an int subclass in Python -- reject it explicitly, mirroring the
    # sibling verifier's strict_int discipline.
    if type(value) is not int:
        raise GateError(f"{label} is not a canonical integer")
    if minimum is not None and value < minimum:
        raise GateError(f"{label} is below {minimum}")
    if maximum is not None and value > maximum:
        raise GateError(f"{label} is above {maximum}")
    return value


def _validate_string_ref(strings: bytes, off: int, length: int, field: str, row: int, errors: list):
    end = off + length
    if end > len(strings):
        errors.append(f"row {row}: {field} string ref out of bounds")
        return None
    slice_ = strings[off:end]
    if 0 in slice_:
        errors.append(f"row {row}: {field} contains NUL")
        return None
    try:
        return slice_.decode("utf-8")
    except UnicodeDecodeError:
        errors.append(f"row {row}: {field} is not utf-8")
        return None


def _valid_country_tag(value: str) -> bool:
    return len(value) == 3 and all(c.isupper() and c.isascii() or c == "-" for c in value)


def gate_payload(payload: bytes) -> dict:
    """Faithful port of gate_payload (session...live/src/lib.rs:311-585). Returns
    {ok, errors, game_hour_raw, status_flags, source_flags, game_speed_values}."""
    errors: list[str] = []
    result = {
        "ok": False,
        "errors": errors,
        "game_hour_raw": 0,
        "status_flags": 0,
        "source_flags": 0,
        "game_speed_values": [],
    }
    if len(payload) < PAYLOAD_HEADER_LEN:
        errors.append("payload shorter than header")
        return result
    if payload[0:4] != PAYLOAD_MAGIC:
        errors.append("invalid payload magic")
    version = _u16(payload, 4)
    header_len = _u16(payload, 6)
    domain_id = _u16(payload, 8)
    player_row_len = _u16(payload, 10)
    event_row_len = _u16(payload, 12)
    if version != PAYLOAD_VERSION:
        errors.append(f"payload version {version} != {PAYLOAD_VERSION}")
    if header_len != PAYLOAD_HEADER_LEN:
        errors.append(f"payload header_len {header_len} != {PAYLOAD_HEADER_LEN}")
    if domain_id != DOMAIN_ID:
        errors.append(f"payload domain_id {domain_id} != {DOMAIN_ID}")
    if player_row_len != PLAYER_COUNTRY_ROW_LEN:
        errors.append("unexpected player-country row length")
    if event_row_len != SESSION_EVENT_ROW_LEN:
        errors.append("unexpected session event row length")
    game_hour_raw = _i32(payload, 16)
    status_flags = _u32(payload, 20)
    source_flags = _u32(payload, 24)
    player_rows = _u32(payload, 28)
    event_rows = _u32(payload, 32)
    string_bytes = _u32(payload, 36)
    schema_flags = _u32(payload, 40)
    root_evidence_mask = _u32(payload, 44)
    decl_min_hour = _i32(payload, 48)
    decl_max_hour = _i32(payload, 52)
    result.update(
        {"game_hour_raw": game_hour_raw, "status_flags": status_flags, "source_flags": source_flags}
    )
    if status_flags & ~STATUS_KNOWN_MASK != 0:
        errors.append(f"unknown status flags 0x{status_flags & ~STATUS_KNOWN_MASK:X}")
    if source_flags & ~SOURCE_FLAG_KNOWN_MASK != 0:
        errors.append(f"unknown source flags 0x{source_flags & ~SOURCE_FLAG_KNOWN_MASK:X}")
    if player_rows > MAX_PLAYER_ROWS:
        errors.append("too many player-country rows")
    if event_rows > MAX_EVENT_ROWS:
        errors.append("too many session event rows")
    if string_bytes > MAX_STRING_BLOB_BYTES:
        errors.append("string blob too large")
    if schema_flags != 0:
        errors.append(f"unknown schema_flags 0x{schema_flags:X}")
    if (
        root_evidence_mask == 0
        and (source_flags & SOURCE_FLAG_PROOF_FIXTURE) == 0
        and (player_rows > 0 or event_rows > 0)
    ):
        errors.append("non-empty production payload lacks root evidence")
    if (
        header_len != PAYLOAD_HEADER_LEN
        or player_row_len != PLAYER_COUNTRY_ROW_LEN
        or event_row_len != SESSION_EVENT_ROW_LEN
        or player_rows > MAX_PLAYER_ROWS
        or event_rows > MAX_EVENT_ROWS
        or string_bytes > MAX_STRING_BLOB_BYTES
    ):
        return result
    strings_start = PAYLOAD_HEADER_LEN + player_rows * player_row_len + event_rows * event_row_len
    expected_len = strings_start + string_bytes
    if expected_len != len(payload):
        errors.append(f"payload length {len(payload)} != expected {expected_len}")
        return result
    strings = payload[strings_start:]
    off = PAYLOAD_HEADER_LEN
    for row_index in range(player_rows):
        row = payload[off : off + player_row_len]
        name = _validate_string_ref(strings, _u32(row, 8), _u16(row, 12), "player_name", row_index, errors)
        tag = _validate_string_ref(strings, _u32(row, 16), _u16(row, 14), "country_tag", row_index, errors)
        if not (tag is not None and _valid_country_tag(tag)):
            errors.append(f"player row {row_index}: invalid country tag")
        tag_db_id = _u32(row, 20)
        if tag_db_id == 0 or tag_db_id > MAX_TAG_DB_ID:
            errors.append(f"player row {row_index}: invalid tag_db_id {tag_db_id}")
        flags = _u32(row, 28)
        if flags & ~PLAYER_FLAG_KNOWN_MASK != 0:
            errors.append(f"player row {row_index}: unknown flags 0x{flags & ~PLAYER_FLAG_KNOWN_MASK:X}")
        del name
        off += player_row_len
    observed_min = None
    observed_max = None
    game_speed_values = []
    for row_index in range(event_rows):
        row = payload[off : off + event_row_len]
        raw_kind = _u16(row, 0)
        flags = _u32(row, 4)
        value_i32 = _i32(row, 32)
        event_hour_raw = _i32(row, 48)
        observed_min = event_hour_raw if observed_min is None else min(observed_min, event_hour_raw)
        observed_max = event_hour_raw if observed_max is None else max(observed_max, event_hour_raw)
        actor = _validate_string_ref(strings, _u32(row, 16), _u16(row, 20), "actor_name", row_index, errors)
        text = _validate_string_ref(strings, _u32(row, 24), _u16(row, 22), "event_text", row_index, errors)
        del actor
        known_kind = KIND_MIN <= raw_kind <= KIND_MAX
        if not known_kind:
            errors.append(f"event row {row_index}: unknown kind {raw_kind}")
        if flags & ~EVENT_FLAG_KNOWN_MASK != 0:
            errors.append(f"event row {row_index}: unknown flags 0x{flags & ~EVENT_FLAG_KNOWN_MASK:X}")
        tag_db_id = _u32(row, 28)
        if tag_db_id > MAX_TAG_DB_ID:
            errors.append(f"event row {row_index}: invalid tag_db_id {tag_db_id}")
        if known_kind:
            if raw_kind == KIND_GAME_SPEED:
                game_speed_values.append(value_i32)
                if not (0 <= value_i32 <= MAX_GAME_SPEED):
                    errors.append(f"event row {row_index}: game speed {value_i32} outside 0..4")
            elif raw_kind == KIND_PAUSE:
                if value_i32 not in (0, 1):
                    errors.append(f"event row {row_index}: pause value {value_i32} is not 0/1")
            elif raw_kind in TEXT_BEARING_KINDS and not (text or ""):
                errors.append(f"event row {row_index}: kind {raw_kind} lacks text")
        off += event_row_len
    exp_min = observed_min if observed_min is not None else game_hour_raw
    exp_max = observed_max if observed_max is not None else game_hour_raw
    if decl_min_hour != exp_min:
        errors.append(f"declared min_event_hour_raw {decl_min_hour} != decoded {exp_min}")
    if decl_max_hour != exp_max:
        errors.append(f"declared max_event_hour_raw {decl_max_hour} != decoded {exp_max}")
    result["game_speed_values"] = game_speed_values
    result["ok"] = not errors
    return result


def gate_and_index_stream(raw: bytes) -> dict:
    """Faithful port of gate_compact_stream (session...live/src/lib.rs:657-853).
    Returns {records, total_game_speed_rows}; raises GateError with every canonical
    reason if the stream is not exactly what the production encoder emits."""
    errors: list[str] = []
    records: dict[int, dict] = {}
    total_game_speed_rows = 0
    domain_data_records = 0
    saw_frame_end = False
    zero_skipped = True
    skipped_hours = 0
    status_or = 0
    failure_status_or = 0
    partial_seen = source_unavailable_seen = worker_panic_seen = bridge_unavailable_seen = False
    previous_sequence = None
    previous_frame_hour = None
    open_frame_hour = None
    last_record_type = None
    frame_data_records = 0
    frame_perf_records = 0

    def observe_status(status_flags: int):
        nonlocal status_or, failure_status_or
        nonlocal partial_seen, source_unavailable_seen, worker_panic_seen, bridge_unavailable_seen
        status_or |= status_flags
        failure_status_or |= status_flags & COMPACT_STATUS_FAILURE_MASK
        partial_seen |= bool(status_flags & STATUS_PARTIAL_DATA)
        source_unavailable_seen |= bool(status_flags & STATUS_SOURCE_UNAVAILABLE)
        worker_panic_seen |= bool(status_flags & STATUS_WORKER_PANIC)
        bridge_unavailable_seen |= bool(status_flags & STATUS_BRIDGE_UNAVAILABLE)

    def observe_open_frame(game_hour_raw: int, offset: int):
        nonlocal open_frame_hour
        if open_frame_hour is None:
            open_frame_hour = game_hour_raw
        elif open_frame_hour != game_hour_raw:
            errors.append(
                f"offset {offset}: record hour {game_hour_raw} encountered before FRAME_END "
                f"closed open frame hour {open_frame_hour}"
            )

    offset = 0
    n = len(raw)
    while offset < n:
        # --- H4CR framing (compact_contract/src/record_header.rs:60-89) ---
        if n - offset < RECORD_HEADER_LEN:
            errors.append(f"offset {offset}: truncated compact record header")
            break
        header = raw[offset : offset + RECORD_HEADER_LEN]
        if header[0:4] != RECORD_MAGIC:
            errors.append(f"offset {offset}: invalid compact record magic")
            break
        if _u16(header, 4) != RECORD_VERSION or _u16(header, 6) != RECORD_HEADER_LEN:
            errors.append(f"offset {offset}: unsupported compact header")
            break
        payload_len = _u32(header, 24)
        if payload_len > COMPACT_MAX_PAYLOAD_LEN:
            errors.append(f"offset {offset}: compact record payload too large")
            break
        sequence = _u64(header, 8)
        game_hour_raw = _i32(header, 16)
        domain_id = _u16(header, 20)
        record_type = _u16(header, 22)
        record_flags = _u32(header, 28)
        # --- sequence continuity (start at 1, strictly +1, no wrap) ---
        if previous_sequence is not None:
            expected = previous_sequence + 1
            if sequence != expected:
                errors.append(
                    f"offset {offset}: compact record sequence discontinuity: "
                    f"expected {expected}, got {sequence}"
                )
        elif sequence != 1:
            errors.append(f"offset {offset}: first compact record sequence is {sequence}, expected 1")
        previous_sequence = sequence
        record_len = RECORD_HEADER_LEN + payload_len
        if record_len > n - offset:
            errors.append(f"offset {offset}: truncated compact record payload")
            break
        payload = raw[offset + RECORD_HEADER_LEN : offset + record_len]
        # --- canonical record semantics (record flags, PERF/FRAME_END grammar) ---
        semantic_error = _validate_record_semantics(domain_id, record_type, record_flags, payload)
        if semantic_error is not None:
            errors.append(f"offset {offset}: {semantic_error}")
            offset += record_len
            continue
        if record_type == RECORD_TYPE_DOMAIN_DATA and domain_id == DOMAIN_ID:
            observe_open_frame(game_hour_raw, offset)
            domain_data_records += 1
            frame_data_records += 1
            pg = gate_payload(payload)
            for err in pg["errors"]:
                errors.append(f"offset {offset}: {err}")
            if pg["game_hour_raw"] != game_hour_raw:
                errors.append(
                    f"offset {offset}: domain57 payload hour {pg['game_hour_raw']} does not "
                    f"match compact record hour {game_hour_raw}"
                )
            observe_status(pg["status_flags"])
            records[sequence] = {
                "sequence": sequence,
                "game_hour_raw": game_hour_raw,
                "game_speed_values": pg["game_speed_values"],
                "source_flags": pg["source_flags"],
            }
            total_game_speed_rows += len(pg["game_speed_values"])
        elif record_type == RECORD_TYPE_DOMAIN_PERF and domain_id == DOMAIN_ID:
            observe_open_frame(game_hour_raw, offset)
            frame_perf_records += 1
            observe_status(_u32(payload, 32))
        elif record_type == RECORD_TYPE_FRAME_END:
            saw_frame_end = True
            observe_status(_u32(payload, 24))
            if frame_data_records != 1:
                errors.append(
                    f"offset {offset}: closed frame hour {game_hour_raw} has {frame_data_records} "
                    f"domain57 DATA records, expected exactly 1"
                )
            if frame_perf_records != 1:
                errors.append(
                    f"offset {offset}: closed frame hour {game_hour_raw} has {frame_perf_records} "
                    f"domain57 PERF records, expected exactly 1"
                )
            if open_frame_hour is None:
                errors.append(f"offset {offset}: FRAME_END hour {game_hour_raw} has no open frame")
            elif open_frame_hour == game_hour_raw:
                open_frame_hour = None
            else:
                errors.append(
                    f"offset {offset}: FRAME_END hour {game_hour_raw} does not close open frame "
                    f"hour {open_frame_hour}"
                )
            if previous_frame_hour is not None:
                expected = previous_frame_hour + 1
                if game_hour_raw != expected:
                    if game_hour_raw > expected:
                        skipped_hours += game_hour_raw - expected
                    errors.append(
                        f"offset {offset}: closed frame hour {game_hour_raw} is not consecutive "
                        f"after {previous_frame_hour} (expected {expected})"
                    )
            previous_frame_hour = game_hour_raw
            frame_data_records = 0
            frame_perf_records = 0
        elif record_type in (RECORD_TYPE_DOMAIN_DATA, RECORD_TYPE_DOMAIN_PERF):
            observe_open_frame(game_hour_raw, offset)
        else:
            errors.append(f"offset {offset}: unknown record type {record_type}")
        last_record_type = record_type
        offset += record_len

    if offset != n:
        errors.append("compact stream ended before all bytes were consumed")
    if domain_data_records == 0:
        errors.append("no domain57 data records")
    if not saw_frame_end:
        errors.append("no FRAME_END record")
    if open_frame_hour is not None:
        errors.append(f"compact stream ended with open frame hour {open_frame_hour}")
    if last_record_type != RECORD_TYPE_FRAME_END:
        errors.append("compact stream does not end at a clean FRAME_END boundary")
    zero_skipped = skipped_hours == 0
    if failure_status_or != 0:
        errors.append(f"domain57 stream contains fatal status flags 0x{failure_status_or:X}")
    fatal_status_seen = (
        partial_seen or source_unavailable_seen or worker_panic_seen or bridge_unavailable_seen
    )
    if not zero_skipped:
        errors.append(f"domain57 stream skipped {skipped_hours} game hour(s)")
    if fatal_status_seen:
        errors.append("domain57 stream carries partial/source-unavailable/panic/bridge status")
    if errors:
        raise GateError("canonical stream gate rejected the emission: " + "; ".join(errors[:6]))
    return {"records": records, "total_game_speed_rows": total_game_speed_rows}


def _validate_record_semantics(domain_id: int, record_type: int, record_flags: int, payload: bytes):
    """Port of validate_compact_record_semantics (record_semantics.rs). Returns an
    error string or None."""
    allowed = 0
    if record_type == RECORD_TYPE_DOMAIN_PERF:
        allowed |= COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER
    if record_type == RECORD_TYPE_DOMAIN_DATA and domain_id == 45:
        allowed |= COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
    rejected = record_flags & ~allowed
    if rejected != 0:
        return (
            f"record flags 0x{record_flags:08x} contain rejected bits 0x{rejected:08x} "
            f"for domain {domain_id} record type {record_type}"
        )
    if record_type == RECORD_TYPE_DOMAIN_PERF:
        return _validate_domain_perf(record_flags, payload)
    if record_type == RECORD_TYPE_FRAME_END:
        if domain_id != DOMAIN_FRAME:
            return f"FRAME_END domain {domain_id} is not {DOMAIN_FRAME}"
        if len(payload) != COMPACT_FRAME_END_PAYLOAD_LEN:
            return f"FRAME_END payload length {len(payload)} is not {COMPACT_FRAME_END_PAYLOAD_LEN}"
        return None
    if record_type == RECORD_TYPE_DOMAIN_DATA:
        return None
    return f"unknown compact record type {record_type}"


def _validate_domain_perf(record_flags: int, payload: bytes):
    if len(payload) < COMPACT_DOMAIN_PERF_BASE_LEN:
        return f"DOMAIN_PERF payload length {len(payload)} is shorter than {COMPACT_DOMAIN_PERF_BASE_LEN}"
    if _u32(payload, 36) != 0:
        return "DOMAIN_PERF reserved base word is nonzero"
    has_trailer = bool(record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER)
    if not has_trailer:
        if len(payload) == COMPACT_DOMAIN_PERF_BASE_LEN:
            return None
        return f"DOMAIN_PERF payload has {len(payload) - COMPACT_DOMAIN_PERF_BASE_LEN} unannounced trailer byte(s)"
    trailer_start = COMPACT_DOMAIN_PERF_BASE_LEN
    if len(payload) < trailer_start + 8:
        return "DOMAIN_PERF diagnostic trailer header is truncated"
    if payload[trailer_start : trailer_start + 4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC:
        return "DOMAIN_PERF diagnostic trailer magic is invalid"
    if _u16(payload, trailer_start + 4) != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION:
        return "DOMAIN_PERF diagnostic trailer version is not 1"
    unit_count = _u16(payload, trailer_start + 6)
    cursor = trailer_start + 8
    for unit_index in range(unit_count):
        if len(payload) - cursor < 12:
            return f"DOMAIN_PERF diagnostic unit {unit_index} header is truncated"
        name_len = _u16(payload, cursor)
        if _u16(payload, cursor + 2) != 0:
            return f"DOMAIN_PERF diagnostic unit {unit_index} reserved field is nonzero"
        cursor += 12
        if len(payload) - cursor < name_len:
            return f"DOMAIN_PERF diagnostic unit {unit_index} name is truncated"
        name_bytes = payload[cursor : cursor + name_len]
        try:
            name_bytes.decode("utf-8")
        except UnicodeDecodeError:
            return f"DOMAIN_PERF diagnostic unit {unit_index} name is not UTF-8"
        # Per-BYTE ASCII grammar, mirroring record_semantics.rs:151-161 exactly:
        # reject if empty OR any byte is not ascii-lowercase / ascii-digit / '_'.
        # (Unicode str methods like "é".islower() would wrongly accept non-ASCII.)
        if not name_bytes or not all(
            0x61 <= b <= 0x7A or 0x30 <= b <= 0x39 or b == 0x5F for b in name_bytes
        ):
            return f"DOMAIN_PERF diagnostic unit {unit_index} name {name_bytes!r} is invalid"
        cursor += name_len
    if cursor != len(payload):
        return f"DOMAIN_PERF diagnostic trailer has {len(payload) - cursor} trailing byte(s)"
    return None


def compare(args, *, allow_fixture_exe: bool = False) -> dict:
    # 1. Precondition: fully seal+verify the raw-words probe transcript. This is
    #    the sibling verifier run end-to-end; it also pins exe/probe/static and
    #    the canonical transcript digest. Only a passing transcript is compared.
    transcript_path = args.transcript.resolve()
    with tempfile.TemporaryDirectory(prefix="d57_crit4_probe_") as probe_temp:
        probe_args = SimpleNamespace(
            transcript=transcript_path,
            nonce_ledger_root=args.nonce_ledger_root,
            expected_transcript_sha256=args.expected_transcript_sha256,
            expected_session_id=args.expected_session_id,
            expected_evidence_nonce=args.expected_evidence_nonce,
            expected_exe_sha256=args.expected_exe_sha256,
            expected_probe_tool_sha256=args.expected_probe_tool_sha256,
            expected_static_disasm_sha256=args.expected_static_disasm_sha256,
            out=Path(probe_temp) / "probe_gate.json",
            allow_fixture_exe=allow_fixture_exe,
        )
        probe_report = probe_verifier.verify(probe_args)

    # 2. Re-derive the ground-truth raw word for each UI speed state (1..5) from
    #    the sealed transcript. verify() already proved speed_samples[i] has
    #    expected_raw_speed == i and every read.speed_raw == i; we read the value
    #    back so the comparison is evidence-derived, not hardcoded.
    transcript = load_object(transcript_path, "speed-root transcript")
    ground_truth_raw: dict[int, int] = {}
    for sample in transcript["speed_samples"]:
        seq = sample["sequence"]
        raw = sample["expected_raw_speed"]
        observed = {row["speed_raw"] for row in sample["reads"]}
        if observed != {raw}:
            raise GateError(f"transcript speed sample {seq}: raw words disagree with expected")
        ground_truth_raw[seq] = raw
    if set(ground_truth_raw) != {1, 2, 3, 4, 5}:
        raise GateError("transcript does not carry the five ordered speed samples")

    # 3. Load + pin the operator alignment manifest (deny-extra, duplicate-key
    #    rejection via the shared load_object/exact_object).
    manifest_path = args.alignment_manifest.resolve()
    if sha256_file(manifest_path) != args.expected_alignment_manifest_sha256 or not HEX64.fullmatch(
        str(args.expected_alignment_manifest_sha256)
    ):
        raise GateError("alignment manifest does not match the caller's durable hash pin")
    manifest = exact_object(
        load_object(manifest_path, "alignment manifest"), ALIGNMENT_ROOT_KEYS, "alignment manifest"
    )
    if manifest["schema"] != ALIGNMENT_SCHEMA:
        raise GateError("alignment manifest schema mismatch")
    if manifest["transcript_sha256"] != transcript["transcript_sha256"] or manifest[
        "transcript_sha256"
    ] != args.expected_transcript_sha256:
        raise GateError("alignment manifest is not bound to the verified probe transcript")
    if (
        manifest["session_id"] != args.expected_session_id
        or manifest["evidence_nonce_hex"] != args.expected_evidence_nonce
    ):
        raise GateError("alignment manifest session/evidence pins do not match the transcript")

    # 4. Pin the emission stream bytes: caller CLI pin, manifest pin, and the file
    #    itself must all agree.
    stream_path = args.emission_stream.resolve()
    if manifest["emission_stream_sha256"] != args.expected_emission_stream_sha256 or not HEX64.fullmatch(
        str(args.expected_emission_stream_sha256)
    ):
        raise GateError("emission stream pin: manifest vs caller hash disagree / not SHA-256")
    if Path(str(manifest["emission_stream_path"])).resolve() != stream_path:
        raise GateError("alignment manifest emission_stream_path does not match the caller stream")
    stream_sha = sha256_file(stream_path)
    if stream_sha != args.expected_emission_stream_sha256:
        raise GateError("emission stream file hash mismatch")
    raw_stream = stream_path.read_bytes()
    if len(raw_stream) > MAX_STREAM_BYTES:
        raise GateError("emission stream exceeds the 512 MiB bound")

    # 5. Canonical stream validation as a fail-closed PRECONDITION -- a faithful
    #    port of gate_compact_stream/gate_payload/validate_compact_record_semantics.
    #    The harness never trusts a row from a stream the canonical gate rejects
    #    (record flags, sequence continuity, DATA/PERF/FRAME_END structure,
    #    payload<->header hour agreement, known source/status/event flags, root
    #    evidence, row-count/length bounds, magic/version/domain).
    decoded = gate_and_index_stream(raw_stream)
    records = decoded["records"]
    if decoded["total_game_speed_rows"] == 0:
        raise GateError("no D57 emission to compare (opt-in capture required)")
    # Opt-in provenance: any frame carrying a GameSpeed row must be root-validated
    # and NOT a proof fixture (a genuine explicit-opt-in production emission).
    for seq, record in records.items():
        if record["game_speed_values"]:
            if not record["source_flags"] & SOURCE_FLAG_SESSION_ROOT_VALIDATED:
                raise GateError(
                    f"record {seq}: GameSpeed frame lacks SOURCE_FLAG_SESSION_ROOT_VALIDATED"
                )
            if record["source_flags"] & SOURCE_FLAG_PROOF_FIXTURE:
                raise GateError(f"record {seq}: GameSpeed frame is a proof fixture, not opt-in capture")

    # 6. Enforce the per-alignment value law: emitted value_i32 == the probe raw
    #    word for that speed state, byte-exact, in range, one anchor row per frame.
    alignments = manifest["alignments"]
    if not isinstance(alignments, list) or not alignments:
        raise GateError("alignment manifest has no alignments to compare")
    seen_record_seq: set[int] = set()
    referenced_samples: set[int] = set()
    compared = []
    for index, entry in enumerate(alignments):
        entry = exact_object(entry, ALIGNMENT_ENTRY_KEYS, f"alignment {index}")
        record_seq = _strict_int(entry["emission_record_sequence"], f"alignment {index} record seq", 1)
        hour = _strict_int(entry["emission_game_hour_raw"], f"alignment {index} game_hour_raw")
        sample_seq = _strict_int(
            entry["probe_speed_sample_sequence"], f"alignment {index} sample seq", 1, 5
        )
        if record_seq in seen_record_seq:
            raise GateError(f"alignment {index}: duplicate emission_record_sequence {record_seq}")
        seen_record_seq.add(record_seq)
        if sample_seq in referenced_samples:
            raise GateError(f"alignment {index}: probe speed sample {sample_seq} referenced twice")
        referenced_samples.add(sample_seq)
        record = records.get(record_seq)
        if record is None:
            raise GateError(f"alignment {index}: no domain-57 record with sequence {record_seq}")
        if record["game_hour_raw"] != hour:
            raise GateError(
                f"alignment {index}: record {record_seq} game_hour_raw "
                f"{record['game_hour_raw']} != attested {hour}"
            )
        values = record["game_speed_values"]
        if len(values) != 1:
            raise GateError(
                f"alignment {index}: record {record_seq} has {len(values)} GameSpeed rows (expected 1 anchor)"
            )
        emitted = values[0]
        expected = ground_truth_raw[sample_seq]
        if not (0 <= emitted <= MAX_GAME_SPEED):
            raise GateError(f"alignment {index}: emitted speed {emitted} out of range")
        if emitted != expected:
            raise GateError(
                f"alignment {index}: emitted speed {emitted} != probe raw word {expected} "
                f"for UI speed {sample_seq} (coercion / mismatch)"
            )
        compared.append(
            {
                "emission_record_sequence": record_seq,
                "emission_game_hour_raw": hour,
                "probe_speed_sample_sequence": sample_seq,
                "emitted_value_i32": emitted,
                "probe_raw_word": expected,
            }
        )

    # 7. Coverage: every UI speed state 1..5 must be compared, so the emission is
    #    proven to carry the full raw domain 0..4 (one per UI speed). This is the
    #    clincher against a displayed 1..5 datum -- UI speed 1 must emit 0, not 1.
    if referenced_samples != {1, 2, 3, 4, 5}:
        raise GateError(
            f"alignment does not cover all five UI speed states (covered: {sorted(referenced_samples)})"
        )

    verifier_path = Path(__file__).resolve()
    return {
        "schema": GATE_SCHEMA,
        "verdict": "passed",
        "transcript_path": str(transcript_path),
        "transcript_sha256": transcript["transcript_sha256"],
        "probe_gate_verdict": probe_report["verdict"],
        "session_id": args.expected_session_id,
        "evidence_nonce_hex": args.expected_evidence_nonce,
        "exe_sha256": args.expected_exe_sha256.lower(),
        "emission_stream_path": str(stream_path),
        "emission_stream_sha256": stream_sha,
        "alignment_manifest_path": str(manifest_path),
        "alignment_manifest_sha256": args.expected_alignment_manifest_sha256,
        "domain_id": DOMAIN_ID,
        "max_game_speed": MAX_GAME_SPEED,
        "domain57_data_records": len(records),
        "total_game_speed_rows": decoded["total_game_speed_rows"],
        "ground_truth_raw_by_ui_speed": {str(k): ground_truth_raw[k] for k in sorted(ground_truth_raw)},
        "comparisons": compared,
        "verifier_path": str(verifier_path),
        "verifier_sha256": sha256_file(verifier_path),
    }


def _write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


# --------------------------------------------------------------------------- #
# Self-test: a valid PASS fixture + an exhaustive mutation battery (one failing
# mutation per check). The emission encoder here is fixture-only and byte-mirrors
# the Rust encode_payload/append_compact_record; the harness decoder above is
# independent of it.
# --------------------------------------------------------------------------- #


def _enc_event_row(kind: int, value_i32: int, event_hour: int, *, flags: int = 1, tag_db_id: int = 0,
                   text_off: int = 0, text_len: int = 0) -> bytes:
    row = bytearray(SESSION_EVENT_ROW_LEN)
    row[0:2] = (kind & 0xFFFF).to_bytes(2, "little")
    row[4:8] = int(flags).to_bytes(4, "little")  # default EVENT_FLAG_DIRECT_CAPTURE
    row[22:24] = int(text_len).to_bytes(2, "little")
    row[24:28] = int(text_off).to_bytes(4, "little")
    row[28:32] = int(tag_db_id).to_bytes(4, "little")
    row[32:36] = int(value_i32).to_bytes(4, "little", signed=True)
    row[48:52] = int(event_hour).to_bytes(4, "little", signed=True)
    return bytes(row)


def _enc_data_payload(game_hour: int, source_flags: int, events: list, *, strings: bytes = b"",
                      player_rows: bytes = b"") -> bytes:
    """Byte-mirror of encode_payload (session...live/src/lib.rs:226-309)."""
    body = b"".join(
        _enc_event_row(*e) if isinstance(e, tuple) else e for e in events
    )
    event_hours = [e[2] for e in events if isinstance(e, tuple)]
    min_hour = min(event_hours) if event_hours else game_hour
    max_hour = max(event_hours) if event_hours else game_hour
    n_players = len(player_rows) // PLAYER_COUNTRY_ROW_LEN
    n_events = len(events)
    root_evidence_mask = (1 << 2) if (n_events or n_players) else 0  # DOMAIN57_ROOT_GAME_SPEED
    out = bytearray()
    out += PAYLOAD_MAGIC
    out += PAYLOAD_VERSION.to_bytes(2, "little")
    out += PAYLOAD_HEADER_LEN.to_bytes(2, "little")
    out += DOMAIN_ID.to_bytes(2, "little")
    out += PLAYER_COUNTRY_ROW_LEN.to_bytes(2, "little")
    out += SESSION_EVENT_ROW_LEN.to_bytes(2, "little")
    out += (0).to_bytes(2, "little")
    out += int(game_hour).to_bytes(4, "little", signed=True)
    out += (0).to_bytes(4, "little")  # status_flags
    out += int(source_flags).to_bytes(4, "little")
    out += n_players.to_bytes(4, "little")
    out += n_events.to_bytes(4, "little")
    out += len(strings).to_bytes(4, "little")
    out += (0).to_bytes(4, "little")  # schema_flags
    out += root_evidence_mask.to_bytes(4, "little")
    out += int(min_hour).to_bytes(4, "little", signed=True)
    out += int(max_hour).to_bytes(4, "little", signed=True)
    out += (0).to_bytes(8, "little")
    assert len(out) == PAYLOAD_HEADER_LEN
    return bytes(out) + player_rows + body + strings


def _perf_payload() -> bytes:
    return bytes(COMPACT_DOMAIN_PERF_BASE_LEN)  # 40 zero bytes, no diagnostic trailer


def _frame_end_payload() -> bytes:
    return bytes(COMPACT_FRAME_END_PAYLOAD_LEN)  # 32 zero bytes


def _enc_record(sequence: int, game_hour: int, domain_id: int, record_type: int, payload: bytes,
                *, record_flags: int = 0) -> bytes:
    out = bytearray()
    out += RECORD_MAGIC
    out += RECORD_VERSION.to_bytes(2, "little")
    out += RECORD_HEADER_LEN.to_bytes(2, "little")
    out += int(sequence).to_bytes(8, "little")
    out += int(game_hour).to_bytes(4, "little", signed=True)
    out += (domain_id & 0xFFFF).to_bytes(2, "little")
    out += (record_type & 0xFFFF).to_bytes(2, "little")
    out += len(payload).to_bytes(4, "little")
    out += int(record_flags).to_bytes(4, "little")
    assert len(out) == RECORD_HEADER_LEN
    return bytes(out) + payload


def _canonical_stream(frames: list) -> tuple[bytes, list[int]]:
    """Build a production-shaped stream: DATA + PERF + FRAME_END per frame, with
    strictly +1 sequences and consecutive game hours. `frames` is a list of dicts
    {hour, events, source_flags?}. Returns (bytes, data_record_sequences)."""
    out = bytearray()
    seq = 1
    data_seqs = []
    for f in frames:
        data_seqs.append(seq)
        payload = _enc_data_payload(
            f["hour"], f.get("source_flags", SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED),
            f["events"], strings=f.get("strings", b""), player_rows=f.get("player_rows", b""),
        )
        out += _enc_record(seq, f["hour"], DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, payload)
        seq += 1
        out += _enc_record(seq, f["hour"], DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, _perf_payload())
        seq += 1
        out += _enc_record(seq, f["hour"], DOMAIN_FRAME, RECORD_TYPE_FRAME_END, _frame_end_payload())
        seq += 1
    return bytes(out), data_seqs


def _record_spans(buf: bytes) -> list[tuple[int, int]]:
    """Walk a well-formed base stream; return [(offset, record_len)] per record."""
    spans = []
    off = 0
    while off < len(buf):
        payload_len = _u32(buf, off + 24)
        spans.append((off, RECORD_HEADER_LEN + payload_len))
        off += RECORD_HEADER_LEN + payload_len
    return spans


def _gs_frames(series, source_flags=SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED,
               start_hour=100):
    """One GameSpeed anchor row per consecutive-hour frame from `series`."""
    return [
        {"hour": start_hour + i, "events": [(KIND_GAME_SPEED, raw, start_hour + i)],
         "source_flags": source_flags}
        for i, raw in enumerate(series)
    ]


def _build_probe_transcript(root: Path):
    """Byte-compatible with verify_speed_root_evidence_v2's own fixture so that
    probe_verifier.verify() passes over it (allow_fixture_exe=True)."""
    pv = probe_verifier
    exe = root / "hoi4.exe"
    probe = root / "probe.py"
    static = root / "static.json"
    transcript_path = root / "transcript.json"
    ledger = root / "nonce_ledger"
    ledger.mkdir()
    exe.write_bytes(b"verified-exe-fixture")
    probe.write_bytes(b"probe-v2-fixture")
    exe_sha = pv.sha256_file(exe)
    probe_sha = pv.sha256_file(probe)
    static_value = {
        "schema": pv.STATIC_SCHEMA,
        "exe_sha256": exe_sha,
        "module_rvas": {k: pv.ROOTS[k] for k in pv.STATIC_MODULE_KEYS},
        "field_offsets": {k: pv.ROOTS[k] for k in pv.STATIC_OFFSET_KEYS},
        "speed_mapping": {"ui_speeds": [1, 2, 3, 4, 5], "raw_values": [0, 1, 2, 3, 4]},
        "evidence_export_sha256": "b" * 64,
    }
    _write_json(static, static_value)
    static_sha = pv.sha256_file(static)
    base = 0x140000000
    gs = 0x200000000
    clock = 0x210000000
    now = 133900000000000000

    def sample(seq, ui, raw, phase, pause, ticks):
        return {
            "sequence": seq,
            "action_nonce_hex": f"{seq:064x}",
            "requested_ui_speed": ui,
            "expected_raw_speed": raw,
            "phase": phase,
            "addresses": {
                "cgame_state_slot": f"0x{base + 0x3316E90:X}",
                "clock_slot_a": f"0x{base + 0x33172E0:X}",
                "clock_slot_b": f"0x{base + 0x33172E8:X}",
                "clock_vtable": f"0x{base + 0x2952510:X}",
                "cgame_state": f"0x{gs:X}",
                "clock": f"0x{clock:X}",
                "speed": f"0x{gs + 0x4BC:X}",
                "pause": f"0x{clock + 0x6C1:X}",
                "tick": f"0x{gs + 0x4B8:X}",
            },
            "reads": [
                {
                    "ordinal": i + 1,
                    "observed_filetime_utc": now + seq * 100000 + i * 1000,
                    "cgame_state_value": f"0x{gs:X}",
                    "clock_slot_a_value": f"0x{clock:X}",
                    "clock_slot_b_value": f"0x{clock:X}",
                    "clock_vtable_value": f"0x{base + 0x2952510:X}",
                    "speed_raw": raw,
                    "pause_raw": pause,
                    "tick_progress_bits": f"0x{bits:08x}",
                }
                for i, bits in enumerate(ticks)
            ],
        }

    moving = [0x3DCCCCCD, 0x3E4CCCCD, 0x3E99999A]
    frozen = [0x3E99999A] * 3
    transcript = {
        "schema": pv.TRANSCRIPT_SCHEMA,
        "capture": {
            "session_id": "session_v2_test",
            "evidence_nonce_hex": "a" * 64,
            "nonce_ledger_root": str(ledger.resolve()),
            "created_filetime_utc": now,
            "completed_filetime_utc": 0,
        },
        "process": {
            "pid": 123,
            "creation_filetime_utc": now - 1000000,
            "exe_path": str(exe.resolve()),
            "exe_sha256": exe_sha,
            "exe_file_id": pv.file_identity(exe),
            "module_base": f"0x{base:X}",
        },
        "tool": {"probe_path": str(probe.resolve()), "probe_sha256": probe_sha},
        "static_disasm": {"artifact_path": str(static.resolve()), "artifact_sha256": static_sha},
        "roots": pv.ROOTS,
        "speed_samples": [sample(i, i, i - 1, "speed_cycle", 0, moving) for i in range(1, 6)],
        "pause_samples": [
            sample(6, 3, 2, "running_before", 0, moving),
            sample(7, 3, 2, "paused", 1, frozen),
            sample(8, 3, 2, "running_after", 0, moving),
        ],
        "transcript_sha256": "",
    }
    for fs in transcript["speed_samples"] + transcript["pause_samples"]:
        for fr in fs["reads"]:
            fr["observed_filetime_utc"] = (
                now + fs["sequence"] * 20_000_000 + (fr["ordinal"] - 1) * 5_000_000
            )
    transcript["capture"]["completed_filetime_utc"] = now + 200_000_000

    def seal():
        transcript["transcript_sha256"] = pv.transcript_digest(transcript)
        _write_json(transcript_path, transcript)
        nonce = transcript["capture"]["evidence_nonce_hex"]
        _write_json(
            ledger / f"{nonce}.consumed_success.json",
            {
                "schema": "battle-plan.speed_root_nonce_seal.v2",
                "state": "consumed_success",
                "session_id": transcript["capture"]["session_id"],
                "evidence_nonce_hex": nonce,
                "transcript_path": str(transcript_path.resolve()),
                "transcript_sha256": transcript["transcript_sha256"],
                "created_filetime_utc": transcript["capture"]["created_filetime_utc"],
                "consumed_filetime_utc": transcript["capture"]["completed_filetime_utc"],
            },
        )
        _write_json(
            ledger / f"{nonce}.issued.json.consumed",
            {
                "schema": "battle-plan.speed_root_nonce_issue.v2",
                "state": "issued",
                "session_id": transcript["capture"]["session_id"],
                "evidence_nonce_hex": nonce,
                "transcript_path": str(transcript_path.resolve()),
                "created_filetime_utc": transcript["capture"]["created_filetime_utc"],
            },
        )

    seal()
    pins = {
        "transcript": transcript_path,
        "nonce_ledger_root": ledger,
        "expected_transcript_sha256": transcript["transcript_sha256"],
        "expected_session_id": "session_v2_test",
        "expected_evidence_nonce": "a" * 64,
        "expected_exe_sha256": exe_sha,
        "expected_probe_tool_sha256": probe_sha,
        "expected_static_disasm_sha256": static_sha,
    }
    return pins


def _manifest_dict(pins, stream_path, stream_sha, alignments):
    return {
        "schema": ALIGNMENT_SCHEMA,
        "emission_stream_path": str(Path(stream_path).resolve()),
        "emission_stream_sha256": stream_sha,
        "transcript_sha256": pins["expected_transcript_sha256"],
        "session_id": pins["expected_session_id"],
        "evidence_nonce_hex": pins["expected_evidence_nonce"],
        "alignments": alignments,
    }


def _default_alignments(data_seqs, hours):
    # Reference the first five consecutive-hour DATA frames -> UI speeds 1..5.
    return [
        {"emission_record_sequence": data_seqs[i], "emission_game_hour_raw": hours[i],
         "probe_speed_sample_sequence": i + 1}
        for i in range(5)
    ]


def _build_emission_and_manifest(root: Path, pins: dict):
    """Clean opt-in emission: 6 consecutive DATA+PERF+FRAME_END frames stepping raw
    0,1,2,3,4 for UI speeds 1..5 plus one extra valid frame (raw 2), and a matching
    manifest referencing the first five DATA records."""
    frames = _gs_frames([0, 1, 2, 3, 4, 2])
    stream, data_seqs = _canonical_stream(frames)
    hours = [100 + i for i in range(len(frames))]
    stream_path = root / "domain57_stream.bin"
    stream_path.write_bytes(stream)
    stream_sha = sha256_file(stream_path)
    manifest = _manifest_dict(pins, stream_path, stream_sha, _default_alignments(data_seqs, hours))
    manifest_path = root / "alignment.json"
    _write_json(manifest_path, manifest)
    return stream_path, stream_sha, manifest, manifest_path, data_seqs, hours


def _args_for(pins, stream_path, stream_sha, manifest_path, out):
    return SimpleNamespace(
        transcript=pins["transcript"],
        nonce_ledger_root=pins["nonce_ledger_root"],
        expected_transcript_sha256=pins["expected_transcript_sha256"],
        expected_session_id=pins["expected_session_id"],
        expected_evidence_nonce=pins["expected_evidence_nonce"],
        expected_exe_sha256=pins["expected_exe_sha256"],
        expected_probe_tool_sha256=pins["expected_probe_tool_sha256"],
        expected_static_disasm_sha256=pins["expected_static_disasm_sha256"],
        emission_stream=stream_path,
        expected_emission_stream_sha256=stream_sha,
        alignment_manifest=manifest_path,
        expected_alignment_manifest_sha256=sha256_file(manifest_path),
        out=out,
    )


def self_test() -> int:
    passed = 0
    global MAX_STREAM_BYTES
    with tempfile.TemporaryDirectory(prefix="d57_crit4_") as temp:
        root = Path(temp)
        pins = _build_probe_transcript(root)
        stream_path, stream_sha, manifest, manifest_path, data_seqs, hours = _build_emission_and_manifest(
            root, pins
        )
        base_bytes = stream_path.read_bytes()
        canon_align = _default_alignments(data_seqs, hours)
        out = root / "gate.json"
        counter = [0]

        # ---- valid PASS ---------------------------------------------------- #
        args = _args_for(pins, stream_path, stream_sha, manifest_path, out)
        report = compare(args, allow_fixture_exe=True)
        assert report["verdict"] == "passed"
        assert report["total_game_speed_rows"] == 6
        assert report["domain57_data_records"] == 6
        assert report["ground_truth_raw_by_ui_speed"] == {"1": 0, "2": 1, "3": 2, "4": 3, "5": 4}
        passed += 1

        def expect_fail(name, *, stream_bytes=None, alignments=None, manifest_mutate=None,
                        args_mutate=None, manifest_text=None, must_contain=None):
            nonlocal passed
            counter[0] += 1
            tag = f"{counter[0]:03d}"
            if stream_bytes is None:
                stream_bytes = base_bytes
            sp = root / f"s_{tag}.bin"
            sp.write_bytes(stream_bytes)
            ssha = sha256_file(sp)
            mp = root / f"m_{tag}.json"
            if manifest_text is not None:
                mp.write_text(manifest_text, encoding="utf-8")
            else:
                chosen = canon_align if alignments is None else alignments
                man = _manifest_dict(pins, sp, ssha, copy.deepcopy(chosen))
                if manifest_mutate:
                    manifest_mutate(man)
                _write_json(mp, man)
            a = _args_for(pins, sp, ssha, mp, out)
            if args_mutate:
                args_mutate(a)
            try:
                compare(a, allow_fixture_exe=True)
            except GateError as exc:
                if must_contain is not None and must_contain not in str(exc):
                    raise AssertionError(f"{name}: wrong reason: {exc}")
                passed += 1
            else:
                raise AssertionError(f"mutation unexpectedly passed: {name}")

        def patched(mutate):
            ba = bytearray(base_bytes)
            spans = _record_spans(base_bytes)
            mutate(ba, spans)
            return bytes(ba)

        # ---- comparison-logic mutations (manifest side; verified-good) ----- #
        expect_fail("manifest extra key",
                    manifest_mutate=lambda m: m.update({"unexpected": True}))
        expect_fail("manifest nested extra key",
                    manifest_mutate=lambda m: m["alignments"][0].update({"note": 1}))
        expect_fail("wrong schema",
                    manifest_mutate=lambda m: m.update({"schema": "battle-plan.d57_emission_alignment.v0"}))
        expect_fail("transcript pin tamper",
                    manifest_mutate=lambda m: m.update({"transcript_sha256": "0" * 64}))
        expect_fail("session pin tamper",
                    manifest_mutate=lambda m: m.update({"session_id": "other_session"}))
        expect_fail("nonce pin tamper",
                    manifest_mutate=lambda m: m.update({"evidence_nonce_hex": "c" * 64}))
        expect_fail("stream path tamper",
                    manifest_mutate=lambda m: m.update({"emission_stream_path": str(root / "nope.bin")}))
        expect_fail("mismatched value (swap sample refs)",
                    manifest_mutate=lambda m: (
                        m["alignments"][0].update({"probe_speed_sample_sequence": 2}),
                        m["alignments"][1].update({"probe_speed_sample_sequence": 1}),
                    ))
        expect_fail("misaligned / missing record",
                    manifest_mutate=lambda m: m["alignments"].__setitem__(
                        0, {"emission_record_sequence": 999, "emission_game_hour_raw": 100,
                            "probe_speed_sample_sequence": 1}))
        expect_fail("game_hour tamper",
                    manifest_mutate=lambda m: m["alignments"][0].update({"emission_game_hour_raw": 555}))
        expect_fail("duplicate record sequence",
                    manifest_mutate=lambda m: m["alignments"].__setitem__(
                        1, {"emission_record_sequence": data_seqs[0], "emission_game_hour_raw": hours[0],
                            "probe_speed_sample_sequence": 2}))
        expect_fail("duplicate sample reference",
                    manifest_mutate=lambda m: m["alignments"].__setitem__(
                        1, {"emission_record_sequence": data_seqs[5], "emission_game_hour_raw": hours[5],
                            "probe_speed_sample_sequence": 1}))
        expect_fail("coverage gap (drop UI speed 5)",
                    manifest_mutate=lambda m: m["alignments"].pop())
        expect_fail("boolean sample seq",
                    manifest_mutate=lambda m: m["alignments"][0].update({"probe_speed_sample_sequence": True}))
        expect_fail("empty alignments",
                    manifest_mutate=lambda m: m.update({"alignments": []}))
        expect_fail("no alignments key handled",
                    manifest_mutate=lambda m: m.update({"alignments": {}}))
        expect_fail("manifest hash pin drift",
                    args_mutate=lambda a: setattr(a, "expected_alignment_manifest_sha256", "0" * 64))
        expect_fail("non-hex manifest sha pin",
                    args_mutate=lambda a: setattr(a, "expected_alignment_manifest_sha256", "z" * 64))
        expect_fail("duplicate JSON key in manifest",
                    manifest_text='{"schema":"x","schema":"y"}')
        expect_fail("tampered probe transcript pin",
                    args_mutate=lambda a: setattr(a, "expected_transcript_sha256", "0" * 64))

        # ---- emission stream pin mutations --------------------------------- #
        expect_fail("emission stream byte tamper (sha mismatch)",
                    stream_bytes=base_bytes + b"\x00",
                    manifest_mutate=lambda m: m.__setitem__("emission_stream_sha256", stream_sha),
                    args_mutate=lambda a: setattr(a, "expected_emission_stream_sha256", stream_sha))
        expect_fail("non-hex emission stream sha pin",
                    manifest_mutate=lambda m: m.__setitem__("emission_stream_sha256", "z" * 64),
                    args_mutate=lambda a: setattr(a, "expected_emission_stream_sha256", "z" * 64))
        MAX_STREAM_BYTES = 64
        expect_fail("stream-size bound", must_contain="512 MiB bound")
        MAX_STREAM_BYTES = 512 * 1024 * 1024

        # ---- coercion / value mutations (in-range and out-of-range) -------- #
        expect_fail("out-of-range raw 5 in a frame",
                    stream_bytes=_canonical_stream(_gs_frames([0, 1, 2, 3, 5, 2]))[0],
                    must_contain="game speed 5 outside 0..4")
        expect_fail("negative raw -1 in a frame",
                    stream_bytes=_canonical_stream(_gs_frames([0, 1, 2, 3, -1, 2]))[0])
        expect_fail("displayed 1..5 datum at the wire",
                    stream_bytes=_canonical_stream(_gs_frames([1, 2, 3, 4, 5, 2]))[0])
        expect_fail("single-point in-range coercion (UI1 emits 1 not 0)",
                    stream_bytes=_canonical_stream(_gs_frames([1, 1, 2, 3, 4, 2]))[0])

        # ---- record framing (H4CR header) --------------------------------- #
        expect_fail("record version wrong",
                    stream_bytes=patched(lambda ba, sp: ba.__setitem__(sp[0][0] + 4, 2)),
                    must_contain="unsupported compact header")
        expect_fail("record header_len wrong",
                    stream_bytes=patched(lambda ba, sp: ba.__setitem__(sp[0][0] + 6, 40)),
                    must_contain="unsupported compact header")
        expect_fail("record magic wrong",
                    stream_bytes=patched(lambda ba, sp: ba.__setitem__(sp[0][0], ord("X"))),
                    must_contain="invalid compact record magic")
        expect_fail("record payload truncated",
                    stream_bytes=base_bytes[:-5],
                    must_contain="truncated compact record payload")
        expect_fail("record payload_len too large",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, sp[0][0] + 24, COMPACT_MAX_PAYLOAD_LEN + 1)),
                    must_contain="payload too large")
        expect_fail("first sequence not 1",
                    stream_bytes=patched(lambda ba, sp: _patch_u64(ba, sp[0][0] + 8, 7)),
                    must_contain="expected 1")
        expect_fail("sequence discontinuity",
                    stream_bytes=patched(lambda ba, sp: _patch_u64(ba, sp[1][0] + 8, 99)),
                    must_contain="sequence discontinuity")

        # ---- record semantics (flags / PERF / FRAME_END grammar) ----------- #
        expect_fail("rejected record flags on DATA",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, sp[0][0] + 28, 1 << 30)),
                    must_contain="rejected bits")
        expect_fail("FRAME_END wrong domain",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, sp[2][0] + 20, 41)),
                    must_contain="FRAME_END domain")
        expect_fail("FRAME_END wrong payload length",
                    stream_bytes=_stream_with_frame_end_len(31),
                    must_contain="FRAME_END payload length")
        expect_fail("PERF base too short",
                    stream_bytes=_stream_with_perf_payload(bytes(30)),
                    must_contain="shorter than 40")
        expect_fail("PERF reserved base nonzero",
                    stream_bytes=_stream_with_perf_payload(bytes(36) + (1).to_bytes(4, "little")),
                    must_contain="reserved base word")
        expect_fail("PERF unannounced trailer",
                    stream_bytes=_stream_with_perf_payload(bytes(44)),
                    must_contain="unannounced trailer")
        _perf_name_stream, _pn_seqs, _pn_hours = _stream_with_perf_name(b"\xc3\xa9")  # UTF-8 "e-acute"
        expect_fail("PERF PDU1 unit name non-ASCII (utf-8 e-acute)",
                    stream_bytes=_perf_name_stream,
                    alignments=_default_alignments(_pn_seqs, _pn_hours),
                    must_contain="is invalid")
        _drop_perf_stream, _dp_seqs, _dp_hours = _stream_dropping_perf(0)
        expect_fail("PERF cardinality (DATA+FRAME_END but no PERF)",
                    stream_bytes=_drop_perf_stream,
                    alignments=_default_alignments(_dp_seqs, _dp_hours),
                    must_contain="PERF records, expected exactly 1")
        expect_fail("unknown record type",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, sp[0][0] + 22, 99)),
                    must_contain="unknown compact record type 99")

        # ---- domain-57 payload (gate_payload) ------------------------------ #
        poff = _record_spans(base_bytes)[0][0] + RECORD_HEADER_LEN  # frame-0 DATA payload start
        eoff = poff + PAYLOAD_HEADER_LEN  # first event row start (no player rows)
        # Frame-5 (unreferenced) DATA payload start: DATA of the 6th frame is the
        # 16th record (index 15). Used for checks whose gate_payload short-circuit
        # would otherwise trip the anchor-count check on a referenced frame.
        p5off = _record_spans(base_bytes)[15][0] + RECORD_HEADER_LEN
        expect_fail("payload magic wrong",
                    stream_bytes=patched(lambda ba, sp: ba.__setitem__(poff, ord("X"))),
                    must_contain="invalid payload magic")
        expect_fail("payload version wrong",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, poff + 4, 2)),
                    must_contain="payload version")
        expect_fail("payload domain wrong",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, poff + 8, 45)),
                    must_contain="payload domain_id")
        expect_fail("payload player_row_len wrong",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, poff + 10, 41)),
                    must_contain="player-country row length")
        expect_fail("payload event_row_len wrong",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, poff + 12, 63)),
                    must_contain="session event row length")
        expect_fail("payload declared-length mismatch",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 32, 5)),
                    must_contain="payload length")
        expect_fail("unknown status flags",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 20, 1 << 14)),
                    must_contain="unknown status flags")
        expect_fail("unknown source flags (0x40000003)",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 24, 0x40000003)),
                    must_contain="unknown source flags")
        expect_fail("schema_flags nonzero",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 40, 1)),
                    must_contain="schema_flags")
        expect_fail("root evidence missing",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 44, 0)),
                    must_contain="root evidence")
        expect_fail("payload header_len wrong (unreferenced frame)",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, p5off + 6, 63)),
                    must_contain="payload header_len")
        expect_fail("declared min_event_hour disagreement",
                    stream_bytes=patched(lambda ba, sp: _patch_i32(ba, poff + 48, 42)),
                    must_contain="min_event_hour_raw")
        expect_fail("declared max_event_hour disagreement",
                    stream_bytes=patched(lambda ba, sp: _patch_i32(ba, poff + 52, 42)),
                    must_contain="max_event_hour_raw")
        expect_fail("unknown event kind",
                    stream_bytes=patched(lambda ba, sp: _patch_u16(ba, eoff + 0, 9)),
                    must_contain="unknown kind 9")
        expect_fail("event flags unknown",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, eoff + 4, 1 << 4)),
                    must_contain="unknown flags")
        expect_fail("event tag_db_id too big",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, eoff + 28, 5000)),
                    must_contain="tag_db_id")
        expect_fail("payload<->header hour disagreement",
                    stream_bytes=patched(lambda ba, sp: _patch_i32(ba, poff + 16, 999)),
                    must_contain="does not match compact record hour")
        expect_fail("invalid Pause value in extra frame",
                    stream_bytes=_canonical_stream(
                        _gs_frames([0, 1, 2, 3, 4]) + [{"hour": 105, "events": [(KIND_PAUSE, 2, 105)]}])[0],
                    must_contain="pause value 2 is not 0/1")
        expect_fail("text-bearing kind lacks text",
                    stream_bytes=_canonical_stream(
                        _gs_frames([0, 1, 2, 3, 4]) + [{"hour": 105, "events": [(KIND_CHAT, 0, 105)]}])[0],
                    must_contain="lacks text")
        _pf_stream, _pf_seqs, _pf_hours = _stream_with_bad_player_flags()
        expect_fail("player row unknown flag bit",
                    stream_bytes=_pf_stream,
                    alignments=_default_alignments(_pf_seqs, _pf_hours),
                    must_contain="player row 0: unknown flags")
        expect_fail("player row invalid country tag",
                    stream_bytes=_stream_with_bad_player_row(),
                    must_contain="invalid country tag")

        # ---- stream aggregate structure ------------------------------------ #
        expect_fail("missing PERF/FRAME_END (DATA-only stream)",
                    stream_bytes=_data_only_stream([0, 1, 2, 3, 4]),
                    must_contain="FRAME_END")
        expect_fail("skipped (non-consecutive) frame hours",
                    stream_bytes=_canonical_stream([
                        {"hour": 100, "events": [(KIND_GAME_SPEED, 0, 100)]},
                        {"hour": 101, "events": [(KIND_GAME_SPEED, 1, 101)]},
                        {"hour": 103, "events": [(KIND_GAME_SPEED, 2, 103)]},
                    ])[0],
                    must_contain="not consecutive")
        expect_fail("fatal partial-data status",
                    stream_bytes=patched(lambda ba, sp: _patch_u32(ba, poff + 20, STATUS_PARTIAL_DATA)),
                    must_contain="fatal status")

        # ---- opt-in provenance + anchor cardinality ------------------------ #
        expect_fail("GameSpeed frame lacks SESSION_ROOT_VALIDATED",
                    stream_bytes=_canonical_stream(_gs_frames([0, 1, 2, 3, 4], source_flags=SOURCE_FLAG_DLL_NATIVE))[0],
                    must_contain="SESSION_ROOT_VALIDATED")
        expect_fail("GameSpeed frame is a proof fixture",
                    stream_bytes=_canonical_stream(
                        _gs_frames([0, 1, 2, 3, 4],
                                   source_flags=SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED | SOURCE_FLAG_PROOF_FIXTURE))[0],
                    must_contain="proof fixture")
        expect_fail("two GameSpeed rows in one anchor frame",
                    stream_bytes=_canonical_stream(
                        [{"hour": 100, "events": [(KIND_GAME_SPEED, 0, 100), (KIND_GAME_SPEED, 0, 100)]}]
                        + _gs_frames([1, 2, 3, 4], start_hour=101))[0],
                    must_contain="GameSpeed rows")

        # ---- absent emission (opt-in required) ----------------------------- #
        expect_fail("absent emission (no GameSpeed rows)",
                    stream_bytes=_canonical_stream(
                        [{"hour": 100 + i, "events": [(KIND_PAUSE, 0, 100 + i)]} for i in range(3)])[0],
                    must_contain="no D57 emission")

    print(f"d57 emission-vs-raw crit4 self-test: passed={passed} failed=0")
    return 0


def _patch_u16(ba: bytearray, off: int, val: int) -> None:
    ba[off : off + 2] = (val & 0xFFFF).to_bytes(2, "little")


def _patch_u32(ba: bytearray, off: int, val: int) -> None:
    ba[off : off + 4] = (val & 0xFFFFFFFF).to_bytes(4, "little")


def _patch_i32(ba: bytearray, off: int, val: int) -> None:
    ba[off : off + 4] = int(val).to_bytes(4, "little", signed=True)


def _patch_u64(ba: bytearray, off: int, val: int) -> None:
    ba[off : off + 8] = (val & ((1 << 64) - 1)).to_bytes(8, "little")


_FIXTURE_SOURCE = SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED


def _data_only_stream(series) -> bytes:
    out = bytearray()
    for i, raw in enumerate(series, 1):
        out += _enc_record(
            i, 100 + i - 1, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA,
            _enc_data_payload(100 + i - 1, _FIXTURE_SOURCE, [(KIND_GAME_SPEED, raw, 100 + i - 1)]),
        )
    return bytes(out)


def _stream_dropping_perf(drop_index=0):
    """5 consecutive GameSpeed frames (speeds 0..4, hours 100..104); frame
    `drop_index` keeps its DATA + FRAME_END but has NO PERF record, so ONLY the
    per-frame PERF cardinality check (exactly-one-PERF) can reject it. Returns
    (bytes, data_seqs, hours)."""
    out = bytearray()
    seq = 1
    data_seqs = []
    hours = []
    for i, raw in enumerate([0, 1, 2, 3, 4]):
        hour = 100 + i
        hours.append(hour)
        data_seqs.append(seq)
        out += _enc_record(seq, hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA,
                           _enc_data_payload(hour, _FIXTURE_SOURCE, [(KIND_GAME_SPEED, raw, hour)]))
        seq += 1
        if i != drop_index:
            out += _enc_record(seq, hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, _perf_payload())
            seq += 1
        out += _enc_record(seq, hour, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, _frame_end_payload())
        seq += 1
    return bytes(out), data_seqs, hours


def _perf_payload_with_name(name_bytes: bytes) -> bytes:
    """A PDU1 diagnostic-trailer PERF payload with one unit named `name_bytes`.
    Pair with record_flags=COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER."""
    p = bytearray(COMPACT_DOMAIN_PERF_BASE_LEN)  # 40-byte base
    p += COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC  # PDU1
    p += COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_bytes(2, "little")
    p += (1).to_bytes(2, "little")  # unit_count = 1
    p += len(name_bytes).to_bytes(2, "little")  # unit name_len
    p += (0).to_bytes(2, "little")  # unit reserved
    p += (7).to_bytes(8, "little")  # unit value (8-byte tail of the 12-byte header)
    p += name_bytes
    return bytes(p)


def _stream_with_perf_name(name_bytes: bytes):
    """5 consecutive GameSpeed frames; frame 0's PERF carries a flagged PDU1 unit
    named `name_bytes`, so ONLY the PERF-name ASCII grammar can reject it (when the
    name is invalid). Returns (bytes, data_seqs, hours)."""
    out = bytearray()
    seq = 1
    data_seqs = []
    hours = []
    for i, raw in enumerate([0, 1, 2, 3, 4]):
        hour = 100 + i
        hours.append(hour)
        data_seqs.append(seq)
        out += _enc_record(seq, hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA,
                           _enc_data_payload(hour, _FIXTURE_SOURCE, [(KIND_GAME_SPEED, raw, hour)]))
        seq += 1
        if i == 0:
            out += _enc_record(seq, hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF,
                               _perf_payload_with_name(name_bytes),
                               record_flags=COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER)
        else:
            out += _enc_record(seq, hour, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, _perf_payload())
        seq += 1
        out += _enc_record(seq, hour, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, _frame_end_payload())
        seq += 1
    return bytes(out), data_seqs, hours


def _stream_with_bad_player_flags():
    """5 consecutive GameSpeed frames; frame 0's DATA also carries a valid-tag
    player-country row whose flags set an unknown bit (1<<4), so ONLY the player
    flag-mask check can reject it. Returns (bytes, data_seqs, hours)."""
    strings = b"host" + b"GER"  # player_name @0 len4, country_tag @4 len3 (valid)
    row = bytearray(PLAYER_COUNTRY_ROW_LEN)
    row[8:12] = (0).to_bytes(4, "little")  # player_name.off
    row[12:14] = (4).to_bytes(2, "little")  # player_name.len
    row[14:16] = (3).to_bytes(2, "little")  # country_tag.len
    row[16:20] = (4).to_bytes(4, "little")  # country_tag.off
    row[20:24] = (1).to_bytes(4, "little")  # tag_db_id (valid)
    row[28:32] = (1 << 4).to_bytes(4, "little")  # flags: unknown bit outside 0xF
    frames = _gs_frames([0, 1, 2, 3, 4])
    frames[0] = {**frames[0], "player_rows": bytes(row), "strings": strings}
    stream, data_seqs = _canonical_stream(frames)
    hours = [100 + i for i in range(5)]
    return stream, data_seqs, hours


def _stream_with_perf_payload(perf_payload: bytes) -> bytes:
    """One clean DATA + a corrupted PERF + FRAME_END frame (speed 0)."""
    out = bytearray()
    out += _enc_record(1, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA,
                       _enc_data_payload(100, SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED,
                                         [(KIND_GAME_SPEED, 0, 100)]))
    out += _enc_record(2, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, perf_payload)
    out += _enc_record(3, 100, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, _frame_end_payload())
    return bytes(out)


def _stream_with_frame_end_len(length: int) -> bytes:
    out = bytearray()
    out += _enc_record(1, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA,
                       _enc_data_payload(100, SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED,
                                         [(KIND_GAME_SPEED, 0, 100)]))
    out += _enc_record(2, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, _perf_payload())
    out += _enc_record(3, 100, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, bytes(length))
    return bytes(out)


def _stream_with_bad_player_row() -> bytes:
    """A frame carrying one player-country row with an invalid (lowercase) tag."""
    strings = b"host" + b"xxx"  # player_name @0 len4, country_tag @4 len3 (lowercase -> invalid)
    row = bytearray(PLAYER_COUNTRY_ROW_LEN)
    row[0:8] = (1).to_bytes(8, "little")  # player_key_hash
    row[8:12] = (0).to_bytes(4, "little")  # player_name.off
    row[12:14] = (4).to_bytes(2, "little")  # player_name.len
    row[14:16] = (3).to_bytes(2, "little")  # country_tag.len
    row[16:20] = (4).to_bytes(4, "little")  # country_tag.off
    row[20:24] = (1).to_bytes(4, "little")  # tag_db_id
    row[28:32] = (0).to_bytes(4, "little")  # flags
    payload = _enc_data_payload(100, SOURCE_FLAG_DLL_NATIVE | SOURCE_FLAG_SESSION_ROOT_VALIDATED,
                                [(KIND_GAME_SPEED, 0, 100)], strings=strings, player_rows=bytes(row))
    out = bytearray()
    out += _enc_record(1, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, payload)
    out += _enc_record(2, 100, DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, _perf_payload())
    out += _enc_record(3, 100, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, _frame_end_payload())
    return bytes(out)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="D57 criterion-4 emission-vs-raw comparison harness")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--transcript", type=Path)
    parser.add_argument("--nonce-ledger-root", type=Path)
    parser.add_argument("--expected-transcript-sha256")
    parser.add_argument("--expected-session-id")
    parser.add_argument("--expected-evidence-nonce")
    parser.add_argument("--expected-exe-sha256")
    parser.add_argument("--expected-probe-tool-sha256")
    parser.add_argument("--expected-static-disasm-sha256")
    parser.add_argument("--emission-stream", type=Path)
    parser.add_argument("--emission-stream-sha256")
    parser.add_argument("--alignment-manifest", type=Path)
    parser.add_argument("--alignment-manifest-sha256")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    required = (
        "transcript",
        "nonce_ledger_root",
        "expected_transcript_sha256",
        "expected_session_id",
        "expected_evidence_nonce",
        "expected_exe_sha256",
        "expected_probe_tool_sha256",
        "expected_static_disasm_sha256",
        "emission_stream",
        "emission_stream_sha256",
        "alignment_manifest",
        "alignment_manifest_sha256",
        "out",
    )
    if any(getattr(args, name) is None for name in required):
        parser.error("all verification inputs are required")
    args.expected_emission_stream_sha256 = args.emission_stream_sha256
    args.expected_alignment_manifest_sha256 = args.alignment_manifest_sha256
    try:
        report = compare(args)
    except (GateError, OSError) as exc:
        print(f"d57 emission-vs-raw crit4: FAIL: {exc}", file=sys.stderr)
        return 1
    _write_json(args.out, report)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
