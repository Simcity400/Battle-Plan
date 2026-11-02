//! Faction state's emit: every lane an hour's projection ([`super::columnar`]) names as runs on
//! the family's series publication, in the order the hour observes them. A lane is a key of its
//! faction's actor (faction 0 for the accounts' shares and the roster), its kind the measure
//! and its first-seen ordinal the dimension; what it is of and the choices its runs number are
//! its catalogue scalar ([`LANES`]), and each faction's goal lists and labels its book
//! ([`BOOKS`]). Retained owners follow an account while its holder keeps it, and every faction
//! the publication holds but the hour lists nowhere reads as absent.
//!
//! A value after an hour without one starts a segment, as does every value after an hour not
//! walked, which ends every run.

use super::{
    columnar::Hour,
    prepared::{
        AccountOp, AccountRow, Accounts, CHANNELS, ChannelOp, Choice, FactionOps, Ghost, GoalsOp,
        Kind, ManpowerEntry, ManpowerOp, NULL, Owner, Projected,
        goals::{GoalBook, GoalList},
    },
};
use dashboard_series::{
    Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{DefaultHasher, Hash, Hasher},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// Each lane's catalogue, by ordinal: `[Text identity, Text tag, [Unsigned slot, Unsigned id]
/// | Null, Unsigned slot, Unsigned group, [[token, name, reason]…]]`, each text or Null.
pub const LANES: u64 = 1 << 30;
/// Each faction's goal book, by its actor: the book's JSON as text.
pub const BOOKS: u64 = 1 << 31;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

fn invalid() -> io::Error {
    dashboard_base::invalid("Saved faction state emit is not valid")
}

fn digest(choice: &Choice) -> u64 {
    let mut hasher = DefaultHasher::new();
    choice.hash(&mut hasher);
    hasher.finish()
}

/// A lane as its catalogue scalar holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub identity: String,
    pub tag: String,
    /// Its country's canonical slot and id, apart from the selection's slot.
    pub holder: Option<(u32, u32)>,
    pub slot: u32,
    pub group: u32,
    pub(super) choices: Vec<Choice>,
}

fn text(text: &str) -> Scalar {
    Scalar::Text(text.as_bytes().into())
}

fn optional(value: &Option<String>) -> Scalar {
    value.as_deref().map_or(Scalar::Null, text)
}

impl Meta {
    fn scalar(&self) -> Scalar {
        let choices = self
            .choices
            .iter()
            .map(|choice| {
                Scalar::List(vec![
                    optional(&choice.token),
                    optional(&choice.name),
                    optional(&choice.reason),
                ])
            })
            .collect();
        Scalar::List(vec![
            text(&self.identity),
            text(&self.tag),
            self.holder.map_or(Scalar::Null, |(slot, id)| {
                Scalar::List(vec![
                    Scalar::Unsigned(u64::from(slot)),
                    Scalar::Unsigned(u64::from(id)),
                ])
            }),
            Scalar::Unsigned(u64::from(self.slot)),
            Scalar::Unsigned(u64::from(self.group)),
            Scalar::List(choices),
        ])
    }

    /// The lane `scalar` catalogues.
    pub fn read(scalar: Option<&Scalar>) -> io::Result<Self> {
        let malformed = || dashboard_base::invalid("A prepared faction lane is not valid");
        let string = |scalar: &Scalar| -> io::Result<String> {
            String::from_utf8(scalar.text().ok_or_else(malformed)?.to_vec()).map_err(|_| malformed())
        };
        let optional = |scalar: &Scalar| -> io::Result<Option<String>> {
            match scalar {
                Scalar::Null => Ok(None),
                held => string(held).map(Some),
            }
        };
        let number = |scalar: &Scalar| -> io::Result<u32> {
            scalar
                .integer()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(malformed)
        };
        let [identity, tag, holder, slot, group, choices] = scalar.ok_or_else(malformed)?.list()
        else {
            return Err(malformed());
        };
        Ok(Self {
            identity: string(identity)?,
            tag: string(tag)?,
            holder: match holder {
                Scalar::Null => None,
                held => match held.list() {
                    [slot, id] => Some((number(slot)?, number(id)?)),
                    _ => return Err(malformed()),
                },
            },
            slot: number(slot)?,
            group: number(group)?,
            choices: choices
                .list()
                .iter()
                .map(|choice| match choice.list() {
                    [token, name, reason] => Ok(Choice {
                        token: optional(token)?,
                        name: optional(name)?,
                        reason: optional(reason)?,
                    }),
                    _ => Err(malformed()),
                })
                .collect::<io::Result<_>>()?,
        })
    }
}

/// A lane: its key, what it is of, and what its catalogue holds.
struct Lane {
    key: KeyHandle,
    faction: u64,
    kind: Kind,
    meta: Meta,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Every lane, by ordinal, and each one's ordinal by faction, kind and identity.
    lanes: Vec<Lane>,
    named: FastMap<(u64, Kind), FastMap<Box<str>, u32>>,
    /// Each lane's choices by digest: the first-seen number of the first choice of that digest.
    numbered: FastMap<(u32, u64), u32>,
    /// Every faction a lane names but 0.
    factions: BTreeSet<u64>,
    /// The lanes and books changed since the last cut.
    stale: BTreeSet<u32>,
    books: BTreeMap<u64, GoalBook>,
    stale_books: BTreeSet<u64>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// Each account's last member identity and faction, while it keeps it.
    previous_owners: BTreeMap<u32, (String, u64)>,
    /// Each owner's live lanes of a kind: an account's shares, associations and upgrades, a
    /// faction's selections, manpower and goal progress.
    live: FastMap<(u64, Kind), BTreeSet<u32>>,
    /// Each read lane's channel, and whether its faction's channel is known.
    read_lanes: BTreeMap<u32, (usize, bool)>,
    /// The lanes holding an unavailable value or choice: a known one starts a segment.
    unknown_values: BTreeSet<u32>,
    unknown_choices: BTreeSet<u32>,
    /// The factions the last hour witnessed, and what one it lists nowhere reads.
    witnessed: BTreeSet<u64>,
    ghost: Option<Ghost>,
    /// The factions the publication held that the last hour listed nowhere; None until read.
    ghosts: Option<BTreeSet<u64>>,
    /// A faction may have joined the publication, or the witnessed changed, since `ghosts`.
    grew: bool,
    /// Every account a share lane names; None until read.
    share_slots: Option<BTreeSet<u32>>,
    /// Each account's share lane, and each faction's total and manpower lanes by entry and goal
    /// progress lanes by ordinal, as their last whole projection observed them: the lanes a
    /// projection's later values name.
    share_lanes: BTreeMap<u32, u32>,
    manpower_lanes: BTreeMap<u64, (u32, Vec<Option<u32>>)>,
    progress_lanes: BTreeMap<u64, Vec<(u32, u32)>>,
    /// The channels' reads the last hour published.
    reads: Option<[bool; CHANNELS.len()]>,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            lanes: Vec::new(),
            named: FastMap::default(),
            numbered: FastMap::default(),
            factions: BTreeSet::new(),
            stale: BTreeSet::new(),
            books: BTreeMap::new(),
            stale_books: BTreeSet::new(),
            last: None,
            first: None,
            previous_owners: BTreeMap::new(),
            live: FastMap::default(),
            read_lanes: BTreeMap::new(),
            unknown_values: BTreeSet::new(),
            unknown_choices: BTreeSet::new(),
            witnessed: BTreeSet::new(),
            ghost: None,
            ghosts: None,
            grew: false,
            share_slots: None,
            share_lanes: BTreeMap::new(),
            manpower_lanes: BTreeMap::new(),
            progress_lanes: BTreeMap::new(),
            reads: None,
        }
    }

    /// The lane of `faction`'s `kind` named `identity`, catalogued at `hour` when it is new.
    #[allow(clippy::too_many_arguments)]
    fn lane(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        kind: Kind,
        identity: &str,
        tag: &str,
        holder: Option<(u32, u32)>,
        (slot, group): (u32, u32),
    ) -> io::Result<u32> {
        if let Some(&lane) = self
            .named
            .get(&(faction, kind))
            .and_then(|named| named.get(identity))
        {
            return Ok(lane);
        }
        let ordinal = u32::try_from(self.lanes.len())
            .ok()
            .filter(|&ordinal| u64::from(ordinal) < LANES)
            .ok_or_else(|| io::Error::other("Faction lanes exhausted their catalogue"))?;
        let actor = self
            .actors
            .actor(writer, faction.to_string().as_bytes(), hour)?;
        let key = self.actors.key(
            writer,
            Place {
                actor,
                measure: kind.measure(),
                dimension: Some(u64::from(ordinal)),
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )?;
        // A faction the publication did not hold may be one an hour lists nowhere.
        self.grew |= faction != 0 && self.factions.insert(faction);
        if kind == Kind::Share
            && let Some(slots) = &mut self.share_slots
        {
            slots.insert(slot);
        }
        self.lanes.push(Lane {
            key,
            faction,
            kind,
            meta: Meta {
                identity: identity.into(),
                tag: tag.into(),
                holder,
                slot,
                group,
                choices: Vec::new(),
            },
        });
        self.named
            .entry((faction, kind))
            .or_default()
            .insert(identity.into(), ordinal);
        self.stale.insert(ordinal);
        Ok(ordinal)
    }

    /// `lane`'s value from `hour` on, none ending it: a value after an hour without one starts a
    /// segment.
    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        lane: u32,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let key = self.lanes[lane as usize].key;
        let starts = value.is_some() && (starts || self.carry.held(key).is_none());
        self.carry.value(writer, key, hour, value, starts).map(drop)
    }

    /// `lane`'s value from `hour` on; a known one after an unavailable one starts a segment.
    fn value(
        &mut self,
        writer: &mut Writer<'_>,
        lane: u32,
        hour: i32,
        value: i128,
        starts: bool,
    ) -> io::Result<()> {
        let recovered = value != NULL && self.unknown_values.remove(&lane);
        if value == NULL {
            self.unknown_values.insert(lane);
        }
        self.observe(writer, lane, hour, Some(value), starts || recovered)
    }

    /// `lane`'s choice from `hour` on, numbered where it first appears.
    fn choice(
        &mut self,
        writer: &mut Writer<'_>,
        lane: u32,
        hour: i32,
        choice: Choice,
        starts: bool,
    ) -> io::Result<()> {
        let unknown = choice.token.is_none() && choice.reason.is_some();
        let recovering = !unknown && self.unknown_choices.remove(&lane);
        if unknown {
            self.unknown_choices.insert(lane);
        }
        let held = &mut self.lanes[lane as usize];
        // The choice the lane holds is the one most often observed again.
        let current = self
            .carry
            .held(held.key)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|&at| held.meta.choices.get(at) == Some(&choice));
        let index = match current {
            Some(index) => index,
            None => {
                let digest = digest(&choice);
                let choices = &held.meta.choices;
                // A digest two choices share finds the second by its scan.
                let found = match self.numbered.get(&(lane, digest)) {
                    Some(&at) if choices[at as usize] == choice => Some(at as usize),
                    Some(_) => choices.iter().position(|old| *old == choice),
                    None => None,
                };
                match found {
                    Some(index) => index,
                    None => {
                        let index = choices.len();
                        self.numbered.entry((lane, digest)).or_insert(index as u32);
                        held.meta.choices.push(choice);
                        self.stale.insert(lane);
                        index
                    }
                }
            }
        };
        self.value(writer, lane, hour, index as i128, starts || recovering)
    }

    /// `faction`'s known and read lanes of channel `channel`.
    #[allow(clippy::too_many_arguments)]
    fn known(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        channel: usize,
        known: bool,
        read: bool,
        starts: bool,
    ) -> io::Result<()> {
        let name = CHANNELS[channel].0;
        let lane = self.lane(writer, hour, faction, Kind::Known, name, "", None, (0, 0))?;
        self.value(writer, lane, hour, i128::from(known), starts)?;
        let lane = self.lane(writer, hour, faction, Kind::Read, name, "", None, (0, 0))?;
        self.value(writer, lane, hour, i128::from(known && read), starts)?;
        self.read_lanes.insert(lane, (channel, known));
        Ok(())
    }

    /// `owner`'s live lanes of `kind` are `now`: the others end, or hold an unavailable value
    /// while `unknown` says why, and stay live until a later read closes or recovers them.
    #[allow(clippy::too_many_arguments)]
    fn finish_group(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        (owner, kind): (u64, Kind),
        now: BTreeSet<u32>,
        unknown: Option<&str>,
        selections: bool,
        starts: bool,
    ) -> io::Result<()> {
        let before = self.live.remove(&(owner, kind)).unwrap_or_default();
        let ended: Vec<u32> = before.difference(&now).copied().collect();
        let kept = match unknown {
            Some(_) => before.union(&now).copied().collect(),
            None => now,
        };
        if !kept.is_empty() {
            self.live.insert((owner, kind), kept);
        }
        for lane in ended {
            if selections {
                let choice = unknown.map_or_else(Choice::default, |why| Choice::unknown(why.to_owned()));
                self.choice(writer, lane, hour, choice, starts)?;
            } else if unknown.is_some() {
                self.value(writer, lane, hour, NULL, starts)?;
            } else {
                self.observe(writer, lane, hour, None, starts)?;
            }
        }
        Ok(())
    }

    /// Keeps one projected hour: lanes, choices and goal books numbered in the order the hour
    /// observes them, retained owners, and every faction the publication holds that the hour
    /// lists nowhere.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("faction emit", None);
        let at = hour.hour;
        let starts = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => false,
            Some(last) if at <= last => {
                return Err(dashboard_base::invalid(
                    "A faction hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run, and is not known.
            Some(last) => {
                self.carry.close(writer, last)?;
                self.known_through(writer, last)?;
                true
            }
            None => true,
        };
        self.first.get_or_insert(at);
        let Projected {
            joined,
            roster,
            witnessed,
            ghost,
            accounts,
            factions,
            reads,
        } = hour.projected;
        if starts || !joined {
            self.previous_owners.clear();
        }
        if let Some(witnessed) = witnessed {
            self.witnessed = Arc::unwrap_or_clone(witnessed);
            self.grew = true;
        }
        let renewed = ghost.is_some();
        if let Some(ghost) = ghost {
            self.ghost = Some(Arc::unwrap_or_clone(ghost));
        }
        // The hour's owners are the factions the publication held before its accounts.
        let before = if self.grew || self.ghosts.is_none() {
            self.grew = false;
            let now = self.factions.difference(&self.witnessed).copied().collect();
            Some(self.ghosts.replace(now))
        } else {
            None
        };
        let ghosts = self.ghosts.as_ref().expect("factions listed nowhere");
        // One read as absent the hour before, for the same reasons, reads alike.
        let due: Vec<u64> = match before {
            Some(Some(before)) if !starts && !renewed => {
                ghosts.difference(&before).copied().collect()
            }
            None if !starts && !renewed => Vec::new(),
            _ => ghosts.iter().copied().collect(),
        };
        // Absence before a faction's first lane is proven only by a complete roster.
        let lane = self.lane(writer, at, 0, Kind::Roster, "", "", None, (0, 0))?;
        self.value(writer, lane, at, i128::from(roster), starts)?;
        self.accounts(writer, at, starts, accounts)?;
        // The factions whose every known lane the hour observes.
        let mut visited: BTreeSet<u64> = due.iter().copied().collect();
        let mut due = due.into_iter().peekable();
        for ops in factions {
            while let Some(faction) = due.next_if(|faction| *faction < ops.faction) {
                self.absent(writer, at, faction, starts)?;
            }
            if ops.known.is_some() {
                visited.insert(ops.faction);
            }
            self.faction(writer, at, ops, reads, starts)?;
        }
        for faction in due {
            self.absent(writer, at, faction, starts)?;
        }
        // Every other faction keeps what it knows; only the channels read anew move.
        let previous = self.reads.replace(reads);
        if !starts && previous != Some(reads) {
            let moved: Vec<(u32, bool)> = self
                .read_lanes
                .iter()
                .filter(|&(&lane, &(index, _))| {
                    previous.is_none_or(|previous| previous[index] != reads[index])
                        && !visited.contains(&self.lanes[lane as usize].faction)
                })
                .map(|(&lane, &(index, known))| (lane, known && reads[index]))
                .collect();
            for (lane, read) in moved {
                self.observe(writer, lane, at, Some(i128::from(read)), false)?;
            }
        }
        self.last = Some(at);
        Ok(())
    }

    fn accounts(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        starts: bool,
        accounts: Accounts,
    ) -> io::Result<()> {
        let failure = accounts
            .unavailable
            .then_some("Faction account inputs are unavailable");
        // A full hour also reads again every account a share lane names.
        let extra: Vec<u32> = if accounts.full {
            let present: BTreeSet<u32> = accounts.ops.iter().map(AccountOp::account).collect();
            let lanes = &self.lanes;
            let slots = self.share_slots.get_or_insert_with(|| {
                lanes
                    .iter()
                    .filter(|lane| lane.kind == Kind::Share)
                    .map(|lane| lane.meta.slot)
                    .collect()
            });
            slots
                .iter()
                .copied()
                .filter(|slot| !present.contains(slot))
                .collect()
        } else {
            Vec::new()
        };
        let mut extra = extra.into_iter().peekable();
        for op in accounts.ops {
            let next = op.account();
            while let Some(slot) = extra.next_if(|slot| *slot < next) {
                self.account(writer, hour, starts, slot, None, failure)?;
            }
            match op {
                AccountOp::Whole { account, row } => {
                    self.account(writer, hour, starts, account, row, failure)?
                }
                AccountOp::Share { account, share } => {
                    let lane = *self
                        .share_lanes
                        .get(&account)
                        .ok_or_else(|| io::Error::other("A faction account share has no lane"))?;
                    self.value(writer, lane, hour, i128::from(share), starts)?;
                }
            }
        }
        for slot in extra {
            self.account(writer, hour, starts, slot, None, failure)?;
        }
        Ok(())
    }

    fn account(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        starts: bool,
        account: u32,
        row: Option<AccountRow>,
        mut failure: Option<&'static str>,
    ) -> io::Result<()> {
        let (mut shares, mut associations, mut upgrades) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        self.share_lanes.remove(&account);
        match row {
            None => {}
            Some(AccountRow::Unidentified) => {
                failure = Some("Faction account country identity is unavailable");
            }
            Some(AccountRow::Identified {
                identity,
                tag,
                holder,
                share,
                owner,
            }) => {
                if self
                    .previous_owners
                    .get(&account)
                    .is_some_and(|(before, _)| before != &identity)
                {
                    self.previous_owners.remove(&account);
                }
                let place = (account, 0);
                let lane = self.lane(writer, hour, 0, Kind::Share, &identity, &tag, Some(holder), place)?;
                shares.insert(lane);
                self.value(writer, lane, hour, i128::from(share), starts)?;
                self.share_lanes.insert(account, lane);
                let (faction, member) = match owner {
                    Owner::Unjoined => {
                        failure = Some("Faction account membership inputs are unavailable");
                        (None, None)
                    }
                    Owner::Member {
                        faction,
                        upgrades: held,
                    } => {
                        self.previous_owners
                            .insert(account, (identity.clone(), faction));
                        (Some(faction), Some(held))
                    }
                    Owner::Ambiguous => {
                        failure = Some("Faction account country has ambiguous membership");
                        (None, None)
                    }
                    Owner::Influence(faction) => (Some(faction), None),
                    Owner::Retained => (
                        self.previous_owners
                            .get(&account)
                            .map(|(_, faction)| *faction),
                        None,
                    ),
                };
                if let Some(faction) = faction {
                    let kind = Kind::Association;
                    let lane =
                        self.lane(writer, hour, faction, kind, &identity, &tag, Some(holder), place)?;
                    associations.insert(lane);
                    let associated = if member.is_some() { 1 } else { 2 };
                    self.value(writer, lane, hour, associated, starts)?;
                    for (ordinal, upgrade) in member.into_iter().flatten().enumerate() {
                        self.group(writer, hour, faction, upgrade.group, upgrade.name, starts)?;
                        let lane = self.lane(
                            writer,
                            hour,
                            faction,
                            Kind::Upgrade,
                            &format!("{identity}:{ordinal}:{}", upgrade.group),
                            &tag,
                            Some(holder),
                            (account, upgrade.group),
                        )?;
                        upgrades.insert(lane);
                        self.choice(writer, lane, hour, upgrade.choice, starts)?;
                    }
                }
            }
        }
        if shares.is_empty() || failure.is_some() {
            self.previous_owners.remove(&account);
        }
        let owner = u64::from(account);
        self.finish_group(writer, hour, (owner, Kind::Share), shares, failure, false, starts)?;
        let group = (owner, Kind::Association);
        self.finish_group(writer, hour, group, associations, failure, false, starts)?;
        self.finish_group(writer, hour, (owner, Kind::Upgrade), upgrades, failure, true, starts)
    }

    fn faction(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        ops: FactionOps,
        reads: [bool; CHANNELS.len()],
        starts: bool,
    ) -> io::Result<()> {
        let FactionOps {
            faction,
            known,
            channels,
        } = ops;
        if let Some(known) = known {
            for channel in 0..CHANNELS.len() {
                let flags = (known[channel], reads[channel]);
                self.known(writer, hour, faction, channel, flags.0, flags.1, starts)?;
            }
        }
        for channel in channels {
            match channel {
                ChannelOp::Selections {
                    kind,
                    values,
                    failure,
                } => {
                    let failure = failure.as_deref();
                    self.selections(writer, hour, faction, kind, values, failure, starts)?
                }
                ChannelOp::Group { group, choice } => {
                    self.group(writer, hour, faction, group, choice, starts)?
                }
                ChannelOp::Manpower(op) => self.manpower(writer, hour, faction, op, starts)?,
                ChannelOp::ManpowerValues { total, values } => {
                    let (total_lane, lanes) = self
                        .manpower_lanes
                        .get(&faction)
                        .ok_or_else(|| io::Error::other("Faction manpower values have no lanes"))?;
                    let total_lane = *total_lane;
                    let changed: Vec<(u32, i32)> = values
                        .into_iter()
                        .filter_map(|(at, value)| {
                            lanes.get(at as usize).copied().flatten().map(|lane| (lane, value))
                        })
                        .collect();
                    self.value(writer, total_lane, hour, total, starts)?;
                    for (lane, value) in changed {
                        self.value(writer, lane, hour, i128::from(value), starts)?;
                    }
                }
                ChannelOp::Goals(op) => self.goals(writer, hour, faction, op, starts)?,
                ChannelOp::GoalProgress(values) => {
                    let lanes = self
                        .progress_lanes
                        .get(&faction)
                        .ok_or_else(|| io::Error::other("Faction goal progress has no lanes"))?;
                    let changed: Vec<(u32, i64)> = values
                        .into_iter()
                        .filter_map(|(ordinal, raw)| {
                            lanes
                                .iter()
                                .find(|(held, _)| *held == ordinal)
                                .map(|&(_, lane)| (lane, raw))
                        })
                        .collect();
                    for (lane, raw) in changed {
                        self.value(writer, lane, hour, i128::from(raw), starts)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A faction the publication holds that the hour lists nowhere: it has no rows anywhere.
    fn absent(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        starts: bool,
    ) -> io::Result<()> {
        let Some(ghost) = self.ghost.clone() else {
            return Ok(());
        };
        for channel in 0..CHANNELS.len() {
            self.known(writer, hour, faction, channel, false, false, starts)?;
        }
        for (kind, failure) in [Kind::Rule, Kind::Programme, Kind::Minimum, Kind::Doctrine]
            .into_iter()
            .zip(&ghost.selections)
        {
            let failure = Some(failure.as_str());
            self.selections(writer, hour, faction, kind, Vec::new(), failure, starts)?;
        }
        let pool = ManpowerOp {
            total: NULL,
            entries: Vec::new(),
            failure: Some(ghost.manpower.clone()),
        };
        self.manpower(writer, hour, faction, pool, starts)?;
        let goals = GoalsOp {
            absent: ghost.goals.absent,
            reason: ghost.goals.reason.clone(),
            entries: Vec::new(),
        };
        self.goals(writer, hour, faction, goals, starts)
    }

    fn group(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        group: u32,
        choice: Choice,
        starts: bool,
    ) -> io::Result<()> {
        let identity = group.to_string();
        let place = (group, group);
        let lane = self.lane(writer, hour, faction, Kind::Group, &identity, "", None, place)?;
        self.choice(writer, lane, hour, choice, starts)
    }

    #[allow(clippy::too_many_arguments)]
    fn selections(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        kind: Kind,
        values: Vec<(u32, u32, Choice)>,
        failure: Option<&str>,
        starts: bool,
    ) -> io::Result<()> {
        let mut now = BTreeSet::new();
        for (slot, group, choice) in values {
            let identity = format!("{slot}:{group}");
            let lane = self.lane(writer, hour, faction, kind, &identity, "", None, (slot, group))?;
            now.insert(lane);
            self.choice(writer, lane, hour, choice, starts)?;
        }
        self.finish_group(writer, hour, (faction, kind), now, failure, true, starts)
    }

    fn manpower(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        op: ManpowerOp,
        starts: bool,
    ) -> io::Result<()> {
        let total = self.lane(writer, hour, faction, Kind::Total, "", "", None, (0, 0))?;
        self.value(writer, total, hour, op.total, starts)?;
        let mut now = BTreeSet::new();
        let mut diagnostics = BTreeSet::new();
        let mut lanes = Vec::with_capacity(op.entries.len());
        for entry in op.entries {
            match entry {
                ManpowerEntry::Unidentified { ordinal, reason } => {
                    let identity = format!("country:{ordinal}");
                    let kind = Kind::ManpowerDiagnostic;
                    let lane =
                        self.lane(writer, hour, faction, kind, &identity, "", None, (ordinal, 0))?;
                    diagnostics.insert(lane);
                    self.choice(writer, lane, hour, Choice::unknown(reason), starts)?;
                    lanes.push(None);
                }
                ManpowerEntry::Member {
                    ordinal,
                    identity,
                    tag,
                    holder,
                    value,
                } => {
                    let kind = Kind::Manpower;
                    let lane =
                        self.lane(writer, hour, faction, kind, &identity, &tag, holder, (ordinal, 0))?;
                    now.insert(lane);
                    self.value(writer, lane, hour, i128::from(value), starts)?;
                    lanes.push(Some(lane));
                }
            }
        }
        let kind = Kind::ManpowerDiagnostic;
        let lane = self.lane(writer, hour, faction, kind, "pool", "", None, (0, 0))?;
        diagnostics.insert(lane);
        let pool = op.failure.clone().map_or_else(Choice::default, Choice::unknown);
        self.choice(writer, lane, hour, pool, starts)?;
        self.finish_group(writer, hour, (faction, kind), diagnostics, None, true, starts)?;
        let failure = op.failure.as_deref();
        self.finish_group(writer, hour, (faction, Kind::Manpower), now, failure, false, starts)?;
        self.manpower_lanes.insert(faction, (total, lanes));
        Ok(())
    }

    fn goals(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        faction: u64,
        op: GoalsOp,
        starts: bool,
    ) -> io::Result<()> {
        let lane = self.lane(writer, hour, faction, Kind::GoalList, "", "", None, (0, 0))?;
        let mut list = GoalList {
            absent: op.absent,
            reason: op.reason,
            ..Default::default()
        };
        let mut progress = Vec::new();
        let book = self.books.entry(faction).or_default();
        let mut grew = false;
        for entry in op.entries {
            let (label, new) = book.labels.intern(entry.label);
            grew |= new;
            match entry.kind {
                0 => {
                    list.goals.push((entry.ordinal, label, entry.status));
                    progress.push((entry.ordinal, entry.progress));
                }
                1 => list.completed.push(label),
                _ => list.cancelled.push(label),
            }
        }
        let (value, new) = book.lists.intern(list);
        if grew || new {
            self.stale_books.insert(faction);
        }
        self.value(writer, lane, hour, i128::from(value), starts)?;
        let mut now = BTreeSet::new();
        let mut lanes = Vec::with_capacity(progress.len());
        for (ordinal, raw) in progress {
            let identity = ordinal.to_string();
            let kind = Kind::GoalProgress;
            let lane = self.lane(writer, hour, faction, kind, &identity, "", None, (ordinal, 0))?;
            now.insert(lane);
            self.value(writer, lane, hour, i128::from(raw), starts)?;
            lanes.push((ordinal, lane));
        }
        let group = (faction, Kind::GoalProgress);
        self.finish_group(writer, hour, group, now, None, false, starts)?;
        self.progress_lanes.insert(faction, lanes);
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the lanes and books that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known_through(writer, last)?;
        for lane in std::mem::take(&mut self.stale) {
            let cell = family::cell(writer, &self.lanes[lane as usize].meta.scalar())?;
            writer.catalogue(LANES + u64::from(lane), cell)?;
        }
        for faction in std::mem::take(&mut self.stale_books) {
            let actor = self
                .actors
                .find(faction.to_string().as_bytes())
                .ok_or_else(|| io::Error::other("A faction goal book has no actor"))?;
            let book = serde_json::to_vec(&self.books[&faction])?;
            let cell = family::cell(writer, &Scalar::Text(book.into()))?;
            writer.catalogue(BOOKS + actor, cell)?;
        }
        Ok(())
    }

    /// The hours walked since the last known span, through `last`, as known.
    fn known_through(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut live: Vec<(u64, Kind, Vec<u32>)> = self
            .live
            .iter()
            .map(|(&(owner, kind), lanes)| (owner, kind, lanes.iter().copied().collect()))
            .collect();
        live.sort_unstable();
        let saved = Saved {
            last: self.last,
            previous_owners: self.previous_owners.clone(),
            live,
            read_lanes: self.read_lanes.clone(),
            unknown_values: self.unknown_values.iter().copied().collect(),
            unknown_choices: self.unknown_choices.iter().copied().collect(),
            witnessed: self.witnessed.iter().copied().collect(),
            ghost: self.ghost.clone(),
            share_lanes: self.share_lanes.clone(),
            manpower_lanes: self.manpower_lanes.clone(),
            progress_lanes: self.progress_lanes.clone(),
            reads: self.reads,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut lanes: Vec<Option<Lane>> = Vec::new();
        for (actor, named) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            let faction: u64 = std::str::from_utf8(&named.name)
                .ok()
                .and_then(|name| name.parse().ok())
                .ok_or_else(invalid)?;
            for kind in Kind::ALL {
                for &(ordinal, key) in directory.dimensions(actor, kind.measure()) {
                    let at = usize::try_from(ordinal).map_err(|_| invalid())?;
                    if at >= lanes.len() {
                        lanes.resize_with(at + 1, || None);
                    }
                    let meta = Meta::read(directory.scalar(LANES + ordinal))?;
                    lanes[at] = Some(Lane {
                        key,
                        faction,
                        kind,
                        meta,
                    });
                }
            }
            if let Some(book) = directory.scalar(BOOKS + actor) {
                let book: GoalBook = serde_json::from_slice(book.text().ok_or_else(invalid)?)?;
                emit.books.insert(faction, book);
            }
        }
        emit.lanes = lanes.into_iter().collect::<Option<_>>().ok_or_else(invalid)?;
        for (ordinal, lane) in emit.lanes.iter().enumerate() {
            emit.named
                .entry((lane.faction, lane.kind))
                .or_default()
                .insert(lane.meta.identity.as_str().into(), ordinal as u32);
            if lane.faction != 0 {
                emit.factions.insert(lane.faction);
            }
            for (index, choice) in lane.meta.choices.iter().enumerate() {
                let key = (ordinal as u32, digest(choice));
                emit.numbered.entry(key).or_insert(index as u32);
            }
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        emit.carry = Carry::load(&bytes[d.position()..])?;
        let count = emit.lanes.len();
        let known = |lane: &u32| (*lane as usize) < count;
        let lanes_known = saved.live.iter().all(|(_, _, lanes)| lanes.iter().all(known))
            && saved.read_lanes.keys().all(known)
            && saved.unknown_values.iter().all(known)
            && saved.unknown_choices.iter().all(known)
            && saved.share_lanes.values().all(known)
            && saved.manpower_lanes.values().all(|(total, lanes)| {
                known(total) && lanes.iter().flatten().all(known)
            })
            && saved
                .progress_lanes
                .values()
                .all(|lanes| lanes.iter().all(|(_, lane)| known(lane)));
        if !lanes_known {
            return Err(invalid());
        }
        emit.last = saved.last;
        emit.previous_owners = saved.previous_owners;
        emit.live = saved
            .live
            .into_iter()
            .map(|(owner, kind, lanes)| ((owner, kind), lanes.into_iter().collect()))
            .collect();
        emit.read_lanes = saved.read_lanes;
        emit.unknown_values = saved.unknown_values.into_iter().collect();
        emit.unknown_choices = saved.unknown_choices.into_iter().collect();
        emit.witnessed = saved.witnessed.into_iter().collect();
        emit.ghost = saved.ghost;
        emit.share_lanes = saved.share_lanes;
        emit.manpower_lanes = saved.manpower_lanes;
        emit.progress_lanes = saved.progress_lanes;
        emit.reads = saved.reads;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    previous_owners: BTreeMap<u32, (String, u64)>,
    live: Vec<(u64, Kind, Vec<u32>)>,
    read_lanes: BTreeMap<u32, (usize, bool)>,
    unknown_values: Vec<u32>,
    unknown_choices: Vec<u32>,
    witnessed: Vec<u64>,
    ghost: Option<Ghost>,
    share_lanes: BTreeMap<u32, u32>,
    manpower_lanes: BTreeMap<u64, (u32, Vec<Option<u32>>)>,
    progress_lanes: BTreeMap<u64, Vec<(u32, u32)>>,
    reads: Option<[bool; CHANNELS.len()]>,
}
