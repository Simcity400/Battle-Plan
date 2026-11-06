//! Game score facts, projected hour by hour from a columnar walk ([`replay::segments`]): the
//! score observer ([`ScoreObserver`]) reads the walk's frame of the score tables and each
//! hour's changed rows, and computes again only the countries those rows reach. An hour that
//! changes no field the score reads reads nothing and carries nothing but its hour. What a
//! country held before the segment is the ordered emit's ([`super::emit`]): every segment
//! starts cold, its first hour read whole.

use super::{Change, Edits, GameScoreIdentity, GameScoreMissingKind, GameScoreValues, SCORE_TABLES};
use super::{ScoreContent, ScoreObserver};
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{format::FormatVersion, rows::TableId};
use std::io;

/// The tables a projection reads: the score's, and slot zero's naming the hourly walk read too.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryGameScoreInputs,
    TableId::AirWingGameScoreStats,
    TableId::GameScoreMultipliers,
    TableId::CountryFactories,
    TableId::AirBaseRegistrations,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LexicalTokens,
    TableId::ReservedCountryNaming,
];

/// The fields the score, its names, colours and factions read of the tables it holds by fields;
/// every other table is held whole.
const FIELDS: [(TableId, &[&str]); 8] = [
    (TableId::Countries, &["canonical_slot", "tag"]),
    (
        TableId::CountryFactories,
        &[
            "naval_total_raw",
            "civilian_total_raw",
            "military_total_raw",
            "civilian_to_others",
            "civilian_to_overlord",
            "military_to_others",
            "military_to_overlord",
        ],
    ),
    (
        TableId::CountryAppearance,
        &[
            "red",
            "green",
            "blue",
            "cosmetic_tag",
            "name_tag",
            "leader_ideology",
            "overlord",
        ],
    ),
    (TableId::Politics, &["ruling_party"]),
    (TableId::PoliticalParties, &["token"]),
    (TableId::Autonomy, &["state"]),
    (TableId::Factions, &["name", "template_token"]),
    (
        TableId::ReservedCountryNaming,
        &[
            "tag",
            "name_tag",
            "cosmetic_tag",
            "leader_ideology",
            "group",
            "overlord",
            "autonomy",
            "built",
        ],
    ),
];

/// How the walk's frame holds each table a projection reads, and the fields a change to which
/// it reads again (every field for a table held whole).
fn asks() -> io::Result<Vec<(TableId, Hold, u64)>> {
    let mut asks = Vec::with_capacity(TABLES.len());
    for &table in TABLES {
        if table == TableId::Campaign {
            continue;
        }
        let hold = match FIELDS.iter().find(|(held, _)| *held == table) {
            Some((_, names)) => {
                let mut mask = 0_u64;
                for name in *names {
                    mask |= 1_u64 << columns::field(table, name)?;
                }
                Hold::Fields(mask).of(table)
            }
            None => Hold::Whole,
        };
        let fields = match hold {
            Hold::Whole => u64::MAX,
            Hold::Fields(fields) => fields,
        };
        asks.push((table, hold, fields));
    }
    Ok(asks)
}

/// What a projection reads of its walk's world: the frame of its tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: asks()?
            .into_iter()
            .map(|(table, hold, _)| (table, hold))
            .collect(),
        ..Needs::default()
    })
}

/// A country's facts at an hour: its tag, score values and identity.
pub type Country = (String, GameScoreValues, GameScoreIdentity);

/// A warning's key: its kind and reason.
pub type Warning = (GameScoreMissingKind, String);

/// What the ordered emit reads of an hour that changed the facts.
#[derive(replay_spool::Spooled)]
pub enum Facts {
    /// Every country was read again: the multipliers, every country in tag order and every
    /// warning with its count this hour.
    Whole {
        multipliers: GameScoreValues,
        countries: Vec<Country>,
        warnings: Vec<(Warning, u64)>,
    },
    /// Only these countries' facts and these warnings' counts changed, none for a warning no
    /// longer counted; the countries present and the multipliers are the hour before's.
    Countries {
        countries: Vec<Country>,
        warnings: Vec<(Warning, Option<u64>)>,
    },
}

/// One hour of a segment: none where it changed nothing the facts read.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub facts: Option<Box<Facts>>,
}

/// One segment's projection: every segment starts cold.
pub struct Projector<'c> {
    content: &'c ScoreContent,
    format: FormatVersion,
    asks: Vec<(TableId, Hold)>,
    /// Each table read and the fields a change to which is read.
    fields: Vec<(TableId, u64)>,
    observer: ScoreObserver<'c>,
    edits: Edits,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"game score projector: cold";

impl<'c> Projector<'c> {
    pub fn new(content: &'c ScoreContent, format: FormatVersion) -> io::Result<Self> {
        let asks = asks()?;
        Ok(Self {
            content,
            format,
            fields: asks.iter().map(|&(table, _, fields)| (table, fields)).collect(),
            asks: asks.into_iter().map(|(table, hold, _)| (table, hold)).collect(),
            observer: content.observer(),
            edits: Edits::new(),
            cold: true,
            last: None,
        })
    }

    /// The hour's changes to every score table: a table is exact where it is complete and was
    /// the hour before, its changes the walker's for it.
    fn edits(&mut self, state: &State, world: &World, opens: bool) -> io::Result<()> {
        for (&table, edited) in SCORE_TABLES.iter().zip(&mut self.edits.tables) {
            edited.changed.clear();
            edited.deleted.clear();
            if table == TableId::Campaign {
                edited.exact = true;
                edited.moved = false;
                continue;
            }
            let restated = opens || world.restated(table);
            edited.exact = !restated && columns::complete(state, table);
            edited.moved = restated || world.touched(table, u64::MAX);
            if !edited.exact || !edited.moved {
                continue;
            }
            let changes = columns::changes(state, table)?;
            let held = columns::table(state, table)?;
            edited.changed.reserve(
                changes.changed.len() + changes.fill_slots.len() + changes.inserted.len(),
            );
            // A new row writes every field.
            let written = changes
                .inserted
                .iter()
                .map(|&slot| (slot, u64::MAX))
                .chain(changes.changed.iter().copied().zip(changes.masks.iter().copied()))
                .chain(
                    changes
                        .fill_slots
                        .iter()
                        .copied()
                        .zip(changes.fill_masks.iter().copied()),
                );
            for (slot, mask) in written {
                if let Some(key) = held.key(slot) {
                    edited.changed.push((key, mask));
                }
            }
            edited.deleted.extend_from_slice(&changes.deleted_keys);
        }
        Ok(())
    }
}

impl Operator<World> for Projector<'_> {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("game_score.columnar", Some("game_score"));
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.observer = self.content.observer();
        }
        self.cold = false;
        self.last = Some(at);
        // Names reaching slot zero name their hour: any change to a score table reads it whole.
        let hourly = self.observer.names_hourly();
        let moved = opens
            || if hourly {
                SCORE_TABLES
                    .iter()
                    .any(|&table| world.touched(table, u64::MAX))
            } else {
                self.fields
                    .iter()
                    .any(|&(table, fields)| world.touched(table, fields))
            };
        if !moved && self.observer.advance(at) {
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: None,
            });
        }
        self.edits(state, world, opens)?;
        let frame = {
            let _cpu = replay_frame::cpu::scope("game_score.columnar.frame", None);
            world.frame_held(state, &self.asks)?
        };
        let content = self.content;
        let multipliers = &content.multipliers;
        let (facts, change) = if hourly && !opens {
            let facts =
                self.observer
                    .observe_whole(&frame, self.format, &mut || multipliers.clone())?;
            (facts, Change::All)
        } else {
            self.observer.observe_changed(
                &frame,
                self.format,
                &mut || multipliers.clone(),
                Some(&self.edits),
            )?
        };
        let facts = match change {
            Change::Same => None,
            Change::All => Some(Box::new(Facts::Whole {
                multipliers: facts.multipliers.clone(),
                countries: facts
                    .countries
                    .iter()
                    .map(|(tag, (values, identity))| (tag.clone(), values.clone(), identity.clone()))
                    .collect(),
                warnings: facts
                    .warnings
                    .iter()
                    .map(|(key, count)| (key.clone(), *count))
                    .collect(),
            })),
            Change::Countries { tags, warnings } if tags.is_empty() && warnings.is_empty() => None,
            Change::Countries { tags, warnings } => {
                let mut countries = Vec::with_capacity(tags.len());
                for tag in tags {
                    let (values, identity) = facts.countries.get(&tag).ok_or_else(|| {
                        io::Error::other("A changed score country is not observed")
                    })?;
                    countries.push((tag, values.clone(), identity.clone()));
                }
                countries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                let warnings = warnings
                    .into_iter()
                    .map(|key| {
                        let count = facts.warnings.get(&key).copied();
                        (key, count)
                    })
                    .collect();
                Some(Box::new(Facts::Countries {
                    countries,
                    warnings,
                }))
            }
        };
        // The walk's views go back before the hour ends; the observer keeps its own.
        drop(frame);
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts,
        })
    }

    // What it held is dropped at the segment's end: the next starts cold.
    fn end(&mut self, _: &mut dyn Sink<Hour>) -> io::Result<()> {
        self.observer = self.content.observer();
        self.cold = true;
        self.last = None;
        Ok(())
    }

    fn carried(&self) -> Option<usize> {
        Some(0)
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved game score projector",
            ));
        }
        self.cold = true;
        self.last = None;
        self.observer = self.content.observer();
        Ok(())
    }
}
