# replay_native — native Rust/wgpu replay UI

The native, read-only, game-faithful replay front-end (see
`../replay_ui/GAME_UI_REPLICA_PLAN.md` v3 + `../replay_ui/VERTICAL_SLICE_SPEC.md`).
Pure Rust + `wgpu`; consumes the on-disk snapshot store produced by the Python
capture/ingest pipeline. **Read-only**: it never reads live memory and never writes the
game or a save; it only reads `../replay/snapshots/`.

## Status — M0 (data contract)

`snapshot/` — a forward-compatible store reader. It types only the stable top-level
header (`game_id`/`date`/`hour_index`/...) and keeps every domain payload as raw JSON,
so it never breaks when the schema grows (old snapshots stay readable). The
no-fabrication contract is structural: `country_path` returns `None` for an absent key
and `Some(Null)` for a captured null — measured-zero, null, and absent stay distinct.

```
cargo run -p snapshot --bin m0_demo                 # defaults: ../replay/snapshots, game "replay", GER, state 1
cargo run -p snapshot --bin m0_demo -- <root> <game> <hour> <tag> <state>
cargo test -p snapshot                              # golden fixture: null != 0 != absent
```

## Portable recordings — `.battleplan`

`snapshot/` also reads a **portable recording**: a single `.battleplan` file (a ZIP
container) holding the entire snapshot series + the mod identity + the build hash +
an integrity manifest, so a recording made on one machine opens on another exactly
like a save game. Spec: `../replay_ui/HOI4REPLAY_CONTAINER_SPEC.md`.

`Store::open(path)` AUTO-DETECTS: a directory → the on-disk store; a regular file →
a `.battleplan` (integrity-verified, served through the SAME `Store` interface), so
the app/scrub open a recording with **zero changes elsewhere**. Identity is enforced
via `Store::open_replay(path, ExpectedIdentity{ build, schema, modset })`.

```
cargo run -p snapshot --bin pack_replay -- pack   ../replay/snapshots campaign out.battleplan
cargo run -p snapshot --bin pack_replay -- verify out.battleplan ../replay/snapshots campaign   # + byte-identity check
```

Integrity (spec §3): per-entry SHA-256 over raw decompressed bytes + a top-level
`manifest_digest` over canonical JSON (RFC-8785) covering all identity metadata;
optional Ed25519 signature slot for true tamper-proofing. Hardened against
path-traversal, duplicate/stowaway entries, zip-bombs, and manifest-less (recovery-
only) files. Payloads are byte-copied, never re-serialized, so the no-fabrication
trichotomy + rollback filenames round-trip losslessly. The *writer* is the Python
hand-off (`../replay_ui/WRITER_INTEGRATION_PLAN.md`); the in-crate `pack` is the
reference/test packer.

## Conventions

- No personal paths in committed files — the store root is a relative default or a CLI
  arg; copyrighted game art stays gitignored under the existing rule.
- Crates are added per milestone (M1 `assets`/`render`, M0.5 `gui_runtime`, M3 `bind`,
  M2+ `app`); see `Cargo.toml`.
