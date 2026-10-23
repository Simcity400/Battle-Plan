//! The name a recording's file gets, for both recorders.

use installed_content_country_names::country_names::{self, CountryNames, Operands};
use installed_content_localisation::localisation::Localisation;
use replay::{container::Replay, container::SeekResult, recording_files, spool::Placement};
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

    /// As the game names it in the recording's language, else its tag.
    fn name(&self, localisation: Option<&Localisation>) -> Option<String> {
        localisation
            .and_then(|localisation| {
                CountryNames::of_operands(localisation, self.operands.clone())
                    .name(self.slot)
                    .ok()
            })
            .filter(|name| !name.trim().is_empty())
            .or_else(|| self.tag.clone().filter(|tag| !tag.is_empty()))
    }
}

fn more_than_one_player(frame: &DecodedFrame) -> bool {
    match &*frame.tables[TableId::Players.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len() > 1,
        TableView::Unavailable { .. } => false,
    }
}

/// Whether an hour of the first `blocks` held more than one human; stops at the first.
pub fn multiplayer(replay: &Replay, blocks: usize) -> io::Result<bool> {
    for block in replay.index.iter().take(blocks) {
        let mut decoder = FrameDecoder::for_tables(&[TableId::Players]);
        for frame in replay.read_block_selected(block, &[TableId::Players])? {
            if more_than_one_player(decoder.advance(&frame)?) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A finished recording's name; `multiplayer` comes from its hours as they were saved.
pub fn recording_name(
    replay: &Replay,
    localisation: Option<&Localisation>,
    multiplayer: bool,
) -> io::Result<RecordingName> {
    let span = replay
        .index
        .iter()
        .map(|block| (block.first_hour, block.last_hour))
        .reduce(|(first, last), (low, high)| (first.min(low), last.max(high)));
    // Of several branches reaching the latest hour, the last written.
    let country = match replay.index.iter().max_by_key(|block| block.last_hour) {
        Some(block) => {
            match replay.seek_selected(block.branch, block.last_hour, Some(&country_tables()))? {
                SeekResult::Frame(frame) => {
                    Country::of(&frame).and_then(|country| country.name(localisation))
                }
                SeekResult::Gap { .. } => None,
            }
        }
        None => None,
    };
    Ok(RecordingName {
        country,
        years: span.and_then(years),
        multiplayer,
    })
}

/// A growing spool's name: reading notes span, roster and latest frames; the country decodes when asked.
#[derive(Default)]
pub struct SpoolName {
    span: Option<(Hour, Hour)>,
    players: Option<FrameDecoder>,
    multiplayer: bool,
    /// The latest hour's branch and its frames since their keyframe, by segment.
    branch: Option<BranchId>,
    frames: Vec<(u32, Placement)>,
    /// Decoded through `frames[..done]`, and the country at the last of them.
    decoded: Option<(FrameDecoder, usize)>,
    country: Option<Country>,
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
            Ok(decoded) => self.multiplayer |= more_than_one_player(decoded),
            Err(_) => self.players = None,
        }
        if !latest {
            return;
        }
        if frame.keyframe || self.branch != Some(frame.branch) {
            self.frames.clear();
            self.decoded = None;
            self.country = None;
            // Only a keyframe starts frames that can be decoded.
            self.branch = frame.keyframe.then_some(frame.branch);
        }
        if self.branch.is_some() {
            self.frames.push((segment, placement));
        }
    }

    /// Decodes the latest hour's frames not decoded yet; `None` before the first frame.
    pub fn name(
        &mut self,
        mut read: impl FnMut(u32, Placement) -> io::Result<Frame>,
        localisation: Option<&Localisation>,
    ) -> Option<RecordingName> {
        let span = self.span?;
        if !self.frames.is_empty() {
            let (decoder, done) = self
                .decoded
                .get_or_insert_with(|| (FrameDecoder::for_tables(&country_tables()), 0));
            while *done < self.frames.len() {
                let (segment, placement) = self.frames[*done];
                *done += 1;
                match read(segment, placement).and_then(|frame| {
                    decoder
                        .advance(&frame)
                        .map(|decoded| (*done == self.frames.len()).then(|| Country::of(decoded)))
                }) {
                    Ok(Some(country)) => self.country = country,
                    Ok(None) => {}
                    Err(_) => {
                        self.frames.clear();
                        self.decoded = None;
                        self.country = None;
                        break;
                    }
                }
            }
        }
        Some(RecordingName {
            country: self
                .country
                .as_ref()
                .and_then(|country| country.name(localisation)),
            years: years(span),
            multiplayer: self.multiplayer,
        })
    }
}
