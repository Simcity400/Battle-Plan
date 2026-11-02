//! Current annual-ring air-statistics buckets, not loss events or differences.
//! Names: `TAG@slot|<regionId>|c<channel>` for all 24 raw signed channels,
//! `TAG@slot|<regionId>|slot` and `TAG@slot|<regionId>|wrapped` for ring state.
//! Queries return only the selected pair (`c<i>`, `c<i+12>`) and ring state.
//! A known hour without a row is absent, not a recorded zero.

use crate::shared::{self, Choice, TheatreChoice};
use dashboard_base::{
    catalogue::{self, Refusals},
    prepared::frame_changes::FrameChanges,
    products::Product,
};
#[cfg(test)]
use dashboard_base::prepared::prepass::Family;
use dashboard_keyed::{
    Observer, Values,
    countries::Countries,
    query::{KeyedWindow, Query},
};
use engine_air_history::cloop_history::air_stats;
use installed_content_localisation::localisation::Argument;
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use ts_rs::TS;

mod served;

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
    const KEY: &'static str = "air-statistics";
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

#[derive(Default, Serialize, Deserialize)]
pub struct AirStatisticsObserver {
    /// Only raw references reached by the supplying read, frozen as TAG@slot.
    bindings: Option<BTreeMap<u32, String>>,
    /// The supplying read the bindings were frozen at.
    #[serde(default)]
    read_hour: Option<i32>,
    pub refusals: BTreeMap<String, u64>,
}

#[cfg(test)]
pub static FAMILY: Family = dashboard_keyed::block::family::<AirStatisticsObserver>(
    "air-statistics",
    "air-statistics.extract",
    "air-statistics.sequential",
    "air-statistics.fallback",
);

impl Observer for AirStatisticsObserver {
    type Content = ();

    fn content(
        _source: &dashboard_base::Source,
        _options: Option<&dashboard_core::config::ContentOptions>,
        _check: &mut dyn FnMut() -> std::io::Result<()>,
    ) -> std::io::Result<Self::Content> {
        Ok(())
    }

    const NAME: &'static str = "air-statistics";
    const TABLES: &'static [TableId] = &[
        TableId::AirStatistics,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::AirStatistics;
    #[cfg(test)]
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(TableId::AirStatistics).when_read(),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }
    fn supplying_read(&self) -> Option<i32> {
        self.read_hour.filter(|_| self.bindings.is_some())
    }
    fn changed(changes: &FrameChanges) -> bool {
        changes.slot_read[Self::READ.index()]
            || Self::TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let read =
            dashboard_text::metadata::slot_read(format, &frame.metrics, Self::READ, frame.hour);
        if read {
            // A refused fresh read cannot leave the preceding read's bindings usable.
            self.bindings = None;
            self.read_hour = None;
        }
        let result = (|| -> Result<_, String> {
            // Without this hour's identities the carried attribution is unknown for
            // the hour; the read-time bindings themselves stay as recorded.
            let countries = Countries::read(frame)
                .ok_or("Air-statistics country identities are not recorded")?;
            let rows = frame
                .complete::<tables::AirStatistics>()
                .map_err(|e| e.to_string())?;
            if read {
                let mut bindings = BTreeMap::new();
                for raw in rows.iter().map(|row| row.country).collect::<BTreeSet<_>>() {
                    let country = countries
                        .raw(raw)
                        .ok_or_else(|| format!("Air-statistics country {raw} is unresolved"))?;
                    if country.contains('|') {
                        return Err("Air-statistics tag contains the series separator".into());
                    }
                    bindings.insert(raw, country);
                }
                self.bindings = Some(bindings);
                self.read_hour = Some(frame.hour);
            }
            let bindings = self
                .bindings
                .as_ref()
                .ok_or("Air-statistics country bindings at the supplying read are not recorded")?;
            for row in rows {
                // A carried row keeps its read-time identity only while this hour's
                // identity still agrees with it; a changed one is unknown until it
                // agrees again or a fresh read rebinds it.
                let country = bindings
                    .get(&row.country)
                    .filter(|country| countries.raw(row.country).as_deref() == Some(country.as_str()))
                    .ok_or_else(|| {
                        format!(
                            "Air-statistics country {} does not match its supplying-read binding at this hour",
                            row.country
                        )
                    })?;
                let name = format!("{country}|{}", row.region);
                // names: TAG@slot|region|c<0..23>, |slot, |wrapped.
                for (channel, &raw) in row.channels.iter().enumerate() {
                    values.push(format!("{name}|c{channel}"), i128::from(raw));
                }
                values.push(format!("{name}|slot"), i128::from(row.year_slot));
                values.push(
                    format!("{name}|wrapped"),
                    i128::from(row.year_window_wrapped),
                );
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
    answer(query, &|| query.open_native::<AirStatisticsObserver>())
}

/// The reply to `query` from the family's publication `open` opens.
pub(crate) fn answer<'q>(
    query: &'q Query<'_>,
    open: &crate::products::air_missions::Open<'q, AirStatisticsObserver>,
) -> io::Result<serde_json::Value> {
    let params = shared::params::<AirStatistics>(query)?;
    let (first, last) = query.window()?;
    let labels = shared::labels(query);
    let mut notes = Refusals::default();
    let mut warnings = labels
        .as_ref()
        .map_or_else(|_| Vec::new(), |labels| labels.warnings.clone());
    let pair = [
        format!("c{}", params.channel),
        format!("c{}", params.channel + 12),
    ];
    let window = match open()? {
        Some(mut opened) => {
            warnings.extend(shared::warnings(&opened.scan.observer.refusals));
            opened.window(first, last, query.through(), |name| {
                shared::parts(name).is_some_and(|(country, _, field)| {
                    shared::selected(country, &params.tag)
                        && (pair.iter().any(|part| part == field)
                            || matches!(field, "slot" | "wrapped"))
                })
            })?
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
