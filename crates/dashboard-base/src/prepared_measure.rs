//! The prepared measure series' shared reply rows and holders, and the stateless country balance
//! sampler the progressive tail's tests read frames with.
use crate::prepared::series::Run;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod query;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub value_raw: String,
    pub slot: u32,
    pub country_id: u32,
    pub starts_segment: bool,
    pub segment_first_hour: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub read_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MeasureActor {
    pub actor: String,
    pub tag: String,
    pub measure: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub run_count: String,
    pub drop_count: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureHolder {
    pub slot: u32,
    pub country_id: u32,
    #[serde(default)]
    pub read_hour: Option<i32>,
    #[serde(default)]
    pub segment_first_hour: Option<i32>,
}
impl MeasureRun {
    pub fn from_segment(run: Run, segment_first_hour: i32, holder: MeasureHolder) -> Self {
        Self {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
            value_raw: run.value.to_string(),
            starts_segment: run.break_before,
            slot: holder.slot,
            country_id: holder.country_id,
            segment_first_hour: holder.segment_first_hour.unwrap_or(segment_first_hour),
            read_hour: holder.read_hour,
        }
    }
}

/// The recorded tables the stateless balance sampler reads.
#[cfg(test)]
pub const TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::Campaign,
    replay_data::rows::TableId::Countries,
    replay_data::rows::TableId::CountryLinks,
];

/// Each tag's political and command power in `frame`, with its holder, addressed by recorded
/// tag and measure label.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub fn sample(
    frame: &replay_frame::frame::DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> std::io::Result<std::collections::BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>>
{
    use crate::metadata::{TaggedCountry, tagged_countries};
    use replay_data::rows::tables;
    const LABELS: [&str; 2] = ["politicalPower", "commandPower"];
    if requested.is_some_and(|rows| rows.iter().any(|(_, label)| !LABELS.contains(&label.as_str())))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Unknown country balance sample measure",
        ));
    }
    let mut result = std::collections::BTreeMap::new();
    if let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        for (tag, lookup) in tagged_countries(countries, links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let (identity, values) = match lookup {
                TaggedCountry::Found(row, slot) => (
                    Some((slot, row.id)),
                    [
                        row.political_power_raw.map(i128::from),
                        Some(i128::from(row.command_power_raw)),
                    ],
                ),
                _ => (None, [None; 2]),
            };
            for (label, value) in LABELS.into_iter().zip(values) {
                if requested.is_none_or(|rows| {
                    rows.iter()
                        .any(|(wanted, asked)| wanted == tag && asked == label)
                }) {
                    result.insert((tag.to_owned(), label.to_owned()), (identity, value));
                }
            }
        }
    }
    for key in requested.into_iter().flatten() {
        result.entry(key.clone()).or_insert((None, None));
    }
    Ok(result)
}
