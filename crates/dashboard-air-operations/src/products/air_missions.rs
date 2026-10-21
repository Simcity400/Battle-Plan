//! Air mission history: `TAG@slot|<regionId>|m<bit>` holds planes and
//! `TAG@slot|<regionId>|wings` holds unique registered wing handles. Mission
//! columns overlap; the unique-wing count never sums those columns. Negative
//! and wrapping plane totals remain exact, even when the map hides an entry.
//! Wings detail uses a reached-hour shared frame-cache read, not a branch walk.

use crate::shared::{self, Choice, TheatreChoice};
use dashboard_base::{
    catalogue::{self, Refusals},
    prepared::{prepass::Family, query::progressive},
    products::{
        Product, ReplyWarnings,
        artwork::sprites::{self, SpriteFrame},
    },
};
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_derive::{
    air_mission_counts::{self, MISSION_ENTRY_TEMPLATE, MISSIONS},
    air_mission_sources::{AirRegistrySources, MissionCountrySources, canonical, effective_raw},
    aircraft_count::{SortieSources, efficiency_override},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
};
use ts_rs::TS;

pub struct AirMissions;

#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "AirMissionsView")]
pub enum View {
    #[default]
    History,
    Wings,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "AirMissionsParams")]
pub struct Params {
    pub branch_id: String,
    pub tag: String,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
    #[serde(default)]
    pub view: View,
    #[ts(optional)]
    pub hour: Option<i32>,
    /// One strategic region for the reached-hour drill.
    #[ts(optional)]
    pub region: Option<u32>,
}

/// A recorded mission column: its bit, its name in the game's language and
/// the frame of the mission strip the game draws for it.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AirMissionChoice")]
pub struct MissionChoice {
    pub id: String,
    pub name: String,
    pub icon: Option<SpriteFrame>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AirMissionsHistoryReply")]
pub struct HistoryReply {
    pub window: KeyedWindow,
    pub missions: Vec<MissionChoice>,
    pub regions: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theatres: Option<Vec<TheatreChoice>>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AirMissionsWing")]
pub struct Wing {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub aircraft_type: Option<String>,
    pub planes: i32,
    pub missions: Vec<String>,
    /// Raw fixed5 decimal text: 100,000 = one, not coverage or retained aircraft/count.
    pub efficiency: Option<String>,
    pub refusals: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AirMissionsWingsReply")]
pub struct WingsReply {
    pub hour: i32,
    pub read_hour: Option<i32>,
    pub region: u32,
    pub wings: Vec<Wing>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
#[ts(rename = "AirMissionsReply")]
pub enum Reply {
    History(HistoryReply),
    Wings(WingsReply),
}
impl ReplyWarnings for Reply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        match self {
            Self::History(reply) => &mut reply.warnings,
            Self::Wings(reply) => &mut reply.warnings,
        }
    }
}

impl Product for AirMissions {
    const KEY: &'static str = "air-missions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = Params;
    type Reply = Reply;
    fn validate(params: &Params) -> Result<(), &'static str> {
        shared::validate(
            &params.branch_id,
            &params.tag,
            params.first_hour,
            params.last_hour,
        )?;
        if matches!(params.view, View::Wings) && (params.hour.is_none() || params.region.is_none())
        {
            return Err("Wings view needs hour and region");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            Params::decl(),
            Reply::decl(),
            HistoryReply::decl(),
            WingsReply::decl(),
            Wing::decl(),
            View::decl(),
            MissionChoice::decl(),
            SpriteFrame::decl(),
            Choice::decl(),
            TheatreChoice::decl(),
            KeyedWindow::decl(),
            dashboard_keyed::query::KeyedSeries::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
        ]
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct AirMissionsObserver {
    pub refusals: BTreeMap<String, u64>,
}

pub static FAMILY: Family = dashboard_keyed::block::family::<AirMissionsObserver>(
    "air-missions",
    "air-missions.extract",
    "air-missions.sequential",
    "air-missions.fallback",
);

impl Observer for AirMissionsObserver {
    const NAME: &'static str = "air-missions";
    const TABLES: &'static [TableId] = &[
        TableId::AirWings,
        TableId::AirRegionalMissions,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::AirRegionalMissions;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::named(
                TableId::AirWings,
                &["owner", "mission_region", "mission_mask", "planes"],
            )?,
            TableFields::named(TableId::AirRegionalMissions, &["ordinal"])?,
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }
    fn observe(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let result = (|| -> Result<_, String> {
            let countries =
                Countries::read(frame).ok_or("Air mission country identities are not recorded")?;
            let wings = frame
                .complete::<tables::AirWings>()
                .map_err(|e| e.to_string())?;
            let registrations = frame
                .complete::<tables::AirRegionalMissions>()
                .map_err(|e| e.to_string())?;
            let owners =
                air_mission_counts::owners(registrations, wings, |raw| countries.slot(raw))?;
            for (owner, (counts, rosters)) in owners {
                let country = countries
                    .slotted(owner)
                    .ok_or("Air mission owner's tag is not recorded")?;
                if country.contains('|') {
                    return Err("Air mission owner's tag contains the series separator".into());
                }
                for (region, roster) in rosters {
                    let unique: HashSet<_> = roster.iter().map(|wing| wing.id).collect();
                    // names: TAG@slot|region|wings; TAG@slot|region|m<bit>.
                    values.push(format!("{country}|{region}|wings"), unique.len() as i128);
                    let mask = roster
                        .iter()
                        .filter(|wing| (wing.mission_mask as i32) > 0)
                        .fold(0, |mask, wing| mask | wing.mission_mask);
                    for (bit, &count) in counts[&region].iter().enumerate() {
                        if mask & (1 << bit) != 0 {
                            values.push(format!("{country}|{region}|m{bit}"), i128::from(count));
                        }
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(true),
            Err(why) => {
                shared::note(&mut self.refusals, why);
                Ok(false)
            }
        }
    }
}

// Native mission-bit localisation lookup, not authored mission definitions or
// fallback titles. Each value is looked up in the recorded content stack.
const MISSION_KEYS: [&str; MISSIONS] = [
    "AIRWING_MISSION_TYPE_AIR_SUPERIORITY",
    "AIRWING_MISSION_TYPE_CAS",
    "AIRWING_MISSION_TYPE_INTERCEPTION",
    "AIRWING_MISSION_TYPE_STRATEGIC_BOMBER",
    "AIRWING_MISSION_TYPE_NAVAL_BOMBER",
    "AIRWING_MISSION_TYPE_DROP_NUKE",
    "AIRWING_MISSION_TYPE_PARADROP",
    "AIRWING_MISSION_TYPE_NAVAL_KAMIKAZE",
    "AIRWING_MISSION_TYPE_PORT_STRIKE",
    "AIRWING_MISSION_TYPE_ATTACK_LOGISTICS",
    "AIRWING_MISSION_TYPE_AIR_SUPPLY",
    "AIRWING_MISSION_TYPE_TRAINING",
    "AIRWING_MISSION_TYPE_NAVAL_MINE_PLANTING",
    "AIRWING_MISSION_TYPE_NAVAL_MINE_SWEEPING",
    "AIRWING_MISSION_TYPE_RECON",
    "AIRWING_MISSION_TYPE_NAVAL_PATROL",
    "AIRWING_MISSION_TYPE_BARRAGE",
    "AIRWING_MISSION_TYPE_SAM",
];

/// The mission strip sprite the regional map's mission entry draws, resolved
/// through the recorded stack's GUI registry: mission bit `b` is its one-based
/// frame `b + 1`, which the sprite route draws as zero-based frame `b`.
fn mission_strip(query: &Query<'_>) -> Result<String, String> {
    let options = query.content.ok_or("Game folders are not chosen")?;
    let catalogue = catalogue::catalogue(query.source, options).map_err(|e| e.to_string())?;
    let guis = catalogue.guis().map_err(|e| e.to_string())?;
    let button = guis
        .template(MISSION_ENTRY_TEMPLATE)
        .ok_or_else(|| format!("GUI template {MISSION_ENTRY_TEMPLATE} is absent"))?
        .find(installed_content_core::gui::native::Kind::Button, "button")
        .and_then(|element| element.button())
        .ok_or_else(|| format!("GUI template {MISSION_ENTRY_TEMPLATE} has no button"))?;
    sprites::icon_key(&catalogue, &button.sprite)
}

pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    let params = shared::params::<AirMissions>(query)?;
    if matches!(params.view, View::Wings) {
        return wings(query, &params);
    }
    let (first, last) = query.window()?;
    let mut notes = Refusals::default();
    let labels = shared::labels(query);
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |labels| labels.warnings.clone());
    let window = match query.open::<AirMissionsObserver>()? {
        Some(mut opened) => {
            warnings.extend(shared::warnings(&opened.scan.observer.refusals));
            opened.window(first, last, query.through(), |name| {
                shared::parts(name)
                    .is_some_and(|(country, _, _)| shared::selected(country, &params.tag))
            })?
        }
        None => shared::empty(first, last),
    };
    let mut region_ids = BTreeSet::new();
    let mut bits = BTreeSet::new();
    for series in &window.series {
        if let Some((_, region, field)) = shared::parts(&series.name) {
            region_ids.insert(region);
            if let Some(bit) = field
                .strip_prefix('m')
                .and_then(|bit| bit.parse::<usize>().ok())
            {
                bits.insert(bit);
            }
        }
    }
    // Read the GUI registry only when a mission column is shown.
    let strip = (!bits.is_empty()).then(|| mission_strip(query));
    let missions = bits
        .into_iter()
        .map(|bit| {
            let name = labels
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|labels| labels.localisation.as_ref().map_err(Clone::clone))
                .and_then(|loc| {
                    MISSION_KEYS
                        .get(bit)
                        .ok_or_else(|| "Mission bit has no supported localisation mapping".into())
                        .and_then(|key| shared::localised(loc, key))
                })
                .unwrap_or_else(|why| {
                    notes.note(why, format!("mission {bit}"));
                    "Mission name unavailable".into()
                });
            let icon = match strip.as_ref().expect("a mission column reads the strip") {
                Ok(key) => Some(SpriteFrame {
                    key: key.clone(),
                    frame: bit as u32,
                }),
                Err(why) => {
                    notes.note(format!("icon: {why}"), format!("mission {bit}"));
                    None
                }
            };
            MissionChoice {
                id: bit.to_string(),
                name,
                icon,
            }
        })
        .collect();
    let regions = shared::region_choices(&region_ids, &labels, &mut notes);
    let hour = params.hour.or((first == last).then_some(first));
    let theatres = match hour {
        Some(hour) => shared::theatres(query, &params.tag, hour, &labels, &mut notes)?,
        None => {
            notes.note("Changing air-theatre membership needs a reached hour; an undated history union is not valid", "air-theatre choices");
            None
        }
    };
    warnings.extend(notes.lines("air mission presentation fields are"));
    shared::finish(
        query,
        &HistoryReply {
            window,
            missions,
            regions,
            theatres,
            warnings,
        },
    )
}

fn wings(query: &Query<'_>, params: &Params) -> io::Result<serde_json::Value> {
    let hour = params.hour.expect("validated wings hour");
    let region = params.region.expect("validated wings region");
    let (first, last) = query.bounds()?;
    if hour < first || hour > last {
        return Err(dashboard_core::invalid(
            "Wing detail hour is outside the branch",
        ));
    }
    let mut reply = WingsReply {
        hour,
        read_hour: None,
        region,
        wings: Vec::new(),
        warnings: Vec::new(),
    };
    // Reached-hour detail is independent of preparation: do not clip it to the
    // history gate, or pretend an unprepared roster is an empty roster.
    let tables = &[
        TableId::AirWings,
        TableId::AirRegionalMissions,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::AirSortieModifiers,
        TableId::AirMissionInputs,
        TableId::AirSortieBaseCountries,
        TableId::AirSortieSpecialBases,
        TableId::AirSortieModifierMetadata,
        TableId::SimulationFeatures,
        TableId::AirReferenceWitnesses,
        TableId::AirBases,
        TableId::VolunteerHosts,
        TableId::Provinces,
    ];
    let Some(frame) =
        progressive::frame_at(query.source, query.branch, hour, tables, query.cancelled)?
    else {
        reply
            .warnings
            .push("1 air wing roster not shown: requested hour is not recorded".into());
        return shared::finish(query, &reply);
    };
    let labels = shared::labels(query);
    let result = (|| -> Result<_, String> {
        let countries =
            Countries::read(&frame).ok_or("Air wing country identities are not recorded")?;
        let registrations = frame
            .complete::<tables::AirRegionalMissions>()
            .map_err(|e| e.to_string())?;
        let wings = frame
            .complete::<tables::AirWings>()
            .map_err(|e| e.to_string())?;
        let slots: BTreeSet<_> = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter_map(|row| {
                let country = countries.slotted(row.canonical_slot)?;
                shared::selected(&country, &params.tag).then_some(row.canonical_slot)
            })
            .collect();
        if slots.len() > 1 {
            return Err("Air wing country tag names multiple slots; select TAG@slot".into());
        }
        let owner = slots
            .first()
            .copied()
            .ok_or("Selected country is not recorded at the reached hour")?;
        let (_, rosters) =
            air_mission_counts::regional_counts(registrations, wings, owner, |raw| {
                countries.slot(raw)
            })?;
        let mut seen = HashSet::new();
        let mut selected = Vec::new();
        for wing in rosters.get(&region).into_iter().flatten().copied() {
            if seen.insert(wing.id) {
                selected.push(wing);
            }
        }
        Ok(selected)
    })();
    let selected = match result {
        Ok(wings) => wings,
        Err(why) => {
            reply
                .warnings
                .push(format!("1 air wing roster not shown: {why}"));
            return shared::finish(query, &reply);
        }
    };
    reply.read_hour = if dashboard_text::metadata::slot_read(
        query.source.replay_ref().format,
        &frame.metrics,
        TableId::AirRegionalMissions,
        hour,
    ) {
        Some(hour)
    } else if let Some(mut opened) = query.open::<AirMissionsObserver>()? {
        let window = opened.window(hour, hour, query.through(), |_| false)?;
        window.reads.first().map(|(_, _, read)| *read)
    } else {
        None
    };
    let mut notes = Refusals::default();
    let types = (|| -> Result<_, String> {
        let options = query.content.ok_or("Game folders are not chosen")?;
        let resolved = query
            .source
            .content(&options.options)
            .map_err(|e| e.to_string())?;
        resolved
            .layers
            .derived::<catalogue::CachedResult<installed_content_core::unit_leader::SubUnits>>()
            .get_or_init(|| installed_content_core::unit_leader::sub_units(&resolved.layers))
            .map_err(|e| e.to_string())
    })();
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let modifiers = frame
        .complete::<tables::AirSortieModifiers>()
        .map(|rows| {
            rows.iter()
                .map(|row| ((row.scope, row.subject, row.region), row.raw))
                .collect()
        })
        .map_err(|error| error.to_string());
    let sources = SortieSources::read(&frame)
        .map_err(|error| error.to_string())
        .and_then(|sources| sources.map_err(str::to_owned));
    let registry = AirRegistrySources::read(&frame)
        .map_err(|error| error.to_string())
        .and_then(|registry| registry.ok_or_else(|| "air base registry is incomplete".to_owned()));
    let countries = registry
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|registry| {
            MissionCountrySources::read(&frame, &links, registry).map_err(|error| error.to_string())
        });
    let fuel_penalty = (|| -> Result<i64, String> {
        let options = query.content.ok_or("Game folders are not chosen")?;
        let resolved = query
            .source
            .content(&options.options)
            .map_err(|error| error.to_string())?;
        let defines = resolved
            .layers
            .derived::<catalogue::CachedResult<installed_content_core::defines::Defines>>()
            .get_or_init(|| installed_content_core::defines::read(&resolved.layers))
            .map_err(|error| error.to_string())?;
        defines
            .fixed5("NAir", "MISSION_EFFICIENCY_MULT_AT_LACK_OF_FUEL")
            .map_err(|error| error.to_string())
    })();
    for wing in selected {
        dashboard_text::metadata::check_cancel(query.cancelled)?;
        let mut refusals = Vec::new();
        let name = wing
            .name
            .require("AirWings.name")
            .map_err(|e| e.to_string())
            .and_then(|name| {
                name.as_ref()
                    .ok_or_else(|| "Air wing name was not recorded".into())
            })
            .and_then(|bytes| shared::stored(query, &labels, bytes, "Air wing name"));
        let aircraft_type = wing
            .pool_subunit
            .require("AirWings.pool_subunit")
            .map_err(|e| e.to_string())
            .and_then(|index| index.ok_or_else(|| "Air wing pool type was not recorded".into()))
            .and_then(|index| {
                types
                    .as_ref()
                    .map_err(Clone::clone)?
                    .name(index)
                    .ok_or_else(|| {
                        format!("Air wing pool type {index} is absent from the recorded content")
                    })?
                    .map(str::to_owned)
            })
            .and_then(|key| {
                let loc = labels
                    .as_ref()
                    .map_err(Clone::clone)?
                    .localisation
                    .as_ref()
                    .map_err(Clone::clone)?;
                shared::localised(loc, &key)
            });
        let efficiency = (|| -> Result<String, String> {
            let raw = effective_raw(wing);
            let slot = canonical(raw, &links).ok_or("Sortie effective country is unresolved")?;
            // Overrides need neither content nor the ordinary law's other operands.
            if let Some(value) =
                efficiency_override(modifiers.as_ref().map_err(Clone::clone)?, (raw, slot))
            {
                return Ok(value.to_string());
            }
            sources
                .as_ref()
                .map_err(Clone::clone)?
                .wing_efficiency(
                    wing,
                    registry.as_ref().map_err(Clone::clone)?,
                    countries.as_ref().map_err(Clone::clone)?,
                    *fuel_penalty.as_ref().map_err(Clone::clone)?,
                )
                .map(|value| value.to_string())
                .map_err(str::to_owned)
        })();
        let mut field = |result: Result<String, String>| {
            result
                .map_err(|why| {
                    notes.note(why.clone(), format!("wing {}", wing.id));
                    refusals.push(why);
                })
                .ok()
        };
        let name = field(name);
        let aircraft_type = field(aircraft_type);
        let efficiency = field(efficiency);
        reply.wings.push(Wing {
            id: wing.id.to_string(),
            name,
            aircraft_type,
            planes: wing.planes,
            missions: (0..u32::BITS)
                .filter(|bit| wing.mission_mask & (1 << bit) != 0)
                .map(|bit| bit.to_string())
                .collect(),
            efficiency,
            refusals,
        });
    }
    reply.warnings.extend(notes.lines("air wing fields are"));
    shared::finish(query, &reply)
}
