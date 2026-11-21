use super::*;

pub const TAKEN: u8 = 0;
pub const REENABLE: u8 = 1;
pub const REMOVAL: u8 = 2;
pub const TIMED: u8 = 3;
pub const TARGETED: u8 = 4;
pub const TARGETED_TIMED: u8 = 5;
pub const CONTAINERS: usize = 6;
pub const PERMANENT: u8 = 1;
pub const IGNORED: u8 = 2;
pub const REMOVE_REQUESTED: u8 = 4;
pub const SWAP: u8 = 8;
pub const ALL: u8 = 16;
pub const MOVED: u8 = 32;
pub const ORDINAL: u8 = 64;
pub const ITERATION: u8 = 128;
pub const CLEAR_ALL_CANDIDATES: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Operation {
    Baseline = 0,
    Ready = 1,
    Invalidate = 2,
    Store = 3,
    Erase = 4,
    Update = 5,
    PassBegin = 6,
    PassCut = 7,
    PassEnd = 8,
    RandomSet = 9,
    RandomIncrement = 10,
    Rebuild = 11,
    Candidate = 12,
    Ignore = 13,
    Refused = 14,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DecisionChange {
    pub operation: Operation,
    pub container: u8,
    pub flags: u8,
    pub ordinal: u32,
    pub political_power: Option<i64>,
}
impl DecisionChange {
    pub fn validate(self, script: EventFeedScript) -> io::Result<()> {
        use Operation::*;
        if self.political_power.is_some() && (self.operation != Rebuild || self.container != 3) {
            return Err(invalid("political balance outside final decision rebuild"));
        }
        let member = matches!(self.operation, Store | Erase | Update | PassCut)
            || self.operation == Ignore && script.instance.is_some();
        if member != script.instance.is_some()
            || script.option.is_some()
            || script.native_event.is_some()
            || script.scope_len != 0
            || script.scope_unrecorded
        {
            return Err(invalid("decision change operands differ"));
        }
        if self.container as usize >= CONTAINERS
            || self.operation == Candidate && self.container >= 4
        {
            return Err(invalid("decision change container differs"));
        }
        if self.operation == RandomSet && script.days.is_none() {
            return Err(invalid("decision random baseline value absent"));
        }
        if self.operation == Store && script.instance == Some(0) {
            return Err(invalid("decision stored compact identity is zero"));
        }
        if self.operation == Refused
            && (script.token_len == 0
                || script.token_unrecorded
                || self.container != 0
                || self.flags != 0
                || self.ordinal != 0)
        {
            return Err(invalid("decision owner refusal operands differ"));
        }
        Ok(())
    }

    pub const fn new(operation: Operation, container: u8) -> Self {
        Self {
            operation,
            container,
            flags: 0,
            ordinal: 0,
            political_power: None,
        }
    }
}
impl Value for DecisionChange {
    const MINIMUM_ENCODED_BYTES: usize = 4;
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(3 + compact_len(self.ordinal)
            + if self.operation == Operation::Rebuild {
                self.political_power.encoded_bytes()?
            } else {
                0
            })
    }
    fn encode_reserved(&self, out: &mut Vec<u8>) -> io::Result<()> {
        (self.operation as u8).encode_reserved(out)?;
        self.container.encode_reserved(out)?;
        self.flags.encode_reserved(out)?;
        compact_write(self.ordinal, out);
        if self.operation == Operation::Rebuild {
            self.political_power.encode_reserved(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let operation = Operation::of(u8::decode(input)?)?;
        Ok(Self {
            operation,
            container: u8::decode(input)?,
            flags: u8::decode(input)?,
            ordinal: compact_read(input)?,
            political_power: if operation == Operation::Rebuild {
                Option::<i64>::decode(input)?
            } else {
                None
            },
        })
    }
}
impl Operation {
    pub fn of(code: u8) -> io::Result<Self> {
        use Operation::*;
        Ok(match code {
            0 => Baseline,
            1 => Ready,
            2 => Invalidate,
            3 => Store,
            4 => Erase,
            5 => Update,
            6 => PassBegin,
            7 => PassCut,
            8 => PassEnd,
            9 => RandomSet,
            10 => RandomIncrement,
            11 => Rebuild,
            12 => Candidate,
            13 => Ignore,
            14 => Refused,
            _ => return Err(invalid("unknown retained decision operation")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetainedDecision {
    pub instance: u32,
    pub token: Vec<u8>,
    pub days: i32,
    pub lifecycle: i32,
    pub flags: u8,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Pass {
    cursor: usize,
    limit: usize,
    dynamic: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetainedRandomCount {
    pub token: Vec<u8>,
    pub target: i32,
    pub count: i32,
}
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DecisionState {
    pub country: EventFeedCountry,
    members: [Vec<RetainedDecision>; CONTAINERS],
    pub random_counts: Vec<RetainedRandomCount>,
    pub candidates: BTreeSet<(u8, Vec<u8>, u32, u32)>,
    pub ignored: BTreeSet<Vec<u8>>,
    pub target_countries: BTreeMap<u32, EventFeedCountry>,
    ready: bool,
    building: bool,
    baseline_hour: Option<i32>,
    passes: [Vec<Pass>; CONTAINERS],
    #[serde(skip)]
    pub(super) version: super::state::Version,
}
impl DecisionState {
    pub fn version(&self) -> Option<u64> {
        self.version.get()
    }
    pub fn members(&self, container: u8) -> Result<&[RetainedDecision], &'static str> {
        if !self.ready || self.passes.iter().any(|passes| !passes.is_empty()) {
            return Err("decision owner has no completed baseline and writer/pass certificate");
        }
        self.members
            .get(container as usize)
            .map(Vec::as_slice)
            .ok_or("decision container differs")
    }
    pub fn complete(&self) -> bool {
        self.ready && self.passes.iter().all(Vec::is_empty)
    }
    pub fn gap(&mut self) {
        self.ready = false;
        for passes in &mut self.passes {
            passes.clear();
        }
    }
    pub fn partial(&mut self, hour: i32) {
        if self.baseline_hour != Some(hour) {
            self.gap();
        }
    }
    fn decrement(member: &mut RetainedDecision, container: u8) {
        let decrement = match container {
            REENABLE => member.flags & PERMANENT == 0,
            REMOVAL => true,
            TIMED => match member.lifecycle {
                0 | 2 => true,
                1 => member.days >= 0,
                4 => member.flags & PERMANENT == 0,
                _ => false,
            },
            TARGETED => match member.lifecycle {
                1 => member.days >= 0,
                2 => member.flags & PERMANENT == 0,
                _ => false,
            },
            TARGETED_TIMED => match member.lifecycle {
                0 | 3 => true,
                1 => member.days >= 0,
                2 => member.flags & PERMANENT == 0,
                _ => false,
            },
            _ => false,
        };
        if decrement {
            member.days = member.days.wrapping_sub(1);
        }
    }
    fn advance(&mut self, container: u8, end: usize) -> io::Result<()> {
        let n = container as usize;
        let Some(pass) = self.passes.get_mut(n).and_then(|passes| passes.last_mut()) else {
            return Ok(());
        };
        if end < pass.cursor {
            return Ok(());
        }
        if end > self.members[n].len() || !pass.dynamic && end > pass.limit {
            self.gap();
            return Ok(());
        }
        for member in &mut self.members[n][pass.cursor..end] {
            Self::decrement(member, container);
        }
        pass.cursor = end;
        Ok(())
    }
    fn erase_at(&mut self, n: usize, at: usize, swap: bool, iteration: bool) {
        self.version.change();
        if swap {
            self.members[n].swap_remove(at);
        } else {
            self.members[n].remove(at);
        }
        if let Some(pass) = self.passes[n].last_mut().filter(|_| iteration) {
            if at < pass.cursor {
                pass.cursor -= 1;
            }
            if at < pass.limit {
                pass.limit -= 1;
            }
        }
    }
    pub fn observe(
        &mut self,
        row: &EventFeedHistoryRow,
        event: &EventFeedEvent,
        script: EventFeedScript,
        change: DecisionChange,
    ) -> io::Result<()> {
        let occurrence = super::state::Change::of(row, event);
        self.apply(row.read_hour, occurrence, script, change, true, false)
    }
    pub fn apply(
        &mut self,
        read_hour: i32,
        occurrence: super::state::Change<'_>,
        script: EventFeedScript,
        change: DecisionChange,
        write: bool,
        cancelled: bool,
    ) -> io::Result<()> {
        use Operation::*;
        let n = change.container as usize;
        let event = occurrence.event;
        let token = occurrence.token(script);
        if self.country != event.actor {
            self.country = event.actor;
            self.version.change();
        }
        if script.token_unrecorded {
            self.gap();
            return Ok(());
        }
        if write && event.target.reference as i32 > 0 {
            self.target_countries
                .insert(event.target.reference, event.target);
        }
        match change.operation {
            Baseline => {
                *self = Self {
                    country: event.actor,
                    building: true,
                    ..Self::default()
                };
            }
            Ready => {
                if !self.building {
                    return Err(invalid("decision ready has no baseline"));
                }
                self.building = false;
                self.ready = true;
                self.baseline_hour = Some(read_hour);
            }
            Invalidate => self.gap(),
            Refused => {
                self.gap();
                self.building = false;
            }
            RandomSet => {
                self.version.change();
                self.random_counts.push(RetainedRandomCount {
                    token: token.to_vec(),
                    target: change.ordinal as i32,
                    count: script
                        .days
                        .ok_or_else(|| invalid("decision random baseline value absent"))?,
                });
            }
            RandomIncrement => {
                self.version.change();
                let target = change.ordinal as i32;
                if let Some(value) = self
                    .random_counts
                    .iter_mut()
                    .find(|value| value.token.eq_ignore_ascii_case(token) && value.target == target)
                {
                    value.count = value.count.wrapping_add(1);
                } else {
                    self.random_counts.push(RetainedRandomCount {
                        token: token.to_vec(),
                        target,
                        count: 1,
                    });
                }
            }
            Rebuild => {}
            Candidate if !cancelled => self.candidate(token, event, script, change),
            Candidate => {}
            Ignore if script.instance.is_none() => {
                let changed = if change.flags & IGNORED != 0 {
                    self.ignored.insert(token.to_vec())
                } else {
                    self.ignored.remove(token)
                };
                if changed {
                    self.version.change();
                }
            }
            PassBegin => {
                if n >= CONTAINERS || self.ready && self.members[n].len() != change.ordinal as usize
                {
                    self.gap();
                    return Ok(());
                }
                self.passes[n].push(Pass {
                    cursor: 0,
                    limit: change.ordinal as usize,
                    dynamic: change.container == REMOVAL,
                });
            }
            PassEnd => {
                if n >= CONTAINERS {
                    return Err(invalid("decision pass container differs"));
                }
                if let Some(pass) = self.passes[n].last().copied() {
                    let end = if pass.dynamic {
                        self.members[n].len()
                    } else {
                        pass.limit
                    };
                    self.advance(change.container, end)?;
                    self.passes[n].pop();
                }
            }
            Store | Erase | Update | PassCut | Ignore => {
                if n >= CONTAINERS {
                    return Err(invalid("decision member container differs"));
                }
                let id = script
                    .instance
                    .ok_or_else(|| invalid("decision member identity absent"))?;
                if change.operation == Store {
                    if change.flags & MOVED != 0 {
                        for old in 0..CONTAINERS {
                            if let Some(at) =
                                self.members[old].iter().position(|r| r.instance == id)
                            {
                                self.erase_at(old, at, true, change.flags & ITERATION != 0);
                                break;
                            }
                        }
                    }
                    self.version.change();
                    self.members[n].push(RetainedDecision {
                        instance: id,
                        token: token.to_vec(),
                        days: script.days.unwrap_or(0),
                        lifecycle: script.lifecycle.unwrap_or(0),
                        flags: change.flags & !(SWAP | ALL | MOVED | ORDINAL | ITERATION),
                        country_target: script.country_target,
                        numeric_target: script.numeric_target,
                    });
                } else if change.operation == Erase
                    && n == TAKEN as usize
                    && change.flags & ALL != 0
                {
                    while let Some(at) = self.members[n]
                        .iter()
                        .position(|r| r.token.as_slice() == token)
                    {
                        self.erase_at(
                            n,
                            at,
                            change.flags & SWAP != 0,
                            change.flags & ITERATION != 0,
                        );
                    }
                } else if let Some(at) = if change.flags & ORDINAL != 0 {
                    let at = change.ordinal as usize;
                    (at < self.members[n].len()).then_some(at)
                } else {
                    self.members[n].iter().position(|r| r.instance == id)
                } {
                    if change.operation == PassCut {
                        self.advance(change.container, at)?;
                        if let Some(pass) = self.passes[n].last_mut() {
                            pass.cursor = pass.cursor.max(at + 1);
                        }
                    }
                    if change.operation == Erase {
                        self.erase_at(
                            n,
                            at,
                            change.flags & SWAP != 0,
                            change.flags & ITERATION != 0,
                        );
                    } else {
                        let value = &mut self.members[n][at];
                        let held = (value.lifecycle, value.flags);
                        if let Some(days) = script.days {
                            value.days = days;
                        }
                        if let Some(state) = script.lifecycle {
                            value.lifecycle = state;
                        }
                        if change.operation == Ignore {
                            value.flags = (value.flags & !IGNORED) | (change.flags & IGNORED);
                        } else if change.flags & REMOVE_REQUESTED != 0 {
                            value.flags |= REMOVE_REQUESTED;
                        }
                        if (value.lifecycle, value.flags) != held {
                            self.version.change();
                        }
                    }
                } else if self.ready {
                    self.gap();
                }
            }
        }
        Ok(())
    }
    pub fn candidate(
        &mut self,
        token: &[u8],
        event: &EventFeedEvent,
        script: EventFeedScript,
        change: DecisionChange,
    ) {
        let held = self.candidates.len();
        if change.flags & ALL != 0 {
            if change.flags & CLEAR_ALL_CANDIDATES != 0 {
                self.candidates.clear();
            } else {
                self.candidates
                    .retain(|(kind, _, _, _)| *kind != change.container);
            }
            if self.candidates.len() != held {
                self.version.change();
            }
            return;
        }
        let key = (
            change.container,
            token.to_vec(),
            change.ordinal,
            script.country_target.unwrap_or(event.actor.reference),
        );
        let changed = if change.flags & REMOVE_REQUESTED != 0 {
            self.candidates.remove(&key)
        } else {
            self.candidates.insert(key)
        };
        if changed {
            self.version.change();
        }
    }
}
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;
