//! Passive map-side carriers — lane HY, task #183.
//!
//! Three producer rows, all of them reads of
//! long-lived engine memory reached by fixed offsets from a static anchor:
//!
//! | Row | Lane | State |
//! |---|---|---|
//! | `EIR-XMAP17-FACTION-LABEL-TEXT` | GP-N1 | [`xmap17`] — built on D53's own `CFaction` census |
//! | `EIR-XMAP17-PLAYER-LABEL-IDENTITY` | GP-N1 | **not built** — the field offered as the player identity is a string length; see `replay_interface/re_evidence/rendering_laws/XMAP00/xmap_exact_evidence_index.tsv` |
//! | `EIR-XMAP07-SNOW-MUD-RUNTIME` (fade half) | GP-U11 | [`xmap07`] — passive source built |
//! | `EIR-XMAP06-BORDER-RUNTIME` | GP-N2 | anchor closed in [`anchor`]; the carrier itself is blocked, see `replay_interface/re_evidence/rendering_laws/XMAP00/xmap_exact_evidence_index.tsv` |
//!
//! # What this crate is, and what it deliberately is not
//!
//! It is the read laws and the carrier shapes: offsets, guards, refusals, and
//! the exact bytes the replay consumer hashes. It contains no engine
//! interaction of any kind — every read goes through [`memory::PassiveMemory`],
//! which the collector implements over its SEH-protected primitives and every
//! test implements over an explicitly mapped fixture. That separation is what
//! makes a read law testable without a game process, and it is why a refusal
//! in this crate is a value and not a log line.
//!
//! It does **not** register anything with the fold.
//! `world_fold.rs::build_snapshot`'s `_r2_*` sidecar block is reserved for the
//! Wave-2 scaffold lane, which has not landed; [`fold_emission`] packages the
//! seam as pure functions for that lane to call.
//!
//! # Fail-closed, uniformly
//!
//! Every entry point returns `Result<_, refusal::Refusal>`. There is no arm
//! anywhere that substitutes a default, clamps an out-of-range value into
//! range, drops a row that could not be read, or returns an empty collection
//! for a read that did not complete. `Refusal::into_domain_outcome` is the one
//! bridge to `compact_contract::domain_outcome::DomainOutcome`, so a site
//! cannot hand-roll a status bit.

pub mod anchor;
pub mod engine_string;
pub mod fold_emission;
pub mod memory;
pub mod refusal;
pub mod xmap07;
pub mod xmap17;

pub use memory::{PassiveMemory, PassiveMemoryExt};
pub use refusal::Refusal;
