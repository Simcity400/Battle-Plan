# LANE REPORT — U1 alerts strip / notification area (lane/u1d-alerts)

**Scope (U1 wave, REPLAY_PRODUCTION_PLAN §6a / UI_COVERAGE_MATRIX §2
"Alerts / notifications").** Derive + populate the topbar alert-icon strip: the
four alert count badges the topbar's `country_menu_window` shows — initiative,
decisions-available, intel, market-contracts — plus a fail-closed binding for
the scrolling alert/notification FEED. Reuses the rc-reconcile READ-ONLY
`replay_core::TopbarAlerts` view (req 9b). No `replay_core` / extractor / fixture
edits; new module `game_ui/src/alerts.rs` + new test `game_ui/tests/alerts_binding.rs`.

## Deliverables

- **`game_ui/src/alerts.rs`** (new) — pure binding seam (Constraint 2: parses no
  recording bytes). `alerts_strip_from_frame(frame, pov)` → `AlertsStripBinding`
  (4 `AlertBadge`s), `alert_feed_binding()` → fail-closed `AlertFeedBinding`.
- **`game_ui/tests/alerts_binding.rs`** (new) — 4 binding gates + 2 layout gates.
- **`game_ui/src/lib.rs`** — `pub mod alerts;`.
- Docs updated same change: FIDELITY_LEDGER §2.16, UI_COVERAGE_MATRIX §2 + §8,
  EXTRACTION_NEEDS §A (A18), REPLAY_DATA_REQUIREMENTS 9b.

## Layout gate (shared game_ui parser)

Parsed `interface/topbar.gui` + `interface/alerts.gui` through the shared
`game_ui` parser (`LayoutSet::add_document`) and the engine layout law
(`resolve_element_rect` / `resolve_topbar_rects_pure`). Skips when the game root
is absent (matches `topbar::topbar_rects_follow_engine_layout_law`); ran against
the real install this session.

- **L1** — all 4 topbar alert count-badge elements resolve to a rect
  (e.g. `initiative_text` @ (98,58) @1920×1080), and `topbar.gui` cites
  `GFX_topbar_alert_bg`.
- **L2** — `alerts.gui` `global_alerticon_window` parses, cites
  `GFX_global_alert_icons`, and resolves to its declared **43×38** rect.

## Binding table (view → display state)

Source view: `replay_core::TopbarAlerts` (`countries.<tag>.scalars.alerts.*`),
surfaced per frame via `WorldFrame::topbar_alerts(tag)` gated on
`topbar_extras_status()`.

| Alert (view field) | FILE topbar.gui element | Label | Notes |
|---|---|---|---|
| `initiative` | `initiative_text` | FILE element / raw count | faction/initiative counter |
| `decisions_available` | `decisionview_amount_to_take_items` | FILE | topbar ALSO shows `..._timeout_items` (Absent) |
| `intel` | `intelview_ops_available_items` | FILE + UNKNOWN which badge | topbar ALSO shows `..._prepared_or_completed_items` (Absent) |
| `market_contracts` | `non_delivering_contract_amount` | FILE + UNKNOWN which badge | topbar ALSO shows `ongoing_contract_amount` (delivering; Absent) |

Per-badge display fields:
- `count: Option<i64>` — measured value, or **fail-closed `None`** (never a
  fabricated `0`) when the leaf/view/POV is unmeasured/absent.
- `badge_shown: bool` — **FIT product convention**: `count > 0`. The engine's
  exact zero-suppression is C++ logic, not in the `.gui` → makes no parity claim.
- `count_text: Option<String>` — **UNKNOWN formatting → rendered raw**
  (`format!("{}")`), same rationale as the topbar scalars in `binding.rs`.

## FILE / FIT / UNKNOWN / gap labels

- **FILE:** the 4 badge element names, the badge-bg sprite `GFX_topbar_alert_bg`,
  the feed window `global_alerticon_window`, feed sprite `GFX_global_alert_icons`,
  the 43×38 feed rect — all cited from shipped `interface/topbar.gui` +
  `interface/alerts.gui` and gated by the shared parser.
- **FIT:** badge zero-suppression (`badge_shown = count>0`).
- **UNKNOWN:** count number formatting (raw); which sub-badge a single view count
  maps to where the topbar shows two (intel, market_contracts).

## Gaps filed (recording-Absent → fail-closed)

New requirement **A18** (EXTRACTION_NEEDS §A; cross-ref REPLAY_DATA_REQUIREMENTS 9b):
1. **Alert-badge SUB-counts** the single aggregate count can't split: decision
   timeout, intel prepared/completed, delivering (ongoing) contracts, prototype
   rewards. Rendered fail-closed absent.
2. **Fired-alert FEED stream** (`global_alerticon_window`): ordered per-POV
   per-hour `{alert_type_key, target, fire_time}`. Entirely Absent → the feed
   binding is `available:false`, empty. Pairs with the localisation join (req 9)
   for per-alert tooltip text.

These are recording-side gaps; closing them is an extractor + `replay_core`
change, not a `game_ui` binding change (this lane is READ-ONLY on `replay_core`).

## Matrix / ledger rows touched

- **UI_COVERAGE_MATRIX §2**: Alerts row → **POPULATED (U1)** (4 counts); feed
  Absent noted. §8 HUD POPULATED 2→3; grand POPULATED 3→4 panels.
- **FIDELITY_LEDGER §2.16**: new row "Topbar alerts strip binding population (U1)"
  = FILE (binding seam) + FIT (zero-suppression) + UNKNOWN (count formatting).
- **EXTRACTION_NEEDS §A**: A18 (sub-counts + fired-alert feed).
- **REPLAY_DATA_REQUIREMENTS 9b**: alert sub-counts + feed caveat.

## Gates (all green, no warnings)

- `cargo build -p game_ui` — ok.
- `cargo test -p game_ui` — 63 passed, 0 failed; adds 12 (6 `alerts.rs` unit +
  6 `alerts_binding.rs`: 4 binding gates + L1/L2 layout gates run against the
  real the game install).
- `cargo test -p replay_core` — all suites ok, 0 failed (view reused, not modified).

Expected values transcribed from the fixture GENERATOR SOURCE
(`replay_fixture_gen/src/main.rs` `fx_topbar_scalars_sweep`):
`initiative=tick%3, decisions_available=tick%4, intel=tick%2, market_contracts=tick%5`
(identical for GER/ENG/SOV), matching the fixture probe
`countries.GER.scalars.alerts.market_contracts==1` at t=11.
