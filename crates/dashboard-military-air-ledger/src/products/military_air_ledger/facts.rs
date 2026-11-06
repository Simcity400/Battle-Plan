//! Military air ledger facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! for each family (air losses, bombing), whether its event table was complete, and its
//! events, already resolved through the hour's country join and summed by leaf. A complete
//! event table holds the events its read delivered, so an hour with none costs nothing. The
//! join ([`dashboard_world`]) is sent where it changed. What a read means over the branch
//! (event ordinals, identity epochs, presence and counts) is the ordered emit's
//! ([`super::emit`]).

use super::LeafKey;
use dashboard_world::{Needs, Roster, Tagged, World};
use replay::{
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        AirLossEventRow, BuildingBombingDamageCause, BuildingBombingDamageRow, LexicalTokenRow,
        TableId,
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::AirLossEvents,
    TableId::BuildingBombingDamage,
];

/// The event table of each family, by [`super::MilitaryAirFamily`] order.
pub const EVENTS: [TableId; 2] = [TableId::AirLossEvents, TableId::BuildingBombingDamage];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// The hour's country join, as the row ledger reads it: each tag the links resolve, in tag
/// order, until the first whose tag is refused.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Identities {
    /// Each resolved country (tag, "slot:id"), in tag order.
    pub countries: Vec<(String, String)>,
    /// Each raw link id's country, by position in `countries`.
    pub by_id: HashMap<u32, usize>,
    /// Resolved tags that are not UTF-8, before any refused tag.
    pub invalid_tags: usize,
    /// A tag was refused: the hour reads nothing, but the countries before it are present.
    pub refused: bool,
}

impl Identities {
    fn heap(&self) -> usize {
        self.countries
            .iter()
            .map(|(tag, identity)| size_of::<(String, String)>() + tag.len() + identity.len())
            .sum::<usize>()
            + self.by_id.capacity() * size_of::<(u32, usize)>()
    }

    fn country(&self, raw: Option<u32>) -> Option<&(String, String)> {
        raw.filter(|raw| *raw != 0)
            .and_then(|raw| self.by_id.get(&raw))
            .map(|&at| &self.countries[at])
    }
}

/// A read with events: what its ordinals and dates check, the counts it notes and its sums.
#[derive(Debug, Default, PartialEq, Eq, replay_spool::Spooled)]
pub struct Read {
    /// The first and last event ordinal, and the earliest native date.
    pub first: u64,
    pub last: u64,
    pub earliest: i32,
    /// The first event, by ordinal, that refuses the read: dated after it, or a sum overflow.
    pub refused: Option<String>,
    /// Counts by reason.
    pub notes: Vec<(String, usize)>,
    /// A caller whose membership in aircraft bombing is unknown.
    pub incomplete: bool,
    /// Each leaf's sum at each native date.
    pub sums: Vec<(LeafKey, i32, i128)>,
}

impl Read {
    fn heap(&self) -> usize {
        self.notes
            .iter()
            .map(|(reason, _)| size_of::<(String, usize)>() + reason.capacity())
            .sum::<usize>()
            + self.sums.capacity() * size_of::<(LeafKey, i32, i128)>()
            + self.refused.as_ref().map_or(0, String::capacity)
    }
}

/// What one family's event table gave the hour.
#[derive(Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Events {
    /// The table was not complete.
    Unread,
    /// Complete, while the countries or their links were not.
    Unjoined,
    /// Complete and joined: its read, none when it held no events.
    Read(Option<Box<Read>>),
}

/// One projected hour.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// The join, at a segment's first hour and where it changed.
    pub identities: Option<Arc<Identities>>,
    pub families: [Events; 2],
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    literal: usize,
    cold: bool,
    last: Option<i32>,
    identities: Option<Arc<Identities>>,
}

/// What [`Projector::save`] writes: nothing but that it restarts.
const SAVED: &[u8] = b"air ledger projector: cold";

/// The countries the roster resolves, as the row ledger's join reads them.
fn identities(roster: &Roster) -> Identities {
    let mut tagged: Vec<(&[u8], (u32, u32))> = roster
        .held()
        .filter_map(|tag| match roster.tagged(tag) {
            Tagged::Found(country) => Some((roster.tag(tag), (country.slot, country.id))),
            _ => None,
        })
        .collect();
    tagged.sort_unstable_by(|left, right| left.0.cmp(right.0));
    let mut countries = Vec::with_capacity(tagged.len());
    let mut slots: BTreeMap<u32, usize> = BTreeMap::new();
    let mut invalid_tags = 0;
    let mut refused = false;
    for (bytes, (slot, id)) in tagged {
        let Ok(tag) = std::str::from_utf8(bytes) else {
            invalid_tags += 1;
            continue;
        };
        if dashboard_core::protocol::validate_tag(tag).is_err() {
            refused = true;
            break;
        }
        slots.insert(slot, countries.len());
        countries.push((tag.to_owned(), format!("{slot}:{id}")));
    }
    let by_id = roster
        .links()
        .iter()
        .filter_map(|link| {
            slots
                .get(&link.canonical_slot)
                .map(|&country| (link.id, country))
        })
        .collect();
    Identities {
        countries,
        by_id,
        invalid_tags,
        refused,
    }
}

/// Counts a reason.
fn note(notes: &mut BTreeMap<String, usize>, reason: impl Into<String>) {
    *notes.entry(reason.into()).or_default() += 1;
}

/// Sums `amount` into `leaf` at `hour`; the overflow refuses at its event.
fn add(
    sums: &mut BTreeMap<(LeafKey, i32), i128>,
    refused: &mut Option<String>,
    leaf: LeafKey,
    hour: i32,
    amount: i64,
) {
    let sum = sums.entry((leaf, hour)).or_default();
    match sum.checked_add(i128::from(amount)) {
        Some(total) => *sum = total,
        None => {
            refused.get_or_insert_with(|| "military air event sum overflow".into());
        }
    }
}

impl Projector {
    pub fn new(format: FormatVersion) -> io::Result<Self> {
        Ok(Self {
            format,
            literal: columns::field(TableId::LexicalTokens, "literal")?,
            cold: true,
            last: None,
            identities: None,
        })
    }

    /// The live rows of `table`, by ordinal.
    fn rows<R: replay_data::rows::RowCodec>(
        &self,
        state: &State,
        table: TableId,
    ) -> io::Result<Vec<R>> {
        let held = columns::table(state, table)?;
        let mut slots: Vec<(u128, u32)> = held
            .live()
            .ones()
            .filter_map(|slot| held.key(slot).map(|key| (key, slot)))
            .collect();
        slots.sort_unstable();
        slots
            .into_iter()
            .map(|(_, slot)| columns::row(state, table, slot, self.format.version()))
            .collect()
    }

    /// The spelling of a building type, as the row ledger reads it: none where the tokens are
    /// not complete, the token is absent, empty or not UTF-8.
    fn token(&self, state: &State, token: u32) -> io::Result<Option<String>> {
        let table = TableId::LexicalTokens;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(token)) else {
            return Ok(None);
        };
        let literal = if held.recorded(self.literal, slot) {
            let text = *columns::text(held, self.literal)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?
                .to_vec()
        } else {
            columns::row::<LexicalTokenRow>(state, table, slot, self.format.version())?.literal
        };
        Ok(String::from_utf8(literal)
            .ok()
            .filter(|token| !token.is_empty()))
    }

    fn air(
        &self,
        state: &State,
        identities: &Identities,
        hour: i32,
    ) -> io::Result<Option<Box<Read>>> {
        let rows: Vec<AirLossEventRow> = self.rows(state, TableId::AirLossEvents)?;
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return Ok(None);
        };
        let mut read = Read {
            first: first.id,
            last: last.id,
            earliest: rows
                .iter()
                .map(|row| row.game_hour)
                .min()
                .expect("an event"),
            ..Read::default()
        };
        let mut notes = BTreeMap::new();
        let mut sums = BTreeMap::new();
        for row in &rows {
            if row.game_hour > hour {
                read.refused
                    .get_or_insert_with(|| "air event is dated after its supplying read".into());
                break;
            }
            let country = identities.country(Some(row.country)).cloned();
            if country.is_none() {
                note(
                    &mut notes,
                    "air event endpoint is not resolved by its recorded country census",
                );
            }
            if !(1..=6).contains(&row.native_cause) {
                note(
                    &mut notes,
                    format!(
                        "air loss cause {} has no proved display meaning; its exact amount is retained",
                        row.native_cause
                    ),
                );
            }
            let opponent = identities
                .country(row.opponent)
                .map(|country| country.0.clone());
            add(
                &mut sums,
                &mut read.refused,
                LeafKey {
                    country,
                    dealt: row.credited_enemy,
                    opponent,
                    region: Some(row.region),
                    building: None,
                    state: None,
                    cause: row.native_cause,
                },
                row.game_hour,
                row.amount_raw,
            );
            if read.refused.is_some() {
                break;
            }
        }
        read.notes = notes.into_iter().collect();
        read.sums = sums
            .into_iter()
            .map(|((leaf, hour), sum)| (leaf, hour, sum))
            .collect();
        Ok(Some(Box::new(read)))
    }

    fn bombing(
        &self,
        state: &State,
        identities: &Identities,
        hour: i32,
    ) -> io::Result<Option<Box<Read>>> {
        let rows: Vec<BuildingBombingDamageRow> =
            self.rows(state, TableId::BuildingBombingDamage)?;
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return Ok(None);
        };
        let mut read = Read {
            first: first.id,
            last: last.id,
            earliest: rows
                .iter()
                .map(|row| row.game_hour)
                .min()
                .expect("an event"),
            incomplete: rows
                .iter()
                .any(|row| row.cause == BuildingBombingDamageCause::Unknown),
            ..Read::default()
        };
        let mut notes = BTreeMap::new();
        let mut sums = BTreeMap::new();
        let mut tokens: HashMap<u32, Option<String>> = HashMap::new();
        for row in &rows {
            if row.cause == BuildingBombingDamageCause::Unknown {
                note(
                    &mut notes,
                    "building damage has an unsupported caller; its contribution to aircraft bombing is not known",
                );
                continue;
            }
            if !matches!(
                row.cause,
                BuildingBombingDamageCause::AirMission | BuildingBombingDamageCause::AirNaval
            ) {
                continue;
            }
            if row.game_hour > hour {
                read.refused.get_or_insert_with(|| {
                    "bombing event is dated after its supplying read".into()
                });
                break;
            }
            let token = match tokens.get(&row.building_type) {
                Some(token) => token.clone(),
                None => {
                    let token = self.token(state, row.building_type)?;
                    tokens.insert(row.building_type, token.clone());
                    token
                }
            };
            if token.is_none() {
                note(
                    &mut notes,
                    "building type is not spelled by the event's recorded lexical dictionary",
                );
            }
            for (dealt, raw) in [(true, row.attacker), (false, Some(row.victim))] {
                let country = identities.country(raw).cloned();
                if country.is_none() {
                    note(
                        &mut notes,
                        if dealt {
                            "bombing attacker not recorded"
                        } else {
                            "bombing victim not recorded"
                        },
                    );
                }
                add(
                    &mut sums,
                    &mut read.refused,
                    LeafKey {
                        country,
                        dealt,
                        opponent: None,
                        region: None,
                        building: Some((row.building_type, token.clone())),
                        state: Some(row.state),
                        cause: row.cause as u32,
                    },
                    row.game_hour,
                    row.amount_raw,
                );
            }
            if read.refused.is_some() {
                break;
            }
        }
        read.notes = notes.into_iter().collect();
        read.sums = sums
            .into_iter()
            .map(|((leaf, hour), sum)| (leaf, hour, sum))
            .collect();
        Ok(Some(Box::new(read)))
    }
}

impl Operator<World> for Projector {
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
        let _cpu = replay_frame::cpu::scope("air_ledger.columnar", Some("air_ledger"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = std::mem::take(&mut self.cold)
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.last = Some(at);
        let joined = roster.complete();
        let mut sent = None;
        if !joined {
            self.identities = None;
        } else if opens || roster.changed() || self.identities.is_none() {
            let read = identities(roster);
            let held = match &self.identities {
                Some(held) if !opens && **held == read => Arc::clone(held),
                _ => Arc::new(read),
            };
            if opens
                || !self
                    .identities
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, &held))
            {
                sent = Some(Arc::clone(&held));
            }
            self.identities = Some(held);
        }
        let mut families = [Events::Unread, Events::Unread];
        for (at_family, table) in EVENTS.into_iter().enumerate() {
            families[at_family] = if !columns::complete(state, table) {
                Events::Unread
            } else if let Some(identities) = self.identities.as_ref().filter(|_| joined) {
                if identities.refused {
                    Events::Read(None)
                } else if at_family == 0 {
                    Events::Read(self.air(state, identities, at)?)
                } else {
                    Events::Read(self.bombing(state, identities, at)?)
                }
            } else {
                Events::Unjoined
            };
        }
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            identities: sent,
            families,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(
            self.identities
                .as_ref()
                .map_or(0, |identities| size_of::<Identities>() + identities.heap()),
        )
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved air ledger projector",
            ));
        }
        *self = Self::new(self.format)?;
        Ok(())
    }
}
