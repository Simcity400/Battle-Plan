
use crate::hud::{OutlinerBinding, OutlinerRowBinding};
use crate::loc::Localisation;
use crate::text_law::{join_exact_runtime_text_view, RuntimeTextContext};
use crate::topbar::{
    flag_candidates_for_size, format_percent, month_name_english, FactoryCounts, FlagSizeFolder,
    TopbarBinding,
};
use replay_core::NamedCountry;
use replay_core::world::FactionRosterStatus;
use replay_core::{
    CountryIdentity, DiplomacyView, Leaf, SubjectAutonomyDirection, TopbarRatios, TopbarScalars,
    RuntimeTextCatalog, ViewStatus, WorldFrame,
};
use std::collections::BTreeMap;

fn raw_f64(leaf: &Leaf<f64>) -> Option<String> {
    match leaf {
        Leaf::Measured(v) => Some(format!("{v}")),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

fn raw_i64(leaf: &Leaf<i64>) -> Option<String> {
    match leaf {
        Leaf::Measured(v) => Some(format!("{v}")),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

fn measured_u32(leaf: &Leaf<i64>) -> Option<u32> {
    match leaf {
        Leaf::Measured(v) if *v >= 0 => u32::try_from(*v).ok(),
        _ => None,
    }
}

fn measured_ratio(leaf: &Leaf<f64>) -> Option<f32> {
    match leaf {
        Leaf::Measured(value) if value.is_finite() && (0.0..=1.0).contains(value) => {
            Some(*value as f32)
        }
        _ => None,
    }
}

fn apply_scalars(binding: &mut TopbarBinding, scalars: &TopbarScalars) {
    binding.political_power = raw_f64(&scalars.political_power);
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
    binding.nukes = None;
    binding.fuel = raw_f64(&scalars.fuel_current);
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
    binding.factory_text = None;
}

fn apply_exact_nuke_stockpile(
    binding: &mut TopbarBinding,
    frame: &WorldFrame,
    scalars: &TopbarScalars,
) {
    let d18_resolved = frame.tag_db_id_index_resolved() == Some(true);
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

pub(crate) fn rendered_cosmetic_override(leaf: &Leaf<Option<String>>) -> Option<&str> {
    match leaf {
        Leaf::Measured(Some(tag)) => Some(tag.as_str()),
        Leaf::Measured(None) | Leaf::Absent | Leaf::Unobtained => None,
    }
}

pub(crate) fn admitted_cosmetic_override(
    leaf: &Leaf<Option<String>>,
) -> std::result::Result<Option<&str>, &'static str> {
    match leaf {
        Leaf::Measured(Some(tag)) if !tag.trim().is_empty() => Ok(Some(tag.as_str())),
        Leaf::Measured(Some(_)) => Err("measured cosmetic tag is empty"),
        Leaf::Measured(None) => Ok(None),
        Leaf::Absent => Err("cosmetic tag was never carried for this country"),
        Leaf::Unobtained => Err("cosmetic tag is unobtained"),
    }
}

pub(crate) fn admitted_flag_cosmetic(
    identity: Option<&CountryIdentity>,
) -> std::result::Result<Option<&str>, &'static str> {
    match identity {
        Some(identity) => admitted_cosmetic_override(&identity.cosmetic_tag),
        None => Ok(None),
    }
}

fn apply_identity(binding: &mut TopbarBinding, pov_tag: &str, identity: &CountryIdentity) {
    binding.country_tag = Some(pov_tag.to_string());
    binding.country_ideology = identity.ruling_ideology.measured().cloned();
    binding.cosmetic_tag = rendered_cosmetic_override(&identity.cosmetic_tag).map(str::to_string);
}

/// The three fields a flag route needs, without the alert trailer.
///
/// `topbar_binding_from_frame` fills eight d79 fields into the same value, so a
/// shipping site holding one could be made to depend on d79 without naming a
/// carrier. EIR-NE-017: the flag sites take this instead, and never hold the
/// alert-bearing binding at all.
pub struct TopbarFlagIdentity {
    pub country_tag: Option<String>,
    pub country_ideology: Option<String>,
    pub flag_texture: Option<String>,
}

pub fn topbar_flag_identity_from_frame(frame: &WorldFrame, pov_tag: &str) -> TopbarFlagIdentity {
    // Same values the full binding carried for these three: `country_tag` is
    // `Some(pov_tag)` on every branch there, `country_ideology` comes only from
    // `apply_identity`, and `flag_texture` is never assigned anywhere.
    let country_ideology = if frame.country_identities_status() == ViewStatus::Present {
        frame
            .country_identity(pov_tag)
            .and_then(|identity| identity.ruling_ideology.measured().cloned())
    } else {
        None
    };
    // A key two ids reach names no country, so it is not the name of the one
    // being shown either. Replay refuses it, and every one of these three
    // routes used to spell it back out of the POV string it was handed --
    // which is exactly where the refusal gets undone, because a flag or a
    // topbar is rendered FROM this field (Codex round 29).
    let country_tag = (!frame.country_key_is_contended(pov_tag)).then(|| pov_tag.to_string());
    TopbarFlagIdentity {
        country_tag,
        country_ideology,
        flag_texture: None,
    }
}

pub fn topbar_binding_from_frame(frame: &WorldFrame, pov_tag: NamedCountry<'_>) -> TopbarBinding {
    let mut binding = TopbarBinding::default();
    // A key two ids reach names no country, so it is not the name of the one
    // being shown either. Replay refuses it, and every one of these three
    // routes used to spell it back out of the POV string it was handed --
    // which is exactly where the refusal gets undone, because a flag or a
    // topbar is rendered FROM this field (Codex round 29).
    let spelled = |tag: &str| (!frame.country_key_is_contended(tag)).then(|| tag.to_string());

    if frame.country_identities_status() == ViewStatus::Present {
        if let Some(identity) = frame.country_identity(pov_tag.tag()) {
            apply_identity(&mut binding, pov_tag.tag(), identity);
        } else {
            binding.country_tag = spelled(pov_tag.tag());
        }
    } else {
        binding.country_tag = spelled(pov_tag.tag());
    }

    if frame.topbar_status() == ViewStatus::Present {
        if let Some(scalars) = frame.topbar_scalars(pov_tag.tag()) {
            apply_scalars(&mut binding, scalars);
            apply_exact_nuke_stockpile(&mut binding, frame, scalars);
        }
    }

    if frame.topbar_extras_status() == ViewStatus::Present {
        if let Some(ratios) = frame.topbar_ratios(pov_tag.tag()) {
            apply_ratios(&mut binding, ratios);
        }
        if let Some(alerts) = frame.topbar_alerts(pov_tag.tag()) {
            apply_alerts(&mut binding, alerts);
        }
    }

    apply_exact_decision_alerts(&mut binding, frame, pov_tag.tag());

    binding
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTopbarGap {
    pub field: &'static str,
    pub required_domain: &'static str,
    pub reason: &'static str,
}

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

/// The observer topbar fields no recorded domain can supply, each naming the
/// domain that would close it. A wrong `required_domain` is worse than none: it
/// sends the next lane to a domain that cannot produce the value.
pub const OBSERVER_TOPBAR_BASE_GAPS: &[ObserverTopbarGap] = &[
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
    // Lane UJ settled the source: both counts are read from the HUD
    // intel singleton at INTEL_STATE_RVA 0x03324490 (available +0x6c,
    // prepared +0x24 and completed +0x0c summed), which counts intel
    // REPORTS for the LOCAL VIEWER only. No d53 pair-matrix predicate
    // reproduces them, so naming d30/d53 here sent the reader to a
    // domain that cannot close it. The only producer is d79, which is
    // descheduled with proof, and its absence is a deliberate omission
    // rather than a recording that predates the carrier.
    ObserverTopbarGap {
        field: "intel_available",
        required_domain: "d79",
        reason: "the displayed available count exists only in the viewer-local HUD intel singleton, whose only producer (d79) is descheduled; no recorded byte supports a predicate, and the owning country is named only by that same absent source",
    },
    ObserverTopbarGap {
        field: "intel_prepared_or_completed",
        required_domain: "d79",
        reason: "the displayed prepared-or-completed count exists only in the viewer-local HUD intel singleton, whose only producer (d79) is descheduled; no recorded byte supports a predicate, and the owning country is named only by that same absent source",
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

pub fn topbar_binding_from_observer_frame(
    frame: &WorldFrame,
    country_tag: NamedCountry<'_>,
) -> ObserverTopbarBinding {
    let mut binding = TopbarBinding::default();
    // A key two ids reach names no country, so it is not the name of the one
    // being shown either. Replay refuses it, and every one of these three
    // routes used to spell it back out of the POV string it was handed --
    // which is exactly where the refusal gets undone, because a flag or a
    // topbar is rendered FROM this field (Codex round 29).
    let spelled = |tag: &str| (!frame.country_key_is_contended(tag)).then(|| tag.to_string());

    if frame.country_identities_status() == ViewStatus::Present {
        if let Some(identity) = frame.country_identity(country_tag.tag()) {
            apply_identity(&mut binding, country_tag.tag(), identity);
        } else {
            binding.country_tag = spelled(country_tag.tag());
        }
    } else {
        binding.country_tag = spelled(country_tag.tag());
    }

    if frame.topbar_status() == ViewStatus::Present {
        if let Some(scalars) = frame.topbar_scalars(country_tag.tag()) {
            apply_scalars(&mut binding, scalars);
            apply_exact_nuke_stockpile(&mut binding, frame, scalars);
        }
    }

    let mut gaps = OBSERVER_TOPBAR_BASE_GAPS.to_vec();

    let runtime_rows = match frame.runtime_text() {
        Ok(Leaf::Measured(catalog)) => {
            exact_observer_topbar_text(&catalog, frame.hour_index(), country_tag.tag())
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

    let prototype_total = if frame.special_project_programs_status() == ViewStatus::Present {
        match frame.special_project_programs(country_tag.tag()) {
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

pub fn world_tension_consumers_from_frame(
    frame: &WorldFrame,
    pov_tag: NamedCountry<'_>,
) -> Option<crate::world_tension::WorldTensionConsumerBinding> {
    crate::world_tension::binding_from_frame(frame, pov_tag)
}

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
    if out.last().map(String::as_str) != Some(tag_upper.as_str()) {
        out.push(tag_upper);
    }
    out
}

pub fn topbar_flag_key_candidates(tag: &str, identity: &CountryIdentity) -> Vec<String> {
    flag_candidates_for_size(
        Some(tag),
        identity.ruling_ideology.measured().map(String::as_str),
        rendered_cosmetic_override(&identity.cosmetic_tag),
        None,
        FlagSizeFolder::Medium,
    )
}

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


pub const OUTLINER_ARMY_ICON: &str = "GFX_theatre_army_shield";
pub const OUTLINER_NAVY_ICON: &str = "GFX_theatre_task_force_shield";
pub const OUTLINER_AIR_ICON: &str = "GFX_theatre_air_force_shield";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerUnitRow {
    pub uid: String,
    pub icon_sprite: String,
}

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

fn owner_is(owner: &Leaf<String>, pov_tag: &str) -> bool {
    matches!(owner, Leaf::Measured(tag) if tag == pov_tag)
}

pub fn outliner_unit_lists_from_frame(frame: &WorldFrame, pov_tag: NamedCountry<'_>) -> OutlinerUnitLists {
    // ...and the outliner is the same claim in list form: no army, no navy, no
    // air for a POV that names no country.
    if frame.country_key_is_contended(pov_tag.tag()) {
        return OutlinerUnitLists {
            pov_tag: pov_tag.to_string(),
            army: Vec::new(),
            navy: Vec::new(),
            air: Vec::new(),
            army_status: ViewStatus::DomainUnavailable,
            navy_status: ViewStatus::DomainUnavailable,
            air_status: ViewStatus::DomainUnavailable,
        };
    }
    let army = frame
        .division_units()
        .iter()
        .filter(|d| owner_is(&d.owner, pov_tag.tag()))
        .map(|d| OutlinerUnitRow {
            uid: d.div_uid.clone(),
            icon_sprite: OUTLINER_ARMY_ICON.to_string(),
        })
        .collect();
    let navy = frame
        .fleets()
        .iter()
        .filter(|f| owner_is(&f.owner, pov_tag.tag()))
        .map(|f| OutlinerUnitRow {
            uid: f.task_force_uid.clone(),
            icon_sprite: OUTLINER_NAVY_ICON.to_string(),
        })
        .collect();
    let air = frame
        .wings()
        .iter()
        .filter(|w| owner_is(&w.owner, pov_tag.tag()))
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

pub fn outliner_binding_from_frame(
    frame: &WorldFrame,
    pov_tag: NamedCountry<'_>,
) -> OutlinerBinding {
    outliner_binding_from_lists(&outliner_unit_lists_from_frame(frame, pov_tag))
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionRow {
    pub faction_name: String,
    pub leader: Option<String>,
    pub members: Vec<String>,
}

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiplomacyPanelBinding {
    pub pov_tag: String,
    pub available: bool,
    pub factions_available: bool,
    pub faction_roster_status: FactionRosterStatus,
    pub factions: Vec<FactionRow>,
    pub relations: Vec<RelationRow>,
    pub wars: Vec<String>,
    pub subject_autonomy_available: bool,
    pub subject_autonomy: Vec<SubjectAutonomyPanelRow>,
}

fn bool_opt(leaf: &Leaf<bool>) -> Option<bool> {
    match leaf {
        Leaf::Measured(v) => Some(*v),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

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

fn relation_rows(view: &DiplomacyView, pov_tag: &str) -> Vec<RelationRow> {
    let mut rows = Vec::new();
    for other in view.countries.keys() {
        if other == pov_tag {
            continue;
        }
        let pair = view.pair(pov_tag, other);
        let pair = pair.as_ref();
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
    // THE CENSUS FIRST. A key two ids reach names no country, however it is
    // spelled, and the three checks below cannot see that: the typed maps
    // refuse a contended key by design, so all three go false and the
    // numeric branch parses the refusal itself into an identity.
    if frame.country_key_is_contended(pov_tag) {
        return None;
    }
    let recorded_tag = view.countries.contains_key(pov_tag)
        || frame.politics_detail(pov_tag).is_some()
        || frame.countries().contains_key(pov_tag);
    if recorded_tag {
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

pub fn diplomacy_panel_from_frame(frame: &WorldFrame, pov_tag: NamedCountry<'_>) -> DiplomacyPanelBinding {
    let view = frame.diplomacy();
    // A POV IS A COUNTRY, and the selector arrives from the CALLER rather than
    // from the recording -- the one entry way the taxonomy did not name (Codex
    // round 48). A key two ids reach is neither country, so this panel is not
    // available: `available: true` with an empty relation list is a claim that
    // the selected country has no relations.
    if frame.country_key_is_contended(pov_tag.tag()) || view.status != ViewStatus::Present {
        return DiplomacyPanelBinding {
            pov_tag: pov_tag.to_string(),
            available: false,
            ..DiplomacyPanelBinding::default()
        };
    }
    let relations = relation_rows(view, pov_tag.tag());
    let wars = relations
        .iter()
        .filter(|r| r.at_war == Some(true))
        .map(|r| r.other_tag.clone())
        .collect();
    let (faction_roster_status, factions) = faction_rows(view);
    let (subject_autonomy_available, subject_autonomy) = subject_autonomy_rows(
        view,
        pov_tag.tag(),
        resolve_pov_numeric_identity(frame, view, pov_tag.tag()),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockDate {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
}

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

fn format_clock_date(date: ClockDate, month_name: &str) -> String {
    format!(
        "{}:00 {}, {} {}",
        date.hour, date.day, month_name, date.year
    )
}

fn format_topbar_date_template(date: &str) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    let key = TOPBAR_CLOCK_MONTH_LOC_KEYS[(parsed.month - 1) as usize];
    Some(format_clock_date(parsed, &format!("${key}$")))
}

pub fn format_topbar_date(date: &str, loc: &Localisation) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    let key = TOPBAR_CLOCK_MONTH_LOC_KEYS[(parsed.month - 1) as usize];
    loc.source_file_for_key(key)?;
    let month = loc.get(key)?;
    Some(format_clock_date(parsed, month))
}

pub fn format_topbar_date_english(date: &str) -> Option<String> {
    let parsed = parse_clock_date(date)?;
    Some(format_clock_date(
        parsed,
        month_name_english(parsed.month as u8),
    ))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockBinding {
    pub date_time: Option<String>,
    pub speed: Option<u8>,
}

pub fn playback_speed_button(speed_step: usize) -> u8 {
    (speed_step + 1).min(5) as u8
}

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

pub fn apply_clock(binding: &mut TopbarBinding, clock: &ClockBinding) {
    binding.date_time = clock.date_time.clone();
    binding.speed = clock.speed;
}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateTooltipBinding {
    pub state_id: u32,
    pub owner: Option<String>,
    pub controller: Option<String>,
    pub available: bool,
}

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
    /// A TAG this frame's census admits. A test that means to name a country says
    /// so here; a test about a contended key asks `named_country` and gets `None`.
    fn named<'tag>(
        frame: &replay_core::WorldFrame,
        tag: &'tag str,
    ) -> replay_core::NamedCountry<'tag> {
        frame.named_country(tag).expect("census-admitted tag")
    }

    use super::*;

    /// kills: re-pointing the two intel alert gaps at d53. Lane UJ proved the
    /// counts come from the viewer-local HUD intel singleton and that no d53
    /// pair-matrix predicate reproduces them, so a gap naming d30/d53 sends the
    /// next reader to a domain that cannot close it. d79 owns them and is
    /// descheduled with proof: the absence is a decision, not a recording that
    /// predates the carrier.
    #[test]
    fn the_intel_alert_gaps_name_d79_as_their_only_producer() {
        for field in ["intel_available", "intel_prepared_or_completed"] {
            let gap = OBSERVER_TOPBAR_BASE_GAPS
                .iter()
                .find(|gap| gap.field == field)
                .unwrap_or_else(|| panic!("{field} must declare a gap"));
            assert_eq!(
                gap.required_domain, "d79",
                "{field} must name the domain that actually produces it"
            );
            assert!(
                gap.reason.contains("descheduled"),
                "{field} must say the source was descheduled, not merely underivable: {}",
                gap.reason
            );
            assert!(
                !gap.reason.contains("d53") && !gap.reason.contains("d30"),
                "{field} must not send the reader back to the refuted d53 route: {}",
                gap.reason
            );
        }
    }

    /// kills: re-pointing the two market contract gaps at d79 by analogy with
    /// the intel pair above. `required_domain` names the domain that would have
    /// to carry the value, not one that carries it today —
    /// `decision_available_items` names d14 on those same terms. No domain
    /// carries the engine's evaluated counts; d43 `market_trade_live` carries
    /// the contracts they are counted over, while the intel counts have no
    /// world-truth carrier at all, which is why only that pair moved to d79.
    #[test]
    fn the_market_contract_gaps_name_d43_not_the_d79_trailer_that_fills_them() {
        for field in ["non_delivering_contracts", "ongoing_contracts"] {
            let gap = OBSERVER_TOPBAR_BASE_GAPS
                .iter()
                .find(|gap| gap.field == field)
                .unwrap_or_else(|| panic!("{field} must declare a gap"));
            assert_eq!(
                gap.required_domain, "d43",
                "{field} must name the domain that carries the contracts"
            );
            assert!(
                gap.reason.contains("contract alert count"),
                "{field} must say what trade relations do not carry: {}",
                gap.reason
            );
        }
    }

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
            cosmetic_tag: Leaf::Measured(cosmetic.map(str::to_string)),
            ruling_ideology: match ideology {
                Some(i) => Leaf::Measured(i.to_string()),
                None => Leaf::Unobtained,
            },
            cores: Leaf::Unobtained,
            claims: Leaf::Unobtained,
            cores_unmeasured_states: Leaf::Unobtained,
            claims_unmeasured_states: Leaf::Unobtained,
            preferred_tactic: Leaf::Unobtained,
        }
    }

    #[test]
    fn admitting_flag_consumers_separate_an_unread_cosmetic_from_a_measured_none() {
        for leaf in [
            Leaf::Measured(None),
            Leaf::Absent,
            Leaf::Unobtained,
        ] {
            assert_eq!(rendered_cosmetic_override(&leaf), None);
        }
        assert_eq!(
            rendered_cosmetic_override(&Leaf::Measured(Some("RAJ_UK".to_string()))),
            Some("RAJ_UK")
        );

        assert_eq!(
            admitted_flag_cosmetic(Some(&identity(Some("neutrality"), None))),
            Ok(None),
            "a measured 'no cosmetic override' keys the base TAG"
        );
        let unread = CountryIdentity {
            cosmetic_tag: Leaf::Absent,
            ..identity(Some("neutrality"), None)
        };
        assert!(
            admitted_flag_cosmetic(Some(&unread)).is_err(),
            "an unread cosmetic field must refuse, not resolve down the base-TAG ladder"
        );
        assert_eq!(admitted_flag_cosmetic(None), Ok(None));
    }

    #[test]
    fn name_key_precedence_cosmetic_ideology_tag() {
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
        assert_eq!(
            country_name_key_candidates("GER", Some("fascism"), None),
            vec!["GER_fascism".to_string(), "GER".to_string()]
        );
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
        assert_eq!(
            format_topbar_date_english("1939.9.29.7").as_deref(),
            Some("7:00 29, September 1939")
        );
        assert_eq!(
            format_topbar_date_english("1936.1.1.0").as_deref(),
            Some("0:00 1, January 1936")
        );
    }

    #[test]
    fn topbar_date_fail_closed_on_bad_input() {
        assert_eq!(format_topbar_date_english(""), None);
        assert_eq!(format_topbar_date_english("1944.12.25"), None);
        assert_eq!(parse_clock_date("1944.12.25"), None);
        assert_eq!(format_topbar_date_english("hour:5"), None);
        assert_eq!(format_topbar_date_english("1936.13.1.0"), None);
        assert_eq!(format_topbar_date_english("1936.1.0.0"), None);
        assert_eq!(format_topbar_date_english("1936.1.1.24"), None);
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
        assert_eq!(playback_speed_button(7), 5);
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

    #[test]
    fn stability_and_war_support_project_percent_like_world_tension() {
        let mut scalars = all_absent_scalars();
        scalars.stability = Leaf::Measured(0.74);
        scalars.war_support = Leaf::Measured(0.305);
        scalars.world_tension = Leaf::Measured(0.74);
        let mut binding = TopbarBinding::default();
        apply_scalars(&mut binding, &scalars);
        assert_eq!(binding.stability_percent.as_deref(), Some("74%"));
        assert_eq!(binding.war_support_percent.as_deref(), Some("31%"));
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


    fn autonomy_view(
        rows: Vec<replay_core::SubjectAutonomyCurrentRow>,
    ) -> DiplomacyView {
        let mut view = DiplomacyView::stating_nothing_for_consumer_tests();
        view.status = ViewStatus::Present;
        view.subject_autonomy_current = Leaf::Measured(rows);
        view
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
            let mut rest = self
                .frames
                .get(&hour_index)
                .and_then(serde_json::Value::as_object)
                .cloned()
                .ok_or_else(|| {
                    replay_core::SourceError::Open(format!("no frame at {hour_index}"))
                })?;
            if replay_core::COUNTRY_COMPOSED_MEMBERS
                .iter()
                .any(|member| rest.contains_key(*member))
            {
                rest.entry("tag_db_id_index")
                    .or_insert_with(|| serde_json::json!({"country_composition_version": 2}));
            }
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

    /// A CONTENDED key is not an identity, however it is spelled.
    ///
    /// The numeric branch below exists for a POV key the frame does not carry
    /// as a country: the spelling IS the id. A contended key is absent from
    /// every typed map ON PURPOSE -- that is what the census does with a key
    /// two ids reach -- so all three "is it recorded" checks go false and the
    /// spelling gets parsed. The census's own refusal became an identity.
    ///
    /// kills: asking the typed maps before asking the census.
    /// The three topbar routes do not spell a refused key back out.
    ///
    /// Each was handed a POV string and wrote `Some(pov_tag)` into
    /// `country_tag` whenever Replay had no identity for it -- which is exactly
    /// the state the census puts a contended key in. A flag and a topbar are
    /// rendered FROM that field, so the refusal was undone at the last step
    /// (Codex round 29).
    /// The one selector path a TYPE cannot carry: a country selection stored in
    /// app state.
    ///
    /// Every consumer that takes a caller-supplied selector now takes
    /// `NamedCountry`, so the census is asked by the compiler at the call site
    /// and there is nothing left for a scan to detect. A selection kept in
    /// viewer state is an owned `String` on a struct the app carries between
    /// frames -- no parameter, no type, nothing the compiler can require -- and
    /// `UiRuntime::execute` published `viewer.selected_country` into an XUI11
    /// candidate that the closed-panel branch then announced as available
    /// (Codex round 50). So the one remaining rule is about the READ: a
    /// function that holds a `&WorldFrame` and reads a stored selection asks
    /// this hour's census about it. Published or not -- the question is not
    /// whether the read is published, which is the judgement three rounds of
    /// scans could not make; it is whether the census was asked.
    #[test]
    fn every_stored_country_selection_is_asked_about() {
        /// The fields an app keeps a country selection in, read off `viewer`.
        const STATE_SELECTOR_FIELDS: &[&str] = &[
            "selected_country",
            "pov_tag",
            "viewer_country",
            "selected_country_tag",
        ];

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sources: Vec<(String, String)> = Vec::new();
        for crate_dir in ["src", "../map_viewer/src", "../replay_app/src"] {
            let root = manifest.join(crate_dir);
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let listing = std::fs::read_dir(&dir).expect("a source directory");
                for entry in listing.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string();
                        sources.push((name, std::fs::read_to_string(&path).expect("source")));
                    }
                }
            }
        }
        assert!(sources.len() > 40, "this scan read {} files", sources.len());

        // The RECEIVER is not part of the rule: `state.selected_country` and
        // `self.viewer.pov_tag` are the same read as `viewer.selected_country`,
        // and a rule that spelled the receiver could not see them (Codex round
        // 51). What IS part of the rule is where the field name ENDS, so
        // `.selected_country_filter_ids` is a different field and not a read.
        // The body is flattened first, because these arrive broken across
        // lines as `viewer` / `.selected_country`.
        let stored_selection_reads = |body: &str| -> usize {
            let flattened: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let flattened = flattened.replace(" . ", ".").replace(". ", ".").replace(" .", ".");
            STATE_SELECTOR_FIELDS
                .iter()
                .map(|field| {
                    let needle = format!(".{field}");
                    flattened
                        .match_indices(&needle)
                        .filter(|(at, _)| {
                            flattened[at + needle.len()..]
                                .chars()
                                .next()
                                .is_none_or(|next| !next.is_alphanumeric() && next != '_')
                        })
                        .count()
                })
                .sum()
        };

        let mut readers = 0usize;
        let mut reads = 0usize;
        let mut unasked: Vec<String> = Vec::new();
        for (file, source) in &sources {
            let lines: Vec<&str> = source.lines().collect();
            let production = lines
                .iter()
                .position(|line| line.trim_start().starts_with("mod tests"))
                .unwrap_or(lines.len());
            let mut at = 0usize;
            while at < production {
                let trimmed = lines[at].trim_start();
                // Visibility and `async` are PREFIXES, not spellings to
                // enumerate: an `async fn` declaration was invisible to the
                // three this rule knew (Codex round 51).
                // `pub`, `pub(crate)`, `pub(super)`, `pub(in path)` -- the
                // visibility is `pub` and OPTIONALLY a parenthesised scope, not
                // a list of the two spellings this rule happened to know
                // (Codex round 53 walked in through `pub(super)`).
                let head_of = match trimmed.strip_prefix("pub") {
                    Some(rest) => match rest.strip_prefix('(') {
                        Some(scoped) => match scoped.split_once(") ") {
                            Some((_, after)) => after,
                            None => trimmed,
                        },
                        None => rest.strip_prefix(' ').unwrap_or(trimmed),
                    },
                    None => trimmed,
                };
                let head_of = head_of.strip_prefix("async ").unwrap_or(head_of);
                let declared = head_of.strip_prefix("fn ");
                let Some(rest) = declared else {
                    at += 1;
                    continue;
                };
                // METHODS TOO: the body ends at the closing brace on the
                // declaration's own indent (Codex round 49).
                let indent = lines[at].len() - lines[at].trim_start().len();
                let closing = format!("{}}}", " ".repeat(indent));
                let name = rest.split(['(', '<', ' ']).next().unwrap_or("").to_string();
                let end = (at + 1..lines.len())
                    .find(|index| lines[*index] == closing)
                    .unwrap_or(lines.len() - 1);
                let body = lines[at..=end].join("\n");
                let head = body.split('{').next().unwrap_or_default().to_string();
                let body_reads = if head.contains("WorldFrame") {
                    stored_selection_reads(&body)
                } else {
                    0
                };
                if body_reads > 0 {
                    readers += 1;
                    reads += body_reads;
                    if !body.contains("named_country") && !body.contains("country_key_is_contended")
                    {
                        unasked.push(format!("{file}: {name}"));
                    }
                }
                at = end + 1;
            }
        }

        assert!(
            unasked.is_empty(),
            "these functions hold a frame and read a stored country selection \
             without asking that frame's census about it. Call \
             `frame.named_country(..)` and carry the answer in the type:\n{}",
            unasked.join("\n")
        );
        assert_eq!(
            readers, 9,
            "the number of functions reading a stored country selection moved; \
             every one of them asks the census, and a new one has to as well"
        );
        // ...and the READS, not just the functions. The rule above is per
        // FUNCTION, so a function that asks about one stored selection can
        // publish a SECOND one unasked and stay green -- Codex round 52
        // demonstrated that by putting `viewer.selected_country.clone()` back
        // into the XUI15 trade surface's `country_tag` inside
        // `prepare_observer_semantic_frame_with_xui12_and_xui22_producers`,
        // which asks elsewhere. A per-READ rule is not reachable: 42 of these
        // 57 reads are neither a comparison nor a statement-local ask -- a
        // tooltip POV, a viewer-state reconcile, a `?`-terminated bind -- and
        // every rule that admits them is the spelling list rounds 48-50 walked
        // past. So the COUNT is the instrument, as it is for every other entry
        // way this lane closed: a new read has to be justified by moving it.
        assert_eq!(
            reads, 57,
            "the number of stored-country-selection READS moved. A new one is              either an ask, a comparison, or a publication -- say which, and              move this number"
        );
    }

    /// A `NamedCountry` cannot be built for a key two ids reach.
    ///
    /// This is the whole of the caller-supplied-selector entry way now: the
    /// constructor is the gate, and `diplomacy_panel_from_frame` and the 32
    /// other converted signatures cannot be CALLED with a contended key (Codex
    /// rounds 48-50 each found a new way to spell publication past a scan;
    /// none of them can be spelled past a type).
    #[test]
    fn a_contended_key_has_no_named_country() {
        let contended = frame_from_json(serde_json::json!({
            "countries": { "ENG": { "_r2_tag_db_id": 12 } },
            "tag_db_id_index": {
                "country_composition_version": 2,
                "withheld_countries": {
                    "contended_keys": { "DUP": { "tag_db_ids": [441, 442] } }
                },
            },
        }));
        let frame = contended.frame(0).expect("frame");
        assert!(
            frame.country_key_is_contended("DUP"),
            "the census must be carried"
        );
        assert!(
            frame.named_country("DUP").is_none(),
            "a key two ids reach names no country, so no consumer can be handed one"
        );
        // THE CONTROL: an uncontended key still names a country, or the
        // assertion above would pass on a constructor that refuses everything.
        assert_eq!(
            frame.named_country("ENG").map(|country| country.tag()),
            Some("ENG"),
            "an uncontended key is still a country, or this proves nothing"
        );
    }

    /// A `NamedCountry` proves the census was asked, not WHICH FRAME answered.
    ///
    /// The type is a borrowed tag with a promise attached; it does not borrow the
    /// frame that made the promise, and a viewer carries a selection from one hour
    /// into the next. So the panels keep asking their own frame, and this is the
    /// test those guards die without: a name another hour admits, handed to an
    /// hour whose census refuses it.
    #[test]
    fn a_panel_re_asks_its_own_frame_about_a_name_another_frame_admitted() {
        let world = |index: serde_json::Value| {
            frame_from_json(serde_json::json!({
                "countries": { "ENG": { "_r2_tag_db_id": 12 } },
                "diplomacy_world": { "relations": {} },
                "tag_db_id_index": index,
            }))
        };
        let contended = world(serde_json::json!({
            "country_composition_version": 2,
            "withheld_countries": {
                "contended_keys": { "DUP": { "tag_db_ids": [441, 442] } }
            },
        }));
        let frame = contended.frame(0).expect("frame");
        assert!(
            frame.country_key_is_contended("DUP"),
            "the census must be carried"
        );
        assert!(
            frame.named_country("DUP").is_none(),
            "this frame cannot name it, so no caller of its own can spell it here"
        );

        // The other hour, where the same key IS a country and hands out the
        // name the panels below are handed.
        let elsewhere = world(serde_json::json!({ "country_composition_version": 2 }));
        let elsewhere = elsewhere.frame(0).expect("frame");
        let dup = elsewhere
            .named_country("DUP")
            .expect("the other hour admits it");

        let panel = diplomacy_panel_from_frame(&frame, dup);
        assert!(
            !panel.available,
            "a panel whose POV names neither claimant is not available: an empty \
             relation list under that name says the country has no relations"
        );
        let overview = crate::waroverview_binding::war_overview_from_frame(&frame, dup);
        assert!(
            !overview.available,
            "and neither is a war overview for nobody"
        );
        let outliner = outliner_unit_lists_from_frame(&frame, dup);
        assert_eq!(
            (
                outliner.army.len(),
                outliner.navy.len(),
                outliner.air.len(),
                outliner.army_status,
            ),
            (0, 0, 0, replay_core::ViewStatus::DomainUnavailable),
            "an outliner for a contended POV states that it could not be built, \
             rather than showing that country as having no units"
        );

        // THE CONTROL, and it is the cost rule: an uncontended POV the frame
        // carries is still served.
        let plain = world(serde_json::json!({ "country_composition_version": 2 }));
        let frame = plain.frame(0).expect("frame");
        assert!(
            diplomacy_panel_from_frame(&frame, named(&frame, "ENG")).available
                && crate::waroverview_binding::war_overview_from_frame(&frame, named(&frame, "ENG")).available,
            "an uncontended POV still gets its panels, or this proves nothing"
        );
        assert_eq!(
            outliner_unit_lists_from_frame(&frame, named(&frame, "ENG")).army_status,
            frame.division_units_status(),
            "and its outliner reports the frame's own status"
        );
    }

    #[test]
    fn no_topbar_route_spells_a_contended_key_back_out() {
        let contended = frame_from_json(serde_json::json!({
            "countries": {},
            "tag_db_id_index": {
                "country_composition_version": 2,
                "withheld_countries": {
                    "contended_keys": { "DUP": { "tag_db_ids": [441, 442] } }
                },
            },
        }));
        let frame = contended.frame(0).expect("frame");
        assert!(frame.country_key_is_contended("DUP"), "the census must be carried");
        // The name comes from an hour that CAN name it, so what is under test
        // is each route's own question rather than the constructor's.
        let elsewhere = frame_from_json(serde_json::json!({
            "countries": {},
            "tag_db_id_index": { "country_composition_version": 2 },
        }));
        let elsewhere = elsewhere.frame(0).expect("frame");
        let dup = elsewhere
            .named_country("DUP")
            .expect("the other hour admits it");
        assert_eq!(
            [
                topbar_flag_identity_from_frame(&frame, "DUP").country_tag,
                topbar_binding_from_frame(&frame, dup).country_tag,
                topbar_binding_from_observer_frame(&frame, dup).binding.country_tag,
            ],
            [None, None, None],
            "no route may name a key two ids reach"
        );

        // The control: an ordinary POV the frame also has no identity for is
        // still spelled, so the assertion above is about contention and not
        // about missing identities.
        let plain = frame_from_json(serde_json::json!({
            "countries": {},
            "tag_db_id_index": { "country_composition_version": 2 },
        }));
        let frame = plain.frame(0).expect("frame");
        assert_eq!(
            [
                topbar_flag_identity_from_frame(&frame, "GER").country_tag,
                topbar_binding_from_frame(&frame, named(&frame, "GER")).country_tag,
                topbar_binding_from_observer_frame(&frame, named(&frame, "GER")).binding.country_tag,
            ],
            [
                Some("GER".to_string()),
                Some("GER".to_string()),
                Some("GER".to_string())
            ],
            "an uncontended POV is still named, or this proves nothing"
        );
    }

    #[test]
    fn a_contended_numeric_pov_key_is_not_parsed_into_an_identity() {
        // The control: the same numeric key, uncontended and unrecorded, still
        // resolves by spelling. Without it this test would pass on a frame
        // where the numeric branch simply never fires.
        let timeline = frame_from_json(serde_json::json!({
            "countries": {},
            "tag_db_id_index": { "country_composition_version": 2 },
        }));
        let frame = timeline.frame(0).expect("frame");
        assert_eq!(
            resolve_pov_numeric_identity(&frame, frame.diplomacy(), "123"),
            Some(123),
            "an unrecorded numeric key is still its own id, or this proves nothing"
        );

        let timeline = frame_from_json(serde_json::json!({
            "countries": {},
            "tag_db_id_index": {
                "country_composition_version": 2,
                "withheld_countries": {
                    "contended_keys": { "123": { "tag_db_ids": [441, 442] } }
                },
            },
        }));
        let frame = timeline.frame(0).expect("frame");
        assert!(
            frame.country_key_is_contended("123"),
            "the frame must carry the census, or the assertion below is vacuous"
        );
        assert_eq!(
            resolve_pov_numeric_identity(&frame, frame.diplomacy(), "123"),
            None,
            "a key the census says two ids reach names no country to be the POV"
        );
    }

    #[test]
    fn pov_numeric_identity_resolves_through_d18_map_before_spelling() {
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
        assert_eq!(resolve_pov_numeric_identity(&frame, view, "789"), Some(789));

        let timeline = frame_from_json(serde_json::json!({
            "countries": { "123": {} }
        }));
        let frame = timeline.frame(0).expect("frame");
        let view = frame.diplomacy();
        assert_eq!(resolve_pov_numeric_identity(&frame, view, "123"), None);
    }
}
