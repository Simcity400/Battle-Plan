# COMBAT1 acceptance spec — XUI20 combat detail and losses

> **SUPERSEDED — lane JL, 2026-08-15.** The two modules this spec is written against,
> `game_ui/src/combat_detail_losses.rs` and `game_ui/tests/xui20_replay_admission.rs`, were
> DELETED under the owner's ruling on the twelve `game_ui` module islands (ruthless debloat;
> git history is the archive — see `extractor/native/runs/jl_render_purge/`). Nothing here is
> actionable as written: every file:line below pointed into deleted source, and the two
> citations the integrity gate flagged have been reduced to prose for that reason. If XUI20
> combat detail is rebuilt, this spec is a starting point to re-derive, not a checklist to run.

Written BEFORE any implementation, per INVERTED LANE PROTOCOL section 2.
Reviewer: the lane agent (Opus). Implementer: Codex.

Worktree (the ONLY writable area):
`.claude\worktrees\agent-a0edd153ca6f47e4b`
Branch `lane-combat1-xui20`, based on `worktree-replay-production` tip `2b7406a0`.

---

## 0. The finding this lane is built on

The packet's own reason string
(`replay_interface/native/game_ui/src/production_renderer.rs:732`) and the
blocking-tier `documents/SEAM_CONTRACTS.md:42` (row A33) both assert:

> the terminal land record owning-vector join is not carried

**That is false.** The join `(combat_id, generation)` is captured by the
production-activated a729 terminal-writer hook, re-joined and location-cross-
checked by the collector, wire-encoded, folded into the owning combat's detail
object, and parsed replay-side into a typed `LandCombatTerminalRecord`. Evidence
with file:line is in section A of
`extractor\native\runs\replay_production\composition\combat1_ledger.txt`.

The single true defect was replay-side, in the now-deleted combat-detail module: its
`land_panel` did not copy `terminal_record` / `terminal_record_status` out of `row.detail`, its
`validate_ended_land` ignored both leaves on ended rows where they actually live, and its
`admit_xui20_replay` set `land_terminal` to a hard-coded `ExternalInputRequired` computed from
no input at all. (Line references removed by lane JL: the file no longer exists.)

---

## 1. Checkable properties — Job 1 (`land_terminal`)

**J1-A. Nothing authenticated is dropped at the binder.**
`LandCombatPanel` carries `terminal_record: Leaf<LandCombatTerminalRecord>` and
`terminal_record_status: Leaf<String>`, populated by `land_panel` with the same
four-state fan-out the nine existing detail fields use: `row.detail` measured =>
clone the leaves verbatim; `row.detail` `Unobtained` => both `Unobtained`;
`row.detail` `Absent` => both `Absent`. No collapse of absent/unobtained.

**J1-B. `land_terminal` is COMPUTED, not a constant.**
`admit_xui20_replay` derives `land_terminal` from a new
`validate_land_terminal_surface(...)` through the existing `state(...)` helper,
exactly as `land_active`, `land_side_detail`, `land_ended_lifecycle` and
`naval_active` do. Grepping the function body for
`ExternalInputRequired {` must find no literal for `land_terminal`.

**J1-C. The surface is over the collection that owns terminal records.**
Terminal records can be carried by either an ENDED row
(`EndedCombatRow.detail.terminal_record`, `replay_core/src/world.rs:335` +
`:449-450`) or an ACTIVE panel. The producer explicitly forbids the same
`(combat_id, generation)` from appearing in both collections in one frame
(`extractor/native/recorder_host/src/world_fold.rs:6689-6691`, enforced by
`:6696-6701`), so a shared identity is corrupted input to reject, never an
overlap to deduplicate. The gate reads BOTH sources and is gated on BOTH
`ended_land_status` and `land_status`: a non-`Present` collection is
`External`, never an admitted or measured-empty contribution.

**J1-D. Every leaf is checked, and nothing is invented.**
Per terminal record, keyed by its owning `(combat_id, generation)`:

* `terminal_record_status == "exact"` requires a `Measured` `terminal_record`;
  a measured `terminal_record` under any other status is `Invalid`.
* `terminal_record_status == "unavailable"` is `External`, naming the identity.
  This is the producer's own explicit "ended, but no record captured" state
  (`world_fold.rs:6799-6803`) and must never be read as zero.
* any other measured status string is `Invalid` ("unknown terminal_record_status").
* an `Absent`/`Unobtained` status leaf on an ended row is `External`.
* `writer` must be exactly `"normal"` or `"overrun"`. `"unknown"` is what
  `world_fold.rs:6782-6786` emits for an unrecognised `writer_kind` and must
  REJECT (`Invalid`), not admit.
* `source` must be exactly `"authenticated_a729_terminal_writer"`
  (`world_fold.rs:6795`). Any other string is `Invalid`.
* `(writer, result_selector_raw)` must be `("overrun", 0)` or `("normal", 1)`.
  This re-checks, at the replay boundary, the exact pair the producer's own
  projection gate enforces (`extractor/native/lanes/land_combat_detail_events_live/src/a729_hook.rs:432-438`). Two authenticated
  validations drifting apart is the failure mode this project has been burned
  by; it is re-checked here for the same reason `admit_xui05_tooltip_route`
  re-checks the tooltip root (`replay_interface/native/game_ui/src/production_renderer.rs:596-608`).
* **The owning-vector join is RE-VERIFIED replay-side.** The terminal record's
  own `location_province` must equal the owning row's `location_province`
  (the ended row's, or the active panel's). A mismatch is `Invalid`. This is
  the same two-field join the collector performs
  (`attach_log.rs:44160-44170`) and is the concrete answer to the packet's
  stale claim.
* `record_index`, `end_date_raw`, `attacker_manpower_loss_raw`,
  `defender_manpower_loss_raw`, `winner_side_raw`, `winter`, `layout_side_raw`
  must all be `Measured` or the record is `External` naming the field.
  `record_index` and `end_date_raw` negative is `Invalid` (both are unsigned
  producer-side: `extractor/native/lanes/land_combat_detail_events_live/src/lib.rs:357`, `:363`).
* **No range is invented for `winner_side_raw` or `layout_side_raw`.** The RE
  dossier documents them as a "winner-side discriminator" and an
  "attacker/defender layout selector" (`d21_land_combats_live.md:43-45`) but
  does not enumerate their legal values. They are required MEASURED and
  nothing more. Adding a `0..=1` bound would be a fabricated constant.
* **Loss figures are never defaulted.** `attacker_manpower_loss_raw` /
  `defender_manpower_loss_raw` absent is `External`, never `0`. No sign or
  magnitude bound is imposed; none is evidenced.

**J1-E. `row_count` means what it says.** `land_terminal`'s admitted
`row_count` is the number of terminal records actually validated, not the
number of ended combats and not the number of land panels.

**J1-F. No sibling surface changes behaviour.** `land_active`,
`land_side_detail`, `land_ended_lifecycle`, `naval_*`, `air_*`, `convoy_*`,
`war_*`, `combat_log_families` and `global_cross_family_order` produce
byte-identical states for every existing fixture. In particular an ACTIVE
combat carrying no terminal record must NOT make `land_active` or
`land_side_detail` refuse.

---

## 2. Checkable properties — Job 2 (the production route)

> **Owner-ruling addendum:** Job 2 was canceled after this pre-implementation
> specification was written. This section is retained only as historical
> planning context; none of J2-A through J2-E is an acceptance requirement for
> the delivered lane.

**Route shape: CONDITIONAL, ADMISSION-ONLY. NOT an eighth `HudSurface`.**
Combat detail is not one of the seven `HudSurfaceActivationKind` variants
(`replay_interface/native/game_ui/src/production_renderer.rs:265-276`) and has no producer-side `hud_role/v1`
consumer token or setup hook, so the always-on shape is unavailable and
`PRODUCT_HUD_SURFACES` is NOT the blocker here (ROUTE4's finding does not
transfer). No row may be added to `PRODUCT_HUD_SURFACES`,
`REQUIRED_AUTHORED_HUD_SOURCES`, `HudSurfaceActivationKind` or
`hud_activation_kind_token`.

The route is modelled on XUI05's, whose product reach is an ADMISSION and never
a second compositor (`replay_interface/native/game_ui/src/production_renderer.rs:64-95`,
`ui_runtime.rs:2141-2202`). The authenticated selected-Present pass owns the
combat panel's pixels.

**J2-A. Carrier authority (contract part 1).**
`admit_xui20_replay` over `combat_detail_losses_from_frame(frame)` is the
carrier admitter and it can reject — fourteen independently gated surfaces,
each `Admitted` / `ExternalInputRequired` / `Rejected` / `NotApplicable`.

**J2-B. Semantic root binding comes from the RECORDING (contract part 2).**
Discovery in `replay_app` scans `pass.ui_pass_operations` for
`UiPassOperation::HudCommand` whose
`command.identity.selected_gui_definition.normalized_game_relative_path` is one
of the combat-detail authored sources. The authored ROOT NAME is read off the
command (`authored_surface_identity` / `runtime_root` / `name`), NOT compared
against a hard-coded root constant — a modded profile that renamed
`landcombatview` still names its root in its own command. Only the SOURCE PATH
is a constant, which is the same precedent XUI05 sets
(`GLOBAL_TOOLTIP_GUI_DEFINITION`, `ui_runtime.rs:1863`) and
`REQUIRED_AUTHORED_HUD_SOURCES` sets for the four always-on roots.

New constants in `game_ui::combat_detail_losses` (verified present in the
shipped a729 install at
`<local-path> of Iron IV\interface\`):

| source | authored root observed in the file |
|---|---|
| `interface/landcombat.gui` | `landcombatview` (`landcombat.gui:321`) |
| `interface/navalcombat.gui` | `navalcombatview` (`navalcombat.gui:8`) |
| `interface/combatlogview.gui` | `combat_log_view` (`combatlogview.gui:11`) |

**J2-C. Authored source admission (contract part 3).**
For each combat-detail source the pass names, the route calls
`content_admission.require_used_content(<path>)` and maps a failure to
`ProductionUiRejection::Asset` — per-route, not unconditional, so a recording
that never showed a combat panel is not refused for lacking the source
(`replay_interface/native/game_ui/src/production_renderer.rs:36-39`).

**J2-D. Invents nothing, and has a completeness gate (contract part 4).**
The route DRAWS NOTHING. Its completeness gate is that everything it reports
traces to admitted state:

* the receipt names, per family, which of the fourteen XUI20 surfaces
  admitted AND which did not, with the non-admitted family's own typed reason
  carried through verbatim. The existing receipt
  (`ui_runtime.rs:1569-1592`) counts only the admitted ones, so a viewer
  currently sees "12 surfaces" and cannot tell that two refused. That is the
  "must not imply completeness it does not have" requirement and it is part of
  this lane.
* row counts are NEVER summed across families that did not admit.
* a pass that PAINTS a combat-detail root while `admit_xui20_replay` admits no
  family for that hour is a named unavailability, in the shape of XUI05's
  "the pass paints N operation(s) that XUI20 did not admit" branch
  (`ui_runtime.rs:2162-2176`) — reported, never drawn, never placeheld.
* an admitted family whose required authored source is not used content is a
  typed refusal, not a silent skip.
* **no unit or equipment id is rendered as a display name anywhere**, and no
  text is produced by this route at all (it draws nothing), so the authored
  font / admitted localisation requirement is met by construction and must be
  stated as such rather than assumed.

**J2-E. The route cannot draw.** The `replay_app` route function's signature is
pinned WHOLE by a test, exactly as XUI05's is
(`ui_runtime.rs:2544-2558`): no parameter that could reach a device, a render
target or the runtime. Its body must not contain `compose_`, `set_rgba` or
`.draw(`. It must be reached before every `self.image.` and `self.m02.` write
in the selected-Present execution.

---

## 3. Tests — and the vacuity bar

Every test must answer: **name the change to production code, inside the
property's space, that would make this test fail.** A test that cannot is
vacuous and is a finding, not coverage.

Two specific traps in this lane:

* The deleted XUI20 replay-admission tests asserted
  `matches!(admission.land_terminal, ExternalInputRequired { .. })`. Those kept
  passing when the constant becomes computed, because the fixture has no
  terminal record. They are NOT evidence that Job 1 landed. A **positive arm is
  mandatory**: a fixture ended row carrying a complete, join-consistent
  `terminal_record` must produce `Admitted { row_count: N }` with the exact N.
* Every refusal arm must assert the **exact typed state and its reason
  substring**, not `matches!(.., ExternalInputRequired { .. })` — `is_err()`-
  grade assertions cannot tell "record absent" from "join mismatch" from
  "unknown writer", and reason-correctness is a separate property from
  rejection.

Required negative arms, each with its own test and its own exact assertion:

1. `writer: "unknown"` rejects (`Rejected`), and the reason names the writer.
2. `source` not `authenticated_a729_terminal_writer` rejects.
3. `(normal, result_selector_raw = 0)` and `(overrun, 1)` both reject.
4. terminal `location_province` != owning row `location_province` rejects, and
   the reason names the join. **This is the lane's headline regression test.**
5. `terminal_record_status: "unavailable"` on an ended row is
   `ExternalInputRequired`, and a fixture asserting that it is NOT
   `Admitted { row_count: 0 }` — the "absent is not zero" property.
6. a measured `terminal_record` under status `"unavailable"` rejects.
7. `attacker_manpower_loss_raw` unobtained is `ExternalInputRequired` naming
   that field, never an admitted record with a zero loss.
8. a non-`Present` `ended_land_status` with retained ended rows carrying valid
   terminal records is still `ExternalInputRequired`, never admitted.
9. the four-state fan-out through `land_panel`: `detail: Unobtained` gives
   `terminal_record: Unobtained` (not `Absent`), `detail: Absent` gives
   `Absent`.
10. any readable `(combat_id, generation)` shared by active and ended
    collections rejects as impossible double-emission, even when the active row
    carries no terminal data.
11. a positively measured active `phase_event: "ended"` with non-measured
    terminal status is `ExternalInputRequired`, never an admitted zero
    contribution.
12. started rows and rows with no d55 phase enrichment still skip when they
    carry no terminal data and do not conflict with an ended identity.
13. non-`Present` `land_status` refuses even when retained ended rows are valid;
    the unknown active contribution is never counted as zero.

Canceled route tests (historical only; not required after the owner ruling):

14. the route admits with a fixture pass painting a combat root + used content.
15. the route refuses (typed `Asset`) when the source is not used content.
16. a pass painting a combat root with no admitted family yields the named
    unavailability, and no draw.
17. the pinned-signature / no-draw / ordering tests from J2-E.
18. the receipt names a non-admitted family; a test that would fail if the
    receipt reported only the admitted count.

---

## 4. Files in bounds

* `replay_interface/native/game_ui/src/combat_detail_losses.rs`
* `replay_interface/native/game_ui/src/production_renderer.rs` — MINIMAL and
  localized (every lane in this wave edits its tables; keep the merge mechanical)
* `replay_interface/native/game_ui/tests/xui20_replay_admission.rs`
* `replay_interface/native/game_ui/tests/production_renderer.rs`
* `replay_interface/native/replay_app/src/ui_runtime.rs`
* `replay_interface/native/replay_app/src/selected_pov_runtime.rs` (only if the
  route needs it)
* `documents/SEAM_CONTRACTS.md` — row A33 only
* new test files under `replay_interface/native/game_ui/tests/` if needed

## 5. Explicitly OUT of bounds

* **anything under `extractor/`** — FROZEN by owner ruling, read-only. Nothing
  in this lane needs an extractor change; if the implementation appears to,
  STOP and report rather than working around it.
* `replay_interface/native/replay_core/**` — `terminal_record` is already
  parsed and merged there; no change is required and none may be made.
* `replay_interface/native/game_ui/src/hud_role_index.rs` and
  `src/content_interpreter.rs` — lane NORM1 owns both.
* `documents/DECISIONS.md` — propose, never edit.
* `portable_replay/**`.
* `PRODUCT_HUD_SURFACES`, `REQUIRED_AUTHORED_HUD_SOURCES`,
  `HudSurfaceActivationKind`, `hud_activation_kind_token` — no new rows or
  variants. An eighth always-on surface needs owner sign-off.
* other lanes' packets: XUI19, XUI13, XUI23, XUI21, XUI06/16/17, XUI14, XUI12.
* any defect exercised by the required gates blocks completion; resolve an
  ownership conflict through the orchestrator and never waive the defect as
  pre-existing.

## 6. Bookkeeping — only AFTER the work

> **Owner-ruling addendum:** Job 2 did not close. XUI20 therefore stays in
> `OMITTED_AUTHORITY_BLOCKED_PACKETS`; this lane leaves the shared packet tables
> untouched and records proposed reason-string corrections in its ledger.
> The pre-ruling promotion plan below is retained only as historical context.

The packet tables are report-only; no production branch reads them
(`replay_interface/native/game_ui/src/production_renderer.rs:11-16`). A move must FOLLOW real work.

* `replay_interface/native/game_ui/src/production_renderer.rs:731-733` — XUI20's reason string is rewritten. The
  "owning-vector join is not carried" clause is DELETED because it is false.
* `replay_interface/native/game_ui/src/production_renderer.rs:677` — the parenthetical "XUI20's terminal land
  owning-vector join" in the `OMITTED_AUTHORITY_BLOCKED_PACKETS` doc comment
  must go with it.
* `documents/SEAM_CONTRACTS.md:42` (row A33) — the false clause is deleted and
  replaced by what is actually true, with the file:line evidence. This is
  blocking-tier; leaving a disproven assertion there is worse than a stale
  comment.
* If BOTH halves close, XUI20 leaves `OMITTED_AUTHORITY_BLOCKED_PACKETS`. It
  does not belong in `OMITTED_PIXEL_CANVAS_ROUTES` either, because that list is
  for packets whose semantic window/root binding is NOT authenticated and after
  this lane XUI20's is. Its destination is `PROMOTED_XUI_PACKETS`, on the same
  ground XUI05 is there: a real product route that the app reaches on every
  selected-Present execution. The entry's doc comment must state plainly what
  XUI20 has and does not have — an admission route, no reconstructed
  compositor, the pass owns the pixels — so the report cannot overstate.
* Both partition assertions move together:
  `replay_interface/native/game_ui/src/production_renderer.rs:1432-1484` (unit) and
  `tests/production_renderer.rs:359-380` (integration), including the
  `OMITTED_AUTHORITY_BLOCKED_PACKETS.len()` count 17 -> 16 and the
  `PROMOTED_XUI_PACKETS` literal in three places.
* If only the route closes and `land_terminal` does not, XUI20 STAYS in
  `OMITTED_AUTHORITY_BLOCKED_PACKETS` and only its reason string is corrected
  to name what actually remains.

## 7. Gates — the implementation lane runs these before review

From the candidate worktree root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Replay -CacheMode Iterative
```

With the real install (the standing gate does NOT set these):

```powershell
$env:BATTLE_PLAN_GAME_ROOT="<local-path> of Iron IV"
$env:BATTLE_PLAN_GAME_DIR="<local-path> of Iron IV"
$env:BATTLE_PLAN_USER_DIR="C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Evidence -Workspace Replay -Package game_ui -CacheMode Reusable
```

Then:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All
```

Every observed failure is actionable before review. Do not waive a failure as
pre-existing or transfer it to another lane while claiming this candidate
complete.

Expected shape: per-target counts strictly >= baseline and zero failures. The
candidate worktree must contain only the intended candidate changes after every
round; validation in one worktree is never evidence for another.

## 8. Closed defect class for this lane

data loss or silently wrong data on input a real capture produces; a four-state
decision that collapses absent/null/unknown/zero; a loss figure that is
defaulted, interpolated or summed from a partial set; a fabricated constant or
range with no evidence behind it; a drawn or reported element that cannot be
traced to admitted state; a route that draws over the authenticated pass; a
citation that does not say what the code claims it says; a test that passes for
the wrong reason.

Out of scope: cosmetics, benchmark realism, speculative refactors, and any
change under `extractor/`. A defect exercised by the required gates is never
out of scope and blocks completion until resolved.
