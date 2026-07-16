#!/usr/bin/env python3
"""Deterministic, deny-extra verifier for read-only CID1 campaign-identity
transcripts. This is the ONLY component that emits a verdict.

It re-derives every rule from its own pinned constants, rejects any missing/extra
key, duplicate JSON key, wrong scalar type, malformed input, or oversized file,
cross-checks the durable nonce ledger, validates each bracketed observation
(coherent-read evidence, root/sentinel stability, MSVC std::string invariants,
canonical UUID grammar, and byte-identical A/B identity brackets), and -- when
the owner supplies the melted-save --expected-seed/--expected-id pins -- requires
exact equality with every stable observation. Absent pins => structural
verification only; absence is never fabricated into a match.

Supported build pin (exe SHA-256):
  870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad

============================ USAGE (owner CLI) ============================
Structural verdict:
  python verify_campaign_identity_evidence.py \
    --transcript <absolute transcript.json> \
    --nonce-ledger-root <absolute nonce ledger> \
    --expected-transcript-sha256 <caller pin> \
    --expected-session-id <caller pin> \
    --expected-evidence-nonce <caller pin> \
    --expected-exe-sha256 <caller pin> \
    --expected-probe-tool-sha256 <caller pin> \
    --expected-static-disasm-sha256 <caller pin> \
    --out <new gate.json>

L0 melt comparison (add both pins together; values extracted from the melted
save, e.g. seed 1026046527 / id a4b17fed-ed12-4361-9064-fb6576638110):
    ... --expected-seed 1026046527 \
        --expected-id a4b17fed-ed12-4361-9064-fb6576638110

Self-test (valid fixture + exhaustive mutation battery, every mutation must fail):
  python verify_campaign_identity_evidence.py --self-test
=========================================================================
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import struct
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

TRANSCRIPT_SCHEMA = "battle-plan.campaign_identity_evidence_transcript.v1"
GATE_SCHEMA = "battle-plan.campaign_identity_gate.v1"
STATIC_SCHEMA = "battle-plan.campaign_identity_static_disasm.v1"
NONCE_SEAL_SCHEMA = "battle-plan.campaign_identity_nonce_seal.v1"
NONCE_ISSUE_SCHEMA = "battle-plan.campaign_identity_nonce_issue.v1"
CURRENT_EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"

CGAMESTATE_SINGLETON_RVA = 0x3316E90
CLOCK_SINGLETON_A_RVA = 0x33172E0
CLOCK_SINGLETON_B_RVA = 0x33172E8
CLOCK_VTABLE_RVA = 0x2952510
CURRENT_HOUR_OFF = 0x468
GAME_UNIQUE_SEED_OFF = 0x720
GAME_UNIQUE_ID_OFF = 0x728
UUID_LEN = 36
MAX_STRING_CAPACITY = 1 << 20
MAX_COHERENT_ATTEMPTS_CEILING = 16
I32_MIN = -(1 << 31)
I32_MAX = (1 << 31) - 1
UUID_HYPHEN_POSITIONS = (8, 13, 18, 23)
UUID_HEX_CHAR_CLASS = "0123456789abcdef"

ROOTS = {
    "cgame_state_singleton_rva": "0x3316e90",
    "clock_singleton_a_rva": "0x33172e0",
    "clock_singleton_b_rva": "0x33172e8",
    "clock_vtable_rva": "0x2952510",
    "current_hour_off": "0x468",
    "game_unique_seed_off": "0x720",
    "game_unique_id_off": "0x728",
    "header_region_len": "0x28",
    "string_ptr_offset": "0x0",
    "string_size_offset": "0x10",
    "string_capacity_offset": "0x18",
    "sso_heap_threshold": "0x10",
    "uuid_hyphen_positions": "8,13,18,23",
    "uuid_hex_char_class": "0123456789abcdef",
}

HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX_ADDRESS = re.compile(r"^0x[0-9A-Fa-f]+$")
LABEL = re.compile(r"^[A-Za-z0-9_]{2,64}$")
UUID_GRAMMAR = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")

ROOT_KEYS = {"schema", "capture", "process", "tool", "static_disasm", "roots", "observations", "transcript_sha256"}
CAPTURE_KEYS = {"session_id", "evidence_nonce_hex", "nonce_ledger_root", "created_filetime_utc", "completed_filetime_utc"}
PROCESS_KEYS = {"pid", "creation_filetime_utc", "exe_path", "exe_sha256", "exe_file_id", "module_base"}
FILE_ID_KEYS = {"volume_serial_number", "file_index", "file_size"}
TOOL_KEYS = {"probe_path", "probe_sha256"}
STATIC_KEYS = {"artifact_path", "artifact_sha256"}
OBSERVATION_KEYS = {"sequence", "label", "action_nonce_hex", "action_token", "addresses", "read_a", "read_b"}
ADDRESS_KEYS = {"cgame_state_slot", "clock_slot_a", "clock_slot_b", "clock_vtable", "cgame_state", "clock", "current_hour", "game_unique_seed", "game_unique_id_string"}
READ_KEYS = {"read_label", "observed_filetime_utc", "coherent_attempts", "cgame_state_value", "clock_slot_a_value", "clock_slot_b_value", "clock_vtable_value", "game_hour_raw", "game_unique_seed", "game_unique_id", "string_storage", "string_size", "string_capacity", "header_region_sha256"}
STATIC_ROOT_KEYS = {"schema", "exe_sha256", "module_rvas", "field_offsets", "string_layout", "identity_grammar", "evidence_export_sha256"}
STATIC_MODULE_KEYS = {"cgame_state_singleton_rva", "clock_singleton_a_rva", "clock_singleton_b_rva", "clock_vtable_rva"}
STATIC_OFFSET_KEYS = {"current_hour_off", "game_unique_seed_off", "game_unique_id_off", "header_region_len"}
STATIC_STRING_LAYOUT_KEYS = {"string_ptr_offset", "string_size_offset", "string_capacity_offset", "sso_heap_threshold"}
STATIC_GRAMMAR_KEYS = {"uuid_len", "max_string_capacity", "hyphen_positions", "hex_char_class"}
NONCE_SEAL_KEYS = {"schema", "state", "session_id", "evidence_nonce_hex", "transcript_path", "transcript_sha256", "created_filetime_utc", "consumed_filetime_utc"}
NONCE_ISSUE_KEYS = {"schema", "state", "session_id", "evidence_nonce_hex", "transcript_path", "created_filetime_utc"}

IDENTITY_BRACKET_KEYS = ("game_unique_seed", "game_unique_id", "string_size", "string_capacity", "string_storage", "header_region_sha256")


class GateError(ValueError):
    pass


def exact_object(value, keys: set[str], label: str) -> dict:
    if not isinstance(value, dict) or set(value) != keys:
        actual = sorted(value) if isinstance(value, dict) else type(value).__name__
        raise GateError(f"{label}: missing/extra keys: {actual}")
    return value


def load_object(path: Path, label: str) -> dict:
    def no_duplicates(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise GateError(f"{label} contains duplicate key {key!r}")
            value[key] = item
        return value
    try:
        raw = path.read_bytes()
        if len(raw) > 16 * 1024 * 1024:
            raise GateError(f"{label} exceeds 16 MiB bound")
        value = json.loads(raw.decode("utf-8-sig"), object_pairs_hook=no_duplicates)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise GateError(f"{label} unreadable: {exc}") from exc
    if not isinstance(value, dict):
        raise GateError(f"{label} is not an object")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def canonical_bytes(value) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")


def transcript_digest(transcript: dict) -> str:
    unhashed = copy.deepcopy(transcript)
    unhashed.pop("transcript_sha256", None)
    return hashlib.sha256(canonical_bytes(unhashed)).hexdigest()


def hex_address(value, label: str) -> int:
    if not isinstance(value, str) or not HEX_ADDRESS.fullmatch(value):
        raise GateError(f"{label} is not a canonical hexadecimal address")
    parsed = int(value, 16)
    if parsed <= 0 or parsed > 0x7FFFFFFFFFFFFFFF:
        raise GateError(f"{label} is outside user address space")
    return parsed


def strict_int(value, label: str, minimum: int | None = None, maximum: int | None = None) -> int:
    if type(value) is not int or (minimum is not None and value < minimum) or (maximum is not None and value > maximum):
        raise GateError(f"{label} is not a canonical integer in range")
    return value


def file_identity(path: Path) -> dict:
    stat = path.stat()
    return {"volume_serial_number": int(stat.st_dev), "file_index": int(stat.st_ino), "file_size": int(stat.st_size)}


def uuid_string(value, label: str) -> str:
    if not isinstance(value, str) or not UUID_GRAMMAR.fullmatch(value):
        raise GateError(f"{label} is not a canonical lowercase UUID")
    return value


def validate_static(path: Path, expected_sha: str, exe_sha: str) -> dict:
    if not HEX64.fullmatch(expected_sha):
        raise GateError("expected static-disasm hash is not lowercase SHA-256")
    if sha256_file(path) != expected_sha:
        raise GateError("static-disasm artifact hash mismatch")
    value = exact_object(load_object(path, "static disassembly artifact"), STATIC_ROOT_KEYS, "static artifact")
    modules = exact_object(value["module_rvas"], STATIC_MODULE_KEYS, "static module_rvas")
    offsets = exact_object(value["field_offsets"], STATIC_OFFSET_KEYS, "static field_offsets")
    layout = exact_object(value["string_layout"], STATIC_STRING_LAYOUT_KEYS, "static string_layout")
    grammar = exact_object(value["identity_grammar"], STATIC_GRAMMAR_KEYS, "static identity_grammar")
    if value["schema"] != STATIC_SCHEMA or value["exe_sha256"] != exe_sha or not HEX64.fullmatch(str(value["evidence_export_sha256"])):
        raise GateError("static artifact schema/executable/export binding mismatch")
    expected_modules = {key: ROOTS[key] for key in STATIC_MODULE_KEYS}
    expected_offsets = {key: ROOTS[key] for key in STATIC_OFFSET_KEYS}
    expected_layout = {key: ROOTS[key] for key in STATIC_STRING_LAYOUT_KEYS}
    if modules != expected_modules or offsets != expected_offsets:
        raise GateError("static artifact roots/offsets mismatch")
    if layout != expected_layout:
        raise GateError("static artifact std::string layout mismatch")
    # The pinned grammar must equal exactly the UUID grammar this verifier enforces
    # (hyphen positions + lowercase-hex char class), so the artifact hash binds the
    # complete grammar the probe checks (finding round-2 #3).
    if grammar != {"uuid_len": UUID_LEN, "max_string_capacity": MAX_STRING_CAPACITY, "hyphen_positions": list(UUID_HYPHEN_POSITIONS), "hex_char_class": UUID_HEX_CHAR_CLASS}:
        raise GateError("static artifact identity grammar does not bind the enforced UUID grammar")
    return value


def validate_read(read, *, expected_label: str, parsed_addresses: dict, sequence: int) -> dict:
    read = exact_object(read, READ_KEYS, f"observation {sequence} read {expected_label}")
    if read["read_label"] != expected_label:
        raise GateError(f"observation {sequence} read label is not {expected_label}")
    strict_int(read["observed_filetime_utc"], f"observation {sequence} read {expected_label} time", 1)
    strict_int(read["coherent_attempts"], f"observation {sequence} read {expected_label} coherent_attempts", 2, MAX_COHERENT_ATTEMPTS_CEILING)
    if hex_address(read["cgame_state_value"], f"observation {sequence} read {expected_label} cgame_state") != parsed_addresses["cgame_state"]:
        raise GateError(f"observation {sequence} read {expected_label}: CGameState root drifted")
    if hex_address(read["clock_slot_a_value"], f"observation {sequence} read {expected_label} clock A") != parsed_addresses["clock"] or hex_address(read["clock_slot_b_value"], f"observation {sequence} read {expected_label} clock B") != parsed_addresses["clock"]:
        raise GateError(f"observation {sequence} read {expected_label}: clock slot A/B not equal to the root clock")
    if hex_address(read["clock_vtable_value"], f"observation {sequence} read {expected_label} clock vtable") != parsed_addresses["clock_vtable"]:
        raise GateError(f"observation {sequence} read {expected_label}: clock vtable sentinel mismatch")
    strict_int(read["game_hour_raw"], f"observation {sequence} read {expected_label} game_hour", I32_MIN, I32_MAX)
    strict_int(read["game_unique_seed"], f"observation {sequence} read {expected_label} game_unique_seed", I32_MIN, I32_MAX)
    uuid = uuid_string(read["game_unique_id"], f"observation {sequence} read {expected_label} game_unique_id")
    size = strict_int(read["string_size"], f"observation {sequence} read {expected_label} string_size", UUID_LEN, UUID_LEN)
    if len(uuid) != size:
        raise GateError(f"observation {sequence} read {expected_label}: UUID length disagrees with string_size")
    capacity = strict_int(read["string_capacity"], f"observation {sequence} read {expected_label} string_capacity", size, MAX_STRING_CAPACITY)
    storage = read["string_storage"]
    if storage not in ("heap", "inline"):
        raise GateError(f"observation {sequence} read {expected_label}: unknown string storage")
    if (capacity >= 16) != (storage == "heap"):
        raise GateError(f"observation {sequence} read {expected_label}: storage/capacity discriminant inconsistent")
    if storage != "heap":
        # A 36-char canonical UUID never fits in the 16-byte SSO buffer.
        raise GateError(f"observation {sequence} read {expected_label}: a canonical UUID must be heap-stored")
    if not isinstance(read["header_region_sha256"], str) or not HEX64.fullmatch(read["header_region_sha256"]):
        raise GateError(f"observation {sequence} read {expected_label}: header_region_sha256 is not lowercase SHA-256")
    return read


def validate_observation(observation, *, expected_sequence: int, base: int, stable_addresses: dict | None, nonce_set: set[str]) -> tuple[dict, list[dict]]:
    observation = exact_object(observation, OBSERVATION_KEYS, f"observation {expected_sequence}")
    if strict_int(observation["sequence"], "observation sequence", 1) != expected_sequence:
        raise GateError(f"observation {expected_sequence}: sequence out of order")
    label = observation["label"]
    if not isinstance(label, str) or not LABEL.fullmatch(label):
        raise GateError(f"observation {expected_sequence}: invalid label")
    if observation["action_token"] != f"OBSERVE_CAMPAIGN_IDENTITY_{label}_AND_ACK":
        raise GateError(f"observation {expected_sequence}: action token does not bind the label")
    nonce = observation["action_nonce_hex"]
    if not isinstance(nonce, str) or not HEX64.fullmatch(nonce) or nonce in nonce_set:
        raise GateError(f"observation {expected_sequence}: invalid/reused action nonce")
    nonce_set.add(nonce)
    addresses = exact_object(observation["addresses"], ADDRESS_KEYS, f"observation {expected_sequence} addresses")
    parsed = {key: hex_address(value, f"observation {expected_sequence} {key}") for key, value in addresses.items()}
    if parsed["current_hour"] != parsed["cgame_state"] + CURRENT_HOUR_OFF or parsed["game_unique_seed"] != parsed["cgame_state"] + GAME_UNIQUE_SEED_OFF or parsed["game_unique_id_string"] != parsed["cgame_state"] + GAME_UNIQUE_ID_OFF:
        raise GateError(f"observation {expected_sequence}: derived identity-field address mismatch")
    if parsed["cgame_state_slot"] != base + CGAMESTATE_SINGLETON_RVA or parsed["clock_slot_a"] != base + CLOCK_SINGLETON_A_RVA or parsed["clock_slot_b"] != base + CLOCK_SINGLETON_B_RVA or parsed["clock_vtable"] != base + CLOCK_VTABLE_RVA:
        raise GateError(f"observation {expected_sequence}: module-relative root address mismatch")
    if parsed["cgame_state"] < 0x10000 or parsed["clock"] < 0x10000 or base <= 0:
        raise GateError(f"observation {expected_sequence}: invalid root address")
    if stable_addresses is not None and addresses != stable_addresses:
        raise GateError(f"observation {expected_sequence}: root addresses changed within the process")
    read_a = validate_read(observation["read_a"], expected_label="A", parsed_addresses=parsed, sequence=expected_sequence)
    read_b = validate_read(observation["read_b"], expected_label="B", parsed_addresses=parsed, sequence=expected_sequence)
    if any(read_a[key] != read_b[key] for key in IDENTITY_BRACKET_KEYS):
        raise GateError(f"observation {expected_sequence}: bracketing reads A and B are not byte-identical on identity")
    return addresses, [read_a, read_b]


def verify(args) -> dict:
    transcript_path = args.transcript.resolve()
    transcript = exact_object(load_object(transcript_path, "campaign-identity transcript"), ROOT_KEYS, "transcript")
    capture = exact_object(transcript["capture"], CAPTURE_KEYS, "capture")
    process = exact_object(transcript["process"], PROCESS_KEYS, "process")
    identity = exact_object(process["exe_file_id"], FILE_ID_KEYS, "process exe_file_id")
    tool = exact_object(transcript["tool"], TOOL_KEYS, "tool")
    static = exact_object(transcript["static_disasm"], STATIC_KEYS, "static_disasm")
    roots = exact_object(transcript["roots"], set(ROOTS), "roots")
    if transcript["schema"] != TRANSCRIPT_SCHEMA or roots != ROOTS:
        raise GateError("transcript schema/root constants mismatch")
    if transcript["transcript_sha256"] != args.expected_transcript_sha256 or not HEX64.fullmatch(str(args.expected_transcript_sha256)):
        raise GateError("transcript does not match the caller's durable hash pin")
    if not isinstance(capture["session_id"], str) or not re.fullmatch(r"[A-Za-z0-9_.-]{8,128}", capture["session_id"]):
        raise GateError("capture session_id is invalid")
    evidence_nonce = capture["evidence_nonce_hex"]
    if not isinstance(evidence_nonce, str) or not HEX64.fullmatch(evidence_nonce):
        raise GateError("capture evidence nonce is invalid")
    if capture["session_id"] != args.expected_session_id or evidence_nonce != args.expected_evidence_nonce:
        raise GateError("session/evidence nonce does not match caller pins")
    ledger_root = Path(str(capture["nonce_ledger_root"])).resolve()
    if ledger_root != args.nonce_ledger_root.resolve():
        raise GateError("nonce ledger root does not match caller pin")
    created = strict_int(capture["created_filetime_utc"], "capture created time", 1)
    completed = strict_int(capture["completed_filetime_utc"], "capture completed time", 1)
    if completed <= created:
        raise GateError("capture time interval is invalid")
    strict_int(process["pid"], "process pid", 1)
    strict_int(process["creation_filetime_utc"], "process creation time", 1)
    if process["creation_filetime_utc"] >= capture["created_filetime_utc"]:
        raise GateError("evidence session predates process creation")
    exe_path = Path(str(process["exe_path"])).resolve()
    expected_exe = args.expected_exe_sha256.lower()
    if not getattr(args, "allow_fixture_exe", False) and expected_exe != CURRENT_EXE_SHA:
        raise GateError("expected executable pin is not the supported current build")
    if not HEX64.fullmatch(expected_exe) or process["exe_sha256"] != expected_exe or sha256_file(exe_path) != expected_exe:
        raise GateError("process executable SHA-256 mismatch")
    if identity != file_identity(exe_path):
        raise GateError("process executable file identity mismatch")
    for key in FILE_ID_KEYS:
        strict_int(identity[key], f"exe file identity {key}", 0)
    base = hex_address(process["module_base"], "module_base")
    probe_path = Path(str(tool["probe_path"])).resolve()
    if tool["probe_sha256"] != args.expected_probe_tool_sha256 or sha256_file(probe_path) != args.expected_probe_tool_sha256 or not HEX64.fullmatch(args.expected_probe_tool_sha256):
        raise GateError("probe tool hash/pin mismatch")
    static_path = Path(str(static["artifact_path"])).resolve()
    if static["artifact_sha256"] != args.expected_static_disasm_sha256:
        raise GateError("transcript static-disasm pin mismatch")
    validate_static(static_path, args.expected_static_disasm_sha256, expected_exe)
    if transcript["transcript_sha256"] != transcript_digest(transcript) or not HEX64.fullmatch(str(transcript["transcript_sha256"])):
        raise GateError("transcript canonical SHA-256 mismatch")

    nonce_seal_path = ledger_root / f"{evidence_nonce}.consumed_success.json"
    nonce_seal = exact_object(load_object(nonce_seal_path, "nonce outcome seal"), NONCE_SEAL_KEYS, "nonce outcome seal")
    if nonce_seal != {"schema": NONCE_SEAL_SCHEMA, "state": "consumed_success", "session_id": capture["session_id"], "evidence_nonce_hex": evidence_nonce, "transcript_path": str(transcript_path), "transcript_sha256": transcript["transcript_sha256"], "created_filetime_utc": capture["created_filetime_utc"], "consumed_filetime_utc": capture["completed_filetime_utc"]}:
        raise GateError("nonce was not durably consumed for this exact transcript")
    issued_marker_path = ledger_root / f"{evidence_nonce}.issued.json"
    consumed_marker_path = ledger_root / f"{evidence_nonce}.issued.json.consumed"
    failure_seal_path = ledger_root / f"{evidence_nonce}.consumed_failure.json"
    if issued_marker_path.exists() or failure_seal_path.exists():
        raise GateError("nonce ledger retains an active issue or conflicting failure outcome")
    issue = exact_object(load_object(consumed_marker_path, "consumed nonce issue marker"), NONCE_ISSUE_KEYS, "consumed nonce issue marker")
    if issue != {"schema": NONCE_ISSUE_SCHEMA, "state": "issued", "session_id": capture["session_id"], "evidence_nonce_hex": evidence_nonce, "transcript_path": str(transcript_path), "created_filetime_utc": capture["created_filetime_utc"]}:
        raise GateError("consumed nonce issue marker does not bind this exact transcript")

    observations = transcript["observations"]
    if not isinstance(observations, list) or len(observations) < 1:
        raise GateError("transcript requires at least one bracketed observation")
    nonces = {evidence_nonce}
    stable_addresses = None
    all_times: list[int] = []
    stable_reads: list[dict] = []
    for index, observation in enumerate(observations, 1):
        stable_addresses, reads = validate_observation(observation, expected_sequence=index, base=base, stable_addresses=stable_addresses, nonce_set=nonces)
        for read in reads:
            all_times.append(read["observed_filetime_utc"])
            stable_reads.append(read)

    # Melt comparison (owner-pinned, optional): both pins together, or neither.
    seed_pin = args.expected_seed
    id_pin = args.expected_id
    if (seed_pin is None) != (id_pin is None):
        raise GateError("melt comparison requires BOTH --expected-seed and --expected-id, or neither")
    melt_checked = False
    if seed_pin is not None:
        strict_int(seed_pin, "expected-seed pin", I32_MIN, I32_MAX)
        uuid_string(id_pin, "expected-id pin")
        for read in stable_reads:
            if read["game_unique_seed"] != seed_pin or read["game_unique_id"] != id_pin:
                raise GateError("a stable observation does not equal the pinned melted-save seed/id")
        melt_checked = True

    if any(right <= left for left, right in zip(all_times, all_times[1:])):
        raise GateError("observation read times are not globally ordered")
    if all_times[0] < capture["created_filetime_utc"] or all_times[-1] > capture["completed_filetime_utc"]:
        raise GateError("observation times fall outside the capture interval")

    verifier_path = Path(__file__).resolve()
    return {
        "schema": GATE_SCHEMA,
        "verdict": "passed",
        "transcript_path": str(transcript_path),
        "transcript_sha256": transcript["transcript_sha256"],
        "transcript_file_id": file_identity(transcript_path),
        "nonce_seal_path": str(nonce_seal_path),
        "nonce_seal_sha256": sha256_file(nonce_seal_path),
        "verifier_path": str(verifier_path),
        "verifier_sha256": sha256_file(verifier_path),
        "probe_tool_sha256": tool["probe_sha256"],
        "static_disasm_sha256": static["artifact_sha256"],
        "session_id": capture["session_id"],
        "evidence_nonce_hex": evidence_nonce,
        "pid": process["pid"],
        "process_creation_filetime_utc": process["creation_filetime_utc"],
        "exe_sha256": expected_exe,
        "module_base": process["module_base"],
        "observation_count": len(observations),
        "observation_labels": [observation["label"] for observation in observations],
        "melt_comparison_checked": melt_checked,
    }


def _write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def self_test() -> int:
    passed = 0
    with tempfile.TemporaryDirectory(prefix="cid1_verify_v1_") as temp:
        root = Path(temp)
        exe = root / "hoi4.exe"
        probe = root / "probe.py"
        static = root / "static.json"
        transcript_path = root / "transcript.json"
        ledger = root / "nonce_ledger"
        ledger.mkdir()
        exe.write_bytes(b"verified-exe-fixture")
        probe.write_bytes(b"probe-cid1-fixture")
        exe_sha = sha256_file(exe)
        probe_sha = sha256_file(probe)
        static_value = {
            "schema": STATIC_SCHEMA,
            "exe_sha256": exe_sha,
            "module_rvas": {k: ROOTS[k] for k in STATIC_MODULE_KEYS},
            "field_offsets": {k: ROOTS[k] for k in STATIC_OFFSET_KEYS},
            "string_layout": {k: ROOTS[k] for k in STATIC_STRING_LAYOUT_KEYS},
            "identity_grammar": {"uuid_len": UUID_LEN, "max_string_capacity": MAX_STRING_CAPACITY, "hyphen_positions": list(UUID_HYPHEN_POSITIONS), "hex_char_class": UUID_HEX_CHAR_CLASS},
            "evidence_export_sha256": "b" * 64,
        }
        _write_json(static, static_value)
        static_sha = sha256_file(static)
        base = 0x140000000
        gs = 0x200000000
        clock = 0x210000000
        now = 133900000000000000
        seed_value = 1026046527
        uuid_value = "a4b17fed-ed12-4361-9064-fb6576638110"
        region_sha = "c" * 64

        def make_read(label, seq, ordinal_time, *, seed=seed_value, uuid=uuid_value, sha=region_sha, storage="heap", capacity=63, size=UUID_LEN, attempts=2, hour=424242):
            return {
                "read_label": label,
                "observed_filetime_utc": ordinal_time,
                "coherent_attempts": attempts,
                "cgame_state_value": f"0x{gs:x}",
                "clock_slot_a_value": f"0x{clock:x}",
                "clock_slot_b_value": f"0x{clock:x}",
                "clock_vtable_value": f"0x{base + CLOCK_VTABLE_RVA:x}",
                "game_hour_raw": hour,
                "game_unique_seed": seed,
                "game_unique_id": uuid,
                "string_storage": storage,
                "string_size": size,
                "string_capacity": capacity,
                "header_region_sha256": sha,
            }

        def make_observation(seq, label, t_a, t_b):
            return {
                "sequence": seq,
                "label": label,
                "action_nonce_hex": f"{seq:064x}",
                "action_token": f"OBSERVE_CAMPAIGN_IDENTITY_{label}_AND_ACK",
                "addresses": {
                    "cgame_state_slot": f"0x{base + CGAMESTATE_SINGLETON_RVA:x}",
                    "clock_slot_a": f"0x{base + CLOCK_SINGLETON_A_RVA:x}",
                    "clock_slot_b": f"0x{base + CLOCK_SINGLETON_B_RVA:x}",
                    "clock_vtable": f"0x{base + CLOCK_VTABLE_RVA:x}",
                    "cgame_state": f"0x{gs:x}",
                    "clock": f"0x{clock:x}",
                    "current_hour": f"0x{gs + CURRENT_HOUR_OFF:x}",
                    "game_unique_seed": f"0x{gs + GAME_UNIQUE_SEED_OFF:x}",
                    "game_unique_id_string": f"0x{gs + GAME_UNIQUE_ID_OFF:x}",
                },
                "read_a": make_read("A", seq, t_a),
                "read_b": make_read("B", seq, t_b),
            }

        transcript = {
            "schema": TRANSCRIPT_SCHEMA,
            "capture": {"session_id": "cid1_v1_test", "evidence_nonce_hex": "a" * 64, "nonce_ledger_root": str(ledger.resolve()), "created_filetime_utc": now, "completed_filetime_utc": now + 200_000_000},
            "process": {"pid": 4242, "creation_filetime_utc": now - 1_000_000, "exe_path": str(exe.resolve()), "exe_sha256": exe_sha, "exe_file_id": file_identity(exe), "module_base": f"0x{base:x}"},
            "tool": {"probe_path": str(probe.resolve()), "probe_sha256": probe_sha},
            "static_disasm": {"artifact_path": str(static.resolve()), "artifact_sha256": static_sha},
            "roots": ROOTS,
            "observations": [
                make_observation(1, "L0_paused_first_frame", now + 10_000_000, now + 20_000_000),
                make_observation(2, "S1_reload_same_save", now + 30_000_000, now + 40_000_000),
            ],
            "transcript_sha256": "",
        }

        def seal(value):
            value["transcript_sha256"] = transcript_digest(value)
            _write_json(transcript_path, value)
            _write_json(ledger / f'{value["capture"]["evidence_nonce_hex"]}.consumed_success.json', {"schema": NONCE_SEAL_SCHEMA, "state": "consumed_success", "session_id": value["capture"]["session_id"], "evidence_nonce_hex": value["capture"]["evidence_nonce_hex"], "transcript_path": str(transcript_path.resolve()), "transcript_sha256": value["transcript_sha256"], "created_filetime_utc": value["capture"]["created_filetime_utc"], "consumed_filetime_utc": value["capture"]["completed_filetime_utc"]})
            _write_json(ledger / f'{value["capture"]["evidence_nonce_hex"]}.issued.json.consumed', {"schema": NONCE_ISSUE_SCHEMA, "state": "issued", "session_id": value["capture"]["session_id"], "evidence_nonce_hex": value["capture"]["evidence_nonce_hex"], "transcript_path": str(transcript_path.resolve()), "created_filetime_utc": value["capture"]["created_filetime_utc"]})

        seal(transcript)
        base_args = dict(transcript=transcript_path, nonce_ledger_root=ledger, expected_transcript_sha256=transcript["transcript_sha256"], expected_session_id="cid1_v1_test", expected_evidence_nonce="a" * 64, expected_exe_sha256=exe_sha, expected_probe_tool_sha256=probe_sha, expected_static_disasm_sha256=static_sha, out=root / "gate.json", allow_fixture_exe=True, expected_seed=None, expected_id=None)
        verify(SimpleNamespace(**base_args))
        passed += 1
        # Correct melt pins pass.
        verify(SimpleNamespace(**{**base_args, "expected_seed": seed_value, "expected_id": uuid_value}))
        passed += 1

        def low_root_mutation(v):
            # Drop the CGameState root below the 0x10000 sentinel floor, coherently
            # across both observations and their reads (keeps derived/module-relative
            # and A/B checks satisfied so the low-root check is the one that fires).
            low = 0x8000
            for obs in v["observations"]:
                obs["addresses"].update({
                    "cgame_state": f"0x{low:x}",
                    "current_hour": f"0x{low + CURRENT_HOUR_OFF:x}",
                    "game_unique_seed": f"0x{low + GAME_UNIQUE_SEED_OFF:x}",
                    "game_unique_id_string": f"0x{low + GAME_UNIQUE_ID_OFF:x}",
                })
                obs["read_a"].update({"cgame_state_value": f"0x{low:x}"})
                obs["read_b"].update({"cgame_state_value": f"0x{low:x}"})

        mutations = [
            ("extra top-level key", lambda v: v.update({"asserted_pass": True})),
            ("nested extra key", lambda v: v["capture"].update({"verified": True})),
            ("transcript schema bump", lambda v: v.update({"schema": "battle-plan.campaign_identity_evidence_transcript.v2"})),
            ("bad transcript hash", lambda v: v.update({"transcript_sha256": "0" * 64})),
            ("exe binding", lambda v: v["process"].update({"exe_sha256": "0" * 64})),
            ("file identity", lambda v: v["process"]["exe_file_id"].update({"file_index": v["process"]["exe_file_id"]["file_index"] + 1})),
            ("probe hash binding", lambda v: v["tool"].update({"probe_sha256": "0" * 64})),
            ("static hash binding", lambda v: v["static_disasm"].update({"artifact_sha256": "0" * 64})),
            ("process creation order", lambda v: v["process"].update({"creation_filetime_utc": v["capture"]["created_filetime_utc"] + 1})),
            ("session pin", lambda v: v["capture"].update({"session_id": "different_session"})),
            ("nonce pin", lambda v: v["capture"].update({"evidence_nonce_hex": "c" * 64})),
            ("boolean integer pid", lambda v: v["process"].update({"pid": True})),
            ("reused action nonce", lambda v: v["observations"][1].update({"action_nonce_hex": v["observations"][0]["action_nonce_hex"]})),
            ("observation sequence order", lambda v: v["observations"][1].update({"sequence": 5})),
            ("label/token unbind", lambda v: v["observations"][0].update({"label": "other_label"})),
            ("seed out of i32 range", lambda v: (v["observations"][0]["read_a"].update({"game_unique_seed": I32_MAX + 1}), v["observations"][0]["read_b"].update({"game_unique_seed": I32_MAX + 1}))),
            ("uuid grammar uppercase", lambda v: (v["observations"][0]["read_a"].update({"game_unique_id": "A4B17FED-ed12-4361-9064-fb6576638110"}), v["observations"][0]["read_b"].update({"game_unique_id": "A4B17FED-ed12-4361-9064-fb6576638110"}))),
            ("string_size not 36", lambda v: (v["observations"][0]["read_a"].update({"string_size": 35}), v["observations"][0]["read_b"].update({"string_size": 35}))),
            ("capacity below size", lambda v: (v["observations"][0]["read_a"].update({"string_capacity": 10}), v["observations"][0]["read_b"].update({"string_capacity": 10}))),
            ("storage inline for uuid", lambda v: (v["observations"][0]["read_a"].update({"string_storage": "inline", "string_capacity": 63}), v["observations"][0]["read_b"].update({"string_storage": "inline", "string_capacity": 63}))),
            ("A/B seed disagree", lambda v: v["observations"][0]["read_b"].update({"game_unique_seed": seed_value + 1})),
            ("A/B region sha disagree", lambda v: v["observations"][0]["read_b"].update({"header_region_sha256": "d" * 64})),
            ("address drift", lambda v: v["observations"][1]["addresses"].update({"clock": f"0x{clock + 0x1000:x}", "cgame_state": v["observations"][1]["addresses"]["cgame_state"]})),
            ("module-relative slot", lambda v: v["observations"][0]["addresses"].update({"cgame_state_slot": f"0x{base + 1:x}"})),
            ("derived offset", lambda v: v["observations"][0]["addresses"].update({"game_unique_seed": f"0x{gs + 0x721:x}"})),
            ("coherent attempts below two", lambda v: v["observations"][0]["read_a"].update({"coherent_attempts": 1})),
            ("clock vtable sentinel", lambda v: v["observations"][0]["read_a"].update({"clock_vtable_value": f"0x{base + CLOCK_VTABLE_RVA + 8:x}"})),
            ("header sha not hex64", lambda v: (v["observations"][0]["read_a"].update({"header_region_sha256": "zz"}), v["observations"][0]["read_b"].update({"header_region_sha256": "zz"}))),
            ("read time out of order", lambda v: v["observations"][1]["read_a"].update({"observed_filetime_utc": now + 5_000_000})),
            ("time outside capture interval", lambda v: v["observations"][1]["read_b"].update({"observed_filetime_utc": now + 500_000_000})),
            # --- round-2 finding 2: one failing mutation per previously-unguarded check ---
            ("read deny-extra key", lambda v: v["observations"][0]["read_a"].update({"surprise_field": 1})),
            ("read_label swapped", lambda v: v["observations"][0]["read_a"].update({"read_label": "B"})),
            ("observed_filetime non-positive", lambda v: v["observations"][0]["read_a"].update({"observed_filetime_utc": 0})),
            ("coherent attempts above ceiling", lambda v: v["observations"][0]["read_a"].update({"coherent_attempts": MAX_COHERENT_ATTEMPTS_CEILING + 1})),
            ("cgame_state value drift", lambda v: v["observations"][0]["read_a"].update({"cgame_state_value": f"0x{gs + 0x10:x}"})),
            ("clock slot value drift", lambda v: v["observations"][0]["read_a"].update({"clock_slot_a_value": f"0x{clock + 0x10:x}"})),
            ("game_hour out of i32 range", lambda v: v["observations"][0]["read_a"].update({"game_hour_raw": I32_MAX + 1})),
            ("capacity above ceiling", lambda v: (v["observations"][0]["read_a"].update({"string_capacity": MAX_STRING_CAPACITY + 1}), v["observations"][0]["read_b"].update({"string_capacity": MAX_STRING_CAPACITY + 1}))),
            ("storage unknown value", lambda v: (v["observations"][0]["read_a"].update({"string_storage": "mmap"}), v["observations"][0]["read_b"].update({"string_storage": "mmap"}))),
            ("observation deny-extra key", lambda v: v["observations"][0].update({"surprise_field": 1})),
            ("observation label bad grammar", lambda v: v["observations"][0].update({"label": "x"})),
            ("action nonce bad format", lambda v: v["observations"][0].update({"action_nonce_hex": "nothex"})),
            ("addresses deny-extra key", lambda v: v["observations"][0]["addresses"].update({"surprise_field": "0x1"})),
            ("current_hour derived offset", lambda v: v["observations"][0]["addresses"].update({"current_hour": f"0x{gs + CURRENT_HOUR_OFF + 1:x}"})),
            ("id-string derived offset", lambda v: v["observations"][0]["addresses"].update({"game_unique_id_string": f"0x{gs + GAME_UNIQUE_ID_OFF + 1:x}"})),
            ("roots constant drift", lambda v: v["roots"].update({"header_region_len": "0x29"})),
            ("capture interval inverted", lambda v: v["capture"].update({"completed_filetime_utc": v["capture"]["created_filetime_utc"]})),
            ("ledger root drift", lambda v: v["capture"].update({"nonce_ledger_root": str((root / "elsewhere").resolve())})),
            ("module_base zero", lambda v: v["process"].update({"module_base": "0x0"})),
            ("invalid root below 0x10000", low_root_mutation),
        ]
        for name, mutate in mutations:
            candidate = copy.deepcopy(transcript)
            mutate(candidate)
            if name != "bad transcript hash":
                seal(candidate)
            else:
                _write_json(transcript_path, candidate)
            probe_args = dict(base_args)
            probe_args["expected_transcript_sha256"] = candidate["transcript_sha256"] if name != "bad transcript hash" else "0" * 64
            try:
                verify(SimpleNamespace(**probe_args))
            except GateError:
                passed += 1
            else:
                raise AssertionError(f"mutation unexpectedly passed: {name}")

        # Static-artifact content mutations: one failing mutation per field class
        # (finding 5). Each writes a mutated artifact, re-pins its hash, re-seals,
        # and must be rejected by validate_static.
        def run_static_mutation(name, mutated_static):
            _write_json(static, mutated_static)
            csha = sha256_file(static)
            ts = copy.deepcopy(transcript)
            ts["static_disasm"]["artifact_sha256"] = csha
            seal(ts)
            sargs = dict(base_args)
            sargs["expected_static_disasm_sha256"] = csha
            sargs["expected_transcript_sha256"] = ts["transcript_sha256"]
            try:
                verify(SimpleNamespace(**sargs))
            except GateError:
                return 1
            raise AssertionError(f"static mutation unexpectedly passed: {name}")

        static_mutations = [
            ("static module_rvas", {**static_value, "module_rvas": {**static_value["module_rvas"], "clock_vtable_rva": "0xdead"}}),
            ("static field_offsets", {**static_value, "field_offsets": {**static_value["field_offsets"], "game_unique_seed_off": "0x999"}}),
            ("static string_layout", {**static_value, "string_layout": {**static_value["string_layout"], "sso_heap_threshold": "0x8"}}),
            ("static grammar uuid_len", {**static_value, "identity_grammar": {**static_value["identity_grammar"], "uuid_len": 32}}),
            ("static grammar hyphen_positions", {**static_value, "identity_grammar": {**static_value["identity_grammar"], "hyphen_positions": [8, 13, 18, 24]}}),
            ("static grammar hex_char_class", {**static_value, "identity_grammar": {**static_value["identity_grammar"], "hex_char_class": "0123456789ABCDEF"}}),
            ("static grammar deny-extra", {**static_value, "identity_grammar": {**static_value["identity_grammar"], "extra": 1}}),
            ("static exe binding", {**static_value, "exe_sha256": "0" * 64}),
            ("static schema binding", {**static_value, "schema": "battle-plan.campaign_identity_static_disasm.v2"}),
            ("static export hash", {**static_value, "evidence_export_sha256": "not-a-sha"}),
            ("static extra key (deny-extra)", {**static_value, "surprise_field": 1}),
        ]
        for name, mutated in static_mutations:
            passed += run_static_mutation(name, mutated)
        _write_json(static, static_value)
        seal(transcript)

        # Melt-pin mismatches and single-pin misuse.
        for name, mut in (("wrong seed pin", {"expected_seed": seed_value + 7, "expected_id": uuid_value}), ("wrong id pin", {"expected_seed": seed_value, "expected_id": "00000000-0000-0000-0000-000000000000"}), ("single seed pin", {"expected_seed": seed_value, "expected_id": None}), ("single id pin", {"expected_seed": None, "expected_id": uuid_value})):
            try:
                verify(SimpleNamespace(**{**base_args, **mut}))
            except GateError:
                passed += 1
            else:
                raise AssertionError(f"melt-pin case unexpectedly passed: {name}")

        # Previously-unguarded checks (orchestrator round: one isolating mutation each,
        # closing the last mutation-coverage residual).
        # (a) session_id GRAMMAR is distinct from the caller pin: an invalid-grammar
        #     session_id with a MATCHING pin passes the pin equality check, so only the
        #     grammar check (re.fullmatch on session_id) can reject it.
        bad_session = copy.deepcopy(transcript)
        bad_session["capture"]["session_id"] = "bad!sess"
        seal(bad_session)
        try:
            verify(SimpleNamespace(**{**base_args, "expected_transcript_sha256": bad_session["transcript_sha256"], "expected_session_id": "bad!sess"}))
        except GateError:
            passed += 1
        else:
            raise AssertionError("invalid-grammar session_id unexpectedly passed")
        seal(transcript)
        base_args["expected_transcript_sha256"] = transcript["transcript_sha256"]
        # (b) 16 MiB input bound: pad the VALID sealed transcript past 16 MiB with
        #     trailing whitespace (json.loads still parses it, and the sealed nonce
        #     path binding still matches), so ONLY the size guard can reject it. A
        #     buffer of non-JSON bytes would also trip the parser, which would not
        #     isolate this check.
        sealed_bytes = transcript_path.read_bytes()
        transcript_path.write_bytes(sealed_bytes + b" " * (16 * 1024 * 1024 + 1 - len(sealed_bytes)))
        try:
            verify(SimpleNamespace(**base_args))
        except GateError:
            passed += 1
        else:
            raise AssertionError("oversized transcript unexpectedly passed")
        seal(transcript)
        base_args["expected_transcript_sha256"] = transcript["transcript_sha256"]
        # (c) supported-current-build pin: with allow_fixture_exe False the fixture exe
        #     sha (which is not the shipped build sha) must be refused at that gate.
        try:
            verify(SimpleNamespace(**{**base_args, "allow_fixture_exe": False}))
        except GateError:
            passed += 1
        else:
            raise AssertionError("non-current build pin unexpectedly passed")

        # Nonce-ledger tampering cases.
        seal(transcript)
        base_args["expected_transcript_sha256"] = transcript["transcript_sha256"]
        issue_path = ledger / ("a" * 64 + ".issued.json")
        _write_json(issue_path, {"schema": NONCE_ISSUE_SCHEMA})
        try:
            verify(SimpleNamespace(**base_args))
        except GateError:
            passed += 1
        else:
            raise AssertionError("active issued nonce unexpectedly passed")
        issue_path.unlink()
        failure_path = ledger / ("a" * 64 + ".consumed_failure.json")
        _write_json(failure_path, {"schema": NONCE_SEAL_SCHEMA})
        try:
            verify(SimpleNamespace(**base_args))
        except GateError:
            passed += 1
        else:
            raise AssertionError("conflicting nonce outcome unexpectedly passed")
        failure_path.unlink()
        marker_path = ledger / ("a" * 64 + ".issued.json.consumed")
        marker_path.unlink()
        try:
            verify(SimpleNamespace(**base_args))
        except GateError:
            passed += 1
        else:
            raise AssertionError("missing consumed nonce issue marker unexpectedly passed")
        seal(transcript)

        # Success nonce-seal binding VALUES and deny-extra (finding 5).
        seal_path = ledger / ("a" * 64 + ".consumed_success.json")
        good_seal = json.loads(seal_path.read_text(encoding="utf-8"))
        for name, mutated_seal in (
            ("nonce-seal value binding", {**good_seal, "created_filetime_utc": good_seal["created_filetime_utc"] + 1}),
            ("nonce-seal extra key (deny-extra)", {**good_seal, "tampered": True}),
        ):
            _write_json(seal_path, mutated_seal)
            try:
                verify(SimpleNamespace(**base_args))
            except GateError:
                passed += 1
            else:
                raise AssertionError(f"nonce-seal mutation unexpectedly passed: {name}")
        seal(transcript)

        # Consumed-marker binding VALUES and deny-extra (finding 5).
        marker_bind_path = ledger / ("a" * 64 + ".issued.json.consumed")
        good_marker = json.loads(marker_bind_path.read_text(encoding="utf-8"))
        for name, mutated_marker in (
            ("consumed-marker value binding", {**good_marker, "session_id": "some_other_session"}),
            ("consumed-marker extra key (deny-extra)", {**good_marker, "tampered": True}),
        ):
            _write_json(marker_bind_path, mutated_marker)
            try:
                verify(SimpleNamespace(**base_args))
            except GateError:
                passed += 1
            else:
                raise AssertionError(f"consumed-marker mutation unexpectedly passed: {name}")
        seal(transcript)

        # Duplicate JSON key.
        transcript_path.write_text('{"schema":"x","schema":"y"}', encoding="utf-8")
        base_args["expected_transcript_sha256"] = "0" * 64
        try:
            verify(SimpleNamespace(**base_args))
        except GateError:
            passed += 1
        else:
            raise AssertionError("duplicate JSON key unexpectedly passed")
    print(f"campaign-identity evidence v1 self-test: passed={passed} failed=0")
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--transcript", type=Path)
    parser.add_argument("--nonce-ledger-root", type=Path)
    parser.add_argument("--expected-transcript-sha256")
    parser.add_argument("--expected-session-id")
    parser.add_argument("--expected-evidence-nonce")
    parser.add_argument("--expected-exe-sha256")
    parser.add_argument("--expected-probe-tool-sha256")
    parser.add_argument("--expected-static-disasm-sha256")
    parser.add_argument("--expected-seed", type=int, default=None)
    parser.add_argument("--expected-id", default=None)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    if any(getattr(args, name) is None for name in ("transcript", "nonce_ledger_root", "expected_transcript_sha256", "expected_session_id", "expected_evidence_nonce", "expected_exe_sha256", "expected_probe_tool_sha256", "expected_static_disasm_sha256", "out")):
        parser.error("all verification inputs are required")
    try:
        report = verify(args)
    except (GateError, OSError) as exc:
        print(f"campaign-identity evidence v1: FAIL: {exc}", file=sys.stderr)
        return 1
    _write_json(args.out, report)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
