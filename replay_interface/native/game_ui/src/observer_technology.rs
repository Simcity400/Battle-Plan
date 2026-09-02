
use replay_core::NamedCountry;
use replay_core::{Leaf, ViewStatus, WorldFrame};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverTechnologyState {
    Completed,
    InProgress,
    Unresearched,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverTechnologyGap {
    D18RosterUnavailable(String),
    CountryNotInD18Roster(String),
    InvalidAuthoredTechnologyOrder,
    ResearchDomainUnavailable(ViewStatus),
    CountryResearchUnavailable(String),
    CompletedTechnologyVectorUnavailable,
    CompletedTechnologyCountMismatch,
    InvalidCompletedTechnologyIdentity,
    ActiveResearchUnavailable,
    InvalidActiveTechnologyIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTechnologyColoring {
    pub country: String,
    pub states: BTreeMap<String, ObserverTechnologyState>,
    pub gap: Option<ObserverTechnologyGap>,
}

impl ObserverTechnologyColoring {
    fn unavailable(country: &str, authored_order: &[String], gap: ObserverTechnologyGap) -> Self {
        Self {
            country: country.to_string(),
            states: authored_order
                .iter()
                .cloned()
                .map(|id| (id, ObserverTechnologyState::Unavailable))
                .collect(),
            gap: Some(gap),
        }
    }
}

pub fn observer_technology_coloring(
    frame: &WorldFrame,
    country: NamedCountry<'_>,
    authored_order: &[String],
) -> ObserverTechnologyColoring {
    if authored_order.is_empty()
        || authored_order.iter().any(|id| id.trim().is_empty())
        || authored_order.iter().collect::<BTreeSet<_>>().len() != authored_order.len()
    {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::InvalidAuthoredTechnologyOrder,
        );
    }

    // PARTIAL: the only question asked of the roster here is whether the
    // SELECTED country is in it, which is a presence claim about one country.
    // Whether some other country could be named this hour cannot change the
    // answer, and refusing this country's technology colouring over it would be
    // the all-or-nothing this split removes.
    let d18 = frame.d18_country_roster();
    let partial = match d18.partial() {
        Ok(partial) => partial,
        Err(refusal) => {
            return ObserverTechnologyColoring::unavailable(
                country.tag(),
                authored_order,
                ObserverTechnologyGap::D18RosterUnavailable(refusal.detail),
            )
        }
    };
    if !partial.resolved.iter().any(|entry| entry.tag == country.tag()) {
        // NAMED, and deliberately NOT by trying to match the selection against
        // the excluded list. `excluded` holds raw emitted keys -- for a country
        // this hour could not name, the numeric tag_db_id -- while `country` is
        // a TAG, so the two can never be equal and a gap that only spoke when
        // they matched would never speak. What is true is that the selection is
        // not among the countries this hour could name, and that the recording
        // carries N keys it could not name at all; both go in the gap.
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            if partial.excluded.is_empty() {
                ObserverTechnologyGap::CountryNotInD18Roster(country.to_string())
            } else {
                ObserverTechnologyGap::D18RosterUnavailable(format!(
                    "country {country} is not among the countries this hour's D18 read could \
                     name; {} emitted country key(s) carry no D18 identity this hour ({})",
                    partial.excluded.len(),
                    partial.excluded.join(",")
                ))
            },
        );
    }
    if frame.research_status() != ViewStatus::Present {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::ResearchDomainUnavailable(frame.research_status()),
        );
    }
    let Some(research) = frame.research(country.tag()) else {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::CountryResearchUnavailable(country.to_string()),
        );
    };
    let Leaf::Measured(completed_rows) = &research.technologies else {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::CompletedTechnologyVectorUnavailable,
        );
    };
    let Leaf::Measured(completed_count) = &research.tech_count else {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::CompletedTechnologyCountMismatch,
        );
    };
    if *completed_count < 0 || *completed_count as usize != completed_rows.len() {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::CompletedTechnologyCountMismatch,
        );
    }

    let authored = authored_order
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut completed = BTreeSet::new();
    let mut unknown_level = BTreeSet::new();
    for row in completed_rows {
        let Leaf::Measured(id) = &row.name else {
            return ObserverTechnologyColoring::unavailable(
                country.tag(),
                authored_order,
                ObserverTechnologyGap::InvalidCompletedTechnologyIdentity,
            );
        };
        if id.trim().is_empty() || !authored.contains(id.as_str()) || !completed.insert(id.as_str())
        {
            return ObserverTechnologyColoring::unavailable(
                country.tag(),
                authored_order,
                ObserverTechnologyGap::InvalidCompletedTechnologyIdentity,
            );
        }
        if !row.level.is_measured() {
            unknown_level.insert(id.as_str());
        }
    }

    let Leaf::Measured(active_rows) = &research.in_progress_live else {
        return ObserverTechnologyColoring::unavailable(
            country.tag(),
            authored_order,
            ObserverTechnologyGap::ActiveResearchUnavailable,
        );
    };
    let mut active = BTreeSet::new();
    for row in active_rows {
        let Leaf::Measured(id) = &row.tech_name else {
            return ObserverTechnologyColoring::unavailable(
                country.tag(),
                authored_order,
                ObserverTechnologyGap::InvalidActiveTechnologyIdentity,
            );
        };
        if id.trim().is_empty()
            || !authored.contains(id.as_str())
            || !active.insert(id.as_str())
            || completed.contains(id.as_str())
        {
            return ObserverTechnologyColoring::unavailable(
                country.tag(),
                authored_order,
                ObserverTechnologyGap::InvalidActiveTechnologyIdentity,
            );
        }
    }

    let states = authored_order
        .iter()
        .map(|id| {
            let state = if active.contains(id.as_str()) {
                ObserverTechnologyState::InProgress
            } else if unknown_level.contains(id.as_str()) {
                ObserverTechnologyState::Unavailable
            } else if completed.contains(id.as_str()) {
                ObserverTechnologyState::Completed
            } else {
                ObserverTechnologyState::Unresearched
            };
            (id.clone(), state)
        })
        .collect();
    ObserverTechnologyColoring {
        country: country.to_string(),
        states,
        gap: None,
    }
}
