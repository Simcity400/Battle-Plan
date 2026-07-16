"""Scripted fake-process dry run for the CID1 campaign-identity probe.

No live game: the process/clock/memory are monkeypatched deterministic fakes.
Proves, end to end and with the real collector/verifier code paths:
  * the coherent read retries through a transient tear and hard-fails a
    persistent one (never fabricating a plausible value);
  * both the heap and SSO MSVC std::string decode paths work;
  * a bracketing A/B identity mismatch is refused AND burns the nonce;
  * the collector's sealed output verifies clean through the independent
    verifier, including the optional melt-comparison pins.

Run: python -m pytest test_cid1_probe_dry.py
"""

from __future__ import annotations

import struct
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

SCRIPTS_DIR = Path(__file__).resolve().parent.parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import collect_campaign_identity_evidence as collector  # noqa: E402
import verify_campaign_identity_evidence as verifier  # noqa: E402

BASE = 0x140000000
GS = 0x100000000
CLOCK = 0x200000000
CHARS_ADDR = 0x300000000
UUID = "a4b17fed-ed12-4361-9064-fb6576638110"
SEED = 1026046527


def build_region(seed: int, chars_addr: int = CHARS_ADDR, size: int = collector.UUID_LEN, cap: int = 63) -> bytes:
    region = bytearray(collector.HEADER_REGION_LEN)
    struct.pack_into("<i", region, 0, seed)
    struct.pack_into("<Q", region, collector.STRING_OFF, chars_addr)
    struct.pack_into("<Q", region, collector.STRING_OFF + 0x10, size)
    struct.pack_into("<Q", region, collector.STRING_OFF + 0x18, cap)
    return bytes(region)


class FakeProc:
    """Duck-typed read-only process serving a deterministic CGameState image.

    The game hour is served OBSERVATION-paced to mirror the authority's stable
    read: each identity observation reads the hour twice (read_i32_stable), so a
    per-observation `hour_seq` value is returned for BOTH reads of that
    observation. `hour_mode`:
      * "fixed"   -> a constant hour on every read;
      * "obs_seq" -> hour_seq[k] for the k-th observation (its two stable reads).
    `region_size`/`region_cap` let a test declare a torn/oversized control block.
    All reads are logged in `read_log` and the widest read in `max_read_size`."""

    def __init__(self, exe_path: Path, *, seed: int = SEED, uuid: str = UUID, hour_mode: str = "fixed", hour_value: int = 424242, hour_seq=None, region_size: int = collector.UUID_LEN, region_cap: int = 63):
        self.pid = 4242
        self._exe = exe_path
        self.seed = seed
        self.uuid = uuid
        self.hour_mode = hour_mode
        self.hour_value = hour_value
        self.hour_seq = list(hour_seq or [])
        self.region_size = region_size
        self.region_cap = region_cap
        self._hour_reads = 0
        self.read_log: list[tuple[int, int]] = []
        self.max_read_size = 0

    # context-manager + binding surface used by the collector
    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False

    def close(self):
        pass

    def exe_path(self) -> Path:
        return self._exe.resolve()

    def creation_filetime(self) -> int:
        return 133900000000000000

    def module_base(self) -> int:
        return BASE

    def _hour(self) -> int:
        if self.hour_mode == "obs_seq":
            index = self._hour_reads // 2  # two stable reads per observation
            self._hour_reads += 1
            return self.hour_seq[index] if index < len(self.hour_seq) else self.hour_seq[-1]
        return self.hour_value

    def read(self, address: int, size: int) -> bytes:
        self.read_log.append((address, size))
        self.max_read_size = max(self.max_read_size, size)
        if address == BASE + collector.CGAMESTATE_SINGLETON_RVA and size == 8:
            return struct.pack("<Q", GS)
        if address in (BASE + collector.CLOCK_SINGLETON_A_RVA, BASE + collector.CLOCK_SINGLETON_B_RVA) and size == 8:
            return struct.pack("<Q", CLOCK)
        if address == CLOCK and size == 8:
            return struct.pack("<Q", BASE + collector.CLOCK_VTABLE_RVA)
        if address == GS + collector.GAME_UNIQUE_SEED_OFF and size == collector.HEADER_REGION_LEN:
            return build_region(self.seed, size=self.region_size, cap=self.region_cap)
        if address == GS + collector.CURRENT_HOUR_OFF and size == 4:
            return struct.pack("<i", self._hour())
        if address == CHARS_ADDR and size == self.region_size + 1:
            return self.uuid.encode("ascii") + b"\x00"
        raise collector.ProbeError(f"fake: unmapped read at 0x{address:x} size {size}")

    def u64(self, address: int) -> int:
        return struct.unpack("<Q", self.read(address, 8))[0]


@pytest.fixture
def fake_exe(tmp_path: Path) -> Path:
    exe = tmp_path / "hoi4.exe"
    exe.write_bytes(b"fake-hoi4-image-for-dry-run")
    return exe


# --- coherent read -----------------------------------------------------------

def test_coherent_read_retries_through_transient_tear(fake_exe):
    # Observation 1 hour=100, observation 2 hour=200 (differs), observation 3
    # hour=200 -> two consecutive identical observations -> stabilizes at #3.
    proc = FakeProc(fake_exe, hour_mode="obs_seq", hour_seq=[100, 200, 200])
    snap = collector.read_coherent_identity(proc, BASE, max_attempts=8)
    assert snap["coherent_attempts"] == 3
    assert snap["seed"] == SEED
    assert snap["uuid"] == UUID
    assert snap["storage"] == "heap"


def test_coherent_read_refuses_persistent_tear(fake_exe):
    # Every observation's stable hour differs -> no two consecutive snapshots
    # ever agree -> refused (never fabricated).
    proc = FakeProc(fake_exe, hour_mode="obs_seq", hour_seq=[100, 200, 300, 400, 500, 600])
    with pytest.raises(collector.ProbeError, match="never stabilized"):
        collector.read_coherent_identity(proc, BASE, max_attempts=6)


# --- FINDING 2: hour read is stable (two-equal) AND precedes the header -------

def test_finding2_hour_is_read_before_the_header_region(fake_exe):
    proc = FakeProc(fake_exe, hour_mode="obs_seq", hour_seq=[777, 777])
    collector.read_coherent_identity(proc, BASE, max_attempts=4)
    hour_addr = GS + collector.CURRENT_HOUR_OFF
    header_addr = GS + collector.GAME_UNIQUE_SEED_OFF
    first_hour = next(i for i, (a, _s) in enumerate(proc.read_log) if a == hour_addr)
    first_header = next(i for i, (a, _s) in enumerate(proc.read_log) if a == header_addr)
    assert first_hour < first_header, "authority order is hour-then-header"


def test_finding2_hour_stable_read_rejects_disagreeing_pair():
    class HourProc:
        def __init__(self, values):
            self._values = list(values)
            self._i = 0

        def read(self, address, size):
            v = self._values[self._i]
            self._i += 1
            return struct.pack("<i", v)

    # Two equal reads -> value returned.
    ok, raw = collector.read_i32_stable(HourProc([1234, 1234]), 0x1000)
    assert ok == 1234 and raw == struct.pack("<i", 1234)
    # Disagreeing pair (e.g. hour advanced mid-read) -> refused, not fabricated.
    with pytest.raises(collector.ProbeError, match="stable read disagreed"):
        collector.read_i32_stable(HourProc([100, 999]), 0x1000)


# --- FINDING 1: over-long size is never dereferenced -------------------------

def test_finding1_oversized_string_is_not_dereferenced(fake_exe):
    # Control block claims size=capacity=1MiB. The exact-size gate must reject it
    # before any heap read, so no read wider than the header region is issued.
    proc = FakeProc(fake_exe, region_size=collector.MAX_STRING_CAPACITY, region_cap=collector.MAX_STRING_CAPACITY)
    with pytest.raises(collector.ProbeError):
        collector.read_coherent_identity(proc, BASE, max_attempts=4)
    assert proc.max_read_size <= collector.HEADER_REGION_LEN, "no read wider than the header region may occur"
    assert all(size != collector.MAX_STRING_CAPACITY + 1 for _addr, size in proc.read_log)


# --- MSVC std::string decode: heap and SSO ----------------------------------

def test_heap_string_decode():
    region = build_region(SEED, cap=63)
    raw, size, cap, storage = collector.decode_msvc_std_string(
        region, collector.STRING_OFF,
        lambda addr, length: UUID.encode("ascii") + b"\x00" if addr == CHARS_ADDR and length == collector.UUID_LEN + 1 else pytest.fail("bad heap read"),
        collector.MAX_STRING_CAPACITY, collector.UUID_LEN,
    )
    assert storage == "heap" and size == collector.UUID_LEN and cap == 63
    assert collector.validate_uuid_grammar(raw) == UUID


def test_sso_string_decode():
    text = b"hello-sso"
    region = bytearray(64)
    region[collector.STRING_OFF:collector.STRING_OFF + len(text)] = text
    struct.pack_into("<Q", region, collector.STRING_OFF + 0x10, len(text))
    struct.pack_into("<Q", region, collector.STRING_OFF + 0x18, 15)  # cap < 16 -> SSO
    raw, size, cap, storage = collector.decode_msvc_std_string(
        bytes(region), collector.STRING_OFF,
        lambda *_: pytest.fail("SSO must not read the heap"),
        collector.MAX_STRING_CAPACITY, len(text),
    )
    assert storage == "inline" and raw == text and size == len(text)


def test_decoder_requires_size_argument():
    # required_size is mandatory -- omitting it is a TypeError, so no call path can
    # silently skip the exact-size gate (round-2 finding 1).
    region = build_region(SEED, cap=63)
    with pytest.raises(TypeError):
        collector.decode_msvc_std_string(region, collector.STRING_OFF, lambda *_: b"", collector.MAX_STRING_CAPACITY)


# --- full observe path: bracketing + ledger burn -----------------------------

def _init_transcript(tmp_path, monkeypatch, fake, static_export="e" * 64):
    exe = fake.exe_path()
    monkeypatch.setattr(collector, "EXE_SHA", collector.sha256_file(exe))
    monkeypatch.setattr(collector, "ReadOnlyProcess", lambda pid: fake)
    static = tmp_path / "static.json"
    static.write_text(
        '{\n'
        f'  "schema": "{verifier.STATIC_SCHEMA}",\n'
        f'  "exe_sha256": "{collector.sha256_file(exe)}",\n'
        '  "module_rvas": {"cgame_state_singleton_rva": "0x3316e90", "clock_singleton_a_rva": "0x33172e0", "clock_singleton_b_rva": "0x33172e8", "clock_vtable_rva": "0x2952510"},\n'
        '  "field_offsets": {"current_hour_off": "0x468", "game_unique_seed_off": "0x720", "game_unique_id_off": "0x728", "header_region_len": "0x28"},\n'
        '  "string_layout": {"string_ptr_offset": "0x0", "string_size_offset": "0x10", "string_capacity_offset": "0x18", "sso_heap_threshold": "0x10"},\n'
        '  "identity_grammar": {"uuid_len": 36, "max_string_capacity": 1048576, "hyphen_positions": [8, 13, 18, 23], "hex_char_class": "0123456789abcdef"},\n'
        f'  "evidence_export_sha256": "{static_export}"\n'
        '}\n',
        encoding="utf-8",
    )
    ledger = tmp_path / "ledger"
    transcript = tmp_path / "transcript.json"
    nonce = "a" * 64
    rc = collector.main([
        "init", "--pid", "4242", "--session-id", "cid1_dry_run",
        "--evidence-nonce", nonce, "--nonce-ledger-root", str(ledger),
        "--static-disasm", str(static), "--out", str(transcript),
        "--action-token", "AUTHORIZE_READ_ONLY_CAMPAIGN_IDENTITY_EVIDENCE_INIT",
    ])
    assert rc == 0
    return transcript, ledger, static, nonce


def test_observe_refuses_bracketing_mismatch_and_burns_nonce(tmp_path, monkeypatch, fake_exe):
    fake = FakeProc(fake_exe)
    transcript, ledger, _static, nonce = _init_transcript(tmp_path, monkeypatch, fake)

    # The operator acknowledgement flips the live identity between reads A and B.
    def flipping_ack(_prompt):
        fake.seed = SEED + 1

    monkeypatch.setattr(collector, "_operator_ack", flipping_ack)
    rc = collector.main([
        "observe", "--transcript", str(transcript),
        "--label", "L0_paused_first_frame",
        "--action-token", "OBSERVE_CAMPAIGN_IDENTITY_L0_paused_first_frame_AND_ACK",
    ])
    assert rc == 1  # bracketing reads disagreed -> refused
    burned = ledger / f"{nonce}.consumed_failure.json"
    assert burned.is_file(), "nonce must be burned on a refused observation"
    assert not (ledger / f"{nonce}.consumed_success.json").exists()


def test_finding4_eof_ack_fails_closed_and_burns_nonce(tmp_path, monkeypatch, fake_exe):
    # A closed/redirected stdin (EOF) at the bracket ack must NOT auto-advance to
    # read B; it fails closed and the observation burns the nonce.
    fake = FakeProc(fake_exe)
    transcript, ledger, _static, nonce = _init_transcript(tmp_path, monkeypatch, fake)

    def eof_input(_prompt=""):
        raise EOFError()

    monkeypatch.setattr("builtins.input", eof_input)  # exercise the REAL _operator_ack
    rc = collector.main([
        "observe", "--transcript", str(transcript),
        "--label", "L0_paused_first_frame",
        "--action-token", "OBSERVE_CAMPAIGN_IDENTITY_L0_paused_first_frame_AND_ACK",
    ])
    assert rc == 1  # EOF -> refused, no silent auto-advance
    assert (ledger / f"{nonce}.consumed_failure.json").is_file()
    assert not (ledger / f"{nonce}.consumed_success.json").exists()
    # Nothing was appended to the transcript.
    import json
    assert json.loads(transcript.read_text(encoding="utf-8"))["observations"] == []


def test_stable_observe_seals_and_verifies_clean(tmp_path, monkeypatch, fake_exe):
    fake = FakeProc(fake_exe)
    transcript, ledger, static, nonce = _init_transcript(tmp_path, monkeypatch, fake)
    monkeypatch.setattr(collector, "_operator_ack", lambda _prompt: None)

    assert collector.main([
        "observe", "--transcript", str(transcript),
        "--label", "L0_paused_first_frame",
        "--action-token", "OBSERVE_CAMPAIGN_IDENTITY_L0_paused_first_frame_AND_ACK",
    ]) == 0
    assert collector.main([
        "seal", "--transcript", str(transcript),
        "--action-token", "SEAL_CAMPAIGN_IDENTITY_TRANSCRIPT_WITHOUT_VERDICT",
    ]) == 0
    assert (ledger / f"{nonce}.consumed_success.json").is_file()

    import json
    sealed = json.loads(transcript.read_text(encoding="utf-8"))
    args = SimpleNamespace(
        transcript=transcript, nonce_ledger_root=ledger,
        expected_transcript_sha256=sealed["transcript_sha256"],
        expected_session_id="cid1_dry_run", expected_evidence_nonce=nonce,
        expected_exe_sha256=collector.sha256_file(fake_exe.resolve()),
        expected_probe_tool_sha256=sealed["tool"]["probe_sha256"],
        expected_static_disasm_sha256=verifier.sha256_file(static),
        out=tmp_path / "gate.json", allow_fixture_exe=True,
        expected_seed=None, expected_id=None,
    )
    report = verifier.verify(args)
    assert report["verdict"] == "passed"
    assert report["observation_count"] == 1

    # Melt comparison against the known seed/id passes; a wrong pin fails.
    report_pinned = verifier.verify(SimpleNamespace(**{**vars(args), "expected_seed": SEED, "expected_id": UUID}))
    assert report_pinned["melt_comparison_checked"] is True
    with pytest.raises(verifier.GateError):
        verifier.verify(SimpleNamespace(**{**vars(args), "expected_seed": SEED + 99, "expected_id": UUID}))
