//! `modes` — ALL map modes (Stage 2). Each mode is a **recolour function over the
//! same province-id raster**: it produces a per-province RGBA colour LUT (exactly the
//! buffer [`crate::map::PoliticalMap::update_lut`] consumes), so the GPU shader is
//! mode-agnostic — switching mode is just uploading a different LUT.
//!
//! ## Fidelity badges
//!
//! Every mode carries a [`Fidelity`] badge describing how faithful its colouring is to
//! the in-game map mode of the same name:
//!   * **Exact**   — the colouring is the real game state (owner, controller, VP,
//!                    buildings, resistance/compliance, diplomacy at country granularity).
//!   * **Derived** — computed from other exact data, not the game's own layer (front
//!                    lines from control adjacency).
//!   * **Proxy**   — coarser granularity than the game shows (air = strategic-region,
//!                    naval = sea-zone region, supply = province proxy / a proven wall).
//!   * **Unavailable** — the underlying data is `null`/absent in this snapshot; the WHOLE
//!                    map renders neutral and the badge says so (never fabricated).
//!
//! ## No-fabrication (the hard invariant, identical to the political LUT)
//!
//! Every LUT starts with EVERY entry = neutral terrain. A LAND province with **no data
//! for the active mode** is written neutral GREY ("land, no value") — it is NEVER given
//! an invented colour. Sea / lake / unassigned provinces are never touched by the land
//! loop and stay terrain. The two neutrals are kept distinct (Codex rule): "no data"
//! (grey) must read differently from "not land" (terrain). The Absent / Null / Measured
//! trichotomy of the snapshot survives into the colouring: a measured `0` (e.g. a state
//! with 0 factories) is a real datum and is coloured at the low end of the ramp; a
//! `null`/absent value is grey, never `0`-coloured.

use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::map::{CountryColors, MapIds, NEUTRAL_GREY, NEUTRAL_TERRAIN};

/// How faithful a mode's colouring is to the real in-game map mode. Surfaced as the
/// on-screen badge next to the active mode name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// The colouring IS the real decoded game state.
    Exact,
    /// Computed from other exact data, not the game's own layer.
    Derived,
    /// Coarser granularity than the game shows (region / sea-zone / a known wall).
    Proxy,
    /// The data is null/absent in this snapshot — the whole map is neutral.
    Unavailable,
}

impl Fidelity {
    pub fn badge(self) -> &'static str {
        match self {
            Fidelity::Exact => "exact",
            Fidelity::Derived => "derived",
            Fidelity::Proxy => "proxy",
            Fidelity::Unavailable => "unavailable",
        }
    }
}

/// The full set of map modes (plan §B.9 table). Order = the cycle order of the switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    /// owner — exact.
    Political,
    /// controller (+ contested hatch where owner != controller) — exact.
    Control,
    /// occupation resistance (0..100) — exact.
    Resistance,
    /// occupation compliance (0..100) — exact.
    Compliance,
    /// per-state victory points — exact.
    VictoryPoints,
    /// buildings of one type per state — exact.
    Buildings(BuildingKind),
    /// strategic resources per state — exact (often unavailable in a snapshot).
    Resources,
    /// per-province weather — exact values, proxy vs the game's visual mode.
    Weather,
    /// front lines derived from control adjacency — derived.
    FrontLines,
    /// faction / wars / subjects (country granularity) — exact.
    Diplomacy,
    /// intel / counter-intel country shading, derived from measured country intel.
    Intel,
    /// air superiority (strategic-region proxy) — proxy.
    Air,
    /// naval / sea-zone dominance (region proxy) — proxy.
    Naval,
    /// supply per province (proxy; exact is a proven wall) — proxy.
    Supply,
}

/// The building types a `Buildings` mode can colour (the per-state `buildings.*` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildingKind {
    Infrastructure,
    CivilianFactory,
    MilitaryFactory,
    Dockyard,
    AirBase,
    AntiAir,
}

impl BuildingKind {
    /// The snapshot `buildings.<key>` this kind maps to.
    pub fn snapshot_key(self) -> &'static str {
        match self {
            BuildingKind::Infrastructure => "infrastructure",
            BuildingKind::CivilianFactory => "industrial_complex",
            BuildingKind::MilitaryFactory => "arms_factory",
            BuildingKind::Dockyard => "dockyard",
            BuildingKind::AirBase => "air_base",
            BuildingKind::AntiAir => "anti_air_building",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            BuildingKind::Infrastructure => "infrastructure",
            BuildingKind::CivilianFactory => "civilian factories",
            BuildingKind::MilitaryFactory => "military factories",
            BuildingKind::Dockyard => "dockyards",
            BuildingKind::AirBase => "air bases",
            BuildingKind::AntiAir => "anti-air",
        }
    }
}

impl MapMode {
    /// The full cycle of modes the switcher walks (one representative building kind in
    /// the cycle; the others are reachable but kept out of the linear cycle to keep it
    /// short — the demo renders every building kind explicitly).
    pub const CYCLE: &'static [MapMode] = &[
        MapMode::Political,
        MapMode::Control,
        MapMode::Resistance,
        MapMode::Compliance,
        MapMode::VictoryPoints,
        MapMode::Buildings(BuildingKind::CivilianFactory),
        MapMode::Resources,
        MapMode::Weather,
        MapMode::FrontLines,
        MapMode::Diplomacy,
        MapMode::Intel,
        MapMode::Air,
        MapMode::Naval,
        MapMode::Supply,
    ];

    /// The next mode in the cycle (wraps). Used by the on-screen / key switcher.
    pub fn next(self) -> MapMode {
        let cyc = MapMode::CYCLE;
        // Match on the discriminant family so Buildings(any kind) advances to Resources.
        let idx = cyc.iter().position(|m| m.same_family(&self)).unwrap_or(0);
        cyc[(idx + 1) % cyc.len()]
    }
    pub fn prev(self) -> MapMode {
        let cyc = MapMode::CYCLE;
        let idx = cyc.iter().position(|m| m.same_family(&self)).unwrap_or(0);
        cyc[(idx + cyc.len() - 1) % cyc.len()]
    }
    /// Same mode family (Buildings(_) collapses regardless of kind) — for cycle indexing.
    fn same_family(&self, other: &MapMode) -> bool {
        matches!(
            (self, other),
            (MapMode::Buildings(_), MapMode::Buildings(_))
        ) || self == other
    }

    /// Human-readable mode name (for the on-screen switcher label).
    pub fn label(self) -> String {
        match self {
            MapMode::Political => "Political".into(),
            MapMode::Control => "Control".into(),
            MapMode::Resistance => "Resistance".into(),
            MapMode::Compliance => "Compliance".into(),
            MapMode::VictoryPoints => "Victory Points".into(),
            MapMode::Buildings(k) => format!("Buildings — {}", k.label()),
            MapMode::Resources => "Resources".into(),
            MapMode::Weather => "Weather".into(),
            // Honest label (Codex): this is a controller-BOUNDARY derivation (every
            // adjacency between two controllers), not the game's active-war battleplan
            // front — the `derived` badge reinforces it.
            MapMode::FrontLines => "Front Lines (controller boundary)".into(),
            MapMode::Diplomacy => "Diplomacy".into(),
            MapMode::Intel => "Intel".into(),
            MapMode::Air => "Air".into(),
            MapMode::Naval => "Naval".into(),
            MapMode::Supply => "Supply".into(),
        }
    }

    /// A short slug used for output filenames (`modes_<slug>.png`).
    pub fn slug(self) -> String {
        match self {
            MapMode::Political => "political".into(),
            MapMode::Control => "control".into(),
            MapMode::Resistance => "resistance".into(),
            MapMode::Compliance => "compliance".into(),
            MapMode::VictoryPoints => "victory_points".into(),
            MapMode::Buildings(k) => format!("buildings_{}", k.snapshot_key()),
            MapMode::Resources => "resources".into(),
            MapMode::Weather => "weather".into(),
            MapMode::FrontLines => "front_lines".into(),
            MapMode::Diplomacy => "diplomacy".into(),
            MapMode::Intel => "intel".into(),
            MapMode::Air => "air".into(),
            MapMode::Naval => "naval".into(),
            MapMode::Supply => "supply".into(),
        }
    }

    /// The *static* fidelity class of this mode versus the in-game version. (For modes
    /// whose data may be entirely missing in a snapshot, [`recolor`] may DOWNGRADE the
    /// returned badge to `Unavailable` — this is the inherent class assuming data.)
    pub fn fidelity(self) -> Fidelity {
        match self {
            MapMode::Political
            | MapMode::Control
            | MapMode::Resistance
            | MapMode::Compliance
            | MapMode::VictoryPoints
            | MapMode::Buildings(_)
            | MapMode::Resources
            | MapMode::Diplomacy => Fidelity::Exact,
            MapMode::Intel => Fidelity::Derived,
            MapMode::Weather => Fidelity::Proxy, // exact values, proxy vs game visual mode
            MapMode::FrontLines => Fidelity::Derived,
            MapMode::Air | MapMode::Naval | MapMode::Supply => Fidelity::Proxy,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-mode input context — gathered ONCE per snapshot from the typed domains.
// All maps are keyed by u32 state-id / province-id (normalized at the boundary,
// matching `map.rs`'s typed-id rule).
// ---------------------------------------------------------------------------

/// state id -> owner TAG (None = null/absent → neutral grey).
type OwnerMap = HashMap<u32, Option<String>>;
/// state id -> controller TAG.
type ControllerMap = HashMap<u32, Option<String>>;

/// All the per-snapshot inputs the recolour functions need, pre-extracted from the
/// snapshot's `states` / `provinces` / `map` / `diplomacy_world` domains. Building this
/// is O(states+provinces) once per tick; the recolour is then a cheap LUT fill.
pub struct ModeData {
    /// state id -> owner TAG (None = neutral).
    pub owner: OwnerMap,
    /// state id -> controller TAG (None = neutral).
    pub controller: ControllerMap,
    /// state id -> resistance (0..100), when an occupation block carries it.
    pub resistance: HashMap<u32, f64>,
    /// state id -> compliance (0..100).
    pub compliance: HashMap<u32, f64>,
    /// state id -> victory_points_total (measured; 0 is a real datum).
    pub victory_points: HashMap<u32, i64>,
    /// state id -> buildings map (key -> count). Measured 0 is real.
    pub buildings: HashMap<u32, HashMap<String, i64>>,
    /// state id -> total resources (sum of resources.*), when present (non-null).
    pub resources: HashMap<u32, f64>,
    /// province id -> controller TAG (province-level control, from map.control_by_province).
    pub province_control: HashMap<u32, String>,
    /// province id -> supply value, when present (the proxy; usually null = a wall).
    pub province_supply: HashMap<u32, f64>,
    /// faction membership: TAG -> faction colour (derived from faction ideology).
    pub faction_color: HashMap<String, [u8; 3]>,
    /// at-war set of TAGs (any side of any war) — for the diplomacy mode tint.
    pub at_war: std::collections::HashSet<String>,
    /// subject TAG -> overlord TAG (for the diplomacy subject tint).
    pub subject_of: HashMap<String, String>,
    /// country TAG -> measured intel/counter-intel signal for the Intel map mode.
    pub intel_score: HashMap<String, f64>,
    /// whether the snapshot carries any weather domain at all (else Unavailable).
    pub has_weather: bool,
    /// whether any air-superiority data is present (else Unavailable).
    pub has_air: bool,
    /// whether any naval region-control data is present (else Unavailable).
    pub has_naval: bool,
    /// front-line provinces precomputed from TRUE raster adjacency (see
    /// [`ModeData::with_front_adjacency`]). When `None`, the FrontLines mode falls back
    /// to the coarse state-level (owner != controller) frontier.
    pub front_provinces: Option<std::collections::HashSet<u32>>,
}

fn tag_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

impl ModeData {
    /// Extract everything from one snapshot's domains. `states` is the `states` object;
    /// `map` / `diplomacy_world` are optional top-level domains (None → those modes go
    /// Unavailable). This is the single Absent/Null/Measured-preserving boundary.
    pub fn from_snapshot(
        states: &Map<String, Value>,
        map_domain: Option<&Value>,
        diplomacy_world: Option<&Value>,
    ) -> ModeData {
        let mut owner = HashMap::new();
        let mut controller = HashMap::new();
        let mut resistance = HashMap::new();
        let mut compliance = HashMap::new();
        let mut victory_points = HashMap::new();
        let mut buildings: HashMap<u32, HashMap<String, i64>> = HashMap::new();
        let mut resources = HashMap::new();

        for (sid_str, sv) in states {
            let Ok(sid) = sid_str.parse::<u32>() else {
                continue;
            };
            owner.insert(sid, tag_of(sv.get("owner")));
            controller.insert(sid, tag_of(sv.get("controller")));

            // occupation.{resistance,compliance} — only when present & numeric.
            if let Some(occ) = sv.get("occupation").and_then(Value::as_object) {
                if let Some(r) = occ.get("resistance").and_then(Value::as_f64) {
                    resistance.insert(sid, r);
                }
                if let Some(c) = occ.get("compliance").and_then(Value::as_f64) {
                    compliance.insert(sid, c);
                }
            }

            // victory_points_total — a MEASURED integer (0 is real, null/absent skipped).
            if let Some(vp) = sv.get("victory_points_total").and_then(Value::as_i64) {
                victory_points.insert(sid, vp);
            }

            // buildings.* — measured counts. A PRESENT buildings object (even the empty
            // `{}` the producer emits for a state with zero buildings of any kind) is a
            // MEASURED datum: the state IS recorded, it just has no buildings → every
            // type reads measured-0 (ramp low), NOT grey "no data". So we insert on the
            // object's PRESENCE, not on non-emptiness (Codex-flagged: dropping `{}` would
            // wrongly grey a real all-zero state). Only a null/absent `buildings` (which
            // this producer never emits, but a future one might) stays out → grey.
            if let Some(b) = sv.get("buildings").and_then(Value::as_object) {
                let mut m = HashMap::new();
                for (k, v) in b {
                    // Only integer counts are kept. A per-key non-integer (e.g. an
                    // explicit `"arms_factory": null`) is dropped → that type reads as a
                    // missing-key measured 0 in this per-type view. The real producer
                    // never emits per-key nulls (it omits the key, or emits an explicit
                    // integer 0, or the whole-state empty `{}`), so this is exact here;
                    // documented as the one schema-evolution caveat (Codex-noted).
                    if let Some(n) = v.as_i64() {
                        m.insert(k.clone(), n);
                    }
                }
                buildings.insert(sid, m); // present object → measured (even if empty)
            }

            // resources.* — sum, ONLY when the resources block is a non-null object.
            if let Some(res) = sv.get("resources").and_then(Value::as_object) {
                let total: f64 = res.values().filter_map(Value::as_f64).sum();
                resources.insert(sid, total);
            }
        }

        // ---- map domain: province-level control + per-province supply. ----------
        let mut province_control = HashMap::new();
        let province_supply = HashMap::new();
        let mut has_weather = false;
        if let Some(map_obj) = map_domain.and_then(Value::as_object) {
            if let Some(cbp) = map_obj
                .get("control_by_province")
                .and_then(Value::as_object)
            {
                for (pid_str, v) in cbp {
                    if let (Ok(pid), Value::String(t)) = (pid_str.parse::<u32>(), v) {
                        province_control.insert(pid, t.clone());
                    }
                }
            }
            if map_obj.contains_key("weather") {
                has_weather = map_obj
                    .get("weather")
                    .map(|w| !w.is_null())
                    .unwrap_or(false);
            }
        }
        // per-province supply lives under a `provinces` domain if captured; usually null
        // (the proven wall). We surface it ONLY if some province carries a numeric supply.
        // (The caller passes the provinces domain separately via `with_provinces`.)

        // ---- diplomacy_world: factions, wars, subjects. -------------------------
        let mut faction_color = HashMap::new();
        let mut at_war = std::collections::HashSet::new();
        let mut subject_of = HashMap::new();
        let mut has_air = false;
        let mut has_naval = false;
        if let Some(dw) = diplomacy_world.and_then(Value::as_object) {
            if let Some(factions) = dw.get("factions").and_then(Value::as_object) {
                for fv in factions.values() {
                    let ideo = fv.get("ideology").and_then(Value::as_str).unwrap_or("");
                    let col = ideology_color(ideo);
                    if let Some(members) = fv.get("members").and_then(Value::as_array) {
                        for m in members {
                            if let Some(t) = m.as_str() {
                                faction_color.insert(t.to_string(), col);
                            }
                        }
                    }
                }
            }
            if let Some(wars) = dw.get("wars").and_then(Value::as_object) {
                for wv in wars.values() {
                    for side in ["a", "b"] {
                        if let Some(t) = wv.get(side).and_then(Value::as_str) {
                            at_war.insert(t.to_string());
                        }
                    }
                }
            }
            if let Some(subjects) = dw.get("subjects").and_then(Value::as_array) {
                for sv in subjects {
                    if let (Some(sub), Some(over)) = (
                        sv.get("subject").and_then(Value::as_str),
                        sv.get("overlord").and_then(Value::as_str),
                    ) {
                        subject_of.insert(sub.to_string(), over.to_string());
                    }
                }
            }
            // Back-compat for old/demo snapshots that temporarily nested these under
            // diplomacy_world. Live snapshots promote them to top-level `naval_air`;
            // callers should also chain `with_naval_air`.
            has_air = dw.contains_key("air_superiority");
            has_naval = dw.contains_key("region_control");
        }

        ModeData {
            owner,
            controller,
            resistance,
            compliance,
            victory_points,
            buildings,
            resources,
            province_control,
            province_supply,
            faction_color,
            at_war,
            subject_of,
            intel_score: HashMap::new(),
            has_weather,
            has_air,
            has_naval,
            front_provinces: None,
        }
    }

    /// Precompute the TRUE front-line province set from raster adjacency: a land
    /// province is on a front line if any orthogonal (4-neighbour) raster pixel belongs
    /// to a province with a DIFFERENT controller (province-level control). This is the
    /// "derived from control adjacency" frontier the plan specifies (still a derived
    /// proxy of the battleplan front, never the game's own front layer). Pure CPU over
    /// the id raster; O(width*height).
    pub fn with_front_adjacency(mut self, raster: &crate::map::ProvinceRaster) -> ModeData {
        let w = raster.width as usize;
        let h = raster.height as usize;
        let mut front = std::collections::HashSet::new();
        // controller of a province = its province-level controller (map domain), else
        // the controller of its state (state domain). A province with NO controller is
        // never on a front (we cannot assert a boundary we don't know).
        let ctrl_of = |pid: u32| -> Option<&str> {
            if let Some(t) = self.province_control.get(&pid) {
                return Some(t.as_str());
            }
            None
        };
        for y in 0..h {
            for x in 0..w {
                let pid = raster.ids[y * w + x];
                let Some(c0) = ctrl_of(pid) else {
                    continue;
                };
                // 4-neighbour scan; first differing controller flips this province on.
                let mut neigh = [(0i64, 0i64); 4];
                neigh[0] = (x as i64 - 1, y as i64);
                neigh[1] = (x as i64 + 1, y as i64);
                neigh[2] = (x as i64, y as i64 - 1);
                neigh[3] = (x as i64, y as i64 + 1);
                for (nx, ny) in neigh {
                    if nx < 0 || ny < 0 || nx as usize >= w || ny as usize >= h {
                        continue;
                    }
                    let npid = raster.ids[(ny as usize) * w + (nx as usize)];
                    if npid == pid {
                        continue;
                    }
                    if let Some(cn) = ctrl_of(npid) {
                        if cn != c0 {
                            front.insert(pid);
                            break;
                        }
                    }
                }
            }
        }
        self.front_provinces = Some(front);
        self
    }

    /// Fold a `provinces` domain in (province id -> supply when numeric, and air/naval
    /// region data if it lives there). Kept separate so callers that only have the map
    /// domain still build. Returns self for chaining.
    pub fn with_provinces(mut self, provinces: Option<&Value>) -> ModeData {
        if let Some(pv) = provinces.and_then(Value::as_object) {
            for (pid_str, v) in pv {
                let Ok(pid) = pid_str.parse::<u32>() else {
                    continue;
                };
                if let Some(s) = v.get("supply").and_then(Value::as_f64) {
                    self.province_supply.insert(pid, s);
                }
            }
        }
        self
    }

    /// Fold the first-class top-level weather domain into map-mode availability.
    ///
    /// The dynamic shader path consumes the detailed values elsewhere; the map-mode
    /// recolor here only needs to know whether the domain is genuinely fed so it
    /// can badge as a weather proxy instead of "unavailable".
    pub fn with_weather(mut self, weather: Option<&Value>) -> ModeData {
        self.has_weather |= domain_has_data(weather, &["regions", "provinces", "global_rollup"]);
        self
    }

    /// Fold country intel into the Intel map-mode signal. This is deliberately country-
    /// granularity and derived: the real game's intel map has richer fog-of-war semantics,
    /// so we only color tags whose snapshot country object carries measured intel fields.
    pub fn with_countries(mut self, countries: Option<&Map<String, Value>>) -> ModeData {
        let Some(countries) = countries else {
            return self;
        };
        for (tag, country) in countries {
            if let Some(score) = intel_country_score(country) {
                self.intel_score.insert(tag.clone(), score);
            }
        }
        self
    }

    /// Fold the first-class top-level naval/air proxy domain into map-mode availability.
    pub fn with_naval_air(mut self, naval_air: Option<&Value>) -> ModeData {
        self.has_naval |= domain_has_data(naval_air, &["region_control"]);
        self.has_air |= domain_has_data(
            naval_air,
            &[
                "air_superiority",
                "air_superiority_model",
                "air_superiority_model_meta",
            ],
        );
        self
    }
}

fn intel_country_score(country: &Value) -> Option<f64> {
    let intel = country.get("intel")?;
    if intel.is_null() {
        return None;
    }
    if let Some(defense) = intel
        .pointer("/agency/capacity/defense")
        .and_then(Value::as_f64)
    {
        return Some(defense.max(0.0));
    }
    if let Some(usable) = intel
        .pointer("/agency/capacity/usable_slots")
        .and_then(Value::as_f64)
    {
        return Some(usable.max(0.0));
    }
    if let Some(max_ops) = intel
        .pointer("/agency/capacity/max_operatives")
        .and_then(Value::as_f64)
    {
        return Some(max_ops.max(0.0));
    }
    if let Some(ops) = intel.get("operatives_live").and_then(Value::as_array) {
        return Some(ops.len() as f64);
    }
    if let Some(ops) = intel.get("operatives").and_then(Value::as_array) {
        return Some(ops.len() as f64);
    }
    None
}

fn domain_has_data(domain: Option<&Value>, keys: &[&str]) -> bool {
    let Some(obj) = domain.and_then(Value::as_object) else {
        return false;
    };
    keys.iter().any(|key| value_has_data(obj.get(*key)))
}

fn value_has_data(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Object(obj)) => !obj.is_empty(),
        Some(Value::Array(arr)) => !arr.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

/// Map a faction ideology to a representative tint (used by the diplomacy mode).
fn ideology_color(ideology: &str) -> [u8; 3] {
    match ideology {
        "fascism" => [150, 60, 50],
        "democratic" => [60, 90, 160],
        "communism" => [170, 40, 40],
        "neutrality" => [120, 120, 110],
        _ => [110, 110, 120],
    }
}

// ---------------------------------------------------------------------------
// Colour ramps — used by the scalar modes (resistance/compliance/VP/buildings/...).
// A perceptual-ish two-stop ramp from a cold low to a hot high. The ramp is applied
// to a NORMALISED [0,1] value; a measured 0 lands at the low colour (still distinct
// from neutral grey, so "0 factories" reads as data, not "no data").
// ---------------------------------------------------------------------------

/// Linear ramp low->high at t in [0,1].
fn ramp(low: [u8; 3], high: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
    [
        lerp(low[0], high[0]),
        lerp(low[1], high[1]),
        lerp(low[2], high[2]),
    ]
}

/// Normalise a value into [0,1] over [0,max]; max<=0 → 0.
fn norm(v: f64, max: f64) -> f64 {
    if max <= 0.0 {
        0.0
    } else {
        (v / max).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// The recolour dispatcher.
// ---------------------------------------------------------------------------

/// The result of building a mode LUT: the per-province RGBA buffer (ready for
/// `update_lut`) and the EFFECTIVE fidelity badge (possibly downgraded to
/// `Unavailable` if the snapshot carried no data for the mode).
pub struct ModeLut {
    pub lut: Vec<u8>,
    pub fidelity: Fidelity,
}

/// Build the colour LUT for `mode` from `data` over the province-id `ids`. `colors`
/// is the country palette (used by political/control/diplomacy). `max_id` sizes the
/// LUT exactly as the GPU pass expects (`(max_id+1)*4`).
///
/// Construction is no-fabrication identical to `build_lut`: start ALL neutral terrain,
/// then for each LAND province write either a real colour or neutral GREY ("no data").
pub fn recolor(
    mode: MapMode,
    data: &ModeData,
    ids: &MapIds,
    colors: &CountryColors,
    max_id: u32,
) -> ModeLut {
    let n = (max_id as usize) + 1;
    let mut lut = vec![0u8; n * 4];
    for i in 0..n {
        lut[i * 4..i * 4 + 4].copy_from_slice(&NEUTRAL_TERRAIN);
    }

    // Helper: write an RGBA cell for a province (bounds-checked).
    let write = |lut: &mut [u8], pid: u32, rgba: [u8; 4]| {
        if pid <= max_id {
            let c = (pid as usize) * 4;
            lut[c..c + 4].copy_from_slice(&rgba);
        }
    };

    // Pre-decide the static fidelity; some branches downgrade to Unavailable.
    let mut fidelity = mode.fidelity();

    // Iterate the land provinces ONCE; per mode decide the colour for the province's
    // state (or the province directly, for province-level modes).
    let land = ids.iter_provinces();

    match mode {
        // ----- country-colour modes (owner / controller) --------------------
        MapMode::Political => {
            for (pid, sid) in land {
                let rgba = data
                    .owner
                    .get(&sid)
                    .and_then(|o| o.as_deref())
                    .and_then(|t| colors.get(t))
                    .map(|c| [c[0], c[1], c[2], 255])
                    .unwrap_or(grey());
                write(&mut lut, pid, rgba);
            }
        }
        MapMode::Control => {
            // controller colour; where controller != owner, darken (contested hatch
            // proxy — a uniform darkening reads as "occupied/contested" on the LUT).
            for (pid, sid) in land {
                let ctrl = data.controller.get(&sid).and_then(|o| o.as_deref());
                let owner = data.owner.get(&sid).and_then(|o| o.as_deref());
                let rgba = match ctrl.and_then(|t| colors.get(t)) {
                    Some(c) => {
                        let contested = matches!((ctrl, owner), (Some(a), Some(b)) if a != b);
                        if contested {
                            // darken toward black ~45% — the "contested" hatch proxy.
                            [
                                (c[0] as f64 * 0.55) as u8,
                                (c[1] as f64 * 0.55) as u8,
                                (c[2] as f64 * 0.55) as u8,
                                255,
                            ]
                        } else {
                            [c[0], c[1], c[2], 255]
                        }
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- scalar occupation modes -------------------------------------
        MapMode::Resistance => {
            // resistance 0..100 → green(low) .. red(high). States with NO occupation
            // block (i.e. core/un-occupied) have no resistance datum → grey "no data".
            if data.resistance.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let rgba = match data.resistance.get(&sid) {
                    Some(&r) => {
                        let c = ramp([40, 120, 40], [200, 40, 40], norm(r, 100.0));
                        [c[0], c[1], c[2], 255]
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        MapMode::Compliance => {
            if data.compliance.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let rgba = match data.compliance.get(&sid) {
                    Some(&c) => {
                        // compliance: low=red, high=green (the inverse of resistance).
                        let col = ramp([200, 40, 40], [40, 160, 60], norm(c, 100.0));
                        [col[0], col[1], col[2], 255]
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- victory points ----------------------------------------------
        MapMode::VictoryPoints => {
            let max_vp = data
                .victory_points
                .values()
                .copied()
                .max()
                .unwrap_or(0)
                .max(1) as f64;
            if data.victory_points.values().all(|&v| v == 0) && data.victory_points.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let rgba = match data.victory_points.get(&sid) {
                    // measured 0 → low end of the ramp (data, not "no data").
                    Some(&vp) => {
                        let col = ramp([40, 40, 70], [240, 220, 60], norm(vp as f64, max_vp));
                        [col[0], col[1], col[2], 255]
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- buildings (per type) ----------------------------------------
        MapMode::Buildings(kind) => {
            let key = kind.snapshot_key();
            let max_b = data
                .buildings
                .values()
                .filter_map(|m| m.get(key).copied())
                .max()
                .unwrap_or(0)
                .max(1) as f64;
            // Availability = was the buildings DOMAIN captured at all (any state has a
            // buildings object), NOT whether any state has a NONZERO `key`. Otherwise a
            // world where every state has 0 of type K (all-zero but measured) would be
            // wrongly flagged "unavailable" and hide real data (Codex-flagged). A state
            // with a buildings object but no `key` is a measured 0, fully available.
            if data.buildings.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let rgba = match data.buildings.get(&sid) {
                    Some(m) => match m.get(key) {
                        // measured count (0 reads as low end, never grey).
                        Some(&b) => {
                            let col = ramp([35, 35, 45], [80, 200, 230], norm(b as f64, max_b));
                            [col[0], col[1], col[2], 255]
                        }
                        // state has a buildings block but not THIS type → measured 0.
                        None => {
                            let col = ramp([35, 35, 45], [80, 200, 230], 0.0);
                            [col[0], col[1], col[2], 255]
                        }
                    },
                    // state has no buildings block at all → no data → grey.
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- resources ----------------------------------------------------
        MapMode::Resources => {
            if data.resources.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            let max_r = data
                .resources
                .values()
                .copied()
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for (pid, sid) in land {
                let rgba = match data.resources.get(&sid) {
                    Some(&r) => {
                        let col = ramp([45, 40, 35], [220, 150, 50], norm(r, max_r));
                        [col[0], col[1], col[2], 255]
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- weather (per-province; proxy vs game visual) -----------------
        MapMode::Weather => {
            // No weather domain in this snapshot family → the whole map is unavailable.
            // (Land = grey "no data", never a fabricated temperature.)
            if !data.has_weather {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, _sid) in land {
                // even when has_weather is true, the per-province values are not in this
                // snapshot family; honest grey until that domain is captured.
                write(&mut lut, pid, grey());
            }
        }
        // ----- front lines (derived from control adjacency) -----------------
        MapMode::FrontLines => {
            // Province-level control; a province is ON a front line if a 4-neighbour
            // province has a DIFFERENT controller. Derived, badged so. Prefer the
            // precomputed TRUE raster-adjacency set; else the coarse state-level one.
            let frontier = match &data.front_provinces {
                Some(f) => f.clone(),
                None => derive_front_provinces(data, ids),
            };
            if data.province_control.is_empty() && data.front_provinces.is_none() {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let rgba = if frontier.contains(&pid) {
                    [220, 40, 40, 255] // front: hot red
                } else {
                    // non-front land tinted faintly by its controller for context.
                    match data
                        .controller
                        .get(&sid)
                        .and_then(|o| o.as_deref())
                        .and_then(|t| colors.get(t))
                    {
                        Some(c) => [
                            (c[0] as u32 * 2 / 5 + 40) as u8,
                            (c[1] as u32 * 2 / 5 + 40) as u8,
                            (c[2] as u32 * 2 / 5 + 40) as u8,
                            255,
                        ],
                        None => grey(),
                    }
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- diplomacy (faction / wars / subjects, country granularity) --
        MapMode::Diplomacy => {
            if data.faction_color.is_empty() && data.at_war.is_empty() && data.subject_of.is_empty()
            {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, sid) in land {
                let owner = data.owner.get(&sid).and_then(|o| o.as_deref());
                let rgba = match owner {
                    Some(t) => {
                        if let Some(col) = data.faction_color.get(t) {
                            // faction member → ideology faction tint.
                            [col[0], col[1], col[2], 255]
                        } else if let Some(over) = data.subject_of.get(t) {
                            // subject → overlord's faction tint, dimmed (or grey).
                            match data.faction_color.get(over).copied() {
                                Some(c) => [
                                    (c[0] as f64 * 0.6) as u8,
                                    (c[1] as f64 * 0.6) as u8,
                                    (c[2] as f64 * 0.6) as u8,
                                    255,
                                ],
                                None => [110, 110, 90, 255],
                            }
                        } else if data.at_war.contains(t) {
                            // at war but factionless → neutral war tint.
                            [150, 110, 60, 255]
                        } else {
                            // non-aligned, at peace → muted neutral country.
                            [90, 95, 100, 255]
                        }
                    }
                    None => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- intel (country-granularity measured agency/counter-intel signal) --
        MapMode::Intel => {
            if data.intel_score.is_empty() {
                fidelity = Fidelity::Unavailable;
            }
            let max_i = data
                .intel_score
                .values()
                .copied()
                .filter(|v| v.is_finite())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for (pid, sid) in land {
                let owner = data.owner.get(&sid).and_then(|o| o.as_deref());
                let rgba = match owner.and_then(|tag| data.intel_score.get(tag).copied()) {
                    Some(score) if score.is_finite() => {
                        let col = ramp([45, 45, 80], [60, 210, 200], norm(score, max_i));
                        [col[0], col[1], col[2], 255]
                    }
                    _ => grey(),
                };
                write(&mut lut, pid, rgba);
            }
        }
        // ----- air (strategic-region proxy) --------------------------------
        MapMode::Air => {
            // strategic-region granularity is the data we *would* colour; this snapshot
            // family carries none → unavailable, land grey (never invented superiority).
            if !data.has_air {
                fidelity = Fidelity::Unavailable;
            }
            for (pid, _sid) in land {
                write(&mut lut, pid, grey());
            }
        }
        // ----- naval (sea-zone region proxy) -------------------------------
        MapMode::Naval => {
            if !data.has_naval {
                fidelity = Fidelity::Unavailable;
            }
            // naval is about SEA zones, not land — colour nothing on land; land stays
            // grey "no data" and sea stays terrain (no fabricated dominance).
            for (pid, _sid) in land {
                write(&mut lut, pid, grey());
            }
        }
        // ----- supply (per-province proxy; the proven wall) -----------------
        MapMode::Supply => {
            let any = data.province_supply.values().any(|v| v.is_finite());
            if !any {
                fidelity = Fidelity::Unavailable;
            }
            let max_s = data
                .province_supply
                .values()
                .copied()
                .filter(|v| v.is_finite())
                .fold(0.0_f64, f64::max)
                .max(1.0);
            for (pid, sid) in land {
                let rgba = match data.province_supply.get(&pid) {
                    Some(&s) if s.is_finite() => {
                        let col = ramp([90, 40, 40], [60, 180, 90], norm(s, max_s));
                        [col[0], col[1], col[2], 255]
                    }
                    // no per-province supply (the wall) → grey, never fabricated.
                    _ => {
                        let _ = sid;
                        grey()
                    }
                };
                write(&mut lut, pid, rgba);
            }
        }
    }

    ModeLut { lut, fidelity }
}

#[inline]
fn grey() -> [u8; 4] {
    NEUTRAL_GREY
}

/// Derive the set of front-line provinces: a controlled province with at least one
/// orthogonal raster neighbour controlled by a DIFFERENT tag. Pure over the id raster +
/// province_control map. (Derived — a proxy for the battleplan front, badged so.)
///
/// Implemented from the province-id raster adjacency via the `ids` land map is not
/// enough (we need pixel adjacency); callers pass the raster separately via
/// [`derive_front_provinces_raster`]. The `ModeData`-only path here falls back to the
/// owner/controller divergence per state, which is a coarser but still derived frontier.
fn derive_front_provinces(data: &ModeData, ids: &MapIds) -> std::collections::HashSet<u32> {
    // Coarse, raster-free frontier: a land province whose STATE controller differs from
    // the state owner is a contested/front state; mark its provinces. Refined to true
    // pixel-adjacency by the raster path in the demo/app.
    let mut out = std::collections::HashSet::new();
    for (pid, sid) in ids.iter_provinces() {
        let ctrl = data.controller.get(&sid).and_then(|o| o.as_deref());
        let owner = data.owner.get(&sid).and_then(|o| o.as_deref());
        if matches!((ctrl, owner), (Some(a), Some(b)) if a != b) {
            out.insert(pid);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::CountryColors;

    fn ids_with(pairs: &[(u32, u32)]) -> MapIds {
        let mut p2s = HashMap::new();
        for &(p, s) in pairs {
            p2s.insert(p, s);
        }
        MapIds::new(p2s, HashMap::new())
    }
    fn colors2() -> CountryColors {
        let mut m = HashMap::new();
        m.insert("ITA".to_string(), [200, 30, 30]);
        m.insert("FRA".to_string(), [30, 30, 200]);
        CountryColors::new(m)
    }

    fn data_from(states_json: serde_json::Value, dw: Option<serde_json::Value>) -> ModeData {
        let states = states_json.as_object().unwrap().clone();
        ModeData::from_snapshot(&states, None, dw.as_ref())
    }

    /// Political mode: owner colour, null owner → grey, sea untouched (terrain).
    #[test]
    fn political_colors_owner_and_neutrals() {
        let ids = ids_with(&[(1, 10), (2, 20)]); // province 3 = sea (absent)
        let data = data_from(
            serde_json::json!({
                "10": {"owner": "ITA", "controller": "ITA"},
                "20": {"owner": null,  "controller": null}
            }),
            None,
        );
        let ml = recolor(MapMode::Political, &data, &ids, &colors2(), 3);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[200, 30, 30, 255]); // ITA
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &NEUTRAL_GREY); // null owner
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_TERRAIN); // sea
        assert_eq!(ml.fidelity, Fidelity::Exact);
    }

    /// Control mode: controller colour, and a contested state (owner != controller) is
    /// darkened — but NEVER fabricated when there is no controller.
    #[test]
    fn control_contested_darkens() {
        let ids = ids_with(&[(1, 10), (2, 20)]);
        let data = data_from(
            serde_json::json!({
                // owned ITA but controlled FRA → contested → darkened FRA
                "10": {"owner": "ITA", "controller": "FRA"},
                // owned & controlled ITA → full ITA
                "20": {"owner": "ITA", "controller": "ITA"}
            }),
            None,
        );
        let ml = recolor(MapMode::Control, &data, &ids, &colors2(), 2);
        // province 1: darkened FRA (0.55 of [30,30,200]) = [16,16,110]
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[16, 16, 110, 255]);
        // province 2: full ITA
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[200, 30, 30, 255]);
    }

    /// Resistance ramp: measured value colours, NO occupation block → grey (no data),
    /// and an all-empty resistance set downgrades the badge to Unavailable.
    #[test]
    fn resistance_ramp_and_unavailable() {
        let ids = ids_with(&[(1, 10), (2, 20)]);
        // state 10 occupied (resistance 100 → hot red end), state 20 no occupation.
        let data = data_from(
            serde_json::json!({
                "10": {"owner":"ITA","controller":"ITA","occupation":{"resistance":100.0,"compliance":0.0}},
                "20": {"owner":"ITA","controller":"ITA"}
            }),
            None,
        );
        let ml = recolor(MapMode::Resistance, &data, &ids, &colors2(), 2);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[200, 40, 40, 255]); // resistance 100
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &NEUTRAL_GREY); // no occupation
        assert_eq!(ml.fidelity, Fidelity::Exact);

        // a snapshot with ZERO occupation blocks → Unavailable.
        let data2 = data_from(
            serde_json::json!({"10": {"owner":"ITA","controller":"ITA"}}),
            None,
        );
        let ml2 = recolor(MapMode::Resistance, &data2, &ids, &colors2(), 2);
        assert_eq!(ml2.fidelity, Fidelity::Unavailable);
        assert_eq!(&ml2.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Buildings mode: a MEASURED 0 (state has buildings block but not this type) is the
    /// low ramp colour, NOT grey; a state with no buildings block at all is grey.
    #[test]
    fn buildings_measured_zero_vs_no_data() {
        let ids = ids_with(&[(1, 10), (2, 20), (3, 30), (4, 40)]);
        let data = data_from(
            serde_json::json!({
                "10": {"buildings": {"arms_factory": 4}},   // has 4
                "20": {"buildings": {"infrastructure": 2}}, // present block, 0 arms
                "30": {"owner": "ITA"},                      // NO buildings key → grey
                "40": {"buildings": {}}                      // empty {} → measured 0 (NOT grey)
            }),
            None,
        );
        let ml = recolor(
            MapMode::Buildings(BuildingKind::MilitaryFactory),
            &data,
            &ids,
            &colors2(),
            4,
        );
        // province 1: max (4/4) → hot end
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[80, 200, 230, 255]);
        // province 2: present block, type absent → measured 0 → low ramp colour
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[35, 35, 45, 255]);
        // province 3: no buildings key at all → grey "no data"
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY);
        // province 4: EMPTY {} block → measured 0 (the Codex fix): low ramp, NOT grey
        assert_eq!(&ml.lut[4 * 4..4 * 4 + 4], &[35, 35, 45, 255]);
        assert_eq!(ml.fidelity, Fidelity::Exact);
    }

    /// Buildings availability follows the DOMAIN, not whether any state has type K:
    /// a world where every state has a buildings block but ZERO of type K is still
    /// `Exact` (all measured-0), NOT `Unavailable` (Codex-flagged hide-data risk).
    #[test]
    fn buildings_all_zero_is_available_not_unavailable() {
        let ids = ids_with(&[(1, 10), (2, 20)]);
        let data = data_from(
            serde_json::json!({
                "10": {"buildings": {"infrastructure": 3}}, // no arms_factory anywhere
                "20": {"buildings": {}}
            }),
            None,
        );
        let ml = recolor(
            MapMode::Buildings(BuildingKind::MilitaryFactory),
            &data,
            &ids,
            &colors2(),
            2,
        );
        assert_eq!(
            ml.fidelity,
            Fidelity::Exact,
            "all-zero K is measured, not unavailable"
        );
        // both states are measured 0 → low ramp colour, neither grey.
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[35, 35, 45, 255]);
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[35, 35, 45, 255]);
    }

    /// Victory points: measured 0 lands at the ramp low, not grey; absent → grey.
    #[test]
    fn vp_measured_zero_low_end() {
        let ids = ids_with(&[(1, 10), (2, 20), (3, 30)]);
        let data = data_from(
            serde_json::json!({
                "10": {"victory_points_total": 50},
                "20": {"victory_points_total": 0},
                "30": {"owner":"ITA"}  // no vp field
            }),
            None,
        );
        let ml = recolor(MapMode::VictoryPoints, &data, &ids, &colors2(), 3);
        // 1: max → hot
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[240, 220, 60, 255]);
        // 2: measured 0 → low end [40,40,70]
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[40, 40, 70, 255]);
        // 3: absent → grey
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Resources absent (null block everywhere) → Unavailable + all land grey.
    #[test]
    fn resources_unavailable_when_null() {
        let ids = ids_with(&[(1, 10)]);
        let data = data_from(
            serde_json::json!({"10": {"owner":"ITA","resources": null}}),
            None,
        );
        let ml = recolor(MapMode::Resources, &data, &ids, &colors2(), 1);
        assert_eq!(ml.fidelity, Fidelity::Unavailable);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Resources mode consumes the now-live per-state deposits: a non-empty resources
    /// block colors the state, an empty measured block is a real 0, and an absent block
    /// remains grey (no fabricated country-level aggregate substitution).
    #[test]
    fn resources_recolor_uses_measured_state_deposits() {
        let ids = ids_with(&[(1, 10), (2, 20), (3, 30)]);
        let data = data_from(
            serde_json::json!({
                "10": {"owner":"ITA","resources": {"steel": 7, "oil": 3}},
                "20": {"owner":"ITA","resources": {}},
                "30": {"owner":"ITA"}
            }),
            None,
        );
        let ml = recolor(MapMode::Resources, &data, &ids, &colors2(), 3);
        assert_eq!(ml.fidelity, Fidelity::Exact);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[220, 150, 50, 255]);
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[45, 40, 35, 255]);
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Weather / Air / Naval: this snapshot family has no such domain → Unavailable,
    /// land all grey, sea terrain — NEVER a fabricated value.
    #[test]
    fn proxy_modes_unavailable_neutral() {
        let ids = ids_with(&[(1, 10)]);
        let data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None);
        for mode in [MapMode::Weather, MapMode::Air, MapMode::Naval] {
            let ml = recolor(mode, &data, &ids, &colors2(), 2);
            assert_eq!(ml.fidelity, Fidelity::Unavailable, "{:?}", mode);
            assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY, "{:?}", mode);
            assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &NEUTRAL_TERRAIN, "{:?}", mode);
        }
    }

    #[test]
    fn top_level_weather_and_naval_air_domains_drive_proxy_availability() {
        let ids = ids_with(&[(1, 10)]);
        let weather = serde_json::json!({
            "provinces": { "1": { "snow": 0.25 } },
            "regions": null,
            "source": "M"
        });
        let weather_data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None)
            .with_weather(Some(&weather));
        let ml = recolor(MapMode::Weather, &weather_data, &ids, &colors2(), 2);
        assert_eq!(
            ml.fidelity,
            Fidelity::Proxy,
            "fed top-level weather domain must not badge unavailable"
        );
        assert_eq!(
            &ml.lut[1 * 4..1 * 4 + 4],
            &NEUTRAL_GREY,
            "availability is not value fabrication; weather recolor still needs its visual pass"
        );

        let naval_air = serde_json::json!({
            "region_control": { "5": [ { "controller": "ITA" } ] },
            "air_superiority": { "5": { "controller": "ITA" } },
            "source": "M",
            "confidence": "region_proxy"
        });
        let naval_air_data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None)
            .with_naval_air(Some(&naval_air));
        assert_eq!(
            recolor(MapMode::Air, &naval_air_data, &ids, &colors2(), 2).fidelity,
            Fidelity::Proxy
        );
        assert_eq!(
            recolor(MapMode::Naval, &naval_air_data, &ids, &colors2(), 2).fidelity,
            Fidelity::Proxy
        );

        let empty_naval_air = serde_json::json!({
            "region_control": null,
            "air_superiority": null,
            "source": null
        });
        let empty_data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None)
            .with_naval_air(Some(&empty_naval_air));
        assert_eq!(
            recolor(MapMode::Air, &empty_data, &ids, &colors2(), 2).fidelity,
            Fidelity::Unavailable
        );
        assert_eq!(
            recolor(MapMode::Naval, &empty_data, &ids, &colors2(), 2).fidelity,
            Fidelity::Unavailable
        );
    }

    /// Front lines: a contested state's provinces colour hot-red; the badge is Derived.
    #[test]
    fn front_lines_derived() {
        let ids = ids_with(&[(1, 10), (2, 20)]);
        let data = data_from(
            serde_json::json!({
                "10": {"owner":"ITA","controller":"FRA"}, // contested → front
                "20": {"owner":"ITA","controller":"ITA"}  // calm
            }),
            // give it province control so it isn't Unavailable
            Some(serde_json::json!({})),
        );
        // inject province_control directly (map domain path) so it's non-empty
        let mut data = data;
        data.province_control.insert(1, "FRA".into());
        data.province_control.insert(2, "ITA".into());
        let ml = recolor(MapMode::FrontLines, &data, &ids, &colors2(), 2);
        assert_eq!(ml.fidelity, Fidelity::Derived);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[220, 40, 40, 255]); // front
                                                                    // province 2 is calm → tinted ITA, not grey, not front-red
        assert_ne!(&ml.lut[2 * 4..2 * 4 + 4], &[220, 40, 40, 255]);
        assert_ne!(&ml.lut[2 * 4..2 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Diplomacy: a faction member gets the ideology faction tint; a non-aligned
    /// peaceful country is muted neutral; absent owner is grey.
    #[test]
    fn diplomacy_faction_tint() {
        let ids = ids_with(&[(1, 10), (2, 20), (3, 30)]);
        let data = data_from(
            serde_json::json!({
                "10": {"owner":"GER"}, // axis member
                "20": {"owner":"SWE"}, // non-aligned peaceful
                "30": {"owner": null}  // grey
            }),
            Some(serde_json::json!({
                "factions": {
                    "1": {"ideology":"fascism","members":["GER","ITA"]}
                },
                "wars": {},
                "subjects": []
            })),
        );
        let ml = recolor(MapMode::Diplomacy, &data, &ids, &colors2(), 3);
        assert_eq!(ml.fidelity, Fidelity::Exact);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[150, 60, 50, 255]); // fascism tint
        assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &[90, 95, 100, 255]); // non-aligned
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY); // null owner
    }

    #[test]
    fn intel_recolor_uses_measured_country_intel_only() {
        let ids = ids_with(&[(1, 10), (2, 20), (3, 30), (4, 40)]);
        let countries = serde_json::json!({
            "GER": {"intel": {"agency": {"capacity": {"defense": 6.0}}}},
            "FRA": {"intel": {"operatives": []}},
            "SWE": {"intel": null}
        });
        let data = data_from(
            serde_json::json!({
                "10": {"owner":"GER"},
                "20": {"owner":"FRA"},
                "30": {"owner":"SWE"},
                "40": {"owner": null}
            }),
            None,
        )
        .with_countries(countries.as_object());

        let ml = recolor(MapMode::Intel, &data, &ids, &colors2(), 4);
        assert_eq!(ml.fidelity, Fidelity::Derived);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &[60, 210, 200, 255]);
        assert_eq!(
            &ml.lut[2 * 4..2 * 4 + 4],
            &[45, 45, 80, 255],
            "measured empty operatives is a real low intel value, not unavailable"
        );
        assert_eq!(&ml.lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY);
        assert_eq!(&ml.lut[4 * 4..4 * 4 + 4], &NEUTRAL_GREY);
    }

    #[test]
    fn intel_recolor_unavailable_without_measured_country_intel() {
        let ids = ids_with(&[(1, 10)]);
        let data = data_from(serde_json::json!({"10": {"owner":"GER"}}), None);
        let ml = recolor(MapMode::Intel, &data, &ids, &colors2(), 1);
        assert_eq!(ml.fidelity, Fidelity::Unavailable);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY);
    }

    #[test]
    fn intel_recolor_ignores_nonempty_agency_without_measured_signal() {
        let ids = ids_with(&[(1, 10)]);
        let countries = serde_json::json!({
            "GER": {"intel": {"agency": {"name": "Abwehr"}}}
        });
        let data = data_from(serde_json::json!({"10": {"owner":"GER"}}), None)
            .with_countries(countries.as_object());
        let ml = recolor(MapMode::Intel, &data, &ids, &colors2(), 1);
        assert_eq!(
            ml.fidelity,
            Fidelity::Unavailable,
            "non-empty agency chrome must not become a fabricated 0 intel score"
        );
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY);
    }

    /// Supply: with no per-province supply (the wall) the mode is Unavailable + grey.
    #[test]
    fn supply_wall_unavailable() {
        let ids = ids_with(&[(1, 10)]);
        let data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None);
        let ml = recolor(MapMode::Supply, &data, &ids, &colors2(), 1);
        assert_eq!(ml.fidelity, Fidelity::Unavailable);
        assert_eq!(&ml.lut[1 * 4..1 * 4 + 4], &NEUTRAL_GREY);
    }

    /// The switcher cycles through every mode and wraps; Buildings collapses to one slot.
    #[test]
    fn switcher_cycles_all() {
        let mut m = MapMode::Political;
        let mut seen = vec![m.slug()];
        for _ in 0..MapMode::CYCLE.len() - 1 {
            m = m.next();
            seen.push(m.slug());
        }
        // back to start after a full cycle
        assert_eq!(m.next().slug(), MapMode::Political.slug());
        // every cycle slug is present and unique
        assert_eq!(seen.len(), MapMode::CYCLE.len());
        let uniq: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!(
            uniq.len(),
            seen.len(),
            "cycle slugs must be unique: {seen:?}"
        );
        // prev is the inverse of next
        assert_eq!(
            MapMode::Political.next().prev().slug(),
            MapMode::Political.slug()
        );
    }

    /// Every mode in the cycle has a badge string and never panics building an empty LUT.
    #[test]
    fn all_modes_build_and_badge() {
        let ids = ids_with(&[(1, 10)]);
        let data = data_from(serde_json::json!({"10": {"owner":"ITA"}}), None);
        for &mode in MapMode::CYCLE {
            let ml = recolor(mode, &data, &ids, &colors2(), 2);
            // LUT is the right size and starts/ends honest.
            assert_eq!(ml.lut.len(), 3 * 4);
            assert!(!ml.fidelity.badge().is_empty());
            // sea province (id 2, absent from land) is ALWAYS terrain, every mode.
            assert_eq!(&ml.lut[2 * 4..2 * 4 + 4], &NEUTRAL_TERRAIN, "{:?}", mode);
        }
    }
}
