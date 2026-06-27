//! `app` — the interactive shell, library half (BATCH-0 host architecture).
//!
//! The headless-testable LOGIC lives here (proven by `cargo test`, no GPU/window). The
//! BATCH-0 spine is [`host`]: the [`host::ViewRegistry`] (distributed `inventory`
//! registration), the generic [`host::WindowHost`] (window stack / z-order / modal /
//! hit-test / open-close routing — ZERO per-view branches), the [`host::BindContext`] /
//! [`host::RenderContext`] passed to every builder, the no-fabrication
//! [`host::Captured`] / [`host::DisplayAtom`] value types, and the row/template factory.
//! The 5 migrated Core surfaces + the 10 pre-created cluster slots live under [`views`].
//!
//! Boundaries:
//!   * input → [`host::ViewInput`] → [`host::WindowHost::route_input`] → `ViewCommand`s,
//!   * the GUI hit-test → [`host::WindowHost::resolve_click`] → a map pick on `Miss`,
//!   * the map is the bottom layer ([`camera`] drives pan/zoom + province pick),
//!   * the [`scrubber`] drives `scrub::Timeline`, and the host `rebind_all` + `bind_all`
//!     rebuild EVERY open registered window per tick via the windowed immutable LRU,
//!   * bundle selection + replay mod auto-match are surfaced in [`provenance`].
//!
//! LEGACY (superseded by [`host`], retained for their test coverage of the prior
//! contract the host now subsumes — do NOT use in new views): [`windows`] (the closed
//! `WindowKind` stack → [`host::WindowHost`]), [`drill`] (the closed `DrillAction`
//! registry → `ViewCommand` routing), [`rebind`] (the old digest → `views::digest`).

pub mod camera;
pub mod combat;
pub mod compose;
pub mod focus;
pub mod host;
pub mod menubar;
pub mod provenance;
pub mod scrubber;
pub mod views;

// LEGACY modules — superseded by `host`; kept for their unit-test coverage only.
pub mod drill;
pub mod rebind;
pub mod windows;

// The GPU/surface + event-loop glue is only built for the binary (needs winit/wgpu
// surface). It is declared in main.rs, not re-exported here, so the testable lib
// compiles and runs its unit tests without a window.
