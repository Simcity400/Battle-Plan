//! Timeline → topbar / ideology bindings (D1 + D2 lane). The chrome law the
//! layout side is gated against is `replay_interface/documents/UI_LAYOUT_LAW.md`.
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
//! ## Number formatting is UNKNOWN
//!
//! the game's topbar number formatting (thousands separators, rounding,
//! percent-vs-fraction display, +/- coloring) is engine-computed and is not
//! cited from any shipped `.gui`/localisation file in this decoupled context.
//! The legacy data-only binding retains each raw measured value. The shipping
//! observer never displays that provisional string: it replaces every
//! UNKNOWN-formatted value with exact A25 final setter bytes for the same
//! frame, selected country, view and element, or omits the element and reports
//! a named gap.
//!
//! The data-only binding retains proven percent projections for
//! `world_tension_percent`, `stability` and `war_support` (see
//! `apply_scalars`). The shipping observer still requires A25 final bytes for
//! their actual labels, so even a proven unit conversion is not mistaken for
//! proof of the engine's string formatting.

use crate::hud::{OutlinerBinding, OutlinerRowBinding};
use crate::loc::Localisation;
use crate::text_law::{join_exact_runtime_text_view, RuntimeTextContext};
use crate::topbar::{
    flag_candidates_for_size, format_percent, month_name_english, FactoryCounts, FlagSizeFolder,
    TopbarBinding,
};
use replay_core::world::FactionRosterStatus;
use replay_core::{
    CountryIdentity, DiplomacyView, Leaf, SubjectAutonomyDirection, TopbarRatios, TopbarScalars,
    RuntimeTextCatalog, ViewStatus, WorldFrame,
};
use std::collections::BTreeMap;

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

/// Measured 0..=1 fraction for an authored progress bar. Non-finite or
/// out-of-range values fail closed instead of being clamped into a plausible
/// display value.
fn measured_ratio(leaf: &Leaf<f64>) -> Option<f32> {
    match leaf {
        Leaf::Measured(value) if value.is_finite() && (0.0..=1.0).contains(value) => {
            Some(*value as f32)
        }
        _ => None,
    }
}

/// Map one country's [`TopbarScalars`] view onto the layout-only
/// [`TopbarBinding`] text/count fields, fail-closed. The flag/ideology
/// identity fields are filled separately by [`apply_identity`].
///
/// Every field is the raw measured value or `None`; see the module header for
/// the UNKNOWN-formatting rationale.
fn apply_scalars(binding: &mut TopbarBinding, scalars: &TopbarScalars) {
    binding.political_power = raw_f64(&scalars.political_power);
    // stability / war_support are recorded as 0.0..=1.0 ratios
    // (REPLAY_SCHEMA.md "stability"/"war_support") and the shipped topbar
    // shows them as percentages. They therefore take the SAME percent
    // projection as the sibling `world_tension_percent` below
    // (`topbar::format_percent`), not the generic raw fallback.
    binding.stability_percent = scalars
        .stability
        .measured()
        .map(|value| format_percent(*value as f32));
    binding.war_support_percent = scalars
        .war_support
        .measured()
        .map(|value| format_percent(*value as f32));
    binding.command_power = raw_f64(&scalars.command_power);
    binding.manpower = raw_i64(&scalars.manpower_available);
    binding.army_experience = raw_f64(&scalars.xp_army);
    binding.navy_experience = raw_f64(&scalars.xp_navy);
    binding.air_experience = raw_f64(&scalars.xp_air);
    // A17 is applied after this generic scalar projection: its displayed value
    // additionally requires the D18 identity and exact NUK1 sidecar.
    binding.nukes = None;
    binding.fuel = raw_f64(&scalars.fuel_current);
    // The engine getter is fixed3 percent (0..=100000 => 0.000..=100.000%),
    // carried as the equivalent fixed5 ratio 0.0..=1.0. This scalar therefore
    // has a proven percent projection rather than the generic raw fallback.
    binding.world_tension_percent = scalars
        .world_tension
        .measured()
        .map(|value| format_percent(*value as f32));
    binding.world_tension = scalars.world_tension.measured().map(|v| *v as f32);
    binding.world_tension_fixed5 = scalars
        .world_tension
        .measured()
        .map(|value| (*value * 100_000.0).round() as i64);
    binding.supply = raw_f64(&scalars.supply_value);
    binding.convoys = raw_f64(&scalars.convoys);
    binding.supply_ratio = measured_ratio(&scalars.supply_ratio);

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

/// Join the D18 identity retained by the recorder to the exact d83 sidecar.
/// A scalar with the same numeric total is not sufficient: two countries can
/// legitimately have equal stockpiles, so identity and value must both agree.
fn apply_exact_nuke_stockpile(
    binding: &mut TopbarBinding,
    frame: &WorldFrame,
    scalars: &TopbarScalars,
) {
    let d18_resolved = frame
        .raw()
        .get("tag_db_id_index")
        .and_then(serde_json::Value::as_object)
        .and_then(|index| index.get("resolved"))
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    binding.nukes = exact_nuke_display_value(
        d18_resolved,
        frame.nuke_stockpile_exact(),
        scalars,
    );
}

fn exact_nuke_display_value(
    d18_resolved: bool,
    exact_view: &replay_core::NukeStockpileExactView,
    scalars: &TopbarScalars,
) -> Option<String> {
    let (Some(tag_db_id), Some(projected_total)) = (
        scalars.nuke_stockpile_tag_db_id.measured().copied(),
        scalars.nukes_total.measured().copied(),
    ) else {
        return None;
    };
    if !d18_resolved || exact_view.status != ViewStatus::Present {
        return None;
    }
    let Some(stockpile) = exact_view.stockpile.as_ref() else {
        return None;
    };
    let mut matches = stockpile
        .countries
        .iter()
        .filter(|country| country.tag_db_id == tag_db_id);
    let Some(exact) = matches.next() else {
        return None;
    };
    if matches.next().is_none() && i64::from(exact.nukes_total) == projected_total {
        Some(projected_total.to_string())
    } else {
        None
    }
}

/// Map the three unambiguous ratio leaves onto their exact authored bars.
/// `xp_ratio` remains intentionally unused: the recording carries one value,
/// while the topbar has distinct army/navy/air overlays, so assigning it to
/// any or all three would fabricate a join.
fn apply_ratios(binding: &mut TopbarBinding, ratios: &TopbarRatios) {
    binding.industrial_ratio = measured_ratio(&ratios.industrial_ratio);
    binding.convoys_ratio = measured_ratio(&ratios.convoy_ratio);
    binding.fuel_ratio = measured_ratio(&ratios.fuel_ratio);
}

fn alert_text(leaf: &Leaf<i64>) -> Option<String> {
    match leaf {
        Leaf::Measured(value) if *value > 0 => Some(value.to_string()),
        _ => None,
    }
}

/// Bind every independently measured a729 topbar alert badge. The exact
/// controller producers hide both text and background at zero.
fn apply_alerts(binding: &mut TopbarBinding, alerts: &replay_core::TopbarAlerts) {
    binding.initiative = alert_text(&alerts.initiative);
    binding.decision_available_items = alert_text(&alerts.decisions_available);
    binding.decision_timeout_items = alert_text(&alerts.decisions_timeout);
    binding.intel_available = alert_text(&alerts.intel);
    binding.intel_prepared_or_completed = alert_text(&alerts.intel_prepared_or_completed);
    binding.prototype_rewards = alert_text(&alerts.prototype_rewards);
    binding.non_delivering_contracts = alert_text(&alerts.market_contracts);
    binding.ongoing_contracts = alert_text(&alerts.market_contracts_ongoing);
}

fn apply_exact_decision_alerts(binding: &mut TopbarBinding, frame: &WorldFrame, pov_tag: &str) {
    let view = frame.decision_panel_exact();
    if view.status != ViewStatus::Present {
        return;
    }
    let Some(panel) = view.panel.as_ref().filter(|panel| panel.country_tag == pov_tag) else {
        return;
    };
    binding.decision_available_items =
        (panel.available_alert_count > 0).then(|| panel.available_alert_count.to_string());
    binding.decision_timeout_items =
        (panel.timeout_alert_count > 0).then(|| panel.timeout_alert_count.to_string());
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
            apply_exact_nuke_stockpile(&mut binding, frame, scalars);
        }
    }

    if frame.topbar_extras_status() == ViewStatus::Present {
        if let Some(ratios) = frame.topbar_ratios(pov_tag) {
            apply_ratios(&mut binding, ratios);
        }
        if let Some(alerts) = frame.topbar_alerts(pov_tag) {
            apply_alerts(&mut binding, alerts);
        }
    }

    // d87 is independently authority- and SelectedPresent-joined.
    apply_exact_decision_alerts(&mut binding, frame, pov_tag);

    binding
}

/// One observer-HUD field which cannot be populated from replay gamestate.
///
/// This is deliberately a named data boundary instead of an empty string or a
/// plausible zero. The viewer owns which country is displayed, but that does
/// not create controller results which the recording never carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTopbarGap {
    pub field: &'static str,
    pub required_domain: &'static str,
    pub reason: &'static str,
}

/// State-first topbar result for the replay observer.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserverTopbarBinding {
    pub binding: TopbarBinding,
    pub gaps: Vec<ObserverTopbarGap>,
}

fn exact_observer_topbar_text(
    catalog: &RuntimeTextCatalog,
    frame_hour: i64,
    country_tag: &str,
) -> BTreeMap<String, RuntimeTextContext> {
    join_exact_runtime_text_view(
        catalog,
        frame_hour,
        country_tag,
        crate::topbar::TOPBAR_WINDOW_NAME,
    )
    .map(|joined| joined.by_element)
    .unwrap_or_default()
}

fn require_exact_observer_format(
    provisional: &mut Option<String>,
    runtime_rows: &BTreeMap<String, RuntimeTextContext>,
    element: &'static str,
    field: &'static str,
    gaps: &mut Vec<ObserverTopbarGap>,
) {
    if provisional.is_none() {
        return;
    }
    if let Some(runtime) = runtime_rows.get(element) {
        // The final bytes are used only to establish element visibility here;
        // Compositor consumes the authenticated context itself.
        *provisional = runtime
            .exact
            .as_ref()
            .map(|exact| exact.engine_utf8.clone());
    } else {
        *provisional = None;
        gaps.push(ObserverTopbarGap {
            field,
            required_domain: "runtime_text",
            reason: "the game-formatted topbar value has no unique complete A25 row for this frame, country, view and element",
        });
    }
}

/// Build the observer-selected country's topbar without recorded
/// selected-Present controller state.
///
/// In particular this path does not consult d79's recorder-owned alert bag or
/// d87's selected-Present decision panel. Scalars, identity, exact stockpile,
/// and the clock are ordinary gamestate. A25 is admitted only as the exact
/// final-text producer for UNKNOWN scalar formatting; its frame/country/view/
/// element join cannot select controller state for another owner. Alert
/// controller outputs are admitted only when an independently typed gamestate
/// family proves the displayed value. Today D65 proves unread prototype
/// rewards; the other alert badges remain named, feature-local gaps.
pub fn topbar_binding_from_observer_frame(
    frame: &WorldFrame,
    country_tag: &str,
) -> ObserverTopbarBinding {
    let mut binding = TopbarBinding::default();

    if frame.country_identities_status() == ViewStatus::Present {
        if let Some(identity) = frame.country_identity(country_tag) {
            apply_identity(&mut binding, country_tag, identity);
        } else {
            binding.country_tag = Some(country_tag.to_string());
        }
    } else {
        binding.country_tag = Some(country_tag.to_string());
    }

    if frame.topbar_status() == ViewStatus::Present {
        if let Some(scalars) = frame.topbar_scalars(country_tag) {
            apply_scalars(&mut binding, scalars);
            apply_exact_nuke_stockpile(&mut binding, frame, scalars);
        }
    }

    let mut gaps = vec![
            ObserverTopbarGap {
                field: "initiative",
                required_domain: "initiative",
                reason: "no replay gamestate family carries the engine-evaluated initiative alert count",
            },
            ObserverTopbarGap {
                field: "decision_available_items",
                required_domain: "d14",
                reason: "d16 active aggregates do not prove evaluated available-decision membership",
            },
            ObserverTopbarGap {
                field: "decision_timeout_items",
                required_domain: "d14",
                reason: "no non-presentation gamestate family carries the evaluated decision timeout alert count",
            },
            ObserverTopbarGap {
                field: "intel_available",
                required_domain: "d30/d53",
                reason: "operation rows do not prove the intelligence controller's displayed available count",
            },
            ObserverTopbarGap {
                field: "intel_prepared_or_completed",
                required_domain: "d30/d53",
                reason: "no admitted semantic leaf carries the intelligence controller's prepared-or-completed count",
            },
            ObserverTopbarGap {
                field: "non_delivering_contracts",
                required_domain: "d43",
                reason: "trade relations do not carry the engine-evaluated non-delivering contract alert count",
            },
            ObserverTopbarGap {
                field: "ongoing_contracts",
                required_domain: "d43",
                reason: "trade relations do not carry the engine-evaluated ongoing contract alert count",
            },
        ];

    let runtime_rows = match frame.runtime_text() {
        Ok(Leaf::Measured(catalog)) => {
            exact_observer_topbar_text(&catalog, frame.hour_index(), country_tag)
        }
        Ok(Leaf::Absent | Leaf::Unobtained) | Err(_) => BTreeMap::new(),
    };
    require_exact_observer_format(
        &mut binding.political_power,
        &runtime_rows,
        "pol_power",
        "political_power",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.manpower,
        &runtime_rows,
        "manpower",
        "manpower",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.stability_percent,
        &runtime_rows,
        "stability",
        "stability_percent",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.war_support_percent,
        &runtime_rows,
        "war_support",
        "war_support_percent",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.command_power,
        &runtime_rows,
        "command_power_value",
        "command_power",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.army_experience,
        &runtime_rows,
        "experience_value",
        "army_experience",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.navy_experience,
        &runtime_rows,
        "navy_experience_value",
        "navy_experience",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.air_experience,
        &runtime_rows,
        "air_experience_value",
        "air_experience",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.nukes,
        &runtime_rows,
        "nukes_value",
        "nukes",
        &mut gaps,
    );
    let mut factory_text = binding
        .factory_text
        .take()
        .or_else(|| {
            binding.factory_counts.take().map(|counts| {
                format!("{}/{}/{}", counts.civilian, counts.military, counts.naval)
            })
        });
    require_exact_observer_format(
        &mut factory_text,
        &runtime_rows,
        "industrial_capacity",
        "factory_counts",
        &mut gaps,
    );
    binding.factory_text = factory_text;
    require_exact_observer_format(
        &mut binding.convoys,
        &runtime_rows,
        "convoys_count",
        "convoys",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.fuel,
        &runtime_rows,
        "fuel_value",
        "fuel",
        &mut gaps,
    );
    require_exact_observer_format(
        &mut binding.supply,
        &runtime_rows,
        "supply_value",
        "supply",
        &mut gaps,
    );
    let mut threat_text = binding
        .world_tension_percent
        .take()
        .or_else(|| binding.world_tension.map(format_percent))
        .or_else(|| {
            binding.world_tension_fixed5.map(|raw| {
                format_percent((raw.clamp(0, 100_000) as f32) / 100_000.0)
            })
        });
    require_exact_observer_format(
        &mut threat_text,
        &runtime_rows,
        "threat_value",
        "world_tension_percent",
        &mut gaps,
    );
    binding.world_tension_percent = threat_text;
    binding.runtime_text_by_element = runtime_rows.clone();
    binding.runtime_text_required_elements = [
        "pol_power",
        "manpower",
        "stability",
        "war_support",
        "command_power_value",
        "experience_value",
        "navy_experience_value",
        "air_experience_value",
        "nukes_value",
        "industrial_capacity",
        "convoys_count",
        "fuel_value",
        "supply_value",
        "threat_value",
        "initiative_text",
        "decisionview_amount_timeout_items",
        "decisionview_amount_to_take_items",
        "intelview_ops_prepared_or_completed_items",
        "intelview_ops_available_items",
        "prototype_rewards_aquired",
        "non_delivering_contract_amount",
        "ongoing_contract_amount",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    // D65 is ordinary per-country gamestate and carries the exact unread
    // count. Overflow is an unavailable value, never a saturated badge.
    let prototype_total = if frame.special_project_programs_status() == ViewStatus::Present {
        match frame.special_project_programs(country_tag) {
            Leaf::Measured(programs) => programs.iter().try_fold(0u64, |sum, row| {
                sum.checked_add(u64::from(row.unread_prototype_rewards))
            }),
            Leaf::Absent | Leaf::Unobtained => None,
        }
    } else {
        None
    };
    match prototype_total {
        Some(total) => {
            binding.prototype_rewards = (total > 0).then(|| total.to_string());
        }
        None => gaps.push(ObserverTopbarGap {
            field: "prototype_rewards",
            required_domain: "d65",
            reason: "complete selected-country unread prototype rewards are unavailable or overflowed",
        }),
    }
    for (value, element, field) in [
        (
            &mut binding.initiative,
            "initiative_text",
            "initiative",
        ),
        (
            &mut binding.decision_timeout_items,
            "decisionview_amount_timeout_items",
            "decision_timeout_items",
        ),
        (
            &mut binding.decision_available_items,
            "decisionview_amount_to_take_items",
            "decision_available_items",
        ),
        (
            &mut binding.intel_prepared_or_completed,
            "intelview_ops_prepared_or_completed_items",
            "intel_prepared_or_completed",
        ),
        (
            &mut binding.intel_available,
            "intelview_ops_available_items",
            "intel_available",
        ),
        (
            &mut binding.prototype_rewards,
            "prototype_rewards_aquired",
            "prototype_rewards",
        ),
        (
            &mut binding.non_delivering_contracts,
            "non_delivering_contract_amount",
            "non_delivering_contracts",
        ),
        (
            &mut binding.ongoing_contracts,
            "ongoing_contract_amount",
            "ongoing_contracts",
        ),
    ] {
        require_exact_observer_format(
            value,
            &runtime_rows,
            element,
            field,
            &mut gaps,
        );
    }

    ObserverTopbarBinding {
        binding,
        gaps,
    }
}

/// Exact D57 WT3X tooltip/popup population seam. This deliberately returns
/// one complete optional binding: a partial ledger, unresolved D18 identity,
/// missing policy threshold, or missing decay input keeps both consumers
/// closed instead of mixing authenticated and fabricated fields.
pub fn world_tension_consumers_from_frame(
    frame: &WorldFrame,
    pov_tag: &str,
) -> Option<crate::world_tension::WorldTensionConsumerBinding> {
    crate::world_tension::binding_from_frame(frame, pov_tag)
}

/// The LAB4 ideology-resolved country **display-name** localisation key
/// candidates, most specific first (the label law in
/// `replay_interface/documents/MAP_LABEL_LAW.md`):
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

// ===========================================================================
// D3 — Outliner (army / navy / air unit lists) population
// ===========================================================================
//
// The outliner layout consumer is [`crate::hud::OutlinerBinding`] (flat row
// list). This seam turns the `replay_core` per-hour unit views
// (`division_units` / `fleets` / `wings`) into POV-keyed army/navy/air rows.
//
// FILE labels:
//   * Row *icon* sprites (`GFX_theatre_army_shield` / `_task_force_shield` /
//     `_air_force_shield`) are FILE — the shipped `interface/theatreselector`
//     sprites already resolved by the outliner layout gate in `hud.rs`.
//   * Row *text* is the unit UID (`div_uid` / `task_force_uid` / `wing_uid`).
//     the game's outliner shows a formatted unit *name* + status; no shipped file
//     citation for that formatter exists in this decoupled context, so the row
//     label is the raw stable identity string (UNKNOWN formatting, rendered
//     raw per Constraint 3).
//   * Row *progress* is fail-closed `None` — no cited law maps unit
//     strength/org onto the outliner progress bar.
//
// Fail-closed: a unit whose `owner` leaf is not `Measured` cannot be attributed
// to any POV and is omitted from every POV list. A domain that is
// absent/unavailable yields an empty list plus the carried [`ViewStatus`].

/// The theatre-shield icon sprites keyed per unit branch (FILE:
/// `interface/theatreselector.gfx`, same sprites the outliner layout gate
/// resolves in `hud.rs`).
pub const OUTLINER_ARMY_ICON: &str = "GFX_theatre_army_shield";
pub const OUTLINER_NAVY_ICON: &str = "GFX_theatre_task_force_shield";
pub const OUTLINER_AIR_ICON: &str = "GFX_theatre_air_force_shield";

/// One populated outliner row: the stable unit identity plus its branch icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerUnitRow {
    /// The unit UID (`div_uid` / `task_force_uid` / `wing_uid`) — the row key.
    pub uid: String,
    /// Branch icon sprite (one of the `OUTLINER_*_ICON` constants).
    pub icon_sprite: String,
}

/// POV-keyed army/navy/air lists plus each branch's source-view status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerUnitLists {
    pub pov_tag: String,
    pub army: Vec<OutlinerUnitRow>,
    pub navy: Vec<OutlinerUnitRow>,
    pub air: Vec<OutlinerUnitRow>,
    pub army_status: ViewStatus,
    pub navy_status: ViewStatus,
    pub air_status: ViewStatus,
}

/// True when a leaf owner string equals `pov_tag` (measured only).
fn owner_is(owner: &Leaf<String>, pov_tag: &str) -> bool {
    matches!(owner, Leaf::Measured(tag) if tag == pov_tag)
}

/// Build the POV-keyed army/navy/air outliner lists for one world frame (D3).
///
/// Army rows come from `division_units` (row key `div_uid`), navy rows from
/// `fleets` (row key `task_force_uid` — the addressable presence unit), and air
/// rows from `wings` (row key `wing_uid`); each is owner-filtered to `pov_tag`.
pub fn outliner_unit_lists_from_frame(frame: &WorldFrame, pov_tag: &str) -> OutlinerUnitLists {
    let army = frame
        .division_units()
        .iter()
        .filter(|d| owner_is(&d.owner, pov_tag))
        .map(|d| OutlinerUnitRow {
            uid: d.div_uid.clone(),
            icon_sprite: OUTLINER_ARMY_ICON.to_string(),
        })
        .collect();
    let navy = frame
        .fleets()
        .iter()
        .filter(|f| owner_is(&f.owner, pov_tag))
        .map(|f| OutlinerUnitRow {
            uid: f.task_force_uid.clone(),
            icon_sprite: OUTLINER_NAVY_ICON.to_string(),
        })
        .collect();
    let air = frame
        .wings()
        .iter()
        .filter(|w| owner_is(&w.owner, pov_tag))
        .map(|w| OutlinerUnitRow {
            uid: w.wing_uid.clone(),
            icon_sprite: OUTLINER_AIR_ICON.to_string(),
        })
        .collect();
    OutlinerUnitLists {
        pov_tag: pov_tag.to_string(),
        army,
        navy,
        air,
        army_status: frame.division_units_status(),
        navy_status: frame.fleets_status(),
        air_status: frame.wings_status(),
    }
}

/// Flatten the typed army→navy→air lists into the layout-only
/// [`OutlinerBinding`] the existing outliner compositor consumes. Row text is
/// the raw UID (UNKNOWN formatting) and progress is fail-closed `None`.
pub fn outliner_binding_from_lists(lists: &OutlinerUnitLists) -> OutlinerBinding {
    let mut rows = Vec::with_capacity(lists.army.len() + lists.navy.len() + lists.air.len());
    for row in lists.army.iter().chain(&lists.navy).chain(&lists.air) {
        rows.push(OutlinerRowBinding {
            icon_sprite: row.icon_sprite.clone(),
            text: row.uid.clone(),
            progress: None,
        });
    }
    OutlinerBinding {
        collapsed: false,
        rows,
    }
}

/// Convenience: build the flat outliner binding for a POV directly from a
/// frame (D3 population path used by the HUD).
pub fn outliner_binding_from_frame(frame: &WorldFrame, pov_tag: &str) -> OutlinerBinding {
    outliner_binding_from_lists(&outliner_unit_lists_from_frame(frame, pov_tag))
}

// ===========================================================================
// D4 — Country / diplomacy panel (read-only) population
// ===========================================================================
//
// Bound from `replay_core`'s [`DiplomacyView`] (requirements items 8/8b):
//   * `factions`  — the global faction-membership matrix (name, leader,
//     sorted members), from each country's `politics.faction`.
//   * `relations` — one row per *other* country from `pov_tag`'s perspective,
//     each flag `Some(bool)` only when the pair leaf is `Measured`, else
//     `None` (fail-closed; an unrecorded relation is never asserted false).
//   * `wars`      — the POV's belligerents (relation rows with `at_war ==
//     Some(true)`), i.e. the panel's war rows.
//   * `subject_autonomy` - A16's complete authenticated current-status set,
//     filtered to rows involving the POV. State identities remain raw hashes;
//     no unproven display-name or formatting join is introduced here.
//
// KNOWN VIEW GAPS (data present in the recording but NOT surfaced by the
// read-only `DiplomacyView`, so absent here):
//   * `diplomacy_world.wars` war objects (attackers/defenders/front/start) —
//     the view exposes only per-pair `at_war` booleans, so a war's belligerent
//     grouping is reconstructed from directional pairs, not read as an object.
//   * `diplomacy_live.war_relations` casualties — not surfaced.
//   * `diplomacy.relation_flags` — the view derives the same facts from the
//     per-country lists, so this is redundant and unread.
// Faction roster completeness is surfaced independently below; unavailable is
// never represented as an available empty roster.

/// One faction and its membership (D4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionRow {
    /// Authoritative roster identity. Legacy recordings may carry a display
    /// name; current d53 rows carry their numeric faction id while name_hash
    /// has no certified localisation resolver.
    pub faction_name: String,
    /// The member flagged `is_leader`, when the recording marks one.
    pub leader: Option<String>,
    /// Member tags, sorted ascending.
    pub members: Vec<String>,
}

/// One directed relation row `pov_tag → other_tag` (D4). Each flag is
/// `Some(bool)` only when the pair leaf was measured; `None` is fail-closed
/// "not recorded", never a fabricated `false`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelationRow {
    pub other_tag: String,
    pub at_war: Option<bool>,
    pub same_faction: Option<bool>,
    pub non_aggression_pact: Option<bool>,
    pub military_access: Option<bool>,
    pub guarantee: Option<bool>,
    pub embargo: Option<bool>,
    pub subject: Option<bool>,
}

/// One authenticated A16 autonomy row involving the selected country. State
/// identities remain exact hashes because no reviewed localisation join for
/// autonomy-state definitions is part of this binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectAutonomyPanelRow {
    pub counterpart: Option<String>,
    pub counterpart_tag_db_id: u32,
    pub pov_is_overlord: bool,
    pub current_state_hash: u32,
    pub previous_state_hash: Option<u32>,
    pub next_state_hash: Option<u32>,
    pub ratio_raw: i32,
    pub current_score_raw: i64,
    pub lower_score_raw: i64,
    pub upper_score_raw: i64,
    pub last_modified_date: Option<String>,
    pub last_modified_date_raw: u32,
    pub direction: Option<SubjectAutonomyDirection>,
}

/// Read-only country/diplomacy panel binding for one POV at one hour (D4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiplomacyPanelBinding {
    pub pov_tag: String,
    /// `true` only when the underlying `DiplomacyView` status is `Present`.
    pub available: bool,
    /// Independent roster availability. Overall diplomacy can be available
    /// while d53 faction membership is unavailable for this hour.
    pub factions_available: bool,
    /// Preserves unavailable vs measured-empty vs measured-nonempty for the UI.
    pub faction_roster_status: FactionRosterStatus,
    /// Global faction-membership matrix, sorted by roster identity.
    pub factions: Vec<FactionRow>,
    /// `pov_tag → other` relation rows, one per other country, sorted by tag.
    pub relations: Vec<RelationRow>,
    /// The POV's belligerents (relation rows with `at_war == Some(true)`).
    pub wars: Vec<String>,
    /// `true` only when the complete A16 current-status set was measured.
    pub subject_autonomy_available: bool,
    /// Exact current autonomy rows involving the POV. An available empty list
    /// is distinct from an unavailable source via `subject_autonomy_available`.
    pub subject_autonomy: Vec<SubjectAutonomyPanelRow>,
}

fn bool_opt(leaf: &Leaf<bool>) -> Option<bool> {
    match leaf {
        Leaf::Measured(v) => Some(*v),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

/// Co-derive roster status and rows from the one typed d53/legacy roster leaf.
/// Country politics cannot create, suppress, or assign membership rows;
/// faction_is_leader only decorates a member already in that roster.
fn faction_rows(view: &DiplomacyView) -> (FactionRosterStatus, Vec<FactionRow>) {
    match &view.faction_members {
        Leaf::Measured(by_faction) if by_faction.is_empty() => {
            (FactionRosterStatus::MeasuredEmpty, Vec::new())
        }
        Leaf::Measured(by_faction) => {
            let rows = by_faction
                .iter()
                .map(|(faction_identity, roster_members)| {
                    let leader = roster_members
                        .iter()
                        .find(|member| {
                            view.countries.get(*member).is_some_and(|country| {
                                matches!(country.faction_is_leader, Leaf::Measured(true))
                            })
                        })
                        .cloned();
                    let mut members = roster_members.clone();
                    members.sort();
                    FactionRow {
                        faction_name: faction_identity.clone(),
                        leader,
                        members,
                    }
                })
                .collect();
            (FactionRosterStatus::MeasuredNonEmpty, rows)
        }
        Leaf::Unobtained | Leaf::Absent => (FactionRosterStatus::Unavailable, Vec::new()),
    }
}

/// `pov_tag → other` relation rows (one per other country in the view).
fn relation_rows(view: &DiplomacyView, pov_tag: &str) -> Vec<RelationRow> {
    let mut rows = Vec::new();
    for other in view.countries.keys() {
        if other == pov_tag {
            continue;
        }
        let pair = view.pairs.get(&(pov_tag.to_string(), other.clone()));
        rows.push(RelationRow {
            other_tag: other.clone(),
            at_war: pair.and_then(|p| bool_opt(&p.at_war)),
            same_faction: pair.and_then(|p| bool_opt(&p.same_faction)),
            non_aggression_pact: pair.and_then(|p| bool_opt(&p.non_aggression_pact)),
            military_access: pair.and_then(|p| bool_opt(&p.military_access)),
            guarantee: pair.and_then(|p| bool_opt(&p.guarantee)),
            embargo: pair.and_then(|p| bool_opt(&p.embargo)),
            subject: pair.and_then(|p| bool_opt(&p.subject)),
        });
    }
    rows
}

/// Resolve the POV tag's numeric database identity for the D53 subject-
/// autonomy join, namespace-safely.
///
/// The d18 `_r2_tag_db_id` map is authoritative: when the frame carries a
/// measured id for `pov_tag`, that id is the ONLY numeric identity used. The
/// canonical-numeric-spelling fallback (REPLAY_SCHEMA.md, subject-autonomy
/// identity law) applies only when `pov_tag` is NOT a recorded country tag —
/// a real three-character tag may be all digits (e.g. `"123"`), and parsing
/// it would otherwise match unrelated rows whose `*_tag_db_id` happens to
/// equal that spelling while the tag's own database id differs.
fn resolve_pov_numeric_identity(
    frame: &WorldFrame,
    view: &DiplomacyView,
    pov_tag: &str,
) -> Option<u32> {
    if let Some(id) = frame
        .politics_detail(pov_tag)
        .and_then(|detail| detail.tag_db_id.measured().copied())
    {
        return Some(id);
    }
    let recorded_tag = view.countries.contains_key(pov_tag)
        || frame.politics_detail(pov_tag).is_some()
        || frame.countries().contains_key(pov_tag);
    if recorded_tag {
        // A recorded tag whose database id is unmeasured this hour: numeric
        // matching is disabled rather than guessed from the spelling.
        None
    } else {
        pov_tag.parse::<u32>().ok()
    }
}

fn subject_autonomy_rows(
    view: &DiplomacyView,
    pov_tag: &str,
    pov_id: Option<u32>,
) -> (bool, Vec<SubjectAutonomyPanelRow>) {
    let Leaf::Measured(rows) = &view.subject_autonomy_current else {
        return (false, Vec::new());
    };
    let mut projected = rows
        .iter()
        .filter_map(|row| {
            let overlord_matches = row.overlord.measured().is_some_and(|tag| tag == pov_tag)
                || pov_id == Some(row.overlord_tag_db_id);
            let subject_matches = row.subject.measured().is_some_and(|tag| tag == pov_tag)
                || pov_id == Some(row.subject_tag_db_id);
            let (pov_is_overlord, counterpart, counterpart_tag_db_id) =
                if overlord_matches && !subject_matches {
                    (
                        true,
                        row.subject.measured().cloned(),
                        row.subject_tag_db_id,
                    )
                } else if subject_matches && !overlord_matches {
                    (
                        false,
                        row.overlord.measured().cloned(),
                        row.overlord_tag_db_id,
                    )
                } else {
                    return None;
                };
            Some(SubjectAutonomyPanelRow {
                counterpart,
                counterpart_tag_db_id,
                pov_is_overlord,
                current_state_hash: row.current_state_hash,
                previous_state_hash: row.previous_state_hash,
                next_state_hash: row.next_state_hash,
                ratio_raw: row.ratio_raw,
                current_score_raw: row.current_score_raw,
                lower_score_raw: row.lower_score_raw,
                upper_score_raw: row.upper_score_raw,
                last_modified_date: row.last_modified_date.measured().cloned(),
                last_modified_date_raw: row.last_modified_date_raw,
                direction: row.direction.measured().copied(),
            })
        })
        .collect::<Vec<_>>();
    projected.sort_by_key(|row| (row.pov_is_overlord, row.counterpart_tag_db_id));
    (true, projected)
}

/// Build the read-only diplomacy panel for `pov_tag` at one world frame (D4).
/// When the diplomacy view is not `Present`, returns the fail-closed empty
/// binding (`available: false`, no faction/relation rows).
pub fn diplomacy_panel_from_frame(frame: &WorldFrame, pov_tag: &str) -> DiplomacyPanelBinding {
    let view = frame.diplomacy();
    if view.status != ViewStatus::Present {
        return DiplomacyPanelBinding {
            pov_tag: pov_tag.to_string(),
            available: false,
            ..DiplomacyPanelBinding::default()
        };
    }
    let relations = relation_rows(view, pov_tag);
    let wars = relations
        .iter()
        .filter(|r| r.at_war == Some(true))
        .map(|r| r.other_tag.clone())
        .collect();
    let (faction_roster_status, factions) = faction_rows(view);
    let (subject_autonomy_available, subject_autonomy) = subject_autonomy_rows(
        view,
        pov_tag,
        resolve_pov_numeric_identity(frame, view, pov_tag),
    );
    DiplomacyPanelBinding {
        pov_tag: pov_tag.to_string(),
        available: true,
        factions_available: faction_roster_status.is_measured(),
        faction_roster_status,
        factions,
        relations,
        wars,
        subject_autonomy_available,
        subject_autonomy,
    }
}

// ===========================================================================
// D6 — Replay-native date/speed clock
// ===========================================================================
//
// The topbar clock shows the RECORDING's current time (from the frame's own
// `date` string) plus the replay app's PLAYBACK speed (the transport step). It
// parses no recording bytes: the date is a typed `WorldFrame::date()` view.
//
// Format law (FILE): `topbar::format_topbar_datetime` — the shipped
// `interface/topbar.gui` `DateText` placeholder `"7:00 29, September 1939"`
// (`{hour}:00 {day}, {MonthName} {year}`) + the `core_l_english.yml` month
// names. The month name is resolved through the recording's localisation so a
// non-English recording renders its own month names.
//
// The speed is the transport `speed_step` (a *playback* control) mapped onto
// the five topbar speed-step buttons; it is **FIT (product UX)**, not the
// engine game-speed law (see `replay_app::transport`). It is NOT the
// recording's own `world.scenario.speed` — that is the game's recorded speed,
// a separate datum this clock deliberately does not display.

/// Month-name localisation keys the clock resolves, index 0..=11 == the shipped
/// `core_l_english.yml` `January`..`December` keys (FILE). Shipping playback
/// keeps the selected key inside the `$KEY$` template until the compositor so
/// its exact, unambiguous source is checked and recorded with every other
/// resolved localisation key.
pub const TOPBAR_CLOCK_MONTH_LOC_KEYS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A parsed Clausewitz `Y.M.D.H` recording date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockDate {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
}

/// Parse a Clausewitz recording date string (`Y.M.D.H`, the REPLAY_SCHEMA
/// `date` shape). Fail-closed: returns `None` unless year/month/day/hour all
/// parse, `month` is 1..=12, `day` is 1..=31, and `hour` is 0..=23. A
/// date-only `Y.M.D` string is refused — the clock renders the hour, and a
/// missing hour must not become a fabricated midnight.
pub fn parse_clock_date(date: &str) -> Option<ClockDate> {
    let mut parts = date.trim().split('.');
    let year: i64 = parts.next()?.trim().parse().ok()?;
    let month: u32 = parts.next()?.trim().parse().ok()?;
    let day: u32 = parts.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let hour: u32 = parts.next()?.trim().parse().ok()?;
    if hour > 23 {
        return None;
    }
    Some(ClockDate {
        year,
        month,
        day,
        hour,
    })
}

/// FILE clock format `{hour}:00 {day}, {MonthName} {year}` (see the module note
/// and `topbar::format_topbar_datetime`).
fn format_clock_date(date: ClockDate, month_name: &str) -> String {
    format!(
        "{}:00 {}, {} {}",
        date.hour, date.day, month_name, date.year
    )
}

/// Build the authored topbar date template while preserving its month
/// localisation key for the compositor's normal provenance gate.
fn format_topbar_date_template(date: &str) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    let key = TOPBAR_CLOCK_MONTH_LOC_KEYS[(parsed.month - 1) as usize];
    Some(format_clock_date(parsed, &format!("${key}$")))
}

/// Format a recording date string for non-product callers which explicitly
/// need a final string. The month must have one exact source; missing or
/// ambiguous definitions fail closed instead of fabricating the English key.
pub fn format_topbar_date(date: &str, loc: &Localisation) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    let key = TOPBAR_CLOCK_MONTH_LOC_KEYS[(parsed.month - 1) as usize];
    loc.source_file_for_key(key)?;
    let month = loc.get(key)?;
    Some(format_clock_date(parsed, month))
}

/// English-month variant needing no `Localisation`: the month names are the
/// shipped `core_l_english.yml` values (FILE, via `topbar::month_name_english`).
/// Used by the product overlay where a full localisation is not threaded.
pub fn format_topbar_date_english(date: &str) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    Some(format_clock_date(
        parsed,
        month_name_english(parsed.month as u8),
    ))
}

/// Replay-native clock binding (D6): the recording's current time (FILE-
/// template) + the replay app's playback speed step, ready to merge onto a
/// [`TopbarBinding`]. The compositor resolves that template so the month's
/// exact localisation source is included in `ComposeReport`. Fail-closed: a
/// frame with no/unparseable date leaves `date_time` `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockBinding {
    pub date_time: Option<String>,
    pub speed: Option<u8>,
}

/// Map the replay transport's 0-based `speed_step` onto the topbar's five
/// speed-step buttons: step `i` lights buttons `1..=i+1`. Clamped to 1..=5.
/// (Playback control — FIT product UX; see the module note.)
pub fn playback_speed_button(speed_step: usize) -> u8 {
    (speed_step + 1).min(5) as u8
}

/// Build the clock binding for one world frame + the current playback speed
/// step (D6). The date uses the recording's localisation month names.
pub fn clock_binding_from_frame(
    frame: &WorldFrame,
    playback_speed_step: usize,
    _loc: &Localisation,
) -> ClockBinding {
    ClockBinding {
        date_time: frame.date().and_then(format_topbar_date_template),
        speed: Some(playback_speed_button(playback_speed_step)),
    }
}

/// Merge a [`ClockBinding`] onto a [`TopbarBinding`] (D6 population seam). The
/// layout is untouched; this only fills the `date_time`/`speed` fields the
/// existing `compose_topbar` consumes.
pub fn apply_clock(binding: &mut TopbarBinding, clock: &ClockBinding) {
    binding.date_time = clock.date_time.clone();
    binding.speed = clock.speed;
}

// ===========================================================================
// D7 — Tile (state) tooltip content binding
// ===========================================================================
//
// the game's state tooltip is computed-not-stored (a localised sentence + building
// breakdown). Only the *data* it keys on — the state's owner and controller
// tags — is timeline-derived; this seam surfaces exactly that, fail-closed, and
// leaves the localised sentence composition descoped (D7 lane).
// Row text formatting is UNKNOWN → the raw stable tag is carried, never a
// fabricated localised phrase (Constraint 3).

/// Read-only state tooltip data bound from the timeline (D7). `owner` comes
/// from `owner_by_state`, `controller` from `control_by_state`; each is `Some`
/// only when the map leaf is `Measured` (fail-closed, never a fabricated tag).
/// `available` mirrors the owner-view status for this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateTooltipBinding {
    pub state_id: u32,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub available: bool,
}

/// Build the read-only state tooltip binding for `state_id` at one world frame
/// (D7). A state absent from the map layers yields `owner`/`controller` `None`
/// (fail-closed) while still reporting the frame's view availability.
pub fn state_tooltip_from_frame(frame: &WorldFrame, state_id: u32) -> StateTooltipBinding {
    let owner = frame
        .owner_by_state()
        .get(&state_id)
        .and_then(Leaf::measured)
        .cloned();
    let controller = frame
        .control_by_state()
        .get(&state_id)
        .and_then(Leaf::measured)
        .cloned();
    StateTooltipBinding {
        state_id,
        owner,
        controller,
        available: frame.owner_by_state_status() == ViewStatus::Present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_text_record(
        context_id: &str,
        hour: i64,
        country: &str,
        view: &str,
        element: &str,
        text: &str,
    ) -> replay_core::RuntimeTextRecord {
        replay_core::RuntimeTextRecord {
            context_id: context_id.to_string(),
            frame_hour: hour,
            pov_country_or_global: country.to_string(),
            view_id: view.to_string(),
            element_id: element.to_string(),
            source: replay_core::RuntimeTextSource::SetterCallerRva(0x188_A0DE),
            final_text_bytes: text.as_bytes().to_vec(),
            final_text_utf8: text.to_string(),
            final_text_sha256: "fixture".to_string(),
            variables: BTreeMap::new(),
            data_functions: BTreeMap::new(),
            dynamic_colors: BTreeMap::new(),
            layout: replay_core::RuntimeTextLayout::AuthoredReproducible,
            producer: replay_core::RuntimeTextProducer {
                setter_entry_rva: replay_core::A729_TEXT_SETTER_RVA,
                setter_caller_rva: Some(0x188_A0DE),
                formatter_entry_rva: Some(replay_core::A729_FORMATTER_RVA),
                identity_source: replay_core::RuntimeTextIdentitySource::ProvenSetterCaller,
            },
        }
    }

    #[test]
    fn observer_runtime_text_join_is_exact_complete_and_unambiguous() {
        let mut contexts = BTreeMap::new();
        contexts.insert(
            "good".to_string(),
            runtime_text_record("good", 42, "GER", "topbar", "manpower", "1.20M"),
        );
        contexts.insert(
            "wrong-country".to_string(),
            runtime_text_record("wrong-country", 42, "ENG", "topbar", "pol_power", "7"),
        );
        contexts.insert(
            "duplicate-a".to_string(),
            runtime_text_record("duplicate-a", 42, "GER", "topbar", "fuel_value", "1K"),
        );
        contexts.insert(
            "duplicate-b".to_string(),
            runtime_text_record("duplicate-b", 42, "GER", "topbar", "fuel_value", "2K"),
        );
        let selected = exact_observer_topbar_text(
            &RuntimeTextCatalog {
                contexts_complete: true,
                contexts: contexts.clone(),
            },
            42,
            "GER",
        );
        assert_eq!(
            selected
                .get("manpower")
                .and_then(|row| row.exact.as_ref())
                .map(|row| row.engine_utf8.as_str()),
            Some("1.20M")
        );
        assert!(!selected.contains_key("pol_power"));
        assert!(!selected.contains_key("fuel_value"));
        assert!(exact_observer_topbar_text(
            &RuntimeTextCatalog {
                contexts_complete: false,
                contexts,
            },
            42,
            "GER",
        )
        .is_empty());
    }

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
            cores: Leaf::Unobtained,
            claims: Leaf::Unobtained,
            preferred_tactic: Leaf::Unobtained,
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
    fn a17_generic_scalars_cannot_bypass_exact_identity_join() {
        let mut scalars = all_absent_scalars();
        scalars.nuke_stockpile_tag_db_id = Leaf::Measured(23);
        scalars.nukes_total = Leaf::Measured(2);
        scalars.nukes_ready = Leaf::Measured(9);
        let mut binding = TopbarBinding::default();
        apply_scalars(&mut binding, &scalars);
        assert_eq!(binding.nukes, None);
    }

    #[test]
    fn a17_exact_identity_and_total_are_both_required() {
        let mut scalars = all_absent_scalars();
        scalars.nuke_stockpile_tag_db_id = Leaf::Measured(23);
        scalars.nukes_total = Leaf::Measured(2);
        scalars.nukes_ready = Leaf::Measured(9);
        let view = replay_core::NukeStockpileExactView {
            status: ViewStatus::Present,
            stockpile: Some(replay_core::NukeStockpileExact {
                schema_version: 2,
                authority: "exact_a729_nuke_stockpile".into(),
                executable_sha256: "e".into(),
                producer_consumer_manifest_sha256: "p".into(),
                container_constructor_manifest_sha256: "c".into(),
                cnuke_vtable_artifact_sha256: "v".into(),
                elapsed_wall_ms: 1,
                frame_hour: 900,
                source_sequence: 1,
                countries: vec![replay_core::NukeStockpileCountryExact {
                    tag_db_id: 23,
                    container_capacity: 2,
                    nukes_total: 2,
                    nukes_ready_total: 9,
                    nukes: vec![
                        replay_core::NukeStockpileEntryExact {
                            nuke_type_raw: 0,
                            amount_raw: 250_000_000,
                            amount: 2,
                            nukes_ready: 9,
                        },
                        replay_core::NukeStockpileEntryExact {
                            nuke_type_raw: 1,
                            amount_raw: 0,
                            amount: 0,
                            nukes_ready: 0,
                        },
                    ],
                }],
            }),
        };
        assert_eq!(
            exact_nuke_display_value(true, &view, &scalars).as_deref(),
            Some("2")
        );
        assert_eq!(exact_nuke_display_value(false, &view, &scalars), None);
        scalars.nuke_stockpile_tag_db_id = Leaf::Measured(24);
        assert_eq!(exact_nuke_display_value(true, &view, &scalars), None);
        scalars.nuke_stockpile_tag_db_id = Leaf::Measured(23);
        scalars.nukes_total = Leaf::Measured(3);
        assert_eq!(exact_nuke_display_value(true, &view, &scalars), None);
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

    #[test]
    fn topbar_date_reproduces_shipped_gui_placeholder() {
        // interface/topbar.gui DateText placeholder is "7:00 29, September 1939".
        assert_eq!(
            format_topbar_date_english("1939.9.29.7").as_deref(),
            Some("7:00 29, September 1939")
        );
        // Midnight on new year, no leading zeros.
        assert_eq!(
            format_topbar_date_english("1936.1.1.0").as_deref(),
            Some("0:00 1, January 1936")
        );
    }

    #[test]
    fn topbar_date_fail_closed_on_bad_input() {
        assert_eq!(format_topbar_date_english(""), None);
        // Date-only (no hour) is refused — a missing hour must not render as
        // a fabricated midnight.
        assert_eq!(format_topbar_date_english("1944.12.25"), None);
        assert_eq!(parse_clock_date("1944.12.25"), None);
        assert_eq!(format_topbar_date_english("hour:5"), None);
        assert_eq!(format_topbar_date_english("1936.13.1.0"), None); // month OOR
        assert_eq!(format_topbar_date_english("1936.1.0.0"), None); // day 0
        assert_eq!(format_topbar_date_english("1936.1.1.24"), None); // hour OOR
    }

    #[test]
    fn topbar_date_uses_localisation_month_name() {
        let mut loc = Localisation::default();
        loc.entries
            .insert("September".to_string(), "Septembre".to_string());
        loc.source_by_key.insert(
            "September".to_string(),
            "localisation/french/core_l_french.yml".to_string(),
        );
        assert_eq!(
            format_topbar_date("1939.9.29.7", &loc).as_deref(),
            Some("7:00 29, Septembre 1939")
        );
        assert_eq!(format_topbar_date("1939.1.1.0", &loc), None);

        loc.entries
            .insert("January".to_string(), "Janvier".to_string());
        loc.ambiguous_sources_by_key.insert(
            "January".to_string(),
            vec![
                "localisation/french/a_l_french.yml".to_string(),
                "localisation/french/b_l_french.yml".to_string(),
            ],
        );
        assert_eq!(format_topbar_date("1939.1.1.0", &loc), None);
    }

    #[test]
    fn playback_speed_button_maps_and_clamps() {
        assert_eq!(playback_speed_button(0), 1);
        assert_eq!(playback_speed_button(1), 2);
        assert_eq!(playback_speed_button(4), 5);
        assert_eq!(playback_speed_button(7), 5); // clamp at 5 buttons
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
            nuke_stockpile_tag_db_id: Leaf::Absent,
            nukes_total: Leaf::Absent,
            nukes_ready: Leaf::Absent,
            world_tension: Leaf::Absent,
            supply_value: Leaf::Absent,
            supply_ratio: Leaf::Absent,
        }
    }

    #[test]
    fn d63_typed_topbar_scalars_reach_render_binding_without_defaults() {
        let mut scalars = all_absent_scalars();
        scalars.manpower_available = Leaf::Measured(1_640_616);
        scalars.xp_army = Leaf::Measured(27.500427);
        scalars.xp_navy = Leaf::Measured(422.10611);
        scalars.xp_air = Leaf::Measured(500.0);
        let mut binding = TopbarBinding::default();
        apply_scalars(&mut binding, &scalars);
        assert_eq!(binding.manpower.as_deref(), Some("1640616"));
        assert_eq!(binding.army_experience.as_deref(), Some("27.500427"));
        assert_eq!(binding.navy_experience.as_deref(), Some("422.10611"));
        assert_eq!(binding.air_experience.as_deref(), Some("500"));
    }

    /// The stability / war-support topbar ratios are percent-projected via the
    /// SAME rule as the sibling `world_tension_percent`: `format_percent` of
    /// the measured 0..=1 fraction, string-exact. Unmeasured leaves stay the
    /// fail-closed `None`, never `"0%"`.
    #[test]
    fn stability_and_war_support_project_percent_like_world_tension() {
        let mut scalars = all_absent_scalars();
        scalars.stability = Leaf::Measured(0.74);
        scalars.war_support = Leaf::Measured(0.305);
        scalars.world_tension = Leaf::Measured(0.74);
        let mut binding = TopbarBinding::default();
        apply_scalars(&mut binding, &scalars);
        // Pin the exact projected strings...
        assert_eq!(binding.stability_percent.as_deref(), Some("74%"));
        assert_eq!(binding.war_support_percent.as_deref(), Some("31%"));
        // ...and pin the rule itself against the sibling projection: the same
        // fraction renders identically through both fields.
        assert_eq!(binding.stability_percent, binding.world_tension_percent);
        assert_eq!(
            binding.war_support_percent,
            Some(format_percent(0.305f32))
        );

        let mut absent = TopbarBinding::default();
        apply_scalars(&mut absent, &all_absent_scalars());
        assert_eq!(absent.stability_percent, None);
        assert_eq!(absent.war_support_percent, None);
        let mut unobtained_scalars = all_absent_scalars();
        unobtained_scalars.stability = Leaf::Unobtained;
        unobtained_scalars.war_support = Leaf::Unobtained;
        let mut unobtained = TopbarBinding::default();
        apply_scalars(&mut unobtained, &unobtained_scalars);
        assert_eq!(unobtained.stability_percent, None);
        assert_eq!(unobtained.war_support_percent, None);
    }

    // ---- D53 subject-autonomy POV identity (namespace-safe numeric join) ----

    fn autonomy_view(
        rows: Vec<replay_core::SubjectAutonomyCurrentRow>,
    ) -> DiplomacyView {
        DiplomacyView {
            status: ViewStatus::Present,
            pairs: Default::default(),
            countries: Default::default(),
            war_relations: Leaf::Absent,
            wars: Leaf::Absent,
            faction_members: Leaf::Absent,
            operation_instances: Leaf::Absent,
            operation_token_memberships: Leaf::Absent,
            intel_levels: Leaf::Absent,
            air_disclosure: Leaf::Absent,
            intel_network_graphs: Leaf::Absent,
            subject_autonomy_current: Leaf::Measured(rows),
            agency_upgrade_controller_refresh: Leaf::Absent,
            cipher_controller_refresh: Leaf::Absent,
            intelligence_controller_companion: Leaf::Absent,
            domain53_snapshot_payload_sha256: Leaf::Absent,
            relation_flags: Default::default(),
        }
    }

    fn autonomy_row(
        overlord_id: u32,
        subject_id: u32,
    ) -> replay_core::SubjectAutonomyCurrentRow {
        replay_core::SubjectAutonomyCurrentRow {
            overlord: Leaf::Unobtained,
            subject: Leaf::Unobtained,
            overlord_tag_db_id: overlord_id,
            subject_tag_db_id: subject_id,
            current_state_hash: 1,
            previous_state_hash: None,
            next_state_hash: None,
            ratio_raw: 0,
            current_score_raw: 0,
            lower_score_raw: 0,
            upper_score_raw: 0,
            last_modified_date: Leaf::Unobtained,
            last_modified_date_raw: 0,
            direction: Leaf::Unobtained,
        }
    }

    /// A POV whose resolved database id is 456 must join ONLY id-456 rows —
    /// never a row whose id happens to equal the POV tag's numeric spelling
    /// (the `"123"`-as-real-TAG collision), and a POV with no numeric
    /// identity joins no row at all.
    #[test]
    fn subject_autonomy_numeric_join_uses_resolved_id_not_tag_spelling() {
        let view = autonomy_view(vec![autonomy_row(123, 7), autonomy_row(456, 8)]);

        let (available, rows) = subject_autonomy_rows(&view, "123", Some(456));
        assert!(available);
        assert_eq!(rows.len(), 1, "only the resolved-id row joins: {rows:?}");
        assert!(rows[0].pov_is_overlord);
        assert_eq!(rows[0].counterpart_tag_db_id, 8);

        let (available, rows) = subject_autonomy_rows(&view, "123", None);
        assert!(available, "the collection stays available");
        assert!(
            rows.is_empty(),
            "no numeric identity => no spelling-guessed join: {rows:?}"
        );
    }

    struct MemorySource {
        hours: Vec<i64>,
        frames: std::collections::BTreeMap<i64, serde_json::Value>,
    }

    impl replay_core::ReplaySource for MemorySource {
        fn game_id(&self) -> &str {
            "binding_unit_test"
        }
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn warnings(&self) -> &[String] {
            &[]
        }
        fn frame(
            &self,
            hour_index: i64,
        ) -> Result<replay_core::RawFrame, replay_core::SourceError> {
            let rest = self
                .frames
                .get(&hour_index)
                .and_then(serde_json::Value::as_object)
                .cloned()
                .ok_or_else(|| {
                    replay_core::SourceError::Open(format!("no frame at {hour_index}"))
                })?;
            Ok(replay_core::RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest,
            })
        }
    }

    fn frame_from_json(rest: serde_json::Value) -> replay_core::WorldTimeline {
        let source = MemorySource {
            hours: vec![0],
            frames: [(0, rest)].into_iter().collect(),
        };
        replay_core::WorldTimeline::open(Box::new(source)).expect("open memory timeline")
    }

    #[test]
    fn pov_numeric_identity_resolves_through_d18_map_before_spelling() {
        // A real recorded tag "123" whose d18 database id is 456.
        let timeline = frame_from_json(serde_json::json!({
            "countries": { "123": { "_r2_tag_db_id": 456 } }
        }));
        let frame = timeline.frame(0).expect("frame");
        let view = frame.diplomacy();
        assert_eq!(
            resolve_pov_numeric_identity(&frame, view, "123"),
            Some(456),
            "the d18 map is authoritative over the numeric spelling"
        );
        // A numeric spelling that is NOT a recorded tag keeps the deliberate
        // canonical-numeric-spelling fallback identity.
        assert_eq!(resolve_pov_numeric_identity(&frame, view, "789"), Some(789));

        // A recorded tag with an unmeasured id disables numeric matching
        // rather than guessing from the spelling.
        let timeline = frame_from_json(serde_json::json!({
            "countries": { "123": {} }
        }));
        let frame = timeline.frame(0).expect("frame");
        let view = frame.diplomacy();
        assert_eq!(resolve_pov_numeric_identity(&frame, view, "123"), None);
    }
}
