//! Current annual-ring air-statistics buckets, not loss events or differences.
//! Names: `TAG@slot|<regionId>|c<channel>` for all 24 raw signed channels,
//! `TAG@slot|<regionId>|slot` and `TAG@slot|<regionId>|wrapped` for ring state.
//! Queries return only the selected pair (`c<i>`, `c<i+12>`) and ring state.
//! A known hour without a row is absent, not a recorded zero.

use crate::shared::{self, Choice, TheatreChoice};
use dashboard_base::{
    catalogue::{self, Refusals},
    prepared::published,
    products::Product,
};
use dashboard_keyed::query::{KeyedWindow, Query};
use dashboard_series::{KeyHandle, family::Directory};
use engine_air_history::cloop_history::air_stats;
use installed_content_localisation::localisation::Argument;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

pub mod emit;
pub mod segment;
mod served;

/// The family's name: its publication's and its status's.
pub const NAME: &str = "air-statistics";

pub struct AirStatistics;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "AirStatisticsParams")]
pub struct Params {
    pub branch_id: String,
    pub tag: String,
    /// Pair index in recorded schema order, not the interface's button order.
    pub channel: u8,
    #[ts(optional)]
    pub first_hour: Option<i32>,
    #[ts(optional)]
    pub last_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "AirStatisticsChannelChoice")]
pub struct ChannelChoice {
    pub id: String,
    pub ours: String,
    pub enemy: String,
    pub divisor: i32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AirStatisticsReply")]
pub struct Reply {
    pub window: KeyedWindow,
    pub channels: Vec<ChannelChoice>,
    pub regions: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theatres: Option<Vec<TheatreChoice>>,
    pub warnings: Vec<String>,
}

impl Product for AirStatistics {
    const KEY: &'static str = NAME;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = Params;
    type Reply = Reply;
    fn validate(params: &Params) -> Result<(), &'static str> {
        shared::validate(
            &params.branch_id,
            &params.tag,
            params.first_hour,
            params.last_hour,
        )?;
        if usize::from(params.channel) >= air_stats::CHANNELS.len() / 2 {
            return Err("Invalid air-statistics channel");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            Params::decl(),
            Reply::decl(),
            ChannelChoice::decl(),
            Choice::decl(),
            TheatreChoice::decl(),
            KeyedWindow::decl(),
            dashboard_keyed::query::KeyedSeries::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
        ]
    }
}

/// Interface detail-channel ordinals for the schema's twelve pairs. Disruption
/// precedes ground-equipment losses in the interface, but not in stored rows.
const DETAIL_CHANNELS: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 8, 12];

pub fn divisor(channel: usize) -> i32 {
    if matches!(
        air_stats::CHANNELS[channel],
        air_stats::OUR_BUILDINGS_BOMBED | air_stats::ENEMY_BUILDINGS_BOMBED
    ) {
        air_stats::BUILDINGS_BOMBED_DISPLAY_DIVISOR as i32
    } else {
        air_stats::DISPLAY_DIVISOR as i32
    }
}

/// The selected country's statistics from the family's native publication.
pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    let opened = published::open(
        query.source,
        query.content,
        NAME,
        emit::SCHEMA,
        query.branch,
    )?;
    answer(query, opened.as_ref())
}

/// What a reply reads of the publication `directory` lists: the refused observations, the
/// known hours' and reads' keys, and the selected pair's and ring state's of each unit of
/// `tag`, in the order the observation first held them.
type Selected = (
    BTreeMap<String, u64>,
    [Option<KeyHandle>; 2],
    Vec<(String, Option<KeyHandle>)>,
);

fn selected(directory: &Directory, tag: &str, channel: u8) -> io::Result<Selected> {
    let refusals = match directory
        .scalar(emit::REFUSALS)
        .and_then(|scalar| scalar.text())
    {
        Some(json) => serde_json::from_slice(json)?,
        None => BTreeMap::new(),
    };
    let own = [
        directory.key(emit::FAMILY, emit::KNOWN, None),
        directory.key(emit::FAMILY, emit::READ, None),
    ];
    let channel = usize::from(channel);
    let fields = [
        (format!("c{channel}"), channel),
        (format!("c{}", channel + 12), channel + 12),
        ("slot".to_owned(), segment::SLOT),
        ("wrapped".to_owned(), segment::WRAPPED),
    ];
    let mut keyed = Vec::new();
    for (actor, unit) in directory.actors.iter().enumerate() {
        let name = std::str::from_utf8(&unit.name)
            .map_err(|_| dashboard_core::invalid("An air-statistics unit is not text"))?;
        let Some((country, _)) = name.split_once('|') else {
            continue;
        };
        if !shared::selected(country, tag) {
            continue;
        }
        for (field, measure) in &fields {
            let key = directory.key(actor as u64, *measure as u64, None);
            keyed.push((format!("{name}|{field}"), key));
        }
    }
    Ok((refusals, own, keyed))
}

/// The reply to `query` from the family's publication `opened`, None before its first.
pub(crate) fn answer(
    query: &Query<'_>,
    opened: Option<&published::Opened>,
) -> io::Result<serde_json::Value> {
    let params = shared::params::<AirStatistics>(query)?;
    let (first, last) = query.window()?;
    let labels = shared::labels(query);
    let mut notes = Refusals::default();
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |labels| labels.warnings.clone());
    warnings.extend(shared::language_note(query));
    let window = match opened {
        Some(opened) => {
            let (refusals, own, keyed) = opened.read(query.cancelled, |_, directory| {
                selected(directory, &params.tag, params.channel)
            })?;
            warnings.extend(shared::warnings(&refusals));
            dashboard_keyed::query::published_window(
                opened,
                own,
                keyed,
                (first, last),
                query.through(),
                query.cancelled,
            )?
        }
        None => shared::empty(first, last),
    };
    let ids = window
        .series
        .iter()
        .filter_map(|series| shared::parts(&series.name).map(|(_, region, _)| region))
        .collect::<BTreeSet<_>>();
    let regions = shared::region_choices(&ids, &labels, &mut notes);
    let channels = DETAIL_CHANNELS
        .iter()
        .enumerate()
        .map(|(channel, &ordinal)| {
            let mut label = |enemy: bool| {
                let key = format!(
                    "AIRVIEW_DETAILS_CHANNEL_{}",
                    ordinal + if enemy { 12 } else { 0 }
                );
                labels
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|labels| labels.localisation.as_ref().map_err(Clone::clone))
                    .and_then(|loc| {
                        if loc.get(&key).is_none() {
                            return Err(format!(
                                "Localisation {key} is absent from the recorded content stack"
                            ));
                        }
                        let text = loc
                            .localise_with(key.as_bytes(), &[("VAL", Argument::Text(""))])
                            .map_err(|e| e.to_string())?;
                        catalogue::plain_text(&text).map(|text| text.trim().to_owned())
                    })
                    .unwrap_or_else(|why| {
                        notes.note(
                            why,
                            format!("channel {channel} {}", if enemy { "enemy" } else { "ours" }),
                        );
                        "Channel name unavailable".into()
                    })
            };
            ChannelChoice {
                id: channel.to_string(),
                ours: label(false),
                enemy: label(true),
                divisor: divisor(channel),
            }
        })
        .collect();
    let theatres = if first == last {
        shared::theatres(query, &params.tag, first, &labels, &mut notes)?
    } else {
        notes.note("Changing air-theatre membership needs a reached hour; an undated history union is not valid", "air-theatre choices");
        None
    };
    warnings.extend(notes.lines("air-statistics presentation fields are"));
    shared::finish(
        query,
        &Reply {
            window,
            channels,
            regions,
            theatres,
            warnings,
        },
    )
}
