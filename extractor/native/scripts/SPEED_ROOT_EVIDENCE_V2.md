# D57 speed-root evidence v2

`speed_root_gate.v1` is not production authority. Its two proof booleans can be
self-asserted without any observation transcript. Production consumers must
reject v1 and admit only a gate emitted by `verify_speed_root_evidence_v2.py`.

## Files and schemas

- `speed_root_evidence_v2.schema.json`: exact transcript JSON Schema.
- `collect_speed_root_evidence_v2.py`: read-only observation collector. It never
  changes speed/pause, clicks UI, or emits a verdict.
- `verify_speed_root_evidence_v2.py`: independent deterministic verifier.
- Transcript schema: `battle-plan.speed_root_evidence_transcript.v2`.
- Static artifact schema: `battle-plan.speed_root_static_disasm.v1`.
- Gate schema: `battle-plan.speed_root_gate.v2`.
- Nonce outcome schema: `battle-plan.speed_root_nonce_seal.v2`.

Every object is deny-extra. JSON duplicate keys, malformed UTF-8, inputs over
16 MiB, wrong scalar types, and unknown/missing fields fail closed.

## Trusted pins

The transcript self-hash is SHA-256 of UTF-8 JSON canonicalized with sorted
keys and `(',', ':')` separators after removing `transcript_sha256`. It detects
accidental mutation but is not authority by itself.

Before verification, the orchestrator must durably pin:

- canonical transcript digest (`transcript_sha256`);
- raw transcript-file SHA-256 (kept separately because JSON formatting bytes are
  intentionally not the canonical digest input);
- session ID and fresh caller-issued 32-byte nonce;
- current executable SHA-256;
- collector tool SHA-256;
- static-disassembly artifact SHA-256.

The verifier receives the canonical digest and semantic pins independently
through required CLI options. The safe driver separately verifies the raw-file
pin before admission and again after retaining the exact transcript bytes.
`collect_speed_root_evidence_v2.py init` requires `--nonce-ledger-root`. Before
it opens the process, it exclusively creates and flushes
`<nonce>.issued.json`. Every later command inherits that absolute ledger path
from the transcript. Success consumes the issue into
`<nonce>.consumed_success.json`; any command failure consumes it into
`<nonce>.consumed_failure.json`. The issued name is then unavailable for reuse.
The verifier accepts only the exact success seal for the exact transcript path,
hash, session, nonce, and creation/completion FILETIMEs.

The current supported executable pin is:

`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`

The independent verification call is structurally:

```text
python verify_speed_root_evidence_v2.py \
  --transcript <absolute transcript.json> \
  --nonce-ledger-root <absolute nonce ledger> \
  --expected-transcript-sha256 <caller pin> \
  --expected-session-id <caller pin> \
  --expected-evidence-nonce <caller pin> \
  --expected-exe-sha256 <caller pin> \
  --expected-probe-tool-sha256 <caller pin> \
  --expected-static-disasm-sha256 <caller pin> \
  --out <new gate.json>
```

The production driver additionally requires the gate PID, process-creation
FILETIME, executable hash, and module base to match the capture target before
launching the recorder. The production capture session ID is the evidence
session ID; supplying a different capture session is rejected.

## Static-disassembly artifact

The pinned artifact is strict JSON with exactly:

```json
{
  "schema": "battle-plan.speed_root_static_disasm.v1",
  "exe_sha256": "<current executable sha256>",
  "module_rvas": {
    "cgame_state_singleton_rva": "0x3316e90",
    "clock_singleton_a_rva": "0x33172e0",
    "clock_singleton_b_rva": "0x33172e8",
    "clock_vtable_rva": "0x2952510"
  },
  "field_offsets": {
    "tick_progress_offset": "0x4b8",
    "speed_index_offset": "0x4bc",
    "pause_offset": "0x6c1"
  },
  "speed_mapping": {
    "ui_speeds": [1, 2, 3, 4, 5],
    "raw_values": [0, 1, 2, 3, 4]
  },
  "evidence_export_sha256": "<pinned disassembler evidence export sha256>"
}
```

## Explicit collection actions

Collection is deliberately multi-step. Before every command, an orchestrator
or operator must perform and visually check the requested UI action. The tool
only calls `OpenProcess` with query/read rights and `ReadProcessMemory`.

1. Initialize with a caller-issued fresh nonce, an absolute durable nonce-ledger
   root, and the exact token
   `AUTHORIZE_READ_ONLY_SPEED_ROOT_EVIDENCE_INIT`.
2. Set visible UI speeds 1 through 5 in order. After each visible check, record
   with token `SET_VISIBLE_UI_SPEED_N_AND_ACK`.
3. Return to visible UI speed 3, unpaused, and use
   `SET_VISIBLE_UI_SPEED_3_UNPAUSED_AND_ACK`.
4. Pause without changing speed and use
   `PAUSE_AT_VISIBLE_UI_SPEED_3_AND_ACK`.
5. Resume without changing speed and use
   `RESUME_AT_VISIBLE_UI_SPEED_3_AND_ACK`.
6. Seal observations with `SEAL_TRANSCRIPT_WITHOUT_VERDICT`.
7. Run the independent verifier. Sealing never emits or implies a verdict.

The pause phase needs at least three reads spanning one second. Default
collection is three reads separated by 500 ms.

## Deterministic verdict

The verifier emits `battle-plan.speed_root_gate.v2` with
`"verdict":"passed"` only when all rules hold. The gate also binds the original
transcript path/hash/file identity and the consumed nonce-seal path/hash. The
safe production driver copies the raw transcript to
`speed_root_evidence_v2/transcript.json`, records both identities in the plan
and durable attempt marker, and the post-capture verifier independently reruns
the canonical v2 verifier against the original before comparing the retained
bytes.

1. Transcript, process, file identity, paths, executable bytes, probe bytes,
   static artifact bytes, transcript digest, session, nonce, ledger root, and
   exact consumed-success outcome seal match pins.
2. PID is positive; process creation precedes evidence creation; module base and
   all derived singleton/field addresses are stable.
3. Every stability read repeats the same CGameState root and equal clock A/B
   roots; clock vtable equals `module_base + 0x2952510`.
4. Exactly five speed samples appear at sequences 1..5. Requested UI speeds are
   1..5, observed raw values are 0..4, and every read is unpaused.
5. Exactly three pause samples appear at sequences 6..8, all at UI speed 3/raw
   2, with phases `running_before`, `paused`, `running_after`.
6. Pause values are exactly 0,1,0. Paused tick bits are identical for at least
   one second. Running-before and running-after tick bits change. A resumed tick
   differs from the frozen tick within five seconds.
7. Every sample has 3..16 reads, consecutive ordinals, unique action nonces,
   strictly increasing per-sample and global FILETIME observations, and lies
   inside the transcript capture interval.
8. Tick f32 bits decode to finite values in `[0,1]`. NaN, infinity, negative
   values, negative zero-equivalent out-of-contract encodings, and values over
   one fail.

No transcript field named `passed`, `verified`,
`multivalue_cycle_verified`, or `pause_orthogonality_verified` is accepted.
The verdict is derived exclusively by the independent verifier.

Recorder admission is exclusively
`--domain57-game-speed-gate-v2 <absolute gate path>`. The legacy
`--speed-root-gate` flag, v1 gate/control schemas, and v1 proof booleans are not
authorization paths. The normalized collector control is
`battle-plan.domain57_game_speed_control.v2` under
`domain57_game_speed_gate_v2` in both collector control and manifest; the two
objects must be semantic-JSON identical.

Raw transcript-file bytes are a separate transport binding, not an additional
gate/control field. The driver passes
`--domain57-game-speed-transcript-file-sha256 <lowercase sha256>`. The recorder
records the same value at collector-control root and as the
`production_profile.domain57_game_speed_transcript_file_sha256` manifest
sibling. The post-capture verifier requires actual raw bytes, plan, CLI,
collector control, and manifest to agree while preserving the exact 19-key gate
and exact 23-key nested control schemas.

This remains fresh operator-attested paired evidence: without a separately
verified screenshot/RenderDoc artifact, the transcript cannot mechanically
prove which pixels the operator saw. It does prove the exact ordered memory
observations paired with explicit UI-action acknowledgements.
