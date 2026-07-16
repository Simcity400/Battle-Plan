# Occupation replay certification

This standalone extractor-side tool certifies a lane proof run before packaging
it as `native_session.battleplan`. It does not read or modify the replay layer.

The input directory must contain the exact `summary.json` and
`collector_stream.bin` emitted by the occupation/resistance/compliance proof.
Certification fails closed unless:

- `summary.json` is duplicate-key-free and binds the exact hashed `live_probe.py`
  emitter, its source schema/path, domain 52, canonical stream metadata, exact
  stream length, and pinned `870ebf...c65ad` executable SHA-256;
- process evidence identifies a nonzero PID, `hoi4.exe` module and absolute
  module path, 64-bit reader, canonical 64-KiB-aligned base, and exact
  `58,531,840`-byte image size;
- the emitted offset/vtable table exactly matches the complete pinned table and
  live-root evidence passes the lane's pointer, coverage, and zero-invalid-row
  constraints;
- every H4CR envelope passes the current canonical compact framing and record
  semantics used by the D44 compatibility and D52 gates;
- sequences and game hours are contiguous, every frame ends canonically, and
  `FRAME_END` counts match independently observed `DOMAIN_PERF` records;
- every frame has exactly one domain-52 PERF and DATA record with matching
  status, `FRAME_END` status exactly equals the OR of every observed PERF
  status, cardinalities are authoritative, and PERF/FRAME elapsed time matches;
- all status bits are known and no global fatal status is present; and
- the packed container has exactly the expected index, timeline, raw stream,
  source summary, and verification-manifest entries, each hash-checked and
  byte-compared after strict reopening.

The tool durably writes `replay_verification_manifest.json` and
`native_session.battleplan` through a unique same-directory `create_new`
temporary, file sync, and atomic write-through replacement. A write or replace
failure removes only the temporary and preserves the prior output. It never
creates or overwrites `native_session_manifest.json`; that name belongs to
recorder-host output.

The verification manifest binds SHA-256 identities for the exact source
summary, compact stream, executable, emitting `live_probe.py`, current D44/D52
gate and compact sources, and this verifier source. Certification booleans are
computed from those checks. Any false source/build/process/geometry/offset/root
verdict stops before publication as explicitly non-certifying. A synthetic
stream plus an arbitrary well-formed build hash is insufficient. The
`stream_authority` records frame count, record count, first/last game hour, game
hours advanced, first/last capture elapsed time, certified duration, per-frame
cardinalities, domain read duration, status, and decoded units.

Run from this directory:

```powershell
cargo run --offline -- --run-dir <lane-proof-run> --game-id orc52
```

Run the adversarial and exact-container tests with:

```powershell
cargo test --offline
```

Historical proof streams that start record sequences at zero no longer
certify. Current canonical streams start at sequence one; accepting a missing
prefix would defeat exact stream identity and ordering authority.
