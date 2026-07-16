#!/usr/bin/env python3
"""Deterministic, deny-extra verifier for read-only D57 speed-root transcripts."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import os
import re
import struct
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

TRANSCRIPT_SCHEMA = "battle-plan.speed_root_evidence_transcript.v2"
GATE_SCHEMA = "battle-plan.speed_root_gate.v2"
STATIC_SCHEMA = "battle-plan.speed_root_static_disasm.v1"
CURRENT_EXE_SHA = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
ROOTS = {
    "cgame_state_singleton_rva": "0x3316e90",
    "clock_singleton_a_rva": "0x33172e0",
    "clock_singleton_b_rva": "0x33172e8",
    "clock_vtable_rva": "0x2952510",
    "tick_progress_offset": "0x4b8",
    "speed_index_offset": "0x4bc",
    "pause_offset": "0x6c1",
}
HEX64 = re.compile(r"^[0-9a-f]{64}$")
HEX_ADDRESS = re.compile(r"^0x[0-9A-Fa-f]+$")

ROOT_KEYS = {"schema", "capture", "process", "tool", "static_disasm", "roots", "speed_samples", "pause_samples", "transcript_sha256"}
CAPTURE_KEYS = {"session_id", "evidence_nonce_hex", "nonce_ledger_root", "created_filetime_utc", "completed_filetime_utc"}
PROCESS_KEYS = {"pid", "creation_filetime_utc", "exe_path", "exe_sha256", "exe_file_id", "module_base"}
FILE_ID_KEYS = {"volume_serial_number", "file_index", "file_size"}
TOOL_KEYS = {"probe_path", "probe_sha256"}
STATIC_KEYS = {"artifact_path", "artifact_sha256"}
SAMPLE_KEYS = {"sequence", "action_nonce_hex", "requested_ui_speed", "expected_raw_speed", "phase", "addresses", "reads"}
ADDRESS_KEYS = {"cgame_state_slot", "clock_slot_a", "clock_slot_b", "clock_vtable", "cgame_state", "clock", "speed", "pause", "tick"}
READ_KEYS = {"ordinal", "observed_filetime_utc", "cgame_state_value", "clock_slot_a_value", "clock_slot_b_value", "clock_vtable_value", "speed_raw", "pause_raw", "tick_progress_bits"}
STATIC_ROOT_KEYS = {"schema", "exe_sha256", "module_rvas", "field_offsets", "speed_mapping", "evidence_export_sha256"}
STATIC_MODULE_KEYS = {"cgame_state_singleton_rva", "clock_singleton_a_rva", "clock_singleton_b_rva", "clock_vtable_rva"}
STATIC_OFFSET_KEYS = {"tick_progress_offset", "speed_index_offset", "pause_offset"}
STATIC_MAPPING_KEYS = {"ui_speeds", "raw_values"}
NONCE_SEAL_KEYS = {"schema", "state", "session_id", "evidence_nonce_hex", "transcript_path", "transcript_sha256", "created_filetime_utc", "consumed_filetime_utc"}
NONCE_ISSUE_KEYS = {"schema", "state", "session_id", "evidence_nonce_hex", "transcript_path", "created_filetime_utc"}


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


def strict_int(value, label: str, minimum: int | None = None) -> int:
    if type(value) is not int or (minimum is not None and value < minimum):
        raise GateError(f"{label} is not a canonical integer")
    return value


def file_identity(path: Path) -> dict:
    stat = path.stat()
    return {"volume_serial_number": int(stat.st_dev), "file_index": int(stat.st_ino), "file_size": int(stat.st_size)}


def tick_value(bits: str, label: str) -> float:
    if not isinstance(bits, str) or not re.fullmatch(r"0x[0-9a-fA-F]{8}", bits):
        raise GateError(f"{label} is not an f32 bit pattern")
    raw = int(bits[2:], 16)
    value = struct.unpack("<f", raw.to_bytes(4, "little"))[0]
    if raw == 0x80000000 or not math.isfinite(value) or not 0.0 <= value <= 1.0:
        raise GateError(f"{label} decodes outside finite [0,1]")
    return value


def validate_static(path: Path, expected_sha: str, exe_sha: str) -> dict:
    if not HEX64.fullmatch(expected_sha):
        raise GateError("expected static-disasm hash is not lowercase SHA-256")
    if sha256_file(path) != expected_sha:
        raise GateError("static-disasm artifact hash mismatch")
    value = exact_object(load_object(path, "static disassembly artifact"), STATIC_ROOT_KEYS, "static artifact")
    modules = exact_object(value["module_rvas"], STATIC_MODULE_KEYS, "static module_rvas")
    offsets = exact_object(value["field_offsets"], STATIC_OFFSET_KEYS, "static field_offsets")
    mapping = exact_object(value["speed_mapping"], STATIC_MAPPING_KEYS, "static speed_mapping")
    if value["schema"] != STATIC_SCHEMA or value["exe_sha256"] != exe_sha or not HEX64.fullmatch(str(value["evidence_export_sha256"])):
        raise GateError("static artifact schema/executable/export binding mismatch")
    expected_modules = {key: ROOTS[key] for key in STATIC_MODULE_KEYS}
    expected_offsets = {key: ROOTS[key] for key in STATIC_OFFSET_KEYS}
    if modules != expected_modules or offsets != expected_offsets or mapping != {"ui_speeds": [1,2,3,4,5], "raw_values": [0,1,2,3,4]}:
        raise GateError("static artifact roots/offsets/speed mapping mismatch")
    return value


def validate_sample(sample, *, expected_sequence: int, expected_ui: int, expected_raw: int, expected_phase: str, base: int, stable_addresses: dict | None, nonce_set: set[str]) -> tuple[dict, list[str]]:
    sample = exact_object(sample, SAMPLE_KEYS, f"sample {expected_sequence}")
    if strict_int(sample["sequence"], "sample sequence", 1) != expected_sequence or strict_int(sample["requested_ui_speed"], "requested UI speed", 1) != expected_ui or strict_int(sample["expected_raw_speed"], "expected raw speed", 0) != expected_raw or sample["phase"] != expected_phase:
        raise GateError(f"sample {expected_sequence}: order/UI/raw/phase mismatch")
    nonce = sample["action_nonce_hex"]
    if not isinstance(nonce, str) or not HEX64.fullmatch(nonce) or nonce in nonce_set:
        raise GateError(f"sample {expected_sequence}: invalid/reused action nonce")
    nonce_set.add(nonce)
    addresses = exact_object(sample["addresses"], ADDRESS_KEYS, f"sample {expected_sequence} addresses")
    parsed = {key: hex_address(value, f"sample {expected_sequence} {key}") for key, value in addresses.items()}
    if parsed["speed"] != parsed["cgame_state"] + 0x4BC or parsed["tick"] != parsed["cgame_state"] + 0x4B8 or parsed["pause"] != parsed["clock"] + 0x6C1:
        raise GateError(f"sample {expected_sequence}: derived address mismatch")
    if parsed["cgame_state_slot"] != base + 0x3316E90 or parsed["clock_slot_a"] != base + 0x33172E0 or parsed["clock_slot_b"] != base + 0x33172E8 or parsed["clock_vtable"] != base + 0x2952510:
        raise GateError(f"sample {expected_sequence}: module-relative root address mismatch")
    if parsed["cgame_state"] < 0x10000 or parsed["clock"] < 0x10000 or base <= 0:
        raise GateError(f"sample {expected_sequence}: invalid root address")
    if stable_addresses is not None and addresses != stable_addresses:
        raise GateError(f"sample {expected_sequence}: root addresses changed")
    reads = sample["reads"]
    if not isinstance(reads, list) or len(reads) < 3 or len(reads) > 16:
        raise GateError(f"sample {expected_sequence}: stability read count outside 3..16")
    ticks, times = [], []
    for index, row in enumerate(reads, 1):
        row = exact_object(row, READ_KEYS, f"sample {expected_sequence} read {index}")
        if strict_int(row["ordinal"], "read ordinal", 1) != index or strict_int(row["speed_raw"], "read raw speed", 0) != expected_raw or type(row["pause_raw"]) is not int or row["pause_raw"] not in (0,1):
            raise GateError(f"sample {expected_sequence} read {index}: ordinal/speed/pause mismatch")
        if hex_address(row["cgame_state_value"], f"sample {expected_sequence} read {index} cgame_state") != parsed["cgame_state"] or hex_address(row["clock_slot_a_value"], f"sample {expected_sequence} read {index} clock A") != parsed["clock"] or hex_address(row["clock_slot_b_value"], f"sample {expected_sequence} read {index} clock B") != parsed["clock"] or hex_address(row["clock_vtable_value"], f"sample {expected_sequence} read {index} clock vtable") != parsed["clock_vtable"]:
            raise GateError(f"sample {expected_sequence} read {index}: root stability/sentinel mismatch")
        times.append(strict_int(row["observed_filetime_utc"], f"sample {expected_sequence} read {index} time", 1))
        tick_value(row["tick_progress_bits"], f"sample {expected_sequence} read {index} tick")
        ticks.append(row["tick_progress_bits"].lower())
    if any(right <= left for left, right in zip(times, times[1:])):
        raise GateError(f"sample {expected_sequence}: read times are not strictly increasing")
    return addresses, ticks


def verify(args) -> dict:
    transcript_path = args.transcript.resolve()
    transcript = exact_object(load_object(transcript_path, "speed-root transcript"), ROOT_KEYS, "transcript")
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
    created = strict_int(capture["created_filetime_utc"], "capture created time", 1); completed = strict_int(capture["completed_filetime_utc"], "capture completed time", 1)
    if completed <= created:
        raise GateError("capture time interval is invalid")
    strict_int(process["pid"], "process pid", 1); strict_int(process["creation_filetime_utc"], "process creation time", 1)
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
    for key in FILE_ID_KEYS: strict_int(identity[key], f"exe file identity {key}", 0)
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
    if nonce_seal != {"schema":"battle-plan.speed_root_nonce_seal.v2","state":"consumed_success","session_id":capture["session_id"],"evidence_nonce_hex":evidence_nonce,"transcript_path":str(transcript_path),"transcript_sha256":transcript["transcript_sha256"],"created_filetime_utc":capture["created_filetime_utc"],"consumed_filetime_utc":capture["completed_filetime_utc"]}:
        raise GateError("nonce was not durably consumed for this exact transcript")
    issued_path = ledger_root / f"{evidence_nonce}.issued.json"
    consumed_marker_path = ledger_root / f"{evidence_nonce}.issued.json.consumed"
    failure_seal_path = ledger_root / f"{evidence_nonce}.consumed_failure.json"
    if issued_path.exists() or failure_seal_path.exists():
        raise GateError("nonce ledger retains an active issue or conflicting failure outcome")
    issue = exact_object(load_object(consumed_marker_path, "consumed nonce issue marker"), NONCE_ISSUE_KEYS, "consumed nonce issue marker")
    if issue != {"schema":"battle-plan.speed_root_nonce_issue.v2","state":"issued","session_id":capture["session_id"],"evidence_nonce_hex":evidence_nonce,"transcript_path":str(transcript_path),"created_filetime_utc":capture["created_filetime_utc"]}:
        raise GateError("consumed nonce issue marker does not bind this exact transcript")

    speed_samples, pause_samples = transcript["speed_samples"], transcript["pause_samples"]
    if not isinstance(speed_samples, list) or len(speed_samples) != 5 or not isinstance(pause_samples, list) or len(pause_samples) != 3:
        raise GateError("transcript requires exactly five speed and three pause samples")
    nonces = {evidence_nonce}; stable_addresses = None; all_times = []
    for index, sample in enumerate(speed_samples, 1):
        stable_addresses, _ticks = validate_sample(sample, expected_sequence=index, expected_ui=index, expected_raw=index-1, expected_phase="speed_cycle", base=base, stable_addresses=stable_addresses, nonce_set=nonces)
        all_times.extend(row["observed_filetime_utc"] for row in sample["reads"])
        if any(row["pause_raw"] != 0 for row in sample["reads"]):
            raise GateError(f"speed sample {index} was paused")
    phase_specs = [("running_before",0),("paused",1),("running_after",0)]
    for offset, (phase, pause_raw) in enumerate(phase_specs, 6):
        stable_addresses, ticks = validate_sample(pause_samples[offset-6], expected_sequence=offset, expected_ui=3, expected_raw=2, expected_phase=phase, base=base, stable_addresses=stable_addresses, nonce_set=nonces)
        rows = pause_samples[offset-6]["reads"]
        if any(row["pause_raw"] != pause_raw for row in rows):
            raise GateError(f"pause phase {phase} has wrong pause value")
        if phase == "paused" and len(set(ticks)) != 1:
            raise GateError("tick progress changed while paused")
        if phase == "paused" and rows[-1]["observed_filetime_utc"] - rows[0]["observed_filetime_utc"] < 10_000_000:
            raise GateError("paused stability evidence spans less than one second")
        if phase != "paused" and len(set(ticks)) < 2:
            raise GateError(f"tick progress did not resume/change during {phase}")
        all_times.extend(row["observed_filetime_utc"] for row in rows)
    paused_end = pause_samples[1]["reads"][-1]["observed_filetime_utc"]
    resumed = pause_samples[2]["reads"]
    frozen_tick = pause_samples[1]["reads"][-1]["tick_progress_bits"].lower()
    resumed_change = next((row for row in resumed if row["tick_progress_bits"].lower() != frozen_tick), None)
    if resumed_change is None or resumed_change["observed_filetime_utc"] - paused_end > 50_000_000:
        raise GateError("tick progress did not resume within five seconds")
    if any(right <= left for left, right in zip(all_times, all_times[1:])):
        raise GateError("sample observations are not globally ordered")
    if all_times[0] < capture["created_filetime_utc"] or all_times[-1] > capture["completed_filetime_utc"]:
        raise GateError("sample times fall outside capture interval")

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
        "speed_sample_sequences": [1,2,3,4,5],
        "pause_sample_sequences": [6,7,8],
    }


def _write_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def self_test() -> int:
    passed = 0
    with tempfile.TemporaryDirectory(prefix="speed_root_v2_") as temp:
        root = Path(temp); exe=root/"hoi4.exe"; probe=root/"probe.py"; static=root/"static.json"; transcript_path=root/"transcript.json"; ledger=root/"nonce_ledger";ledger.mkdir()
        exe.write_bytes(b"verified-exe-fixture"); probe.write_bytes(b"probe-v2-fixture")
        exe_sha=sha256_file(exe); probe_sha=sha256_file(probe)
        static_value={"schema":STATIC_SCHEMA,"exe_sha256":exe_sha,"module_rvas":{k:ROOTS[k] for k in STATIC_MODULE_KEYS},"field_offsets":{k:ROOTS[k] for k in STATIC_OFFSET_KEYS},"speed_mapping":{"ui_speeds":[1,2,3,4,5],"raw_values":[0,1,2,3,4]},"evidence_export_sha256":"b"*64}
        _write_json(static,static_value); static_sha=sha256_file(static)
        base=0x140000000; gs=0x200000000; clock=0x210000000; now=133900000000000000
        def sample(seq,ui,raw,phase,pause,ticks):
            return {"sequence":seq,"action_nonce_hex":f"{seq:064x}","requested_ui_speed":ui,"expected_raw_speed":raw,"phase":phase,"addresses":{"cgame_state_slot":f"0x{base+0x3316E90:X}","clock_slot_a":f"0x{base+0x33172E0:X}","clock_slot_b":f"0x{base+0x33172E8:X}","clock_vtable":f"0x{base+0x2952510:X}","cgame_state":f"0x{gs:X}","clock":f"0x{clock:X}","speed":f"0x{gs+0x4BC:X}","pause":f"0x{clock+0x6C1:X}","tick":f"0x{gs+0x4B8:X}"},"reads":[{"ordinal":i+1,"observed_filetime_utc":now+seq*100000+i*1000,"cgame_state_value":f"0x{gs:X}","clock_slot_a_value":f"0x{clock:X}","clock_slot_b_value":f"0x{clock:X}","clock_vtable_value":f"0x{base+0x2952510:X}","speed_raw":raw,"pause_raw":pause,"tick_progress_bits":f"0x{bits:08x}"} for i,bits in enumerate(ticks)]}
        moving=[0x3DCCCCCD,0x3E4CCCCD,0x3E99999A]; frozen=[0x3E99999A]*3
        transcript={"schema":TRANSCRIPT_SCHEMA,"capture":{"session_id":"session_v2_test","evidence_nonce_hex":"a"*64,"nonce_ledger_root":str(ledger.resolve()),"created_filetime_utc":now,"completed_filetime_utc":now+900000},"process":{"pid":123,"creation_filetime_utc":now-1000000,"exe_path":str(exe.resolve()),"exe_sha256":exe_sha,"exe_file_id":file_identity(exe),"module_base":f"0x{base:X}"},"tool":{"probe_path":str(probe.resolve()),"probe_sha256":probe_sha},"static_disasm":{"artifact_path":str(static.resolve()),"artifact_sha256":static_sha},"roots":ROOTS,"speed_samples":[sample(i,i,i-1,"speed_cycle",0,moving) for i in range(1,6)],"pause_samples":[sample(6,3,2,"running_before",0,moving),sample(7,3,2,"paused",1,frozen),sample(8,3,2,"running_after",0,moving)],"transcript_sha256":""}
        for fixture_sample in transcript["speed_samples"] + transcript["pause_samples"]:
            for fixture_read in fixture_sample["reads"]:
                fixture_read["observed_filetime_utc"] = now + fixture_sample["sequence"] * 20_000_000 + (fixture_read["ordinal"] - 1) * 5_000_000
        transcript["capture"]["completed_filetime_utc"] = now + 200_000_000
        def seal(value):
            value["transcript_sha256"]=transcript_digest(value); _write_json(transcript_path,value)
            _write_json(ledger/f'{value["capture"]["evidence_nonce_hex"]}.consumed_success.json',{"schema":"battle-plan.speed_root_nonce_seal.v2","state":"consumed_success","session_id":value["capture"]["session_id"],"evidence_nonce_hex":value["capture"]["evidence_nonce_hex"],"transcript_path":str(transcript_path.resolve()),"transcript_sha256":value["transcript_sha256"],"created_filetime_utc":value["capture"]["created_filetime_utc"],"consumed_filetime_utc":value["capture"]["completed_filetime_utc"]})
            _write_json(ledger/f'{value["capture"]["evidence_nonce_hex"]}.issued.json.consumed',{"schema":"battle-plan.speed_root_nonce_issue.v2","state":"issued","session_id":value["capture"]["session_id"],"evidence_nonce_hex":value["capture"]["evidence_nonce_hex"],"transcript_path":str(transcript_path.resolve()),"created_filetime_utc":value["capture"]["created_filetime_utc"]})
        seal(transcript)
        args=SimpleNamespace(transcript=transcript_path,nonce_ledger_root=ledger,expected_transcript_sha256=transcript["transcript_sha256"],expected_session_id="session_v2_test",expected_evidence_nonce="a"*64,expected_exe_sha256=exe_sha,expected_probe_tool_sha256=probe_sha,expected_static_disasm_sha256=static_sha,out=root/"gate.json",allow_fixture_exe=True)
        verify(args); passed+=1
        mutations=[
            ("extra key",lambda v:v.update({"asserted_pass":True})),
            ("nested extra key",lambda v:v["capture"].update({"verified":True})),
            ("v1 schema",lambda v:v.update({"schema":"battle-plan.speed_root_gate.v1"})),
            ("bad transcript hash",lambda v:v.update({"transcript_sha256":"0"*64})),
            ("exe binding",lambda v:v["process"].update({"exe_sha256":"0"*64})),
            ("file identity",lambda v:v["process"]["exe_file_id"].update({"file_index":0})),
            ("probe hash binding",lambda v:v["tool"].update({"probe_sha256":"0"*64})),
            ("static hash binding",lambda v:v["static_disasm"].update({"artifact_sha256":"0"*64})),
            ("process creation order",lambda v:v["process"].update({"creation_filetime_utc":v["capture"]["created_filetime_utc"]+1})),
            ("session pin",lambda v:v["capture"].update({"session_id":"different_session"})),
            ("nonce pin",lambda v:v["capture"].update({"evidence_nonce_hex":"c"*64})),
            ("boolean integer",lambda v:v["process"].update({"pid":True})),
            ("reused nonce",lambda v:v["speed_samples"][1].update({"action_nonce_hex":v["speed_samples"][0]["action_nonce_hex"]})),
            ("speed order",lambda v:v["speed_samples"][1].update({"requested_ui_speed":5})),
            ("speed raw",lambda v:v["speed_samples"][2]["reads"][0].update({"speed_raw":1})),
            ("address drift",lambda v:v["pause_samples"][0]["addresses"].update({"clock":"0x220000000"})),
            ("module-relative slot",lambda v:v["speed_samples"][0]["addresses"].update({"cgame_state_slot":"0x140000001"})),
            ("pause pattern",lambda v:v["pause_samples"][1]["reads"][0].update({"pause_raw":0})),
            ("paused tick moved",lambda v:v["pause_samples"][1]["reads"][2].update({"tick_progress_bits":"0x3f000000"})),
            ("tick NaN",lambda v:v["speed_samples"][0]["reads"][0].update({"tick_progress_bits":"0x7fc00000"})),
            ("tick negative zero",lambda v:v["speed_samples"][0]["reads"][0].update({"tick_progress_bits":"0x80000000"})),
            ("paused dwell short",lambda v:v["pause_samples"][1]["reads"][2].update({"observed_filetime_utc":v["pause_samples"][1]["reads"][0]["observed_filetime_utc"]+1000})),
            ("too few stability reads",lambda v:v["speed_samples"][0].update({"reads":v["speed_samples"][0]["reads"][:2]})),
            ("resumed tick frozen",lambda v:v["pause_samples"][2].update({"reads":copy.deepcopy(v["pause_samples"][1]["reads"])})),
            ("global time order",lambda v:v["pause_samples"][0]["reads"][0].update({"observed_filetime_utc":now+1000})),
        ]
        for name, mutate in mutations:
            candidate=copy.deepcopy(transcript); mutate(candidate)
            if name!="bad transcript hash": seal(candidate)
            else: _write_json(transcript_path,candidate)
            args.expected_transcript_sha256=candidate["transcript_sha256"]
            try: verify(args)
            except GateError: passed+=1
            else: raise AssertionError(f"mutation unexpectedly passed: {name}")
        seal(transcript);args.expected_transcript_sha256=transcript["transcript_sha256"]
        issue_path=ledger/("a"*64+".issued.json");_write_json(issue_path,{"schema":"battle-plan.speed_root_nonce_issue.v2"})
        try:verify(args)
        except GateError:passed+=1
        else:raise AssertionError("active issued nonce unexpectedly passed")
        issue_path.unlink()
        failure_path=ledger/("a"*64+".consumed_failure.json");_write_json(failure_path,{"schema":"battle-plan.speed_root_nonce_seal.v2"})
        try:verify(args)
        except GateError:passed+=1
        else:raise AssertionError("conflicting nonce outcome unexpectedly passed")
        failure_path.unlink()
        marker_path=ledger/("a"*64+".issued.json.consumed");marker_path.unlink()
        try:verify(args)
        except GateError:passed+=1
        else:raise AssertionError("missing consumed nonce issue marker unexpectedly passed")
        seal(transcript)
        transcript_path.write_text('{"schema":"x","schema":"y"}',encoding="utf-8")
        args.expected_transcript_sha256="0"*64
        try:verify(args)
        except GateError:passed+=1
        else:raise AssertionError("duplicate JSON key unexpectedly passed")
    print(f"speed-root evidence v2 self-test: passed={passed} failed=0")
    return 0


def main(argv=None) -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test",action="store_true")
    parser.add_argument("--transcript",type=Path)
    parser.add_argument("--nonce-ledger-root",type=Path)
    parser.add_argument("--expected-transcript-sha256")
    parser.add_argument("--expected-session-id")
    parser.add_argument("--expected-evidence-nonce")
    parser.add_argument("--expected-exe-sha256")
    parser.add_argument("--expected-probe-tool-sha256")
    parser.add_argument("--expected-static-disasm-sha256")
    parser.add_argument("--out",type=Path)
    args=parser.parse_args(argv)
    if args.self_test:return self_test()
    if any(getattr(args,name) is None for name in ("transcript","nonce_ledger_root","expected_transcript_sha256","expected_session_id","expected_evidence_nonce","expected_exe_sha256","expected_probe_tool_sha256","expected_static_disasm_sha256","out")):parser.error("all verification inputs are required")
    try: report=verify(args)
    except (GateError,OSError) as exc:
        print(f"speed-root evidence v2: FAIL: {exc}",file=sys.stderr);return 1
    _write_json(args.out,report);print(json.dumps(report,indent=2,sort_keys=True));return 0


if __name__=="__main__":raise SystemExit(main())
