//! The name a recording's file gets, for both recorders.

use installed_content_country_names::country_names::{self, CountryNames, Operands};
use installed_content_localisation::localisation::Localisation;
use replay::{campaign::Replay, campaign::SeekResult, recording_files, spool::Placement};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, Frame, FrameDecoder, TableView};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingName {
    /// The country's name, else its recorded tag; `None` when neither is known.
    pub country: Option<String>,
    /// The first and last in-game years recorded.
    pub years: Option<(i32, i32)>,
    pub multiplayer: bool,
    /// Why the country uses its recorded tag instead of a localised name.
    pub by_tag: Option<String>,
}

impl RecordingName {
    /// `Germany 1936–1941 (Multiplayer)` and the like, safe as a Windows file name.
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

/// The in-game year of an hour; none before the calendar's epoch.
pub fn year(hour: Hour) -> Option<i32> {
    (hour >= weather_period::EPOCH)
        .then(|| (hour - weather_period::EPOCH) / weather_period::HOURS_PER_YEAR)
}

fn years((first, last): (Hour, Hour)) -> Option<(i32, i32)> {
    Some((year(first)?, year(last)?))
}

/// The tables the recording player's country is named from.
fn country_tables() -> Vec<TableId> {
    let mut tables = country_names::TABLES.to_vec();
    tables.push(TableId::LocalPlayer);
    tables
}

/// The recording player's country at one hour, ready to be named.
#[derive(Clone, Debug)]
struct Country {
    slot: u32,
    tag: Option<String>,
    /// Its naming operands and those of its overlords.
    operands: BTreeMap<u32, Result<Operands, String>>,
}

impl Country {
    /// The recording client's playing country; none for an observer or rows that cannot tell.
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

    /// As the game names it in the recording's language, else its tag with the named refusal.
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

/// Whether saved hours prove multiplayer or single-player; neither when a roster is missing.
pub fn known_multiplayer(replay: &Replay, blocks: usize) -> io::Result<Option<bool>> {
    let mut known = true;
    let mut multiplayer = false;
    let chosen: Vec<_> = replay.index.iter().take(blocks).collect();
    replay.visit_blocks(
        &chosen,
        &[TableId::Players],
        None,
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

/// Whether an hour of the first `blocks` held more than one human; stops at the first.
pub fn multiplayer(replay: &Replay, blocks: usize) -> io::Result<bool> {
    let mut multiplayer = false;
    let chosen: Vec<_> = replay.index.iter().take(blocks).collect();
    replay.visit_blocks(
        &chosen,
        &[TableId::Players],
        None,
        |_, _, frame| {
            multiplayer = more_than_one_player(frame);
            Ok(!multiplayer)
        },
        || Ok(()),
    )?;
    Ok(multiplayer)
}

/// What a recording's name is made of, its country not yet put in words.
#[derive(Clone, Debug, Default)]
pub struct Contents {
    span: Option<(Hour, Hour)>,
    multiplayer: Option<bool>,
    country_known: bool,
    /// The recording player's country at the latest hour; `None` when it plays none.
    country: Option<Country>,
}

impl Contents {
    /// A finished recording's; `multiplayer` comes from its hours as they were saved.
    pub fn of(replay: &Replay, multiplayer: bool) -> io::Result<Self> {
        let span = replay
            .index
            .iter()
            .map(|block| (block.first_hour, block.last_hour))
            .reduce(|(first, last), (low, high)| (first.min(low), last.max(high)));
        // Of several branches reaching the latest hour, the last written.
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

    /// These hours with `added` appended after them, as a save appends a spool: written last, its
    /// country names the recording when it reaches the latest hour.
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

/// A finished recording's name; `multiplayer` comes from its hours as they were saved.
pub fn recording_name(
    replay: &Replay,
    localisation: Option<&Localisation>,
    multiplayer: bool,
) -> io::Result<RecordingName> {
    Ok(Contents::of(replay, multiplayer)?.name(localisation))
}

/// A growing spool's name: reading notes span, roster and latest frames; the country decodes when asked.
#[derive(Default)]
pub struct SpoolName {
    span: Option<(Hour, Hour)>,
    players: Option<FrameDecoder>,
    multiplayer: bool,
    /// A roster could not be read, so a single-player spool cannot be told from a multiplayer one.
    roster_lost: bool,
    /// The latest hour's branch and its frames since their keyframe, by segment.
    branch: Option<BranchId>,
    frames: Vec<(u32, Placement)>,
    /// Decoded through `frames[..done]`, and the country at the last of them.
    decoded: Option<(FrameDecoder, usize)>,
    country: Option<Country>,
    country_known: bool,
}

impl SpoolName {
    pub fn observe(&mut self, segment: u32, placement: Placement, frame: &Frame) {
        let latest = self.span.is_none_or(|(_, last)| frame.hour >= last);
        self.span = Some(self.span.map_or((frame.hour, frame.hour), |(first, last)| {
            (first.min(frame.hour), last.max(frame.hour))
        }));
        let players = self
            .players
            .get_or_insert_with(|| FrameDecoder::for_tables(&[TableId::Players]));
        match players.advance(frame) {
            Ok(decoded) => match roster(decoded) {
                Some(multiplayer) => self.multiplayer |= multiplayer,
                None => self.roster_lost = true,
            },
            Err(_) => {
                self.players = None;
                self.roster_lost = true;
            }
        }
        if !latest {
            return;
        }
        if frame.keyframe || self.branch != Some(frame.branch) {
            self.frames.clear();
            self.decoded = None;
            self.country = None;
            self.country_known = false;
            // Only a keyframe starts frames that can be decoded.
            self.branch = frame.keyframe.then_some(frame.branch);
        }
        if self.branch.is_some() {
            self.frames.push((segment, placement));
        }
    }

    pub fn last_hour(&self) -> Option<Hour> {
        self.span.map(|(_, last)| last)
    }

    pub fn summary(&self) -> Option<Contents> {
        if self.roster_lost && !self.multiplayer {
            return None;
        }
        Some(Contents {
            span: Some(self.span?),
            multiplayer: Some(self.multiplayer),
            ..Contents::default()
        })
    }

    /// Decodes the latest hour's frames not decoded yet; `None` before the first frame, and while
    /// its roster or the latest hour's country cannot be read.
    pub fn contents(
        &mut self,
        mut read: impl FnMut(u32, Placement) -> io::Result<Frame>,
    ) -> Option<Contents> {
        let span = self.span?;
        if self.frames.is_empty() || (self.roster_lost && !self.multiplayer) {
            return None;
        }
        let (decoder, done) = self
            .decoded
            .get_or_insert_with(|| (FrameDecoder::for_tables(&country_tables()), 0));
        while *done < self.frames.len() {
            let (segment, placement) = self.frames[*done];
            let frame = match read(segment, placement) {
                Ok(frame) => frame,
                Err(error) if invalid_frame(&error) => {
                    self.forget_frames();
                    return None;
                }
                // Unread, so the decoder still holds the frame before: read it again next time.
                Err(_) => return None,
            };
            *done += 1;
            match decoder.advance(&frame).map(|decoded| {
                (*done == self.frames.len()).then(|| {
                    let country = Country::of(decoded);
                    let known = Country::known(decoded, country.as_ref());
                    (country, known)
                })
            }) {
                Ok(Some((country, known))) => {
                    self.country = country;
                    self.country_known = known;
                }
                Ok(None) => {}
                Err(error) if invalid_frame(&error) => {
                    self.forget_frames();
                    return None;
                }
                // A refused advance may have spent its baseline: decode again from the keyframe.
                Err(_) => {
                    self.decoded = None;
                    return None;
                }
            }
        }
        if !self.country_known {
            return None;
        }
        Some(Contents {
            span: Some(span),
            multiplayer: Some(self.multiplayer),
            country_known: self.country_known,
            country: self.country.clone(),
        })
    }

    /// Drops the latest hour's frames once one of them cannot be decoded.
    fn forget_frames(&mut self) {
        self.frames.clear();
        self.decoded = None;
        self.country = None;
        self.country_known = false;
    }
}

/// Recorded bytes that cannot be a frame, unlike a read that failed for now.
fn invalid_frame(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
    )
}
