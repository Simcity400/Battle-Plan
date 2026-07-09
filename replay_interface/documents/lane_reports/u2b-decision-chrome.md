# LANE lane/u2b-decision-chrome — Decisions-panel chrome (U2 wave)

Derived + populated the **decisions panel chrome** (`countrydecisionview` — the
window that renders all vanilla decisions/missions data-driven through a small
chrome panel). All work in `game_ui`; new module `src/decision_chrome.rs` + new
test `tests/decision_chrome.rs`. No edits to `extractor/`, `replay_core`,
`map_viewer`, or fixtures; no existing module renamed/moved. `cargo build -p
game_ui` and `cargo test -p game_ui` green (64 unit + 2 file-gated + all
pre-existing suites unchanged). `decision_chrome` is clippy-clean and fmt-clean.

## 1. Layout gates (FILE — `interface/countrydecisionview.gui`)

Parsed via the shared `game_ui::gui` parser (reuse) through the
`AssetResolver` (vanilla root `<local-path>`; no bare joins).

GATE **U2-LAYOUT** (`decision_panel_layout_is_complete`) — all present, 0 parse recoveries:

| chrome element | kind | role |
|---|---|---|
| `countrydecisionview` | containerWindowType | root panel window |
| `decision_grid` | gridBoxType | the scrolling decision/category grid (1 col) |
| `category_header` | containerWindowType | category tab (icon + name + collapse) |
| `decision_item` | containerWindowType | plain decision card |
| `targeted_decision_item` | containerWindowType | targeted (has target flag) |
| `timed_decision_item` | containerWindowType | timed/mission (timer icon+text) |
| `on_map_decision_locator_item` | containerWindowType | on-map locator card |

Each of the 4 card templates carries an `icon` (or `decision_icon`) + a
`name_text` element; the plain `decision_item` also has `cost_and_timer_text` +
`target_flag`, and its icon default sprite is **`GFX_decision_unknown`** (the
FILE-cited fail-closed card icon).

## 2. Decision-def → card resolution (FILE — `common/decisions/**`)

Parsed every `common/decisions/*.txt` (excluding `categories/`) as
`category = { decision = { … } }` (2-level), cross-referenced against every
`common/decisions/categories/*.txt` category def. GATE **U2-RESOLVE**
(`decision_definitions_resolve_at_pinned_rates`), pinned vs vanilla 1.16.x:

| metric | count | rate |
|---|---|---|
| decision files parsed | 88 | (0 recoveries) |
| category files parsed | 57 | |
| **total decisions** | **4125** | |
| total categories | 591 | |
| orphan top-level blocks (not a known category) | 0 | clean cross-ref |
| decisions with known category | 4125 / 4125 | 100% |
| **name loc key resolves** | **3839 / 4125** | **93.1%** |
| `<id>_desc` tooltip key resolves | 1373 / 4125 | 33.3% |
| decisions declaring an `icon` | 3595 / 4125 | |
| **declared icon → shipped sprite resolves** | **3518 / 3595** | **97.9%** |
| category tab name (id) resolves | 571 / 591 | 96.6% |
| category tab icon/picture resolves | 548 / 591 | |
| categories declaring no icon/picture | 37 / 591 | tab uses layout default |
| card templates (decision_item / targeted / timed) | 1313 / 782 / 2030 | classified from declared fields |

**Resolution laws (all FILE-cited):**
- name loc key = decision id, or the decision's own `name = <key>` override.
- desc/tooltip loc key = `<id>_desc`.
- card icon sprite = `GFX_decision_<icon>` (verbatim if the token already starts
  with `GFX_`); no icon → `GFX_decision_unknown`.
- category tab icon = `picture` (a full `GFX_…`) else `GFX_decision_category_<icon>`.

**Documented residuals** (honest, not fitted): 286 decisions have no name loc
entry (dev/`debug_decisions`, `DEBUG_*`, and decisions whose name is set
dynamically at runtime); 77 declared icons (19 distinct tokens) don't map to a
shipped sprite — vanilla anomalies: double-`decision_` typos (e.g.
`icon = decision_generic_construction` → `GFX_decision_decision_generic_construction`,
absent), bare `icon = generic`, and referenced-but-unshipped sprites
(`GFX_decision_mission_icon`, `GFX_decision_generic_special_project`). All of
these degrade in-panel to the same `GFX_decision_unknown` the layout defaults to,
so the fail-closed behaviour matches the engine. Rates are asserted as exact
pinned counts **and** floor invariants (name ≥92%, icon ≥97%).

## 3. Binding (fail-closed) + the decision-state data ask

`decision_card_binding(def, Option<&DecisionStateEntry>)` fills the static card
chrome (id/category/name-key/icon-sprite/template) from files and folds an
optional per-decision dynamic state; every dynamic field (`availability`,
`cost_and_timer_text`, `timer_text`, `target_tag`) is `Some` only when the
stream supplies it, else `None` — never a fabricated "Available"/cost/timer.
Cost/timer NUMBER formatting is engine-computed → **UNKNOWN**, carried raw.

The per-decision dynamic state (which decisions are visible/available/active/
on-cooldown, live timers + costs) is **NEW recording data** — one of the
inventory's 14 NEW streams. Filed as:
- **EXTRACTION_NEEDS.md A18 / R-DECISION-STATE** (new §A row).
- **REPLAY_DATA_REQUIREMENTS.md §9c** (new section, inventory item 11).

`replay_core` has no decision-state view yet, so the binding input is a
`game_ui`-local `DecisionStateEntry` shape (this lane does not touch
`replay_core`). When that view lands the binding wires straight to it.

## 4. Docs updated (same change)

- `FIDELITY_LEDGER.md` §2.16 — new "Decisions-panel chrome + decision-def→card
  mapping (U2)" row (FILE layout+mapping; state = NEW stream) with the gate counts.
- `UI_COVERAGE_MATRIX.md` §1 "Decisions" row — status → **DEF-MAPPED (U2)**.
- `EXTRACTION_NEEDS.md` §A — new row **A18**.
- `REPLAY_DATA_REQUIREMENTS.md` — new section **§9c**.

## 5. Public surface (`game_ui::decision_chrome`)

`parse_decision_layout`, `DecisionPanelLayout`/`DecisionCardTemplate`;
`parse_decisions`/`parse_decision_categories`, `DecisionSet`/`DecisionDef`/
`DecisionCategoryDef`; `resolve_decisions`/`DecisionResolutionReport`;
`decision_name_loc_key`/`decision_desc_loc_key`/`decision_icon_sprite`/
`category_icon_sprite`/`classify_template`; `decision_card_binding`/
`DecisionCardBinding`/`DecisionStateEntry`/`DecisionAvailability`. 7 inline unit
tests (pure, no installed content) + 2 file-gated integration tests (SKIP when the game
root is absent, same pattern as `tests/widget_layout.rs`).
