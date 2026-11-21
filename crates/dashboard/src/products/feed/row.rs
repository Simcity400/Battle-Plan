
use dashboard_query::log::{Cell, Row};
use replay_data::{
    codec::Decoder,
    event_feed_rows::EventFeedScope,
    rows::{FeedEventRow, TableId, TagHolderRow},
    value::Value,
};
use std::io;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    None,
    Of(u32),
    NotRecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Party {
    pub reference: u32,
    pub id: Option<u32>,
    pub slot: Slot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cause {
    pub route: u8,
    pub within: u8,
    pub conference: Option<u64>,
    pub state: Option<u32>,
    pub ally: Option<Party>,
    pub reason: Option<u32>,
    pub flags: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeedEvent {
    pub generation: u64,
    pub sequence: u64,
    pub hour: i32,
    pub read_hour: i32,
    pub code: u8,
    pub actor: Party,
    pub target: Party,
    pub cause: Option<Cause>,
    pub token: Option<Vec<u8>>,
    pub native_event: Option<(u32, u32)>,
    pub instance: Option<u32>,
    pub option: Option<i32>,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
    pub lifecycle: Option<i32>,
    pub days: Option<i32>,
    pub scopes: Option<Vec<EventFeedScope>>,
    pub opener: Option<u64>,
    pub dated: Option<i32>,
    pub costs: Option<[Option<i64>; 2]>,
    pub coverage: Option<u32>,
    pub outstanding: Option<u64>,
    pub refusals: Option<u32>,
    pub failure: Vec<u8>,
}

impl FeedEvent {
    pub fn derived(&self) -> bool {
        self.generation == FeedEventRow::DERIVED
    }

    pub fn scripted(&self) -> bool {
        (8..=23).contains(&self.code) && self.code != 17
    }

    pub fn id(&self) -> String {
        if self.derived() {
            format!("derived:{}:{}", self.read_hour, self.sequence as u32)
        } else {
            format!("native:{}:{}", self.generation, self.sequence)
        }
    }
}

pub(crate) struct Columns {
    at: [usize; NAMES.len()],
}

const NAMES: [&str; 41] = [
    "hour",
    "read_hour",
    "kind",
    "actor_reference",
    "actor_id",
    "actor_instance",
    "actor_slot",
    "target_reference",
    "target_id",
    "target_instance",
    "target_slot",
    "route",
    "within",
    "conference",
    "state",
    "ally_reference",
    "ally_id",
    "ally_instance",
    "ally_slot",
    "reason",
    "flags",
    "token",
    "native_event_kind",
    "native_event_id",
    "instance",
    "option",
    "country_target",
    "numeric_target",
    "lifecycle",
    "days",
    "scopes",
    "opener",
    "opener_hour",
    "dated",
    "political_power",
    "command_power",
    "coverage",
    "outstanding",
    "refusals",
    "failure",
    "backlog",
];

enum Held<'a> {
    Value(&'a Cell),
    Missing,
    NotRecorded,
}

impl Columns {
    pub fn new() -> io::Result<Self> {
        let mut at = [0; NAMES.len()];
        for (held, name) in at.iter_mut().zip(NAMES) {
            *held = dashboard_declare::field(TableId::FeedEvents, name)?;
        }
        Ok(Self { at })
    }

    fn cell<'r>(&self, row: &'r Row, name: usize) -> Held<'r> {
        match row.cells.get(self.at[name]) {
            Some(Cell::Missing) => Held::Missing,
            Some(Cell::Unrecorded) | None => Held::NotRecorded,
            Some(cell) => Held::Value(cell),
        }
    }

    fn number(&self, row: &Row, name: usize) -> Option<i64> {
        match self.cell(row, name) {
            Held::Value(Cell::Number(value)) => Some(*value),
            _ => None,
        }
    }

    fn optional(&self, row: &Row, name: usize) -> Option<Option<i64>> {
        match self.cell(row, name) {
            Held::Value(Cell::Number(value)) => Some(Some(*value)),
            Held::Missing => Some(None),
            _ => None,
        }
    }

    fn bytes(&self, row: &Row, name: usize) -> Option<Vec<u8>> {
        match self.cell(row, name) {
            Held::Value(Cell::Text(text)) => Some(text.as_bytes().to_vec()),
            Held::Value(Cell::List(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    fn slot(&self, row: &Row, name: usize) -> Slot {
        match self.number(row, name) {
            Some(slot) if slot as u32 == TagHolderRow::NONE => Slot::None,
            Some(slot) => Slot::Of(slot as u32),
            None => Slot::NotRecorded,
        }
    }

    pub fn read(&self, row: &Row) -> io::Result<FeedEvent> {
        let required = |name: usize| self.number(row, name).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("A feed row's {} is not recorded", NAMES[name])));
        let some = |name: usize| self.optional(row, name).flatten();
        let party = |reference: usize, id: usize, slot: usize| -> io::Result<Party> {
            Ok(Party { reference: required(reference)? as u32, id: self.number(row, id).map(|id| id as u32), slot: self.slot(row, slot) })
        };
        let cause = match (self.number(row, 11), self.number(row, 12)) {
            (Some(route), Some(within)) => Some(Cause {
                route: route as u8,
                within: within as u8,
                conference: some(13).map(|conference| conference as u64),
                state: some(14).map(|state| state as u32),
                ally: some(15).map(|reference| Party {
                    reference: reference as u32,
                    id: some(16).map(|id| id as u32),
                    slot: match self.optional(row, 18) {
                        Some(Some(slot)) if slot as u32 == TagHolderRow::NONE => Slot::None,
                        Some(Some(slot)) => Slot::Of(slot as u32),
                        _ => Slot::NotRecorded,
                    },
                }),
                reason: some(19).map(|reason| reason as u32),
                flags: self.number(row, 20).unwrap_or(0) as u8,
            }),
            _ => None,
        };
        let scopes = match self.cell(row, 30) {
            Held::Value(Cell::List(bytes)) => Some(scopes(bytes)?),
            _ => None,
        };
        let costs = match (self.optional(row, 34), self.optional(row, 35)) {
            (Some(political), Some(command)) => Some([political, command]),
            _ => None,
        };
        Ok(FeedEvent {
            generation: (row.key >> 64) as u64,
            sequence: row.key as u64,
            hour: required(0)? as i32,
            read_hour: required(1)? as i32,
            code: required(2)? as u8,
            actor: party(3, 4, 6)?,
            target: party(7, 8, 10)?,
            cause,
            token: self.bytes(row, 21),
            native_event: some(22).zip(some(23)).map(|(kind, id)| (kind as u32, id as u32)),
            instance: some(24).map(|value| value as u32),
            option: some(25).map(|value| value as i32),
            country_target: some(26).map(|value| value as u32),
            numeric_target: some(27).map(|value| value as i32),
            lifecycle: some(28).map(|value| value as i32),
            days: some(29).map(|value| value as i32),
            scopes,
            opener: some(31).map(|value| value as u64),
            dated: some(33).map(|value| value as i32),
            costs,
            coverage: some(36).map(|value| value as u32),
            outstanding: some(37).map(|value| value as u64),
            refusals: some(38).map(|value| value as u32),
            failure: self.bytes(row, 39).unwrap_or_default(),
        })
    }
}

pub(crate) fn scopes(bytes: &[u8]) -> io::Result<Vec<EventFeedScope>> {
    let mut decoder = Decoder::new(bytes);
    let scopes = <Vec<EventFeedScope> as Value>::decode(&mut decoder)?;
    decoder.finish()?;
    Ok(scopes)
}
