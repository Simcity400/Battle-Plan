
use super::{Line, MissingSpan, NavigationKey, SeriesRead, SeriesState, hour_of, invalid};
use crate::{chart::ChartSummary, tile::TilePart};
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RankedSpan {
    pub first_hour: i32,
    pub last_hour: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankedLine {
    pub tag: String,
    #[doc = " Its holders over the view."]
    pub parts: Vec<TilePart>,
    pub summary: ChartSummary,
    pub membership: Vec<RankedSpan>,
    pub missing: Vec<MissingSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRankedReply {
    pub branch_id: String,
    pub measure: String,
    pub decimals: u8,
    pub lines: Vec<SeriesRankedLine>,
}

const WIDTH: u32 = 8192;

pub(super) fn top(line: &Line<'_, '_>, top: Option<u32>) -> io::Result<Option<usize>> {
    match (top, line.series.parts) {
        (None, _) => Ok(None),
        (Some(_), Some(_)) => Ok(None),
        (Some(0), None) => Err(invalid(format!("A ranked {} names a top of at least 1", line.series.kind))),
        (Some(n), None) => Ok(Some(n as usize)),
    }
}

pub(super) fn states(line: &Line<'_, '_>, hour: Option<f64>, n: usize) -> io::Result<Vec<SeriesState>> {
    let hour = hour_of(hour, "hour")?;
    let reads: Vec<SeriesRead> = line.rank_at(hour, n)?.into_iter().map(|tag| SeriesRead { country: tag.into(), hour, part: None }).collect();
    line.states(&reads, None)
}

pub(super) fn summary(line: &Line<'_, '_>, branch_id: &str, from: Option<f64>, to: Option<f64>, width: Option<u32>, n: usize) -> io::Result<SeriesRankedReply> {
    let (from, to) = (hour_of(from, "from")?, hour_of(to, "to")?);
    if from > to {
        return Err(invalid("Invalid view"));
    }
    let width = width.filter(|width| (1..=WIDTH).contains(width)).ok_or_else(|| invalid("Invalid width"))?;
    let mut lines = Vec::new();
    for ranked in line.ranked(from, to, width, n)? {
        let parts = line
            .parts(&ranked.tag, from, to)
            .into_iter()
            .map(|part| TilePart { first_hour: part.first, last_hour: part.last, slot: part.slot, country_id: part.id, segment_first_hour: part.segment_first })
            .collect();
        lines.push(SeriesRankedLine {
            tag: ranked.tag,
            parts,
            summary: ranked.summary,
            membership: ranked.membership.into_iter().map(|(first_hour, last_hour)| RankedSpan { first_hour, last_hour }).collect(),
            missing: ranked.missing,
        });
    }
    Ok(SeriesRankedReply { branch_id: branch_id.to_owned(), measure: line.measure_name().to_owned(), decimals: line.decimals(), lines })
}

pub(super) fn stepped(line: &Line<'_, '_>, origin: Option<i32>, key: NavigationKey, bounds: (i32, i32), n: usize) -> io::Result<Vec<String>> {
    let at = match (origin, key) {
        (Some(origin), _) => origin.clamp(bounds.0, bounds.1),
        (None, NavigationKey::End | NavigationKey::ArrowLeft) => bounds.1,
        (None, _) => bounds.0,
    };
    line.rank_at(at, n)
}
