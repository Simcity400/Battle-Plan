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
//! * **Org bar fraction** — UNKNOWN / GAP: `DivisionUnitRow` carries the raw
//!   `organisation` value (e.g. 42.0 points) but NO `max_organisation`, so the
//!   `GFX_mapicon_unit_bar_org` fill fraction (`org / max_org`) cannot be
//!   computed. `organisation` is surfaced raw; `org_bar_fraction` is always
//!   `None`. Filed: `REPLAY_DATA_REQUIREMENTS.md` item 3 / `EXTRACTION_NEEDS.md`
//!   A15 (needs a max-organisation datum).
//! * **Badges (A15)** — the on-map badge slots are supply / orders / dig-in /
//!   training / attrition / active-ability (`interface/mapicons.gui`, A15). Of
//!   these, `DivisionUnitRow` serves ONLY the orders signal (`moving`) and the
//!   combat signal (`in_combat`); supply / dig-in / training(experience) /
//!   attrition / active-ability are NOT surfaced by the view at all
//!   ([`ServedBadge::NotSurfaced`], a recording-side gap filed in A15). Absence
//!   is honest: a not-served badge is badge-absent, never defaulted on/off.

use replay_core::{DivisionUnitRow, FleetUnitRow, Leaf, ViewStatus, WingUnitRow, WorldFrame};
use std::collections::BTreeSet;

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

/// The A15 badge slot set for one counter. Only `orders` and `combat` are
/// served by `DivisionUnitRow`; the remainder are structurally `NotSurfaced`
/// (recording-side gaps, filed under A15) and so always badge-absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterBadges {
    /// Orders badge, from the division `moving` leaf: `Measured(true)` = has a
    /// move order, `Measured(false)` = no move order (`GFX_mapicon_unit_no_orders`).
    /// The full orders KIND (area-defense / invalid-orders) is not distinguished
    /// by the view — only move-vs-not is served.
    pub orders: ServedBadge,
    /// Combat indicator, from the division `in_combat` leaf.
    pub combat: ServedBadge,
    /// Low-supply badge (A15) — `NotSurfaced`: `supply_status` is not in
    /// `DivisionUnitRow`.
    pub low_supply: ServedBadge,
    /// Dig-in badge (A15) — `NotSurfaced`: `dig_in` is not in `DivisionUnitRow`.
    pub dig_in: ServedBadge,
    /// Training badge (A15) — `NotSurfaced`: division `experience` is not in
    /// `DivisionUnitRow`.
    pub training: ServedBadge,
    /// Attrition-active badge (A15) — `NotSurfaced`: no attrition signal in the
    /// view.
    pub attrition: ServedBadge,
    /// Active-ability badge (A15) — `NotSurfaced`: no per-unit active-ability
    /// signal in the view.
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
    /// `GFX_mapicon_unit_bar_str` fill: `strength` when in `[0,1]`, else `None`.
    pub str_bar_fraction: Option<f32>,
    /// `GFX_mapicon_unit_bar_org` fill: always `None` — no max-org served (GAP,
    /// A15). See module note.
    pub org_bar_fraction: Option<f32>,
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
        organisation: measured_f64(&row.organisation),
        str_bar_fraction: strength_bar_fraction(strength),
        // GAP (A15): no max-organisation served → no org bar fill.
        org_bar_fraction: None,
        // Land counters aggregate a stack COUNT at render (CAP6), not a
        // per-division datum.
        count: None,
        badges: CounterBadges {
            orders: ServedBadge::from_bool_leaf(&row.moving),
            combat: ServedBadge::from_bool_leaf(&row.in_combat),
            low_supply: ServedBadge::NotSurfaced,
            dig_in: ServedBadge::NotSurfaced,
            training: ServedBadge::NotSurfaced,
            attrition: ServedBadge::NotSurfaced,
            active_ability: ServedBadge::NotSurfaced,
        },
    }
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
        str_bar_fraction: None,
        org_bar_fraction: None,
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
        str_bar_fraction: None,
        org_bar_fraction: None,
        count: measured_i64(&row.plane_count),
        badges: CounterBadges::all_not_surfaced(),
    }
}

/// Build the counter-content set for one world frame. When `pov_tag` is
/// `Some(tag)`, only units whose measured `owner == tag` are included (own-unit
/// counters — visibility of foreign units is a separate concern, see
/// `fx_pov_visibility`); when `None`, every served unit is bound. Each domain
/// carries its source-view [`ViewStatus`]; a non-`Present` domain yields an
/// empty list (fail-closed).
pub fn counter_contents_from_frame(frame: &WorldFrame, pov_tag: Option<&str>) -> CounterContentSet {
    let keep = |owner: &Leaf<String>| match pov_tag {
        Some(tag) => owner_is(owner, tag),
        None => true,
    };
    let land = frame
        .division_units()
        .iter()
        .filter(|d| keep(&d.owner))
        .map(land_counter_content)
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

    fn land_row(role: Option<&str>, strength: Leaf<f64>, moving: Leaf<bool>) -> DivisionUnitRow {
        DivisionUnitRow {
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
            moving,
            move_target_province: Leaf::Absent,
            move_progress: Leaf::Measured(0.0),
            in_combat: Leaf::Measured(false),
            equipment: Leaf::Measured(vec![]),
        }
    }

    #[test]
    fn land_counter_binds_name_symbol_bars_and_orders() {
        let c = land_counter_content(&land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(true),
        ));
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
        // GAP: org bar fraction never computed (no max served).
        assert_eq!(c.org_bar_fraction, None);
        assert_eq!(c.count, None);
        assert_eq!(c.badges.orders, ServedBadge::Measured(true));
        assert_eq!(c.badges.combat, ServedBadge::Measured(false));
    }

    #[test]
    fn a15_gap_badges_are_not_surfaced_never_defaulted() {
        let c = land_counter_content(&land_row(
            Some("infantry"),
            Leaf::Measured(0.95),
            Leaf::Measured(false),
        ));
        for badge in [
            c.badges.low_supply,
            c.badges.dig_in,
            c.badges.training,
            c.badges.attrition,
            c.badges.active_ability,
        ] {
            assert_eq!(badge, ServedBadge::NotSurfaced);
            assert!(!badge.is_active());
        }
    }

    #[test]
    fn absent_leaves_render_fail_closed_none_never_default() {
        // No role, absent strength, absent moving.
        let c = land_counter_content(&land_row(None, Leaf::Absent, Leaf::Absent));
        assert_eq!(c.nato_symbol_key, None);
        assert_eq!(c.symbol_source, NatoKeySource::LandRoleUnavailable);
        assert_eq!(c.strength, None);
        assert_eq!(c.str_bar_fraction, None);
        assert_eq!(c.badges.orders, ServedBadge::AbsentThisHour);
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
