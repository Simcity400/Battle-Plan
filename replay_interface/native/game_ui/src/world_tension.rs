use replay_core::{ViewStatus, WorldFrame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatValueColour {
    Green,
    Yellow,
    Red,
}

pub fn threat_value_colour(raw: i64) -> ThreatValueColour {
    if raw <= 200_000 {
        ThreatValueColour::Green
    } else if raw <= 1_000_000 {
        ThreatValueColour::Yellow
    } else {
        ThreatValueColour::Red
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreatValueBinding {
    pub raw_fixed: i64,
    pub colour: ThreatValueColour,
}

impl ThreatValueBinding {
    fn new(raw_fixed: i64) -> Self {
        Self { raw_fixed, colour: threat_value_colour(raw_fixed) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionPolicyBinding {
    pub localisation_key: &'static str,
    pub define_id: u32,
    pub threshold_fixed5: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionTooltipBinding {
    pub generated_by_us: Option<ThreatValueBinding>,
    pub largest_source_country: Option<String>,
    pub largest_source: Option<ThreatValueBinding>,
    pub policies: Vec<WorldTensionPolicyBinding>,
    pub monthly_decrease_fixed5: i64,
    pub time_scale_fixed5: i64,
    pub start_date_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionPopupRowBinding {
    pub source_country: String,
    pub source_tag_db_id: u32,
    pub date_raw: i32,
    pub current: ThreatValueBinding,
    pub original: ThreatValueBinding,
    pub reason: String,
    pub source_sequence: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldTensionPopupBinding {
    pub rows: Vec<WorldTensionPopupRowBinding>,
    pub wars: crate::waroverview_binding::WarOverviewBinding,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldTensionConsumerBinding {
    pub current_fixed5: i64,
    pub prior_fixed5: i64,
    pub increased: bool,
    pub tooltip: WorldTensionTooltipBinding,
    pub popup: WorldTensionPopupBinding,
}

pub fn binding_from_frame(frame: &WorldFrame, pov_tag: &str) -> Option<WorldTensionConsumerBinding> {
    let view = frame.world_tension_presentation();
    if view.status != ViewStatus::Present {
        return None;
    }
    let country = view.countries.get(pov_tag)?;
    let current_fixed5 = view.current_fixed5?;
    let prior_fixed5 = view.prior_fixed5?;
    let increased = view.increased?;
    let generated_by_us = (country.generated_threat_fixed > 0)
        .then(|| ThreatValueBinding::new(country.generated_threat_fixed));
    let mut largest = None;
    for ledger_row in &view.ledger {
        let candidate = view.countries.get(&ledger_row.source_country_tag)?;
        if candidate.generated_threat_fixed > 0
            && largest.map_or(true, |current: &replay_core::world::WorldTensionCountryPresentation| {
                candidate.generated_threat_fixed > current.generated_threat_fixed
            })
        {
            largest = Some(candidate);
        }
    }
    let policies = [
        ("GENERATE_WARGOAL_TENSION_DESC", 0xE9, country.generate_wargoal_threshold_fixed5),
        ("JOIN_FACTION_TENSION_DESC", 0xEB, country.join_faction_threshold_fixed5),
        ("LEND_LEASE_TENSION_DESC", 0xEC, country.lend_lease_threshold_fixed5),
        ("SEND_VOLUNTEERS_TENSION_DESC", 0xEF, country.send_volunteers_threshold_fixed5),
        ("GUARANTEE_TENSION_DESC", 0xF0, country.guarantee_threshold_fixed5),
    ]
    .into_iter()
    .filter(|(_, _, value)| *value > 0)
    .map(|(localisation_key, define_id, threshold_fixed5)| WorldTensionPolicyBinding {
        localisation_key,
        define_id,
        threshold_fixed5,
    })
    .collect();
    let popup = WorldTensionPopupBinding {
        rows: view.ledger.iter().map(|row| WorldTensionPopupRowBinding {
            source_country: row.source_country_tag.clone(),
            source_tag_db_id: row.source_tag_db_id,
            date_raw: row.date_raw,
            current: ThreatValueBinding::new(row.current_threat_fixed),
            original: ThreatValueBinding::new(row.original_threat_fixed),
            reason: row.reason.clone(),
            source_sequence: row.source_sequence,
        }).collect(),
        wars: crate::waroverview_binding::war_overview_from_frame(frame, pov_tag),
    };
    Some(WorldTensionConsumerBinding {
        current_fixed5,
        prior_fixed5,
        increased,
        tooltip: WorldTensionTooltipBinding {
            generated_by_us,
            largest_source_country: largest.map(|row| row.country_tag.clone()),
            largest_source: largest.map(|row| ThreatValueBinding::new(row.generated_threat_fixed)),
            policies,
            monthly_decrease_fixed5: view.monthly_decrease_fixed5?,
            time_scale_fixed5: view.time_scale_fixed5?,
            start_date_raw: view.start_date_raw?,
        },
        popup,
    })
}
