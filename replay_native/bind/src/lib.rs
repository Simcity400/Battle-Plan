//! `bind` — the DECLARATIVE BINDING SPEC + the apply layer for the topbar.
//!
//! This crate encodes the verified topbar element→snapshot-path→formatter→fidelity
//! map (`replay_ui/BINDING_MAP_topbar_state.md`) as TYPED RUST STRUCTS (the
//! authoritative declarative form; the spec's "structs / RON" alternative — we
//! pick structs to avoid a new dependency and keep the map compile-checked), and
//! APPLIES it to a `gui_runtime` scene-graph: for each dynamic element it reads
//! the value from a loaded [`snapshot::Snapshot`], formats it, and substitutes the
//! result into the element's resolved text (or, for the flag, swaps the sprite).
//!
//! NO-FABRICATION is structural here. The snapshot crate distinguishes:
//!   * ABSENT  (`country_path` -> `None`)              — never captured
//!   * NULL    (`country_path` -> `Some(Value::Null)`) — captured-but-unobtained
//!   * MEASURED (`Some(v)`, `v` a number incl. `0`)     — a real value
//! A binding badged [`Fidelity::Unavailable`], OR any path that resolves to
//! absent/null, renders the NOT-CAPTURED glyph ([`format::NOT_CAPTURED`] = "—").
//! A formatter is therefore ONLY ever invoked on a measured value: a fabricated
//! `0` can never reach the screen (proven by [`apply::bind_text`]'s guard + the
//! `unavailable_never_zero` test).

pub mod format;

use serde_json::Value;

/// The fidelity badge carried by every binding (from the binding map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// A directly measured, display-faithful value.
    Exact,
    /// Computed/approximated from stored fields (still measured inputs).
    Derived,
    /// Schema-present-but-unfed, or a runtime-computed value we do not store.
    /// ALWAYS renders the not-captured glyph — never a fabricated value.
    Unavailable,
}

/// Which formatter a text binding uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formatter {
    /// 0..1 float ×100 -> "NN%".
    Percent,
    /// Stored stock -> truncated whole int (political/command power).
    UnsignedInt,
    /// Plain integer count.
    Int,
    /// Compact people (`525000 -> "525K"`).
    Manpower,
    /// Compact fuel amount (`849000 -> "849K"`).
    Fuel,
    /// `civ/mil/dockyard` triple (special: reads three paths).
    FactoryTriple,
    /// Clausewitz date -> "H:00 D, Month YYYY" (special: reads the top-level date).
    Clock,
}

/// One declarative TEXT binding: a topbar element name -> a snapshot path (rooted
/// at the observed country, dotted) -> a formatter -> a fidelity badge.
#[derive(Debug, Clone, Copy)]
pub struct TextBinding {
    /// The `.gui` element `name` whose text this binding drives.
    pub element: &'static str,
    /// Dotted snapshot path under `countries.<TAG>` (e.g. `politics.stability`).
    /// `None` for the special formatters (FactoryTriple / Clock) that read fixed
    /// fields of their own.
    pub path: Option<&'static str>,
    pub formatter: Formatter,
    pub fidelity: Fidelity,
}

/// The FLAG binding: a topbar element name -> the country flag, selected by TAG +
/// ruling party ideology (the game `gfx/flags/.../<TAG>_<ideology>.tga` scheme).
#[derive(Debug, Clone, Copy)]
pub struct FlagBinding {
    pub element: &'static str,
    pub fidelity: Fidelity,
}

/// The complete topbar binding spec.
pub struct TopbarSpec {
    pub flag: FlagBinding,
    pub texts: Vec<TextBinding>,
}

impl TopbarSpec {
    /// The authoritative topbar spec — every DYNAMIC topbar element from
    /// `BINDING_MAP_topbar_state.md` §1a (the `unavailable` rows are present and
    /// badged so they render not-captured, NOT omitted).
    pub fn topbar() -> TopbarSpec {
        use Fidelity::*;
        use Formatter::*;
        TopbarSpec {
            flag: FlagBinding { element: "player_flag", fidelity: Exact },
            texts: vec![
                // --- measured, exact ---
                TextBinding { element: "pol_power", path: Some("politics.political_power"), formatter: UnsignedInt, fidelity: Exact },
                TextBinding { element: "stability", path: Some("politics.stability"), formatter: Percent, fidelity: Exact },
                TextBinding { element: "war_support", path: Some("politics.war_support"), formatter: Percent, fidelity: Exact },
                TextBinding { element: "manpower", path: Some("military.manpower.available"), formatter: Manpower, fidelity: Exact },
                TextBinding { element: "command_power_value", path: Some("politics.command_power"), formatter: UnsignedInt, fidelity: Exact },
                // Fuel: the stored AMOUNT is exact, but the vanilla slot shows a
                // days/years DURATION (needs a consumption rate we don't store) —
                // so binding the amount here is a replay SUBSTITUTE -> Derived.
                TextBinding { element: "fuel_value", path: Some("economy.fuel.current"), formatter: Fuel, fidelity: Derived },
                // industrial_capacity reads three paths -> special FactoryTriple.
                TextBinding { element: "industrial_capacity", path: None, formatter: FactoryTriple, fidelity: Exact },
                // DateText reads the top-level snapshot.date -> special Clock.
                TextBinding { element: "DateText", path: None, formatter: Clock, fidelity: Exact },
                // --- derived (measured inputs, computed aggregate) ---
                TextBinding { element: "convoys_count", path: None, formatter: Int, fidelity: Derived },
                // --- UNAVAILABLE: schema-present-but-unfed / not-stored. These
                //     MUST render not-captured ("—"), never a fabricated value. ---
                TextBinding { element: "experience_value", path: Some("military.xp.army"), formatter: Int, fidelity: Unavailable },
                TextBinding { element: "navy_experience_value", path: Some("military.xp.navy"), formatter: Int, fidelity: Unavailable },
                TextBinding { element: "air_experience_value", path: Some("military.xp.air"), formatter: Int, fidelity: Unavailable },
                TextBinding { element: "nukes_value", path: Some("extra.nukes_total"), formatter: Int, fidelity: Unavailable },
                TextBinding { element: "supply_value", path: None, formatter: Int, fidelity: Unavailable },
                TextBinding { element: "threat_value", path: None, formatter: Percent, fidelity: Unavailable },
                TextBinding { element: "initiative_text", path: None, formatter: Int, fidelity: Unavailable },
            ],
        }
    }
}

/// The structural classification of a resolved snapshot path (the no-fabrication
/// trichotomy). Drives whether a formatter runs or the not-captured glyph shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueState {
    /// A key along the path was absent — never captured.
    Absent,
    /// Present but JSON null — captured-but-unobtained.
    Null,
    /// A measured value (a number, including a measured `0`).
    Measured,
    /// Present but a non-numeric/non-scalar shape (e.g. `{}` or `[]`) — treated as
    /// NOT a usable measurement, so it renders not-captured.
    NonScalar,
}

/// Classify a `country_path` lookup result for a SCALAR binding.
pub fn classify(opt: Option<&Value>) -> ValueState {
    match opt {
        None => ValueState::Absent,
        Some(Value::Null) => ValueState::Null,
        Some(v) if v.is_number() => ValueState::Measured,
        // strings could be measured (ruling_party) but the scalar text formatters
        // operate on numbers; a non-number scalar reaching a numeric formatter is
        // a binding error, surfaced as not-captured rather than fabricated.
        Some(Value::String(_)) => ValueState::Measured,
        Some(_) => ValueState::NonScalar,
    }
}

pub mod apply;
pub mod flag;
pub mod state;

pub use apply::{apply_topbar, BindReport};
pub use flag::FlagResolver;
pub use state::{bind_state, BuildingRow, Occupation, StateBindReport, StateField};
