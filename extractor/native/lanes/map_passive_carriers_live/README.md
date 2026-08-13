# `map_passive_carriers_live` — passive map-side read laws

Lane HY, task #183. Three of the gap map's map-side producer rows, plus the object anchor two of
them share.

## What is in here

| Module | Row | State |
|---|---|---|
| `anchor` | shared | the guarded game-root → delegate → map object → `map_render_state` chain (GP-N2's residual, closed) |
| `engine_string` | shared | the MSVC small-string reader, with every torn/overrunning shape refused |
| `xmap17` | `EIR-XMAP17-FACTION-LABEL-TEXT` | **built + jointly certified** |
| `xmap07` | `EIR-XMAP07-SNOW-MUD-RUNTIME` (fade) | **built**; the shipped promotion path now seals it |
| `fold_emission` | XMAP17 faction | the sidecar seam, as pure functions for the Wave-2 scaffold lane |

`EIR-XMAP17-PLAYER-LABEL-IDENTITY` and `EIR-XMAP06-BORDER-RUNTIME` are **not** built here, each
for a proven reason recorded in `runs/hy_map_producers/`.

## The one architectural decision

Every read goes through `memory::PassiveMemory`. This crate contains no `ReadProcessMemory`, no
SEH wrapper, and no engine interaction of any kind — the collector supplies one adapter
(`lanes/production_render_submit_live/src/a729_hook.rs::InProcessMemory`) and every test supplies
`memory::SliceMemory`, a fixture that serves a read only when the whole span falls inside an
explicitly mapped region.

That is what makes a read law testable without a game process, and it is why "the anchor refuses
when an inner link is swapped mid-walk" is a unit test here rather than a hoped-for property.

## Fail-closed, and what that costs

Every entry point returns `Result<_, Refusal>`. There is no arm that substitutes a default,
clamps an out-of-range value into range, drops a row that could not be read, or returns an empty
collection for a read that did not complete. In particular:

* one faction whose display text is unreadable refuses the **whole** carrier, because the carrier
  declares `faction_census_complete` and a silently dropped row would make that a false claim;
* a refused read is emitted as an explicit `null` under the consumer's key, never as a missing
  key, so `DomainUnavailable` stays distinguishable from `DomainAbsent`;
* `Refusal::into_domain_outcome` is the single bridge to
  `compact_contract::domain_outcome::DomainOutcome`, and it can never produce
  `AnswerPresence::Empty` — a chain that refused has no answer to call empty.

## How the faction carrier is meant to be wired

`build_faction_labels_carrier` takes D53's own census — `(faction_id, CFaction*)` pairs — rather
than resolving a handle it would have to guess. That is deliberate: `lanes/
diplomacy_intel_operatives_live` already walks `CGameState+0x3F8 -> CFactionSystem`,
authenticates each faction's packed id and members against a validated `CCountry`, and re-reads
the whole row topology before publishing. Re-deriving the roster here would let the two disagree,
which is the harness-authority-drift defect; taking it removes the join question entirely.

d53 already touches the string this row wants, too: it reads `CFaction + 0x38`
(`DOMAIN53_FACTION_NAME`) and keeps only a `stable_hash32` of it. `+0x38` is the field
read `+0x38` at all: a comparison against a cache whose staleness rules are unknown would
manufacture refusals on legitimate hours, and a reader nothing calls is worse than no reader. The
one field the adopting lane has to add is the `CFaction*` on `FactionRow`.

The census it passes in is not taken on trust. `build_faction_labels_carrier` re-reads the
engine's own packed identity word at `CFaction + 0x08` and requires the type discriminator and the
identity to match what the caller claimed, so a mispaired `(faction_id, CFaction*)` refuses
instead of publishing one faction's name under another's id.

## Joint certification

`tests/joint_certification.rs` (producer) and
`replay_interface/native/replay_core/src/xmap17_player_labels.rs::lane_hy_joint_certification_fixture_is_admitted`
(consumer) read the same committed fixture. Both sides authenticate the carrier by re-hashing
`serde_json::to_vec` of their own struct definitions, so a field added, renamed, reordered or
retyped on either side changes the digest — which would otherwise make every recorded hour
silently unavailable at replay time. The fixture keeps the roster's lexical key order different
from the consumer's numeric order, and the consumer half also mutates one byte of the payload and
requires rejection, so the pair cannot pass by being self-consistent.

Regenerate deliberately:

```text
HY_WRITE_XMAP17_FIXTURE=1 cargo test -p map_passive_carriers_live_lane --test joint_certification
```

## Mod-agnosticism

Every constant here is a structural offset or RVA in the compiled binary, pinned to
`hoi4.exe 1.19.2.0.a729` / `870ebf…c65ad`. Nothing encodes game content: the faction census comes
from D53's runtime walk, the D18 roster from the fold, and the strings from runtime storage. A mod
adds rows to the same tables; a game patch invalidates the offsets, which is why the executable
identity is part of every carrier and the anchor compares a vtable before following a field.
