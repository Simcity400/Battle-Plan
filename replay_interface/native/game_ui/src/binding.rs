//! Timeline → topbar / ideology bindings (REPLAY_PRODUCTION_PLAN D1 + D2).
//!
//! This module is the *population* seam: it maps `replay_core`'s typed
//! `WorldFrame` views (topbar scalars, country identity) onto the
//! layout-only [`TopbarBinding`] and the ideology-keyed flag/localisation
//! candidates. It parses no recording bytes — every value comes through a
//! typed view (Constraint 2). It fabricates no value: a `Leaf` that is
//! `Absent`/`Unobtained`, or a view whose status is not `Present`, renders
//! the **fail-closed absent form** (a `None` field / empty candidate set),
//! never a default like `0` (Constraint 3).
//!
//! ## Number formatting is UNKNOWN → rendered raw
//!
//! the game's topbar number formatting (thousands separators, rounding,
//! percent-vs-fraction display, +/- coloring) is engine-computed and is not
//! cited from any shipped `.gui`/localisation file in this decoupled context.
//! Per Constraint 3 ("cited from files or labelled UNKNOWN and rendered
//! raw"), every scalar here is rendered as the **raw measured value** via
//! `format!("{}")` on the typed leaf — no invented percent scaling, grouping,
//! or sign coloring. When a shipped-file citation for a field's format is
//! found, replace the raw renderer for that field and cite it. See
//! `LANE_REPORT.md` for the per-field FILE/UNKNOWN table.

use crate::topbar::{flag_candidates_for_size, FactoryCounts, FlagSizeFolder, TopbarBinding};
use replay_core::{CountryIdentity, Leaf, TopbarScalars, ViewStatus, WorldFrame};

/// Render an `f64` leaf as its raw shortest round-trip decimal, or `None`
/// (the fail-closed absent form) when the value was never measured.
///
/// UNKNOWN formatting: no percent scaling / grouping / rounding is applied.
fn raw_f64(leaf: &Leaf<f64>) -> Option<String> {
    match leaf {
        Leaf::Measured(v) => Some(format!("{v}")),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

/// Render an `i64` leaf as its raw decimal, or `None` when unmeasured.
fn raw_i64(leaf: &Leaf<i64>) -> Option<String> {
    match leaf {
        Leaf::Measured(v) => Some(format!("{v}")),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

/// Non-negative `i64` leaf → `u32` for [`FactoryCounts`]; `None` if unmeasured
/// or negative (fail-closed, never coerced to a default).
fn measured_u32(leaf: &Leaf<i64>) -> Option<u32> {
    match leaf {
        Leaf::Measured(v) if *v >= 0 => u32::try_from(*v).ok(),
        _ => None,
    }
}

/// Map one country's [`TopbarScalars`] view onto the layout-only
/// [`TopbarBinding`] text/count fields, fail-closed. The flag/ideology
/// identity fields are filled separately by [`apply_identity`].
///
/// Every field is the raw measured value or `None`; see the module header for
/// the UNKNOWN-formatting rationale and `LANE_REPORT.md` for the field table.
fn apply_scalars(binding: &mut TopbarBinding, scalars: &TopbarScalars) {
    binding.political_power = raw_f64(&scalars.political_power);
    binding.stability_percent = raw_f64(&scalars.stability);
    binding.war_support_percent = raw_f64(&scalars.war_support);
    binding.command_power = raw_f64(&scalars.command_power);
    binding.manpower = raw_i64(&scalars.manpower_available);
    binding.army_experience = raw_f64(&scalars.xp_army);
    binding.navy_experience = raw_f64(&scalars.xp_navy);
    binding.air_experience = raw_f64(&scalars.xp_air);
    // Nukes: the view carries total vs ready separately; the topbar shows a
    // single nuke count. Which one is UNKNOWN (no cited element), so this binds
    // the total and records the ambiguity in LANE_REPORT.md.
    binding.nukes = raw_i64(&scalars.nukes_total);
    binding.fuel = raw_f64(&scalars.fuel_current);
    binding.world_tension_percent = raw_f64(&scalars.world_tension);
    binding.world_tension = scalars.world_tension.measured().map(|v| *v as f32);
    binding.supply = raw_f64(&scalars.supply_value);
    binding.convoys = raw_f64(&scalars.convoys);

    // Factory triple: only bound when all three counts are measured, so a
    // partial capture never renders a mixed real/zero triple.
    binding.factory_counts = match (
        measured_u32(&scalars.factories_civ),
        measured_u32(&scalars.factories_mil),
        measured_u32(&scalars.factories_dockyard),
    ) {
        (Some(civilian), Some(military), Some(naval)) => Some(FactoryCounts {
            civilian,
            military,
            naval,
        }),
        _ => None,
    };
    // The single-count `factory_text` alias is left absent; the triple above is
    // the measured form. (Its `/`-join is UNKNOWN formatting — see report.)
    binding.factory_text = None;
}

/// Fill the flag/ideology identity fields from a [`CountryIdentity`] view.
/// `ruling_ideology`/`cosmetic_tag` that are absent/unobtained stay `None`, so
/// flag selection falls back exactly as the shipped inventory dictates.
fn apply_identity(binding: &mut TopbarBinding, pov_tag: &str, identity: &CountryIdentity) {
    binding.country_tag = Some(pov_tag.to_string());
    binding.country_ideology = identity.ruling_ideology.measured().cloned();
    binding.cosmetic_tag = identity.cosmetic_tag.measured().cloned();
}

/// Build the topbar binding for `pov_tag` at one world frame (D1 + D2
/// population). Returns a fully fail-closed binding: when the topbar-scalar
/// view is not `Present` for this POV, the scalar fields stay `None`; when the
/// identity view is not `Present`, the flag falls back to the bare tag.
///
/// The layout is untouched — this only fills the binding the existing
/// `compose_topbar` consumes.
pub fn topbar_binding_from_frame(frame: &WorldFrame, pov_tag: &str) -> TopbarBinding {
    let mut binding = TopbarBinding::default();

    // Identity first, so a POV with no scalar row still keys its flag.
    if frame.country_identities_status() == ViewStatus::Present {
        if let Some(identity) = frame.country_identity(pov_tag) {
            apply_identity(&mut binding, pov_tag, identity);
        } else {
            // Country absent from a present identity view: still name the tag
            // so the base flag resolves; ideology/cosmetic stay absent.
            binding.country_tag = Some(pov_tag.to_string());
        }
    } else {
        binding.country_tag = Some(pov_tag.to_string());
    }

    if frame.topbar_status() == ViewStatus::Present {
        if let Some(scalars) = frame.topbar_scalars(pov_tag) {
            apply_scalars(&mut binding, scalars);
        }
    }

    binding
}

/// The LAB4 ideology-resolved country **display-name** localisation key
/// candidates, most specific first (FIDELITY_LEDGER §label law):
/// `<cosmetic>_<ideology>` → `<cosmetic>` → `<tag>_<ideology>` → `<tag>` →
/// bare `<tag>`. A key is only emitted when its inputs exist; the bare tag is
/// always the final fail-closed fallback.
///
/// This is a pure key-precedence function — resolution against a
/// [`crate::loc::Localisation`] (via the GameFileResolver) is
/// [`resolve_country_name`].
pub fn country_name_key_candidates(
    tag: &str,
    ideology: Option<&str>,
    cosmetic_tag: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let tag_upper = tag.to_ascii_uppercase();
    if let Some(cosmetic) = cosmetic_tag.filter(|c| !c.is_empty()) {
        if let Some(ideology) = ideology {
            out.push(format!("{cosmetic}_{ideology}"));
        }
        out.push(cosmetic.to_string());
    }
    if let Some(ideology) = ideology {
        out.push(format!("{tag_upper}_{ideology}"));
    }
    out.push(tag_upper.clone());
    // Bare tag as the last-ditch fallback (dedup if it already equals <tag>).
    if out.last().map(String::as_str) != Some(tag_upper.as_str()) {
        out.push(tag_upper);
    }
    out
}

/// The ideology-keyed flag-texture candidates for a POV's topbar shield, in
/// engine precedence (cosmetic before tag, `_<ideology>` before base), pinned
/// to the `gfx/flags/medium/` folder (UI_LAYOUT_LAW §2 / topbar
/// `GFX_shield_medium`). Thin wrapper over [`flag_candidates_for_size`] keyed
/// off a [`CountryIdentity`] view.
pub fn topbar_flag_key_candidates(tag: &str, identity: &CountryIdentity) -> Vec<String> {
    flag_candidates_for_size(
        Some(tag),
        identity.ruling_ideology.measured().map(String::as_str),
        identity.cosmetic_tag.measured().map(String::as_str),
        None,
        FlagSizeFolder::Medium,
    )
}

/// Resolve the LAB4 display name against a loaded localisation, returning the
/// first candidate key that exists plus the value. Fail-closed: if no key is
/// present, returns the bare tag as `(tag, tag)` — never a fabricated name.
pub fn resolve_country_name(
    loc: &crate::loc::Localisation,
    tag: &str,
    ideology: Option<&str>,
    cosmetic_tag: Option<&str>,
) -> (String, String) {
    for key in country_name_key_candidates(tag, ideology, cosmetic_tag) {
        if let Some(value) = loc.get(&key) {
            return (key, value.to_string());
        }
    }
    let tag_upper = tag.to_ascii_uppercase();
    (tag_upper.clone(), tag_upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(ideology: Option<&str>, cosmetic: Option<&str>) -> CountryIdentity {
        CountryIdentity {
            cosmetic_tag: match cosmetic {
                Some(c) => Leaf::Measured(c.to_string()),
                None => Leaf::Unobtained,
            },
            ruling_ideology: match ideology {
                Some(i) => Leaf::Measured(i.to_string()),
                None => Leaf::Unobtained,
            },
        }
    }

    #[test]
    fn name_key_precedence_cosmetic_ideology_tag() {
        // Cosmetic + ideology present → the full LAB4 ladder.
        assert_eq!(
            country_name_key_candidates(
                "GER",
                Some("democratic"),
                Some("GER_civil_war_democratic")
            ),
            vec![
                "GER_civil_war_democratic_democratic".to_string(),
                "GER_civil_war_democratic".to_string(),
                "GER_democratic".to_string(),
                "GER".to_string(),
            ]
        );
        // No cosmetic → tag ladder only.
        assert_eq!(
            country_name_key_candidates("GER", Some("fascism"), None),
            vec!["GER_fascism".to_string(), "GER".to_string()]
        );
        // No ideology, no cosmetic → bare tag only (single entry, deduped).
        assert_eq!(
            country_name_key_candidates("ger", None, None),
            vec!["GER".to_string()]
        );
    }

    #[test]
    fn absent_leaves_render_none_never_zero() {
        let mut binding = TopbarBinding::default();
        apply_scalars(&mut binding, &all_absent_scalars());
        assert_eq!(binding.political_power, None);
        assert_eq!(binding.manpower, None);
        assert_eq!(binding.fuel, None);
        assert_eq!(binding.world_tension_percent, None);
        assert_eq!(binding.factory_counts, None);
    }

    #[test]
    fn flag_candidates_key_off_identity() {
        let cands = topbar_flag_key_candidates(
            "GER",
            &identity(Some("democratic"), Some("GER_civil_war_democratic")),
        );
        assert_eq!(
            cands[0],
            "gfx/flags/medium/GER_civil_war_democratic_democratic.tga"
        );
        assert!(cands.iter().all(|c| c.starts_with("gfx/flags/medium/")));
    }

    fn all_absent_scalars() -> TopbarScalars {
        TopbarScalars {
            political_power: Leaf::Absent,
            stability: Leaf::Absent,
            war_support: Leaf::Absent,
            command_power: Leaf::Absent,
            manpower_available: Leaf::Absent,
            xp_army: Leaf::Absent,
            xp_navy: Leaf::Absent,
            xp_air: Leaf::Absent,
            factories_civ: Leaf::Absent,
            factories_mil: Leaf::Absent,
            factories_dockyard: Leaf::Absent,
            fuel_current: Leaf::Absent,
            fuel_max: Leaf::Absent,
            convoys: Leaf::Absent,
            nukes_total: Leaf::Absent,
            nukes_ready: Leaf::Absent,
            world_tension: Leaf::Absent,
            supply_value: Leaf::Absent,
            supply_ratio: Leaf::Absent,
        }
    }
}
