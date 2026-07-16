#!/usr/bin/env python3
"""Fail-closed cross-check of capture-bound Speed5 evidence against H4CR d57 rows."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import struct
import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace

import zstandard

H4CR = b"H4CR"
ZSTD = b"\x28\xb5\x2f\xfd"
HEADER = struct.Struct("<4sHHQiHHII")
DOMAIN57 = 57
DOMAIN_DATA = 3
FRAME_END = 2
S57_HEADER_LEN = 64
S57_EVENT_LEN = 64
ROOT_GAME_SPEED = 1 << 2
GATE_KEYS = {
    "schema", "verdict", "transcript_path", "transcript_sha256", "transcript_file_id",
    "nonce_seal_path", "nonce_seal_sha256", "verifier_path", "verifier_sha256",
    "probe_tool_sha256", "static_disasm_sha256", "session_id", "evidence_nonce_hex",
    "pid", "process_creation_filetime_utc", "exe_sha256", "module_base",
    "speed_sample_sequences", "pause_sample_sequences",
}
CONTROL_GATE_KEYS = {
    "schema", "verdict", "required", "enabled", "gate_path", "gate_sha256",
    "transcript_path", "transcript_sha256", "transcript_file_id", "nonce_seal_path",
    "nonce_seal_sha256", "verifier_path", "verifier_sha256", "probe_tool_sha256",
    "static_disasm_sha256", "session_id", "evidence_nonce_hex", "pid",
    "process_creation_filetime_utc", "exe_sha256", "module_base",
    "speed_sample_sequences", "pause_sample_sequences",
}
GATE_BINDING_KEYS = GATE_KEYS - {"schema", "verdict"}
PROOF_KEYS = {
    "schema", "session_id", "pid", "exe_path", "exe_sha256",
    "domain57_game_speed_gate_v2_sha256", "capture_plan_sha256", "monitor_sha256",
    "collector_control_sha256", "sample_count", "speed_raw", "first_game_hour_raw",
    "last_game_hour_raw", "capture_first_game_hour_raw", "capture_last_game_hour_raw",
}


class GateError(ValueError):
    pass


def load_object(path: Path, label: str) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        raise GateError(f"{label} is unreadable: {exc}") from exc
    if not isinstance(value, dict):
        raise GateError(f"{label} is not a JSON object")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def require_hex64(value, label: str) -> str:
    normalized = str(value)
    if len(normalized) != 64 or normalized.lower() != normalized or any(ch not in "0123456789abcdef" for ch in normalized):
        raise GateError(f"{label} is not a lowercase SHA-256 value")
    return normalized


def rerun_evidence_verifier(args, gate: dict, plan: dict) -> dict:
    verifier = args.evidence_verifier.resolve()
    if sha256_file(verifier) != require_hex64(args.expected_verifier_sha256, "expected verifier hash"):
        raise GateError("canonical evidence verifier bytes differ from the out-of-band pin")
    if Path(str(gate.get("verifier_path", ""))).resolve() != verifier or gate.get("verifier_sha256") != args.expected_verifier_sha256:
        raise GateError("stored gate is not bound to the canonical evidence verifier")
    expected_transcript_sha = require_hex64(plan.get("speed_evidence_transcript_sha256"), "plan transcript hash")
    command = [
        sys.executable, str(verifier), "--transcript", str(args.evidence_transcript.resolve()),
        "--nonce-ledger-root", str(args.nonce_ledger_root.resolve()),
        "--expected-transcript-sha256", expected_transcript_sha,
        "--expected-session-id", str(plan.get("speed_evidence_session_id", "")),
        "--expected-evidence-nonce", str(plan.get("speed_evidence_nonce", "")),
        "--expected-exe-sha256", require_hex64(gate.get("exe_sha256"), "gate executable hash"),
        "--expected-probe-tool-sha256", require_hex64(args.expected_probe_sha256, "expected probe hash"),
        "--expected-static-disasm-sha256", require_hex64(args.expected_static_sha256, "expected static disassembly hash"),
    ]
    with tempfile.TemporaryDirectory(prefix="speed_root_reverify_") as temp:
        rerun_path = Path(temp) / "gate.json"
        result = subprocess.run(command + ["--out", str(rerun_path)], capture_output=True, text=True, timeout=120, check=False)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            raise GateError(f"independent v2 evidence re-verification failed: {detail}")
        rerun_gate = load_object(rerun_path, "independently regenerated speed-root gate")
    if rerun_gate != gate:
        raise GateError("stored speed-root gate differs from independently regenerated v2 gate")
    return rerun_gate


def read_exact(reader, size: int) -> bytes:
    result = bytearray()
    while len(result) < size:
        block = reader.read(size - len(result))
        if not block:
            raise GateError(f"stream truncated: need {size}, got {len(result)} bytes")
        result.extend(block)
    return bytes(result)


def skip_exact(reader, size: int) -> None:
    remaining = size
    while remaining:
        block = reader.read(min(remaining, 1024 * 1024))
        if not block:
            raise GateError(f"stream truncated while skipping {size} bytes")
        remaining -= len(block)


def open_stream(path: Path):
    raw = path.open("rb")
    magic = raw.read(4)
    raw.seek(0)
    if magic == ZSTD:
        return raw, zstandard.ZstdDecompressor(max_window_size=1 << 27).stream_reader(raw)
    return raw, raw


def parse_d57(payload: bytes, header_hour: int) -> tuple[int, int]:
    if len(payload) < S57_HEADER_LEN or payload[:4] != b"S57M":
        raise GateError(f"d57 hour {header_hour}: malformed payload header")
    version, header_len, domain_id, player_len, event_len = struct.unpack_from("<HHHHH", payload, 4)
    if (version, header_len, domain_id, player_len, event_len) != (1, 64, 57, 40, 64):
        raise GateError(f"d57 hour {header_hour}: unexpected payload contract")
    payload_hour = struct.unpack_from("<i", payload, 16)[0]
    players, events, strings = struct.unpack_from("<III", payload, 28)
    root_mask = struct.unpack_from("<I", payload, 44)[0]
    if payload_hour != header_hour:
        raise GateError(f"d57 payload/header hour mismatch {payload_hour}!={header_hour}")
    if root_mask & ROOT_GAME_SPEED == 0:
        raise GateError(f"d57 hour {header_hour}: game-speed root evidence absent")
    events_start = header_len + players * player_len
    expected_len = events_start + events * event_len + strings
    if expected_len != len(payload):
        raise GateError(f"d57 hour {header_hour}: payload length mismatch")
    speed_rows = []
    pause_rows = []
    for index in range(events):
        row = events_start + index * event_len
        kind = struct.unpack_from("<H", payload, row)[0]
        value = struct.unpack_from("<i", payload, row + 32)[0]
        event_hour = struct.unpack_from("<i", payload, row + 52)[0]
        if kind == 3:
            speed_rows.append((value, event_hour))
        elif kind == 4:
            pause_rows.append((value, event_hour))
    if speed_rows != [(4, header_hour)]:
        raise GateError(f"d57 hour {header_hour}: expected exactly one Speed5 row, got {speed_rows}")
    if pause_rows != [(0, header_hour)]:
        raise GateError(f"d57 hour {header_hour}: expected exactly one unpaused row, got {pause_rows}")
    return speed_rows[0][0], pause_rows[0][0]


def scan_stream(path: Path) -> tuple[list[int], dict[int, int]]:
    physical, reader = open_stream(path)
    frame_hours: list[int] = []
    d57_counts: dict[int, int] = {}
    last_sequence = None
    try:
        while True:
            prefix = reader.read(1)
            if not prefix:
                break
            header = prefix + read_exact(reader, HEADER.size - 1)
            magic, version, header_len, sequence, hour, domain, record_type, payload_len, _flags = HEADER.unpack(header)
            if magic != H4CR or version != 1 or header_len != HEADER.size:
                raise GateError("invalid H4CR framing")
            if payload_len > (1 << 28):
                raise GateError(f"record payload exceeds bound: {payload_len}")
            if last_sequence is not None and sequence != last_sequence + 1:
                raise GateError(f"sequence gap: {last_sequence}->{sequence}")
            last_sequence = sequence
            if domain == DOMAIN57 and record_type == DOMAIN_DATA:
                if payload_len > 16 * 1024 * 1024:
                    raise GateError("d57 payload exceeds 16 MiB validation bound")
                parse_d57(read_exact(reader, payload_len), hour)
                d57_counts[hour] = d57_counts.get(hour, 0) + 1
            else:
                skip_exact(reader, payload_len)
            if record_type == FRAME_END:
                frame_hours.append(hour)
    finally:
        if reader is not physical:
            reader.close()
        physical.close()
    if not frame_hours:
        raise GateError("stream has no completed frames")
    if any(b != a + 1 for a, b in zip(frame_hours, frame_hours[1:])):
        raise GateError("completed frame hours are not strictly consecutive")
    if any(d57_counts.get(hour) != 1 for hour in frame_hours):
        missing = [hour for hour in frame_hours if d57_counts.get(hour) != 1]
        raise GateError(f"d57 is not exactly once per completed frame: {missing[:8]}")
    extras = sorted(set(d57_counts) - set(frame_hours))
    if extras:
        raise GateError(f"d57 rows exist outside completed frames: {extras[:8]}")
    return frame_hours, d57_counts


def load_monitor(path: Path) -> list[dict]:
    rows = []
    try:
        for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
            value = json.loads(line)
            if not isinstance(value, dict):
                raise GateError(f"monitor line {number} is not an object")
            rows.append(value)
    except (OSError, json.JSONDecodeError) as exc:
        raise GateError(f"monitor log is invalid: {exc}") from exc
    if len(rows) < 3:
        raise GateError("monitor log has fewer than three samples")
    for index, row in enumerate(rows):
        if row.get("speed_raw") != 4 or row.get("paused") is not False:
            raise GateError(f"monitor sample {index} is not unpaused Speed5")
        if not isinstance(row.get("game_hour_raw"), int):
            raise GateError(f"monitor sample {index} has no integer game hour")
    hours = [row["game_hour_raw"] for row in rows]
    if any(b < a for a, b in zip(hours, hours[1:])) or hours[-1] <= hours[0]:
        raise GateError("monitor game-hour progression is absent or regressed")
    return rows


def verify(args) -> dict:
    manifest = load_object(args.manifest, "capture manifest")
    proof = load_object(args.speed_proof, "speed proof")
    plan = load_object(args.capture_plan, "capture plan")
    gate = load_object(args.root_gate, "speed-root gate")
    control = load_object(args.collector_control, "collector control")
    monitor = load_monitor(args.monitor)
    frames, _ = scan_stream(args.stream)
    raw_transcript_sha = sha256_file(args.evidence_transcript)

    if set(proof) != PROOF_KEYS or proof.get("schema") != "battle-plan.speed5_proof.v2":
        raise GateError("unsupported speed proof schema")
    if plan.get("schema") != "battle-plan.production_capture_plan.v1":
        raise GateError("unsupported capture plan schema")
    if set(gate) != GATE_KEYS or gate.get("schema") != "battle-plan.speed_root_gate.v2" or gate.get("verdict") != "passed":
        raise GateError("speed-root gate is not an exact passed v2 object")
    if gate.get("speed_sample_sequences") != [1, 2, 3, 4, 5] or gate.get("pause_sample_sequences") != [6, 7, 8]:
        raise GateError("speed-root gate sample sequences are noncanonical")
    if "domain57_game_speed_gate" in control:
        raise GateError("legacy domain57_game_speed_gate control must not coexist")
    control_gate = control.get("domain57_game_speed_gate_v2")
    if not isinstance(control_gate, dict) or set(control_gate) != CONTROL_GATE_KEYS:
        raise GateError("collector control has missing/extra d57 v2 gate fields")
    expected_control = {
        "schema": "battle-plan.domain57_game_speed_control.v2", "verdict": "passed",
        "required": True, "enabled": True, "gate_path": str(args.root_gate.resolve()),
        "gate_sha256": sha256_file(args.root_gate),
        **{key: gate[key] for key in GATE_BINDING_KEYS},
    }
    try:
        control_gate_path = Path(str(control_gate["gate_path"]))
        if not control_gate_path.is_absolute() or str(control_gate_path.resolve()) != str(control_gate["gate_path"]):
            raise GateError("collector control root-gate path is not canonical absolute")
    except OSError as exc:
        raise GateError(f"collector control root-gate path is invalid: {exc}") from exc
    if control_gate != expected_control:
        raise GateError("collector control d57 v2 gate binding/metadata mismatch")
    profile = manifest.get("production_profile", {})
    if not isinstance(profile, dict) or "domain57_game_speed_gate" in profile:
        raise GateError("legacy manifest d57 gate must not coexist")
    manifest_gate = profile.get("domain57_game_speed_gate_v2")
    if manifest_gate != control_gate:
        raise GateError("manifest d57 v2 gate is not semantic-JSON-identical to collector control")
    raw_transcript_pin = require_hex64(plan.get("speed_evidence_transcript_file_sha256"), "plan raw transcript-file hash")
    if raw_transcript_sha != raw_transcript_pin:
        raise GateError("actual raw transcript bytes differ from the capture-plan pin")
    if control.get("domain57_game_speed_transcript_file_sha256") != raw_transcript_pin:
        raise GateError("collector control raw transcript-file hash differs from actual bytes/plan")
    if profile.get("domain57_game_speed_transcript_file_sha256") != raw_transcript_pin:
        raise GateError("manifest raw transcript-file hash differs from actual bytes/plan/control")
    rerun_evidence_verifier(args, gate, plan)
    if args.evidence_transcript.resolve() != Path(str(gate["transcript_path"])).resolve():
        raise GateError("original transcript path differs from the admitted gate")
    if plan.get("speed_evidence_transcript_sha256") != gate["transcript_sha256"]:
        raise GateError("capture plan canonical transcript digest differs from the admitted gate")
    retained = Path(str(plan.get("retained_speed_evidence_transcript", "")))
    expected_retained = args.stream.resolve().parent / "speed_root_evidence_v2" / "transcript.json"
    if retained.resolve() != expected_retained.resolve() or not retained.is_file():
        raise GateError("capture plan does not bind the canonical retained raw transcript")
    if retained.read_bytes() != args.evidence_transcript.read_bytes():
        raise GateError("retained raw transcript bytes differ from the independently verified original")
    if Path(str(plan.get("speed_evidence_nonce_ledger_root", ""))).resolve() != args.nonce_ledger_root.resolve():
        raise GateError("capture plan nonce-ledger root differs from the independent caller pin")
    if plan.get("speed_evidence_transcript_path") != str(args.evidence_transcript.resolve()):
        raise GateError("capture plan transcript path differs from the independent caller pin")
    if plan.get("speed_evidence_transcript_file_id") != gate["transcript_file_id"]:
        raise GateError("capture plan transcript file identity differs from the admitted gate")
    if plan.get("speed_evidence_nonce_seal_path") != gate["nonce_seal_path"] or plan.get("speed_evidence_nonce_seal_sha256") != gate["nonce_seal_sha256"]:
        raise GateError("capture plan nonce outcome seal differs from the admitted gate")
    if proof.get("capture_plan_sha256") != sha256_file(args.capture_plan):
        raise GateError("speed proof is not bound to capture plan bytes")
    if plan.get("domain57_game_speed_gate_v2_sha256") != sha256_file(args.root_gate):
        raise GateError("capture plan is not bound to speed-root gate bytes")
    recorder_args = plan.get("recorder_arguments")
    if not isinstance(recorder_args, list) or recorder_args.count("--domain57-game-speed-gate-v2") != 1 or "--speed-root-gate" in recorder_args:
        raise GateError("capture plan does not contain exactly one exclusive d57 v2 gate argument")
    gate_arg = recorder_args.index("--domain57-game-speed-gate-v2")
    if gate_arg + 1 >= len(recorder_args) or Path(str(recorder_args[gate_arg + 1])).resolve() != args.root_gate.resolve():
        raise GateError("capture plan speed-root-gate argument mismatch")
    raw_flag = "--domain57-game-speed-transcript-file-sha256"
    if recorder_args.count(raw_flag) != 1:
        raise GateError("capture plan does not contain exactly one raw transcript-file hash argument")
    raw_arg = recorder_args.index(raw_flag)
    if raw_arg + 1 >= len(recorder_args) or recorder_args[raw_arg + 1] != raw_transcript_pin:
        raise GateError("capture plan raw transcript-file hash argument mismatch")
    if proof.get("domain57_game_speed_gate_v2_sha256") != sha256_file(args.root_gate):
        raise GateError("speed proof is not bound to speed-root gate bytes")
    if proof.get("monitor_sha256") != sha256_file(args.monitor):
        raise GateError("speed proof is not bound to monitor bytes")
    if proof.get("collector_control_sha256") != sha256_file(args.collector_control):
        raise GateError("speed proof is not bound to collector control bytes")
    for field in ("session_id", "pid"):
        if proof.get(field) != manifest.get(field) or proof.get(field) != plan.get(field):
            raise GateError(f"speed proof {field} is not capture-bound")
    if proof.get("pid") != gate.get("pid"):
        raise GateError("speed-root evidence PID is not the captured process")
    exe_sha = str(proof.get("exe_sha256", "")).lower()
    if exe_sha != str(gate.get("exe_sha256", "")).lower() or exe_sha != str(plan.get("exe_sha256", "")).lower():
        raise GateError("speed proof executable hash binding mismatch")
    if proof.get("sample_count") != len(monitor):
        raise GateError("speed proof sample count mismatch")
    if proof.get("first_game_hour_raw") != monitor[0]["game_hour_raw"] or proof.get("last_game_hour_raw") != monitor[-1]["game_hour_raw"]:
        raise GateError("speed proof monitor interval mismatch")
    if proof.get("capture_first_game_hour_raw") != frames[0] or proof.get("capture_last_game_hour_raw") != frames[-1]:
        raise GateError("speed proof capture interval mismatch")
    if manifest.get("first_game_hour_raw") != frames[0] or manifest.get("last_game_hour_raw") != frames[-1]:
        raise GateError("manifest capture interval differs from stream")
    monitor_hours = [row["game_hour_raw"] for row in monitor]
    if min(monitor_hours) > frames[0] or max(monitor_hours) < frames[-1]:
        raise GateError("monitor interval does not cover all completed frames")
    return {
        "schema": "battle-plan.speed5_stream_gate.v2",
        "passed": True,
        "completed_frames": len(frames),
        "first_game_hour_raw": frames[0],
        "last_game_hour_raw": frames[-1],
        "monitor_samples": len(monitor),
        "domain57_game_speed_gate_v2_sha256": sha256_file(args.root_gate),
        "speed_evidence_transcript_sha256": gate["transcript_sha256"],
        "speed_evidence_transcript_file_sha256": raw_transcript_sha,
        "nonce_seal_sha256": gate["nonce_seal_sha256"],
        "collector_control_sha256": sha256_file(args.collector_control),
    }


def _record(sequence: int, hour: int, domain: int, record_type: int, payload: bytes) -> bytes:
    return HEADER.pack(H4CR, 1, HEADER.size, sequence, hour, domain, record_type, len(payload), 0) + payload


def _d57(hour: int, speed: int = 4, pause: int = 0) -> bytes:
    payload = bytearray(S57_HEADER_LEN + 2 * S57_EVENT_LEN)
    payload[:4] = b"S57M"
    struct.pack_into("<HHHHH", payload, 4, 1, 64, 57, 40, 64)
    struct.pack_into("<i", payload, 16, hour)
    struct.pack_into("<III", payload, 28, 0, 2, 0)
    struct.pack_into("<I", payload, 44, ROOT_GAME_SPEED)
    for index, (kind, value) in enumerate(((3, speed), (4, pause))):
        row = S57_HEADER_LEN + index * S57_EVENT_LEN
        struct.pack_into("<H", payload, row, kind)
        struct.pack_into("<i", payload, row + 32, value)
        struct.pack_into("<i", payload, row + 52, hour)
    return bytes(payload)


def self_test() -> int:
    passed = 0
    with tempfile.TemporaryDirectory(prefix="speed5_stream_gate_") as temp:
        root = Path(temp)
        paths = SimpleNamespace(
            stream=root / "collector_stream.bin", manifest=root / "native_session_manifest.json",
            speed_proof=root / "speed5_proof.json", monitor=root / "speed5_monitor.jsonl",
            capture_plan=root / "production_capture_plan.json", root_gate=root / "speed_root_gate.json",
            collector_control=root / "collector_control.json", evidence_transcript=root / "transcript.json",
            nonce_ledger_root=root / "nonce_ledger", evidence_verifier=root / "fixture_verifier.py",
            out=root / "report.json",
        )
        paths.nonce_ledger_root.mkdir()
        retained = root / "speed_root_evidence_v2" / "transcript.json"
        retained.parent.mkdir()
        transcript_bytes = b'{"fixture":"raw evidence transcript"}\n'
        paths.evidence_transcript.write_bytes(transcript_bytes); retained.write_bytes(transcript_bytes)
        verifier_source = """#!/usr/bin/env python3
import argparse
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--out',required=True);a,_=p.parse_known_args()
Path(a.out).write_bytes(Path(__file__).with_name('fixture_gate.json').read_bytes())
"""
        paths.evidence_verifier.write_text(verifier_source, encoding="utf-8")
        verifier_sha = sha256_file(paths.evidence_verifier)
        exe_sha = "a" * 64; probe_sha = "b" * 64; static_sha = "c" * 64; seal_sha = "d" * 64
        transcript_file_sha = sha256_file(paths.evidence_transcript); transcript_digest_sha = "9" * 64
        gate = {
            "schema":"battle-plan.speed_root_gate.v2","verdict":"passed",
            "transcript_path":str(paths.evidence_transcript.resolve()),"transcript_sha256":transcript_digest_sha,
            "transcript_file_id":{"volume_serial_number":1,"file_index":2,"file_size":len(transcript_bytes)},
            "nonce_seal_path":str((paths.nonce_ledger_root/("e"*64+".consumed_success.json")).resolve()),
            "nonce_seal_sha256":seal_sha,"verifier_path":str(paths.evidence_verifier.resolve()),
            "verifier_sha256":verifier_sha,"probe_tool_sha256":probe_sha,"static_disasm_sha256":static_sha,
            "session_id":"test_session","evidence_nonce_hex":"e"*64,"pid":123,
            "process_creation_filetime_utc":100,"exe_sha256":exe_sha,"module_base":"0x140000000",
            "speed_sample_sequences":[1,2,3,4,5],"pause_sample_sequences":[6,7,8],
        }
        fixture_gate = root / "fixture_gate.json"
        fixture_gate.write_text(json.dumps(gate, sort_keys=True), encoding="utf-8")
        monitor = [{"speed_raw":4,"paused":False,"game_hour_raw":hour} for hour in (9,10,11)]
        paths.root_gate.write_text(json.dumps(gate, sort_keys=True), encoding="utf-8")
        gate_sha = sha256_file(paths.root_gate)
        control_gate = {
            "schema":"battle-plan.domain57_game_speed_control.v2","verdict":"passed","required":True,"enabled":True,
            "gate_path":str(paths.root_gate.resolve()),"gate_sha256":gate_sha,
            **{key: gate[key] for key in GATE_BINDING_KEYS},
        }
        plan = {
            "schema":"battle-plan.production_capture_plan.v1","session_id":"test_session","pid":123,"exe_sha256":exe_sha,
            "domain57_game_speed_gate_v2_sha256":gate_sha,"recorder_arguments":["--domain57-game-speed-gate-v2",str(paths.root_gate.resolve()),"--domain57-game-speed-transcript-file-sha256",transcript_file_sha],
            "speed_evidence_transcript_sha256":transcript_digest_sha,"speed_evidence_session_id":"test_session","speed_evidence_nonce":"e"*64,
            "speed_evidence_transcript_file_sha256":transcript_file_sha,
            "speed_evidence_nonce_ledger_root":str(paths.nonce_ledger_root.resolve()),"speed_evidence_transcript_path":str(paths.evidence_transcript.resolve()),
            "speed_evidence_transcript_file_id":gate["transcript_file_id"],"retained_speed_evidence_transcript":str(retained.resolve()),
            "speed_evidence_nonce_seal_path":gate["nonce_seal_path"],"speed_evidence_nonce_seal_sha256":seal_sha,
        }
        manifest = {"session_id":"test_session","pid":123,"first_game_hour_raw":10,"last_game_hour_raw":11,"production_profile":{"domain57_game_speed_gate_v2":control_gate,"domain57_game_speed_transcript_file_sha256":transcript_file_sha}}
        paths.monitor.write_text("".join(json.dumps(row)+"\n" for row in monitor), encoding="utf-8")
        valid_stream = b"".join((_record(0,10,57,DOMAIN_DATA,_d57(10)),_record(1,10,0,FRAME_END,b""),_record(2,11,57,DOMAIN_DATA,_d57(11)),_record(3,11,0,FRAME_END,b"")))
        paths.stream.write_bytes(valid_stream)

        def restore() -> tuple[dict, dict]:
            paths.evidence_verifier.write_text(verifier_source, encoding="utf-8")
            paths.evidence_transcript.write_bytes(transcript_bytes); retained.write_bytes(transcript_bytes)
            fixture_gate.write_text(json.dumps(gate, sort_keys=True), encoding="utf-8")
            paths.root_gate.write_text(json.dumps(gate, sort_keys=True), encoding="utf-8")
            paths.capture_plan.write_text(json.dumps(plan), encoding="utf-8")
            paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":control_gate,"domain57_game_speed_transcript_file_sha256":transcript_file_sha}), encoding="utf-8")
            paths.manifest.write_text(json.dumps(manifest), encoding="utf-8")
            paths.monitor.write_text("".join(json.dumps(row)+"\n" for row in monitor), encoding="utf-8")
            paths.stream.write_bytes(valid_stream)
            proof = {
                "schema":"battle-plan.speed5_proof.v2","session_id":"test_session","pid":123,"exe_path":"C:/fixture/hoi4.exe","exe_sha256":exe_sha,
                "capture_plan_sha256":sha256_file(paths.capture_plan),"domain57_game_speed_gate_v2_sha256":gate_sha,
                "monitor_sha256":sha256_file(paths.monitor),"sample_count":3,"speed_raw":4,
                "collector_control_sha256":sha256_file(paths.collector_control),
                "first_game_hour_raw":9,"last_game_hour_raw":11,"capture_first_game_hour_raw":10,"capture_last_game_hour_raw":11,
            }
            paths.speed_proof.write_text(json.dumps(proof), encoding="utf-8")
            paths.expected_verifier_sha256=verifier_sha;paths.expected_probe_sha256=probe_sha;paths.expected_static_sha256=static_sha
            return proof, plan

        def write_plan_args(recorder_arguments: list[str]) -> None:
            candidate = {**plan, "recorder_arguments": recorder_arguments}
            paths.capture_plan.write_text(json.dumps(candidate), encoding="utf-8")
            candidate_proof = load_object(paths.speed_proof, "fixture speed proof")
            candidate_proof["capture_plan_sha256"] = sha256_file(paths.capture_plan)
            paths.speed_proof.write_text(json.dumps(candidate_proof), encoding="utf-8")

        proof, _ = restore()
        verify(paths); passed += 1
        mutations = [
            ("dummy stream", lambda: paths.stream.write_bytes(b"stream")),
            ("non-Speed5 d57", lambda: paths.stream.write_bytes(b"".join((_record(0,10,57,DOMAIN_DATA,_d57(10,3)),_record(1,10,0,FRAME_END,b""))))),
            ("missing d57", lambda: paths.stream.write_bytes(_record(0,10,0,FRAME_END,b""))),
            ("unbound plan", lambda: paths.capture_plan.write_text(json.dumps({**plan,"pid":999}), encoding="utf-8")),
            ("no monitor progression", lambda: paths.monitor.write_text("".join(json.dumps({"speed_raw":4,"paused":False,"game_hour_raw":10})+"\n" for _ in range(3)), encoding="utf-8")),
            ("control metadata mismatch", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":{**control_gate,"gate_sha256":"f"*64}}), encoding="utf-8")),
            ("control gate missing", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll"}), encoding="utf-8")),
            ("control gate extra field", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":{**control_gate,"unexpected":True}}), encoding="utf-8")),
            ("manifest/control mismatch", lambda: paths.manifest.write_text(json.dumps({**manifest,"production_profile":{"domain57_game_speed_gate_v2":{**control_gate,"enabled":False}}}), encoding="utf-8")),
            ("legacy control coexistence", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":control_gate,"domain57_game_speed_transcript_file_sha256":transcript_file_sha,"domain57_game_speed_gate":{}}), encoding="utf-8")),
            ("raw control hash missing", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":control_gate}), encoding="utf-8")),
            ("raw control hash mismatch", lambda: paths.collector_control.write_text(json.dumps({"mode":"hourly_poll","domain57_game_speed_gate_v2":control_gate,"domain57_game_speed_transcript_file_sha256":"f"*64}), encoding="utf-8")),
            ("raw manifest hash missing", lambda: paths.manifest.write_text(json.dumps({**manifest,"production_profile":{"domain57_game_speed_gate_v2":control_gate}}), encoding="utf-8")),
            ("raw manifest hash mismatch", lambda: paths.manifest.write_text(json.dumps({**manifest,"production_profile":{"domain57_game_speed_gate_v2":control_gate,"domain57_game_speed_transcript_file_sha256":"f"*64}}), encoding="utf-8")),
            ("raw CLI missing", lambda: write_plan_args(["--domain57-game-speed-gate-v2",str(paths.root_gate.resolve())])),
            ("raw CLI mismatch", lambda: write_plan_args(["--domain57-game-speed-gate-v2",str(paths.root_gate.resolve()),"--domain57-game-speed-transcript-file-sha256","f"*64])),
            ("raw CLI duplicate", lambda: write_plan_args(["--domain57-game-speed-gate-v2",str(paths.root_gate.resolve()),"--domain57-game-speed-transcript-file-sha256",transcript_file_sha,"--domain57-game-speed-transcript-file-sha256",transcript_file_sha])),
            ("gate extra field", lambda: paths.root_gate.write_text(json.dumps({**gate,"unexpected":True}), encoding="utf-8")),
            ("legacy proof coexistence", lambda: paths.speed_proof.write_text(json.dumps({**proof,"root_gate_sha256":gate_sha}), encoding="utf-8")),
            ("retained transcript mismatch", lambda: retained.write_bytes(b"different")),
            ("original transcript mismatch", lambda: paths.evidence_transcript.write_bytes(b"different")),
            ("verifier byte mismatch", lambda: paths.evidence_verifier.write_text(verifier_source+"# changed\n", encoding="utf-8")),
            ("nonce ledger plan mismatch", lambda: paths.capture_plan.write_text(json.dumps({**plan,"speed_evidence_nonce_ledger_root":str(root)}), encoding="utf-8")),
        ]
        for name, mutate in mutations:
            restore()
            mutate()
            try: verify(paths)
            except GateError: passed += 1
            else: raise AssertionError(f"mutation unexpectedly passed: {name}")
    print(f"speed5 stream gate self-test: passed={passed} failed=0")
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--stream", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--speed-proof", type=Path)
    parser.add_argument("--monitor", type=Path)
    parser.add_argument("--capture-plan", type=Path)
    parser.add_argument("--root-gate", type=Path)
    parser.add_argument("--collector-control", type=Path)
    parser.add_argument("--evidence-transcript", type=Path)
    parser.add_argument("--nonce-ledger-root", type=Path)
    parser.add_argument("--evidence-verifier", type=Path)
    parser.add_argument("--expected-verifier-sha256")
    parser.add_argument("--expected-probe-sha256")
    parser.add_argument("--expected-static-sha256")
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    if any(getattr(args, field) is None for field in ("stream","manifest","speed_proof","monitor","capture_plan","root_gate","collector_control","evidence_transcript","nonce_ledger_root","evidence_verifier","expected_verifier_sha256","expected_probe_sha256","expected_static_sha256","out")):
        parser.error("all capture inputs and --out are required unless --self-test is used")
    try:
        report = verify(args)
    except (GateError, OSError) as exc:
        print(f"speed5 stream gate: FAIL: {exc}", file=sys.stderr)
        return 1
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
