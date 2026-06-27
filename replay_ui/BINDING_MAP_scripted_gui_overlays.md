# BINDING MAP — GROUP-1 scripted_gui OVERLAYS (GAME_UI_REPLICA_PLAN §C.4 group 1)

> **What this is.** The COMPLETE, no-truncation binding/accounting map for the **scripted_gui
> overlay** surface of the native replay UI: the mod's own in-game overlay `.gui` views plus the
> vanilla **nation-specific `*_scripted_gui`** overlays. These are IN-GAME views (the game loads
> and shows them during normal play), so a **100%-in-game-coverage contract** must account for every
> one of them. Unlike the chrome views (topbar / outliner / designers), the *content* of a
> scripted_gui overlay is driven by **script VARIABLES / FLAGS attached OUTSIDE the `.gui` tree** —
> the classic "scripted_gui attaches behavior outside the `.gui`" case. The `.gui` file is only the
> **layout shell**; a sibling `common/scripted_guis/<name>.txt` definition binds each element to a
> script variable, `pdx_tooltip`, or a dynamic `frame =`, and a `common/scripted_localisation/*.txt`
> turns the variable into the on-screen text.
>
> **These `.gui` files are NOT in PRIORITY_GUI**, so they are **not** pre-parsed into
> `game_assets/gui/*.json`. Their structure here was enumerated by **reading the raw `.gui` source
> read-only**. Mod overlays live at `interface/<name>.gui` (mod, the `mod/test` overlay set); vanilla
> overlays live at `interface/<name>.gui` **(vanilla)** — the vanilla install is resolved via the
> `extract.py` rule (`BATTLE_PLAN_VANILLA` env / `--vanilla` flag / generic Steam fallback;
> `game_assets/extract.py` lines 36–47). **No absolute install path appears in this doc** (hard
> rule); vanilla files are cited as `interface/<name>.gui (vanilla)`.

---

## 0. The GROUP-1 scripted_gui binding rule

A scripted_gui overlay's on-screen datum lives in a **script variable or flag**, not in the `.gui`.
The replay snapshot records script variables/flags in exactly **one channel**:

* **`world.variables`** — `{name: value}` **GLOBAL** script variables (SCHEMA §18; array members
  keyed `name^i`).
* **`world.flags`** — `{name: {value, date}}` **GLOBAL** flags (SCHEMA §18).
* **`world.saved_event_targets[]`** — named global scopes (SCHEMA §18).

> **CRITICAL SCOPING FACT (drives almost every verdict below).** The schema records **GLOBAL** script
> vars/flags only. There is **NO per-country arbitrary-variable bag** anywhere in the `Country`
> object (SCHEMA §3 — the only var-like leaves are `economy.market_automation` flags and named
> scalars; grep of SCHEMA confirms `variables`/`flags` appear **only** under §18 `world`). The
> `Country` schema is a **fixed scalar set** (economy/military/politics/research/…), not a generic
> key-value store. Therefore **a scripted_gui whose backing variable is COUNTRY-scoped
> (`context_type = player_context` / `country_context` / `decision_category`, var like
> `ETH.ETH_war_escalation_level` or `USA.usa_congress_senate_support`) is NOT bindable** — its datum
> is not in our snapshot. Only a scripted_gui whose backing variable is **GLOBAL**
> (`global.<var>` → `world.variables[...]`) is bindable.

**Verdict taxonomy** (every overlay gets exactly one):

| verdict | meaning | render |
|---|---|---|
| **(i) CHROME-ONLY** | backing script var is country-scoped or otherwise absent from schema/snapshot | real `.gui` chrome (background, icons, labels) + **`scripted content not captured`** badge over the value text/dynamic-frame slots |
| **(ii) PARTIALLY BINDABLE** | at least one element binds to `world.variables[...]` / `world.flags[...]` / `economy.resources.*` / VP (`map.vp_by_state` / `states.*.victory_points_total`) | chrome + element→source rows; non-bindable siblings keep the badge |
| **(iii) STATIC DECORATION** | pure cosmetic, no datum even in-game (e.g. focus-tree background image) | render as-is, no badge needed |
| **(iv) DEV-TOOL / INERT** | dev/debug surface with no replay meaning, or a **mutation** panel whose buttons are inert in a read-only replay | render chrome inert; buttons disabled; var-readouts (if any) bound per (ii) |

**Mutation buttons are INERT in replay (PLAN §D).** Every "do X" button (host-tool admin actions,
`select_law`, garrison assign, take-decision hotspots) is a write verb — disabled and non-interactive
in the read-only replay. We still render the button chrome so the panel looks right; we never fire it.

**No-fabrication (SCHEMA §0.2).** A bindable value that is `null` renders **"—"**, never `0`. We never
invent a scripted-gui value; a chrome-only overlay shows the badge, not a guessed number.

**DEMO-snapshot caveat (fold-in artifact, not a true gap).** The current demo snapshot is
**save-only**, and the save oracle's `world.json` pass may leave `world.variables`/`world.flags`
**empty/unfed** in that snapshot. That makes even the *bindable* VP overlay render the badge **right
now** — but `world.variables`/`world.flags` are **schema-present** (§18), so this is a **fold-in
artifact** of the demo generator, **distinct from a true schema gap**. See §3 ledger.

---

## 1. MOD overlays (`interface/<name>.gui`, the `mod/test` overlay set)

### 1.1 `horsttool.gui` — MP HOST TOOL (the host/admin control panel) — **(iv) DEV-TOOL / INERT (mutation panel)**

The largest overlay (1257 lines). It is an **admin mutation console**: nearly every element is a
button that **writes** game state (add factories, annex, award states, set laws, cheat-spawn, pause
control). In a read-only replay **all of these are INERT**. We render the panel chrome so it is
faithful, disable every button, and bind only the few **scalar readouts** it shows.

**Main display windows / sections** (enumerated at window grain, not per-button):

| window (`containerWindowType`) | role | replay treatment |
|---|---|---|
| `pause_block_window` | full-screen pause-block backdrop (host pause gate) | chrome; inert (pause is the scrubber's job in replay) |
| `horsttool_open_container` (→ `tool_open` button, `title` text `"[transfer_states_tooltip]"`) | the toggle button that opens the tool | chrome; `tool_open` button **inert**; `title` is a dynamic-loc tooltip token → badge |
| `horsttool_container` (root) → header `Owner Title` `[This.Owner.GetName]` | tool root + "owner of selected state" name | `[This.Owner.GetName]` = selected state's owner TAG name → **bindable** to `states.<sel>.owner` → country name (see readouts below) |
| `ht_admin_panel` | **country/admin powers** sub-panel: change-host, fix-production, tag-switch, observer, declare-war, building drop-menu, fix-AI-control, the pause-blocker mode cluster (`pause_blocker_*`), factory ± (`mill/civ/dock _add/_remove`), war-support ± (`wt_add/wt_remove`, `ws_*`), PP/manpower/resource cheats (`addpp`, `addmanpower`, `addtungsten…addcoal`, `addoil`), `award_states`, `annex_country`, `add_cores`/`remove_cores`, `puppet_country`, `free_country`, stability ± (`st_*`), compliance/resistance ± + readouts | **all buttons INERT (mutations)**; the two readouts `compliance_number` (`�G[?ROOT.compl]�!`) and `resistance_number` (`�R[?ROOT.rest]�!`) bind per readout table |
| `st_state_manager_powers_container` | **state-manager / cheat-menu** sub-panel: `toogle_mark`, `clear_all_markings`, and a 32-slot cheat grid (`cheat1…cheat32` buttons + `cheat1_text…cheat32_text` labels, label text `[cheat_selected]`), header `[This.GetName]` | **all buttons INERT**; `[This.GetName]` (selected-state name) bindable to state meta; `[cheat_selected]`/cheat labels = dynamic-loc selection echo → badge |

**Country/state scalar readouts the host tool shows (the only bindable data in horsttool):**

| element (name) | raw text token | snapshot source | formatter | fidelity |
|---|---|---|---|---|
| `Owner Title` | `[This.Owner.GetName]` | `states.<sel>.owner` → country display name | TAG→loc name | **derived** (needs the selected-state context `<sel>`; owner TAG is exact, name is a static loc lookup) |
| `title` (in `st_state_manager_powers_container`) | `[This.GetName]` | `<sel>` state name (map `state_meta`) | state→loc name | **derived** (static map meta, not snapshot) |
| `compliance_number` | `�G[?ROOT.compl]�!` | `states.<sel>.occupation.compliance` | int (0..100, green-tinted) | **exact** when a state is selected & occupied; **`—`** otherwise. (`[?ROOT.compl]` is a temp var the tool fills from the selected state's compliance — same datum as `states.<id>.occupation.compliance`, SCHEMA §4.) |
| `resistance_number` | `�R[?ROOT.rest]�!` | `states.<sel>.occupation.resistance` | int (0..100, red-tinted) | **exact** when selected & occupied; else **`—`**. |
| `transfer_states_tooltip` / `pause_selected` / `cheat_selected` | dynamic-loc tokens | — (UI selection-echo strings, not game data) | — | **badge** — these are interaction-state echoes (what you're about to do), meaningless in a non-interactive replay. |

> **Verdict: (iv) DEV-TOOL / INERT.** Host tool renders as faithful **inert** chrome. The ONLY bound
> data are the compliance/resistance readouts (= `states.<sel>.occupation.*`, already first-class in
> SCHEMA §4) and the owner/state name labels (derived from `states.<sel>.owner` + static state meta).
> Every other element is a **mutation button** (disabled) or a selection-echo loc token (badge).
> No host-tool action ever fires in replay.

### 1.2 `topbarresources.gui` — extra topbar resource readout — **(ii) PARTIALLY BINDABLE**

Displays 6 stockpile/availability resource counters (aluminium, rubber, tungsten, steel, chromium,
coal) in a topbar strip, plus a `toogle_button` to show/hide. Each counter's text is a
`scripted_localisation` `*_text` defined_text whose trigger compares `resource@<res>` vs
`resource_imported@<res>` **country-scoped** variables (confirmed in
`common/scripted_localisation/topbarresources.txt`: e.g. `steel_text` checks
`resource_imported@steel` vs `resource@steel`). The scripted_gui is `player_context`
(`common/scripted_guis/topbarresources.txt`).

| element (name + type) | raw token | snapshot source | formatter | fidelity | notes |
|---|---|---|---|---|---|
| `toogle_button` (buttonType) | `"Topbar Resources"` | — | — | static | show/hide toggle; **inert** in replay. |
| `aluminium_value` (instantTextboxType) | `[aluminium_text]` | `countries.<TAG>.economy.resources.available_derived.aluminium` (or `.produced.aluminium`) | resource-int (green/yellow by import) | **derived** | The `@res` variable is the country's net available resource; closest schema field is `economy.resources.available_derived.*` (= produced − exported, SCHEMA §3.1). |
| `rubber_value` | `[rubber_text]` | `economy.resources.available_derived.rubber` | resource-int | **derived** | as above |
| `tungsten_value` | `[tungsten_text]` | `economy.resources.available_derived.tungsten` | resource-int | **derived** | |
| `steel_value` | `[steel_text]` | `economy.resources.available_derived.steel` | resource-int | **derived** | |
| `chromium_value` | `[chromium_text]` | `economy.resources.available_derived.chromium` | resource-int | **derived** | |
| `coal_value` | `[coal_text]` | `economy.resources.available_derived.coal` | resource-int | **derived** | |
| `*_icon` (6×, `GFX_resources_strip`), `Background`/`Background2` | — | — | — | static | resource icons (frame=2..7) + box chrome. |

> **Verdict: (ii) PARTIALLY BINDABLE.** The 6 amounts bind to `economy.resources.*`. **Fidelity
> caveat:** the *exact* in-game number is the script var `resource@<res>` (net-of-import availability)
> and the **green/yellow tint** depends on `resource_imported@<res>` — `economy.resources.imported.*`
> is schema-reserved but currently **unfed** (SCHEMA §3.1: "reserved; null until oracle emits"), so we
> bind the **amount** (`available_derived`) and **badge the import-tint** (render neutral). The amount
> is `derived`, not `exact`, because we approximate the country var with the produced/exported ledger.

### 1.3 `topbarvp.gui` + `vp_scripted_gui.gui` — VICTORY-POINT scoring overlay — **(ii) PARTIALLY BINDABLE**

The mod's **VP victory-score system**. `topbarvp.gui` is a topbar widget (`topbar_vp_container` +
`topbar_vp_ticking`); `vp_scripted_gui.gui` is the `vp_explainer_window` decision-category breakdown
listing per-city VP loc lines. The wiring is in `common/scripted_guis/redbaron_scripted_gui.txt`
(`vp_topbar`, `topbar_vp_ticking`, `vp_explainer_window`, all gated `rule = vp_system`) and the text
in `common/scripted_localisation/topbarresources.txt` (`vp_text`, `vp_ticking_text`). **Crucially the
backing variables are GLOBAL**: `vp_text`/`vp_ticking_text` branch on
`check_variable = { global.allies_score … }` and `{ global.axis_score … }` →
**`world.variables["allies_score"]` / `world.variables["axis_score"]`** (SCHEMA §18). This is the
**one genuinely bindable nation-agnostic scripted system** in the assigned set.

| element (name + type) | raw token | snapshot source | formatter | fidelity | notes |
|---|---|---|---|---|---|
| `vp_value` (in `vp_container`) | `[vp_text]` | `world.variables["allies_score"]`, `world.variables["axis_score"]` | category (score→"axis leading/winning"/"allies …"/"not started") | **exact** (logic) | the loc picks a band from the two global scores (thresholds 0 / 7 / 10); we reproduce the same branch from the two recorded globals. |
| `vp_value` (in `vp_ticking_container`) | `[vp_ticking_text]` | `world.variables["axis_score"]` > 10 / `world.variables["allies_score"]` > 10 | category (victory-imminent banner) | **exact** (logic) | shows "axis victory / allied victory" when a side passes 10. |
| `tool_vp2` (buttonType, `GFX_generic_text_bg_154`) | — | — | — | static/inert | clickable header → would open the explainer; **inert** in replay (read-only open verb). |
| `vp_icon` (`GFX_victory_points`), `Background2` | — | — | — | static | VP icon + box chrome. |
| `vp_explainer_window` rows (`vp_paris`, `vp_moscow`, `vp_cairo`, … ~16 city lines + region headers `WesternEurope`/`EasternEurope`/`North Africa`/`Pacific` + `vp_overview`/`vp_score`) | static loc keys (`vp_victory_paris`, …) | per-VP-city control → which side holds it | category/loc | **derived** | Each city line's *colour/state* (held by axis/allies) is computed from **who controls that VP state** → `map.vp_by_state` + `states.<id>.controller` (SCHEMA §4/§8). The static label text is fixed loc; the **dynamic held-by tint is derived** from control. |

> **Verdict: (ii) PARTIALLY BINDABLE.** The two topbar score banners bind to GLOBAL
> `world.variables["allies_score"]`/`["axis_score"]` (schema-present, §18). The explainer's per-city
> lines bind their **held-by state** to `states.<id>.controller` / `map.vp_by_state`. The
> `tool_vp2` open button is inert. **DEMO caveat:** if the demo snapshot's `world.variables` is unfed,
> these render the badge **now** (fold-in artifact — see §3), not a true gap.

### 1.4 `naval_scripted_gui.gui` — admiral / command-limit fleet readout — **(i) CHROME-ONLY**

A naval command-limit overlay: a scrollable list (`admirals_leading_battlefleets_list`) of admiral
rows (`admirals_with_fleets_list`) showing per-admiral ship-type counts and an "over command-limit"
banner (`over_cap_open_container`, text `cap_over_on`). Every value is a **custom GUI script function
or country temp var**: `[GetAdmiralNameTitle]`, `[GetShipCount]`, `[GetCommandLimit]`,
`[GetScreenCount]`, `[GetScreensNeededCount]`, and per-hull temp counts `[?destroyer_count]`,
`[?submarine_count]`, `[?light_cruiser_count]`, `[?heavy_cruiser_count]`, `[?battleship_count]`,
`[?battlecruiser_count]`, `[?carrier_count]`, `[?capital_count]`.

The `[?*_count]` tokens are **country-scoped temp variables** the scripted_gui populates per admiral;
the `[Get*]` tokens are **custom_gui_container script functions**. Neither is in the snapshot: there
is no per-admiral fleet-composition object, and `[?destroyer_count]` etc. are transient temp vars, not
global `world.variables`.

> **Verdict: (i) CHROME-ONLY (scripted content not captured).** Render the window + list chrome +
> admiral portrait frames; badge the per-admiral counts. **Adjacent note:** the *aggregate* fleet data
> (per-country `military.ships_by_hull` SCHEMA §3.2, `military.num_ships`, and per-division naval data)
> IS in the schema, but it is **not the same datum** as this overlay's **per-admiral command-group**
> breakdown — the overlay groups ships by their leading admiral, a grouping we do not record. So this
> overlay stays chrome-only even though country-level naval totals exist elsewhere. **True gap → bucket
> (a)** (per-admiral fleet composition is live-extractable from the command structure; see §3).

### 1.5 `SWOA_focus_background.gui` — focus-tree background image — **(iii) STATIC DECORATION**

A single `iconType` (`SWOA_focus_background_image`) in `SWOA_focus_background_container` — a decorative
backdrop drawn behind a focus tree (the sprite is set by `common/scripted_guis/SWOA_focus_background.txt`).
No datum, in-game or otherwise.

> **Verdict: (iii) STATIC DECORATION.** Render the image as the focus-tree backdrop; no binding, no
> badge. (The replay's focus-tree view S3 already owns focus rendering; this is just its wallpaper.)

### 1.6 `game_check.gui` — game-rules dev/debug panel — **(iv) DEV-TOOL / INERT (chrome-only meaning)**

`check_gamerules_decision_ui_window` — a developer/debug surface (paired with
`common/scripted_guis/game_check.txt`) that echoes active game-rule selections, used during mod dev to
verify the lobby rules took effect. It carries no player-facing campaign datum and has **no replay
meaning**; it should normally not even be open in a real game.

> **Verdict: (iv) DEV-TOOL / INERT.** **Listed for completeness** (it is a loadable in-game `.gui`).
> Render chrome-only if ever surfaced; it is **out-of-replay-meaning** (a dev tool), not a data gap.
> Default: not shown in the replay shell.

### 1.7 Propaganda-campaign overlays (8 files) — **(i) CHROME-ONLY**

`com_` / `eng_` / `fra_` / `ger_` / `ita_` / `jap_` / `usa_` / `non_major_propaganda_campaigns_scripted_gui.gui`.
Each is a **poster board**: a `*_propaganda_bg` background `iconType` plus one or more poster-slot
`iconType`s (`ger_poster_slot_left`/`_center`, `com_poster_slot`, `axis_/man_/mon_/allies_poster_slot`,
etc.) drawn from a `GFX_<X>_propaganda_strip`. The **active poster is chosen by the scripted_gui
definition setting the icon's `frame =`** based on which propaganda campaign the country has running —
a **country-scoped** decision/variable (wired in `common/scripted_guis/<TAG>_propaganda_campaigns_scripted_gui.txt`).
`non_major_*` bundles 5 ideology variants (`axis_/man_/mon_/allies_` windows). The `.gui` itself has
**no text values** — only background + poster-strip frames.

| element class | per-file examples | binding |
|---|---|---|
| `propaganda_bg` (iconType) | every file | **static** background |
| poster-slot iconType(s) (dynamic `frame=`) | `ger_poster_slot_left/center`, `com_poster_slot`, `usa_poster_slot`, `axis_/man_/mon_/allies_poster_slot` | **badge** — the active-campaign frame is a country-scoped scripted var, not captured. |

> **Verdict (all 8): (i) CHROME-ONLY.** Render the propaganda board + background; the **which-poster
> frame is not captured** (country-scoped campaign state) → badge the poster slot. **True gap →
> bucket (c)** primarily: closure needs parsing `common/scripted_guis/<TAG>_propaganda_*.txt` to learn
> which decision/var sets the frame, **plus** recording that country-scoped variable (today out of
> schema). See §3.

---

## 2. VANILLA nation-specific `*_scripted_gui` overlays (`interface/<name>.gui` (vanilla))

These are vanilla "national-system" decision-category / topbar overlays. **All read country-scoped
script variables** (`context_type = decision_category` / `country_context` / `player_context`; backing
vars like `ETH.ETH_war_escalation_level`, `USA.usa_congress_senate_support`). Per §0 scoping fact,
**none of their values are in the snapshot** (country-scoped var bag is not recorded) → **all
CHROME-ONLY** unless a backing global is identified (none were). Each row gives the displayed content
(1–3 lines) and confirms the verdict. All files were **read successfully** (line counts noted); none
is dropped or unreadable.

| # | overlay (`interface/<name>.gui` (vanilla)) | what it displays | backing (scope) | verdict |
|---|---|---|---|---|
| V1 | `GER_inner_circle_scripted_gui` (477 ln) | Germany "inner circle" minister/faction-power board: portrait slots + influence/loyalty counters | GER-scoped inner-circle vars | **(i) CHROME-ONLY** |
| V2 | `GER_inner_circle_hidden_scripted_gui` (19 ln) | hidden/collapsed inner-circle helper window (tiny) | GER-scoped | **(i) CHROME-ONLY** |
| V3 | `GER_rk_scripted_gui` (25 ln) | Reichskommissariat status mini-readout | GER-scoped | **(i) CHROME-ONLY** |
| V4 | `ger_monroe_doctrine_scripted_gui` (63 ln) | GER "Monroe doctrine" board: countries-in-doctrine / in-support / in-opposition text + tooltips | GER-scoped doctrine country-count vars | **(i) CHROME-ONLY** |
| V5 | `JAP_imperial_influence_scripted_gui` (723 ln) | Japan imperial-influence board: per-puppet influence bars/portraits (largest vanilla one) | JAP-scoped influence vars | **(i) CHROME-ONLY** |
| V6 | `sov_paranoia_system_scripted_gui` (189 ln) | Soviet paranoia meter + paranoia-target portraits | SOV-scoped `paranoia` var | **(i) CHROME-ONLY** |
| V7 | `sov_propaganda_campaigns_scripted_gui` (76 ln) | Soviet propaganda poster board (vanilla analogue of the mod's set) | SOV-scoped campaign frame | **(i) CHROME-ONLY** |
| V8 | `usa_congress_scripted_gui` (62 ln) | USA congress board: Senate + House support headers & amounts | `USA.usa_congress_*_support` (country) | **(i) CHROME-ONLY** |
| V9 | `swe_rearnament_system_scripted_gui` (136 ln) | Sweden rearmament meter + status | SWE-scoped rearmament var | **(i) CHROME-ONLY** |
| V10 | `swe_train_the_sheltered_gui` (57 ln) | Sweden "train the sheltered" manpower-program readout | SWE-scoped var | **(i) CHROME-ONLY** |
| V11 | `AST_cabinet_trust_scripted_gui` (96 ln) | Australia cabinet-trust meter + minister trust | AST-scoped trust var | **(i) CHROME-ONLY** |
| V12 | `ast_right_vs_left_campaign_scripted_gui` (264 ln) | Australia right-vs-left campaign track + icons | AST-scoped campaign vars | **(i) CHROME-ONLY** |
| V13 | `ast_right_vs_left_campaign_empty_scripted_gui` (53 ln) | empty/placeholder variant of V12 (shown when campaign inactive) | AST-scoped | **(i) CHROME-ONLY** |
| V14 | `aus_antischluss_measures_scripted_gui` (28 ln) | Austria anti-Anschluss risk text + tooltip | AUS-scoped risk var | **(i) CHROME-ONLY** |
| V15 | `bul_internal_factions_scripted_gui` (224 ln) | Bulgaria internal-factions balance board | BUL-scoped faction vars | **(i) CHROME-ONLY** |
| V16 | `gre_internal_factions_scripted_gui` (242 ln) | Greece internal-factions balance board | GRE-scoped faction vars | **(i) CHROME-ONLY** |
| V17 | `CZE_industry_focus_scripted_gui` (161 ln) | Czechoslovakia industry-focus allocation board | CZE-scoped industry vars | **(i) CHROME-ONLY** |
| V18 | `SIA_khana_ratsadon_scripted_gui` (143 ln) | Siam Khana Ratsadon (people's party) influence board | SIA-scoped vars | **(i) CHROME-ONLY** |
| V19 | `sia_movie_theater_campaigns_scripted_gui` (49 ln) | Siam movie-theater propaganda poster board | SIA-scoped campaign frame | **(i) CHROME-ONLY** |
| V20 | `RAJ_famine` (57 ln) | Raj famine board: timeframe-reveal desc + spread-risk text | RAJ-scoped famine vars | **(i) CHROME-ONLY** |
| V21 | `RAJ_eic_tax_fraud` (172 ln) | Raj East-India-Company tax-fraud investigation board | RAJ-scoped vars | **(i) CHROME-ONLY** |
| V22 | `raj_local_leaders_investments_decision_ui` (223 ln) | Raj local-leaders investment allocation board | RAJ-scoped investment vars | **(i) CHROME-ONLY** |
| V23 | `war_escalation_scripted_gui` (126 ln) | War-escalation level text + 10 escalation-level icons (dynamic `frame=`) | `<TAG>.<TAG>_war_escalation_level` (country; confirmed `ETH.ETH_war_escalation_level` in `BBA_ethiopia_scripted_loc.txt`) | **(i) CHROME-ONLY** |

> **Confirmation of the scoping verdict (V8, V23).** `USA_congress_scripted_gui.txt` and
> `war_escalation_scripted_gui.txt` are both `context_type = decision_category` (country-scoped); the
> war-escalation level resolves to the **country variable** `ETH.ETH_war_escalation_level`. Neither is a
> `global.*` variable, so neither is in `world.variables`. This is representative of **all 23** vanilla
> overlays. **None is bindable** through the GLOBAL-only var channel.

> **Readability.** All 23 vanilla files + all mod files were read successfully (no unreadable file;
> none silently dropped). If a future run cannot resolve the vanilla root, the still-correct fallback
> is to account for the view by name with a `chrome-only, scripted content not captured` badge — but
> that fallback was **not needed here**.

---

## 3. SCHEMA-GAP LEDGER (artifact vs true gap; closure bucket)

For each overlay whose backing data is absent, classify **ARTIFACT** (schema-present, demo generator
just hasn't fed it) vs **TRUE GAP** (no schema field), and for true gaps a **closure bucket**:
**(a) LIVE-EXTRACTABLE** (read it from the live game / save), **(b) COMPUTED-NOT-STORED** (must be
recomputed), **(c) STATIC DEF / WORLD DATA** (parse `common/scripted_guis/*.txt` + `scripted_localisation`
to know which var feeds which element, and/or record that script var).

| overlay(s) | backing datum | artifact vs TRUE GAP | closure bucket |
|---|---|---|---|
| `topbarvp` + `vp_scripted_gui` (score banners) | `global.allies_score` / `global.axis_score` | **ARTIFACT** — `world.variables` is schema-present (§18); demo just unfed | (a) record `world.variables` in the live/save world-pass (already in the oracle's `world.json`); **already closeable** |
| `vp_scripted_gui` (per-city held-by tint) | who controls each VP state | **ARTIFACT** — `states.<id>.controller` + `map.vp_by_state` schema-present & fed | — (already bindable; just wire the per-city → state lookup) |
| `horsttool` compliance/resistance readouts | `states.<sel>.occupation.{compliance,resistance}` | **ARTIFACT** — schema-present & fed (§4) | — (already bindable) |
| `topbarresources` amounts | `resource@<res>` (net available) | **ARTIFACT (amount)** — `economy.resources.available_derived.*` present; import-tint leg `economy.resources.imported.*` **reserved/unfed** | (a) for amount (oracle already emits produced/exported); the exact `resource@` var + import-tint via (a)+(c) |
| Propaganda boards (mod ×8 + vanilla V7/V19) | active-campaign poster `frame` (country-scoped var) | **TRUE GAP** — no per-country campaign var in schema | **(c)** parse `common/scripted_guis/<TAG>_propaganda_*.txt` to map decision/var→`frame`; **+ (a)** record that country-scoped var |
| `naval_scripted_gui` per-admiral counts | per-admiral ship-type composition (`[?*_count]` temp vars, `[Get*]` fns) | **TRUE GAP** — no per-admiral fleet object in schema | **(a)** live-extractable from the command/fleet structure (admiral→fleet→ships); regroup ships by leading admiral |
| All 23 vanilla nation-systems + GER inner-circle/RK/monroe, JAP influence, SOV paranoia, USA congress, SWE rearm/sheltered, AST/AUS/BUL/GRE/CZE/SIA/RAJ boards, war_escalation | country-scoped national-system variables (e.g. `usa_congress_*`, `ETH_war_escalation_level`, `paranoia`, `cabinet_trust`, …) | **TRUE GAP** — schema has **no per-country arbitrary-variable bag** (§0 scoping fact) | **(c)** parse `common/scripted_guis/<name>.txt` (mod-aware: mod overrides vanilla by filename per `active_modset.json`) to learn which var feeds which element, **+ (a)** record those country-scoped script vars (a generic per-country `script_variables` map would close the entire class) |
| `horsttool` selection-echo tokens (`transfer_states_tooltip`, `pause_selected`, `cheat_selected`), naval/propaganda mutation hotspots | live UI interaction state | **TRUE GAP by nature** — interaction state has no replay meaning | **(b)/N-A** — not a data field; stays inert/badge (no closure needed) |
| `SWOA_focus_background` | — (decoration) | not a gap | — |
| `game_check` | — (dev tool) | not a gap (out-of-replay-meaning) | — |

**The headline closure path** for this whole group is **(a) + (c) together**: record the specific
script variables (GLOBAL ones already land in `world.variables`; the missing mass is **country-scoped**
vars, closeable by adding a per-country `script_variables` bag to the oracle) **and** parse
`common/scripted_guis/` **mod-aware** so the binder knows *which* variable feeds *which* `.gui` element
(the definition `.txt` is the authoritative element→var map, since the `.gui` only holds the layout
and a loc/token reference). Pure `world.variables`/`world.flags` alone closes **only the VP system**;
everything else needs the country-scoped var channel + the scripted_guis definition parse.

> **Codex caveat (folded in).** A per-country `script_variables` bag closes the **persistent
> country-scoped variable** mass (war_escalation, congress, paranoia, cabinet-trust, the
> propaganda-campaign frame var, …), but it does **not** close the **transient temp-var / GUI
> script-function** cases — chiefly `naval_scripted_gui` (`[?destroyer_count]` temp vars + `[Get*]`
> custom-gui functions computed per admiral). Those are bucket **(a)**: they close by **modeling the
> per-admiral fleet composition from the live command/fleet structure**, not by the var bag. So
> closure is two-pronged: (var bag + scripted_guis parse) for the persistent-var systems, and
> (live fleet/command extraction) for the script-function/temp-var ones.

---

## 4. OVERLAY ACCOUNTING TABLE (provably complete — none dropped)

**Total overlays accounted for: 38** (15 mod `.gui` files + 23 vanilla `.gui` files).

### Mod overlays (15 files)

| # | overlay | verdict |
|---|---|---|
| M1 | `horsttool.gui` | (iv) DEV-TOOL / INERT (mutation; 4 bound readouts) |
| M2 | `topbarresources.gui` | (ii) PARTIALLY BINDABLE (6 resource amounts) |
| M3 | `topbarvp.gui` | (ii) PARTIALLY BINDABLE (global score banners) |
| M4 | `vp_scripted_gui.gui` | (ii) PARTIALLY BINDABLE (global score + per-city control) |
| M5 | `naval_scripted_gui.gui` | (i) CHROME-ONLY |
| M6 | `SWOA_focus_background.gui` | (iii) STATIC DECORATION |
| M7 | `game_check.gui` | (iv) DEV-TOOL / INERT |
| M8 | `com_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M9 | `eng_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M10 | `fra_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M11 | `ger_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M12 | `ita_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M13 | `jap_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M14 | `usa_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |
| M15 | `non_major_propaganda_campaigns_scripted_gui.gui` | (i) CHROME-ONLY |

### Vanilla overlays (23 files) — V1…V23 from §2, all **(i) CHROME-ONLY**

`GER_inner_circle_scripted_gui`, `GER_inner_circle_hidden_scripted_gui`, `GER_rk_scripted_gui`,
`ger_monroe_doctrine_scripted_gui`, `JAP_imperial_influence_scripted_gui`,
`sov_paranoia_system_scripted_gui`, `sov_propaganda_campaigns_scripted_gui`,
`usa_congress_scripted_gui`, `swe_rearnament_system_scripted_gui`, `swe_train_the_sheltered_gui`,
`AST_cabinet_trust_scripted_gui`, `ast_right_vs_left_campaign_scripted_gui`,
`ast_right_vs_left_campaign_empty_scripted_gui`, `aus_antischluss_measures_scripted_gui`,
`bul_internal_factions_scripted_gui`, `gre_internal_factions_scripted_gui`,
`CZE_industry_focus_scripted_gui`, `SIA_khana_ratsadon_scripted_gui`,
`sia_movie_theater_campaigns_scripted_gui`, `RAJ_famine`, `RAJ_eic_tax_fraud`,
`raj_local_leaders_investments_decision_ui`, `war_escalation_scripted_gui`.

### Verdict tally

| verdict | count | which |
|---|---:|---|
| **(i) CHROME-ONLY** | **33** | M5, M8–M15 (9 mod) + V1–V23 (23 vanilla) … = 9 + 23 = **32**, plus naval M5 already counted → see note |
| **(ii) PARTIALLY BINDABLE** | **3** | M2 (topbarresources), M3 (topbarvp), M4 (vp_scripted_gui) |
| **(iii) STATIC DECORATION** | **1** | M6 (SWOA_focus_background) |
| **(iv) DEV-TOOL / INERT** | **2** | M1 (horsttool — mutation panel, 4 bound readouts), M7 (game_check) |

> **Tally reconciliation (exact):** CHROME-ONLY = M5 + M8 + M9 + M10 + M11 + M12 + M13 + M14 + M15
> (**9 mod**) + V1…V23 (**23 vanilla**) = **32**. PARTIALLY BINDABLE = **3** (M2,M3,M4).
> STATIC DECORATION = **1** (M6). DEV-TOOL/INERT = **2** (M1,M7). **32 + 3 + 1 + 2 = 38.** Every one
> of the 38 assigned overlays is in exactly one bucket; **none dropped, none unreadable.**

**Bindable-content summary:** only **5 overlays carry any bound datum** — M2 (6 resource amounts →
`economy.resources`), M3 + M4 (VP score banners → GLOBAL `world.variables`; per-city tint →
`states.controller`/`map.vp_by_state`), and M1 (compliance/resistance readouts →
`states.<sel>.occupation.*`). The other **33** render faithful chrome with a `scripted content not
captured` badge over their value/dynamic-frame slots, because their backing variables are
**country-scoped** (not in the GLOBAL-only `world.variables`/`world.flags` channel) — the systematic
true-gap class whose closure is **(a) per-country script-var recording + (c) `common/scripted_guis/`
mod-aware parse**.

---

## 5. Codex round

**Asked** (Mode-A, read-only, `model_reasoning_effort=xhigh`, `service_tier=priority`): (a) is every
overlay in the assigned set accounted for with none dropped; (b) is the bind-vs-chrome-only verdict
correct given that `world.variables`/`world.flags` (GLOBAL) is the only scripted-var channel and there
is no per-country var bag; (c) is the closure path (parse `common/scripted_guis/` + record
`world.variables`, extended to a country-scoped var bag) correct.

**Result** (`codex exec`, read-only, `model_reasoning_effort=xhigh`, `service_tier=priority`, exit 0,
non-empty):

* **(a) AGREED** — every overlay accounted for; 15 mod + 23 vanilla = 38; buckets sum 32 + 3 + 1 + 2 = 38;
  no dropped item.
* **(b) AGREED** — bind-vs-chrome-only verdict correct: VP binds via GLOBAL `global.allies_score` /
  `global.axis_score`; the country-scoped national systems (war_escalation, usa_congress, propaganda,
  paranoia, etc.) are correctly CHROME-ONLY because the snapshot has no country-scoped var capture.
* **(c) AGREED, with one caveat** — parsing `common/scripted_guis/*.txt` mod-aware is the right
  element→var discovery path, and `world.variables` covers GLOBALs; a per-country `script_variables`
  bag closes most of the country-scoped mass **but not** temp-var / GUI-script-function cases like
  `naval_scripted_gui` — those need separately modeled (per-admiral fleet) data or stay chrome-only.

**Refuted:** none (no factual/logical error flagged). **Fixed:** folded Codex's (c) caveat into the §3
headline (the "Codex caveat" note: var-bag closes persistent country-vars; naval temp-vars/script-
functions close via live fleet/command extraction, bucket (a) — not the var bag). All three questions
AGREED; the one nuance was additive, now incorporated.
