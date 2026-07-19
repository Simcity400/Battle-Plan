# EXTRACTOR-P bounded replay integration patch

Status: **COMPLETED / PRODUCTION ADMITTED**. This document consumes only the frozen
`PASS 106/106` promotion tuple. It makes no new executable or memory-layout
claim.

## Exact gap

The domain-57 compact payload already carries the promoted A15 `SessionEventRow`
shape and the collector tests its exact chat/system/hotjoin/tag-switch source
gates. The prior recorder fold kept only `SessionEventKind::Pause`. The
completed patch now exposes the four promoted A15 event kinds under the
top-level `_r2_session_multiplayer_metadata_live` sidecar without changing the
compact payload or collector.

This is the only replay-boundary patch in this handoff. The A1, A2, A4, A12,
A14, A21, A22, and B8-X6 projections already exist in the frozen world fold.

## Patch boundary

Allowed shared files:

- `extractor/native/recorder_host/src/world_fold.rs`
- `extractor/native/recorder_host/src/world_fold/tests.rs`

No compact payload, collector, binary profile, registry, rendering-law, or
`PROGRAM_STATUS` change is required.

## Implemented contract

1. Add an hour-scoped domain-57 event projection carrying the existing compact
   fields without renaming or loss:
   `kind`, `flags`, `actor_key_hash`, `actor_name`, `text`, `tag_db_id`,
   `value_i32`, `related_value_i32`, `command_sequence`, `event_hour_raw`,
   `event_source_id`, and `event_state`.
2. Continue folding `Pause` into the existing `pause` object. Project only the
   promoted A15 kinds `Hotjoin`, `TagSwitch`, `Chat`, and `SystemMessage` into
   `events.rows`, preserving payload order. Game-speed, alert, and notification
   remain owned by their separate gates and are not admitted by this patch.
3. Admit `Chat`/`SystemMessage` rows only when both
   `SOURCE_FLAG_MESSAGE_HOOK_VALIDATED` and root-evidence bit `1 << 3` are set.
   Admit `Hotjoin`/`TagSwitch` only when both
   `SOURCE_FLAG_COMMAND_HOOK_VALIDATED` and root-evidence bit `1 << 4` are set.
4. Emit `events.state="known"` with the admitted ordered rows when the relevant
   source-family witness is valid. Emit `events.state="unavailable"` with an
   empty row array when neither promoted event family is witnessed. A failure in
   one optional family must not discard a valid other family, player-country
   projection, or pause projection.
5. Reject malformed strings, unknown flags, invalid ids, invalid kind-specific
   values, and duplicate delta identities before projection. Do not fabricate
   actor names, message text, session slots, host flags, or connected flags.

## Completed tests

- One exact row for each of `hotjoin`, `tag_switch`, `chat`, and
  `system_message`, asserting every replay field.
- Message rows are unavailable when the message source flag or root witness is
  absent, while player events, players, and pause remain intact.
- Player-event rows are unavailable when the command source flag or root
  witness is absent, while message rows, players, and pause remain intact.
- Payload order is preserved and repeated full snapshots do not duplicate
  delta events.
- Non-clean/malformed domain-57 payloads produce the normal fail-closed
  provenance and never a measured-empty event claim.
- Existing player-country and pause tests remain unchanged and pass.

## Frozen completion evidence

- `world_fold.rs` SHA-256:
  `d0e42d67c678a38ba6e214af0e542287730812fbd553c2bdc9a1b09d28bb6cab`.
- `world_fold/tests.rs` SHA-256:
  `55ba4f4d8ea813909f8e903f33cb4edf043e0d3ea960c2694e03ce82d93af07a`.
- Focused A15 replay tests: 6 passed, 0 failed.
- Complete recorder library: 540 passed, 0 failed, 2 ignored.
- Complete recorder binary: 233 passed, 0 failed, 4 ignored.
- Stitch-container integration: 10 passed, 0 failed.

The first complete run also exposed one stale d42 test-string expectation. The
frozen A4/A21 authority surface `domain_data_authority.rs` (SHA-256
`1bcc03a41790b4c77640f3f5c4ebe08b29fb61f90aed90367b6527bd2ac68ad1`)
defines the exact 48-byte header and 76-byte row, so a one-row payload is 124
bytes and the intentionally stale 68-byte-row fixture is 116 bytes. The actual
unchanged parser diagnostic is exactly:

`offset 0: air combat losses live data payload is 116 byte(s), expected at least 124`

Only the assertion was reconciled to that exact diagnostic; the d42 parser and
collector were not changed. Final `recorder_host/src/main.rs` SHA-256 is
`aa35d560d22ef409e5e91e04de811d09d12655861402933b42b012f669ed5a81`.
