//! Countries' recorded map colours at many hours in one reply, for a chart that
//! colours each of its days as the game showed that day.
//!
//! Each hour is read as [`country_styles`](super::country_styles) reads it (the
//! same tables, the same gaps), so an hour either product can read gives the same
//! colour, and one it cannot is unread here. Colours seldom change, so the reply
//! holds each country's colour, or why it has none, only where that changes.

use super::{
    ContentOptions, LoadContext, MAX_PRODUCT_REPLY_BYTES, Product,
    country_styles::{HourStyles, STYLE_TABLES, prepared_inputs, seek_styles},
    validate_tag,
};
use crate::{Source, invalid, prepared::query::frames};
use dashboard_core::transport::json_len;
use dashboard_text::metadata::check_cancel;
use replay::campaign::SeekResult;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub struct CountryColours;

/// How a reply past [`MAX_PRODUCT_REPLY_BYTES`] is refused: the caller asks for fewer hours.
pub const OVER_BUDGET: &str = "Country colours exceed the product reply budget";

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryColourParams {
    pub branch_id: String,
    /// The hours to read, in any order.
    pub hours: Vec<i32>,
    /// The countries to read at every one of those hours.
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryColourRun {
    pub tag: String,
    /// The first read hour with this colour; it holds at each later read hour up to the tag's next run.
    pub hour: i32,
    /// The recorded map colour as sRGB bytes; null when none was recorded.
    pub color: Option<[u8; 3]>,
    /// Why there is no colour, as `country-styles` warns of it; null with a colour.
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnreadColourHour {
    pub hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryColourReply {
    pub branch_id: String,
    /// Every tag's runs over the read hours, in hour order.
    pub runs: Vec<CountryColourRun>,
    /// Requested hours that could not be read, and why.
    pub unread: Vec<UnreadColourHour>,
    pub warnings: Vec<String>,
}

impl Product for CountryColours {
    const KEY: &'static str = "country-colours";
    const CONTENT_DEPENDENT: bool = false;
    type Params = CountryColourParams;
    type Reply = CountryColourReply;

    fn types() -> Vec<String> {
        vec![
            CountryColourParams::decl(),
            CountryColourRun::decl(),
            UnreadColourHour::decl(),
            CountryColourReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        if params.hours.is_empty() || params.tags.is_empty() {
            return Err("Invalid country colour selection");
        }
        for tag in &params.tags {
            validate_tag(tag)?;
        }
        if params.branch_id.parse::<u64>().is_err() {
            return Err("Invalid country colour branch");
        }
        Ok(())
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        load_with(
            source,
            params,
            context.content.as_ref(),
            cancelled,
            MAX_PRODUCT_REPLY_BYTES,
        )
    }
}

/// The reply, refused with [`OVER_BUDGET`] as soon as it would pass `budget` bytes of JSON.
#[cfg(test)]
fn load_within(
    source: &Source,
    params: &CountryColourParams,
    cancelled: &AtomicBool,
    budget: usize,
) -> io::Result<CountryColourReply> {
    load_with(source, params, None, cancelled, budget)
}

/// [`load_within`], from preparation's runs when they hold every requested hour under `content`.
fn load_with(
    source: &Source,
    params: &CountryColourParams,
    content: Option<&ContentOptions>,
    cancelled: &AtomicBool,
    budget: usize,
) -> io::Result<CountryColourReply> {
    let branch = params
        .branch_id
        .parse::<u64>()
        .map_err(|_| invalid("Invalid country colour branch"))?;
    let mut hours = params.hours.clone();
    hours.sort_unstable();
    hours.dedup();
    let mut reply = Reply::new(params, budget)?;
    let tags = reply.tags.clone();
    // Each hour is taken as it is read, so the reply's budget stops the read as the seeks' does.
    if prepared_inputs(
        source,
        content,
        branch,
        &hours,
        &tags,
        false,
        cancelled,
        &mut |hour, inputs| reply.styles(hour, Ok(HourStyles::build(inputs, None)), cancelled),
    )? {
        return Ok(reply.reply);
    }
    // One block's hours at a time, so a block the pass cannot read is sought hour by hour.
    let mut groups: Vec<(Option<&replay::campaign::BlockIndex>, &[i32])> = Vec::new();
    let mut rest = &hours[..];
    while let Some(&first) = rest.first() {
        let block = source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= first && first <= block.last_hour
        });
        let count = block.map_or(1, |block| {
            rest.partition_point(|hour| *hour <= block.last_hour)
        });
        let (now, later) = rest.split_at(count);
        groups.push((block, now));
        rest = later;
    }
    // A request refusing block decodes reads as it always has: no reader decodes for it.
    if crate::prepared::query::progressive::block_decodes_disabled() {
        return read_groups(source, branch, &groups, reply, cancelled);
    }
    check_cancel(cancelled)?;
    // Readers warm the frame cache for later blocks; this thread reads and admits each hour in order.
    let blocks: Vec<_> = groups.iter().filter_map(|(block, _)| *block).collect();
    let hours_of: HashMap<u64, &[i32]> = groups
        .iter()
        .filter_map(|(block, now)| block.map(|block| (block.key(), *now)))
        .collect();
    let reader = std::thread::current().id();
    let prefetch = |block: &replay::campaign::BlockIndex| {
        if std::thread::current().id() != reader {
            let _ = frames::hours(
                source,
                branch,
                hours_of[&block.key()],
                STYLE_TABLES,
                cancelled,
                |_, _| Ok(()),
            );
        }
        Ok(())
    };
    let mut consumed = false;
    let scanned = dashboard_core::blocks::scan_blocks_by(
        blocks,
        PREFETCH,
        cancelled,
        &prefetch,
        |prefetched| {
            consumed = true;
            for (block, now) in &groups {
                check_cancel(cancelled)?;
                if block.is_some() {
                    prefetched.next();
                }
                read_group(source, branch, now, &mut reply, cancelled)?;
            }
            Ok(())
        },
    );
    match scanned {
        Ok(()) => Ok(reply.reply),
        // The read-ahead could not start: read as a request without it would.
        Err(_) if !consumed => read_groups(source, branch, &groups, reply, cancelled),
        Err(error) => Err(error),
    }
}

/// Every group in order, on this thread alone.
fn read_groups(
    source: &Source,
    branch: u64,
    groups: &[(Option<&replay::campaign::BlockIndex>, &[i32])],
    mut reply: Reply,
    cancelled: &AtomicBool,
) -> io::Result<CountryColourReply> {
    for (_, now) in groups {
        check_cancel(cancelled)?;
        read_group(source, branch, now, &mut reply, cancelled)?;
    }
    Ok(reply.reply)
}

/// Blocks the readers may decode ahead of the one this request reads.
const PREFETCH: usize = 4;

/// One block's hours (or one hour outside any block) read into the reply.
fn read_group(
    source: &Source,
    branch: u64,
    now: &[i32],
    reply: &mut Reply,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut visited = 0;
    let mut stop = None;
    let pass = frames::hours(
        source,
        branch,
        now,
        STYLE_TABLES,
        cancelled,
        |hour, result| {
            visited += 1;
            // The pass names each gap as a seek of its hour would.
            reply.read(hour, Ok(result), cancelled).map_err(|error| {
                let signal = io::Error::new(error.kind(), error.to_string());
                stop = Some(error);
                signal
            })
        },
    );
    if let Some(error) = stop {
        return Err(error);
    }
    match pass {
        Ok(()) => {}
        Err(error) if fatal(&error) => return Err(error),
        // Damage later in the block can fail the pass where a seek of an earlier hour reads.
        Err(_) => {
            for &hour in &now[visited..] {
                let result = seek_styles(source, branch, hour, cancelled);
                if let Err(error) = &result
                    && fatal(error)
                {
                    return Err(result.unwrap_err());
                }
                reply.read(hour, result, cancelled)?;
            }
        }
    }
    Ok(())
}

/// What ends the whole reply rather than one hour: cancellation, a refused
/// decode, running out of memory, or the reply's own budget.
fn fatal(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    ) || engine_allocation::is_allocation_error(error)
        || error.to_string().starts_with(OVER_BUDGET)
}

fn over_budget(budget: usize) -> io::Error {
    invalid(format!(
        "{OVER_BUDGET} ({budget} bytes of JSON); ask for fewer hours"
    ))
}

/// The reply being built, with its JSON size accounted as each entry joins.
struct Reply {
    reply: CountryColourReply,
    tags: Vec<String>,
    /// Each tag's last colour or reason, so only a change starts a run.
    last: Vec<Option<Result<[u8; 3], String>>>,
    bytes: usize,
    budget: usize,
}

impl Reply {
    fn new(params: &CountryColourParams, budget: usize) -> io::Result<Self> {
        let mut tags = params.tags.clone();
        tags.sort_unstable();
        tags.dedup();
        let reply = CountryColourReply {
            branch_id: params.branch_id.clone(),
            runs: Vec::new(),
            unread: Vec::new(),
            warnings: Vec::new(),
        };
        let bytes = json_len(&reply, budget).ok_or_else(|| over_budget(budget))?;
        Ok(Self {
            reply,
            last: vec![None; tags.len()],
            tags,
            bytes,
            budget,
        })
    }

    /// Accounts one more array entry, and its comma after an earlier one, before it is kept.
    fn admit(&mut self, entry: &impl Serialize, first: bool) -> io::Result<()> {
        let room = self.budget.saturating_sub(self.bytes);
        let bytes =
            json_len(entry, room).ok_or_else(|| over_budget(self.budget))? + usize::from(!first);
        if bytes > room {
            return Err(over_budget(self.budget));
        }
        self.bytes += bytes;
        Ok(())
    }

    fn unread(&mut self, hour: i32, reason: String) -> io::Result<()> {
        let entry = UnreadColourHour { hour, reason };
        self.admit(&entry, self.reply.unread.is_empty())?;
        self.reply.unread.push(entry);
        Ok(())
    }

    /// One hour's seek: its colours, or the hour unread with why.
    fn read(
        &mut self,
        hour: i32,
        result: io::Result<SeekResult>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let frame = match result {
            Ok(SeekResult::Frame(frame)) => frame,
            Ok(SeekResult::Gap { reason, .. }) => return self.unread(hour, reason),
            Err(error) => return self.unread(hour, error.to_string()),
        };
        self.styles(hour, HourStyles::read(&frame, None), cancelled)
    }

    /// One hour's styles: their colours, or the hour unread with why.
    fn styles(
        &mut self,
        hour: i32,
        styles: io::Result<HourStyles>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let styles = match styles {
            Ok(styles) => styles,
            Err(error) if engine_allocation::is_allocation_error(&error) => return Err(error),
            Err(error) => return self.unread(hour, error.to_string()),
        };
        for index in 0..self.tags.len() {
            check_cancel(cancelled)?;
            let colour = styles.colour(&self.tags[index]);
            if self.last[index].as_ref() == Some(&colour) {
                continue;
            }
            let (color, missing) = match &colour {
                Ok(color) => (Some(*color), None),
                Err(reason) => (None, Some(reason.clone())),
            };
            let run = CountryColourRun {
                tag: self.tags[index].clone(),
                hour,
                color,
                missing,
            };
            self.admit(&run, self.reply.runs.is_empty())?;
            self.reply.runs.push(run);
            self.last[index] = Some(colour);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
