//! U1 — on-map unit-counter **CONTENT** binding (REPLAY_PRODUCTION_PLAN §6a
//! wave U1 / `UI_COVERAGE_MATRIX` C1 / `EXTRACTION_NEEDS` A15).
//!
//! This module is the *population* seam for what an on-map unit counter
//! **displays** — its name, strength/org bars, NATO symbol, owner colour key,
//! and badge states — for every land division / naval task force / air wing the
//! `replay_core` unit views serve at one hour. It is the CONTENT half of unit
//! rendering; the counter's **screen-space geometry / stacking / selection** is
//! CAP6-gated and deliberately NOT in scope here (see `map_viewer::unit_placement`
//! for the placement half and `map_viewer::unit_counter_assets` for the decoded
//! asset chain — both READ-ONLY references for this lane).
//!
//! ## Data path (Constraint 2 — typed views only)
//!
//! Every value comes through a typed `replay_core` view leaf
//! ([`WorldFrame::division_units`] / [`fleets`](WorldFrame::fleets) /
//! [`wings`](WorldFrame::wings)); no recording bytes are parsed. Mirroring
//! `binding.rs`, a leaf that is `Absent`/`Unobtained`, or a domain whose
//! [`ViewStatus`] is not `Present`, renders the **fail-closed absent form** (a
//! `None` field / empty list / `ServedBadge::AbsentThisHour`), never a fabricated
//! default (Constraint 3).
//!
//! ## FILE / FIT / UNKNOWN labels (per field)
//!
//! * **Owner colour key** — FILE (recording): the unit's own `owner` tag. The
//!   counter's country stripe (`GFX_mapicon_unit_flag_stripe`, decoded in
//!   `unit_counter_assets`) is tinted by the owner's flag/map colour; the tag →
//!   RGB mapping is engine-derived and NOT available in this decoupled context,
//!   so the KEY carried here is the raw owner tag (fail-closed `None` when
//!   unmeasured). Resolving the tag to a colour is a later render step.
//! * **NATO symbol key (land)** — FILE (naming law): the on-map division symbol
//!   sprite is `GFX_unit_<role>_icon_medium_white` in `interface/subuniticons.gfx`
//!   (prefix/suffix cited from `unit_counter_assets`). Derived from the division
//!   `template_role`. The precise `template_role → map_icon_category` mapping
//!   (per `common/units/*` `sub_unit` files) is a refinement not yet threaded;
//!   for the served roles the direct key resolves. Naval/air counters use
//!   separate symbol systems (ship silhouettes / air-mission icons), NOT the
//!   subuniticons division mapping named by this lane, so their symbol key is
//!   `None` with the reason recorded ([`NatoKeySource`]).
//! * **Strength bar fraction** — FILE-derived: the division `strength` leaf is a
//!   `0..=1` strength RATIO in the schema, so it doubles as the `GFX_mapicon_unit_bar_str`
//!   fill. Surfaced verbatim as `strength`; `str_bar_fraction` is that value only
//!   when it lies in `[0,1]` (a bar fill must; a value outside flags a non-ratio
//!   strength and yields `None`, fail-closed).
//! * **Org bar fraction** — exact only when the row carries both
//!   `organisation` and `max_organisation`; invalid or absent operands produce
//!   `None`. No template/doctrine reconstruction is used. Extended compact D20
//!   rows carry the authenticated maximum; retained legacy rows fail closed.
//! * **Badges (A15)** — D20 primary-order linkage replaces the old movement
//!   proxy; order kind is admitted only through a same-hour D54 identity and
//!   membership join. A36 active abilities join by exact `division:<id>`
//!   membership. Exact-a729 D20 counter-supply status drives low-supply frames;
//!   experience is never substituted for the engine's exercising predicate.

use replay_core::{
    ArmyBattleplansView, AuthenticatedUiPass, CommandAbilityEnvelope, DivisionPrimaryOrderExact,
    DivisionUnitRow, ExactProvinceSupplyTooltipView, FleetUnitRow, HudCommandExecution,
    HudCommandIdentity, Leaf, PoliticsDetailView, UiPassOperation, ViewStatus, WingUnitRow,
    WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// NATO symbol naming law (FILE: interface/subuniticons.gfx).
// ---------------------------------------------------------------------------

/// The on-map division-symbol sprite name prefix (FILE:
/// `interface/subuniticons.gfx`, mirrored from
/// `map_viewer::unit_counter_assets`). A division symbol is
/// `<PREFIX><role><SUFFIX>`.
pub const NATO_LAND_SYMBOL_PREFIX: &str = "GFX_unit_";
/// The on-map division-symbol sprite name suffix (the `_medium_white` on-map
/// variant selected by `on_map_division_counter_inventory_from_resolver`).
pub const NATO_LAND_SYMBOL_SUFFIX: &str = "_icon_medium_white";

/// A small FILE-cited sample of vanilla land-symbol keys (from the
/// `unit_counter_assets` vanilla decode test) usable as a deterministic
/// resolution oracle without the live install. NOT exhaustive — the full set is
/// the 125 resolved symbols in `subuniticons.gfx`; this is the subset the
/// fixtures + reference test exercise.
pub const KNOWN_LAND_SYMBOL_KEYS_SAMPLE: &[&str] = &[
    "GFX_unit_infantry_icon_medium_white",
    "GFX_unit_medium_armor_icon_medium_white",
    "GFX_unit_light_armor_icon_medium_white",
    "GFX_unit_artillery_icon_medium_white",
];

/// Normalise a `template_role` to the sprite-name token (lowercase, spaces →
/// underscores). Pure string law; does not assert the token is a real symbol.
fn normalize_role_token(role: &str) -> String {
    role.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect()
}

/// Derive the on-map division NATO symbol key from a `template_role` (FILE
/// naming law). Returns `None` for an empty role (fail-closed — no key is
/// fabricated for a unit with no role).
pub fn nato_land_symbol_key(role: &str) -> Option<String> {
    let token = normalize_role_token(role);
    if token.is_empty() {
        return None;
    }
    Some(format!(
        "{NATO_LAND_SYMBOL_PREFIX}{token}{NATO_LAND_SYMBOL_SUFFIX}"
    ))
}

/// How a counter's NATO symbol key was (or was not) derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NatoKeySource {
    /// FILE: land division symbol from `template_role` via the subuniticons law.
    LandTemplateRole,
    /// The land `template_role` leaf was not measured — no key.
    LandRoleUnavailable,
    /// Naval counter: the ship-silhouette symbol system is separate from the
    /// subuniticons division mapping this lane binds; symbol key not derived
    /// here (documented). The `dominant_hull_class` is still carried as the
    /// content hint on [`CounterContent::symbol_hint`].
    NavalHullClassNotMapped,
    /// Air counter: the air-mission icon system is separate from subuniticons;
    /// symbol key not derived here. `mission_kind` is carried as the hint.
    AirMissionNotMapped,
}

// ---------------------------------------------------------------------------
// Badges (A15).
// ---------------------------------------------------------------------------

/// One counter badge signal, honest about provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServedBadge {
    /// The view carries this signal and it was measured this hour.
    Measured(bool),
    /// The view carries this signal but it was `Absent`/`Unobtained` this hour
    /// (fail-closed: badge-absent, never defaulted).
    AbsentThisHour,
    /// The `replay_core` unit view does NOT surface this A15 signal at all — a
    /// recording-side gap (`EXTRACTION_NEEDS` A15). Always badge-absent; filed.
    NotSurfaced,
}

impl ServedBadge {
    /// From a served `Leaf<bool>`: measured → `Measured`, else `AbsentThisHour`.
    fn from_bool_leaf(leaf: &Leaf<bool>) -> ServedBadge {
        match leaf {
            Leaf::Measured(v) => ServedBadge::Measured(*v),
            Leaf::Absent | Leaf::Unobtained => ServedBadge::AbsentThisHour,
        }
    }

    /// `true` only when measured-true (a lit badge). `AbsentThisHour` and
    /// `NotSurfaced` are both unlit.
    pub fn is_active(&self) -> bool {
        matches!(self, ServedBadge::Measured(true))
    }
}

/// The A15 badge slot set for one counter. Primary-order linkage, combat,
/// exact-a729 attrition, and exact affected-division abilities are served;
/// unproven visibility predicates stay fail-closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterBadges {
    /// Orders badge from D20's primary-order link. Authenticated null is false;
    /// army/army-group is true; unresolved linkage is absent this hour.
    pub orders: ServedBadge,
    /// Combat indicator, from the division `in_combat` leaf.
    pub combat: ServedBadge,
    /// Exact low-supply badge from the dedicated engine counter scalar and
    /// authenticated frame thresholds (never from current/max inventory).
    pub low_supply: ServedBadge,
    /// Exact post-update dig-in control visibility/frame from the game UI.
    pub dig_in: ServedBadge,
    /// Exact post-update training control visibility from the game UI.
    pub training: ServedBadge,
    /// Exact post-update invalid-orders control visibility from the game UI.
    pub invalid_orders: ServedBadge,
    /// Attrition-active badge (A15), derived from the exact-a729 ATR1 carrier
    /// when the current frame authenticates that source.
    pub attrition: ServedBadge,
    /// Active-ability badge from A36's exact affected-division identities.
    pub active_ability: ServedBadge,
}

impl CounterBadges {
    /// All badges absent/not-surfaced — the fail-closed default used for naval
    /// and air counters (whose views serve none of the A15 land badge signals).
    fn all_not_surfaced() -> CounterBadges {
        CounterBadges {
            orders: ServedBadge::NotSurfaced,
            combat: ServedBadge::NotSurfaced,
            low_supply: ServedBadge::NotSurfaced,
            dig_in: ServedBadge::NotSurfaced,
            training: ServedBadge::NotSurfaced,
            invalid_orders: ServedBadge::NotSurfaced,
            attrition: ServedBadge::NotSurfaced,
            active_ability: ServedBadge::NotSurfaced,
        }
    }
}

// ---------------------------------------------------------------------------
// Counter content record.
// ---------------------------------------------------------------------------

/// Which unit view a counter's content came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterDomain {
    Land,
    Naval,
    Air,
}

/// The bound CONTENT of one on-map unit counter at one hour. Screen-space
/// geometry/stacking is out of scope (CAP6); this is purely what the counter
/// *shows*.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterContent {
    /// Stable unit identity (`div_uid` / `task_force_uid` / `wing_uid`).
    pub uid: String,
    pub domain: CounterDomain,
    /// Owner tag, or `None` when unmeasured (fail-closed).
    pub owner: Option<String>,
    /// Country colour KEY = the owner tag (see module note on the tag → colour
    /// step). `None` when the owner is unmeasured.
    pub country_color_key: Option<String>,
    /// Display name the counter shows: land = `template_name`; air = wing
    /// `name`; naval = `None` (the task-force name is NOT served by
    /// `FleetUnitRow` — gap; the `uid` is the stable identity).
    pub name: Option<String>,
    /// Derived NATO symbol sprite key (land only), else `None` (see `symbol_source`).
    pub nato_symbol_key: Option<String>,
    pub symbol_source: NatoKeySource,
    /// Content hint feeding the domain's own symbol system when `nato_symbol_key`
    /// is `None`: naval = `dominant_hull_class`, air = `mission_kind`. `None`
    /// for land (the key itself carries the role) or when unmeasured.
    pub symbol_hint: Option<String>,
    /// Raw measured strength (land = `0..1` strength ratio; naval/air not served
    /// at the counter level → `None`).
    pub strength: Option<f64>,
    /// Raw measured organisation (land = raw points; naval/air not served → `None`).
    pub organisation: Option<f64>,
    /// Exact recorded maximum; never reconstructed from templates/doctrines.
    pub max_organisation: Option<f64>,
    /// `GFX_mapicon_unit_bar_str` fill: `strength` when in `[0,1]`, else `None`.
    pub str_bar_fraction: Option<f32>,
    /// `GFX_mapicon_unit_bar_org` fill from exact current/max operands only.
    pub org_bar_fraction: Option<f32>,
    /// Source-owned supply operands and their exact same-row ratio. Replay
    /// never mixes sources to manufacture a missing ratio.
    pub current_supply: Option<f64>,
    pub max_supply: Option<f64>,
    pub supply_ratio: Option<f64>,
    /// Exact-a729 counter warning scalar (not current/max supply inventory).
    pub counter_supply_status: Option<f64>,
    /// `GFX_mapicon_unit_low_supply` sprite frame: 3/2/1, or `None` hidden.
    pub low_supply_frame: Option<u8>,
    /// Latest authenticated PST1 formatter observation for this division's
    /// province and POV observer in the current replay frame.
    pub remaining_supply: Leaf<f64>,
    /// Recorded experience is surfaced without calling it an active-training
    /// predicate.
    pub experience: Option<f64>,
    pub dig_in: Option<f64>,
    pub dig_in_cap: Option<f64>,
    pub dig_in_fraction: Option<f32>,
    /// `GFX_mapicon_unit_dig_in` frame: 1 building, 2 capped; absent when the
    /// complete training/friendly aggregate predicate cannot be evaluated.
    pub dig_in_frame: Option<u8>,
    /// Exact D20 primary-order to D54 order-kind join.
    pub current_order_kind: Leaf<i32>,
    /// Active A36 ability ids exactly joined to this division.
    pub active_ability_kinds: Leaf<Vec<String>>,
    /// The counter's numeric badge: naval = `ship_count`, air = `plane_count`;
    /// `None` for land (a land counter shows a stack COUNT that is a CAP6
    /// aggregation, not a per-division datum).
    pub count: Option<i64>,
    pub badges: CounterBadges,
}

/// The per-hour counter-content sets, split by domain, optionally POV-filtered.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterContentSet {
    /// `Some(tag)` when owner-filtered to a POV; `None` for all units.
    pub pov_tag: Option<String>,
    pub land: Vec<CounterContent>,
    pub naval: Vec<CounterContent>,
    pub air: Vec<CounterContent>,
    pub land_status: ViewStatus,
    pub naval_status: ViewStatus,
    pub air_status: ViewStatus,
}

impl CounterContentSet {
    /// Every bound counter across the three domains (land, then naval, then air).
    pub fn all(&self) -> impl Iterator<Item = &CounterContent> {
        self.land.iter().chain(&self.naval).chain(&self.air)
    }

    /// Total counter count across all domains.
    pub fn len(&self) -> usize {
        self.land.len() + self.naval.len() + self.air.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A measured owner string equals `pov_tag`.
fn owner_is(owner: &Leaf<String>, pov_tag: &str) -> bool {
    matches!(owner, Leaf::Measured(tag) if tag == pov_tag)
}

/// Measured owner string as `Option<String>` (fail-closed `None`).
fn owner_opt(owner: &Leaf<String>) -> Option<String> {
    owner.measured().cloned()
}

/// Raw measured `f64` leaf → `Option<f64>`.
fn measured_f64(leaf: &Leaf<f64>) -> Option<f64> {
    leaf.measured().copied()
}

/// Measured `String` leaf → `Option<String>`.
fn measured_string(leaf: &Leaf<String>) -> Option<String> {
    leaf.measured().cloned()
}

/// Measured `i64` leaf → `Option<i64>`.
fn measured_i64(leaf: &Leaf<i64>) -> Option<i64> {
    leaf.measured().copied()
}

/// A strength ratio → bar fill: `Some(v as f32)` only when `0.0 <= v <= 1.0`
/// (a bar fill must lie in `[0,1]`; anything else flags a non-ratio value and is
/// fail-closed to `None`).
fn strength_bar_fraction(strength: Option<f64>) -> Option<f32> {
    strength
        .filter(|v| (0.0..=1.0).contains(v))
        .map(|v| v as f32)
}

/// Exact authored counter roots accepted from the selected-Present HUD
/// frontier. These strings are not a heuristic: the a729 owner classifier
/// authenticates them together with `interface/mapicons.gui` and the
/// `mapicons_container` runtime root before a HUD command can be emitted.
const COUNTER_GUI_DEFINITION: &str = "interface/mapicons.gui";
const COUNTER_RUNTIME_ROOT: &str = "mapicons_container";
const LAND_COUNTER_SURFACE: &str = "unit_counter";
const NAVAL_COUNTER_SURFACE: &str = "unit_counter_navy";
const RAILWAY_COUNTER_SURFACE: &str = "unit_counter_railway_gun";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AuthenticatedCounterSurface {
    Land,
    Naval,
    RailwayGun,
}

/// Coverage of exact, same-hour inputs consumed by the counter binding. A
/// count is evidence coverage, not a default: zero can mean a measured-empty
/// domain, while `CounterSelectedPresentBinding` itself is absent when the
/// selected-Present counter frontier was not authenticated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CounterFieldCoverage {
    pub land_rows: usize,
    pub exact_supply_rows: usize,
    pub exact_organisation_bar_rows: usize,
    pub exact_order_kind_rows: usize,
    pub evaluated_low_supply_rows: usize,
    pub evaluated_dig_in_rows: usize,
    pub evaluated_training_rows: usize,
    pub evaluated_invalid_orders_rows: usize,
    pub evaluated_attrition_rows: usize,
    pub exact_active_ability_rows: usize,
}

/// Typed counter state consumed beside the real authenticated selected-Present
/// HUD commands. The commands remain the sole pixel authority. This receipt
/// proves only that the semantic counter model and the exact authored counter
/// owners were consumed from the same validated `WorldFrame`/Present; it does
/// not invent a per-unit join from an opaque GUI instance token.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterSelectedPresentBinding {
    /// Typed land rows are consumed only when this Present authenticated at
    /// least one authored land-counter owner.
    pub land_contents: Option<Vec<CounterContent>>,
    /// Typed naval rows are consumed only when this Present authenticated at
    /// least one authored naval-counter owner.
    pub naval_contents: Option<Vec<CounterContent>>,
    /// There is no authenticated air-counter surface in the current a729 HUD
    /// owner vocabulary. Keep this structurally absent instead of borrowing a
    /// land/naval/railway owner as provenance.
    pub air_contents: Option<Vec<CounterContent>>,
    pub authenticated_owner_count: usize,
    pub authenticated_command_count: usize,
    pub authenticated_land_owner_count: usize,
    pub authenticated_naval_owner_count: usize,
    pub authenticated_railway_gun_owner_count: usize,
    /// Every currently implemented exact field belongs to a land division
    /// row, so coverage is unavailable unless a land owner was authenticated.
    pub land_field_coverage: Option<CounterFieldCoverage>,
}

fn authenticated_counter_surface(
    identity: &HudCommandIdentity,
) -> Result<Option<AuthenticatedCounterSurface>, String> {
    let surface = match identity.authored_surface_identity.as_str() {
        LAND_COUNTER_SURFACE => AuthenticatedCounterSurface::Land,
        NAVAL_COUNTER_SURFACE => AuthenticatedCounterSurface::Naval,
        RAILWAY_COUNTER_SURFACE => AuthenticatedCounterSurface::RailwayGun,
        _ => return Ok(None),
    };
    if identity.runtime_root != COUNTER_RUNTIME_ROOT
        || identity.selected_gui_definition.normalized_game_relative_path
            != COUNTER_GUI_DEFINITION
    {
        return Err(
            "selected-Present counter owner is not bound to the authenticated mapicons root"
                .into(),
        );
    }
    Ok(Some(surface))
}

fn counter_field_coverage(contents: &CounterContentSet) -> CounterFieldCoverage {
    let evaluated = |badge: ServedBadge| badge != ServedBadge::AbsentThisHour
        && badge != ServedBadge::NotSurfaced;
    CounterFieldCoverage {
        land_rows: contents.land.len(),
        exact_supply_rows: contents
            .land
            .iter()
            .filter(|row| {
                row.current_supply.is_some()
                    && row.max_supply.is_some()
                    && row.supply_ratio.is_some()
            })
            .count(),
        exact_organisation_bar_rows: contents
            .land
            .iter()
            .filter(|row| row.org_bar_fraction.is_some())
            .count(),
        exact_order_kind_rows: contents
            .land
            .iter()
            .filter(|row| row.current_order_kind.is_measured())
            .count(),
        evaluated_low_supply_rows: contents
            .land
            .iter()
            .filter(|row| evaluated(row.badges.low_supply))
            .count(),
        evaluated_dig_in_rows: contents
            .land
            .iter()
            .filter(|row| evaluated(row.badges.dig_in))
            .count(),
        evaluated_training_rows: contents
            .land
            .iter()
            .filter(|row| evaluated(row.badges.training))
            .count(),
        evaluated_invalid_orders_rows: contents
            .land
            .iter()
            .filter(|row| evaluated(row.badges.invalid_orders))
            .count(),
        evaluated_attrition_rows: contents
            .land
            .iter()
            .filter(|row| evaluated(row.badges.attrition))
            .count(),
        exact_active_ability_rows: contents
            .land
            .iter()
            .filter(|row| row.active_ability_kinds.is_measured())
            .count(),
    }
}

type AuthenticatedCounterDomainContent = (
    Option<Vec<CounterContent>>,
    Option<Vec<CounterContent>>,
    Option<Vec<CounterContent>>,
    Option<CounterFieldCoverage>,
);

/// Select semantic content only for counter surfaces authenticated in this
/// Present. Air remains unavailable until its own authored surface is proven;
/// it may never inherit land, naval, or railway provenance.
fn authenticated_counter_domain_content(
    contents: CounterContentSet,
    authenticated_land_owner_count: usize,
    authenticated_naval_owner_count: usize,
) -> AuthenticatedCounterDomainContent {
    let land_field_coverage = (authenticated_land_owner_count > 0)
        .then(|| counter_field_coverage(&contents));
    let land_contents = (authenticated_land_owner_count > 0).then_some(contents.land);
    let naval_contents = (authenticated_naval_owner_count > 0).then_some(contents.naval);
    (land_contents, naval_contents, None, land_field_coverage)
}

/// Bind typed counter content to the authenticated selected-Present HUD
/// consumer. No counter command means a feature-local fail-closed error; the
/// legacy CAP6 captured-map pass is never consulted or enabled here.
pub fn counter_binding_for_selected_present(
    frame: &WorldFrame,
    pov_tag: Option<&str>,
    pass: &AuthenticatedUiPass,
) -> Result<CounterSelectedPresentBinding, String> {
    pass.validate()?;
    let render = frame
        .production_render()
        .ok_or("counter HUD binding has no production render frame")?;
    if render.selected_present.computed_sha256()? != pass.selected_present_receipt_sha256 {
        return Err("counter HUD binding crosses selected-Present receipts".into());
    }

    let mut owners = BTreeSet::new();
    let mut commands = 0usize;
    for operation in &pass.ui_pass_operations {
        let UiPassOperation::HudCommand { command } = operation else {
            continue;
        };
        let Some(surface) = authenticated_counter_surface(&command.identity)? else {
            continue;
        };
        if !matches!(&command.execution, HudCommandExecution::ExactPipeline { .. }) {
            return Err(
                "authenticated counter owner does not carry an exact-pipeline HUD command".into(),
            );
        }
        owners.insert((surface, command.identity.engine_instance_token));
        commands += 1;
    }
    if owners.is_empty() || commands == 0 {
        return Err("selected-Present HUD frontier has no authenticated counter owner".into());
    }

    let mut owner_counts = BTreeMap::new();
    for (surface, _) in &owners {
        *owner_counts.entry(*surface).or_insert(0usize) += 1;
    }
    let contents = counter_contents_from_frame(frame, pov_tag);
    if owner_counts
        .get(&AuthenticatedCounterSurface::Land)
        .copied()
        .unwrap_or(0)
        > 0
        && contents.land_status != ViewStatus::Present
    {
        return Err("authenticated land-counter HUD owners have no present typed land view".into());
    }
    if owner_counts
        .get(&AuthenticatedCounterSurface::Naval)
        .copied()
        .unwrap_or(0)
        > 0
        && contents.naval_status != ViewStatus::Present
    {
        return Err(
            "authenticated naval-counter HUD owners have no present typed naval view".into(),
        );
    }
    let authenticated_land_owner_count = owner_counts
        .get(&AuthenticatedCounterSurface::Land)
        .copied()
        .unwrap_or(0);
    let authenticated_naval_owner_count = owner_counts
        .get(&AuthenticatedCounterSurface::Naval)
        .copied()
        .unwrap_or(0);
    let authenticated_railway_gun_owner_count = owner_counts
        .get(&AuthenticatedCounterSurface::RailwayGun)
        .copied()
        .unwrap_or(0);
    let (land_contents, naval_contents, air_contents, land_field_coverage) =
        authenticated_counter_domain_content(
            contents,
            authenticated_land_owner_count,
            authenticated_naval_owner_count,
        );
    Ok(CounterSelectedPresentBinding {
        land_contents,
        naval_contents,
        // No authenticated air surface is currently admitted. Railway owners
        // likewise remain owner-only until a typed railway-gun content join
        // exists; neither may borrow another surface's provenance.
        air_contents,
        authenticated_owner_count: owners.len(),
        authenticated_command_count: commands,
        authenticated_land_owner_count,
        authenticated_naval_owner_count,
        authenticated_railway_gun_owner_count,
        land_field_coverage,
    })
}

/// Exact bounded fraction. Invalid, non-finite, negative, zero-denominator,
/// and numerator-above-denominator pairs fail closed; presentation never
/// clamps an unproven value into range.
fn exact_fraction(numerator: Option<f64>, denominator: Option<f64>) -> Option<f32> {
    match (numerator, denominator) {
        (Some(n), Some(d)) if n.is_finite() && d.is_finite() && n >= 0.0 && d > 0.0 && n <= d => {
            Some((n / d) as f32)
        }
        _ => None,
    }
}

/// Admit supply only as one internally consistent same-row triple. A lone
/// legacy `max_supply` or historical normalized `supply_status` value cannot
/// be relabelled as the current D20 numerator/denominator contract.
fn exact_supply_operands(row: &DivisionUnitRow) -> (Option<f64>, Option<f64>, Option<f64>) {
    match (
        measured_f64(&row.current_supply),
        measured_f64(&row.max_supply),
        measured_f64(&row.supply_ratio),
    ) {
        (Some(current), Some(maximum), Some(ratio))
            if current.is_finite()
                && maximum.is_finite()
                && ratio.is_finite()
                && current >= 0.0
                && maximum > 0.0
                && current <= maximum
                && (0.0..=1.0).contains(&ratio)
                && (ratio - current / maximum).abs() <= f64::EPSILON * 8.0 =>
        {
            (Some(current), Some(maximum), Some(ratio))
        }
        _ => (None, None, None),
    }
}

/// Exact organization bar law: divide current by the authenticated denominator
/// returned by wrapper vslot `+0x130` (whose source pointer is at wrapper
/// `+0x138`) and clamp the result to the 0..1 bar interval.
fn exact_org_bar_fraction(current: Option<f64>, maximum: Option<f64>) -> Option<f32> {
    match (current, maximum) {
        (Some(current), Some(maximum))
            if current.is_finite() && maximum.is_finite() && current >= 0.0 && maximum > 0.0 =>
        {
            Some((current / maximum).clamp(0.0, 1.0) as f32)
        }
        _ => None,
    }
}

/// engine fixed-5 scale: 3 below .20, 2 below .35, 1 below .50, hidden at/above
/// .50. Invalid/unmeasured values remain absent rather than being clamped.
fn exact_low_supply_frame(status: &Leaf<f64>) -> (ServedBadge, Option<u8>) {
    match status {
        Leaf::Measured(value) if value.is_finite() && (0.0..=1.0).contains(value) => {
            let scaled = *value * 100_000.0;
            let rounded = scaled.round();
            if (scaled - rounded).abs() > 1.0e-6 {
                return (ServedBadge::AbsentThisHour, None);
            }
            let raw = rounded as u32;
            let frame = if raw < 20_000 {
                Some(3)
            } else if raw < 35_000 {
                Some(2)
            } else if raw < 50_000 {
                Some(1)
            } else {
                None
            };
            (ServedBadge::Measured(frame.is_some()), frame)
        }
        Leaf::Measured(_) | Leaf::Unobtained => (ServedBadge::AbsentThisHour, None),
        Leaf::Absent => (ServedBadge::NotSurfaced, None),
    }
}

fn exact_dig_in_ui_frame(frame: &Leaf<u32>) -> (ServedBadge, Option<u8>) {
    match frame {
        Leaf::Measured(0) => (ServedBadge::Measured(false), None),
        Leaf::Measured(value @ 1..=2) => {
            (ServedBadge::Measured(true), Some(*value as u8))
        }
        Leaf::Measured(_) | Leaf::Absent | Leaf::Unobtained => {
            (ServedBadge::AbsentThisHour, None)
        }
    }
}

fn primary_order_badge(order: &Leaf<Option<DivisionPrimaryOrderExact>>) -> ServedBadge {
    match order {
        Leaf::Measured(Some(_)) => ServedBadge::Measured(true),
        Leaf::Measured(None) => ServedBadge::Measured(false),
        Leaf::Unobtained => ServedBadge::AbsentThisHour,
        Leaf::Absent => ServedBadge::NotSurfaced,
    }
}

fn exact_order_kind(row: &DivisionUnitRow, battleplans: &ArmyBattleplansView) -> Leaf<i32> {
    let (army_id, engine_order_instance_id) = match (&row.primary_order, &row.army_id) {
        (Leaf::Measured(None), _)
        | (Leaf::Measured(Some(DivisionPrimaryOrderExact::ArmyGroup)), _) => {
            return Leaf::Absent;
        }
        (
            Leaf::Measured(Some(DivisionPrimaryOrderExact::Army {
                engine_order_instance_id,
            })),
            Leaf::Measured(army_id),
        ) => (*army_id, *engine_order_instance_id),
        (Leaf::Absent, _) => return Leaf::Absent,
        _ => return Leaf::Unobtained,
    };
    match battleplans.status {
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
        ViewStatus::Present => {
            let mut matches = battleplans.orders.iter().filter(|order| {
                order.key.owner.kind == replay_core::ArmyBattleplanOwnerKind::Army
                    && order.key.owner.id == army_id
                    && order.key.engine_order_instance_id == engine_order_instance_id
                    && order.assigned_division_ids.contains(&row.raw_division_id)
            });
            match (matches.next(), matches.next()) {
                (Some(order), None) => Leaf::Measured(order.order_type),
                _ => Leaf::Unobtained,
            }
        }
    }
}

fn exact_active_ability_kinds(
    row: &DivisionUnitRow,
    envelope: Option<&Leaf<CommandAbilityEnvelope>>,
) -> Leaf<Vec<String>> {
    let envelope = match envelope {
        Some(Leaf::Measured(envelope)) if envelope.validates_exact_authority() => envelope,
        Some(Leaf::Measured(_)) | Some(Leaf::Unobtained) => return Leaf::Unobtained,
        Some(Leaf::Absent) | None => return Leaf::Absent,
    };
    let identity = format!("division:{}", row.raw_division_id);
    let kinds = envelope
        .snapshot
        .rows
        .iter()
        .filter(|ability| {
            ability.active
                && ability
                    .affected_division_ids
                    .iter()
                    .any(|candidate| candidate == &identity)
        })
        .map(|ability| ability.ability_id.clone())
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        // A36 is selected-context scoped. Absence from this one context is not
        // proof that no other leader/context affects the division.
        Leaf::Absent
    } else {
        Leaf::Measured(kinds)
    }
}

fn exact_remaining_supply(
    row: &DivisionUnitRow,
    observer_tag_db_id: Option<&Leaf<u32>>,
    tooltip: &ExactProvinceSupplyTooltipView,
) -> Leaf<f64> {
    let province_id = match &row.location_province {
        Leaf::Measured(value) => *value,
        Leaf::Unobtained => return Leaf::Unobtained,
        Leaf::Absent => return Leaf::Absent,
    };
    let observer_tag_db_id = match observer_tag_db_id {
        Some(Leaf::Measured(value)) => *value,
        Some(Leaf::Unobtained) => return Leaf::Unobtained,
        Some(Leaf::Absent) | None => return Leaf::Absent,
    };
    match tooltip.status {
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
        ViewStatus::Present => match &tooltip.rows {
            Leaf::Measured(rows) => rows
                .iter()
                .filter(|observation| {
                    observation.province_id == province_id
                        && observation.observer_tag_db_id == observer_tag_db_id
                })
                .max_by_key(|observation| observation.sequence)
                .map_or(Leaf::Absent, |observation| {
                    Leaf::Measured(observation.remaining_supply)
                }),
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Absent => Leaf::Absent,
        },
    }
}

/// Bind one land division row to its counter content.
pub fn land_counter_content(row: &DivisionUnitRow) -> CounterContent {
    let owner = owner_opt(&row.owner);
    let (nato_symbol_key, symbol_source) = match row.template_role.measured() {
        Some(role) => match nato_land_symbol_key(role) {
            Some(key) => (Some(key), NatoKeySource::LandTemplateRole),
            None => (None, NatoKeySource::LandRoleUnavailable),
        },
        None => (None, NatoKeySource::LandRoleUnavailable),
    };
    let strength = measured_f64(&row.strength);
    let organisation = measured_f64(&row.organisation);
    let max_organisation = measured_f64(&row.max_organisation);
    let (current_supply, max_supply, supply_ratio) = exact_supply_operands(row);
    let dig_in = measured_f64(&row.dig_in);
    let dig_in_cap = measured_f64(&row.dig_in_cap);
    let (low_supply_badge, low_supply_frame) =
        exact_low_supply_frame(&row.counter_supply_status);
    let training_badge = ServedBadge::from_bool_leaf(&row.counter_training_visible);
    let invalid_orders_badge =
        ServedBadge::from_bool_leaf(&row.counter_invalid_orders_visible);
    let (dig_in_badge, dig_in_frame) =
        exact_dig_in_ui_frame(&row.counter_dig_in_frame);
    let attrition_badge = match &row.attrition {
        Leaf::Measured(attrition) => ServedBadge::from_bool_leaf(&attrition.badge_active),
        Leaf::Unobtained => ServedBadge::AbsentThisHour,
        Leaf::Absent => ServedBadge::NotSurfaced,
    };
    CounterContent {
        uid: row.div_uid.clone(),
        domain: CounterDomain::Land,
        country_color_key: owner.clone(),
        owner,
        name: measured_string(&row.template_name),
        nato_symbol_key,
        symbol_source,
        symbol_hint: None,
        strength,
        organisation,
        max_organisation,
        str_bar_fraction: strength_bar_fraction(strength),
        org_bar_fraction: exact_org_bar_fraction(organisation, max_organisation),
        current_supply,
        max_supply,
        supply_ratio,
        counter_supply_status: measured_f64(&row.counter_supply_status),
        low_supply_frame,
        remaining_supply: Leaf::Absent,
        experience: measured_f64(&row.experience),
        dig_in,
        dig_in_cap,
        dig_in_fraction: exact_fraction(dig_in, dig_in_cap),
        dig_in_frame,
        current_order_kind: Leaf::Absent,
        active_ability_kinds: Leaf::Absent,
        // Land counters aggregate a stack COUNT at render (CAP6), not a
        // per-division datum.
        count: None,
        badges: CounterBadges {
            orders: primary_order_badge(&row.primary_order),
            combat: ServedBadge::from_bool_leaf(&row.in_combat),
            low_supply: low_supply_badge,
            dig_in: dig_in_badge,
            training: training_badge,
            invalid_orders: invalid_orders_badge,
            attrition: attrition_badge,
            active_ability: ServedBadge::NotSurfaced,
        },
    }
}

/// Identity-only land counter authorized by D95 map visibility without exact
/// detail visibility. Every semantic field sourced from D20 (including owner,
/// role, bars, supply, orders, abilities, and badges) is replaced by one
/// constant fail-closed form so hidden values cannot influence replay output.
fn redacted_land_counter_content(row: &DivisionUnitRow) -> CounterContent {
    CounterContent {
        uid: row.div_uid.clone(),
        domain: CounterDomain::Land,
        owner: None,
        country_color_key: None,
        name: None,
        nato_symbol_key: None,
        symbol_source: NatoKeySource::LandRoleUnavailable,
        symbol_hint: None,
        strength: None,
        organisation: None,
        max_organisation: None,
        str_bar_fraction: None,
        org_bar_fraction: None,
        current_supply: None,
        max_supply: None,
        supply_ratio: None,
        counter_supply_status: None,
        low_supply_frame: None,
        remaining_supply: Leaf::Unobtained,
        experience: None,
        dig_in: None,
        dig_in_cap: None,
        dig_in_fraction: None,
        dig_in_frame: None,
        current_order_kind: Leaf::Unobtained,
        active_ability_kinds: Leaf::Unobtained,
        count: None,
        badges: CounterBadges {
            orders: ServedBadge::AbsentThisHour,
            combat: ServedBadge::AbsentThisHour,
            low_supply: ServedBadge::AbsentThisHour,
            dig_in: ServedBadge::AbsentThisHour,
            training: ServedBadge::AbsentThisHour,
            invalid_orders: ServedBadge::AbsentThisHour,
            attrition: ServedBadge::AbsentThisHour,
            active_ability: ServedBadge::AbsentThisHour,
        },
    }
}

fn land_counter_content_with_joins(
    row: &DivisionUnitRow,
    battleplans: &ArmyBattleplansView,
    politics: Option<&PoliticsDetailView>,
    tooltip: &ExactProvinceSupplyTooltipView,
) -> CounterContent {
    let mut content = land_counter_content(row);
    content.current_order_kind = exact_order_kind(row, battleplans);
    let ability_envelope = politics.map(|politics| &politics.command_ability_envelope);
    content.active_ability_kinds = exact_active_ability_kinds(row, ability_envelope);
    content.remaining_supply =
        exact_remaining_supply(row, politics.map(|politics| &politics.tag_db_id), tooltip);
    content.badges.active_ability = match &content.active_ability_kinds {
        Leaf::Measured(kinds) if !kinds.is_empty() => ServedBadge::Measured(true),
        Leaf::Measured(_) | Leaf::Unobtained => ServedBadge::AbsentThisHour,
        Leaf::Absent if matches!(ability_envelope, Some(Leaf::Measured(_))) => {
            ServedBadge::AbsentThisHour
        }
        Leaf::Absent => ServedBadge::NotSurfaced,
    };
    content
}

/// Bind one naval task-force row to its counter content.
pub fn naval_counter_content(row: &FleetUnitRow) -> CounterContent {
    let owner = owner_opt(&row.owner);
    CounterContent {
        uid: row.task_force_uid.clone(),
        domain: CounterDomain::Naval,
        country_color_key: owner.clone(),
        owner,
        // GAP: FleetUnitRow does not surface the task-force name.
        name: None,
        nato_symbol_key: None,
        symbol_source: NatoKeySource::NavalHullClassNotMapped,
        symbol_hint: measured_string(&row.dominant_hull_class),
        // Fleet-level strength/org bars are not served (per-ship data exists in
        // `row.ships` but the task-force aggregation law is UNKNOWN — gap).
        strength: None,
        organisation: None,
        max_organisation: None,
        str_bar_fraction: None,
        org_bar_fraction: None,
        current_supply: None,
        max_supply: None,
        supply_ratio: None,
        counter_supply_status: None,
        low_supply_frame: None,
        remaining_supply: Leaf::Absent,
        experience: None,
        dig_in: None,
        dig_in_cap: None,
        dig_in_fraction: None,
        dig_in_frame: None,
        current_order_kind: Leaf::Absent,
        active_ability_kinds: Leaf::Absent,
        count: measured_i64(&row.ship_count),
        badges: CounterBadges::all_not_surfaced(),
    }
}

/// Bind one air-wing row to its counter content.
pub fn air_counter_content(row: &WingUnitRow) -> CounterContent {
    let owner = owner_opt(&row.owner);
    CounterContent {
        uid: row.wing_uid.clone(),
        domain: CounterDomain::Air,
        country_color_key: owner.clone(),
        owner,
        name: measured_string(&row.name),
        nato_symbol_key: None,
        symbol_source: NatoKeySource::AirMissionNotMapped,
        symbol_hint: measured_string(&row.mission_kind),
        strength: None,
        organisation: None,
        max_organisation: None,
        str_bar_fraction: None,
        org_bar_fraction: None,
        current_supply: None,
        max_supply: None,
        supply_ratio: None,
        counter_supply_status: None,
        low_supply_frame: None,
        remaining_supply: Leaf::Absent,
        experience: None,
        dig_in: None,
        dig_in_cap: None,
        dig_in_fraction: None,
        dig_in_frame: None,
        current_order_kind: Leaf::Absent,
        active_ability_kinds: Leaf::Absent,
        count: measured_i64(&row.plane_count),
        badges: CounterBadges::all_not_surfaced(),
    }
}

/// Build the counter-content set for one world frame. When `pov_tag` is
/// `Some(tag)`, own divisions and exact D95 map-visible foreign divisions are
/// admitted; foreign D20 details are redacted unless D95 marks them exact.
/// Naval and air remain own-unit-only. When `None`, every served unit is bound
/// for omniscient observer mode. Each domain carries its source-view
/// [`ViewStatus`].
pub fn counter_contents_from_frame(frame: &WorldFrame, pov_tag: Option<&str>) -> CounterContentSet {
    let keep = |owner: &Leaf<String>| match pov_tag {
        Some(tag) => owner_is(owner, tag),
        None => true,
    };
    let battleplans = frame.army_battleplans();
    let land = frame
        .division_units()
        .iter()
        .filter_map(|row| {
            let exact_details = match pov_tag {
                Some(tag) => frame.division_pov_exact_details(tag, row)?,
                None => true,
            };
            if !exact_details {
                return Some(redacted_land_counter_content(row));
            }
            let ability_tag = pov_tag.or_else(|| row.owner.measured().map(String::as_str));
            let politics = ability_tag.and_then(|tag| frame.politics_detail(tag));
            Some(land_counter_content_with_joins(
                row,
                &battleplans,
                politics,
                frame.exact_province_supply_tooltip(),
            ))
        })
        .collect();
    let naval = frame
        .fleets()
        .iter()
        .filter(|f| keep(&f.owner))
        .map(naval_counter_content)
        .collect();
    let air = frame
        .wings()
        .iter()
        .filter(|w| keep(&w.owner))
        .map(air_counter_content)
        .collect();
    CounterContentSet {
        pov_tag: pov_tag.map(str::to_string),
        land,
        naval,
        air,
        land_status: frame.division_units_status(),
        naval_status: frame.fleets_status(),
        air_status: frame.wings_status(),
    }
}

/// Convenience: own-unit counter content for a POV at one frame.
pub fn counter_contents_for_pov(frame: &WorldFrame, pov_tag: &str) -> CounterContentSet {
    counter_contents_from_frame(frame, Some(pov_tag))
}

/// Every served unit's counter content at one frame (no POV filter).
pub fn all_counter_contents(frame: &WorldFrame) -> CounterContentSet {
    counter_contents_from_frame(frame, None)
}

// ---------------------------------------------------------------------------
// NATO symbol resolution rate (gate metric).
// ---------------------------------------------------------------------------

/// NATO-symbol key resolution over a set of counters: `(resolved, keyed)` where
/// `keyed` counts counters that produced a symbol key at all (land counters with
/// a measured role) and `resolved` counts those whose key is present in
/// `known_keys` (e.g. the `unit_counter_assets` division-symbol name set, or the
/// FILE-cited [`KNOWN_LAND_SYMBOL_KEYS_SAMPLE`]). Naval/air counters carry no
/// subuniticons key and are excluded from both totals.
pub fn nato_symbol_resolution<'a>(
    counters: impl IntoIterator<Item = &'a CounterContent>,
    known_keys: &BTreeSet<String>,
) -> (usize, usize) {
    let mut resolved = 0usize;
    let mut keyed = 0usize;
    for c in counters {
        if let Some(key) = &c.nato_symbol_key {
            keyed += 1;
            if known_keys.contains(key) {
                resolved += 1;
            }
        }
    }
    (resolved, keyed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hud_identity(surface: &str, root: &str, definition: &str) -> HudCommandIdentity {
        HudCommandIdentity {
            authored_surface_identity: surface.into(),
            runtime_root: root.into(),
            selected_gui_definition: replay_core::UsedContentKey {
                effective_vfs_manifest_sha256: "11".repeat(32),
                normalized_game_relative_path: definition.into(),
                winning_layer_ordinal: 0,
                layer_kind: replay_core::ContentLayerKind::Base,
                layer_id: "vanilla".into(),
                source_sha256: "22".repeat(32),
                payload_sha256: "33".repeat(32),
                payload_bytes: 1,
            },
            engine_instance_token: 7,
            child_path: vec![0, 1],
            element_type: "iconType".into(),
            name: "counter_bg".into(),
            paint_ordinal: 0,
        }
    }

    #[test]
    fn selected_present_counter_identity_requires_exact_authored_owner_join() {
        let exact = hud_identity(
            LAND_COUNTER_SURFACE,
            COUNTER_RUNTIME_ROOT,
            COUNTER_GUI_DEFINITION,
        );
        assert_eq!(
            authenticated_counter_surface(&exact).unwrap(),
            Some(AuthenticatedCounterSurface::Land)
        );

        let wrong_root = hud_identity(
            LAND_COUNTER_SURFACE,
            LAND_COUNTER_SURFACE,
            COUNTER_GUI_DEFINITION,
        );
        assert!(authenticated_counter_surface(&wrong_root).is_err());
        let wrong_definition = hud_identity(
            LAND_COUNTER_SURFACE,
            COUNTER_RUNTIME_ROOT,
            "interface/unitview.gui",
        );
        assert!(authenticated_counter_surface(&wrong_definition).is_err());

        let unrelated = hud_identity("topbar", "topbar", "interface/topbar.gui");
        assert_eq!(authenticated_counter_surface(&unrelated).unwrap(), None);
    }

    #[test]
    fn nato_land_symbol_key_follows_subuniticons_naming() {
        assert_eq!(
            nato_land_symbol_key("infantry").as_deref(),
            Some("GFX_unit_infantry_icon_medium_white")
        );
        // Case + whitespace normalisation.
        assert_eq!(
            nato_land_symbol_key("Medium Armor").as_deref(),
            Some("GFX_unit_medium_armor_icon_medium_white")
        );
        // Empty role → no fabricated key.
        assert_eq!(nato_land_symbol_key(""), None);
        assert_eq!(nato_land_symbol_key("   "), None);
    }

    #[test]
    fn strength_bar_fraction_only_accepts_ratio_range() {
        assert_eq!(strength_bar_fraction(Some(0.95)), Some(0.95));
        assert_eq!(strength_bar_fraction(Some(0.0)), Some(0.0));
        assert_eq!(strength_bar_fraction(Some(1.0)), Some(1.0));
        // Out-of-ratio (e.g. a raw HP strength) is fail-closed.
        assert_eq!(strength_bar_fraction(Some(42.0)), None);
        assert_eq!(strength_bar_fraction(Some(-0.1)), None);
        assert_eq!(strength_bar_fraction(None), None);
    }

    #[test]
    fn exact_fraction_rejects_missing_invalid_and_overfull_operands() {
        assert_eq!(exact_fraction(Some(42.0), Some(84.0)), Some(0.5));
        assert_eq!(exact_fraction(Some(0.0), Some(84.0)), Some(0.0));
        assert_eq!(exact_fraction(Some(85.0), Some(84.0)), None);
        assert_eq!(exact_fraction(Some(-1.0), Some(84.0)), None);
        assert_eq!(exact_fraction(Some(1.0), Some(0.0)), None);
        assert_eq!(exact_fraction(None, Some(84.0)), None);
    }

    #[test]
    fn organisation_bar_matches_engine_clamp() {
        assert_eq!(exact_org_bar_fraction(Some(42.0), Some(84.0)), Some(0.5));
        assert_eq!(exact_org_bar_fraction(Some(85.0), Some(84.0)), Some(1.0));
        assert_eq!(exact_org_bar_fraction(Some(-1.0), Some(84.0)), None);
        assert_eq!(exact_org_bar_fraction(Some(1.0), Some(0.0)), None);
    }

    #[test]
    fn low_supply_frames_match_exact_a729_boundaries() {
        for (value, frame) in [
            (0.0, Some(3)),
            (0.19999, Some(3)),
            (0.20, Some(2)),
            (0.34999, Some(2)),
            (0.35, Some(1)),
            (0.49999, Some(1)),
            (0.50, None),
            (1.0, None),
        ] {
            let (badge, actual) = exact_low_supply_frame(&Leaf::Measured(value));
            assert_eq!(actual, frame, "status {value}");
            assert_eq!(badge, ServedBadge::Measured(frame.is_some()));
        }
        assert_eq!(
            exact_low_supply_frame(&Leaf::Absent),
            (ServedBadge::NotSurfaced, None)
        );
        assert_eq!(
            exact_low_supply_frame(&Leaf::Unobtained),
            (ServedBadge::AbsentThisHour, None)
        );
    }

    fn land_row(role: Option<&str>, strength: Leaf<f64>, moving: Leaf<bool>) -> DivisionUnitRow {
        let primary_order = match &moving {
            Leaf::Measured(true) => Leaf::Measured(Some(DivisionPrimaryOrderExact::Army {
                engine_order_instance_id: 77,
            })),
            Leaf::Measured(false) => Leaf::Measured(None),
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Absent => Leaf::Absent,
        };
        DivisionUnitRow {
            raw_division_id: 101,
            raw_division_type: Leaf::Measured(1),
            div_uid: "101:1".into(),
            owner: Leaf::Measured("GER".into()),
            location_province: Leaf::Measured(1000),
            state_id: Leaf::Measured(10),
            template_id: Leaf::Measured(1),
            template_type: Leaf::Measured(1),
            template_name: Leaf::Measured("Fixture Infantry".into()),
            template_role: match role {
                Some(r) => Leaf::Measured(r.into()),
                None => Leaf::Absent,
            },
            strength,
            organisation: Leaf::Measured(42.0),
            max_organisation: Leaf::Absent,
            experience: Leaf::Absent,
            current_supply: Leaf::Absent,
            max_supply: Leaf::Absent,
            counter_supply_status: Leaf::Absent,
            supply_ratio: Leaf::Absent,
            dig_in: Leaf::Absent,
            dig_in_cap: Leaf::Absent,
            counter_training_visible: Leaf::Absent,
            counter_dig_in_frame: Leaf::Absent,
            counter_invalid_orders_visible: Leaf::Absent,
            army_id: Leaf::Absent,
            primary_order,
            moving,
            move_target_province: Leaf::Absent,
            move_progress: Leaf::Measured(0.0),
            in_combat: Leaf::Measured(false),
            attrition: Leaf::Absent,
            equipment: Leaf::Measured(vec![]),
        }
    }

    #[test]
    fn land_counter_binds_name_symbol_bars_and_orders() {
        let mut row = land_row(Some("infantry"), Leaf::Measured(0.95), Leaf::Measured(true));
        row.max_organisation = Leaf::Measured(84.0);
        row.current_supply = Leaf::Measured(12.5);
        row.max_supply = Leaf::Measured(20.0);
        row.counter_supply_status = Leaf::Measured(0.34999);
        row.supply_ratio = Leaf::Measured(0.625);
        row.experience = Leaf::Measured(0.3);
        row.dig_in = Leaf::Measured(5.0);
        row.dig_in_cap = Leaf::Measured(10.0);
        let c = land_counter_content(&row);
        assert_eq!(c.domain, CounterDomain::Land);
        assert_eq!(c.owner.as_deref(), Some("GER"));
        assert_eq!(c.country_color_key.as_deref(), Some("GER"));
        assert_eq!(c.name.as_deref(), Some("Fixture Infantry"));
        assert_eq!(
            c.nato_symbol_key.as_deref(),
            Some("GFX_unit_infantry_icon_medium_white")
        );
        assert_eq!(c.symbol_source, NatoKeySource::LandTemplateRole);
        assert_eq!(c.strength, Some(0.95));
        assert_eq!(c.organisation, Some(42.0));
        assert_eq!(c.str_bar_fraction, Some(0.95));
        assert_eq!(c.max_organisation, Some(84.0));
        assert_eq!(c.org_bar_fraction, Some(0.5));
        assert_eq!(c.current_supply, Some(12.5));
        assert_eq!(c.max_supply, Some(20.0));
        assert_eq!(c.supply_ratio, Some(0.625));
        assert_eq!(c.counter_supply_status, Some(0.34999));
        assert_eq!(c.low_supply_frame, Some(2));
        assert_eq!(c.badges.low_supply, ServedBadge::Measured(true));
        assert_eq!(c.experience, Some(0.3));
        assert_eq!(c.dig_in_fraction, Some(0.5));
        assert_eq!(c.count, None);
        assert_eq!(c.badges.orders, ServedBadge::Measured(true));
        assert_eq!(c.badges.combat, ServedBadge::Measured(false));
    }

    #[test]
    fn non_exact_d95_land_counter_redacts_every_d20_detail() {
        let mut row = land_row(Some("infantry"), Leaf::Measured(0.95), Leaf::Measured(true));
        row.max_organisation = Leaf::Measured(84.0);
        row.experience = Leaf::Measured(0.75);
        row.current_supply = Leaf::Measured(12.0);
        row.counter_training_visible = Leaf::Measured(true);
        row.in_combat = Leaf::Measured(true);
        let content = redacted_land_counter_content(&row);
        assert_eq!(content.uid, "101:1");
        assert_eq!(content.domain, CounterDomain::Land);
        assert!(content.owner.is_none());
        assert!(content.country_color_key.is_none());
        assert!(content.name.is_none());
        assert!(content.nato_symbol_key.is_none());
        assert!(content.strength.is_none());
        assert!(content.organisation.is_none());
        assert!(content.max_organisation.is_none());
        assert!(content.experience.is_none());
        assert!(content.current_supply.is_none());
        assert_eq!(content.current_order_kind, Leaf::Unobtained);
        assert_eq!(content.active_ability_kinds, Leaf::Unobtained);
        assert_eq!(content.badges.combat, ServedBadge::AbsentThisHour);
        assert_eq!(content.badges.training, ServedBadge::AbsentThisHour);
    }

    #[test]
    fn selected_present_domain_content_never_borrows_another_surface_owner() {
        let contents = || CounterContentSet {
            pov_tag: Some("GER".into()),
            land: vec![land_counter_content(&land_row(
                Some("infantry"),
                Leaf::Measured(0.95),
                Leaf::Measured(false),
            ))],
            naval: Vec::new(),
            air: Vec::new(),
            land_status: ViewStatus::Present,
            naval_status: ViewStatus::Present,
            air_status: ViewStatus::Present,
        };

        let (land, naval, air, coverage) =
            authenticated_counter_domain_content(contents(), 0, 1);
        assert!(land.is_none(), "a naval owner cannot authenticate land rows");
        assert_eq!(naval.as_ref().map(Vec::len), Some(0));
        assert!(air.is_none(), "air has no authenticated owner vocabulary");
        assert!(coverage.is_none(), "land fields require a land owner");

        let (land, naval, air, coverage) =
            authenticated_counter_domain_content(contents(), 1, 0);
        assert_eq!(land.as_ref().map(Vec::len), Some(1));
        assert!(naval.is_none(), "a land owner cannot authenticate naval rows");
        assert!(air.is_none());
        assert_eq!(coverage.map(|value| value.land_rows), Some(1));

        let (land, naval, air, coverage) =
            authenticated_counter_domain_content(contents(), 0, 0);
        assert!(land.is_none() && naval.is_none() && air.is_none() && coverage.is_none());
    }

    #[test]
    fn d20_primary_order_joins_d54_kind_only_on_exact_membership() {
        let mut row = land_row(Some("infantry"), Leaf::Measured(0.95), Leaf::Measured(true));
        row.army_id = Leaf::Measured(17);
        row.primary_order = Leaf::Measured(Some(DivisionPrimaryOrderExact::Army {
            engine_order_instance_id: 10_002,
        }));
        let mut view = ArmyBattleplansView {
            status: ViewStatus::Present,
            baseline_seen: true,
            omitted_non_exact_rows_this_hour: Some(0),
            path_geometry_omitted_rows_this_hour: Some(0),
            identity_or_active_omitted_rows_this_hour: Some(0),
            orders: vec![replay_core::ArmyBattleplanOrderExact {
                key: replay_core::ArmyBattleplanOrderKey {
                    owner: replay_core::ArmyBattleplanOwnerKey {
                        kind: replay_core::ArmyBattleplanOwnerKind::Army,
                        id: 17,
                    },
                    engine_order_instance_id: 10_002,
                    generation: 3,
                },
                slot_index: 0,
                order_type: 2,
                geometry: replay_core::ArmyBattleplanGeometry::PathProvinces(vec![1000, 1001]),
                assigned_division_ids: vec![101],
                active: true,
            }],
            diagnostic: None,
        };
        assert_eq!(exact_order_kind(&row, &view), Leaf::Measured(2));
        view.orders[0].assigned_division_ids = vec![999];
        assert_eq!(exact_order_kind(&row, &view), Leaf::Unobtained);
        view.status = ViewStatus::DomainUnavailable;
        assert_eq!(exact_order_kind(&row, &view), Leaf::Unobtained);
    }

    #[test]
    fn a36_active_ability_joins_only_exact_affected_division_identity() {
        const ENVELOPE: &str = include_str!(
            "../../../re_evidence/rendering_laws/XUI23/canonical_active_abilities_envelope.json"
        );
        let envelope: CommandAbilityEnvelope = serde_json::from_str(ENVELOPE).unwrap();
        let mut row = land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        );
        row.raw_division_id = 9;
        assert_eq!(
            exact_active_ability_kinds(&row, Some(&Leaf::Measured(envelope.clone()))),
            Leaf::Measured(vec!["force_attack".to_string()])
        );
        row.raw_division_id = 10;
        assert_eq!(
            exact_active_ability_kinds(&row, Some(&Leaf::Measured(envelope))),
            Leaf::Absent
        );
    }

    #[test]
    fn pst1_remaining_supply_joins_latest_exact_province_observer_pair() {
        let row = land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        );
        let tooltip = ExactProvinceSupplyTooltipView {
            status: ViewStatus::Present,
            authority: Some("exact_a729_province_supply_tooltip_formatter_sink".into()),
            hook_epoch: Some(1),
            drain_sequence: Some(3),
            dropped_rows: Some(0),
            rows: Leaf::Measured(vec![
                replay_core::ExactProvinceSupplyTooltipObservation {
                    province_id: 1000,
                    observer_tag_db_id: 47,
                    remaining_supply_raw: 125_000,
                    remaining_supply: 1.25,
                    fixed_scale: 100_000,
                    displayed_decimals: 2,
                    tick: 10,
                    sequence: 1,
                },
                replay_core::ExactProvinceSupplyTooltipObservation {
                    province_id: 1000,
                    observer_tag_db_id: 47,
                    remaining_supply_raw: 150_000,
                    remaining_supply: 1.5,
                    fixed_scale: 100_000,
                    displayed_decimals: 2,
                    tick: 11,
                    sequence: 2,
                },
            ]),
        };
        assert_eq!(
            exact_remaining_supply(&row, Some(&Leaf::Measured(47)), &tooltip),
            Leaf::Measured(1.5)
        );
        assert_eq!(
            exact_remaining_supply(&row, Some(&Leaf::Measured(48)), &tooltip),
            Leaf::Absent
        );
        let unavailable = ExactProvinceSupplyTooltipView {
            status: ViewStatus::DomainUnavailable,
            authority: None,
            hook_epoch: Some(1),
            drain_sequence: Some(3),
            dropped_rows: Some(1),
            rows: Leaf::Unobtained,
        };
        assert_eq!(
            exact_remaining_supply(&row, Some(&Leaf::Measured(47)), &unavailable),
            Leaf::Unobtained
        );
    }

    #[test]
    fn a15_unobserved_badges_are_fail_closed_never_defaulted() {
        let c = land_counter_content(&land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        ));
        for badge in [c.badges.low_supply, c.badges.attrition, c.badges.active_ability] {
            assert_eq!(badge, ServedBadge::NotSurfaced);
            assert!(!badge.is_active());
        }
        for badge in [c.badges.dig_in, c.badges.training] {
            assert_eq!(badge, ServedBadge::AbsentThisHour);
            assert!(!badge.is_active());
        }
    }

    #[test]
    fn evaluated_training_and_dig_in_badges_bind_exact_ui_result() {
        let mut row = land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        );
        row.counter_training_visible = Leaf::Measured(true);
        row.counter_dig_in_frame = Leaf::Measured(2);
        let c = land_counter_content(&row);
        assert_eq!(c.badges.training, ServedBadge::Measured(true));
        assert_eq!(c.badges.dig_in, ServedBadge::Measured(true));
        assert_eq!(c.dig_in_frame, Some(2));
    }

    #[test]
    fn absent_leaves_render_fail_closed_none_never_default() {
        // No role, absent strength, absent moving.
        let c = land_counter_content(&land_row(None, Leaf::Absent, Leaf::Absent));
        assert_eq!(c.nato_symbol_key, None);
        assert_eq!(c.symbol_source, NatoKeySource::LandRoleUnavailable);
        assert_eq!(c.strength, None);
        assert_eq!(c.str_bar_fraction, None);
        assert_eq!(c.badges.orders, ServedBadge::NotSurfaced);
    }

    #[test]
    fn nato_resolution_counts_only_keyed_land_counters() {
        let known: BTreeSet<String> = KNOWN_LAND_SYMBOL_KEYS_SAMPLE
            .iter()
            .map(|s| s.to_string())
            .collect();
        let resolvable = land_counter_content(&land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        ));
        let unknown_role = land_counter_content(&land_row(
            Some("space_marine"),
            Leaf::Measured(0.5),
            Leaf::Measured(false),
        ));
        let no_role = land_counter_content(&land_row(None, Leaf::Measured(0.5), Leaf::Absent));
        let counters = [resolvable, unknown_role, no_role];
        // 3 land counters, 2 keyed (roles present), 1 resolved (infantry).
        assert_eq!(nato_symbol_resolution(counters.iter(), &known), (1, 2));
    }
}
