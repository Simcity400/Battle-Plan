
use content_reader_country_names::country_names::{self, CountryNames, Operands};
use content_reader_localisation::localisation::Localisation;
use replay::campaign::{Replay, SeekResult};
use replay_files::recording_files;
use replay_data::{Hour, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingName {
    pub country: Option<String>,
    pub years: Option<(i32, i32)>,
    pub multiplayer: bool,
    pub by_tag: Option<String>,
}

impl RecordingName {
    pub fn stem(&self) -> String {
        let mut tail = String::new();
        match self.years {
            Some((first, last)) if first == last => tail.push_str(&format!(" {first}")),
            Some((first, last)) => tail.push_str(&format!(" {first}–{last}")),
            None => {}
        }
        if self.multiplayer {
            tail.push_str(" (Multiplayer)");
        }
        let head = self
            .country
            .as_deref()
            .and_then(recording_files::file_stem)
            .unwrap_or_else(|| "Campaign".into());
        let room = recording_files::MAX_STEM - recording_files::utf16_len(&tail);
        let head = recording_files::truncate(&head, room).trim_end();
        recording_files::file_stem(&format!("{head}{tail}"))
            .expect("a recording name is never empty")
    }
}

pub fn year(hour: Hour) -> Option<i32> {
    (hour >= weather_period::EPOCH)
        .then(|| (hour - weather_period::EPOCH) / weather_period::HOURS_PER_YEAR)
}

fn years((first, last): (Hour, Hour)) -> Option<(i32, i32)> {
    Some((year(first)?, year(last)?))
}

pub fn name_tables() -> Vec<TableId> {
    country_tables()
}

fn country_tables() -> Vec<TableId> {
    let mut tables = country_names::TABLES.to_vec();
    tables.push(TableId::LocalPlayer);
    tables
}

#[derive(Clone, Debug)]
struct Country {
    slot: u32,
    tag: Option<String>,
    operands: BTreeMap<u32, Result<Operands, String>>,
}

impl Country {
    fn of(frame: &DecodedFrame) -> Option<Self> {
        let [player] = frame.complete::<tables::LocalPlayer>().ok()? else {
            return None;
        };
        if (player.country as i32) <= 0 {
            return None;
        }
        let slot = frame
            .complete::<tables::CountryLinks>()
            .ok()?
            .iter()
            .rev()
            .find(|link| link.id == player.country)?
            .canonical_slot;
        let tag = frame
            .complete::<tables::Countries>()
            .ok()?
            .iter()
            .rev()
            .find(|country| country.canonical_slot == slot)
            .and_then(|country| String::from_utf8(country.tag.clone()).ok());
        let empty = Localisation::default();
        let names = CountryNames::read_selected(frame, &empty, [slot]);
        let mut operands = BTreeMap::new();
        let mut pending = vec![slot];
        while let Some(next) = pending.pop() {
            if operands.contains_key(&next) {
                continue;
            }
            let Some(input) = names.operands(next) else {
                continue;
            };
            if let Ok(Operands {
                overlord: Some(overlord),
                ..
            }) = input
            {
                pending.push(*overlord);
            }
            operands.insert(next, input.clone());
        }
        Some(Self {
            slot,
            tag,
            operands,
        })
    }

    fn known(frame: &DecodedFrame, country: Option<&Self>) -> bool {
        let Ok([player]) = frame.complete::<tables::LocalPlayer>() else {
            return false;
        };
        if (player.country as i32) <= 0 {
            return true;
        }
        if country_tables()
            .iter()
            .any(|table| !matches!(&*frame.tables[table.index()], TableView::Complete(_)))
        {
            return false;
        }
        let Ok(links) = frame.complete::<tables::CountryLinks>() else {
            return false;
        };
        let mut links = links.iter().filter(|link| link.id == player.country);
        let Some(link) = links.next() else {
            return false;
        };
        if links.next().is_some() {
            return false;
        }
        let Ok(countries) = frame.complete::<tables::Countries>() else {
            return false;
        };
        countries
            .iter()
            .rev()
            .find(|country| country.canonical_slot == link.canonical_slot)
            .is_some_and(|country| {
                std::str::from_utf8(&country.tag).is_ok_and(|tag| !tag.trim().is_empty())
            })
            && country.is_some_and(Self::operands_known)
    }

    fn operands_known(&self) -> bool {
        let mut pending = vec![self.slot];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(slot) = pending.pop() {
            if !seen.insert(slot) {
                continue;
            }
            let Some(Ok(operands)) = self.operands.get(&slot) else {
                return false;
            };
            pending.extend(operands.overlord);
        }
        true
    }

    fn name(&self, localisation: Option<&Localisation>) -> (Option<String>, Option<String>) {
        let refusal = if let Some(localisation) = localisation {
            match CountryNames::of_operands(localisation, self.operands.clone()).name(self.slot) {
                Ok(name) if !name.trim().is_empty() => return (Some(name), None),
                Ok(_) => Some("the country name is empty".to_owned()),
                Err(why) => Some(why),
            }
        } else {
            None
        };
        (
            self.tag.clone().filter(|tag| !tag.is_empty()),
            refusal.map(|why| format!("{}: {why}", self.tag.as_deref().unwrap_or("country"))),
        )
    }
}

fn roster(frame: &DecodedFrame) -> Option<bool> {
    match &*frame.tables[TableId::Players.index()] {
        TableView::Complete(rows) => Some(rows.len() > 1),
        TableView::Truncated { rows, .. } if rows.len() > 1 => Some(true),
        _ => None,
    }
}

fn more_than_one_player(frame: &DecodedFrame) -> bool {
    roster(frame) == Some(true)
}

pub fn known_multiplayer(replay: &Replay, blocks: usize) -> io::Result<Option<bool>> {
    let mut known = true;
    let mut multiplayer = false;
    let chosen: Vec<_> = replay.index.iter().take(blocks).collect();
    replay.visit_blocks(
        &chosen,
        &[TableId::Players],
        |_, _, frame| {
            match roster(frame) {
                Some(true) => multiplayer = true,
                Some(false) => {}
                None => known = false,
            }
            Ok(!multiplayer)
        },
        || Ok(()),
    )?;
    if multiplayer {
        return Ok(Some(true));
    }
    Ok(known.then_some(false))
}

pub fn multiplayer(replay: &Replay, blocks: usize) -> io::Result<bool> {
    multiplayer_among(replay, replay.index.iter().take(blocks).collect())
}

pub fn multiplayer_after(replay: &Replay, read: usize) -> io::Result<bool> {
    multiplayer_among(replay, replay.index.iter().skip(read).collect())
}

fn multiplayer_among(replay: &Replay, chosen: Vec<&replay::campaign::BlockIndex>) -> io::Result<bool> {
    let mut multiplayer = false;
    replay.visit_blocks(
        &chosen,
        &[TableId::Players],
        |_, _, frame| {
            multiplayer = more_than_one_player(frame);
            Ok(!multiplayer)
        },
        || Ok(()),
    )?;
    Ok(multiplayer)
}

#[derive(Clone, Debug, Default)]
pub struct Contents {
    span: Option<(Hour, Hour)>,
    multiplayer: Option<bool>,
    country_known: bool,
    country: Option<Country>,
}

impl Contents {
    pub fn of(replay: &Replay, multiplayer: bool) -> io::Result<Self> {
        let span = replay
            .index
            .iter()
            .map(|block| (block.first_hour, block.last_hour))
            .reduce(|(first, last), (low, high)| (first.min(low), last.max(high)));
        let mut country_known = false;
        let country = match replay.index.iter().max_by_key(|block| block.last_hour) {
            Some(block) => {
                match replay.seek_selected(
                    block.branch,
                    block.last_hour,
                    Some(&country_tables()),
                )? {
                    SeekResult::Frame(frame) => {
                        let country = Country::of(&frame);
                        country_known = Country::known(&frame, country.as_ref());
                        country
                    }
                    SeekResult::Gap { .. } => None,
                }
            }
            None => None,
        };
        Ok(Self {
            span,
            multiplayer: Some(multiplayer),
            country_known,
            country,
        })
    }

    pub fn then(self, added: Self) -> Self {
        let last = |contents: &Self| contents.span.map(|(_, last)| last);
        Self {
            country_known: if last(&added) >= last(&self) {
                added.country_known
            } else {
                self.country_known
            },
            country: if last(&added) >= last(&self) {
                added.country
            } else {
                self.country
            },
            span: match (self.span, added.span) {
                (Some((first, last)), Some((low, high))) => Some((first.min(low), last.max(high))),
                (span, other) => span.or(other),
            },
            multiplayer: match (self.multiplayer, added.multiplayer) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
        }
    }

    pub fn recording(span: (Hour, Hour), multiplayer: bool, frame: &DecodedFrame) -> Self {
        let country = Country::of(frame);
        Self {
            span: Some(span),
            multiplayer: Some(multiplayer),
            country_known: Country::known(frame, country.as_ref()),
            country,
        }
    }

    pub fn for_prediction(replay: &Replay) -> io::Result<Self> {
        let multiplayer = known_multiplayer(replay, replay.index.len())?;
        let mut contents = Self::of(replay, multiplayer == Some(true))?;
        contents.multiplayer = multiplayer;
        Ok(contents)
    }

    pub fn predicted(&self, localisation: Option<&Localisation>) -> Option<RecordingName> {
        self.multiplayer?;
        self.country_known.then(|| self.name(localisation))
    }

    pub fn name(&self, localisation: Option<&Localisation>) -> RecordingName {
        let (country, by_tag) = self
            .country
            .as_ref()
            .map(|country| country.name(localisation))
            .unwrap_or_default();
        RecordingName {
            country,
            years: self.span.and_then(years),
            multiplayer: self.multiplayer == Some(true),
            by_tag,
        }
    }
}

pub fn recording_name(
    replay: &Replay,
    localisation: Option<&Localisation>,
    multiplayer: bool,
) -> io::Result<RecordingName> {
    Ok(Contents::of(replay, multiplayer)?.name(localisation))
}
