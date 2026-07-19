# Collector Recovery 2026-07-17

Status: recovered and mechanically green.

## Cause and bounded repair

An A15 `apply_patch` write landed from a stale full-file image while the shared
collector was changing. It produced ten one-line truncated splice fragments in
the prefix of `collector/src/lib.rs`, reducing the file to 65,342 lines and a
working-tree diff of 1,746 additions / 11,428 deletions. The splice markers were
at then-current lines 596, 701, 873, 990, 1201, 1386, 1580, 1747, 2084, and
2260. Each marker was replaced with its exact intact reference range while
retaining the validated A15, d42, A1, A14, A22, hub62, domain61, and B8 overlays.

The intact reference was:

- path: `C:\Users\Example\hoi4_lanes\port_w1\master_merge\extractor\native\collector\src\lib.rs`
- commit: `8ec852c`
- reference SHA-256: `3d5f8d85d52e178014c1584c66f7188f18aed85cdbd50ccc8d83c7c6f019f2`

The recovery also restored the complete dormant duplex dependency closure from
that same reference commit:

| File | SHA-256 |
| --- | --- |
| `collector/src/duplex_client.rs` | `3d82734d46ebeb852fd0344ae1c518fe89ff40e8b0925677a2e3f6626265048a` |
| `collector/src/duplex_client/tests.rs` | `598ec9baad9896191f4e7fb179a8fd316f5efc96b0ea7b181256da6a319ae15d` |
| `compact_contract/src/control_transport.rs` | `6a1aab2b26232d4f50cd05066751073dc882615c4c351dfdff161df75a215eea` |
| `compact_contract/src/control_transport/payloads.rs` | `0d0747b8f9a3c01e27d6e5f55a75a4236c2cc306b8b156163bf130e02c60afe6` |
| `compact_contract/src/identity_control.rs` | `f18694793f347aa6dd76dfd0e736e797b75bde862d016b5cd9b6dab1d2b1a82f` |
| `compact_contract/src/sink_failure.rs` | `cef9d847faef2bf8a85b87ecbbe9b2c16413b23cfb6443797478c301eb062350` |

The matching `test-support` feature declarations, test-only control
initializers, and `campaign_identity::Sha256` crate visibility were restored as
required by that exact module set. The existing B8
`compact_contract::building_registry` lookup was retained.

## Mechanical verification

- `cargo check -p collector --lib`: exit 0, 0 errors.
- Independent root verification of the same command: exit 0, 0 errors.
- `cargo fmt -p collector -- --check`: exit 0.
- `cargo test -p collector domain57_ -- --nocapture`: 8 passed, 0 failed.

Recovered `collector/src/lib.rs`: 75,887 lines, SHA-256
`64099eeade0d56cd5400ac1f17703343d23808ec1c7c7268de247826937fb64a`.
