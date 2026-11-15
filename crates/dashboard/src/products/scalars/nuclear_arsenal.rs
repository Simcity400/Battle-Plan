//! Every country's nuclear stockpile at an hour as the game's top bar shows it: each type's amount
//! truncated, the amounts added with signed 32-bit wrapping. A rows-at-an-hour product over the
//! `Nukes` table (a few rows a country, a few kilobytes a campaign), so the total and the rank are
//! exact at every hour without a table of their own. Each type's line over time is the declared
//! `country-nukes` series. Type names are the recorded lexical tokens, localised in the reader's
//! language; a type the game does not name is counted and refused by name.

use super::{Named, rows};
use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour, Read},
    },
};
use engine_native::raid_tooltip_operands::{NUCLEAR_BOMB_TOKEN, THERMONUCLEAR_BOMB_TOKEN};
use installed_content_localisation::localisation::Localisation;
use replay_data::rows::{NukeRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc};
use ts_rs::TS;

pub struct NuclearArsenal;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NuclearArsenalParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeType {
    pub ordinal: u32,
    /// Its lexical token's spelling; null when the game names no token for it.
    pub token: Option<String>,
    /// Its name in the reader's language; null when the content has none.
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeCount {
    pub ordinal: u32,
    /// Whole bombs, as the top bar shows them.
    pub value: i32,
    /// Queued drops whose delay elapsed.
    pub ready: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NukeCountry {
    pub tag: String,
    pub country_id: u32,
    /// The top bar's stockpile: every type's count added with signed 32-bit wrapping.
    pub total: i32,
    pub counts: Vec<NukeCount>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NuclearArsenalReply {
    pub hour: i32,
    /// Every type the game names and every other type a country holds at the hour, by ordinal.
    pub types: Vec<NukeType>,
    /// Every country a tag holds with a stockpile row, greatest total first, ties by tag.
    pub countries: Vec<NukeCountry>,
    /// Why no stockpile is known at the hour.
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(NuclearArsenalReply);

/// The nuclear types the game names.
const NAMED_TYPES: [u32; 2] = [0, 1];

/// The lexical token the game names nuclear type `ordinal` by.
pub(crate) fn type_token(ordinal: u32) -> Option<u32> {
    match ordinal {
        0 => Some(NUCLEAR_BOMB_TOKEN),
        1 => Some(THERMONUCLEAR_BOMB_TOKEN),
        _ => None,
    }
}

/// Each country's counts from `rows` (keyed by country, then type), totalled as the top bar adds
/// them, greatest first, ties by tag; a country no tag holds is left out and counted.
pub(super) fn countries(rows: &[NukeRow], tag_of: impl Fn(u32) -> Option<String>) -> (Vec<NukeCountry>, usize) {
    let mut by_country: BTreeMap<u32, Vec<NukeCount>> = BTreeMap::new();
    for row in rows {
        by_country.entry(row.country).or_default().push(NukeCount { ordinal: row.nuke_type, value: NukeRow::shown(row.amount_raw), ready: row.ready });
    }
    let mut unheld = 0;
    let mut out: Vec<NukeCountry> = Vec::new();
    for (country, counts) in by_country {
        let Some(tag) = tag_of(country) else {
            unheld += 1;
            continue;
        };
        let total = counts.iter().fold(0i32, |sum, count| sum.wrapping_add(count.value));
        out.push(NukeCountry { tag, country_id: country, total, counts });
    }
    out.sort_unstable_by(|a, b| b.total.cmp(&a.total).then_with(|| a.tag.cmp(&b.tag)));
    (out, unheld)
}

impl AtHour for NuclearArsenal {
    const KEY: &'static str = "nuclear-arsenal";
    const TITLE: &'static str = "Nuclear arsenal";
    const CONTENT_DEPENDENT: bool = true;
    type Params = NuclearArsenalParams;
    type Reply = NuclearArsenalReply;
    type Context = Result<Arc<Localisation>, String>;

    fn types() -> Vec<String> {
        vec![NuclearArsenalParams::decl(), NukeType::decl(), NukeCount::decl(), NukeCountry::decl(), NuclearArsenalReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::Nukes, TableId::LexicalTokens]
    }

    fn reads(_params: &Self::Params, _owner: Option<crate::products::at_hour::Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => vec![Read::all(TableId::Nukes), Read::these(TableId::LexicalTokens, [NUCLEAR_BOMB_TOKEN, THERMONUCLEAR_BOMB_TOKEN].map(u128::from))],
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(match load.content.as_ref() {
            Some(content) => dashboard_base::content::localisation(source, content).map_err(|error| format!("Nuclear type names are unavailable: {error}")),
            None => Err("Nuclear type names are unavailable until the game folders are chosen".into()),
        })
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut reply = NuclearArsenalReply { hour: frame.hour, types: Vec::new(), countries: Vec::new(), missing: None, warnings: Vec::new() };
        let nukes = match rows::<tables::Nukes>(frame) {
            Ok(nukes) => nukes,
            Err(reason) => {
                reply.missing = Some(format!("Nuclear stockpiles are not known at this hour: {reason}"));
                return Ok(reply);
            }
        };
        let named = Named::new(at.holders, frame.hour);
        let (countries, unheld) = countries(nukes, |country| named.id(country).map(str::to_owned));
        reply.countries = countries;
        if unheld > 0 {
            reply.warnings.push(format!("{unheld} countries with a stockpile are held by no tag at this hour and are not shown"));
        }
        let spellings: BTreeMap<u32, &[u8]> = rows::<tables::LexicalTokens>(frame).map(|held| held.iter().map(|row| (row.token, row.literal.as_slice())).collect()).unwrap_or_default();
        // Every type the game names is listed, held or not, so a reply at any hour names every part a line may hold.
        let mut ordinals: Vec<u32> = nukes.iter().map(|row| row.nuke_type).chain(NAMED_TYPES).collect();
        ordinals.sort_unstable();
        ordinals.dedup();
        if let Err(reason) = &at.context {
            reply.warnings.push(reason.clone());
        }
        for ordinal in ordinals {
            let Some(token) = type_token(ordinal) else {
                reply.warnings.push(format!("Nuclear type {ordinal} is not one the game names: it is counted in totals and shown unnamed"));
                reply.types.push(NukeType { ordinal, token: None, name: None });
                continue;
            };
            let spelling = spellings.get(&token).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_owned);
            if spelling.is_none() {
                reply.warnings.push(format!("Nuclear type {ordinal}'s lexical token {token} has no recorded spelling"));
            }
            let name = match (&spelling, &at.context) {
                (Some(spelling), Ok(localisation)) => dashboard_text::catalogue::localised(localisation, spelling, dashboard_text::catalogue::View::Plain)
                    .map_err(|reason| reply.warnings.push(format!("{spelling}: {reason}")))
                    .ok(),
                _ => None,
            };
            reply.types.push(NukeType { ordinal, token: spelling, name });
        }
        Ok(reply)
    }
}
