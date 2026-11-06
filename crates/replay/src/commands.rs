use crate::{
    codec::{Decoder, invalid_static as invalid, put_varint},
    container::Replay,
    spool::ControlRecord,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    io::Read,
    path::Path,
};

mod codec;
mod index;
use codec::{decode_batch, encode_batch, narrow};
pub use index::{ClockMeasurement, CountSeries, Counts, Index, Measurement, Timeline};

#[cfg(test)]
mod tests;

pub struct Recording {
    pub sessions: Vec<replay_data::identity::SessionStart>,
    pub branches: Vec<crate::branches::BranchStart>,
    pub controls: Vec<ControlRecord>,
}

impl Recording {
    pub fn open_path(path: &Path) -> io::Result<Self> {
        let mut magic = [0; 8];
        std::fs::File::open(path)?.read_exact(&mut magic)?;
        if &magic == b"H4COLUMN" {
            let recording = crate::columnar_frames::Recording::open_path(path)?;
            let capture = recording
                .capture()?
                .ok_or_else(|| invalid("columnar command metadata is absent"))?;
            Self::from_columnar(capture)
        } else {
            let replay = Replay::open_path(path)?;
            Ok(Self {
                sessions: replay.sessions,
                branches: replay.branches,
                controls: replay.controls,
            })
        }
    }

    pub fn from_columnar(capture: crate::columnar_file::Capture) -> io::Result<Self> {
        use replay_data::identity::IdentityCodec;
        crate::format::FormatVersion::read(capture.format)?;
        let mut sessions = engine_allocation::vec(capture.sessions.len())?;
        for bytes in capture.sessions {
            let mut input = Decoder::new(&bytes);
            input.format = capture.format;
            let session = replay_data::identity::SessionStart::decode(&mut input)?;
            input.finish()?;
            sessions.push(session);
        }
        let mut branches = BTreeMap::new();
        let mut controls = engine_allocation::vec(capture.controls.len())?;
        for bytes in capture.controls {
            match crate::spool::decode_record(&bytes)? {
                crate::spool::Record::BranchStart(start) => {
                    if let Some(previous) = branches.insert(start.id, start.clone()) {
                        if previous.session != start.session {
                            return Err(invalid("conflicting columnar command branch identity"));
                        }
                    }
                }
                crate::spool::Record::Control(control) => controls.push(control),
                crate::spool::Record::Gap(_) => {}
                _ => return Err(invalid("unexpected columnar command metadata record")),
            }
        }
        for control in &controls {
            if let ControlRecord::PlayerCommands { session, packet } = control
                && let Some(phase) = &packet.phase
            {
                phase.validate()?;
                if phase.start.session != *session
                    || phase.start.id != packet.branch
                    || !phase.campaign.eq_ignore_ascii_case(&capture.campaign_id)
                {
                    return Err(invalid("conflicting columnar command phase identity"));
                }
                if let Some(previous) = branches.insert(phase.start.id, phase.start.clone())
                    && previous != phase.start
                {
                    return Err(invalid("conflicting columnar command branch declaration"));
                }
            }
        }
        for control in &controls {
            if !sessions
                .iter()
                .any(|session| session.id == control.session())
            {
                return Err(invalid("columnar command metadata has an unknown session"));
            }
            if let ControlRecord::PlayerCommands { session, packet } = control {
                if branches.get(&packet.branch).map(|branch| branch.session) != Some(*session) {
                    return Err(invalid(
                        "columnar commands refer to another session's branch",
                    ));
                }
            }
        }
        Ok(Self {
            sessions,
            branches: branches.into_values().collect(),
            controls,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Category {
    #[default]
    Other,
    Army,
    BattlePlans,
    Navy,
    Air,
    Production,
    Construction,
    Research,
    Politics,
    Diplomacy,
    Intelligence,
    Logistics,
    Deployment,
    Equipment,
    Selection,
    Interface,
    Session,
    Automatic,
    System,
}

impl Category {
    pub fn counted(self) -> bool {
        !matches!(
            self,
            Self::Other
                | Self::System
                | Self::Session
                | Self::Automatic
                | Self::Interface
                | Self::Selection
        )
    }

    fn read(value: u8) -> io::Result<Self> {
        const ALL: [Category; 19] = [
            Category::Other,
            Category::Army,
            Category::BattlePlans,
            Category::Navy,
            Category::Air,
            Category::Production,
            Category::Construction,
            Category::Research,
            Category::Politics,
            Category::Diplomacy,
            Category::Intelligence,
            Category::Logistics,
            Category::Deployment,
            Category::Equipment,
            Category::Selection,
            Category::Interface,
            Category::Session,
            Category::Automatic,
            Category::System,
        ];
        ALL.get(value as usize)
            .copied()
            .ok_or_else(|| invalid("unknown command category"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kind {
    pub id: u32,
    pub token: u32,
    pub name: String,
    pub category: Category,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub id: u64,
    pub player: i32,
    pub country: u32,
    pub since_qpc: u64,
    pub at_qpc: u64,
    pub user: Vec<u8>,
    pub name: Vec<u8>,
    pub tag: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Origin {
    #[default]
    Unresolved,
    PlayerOrder,
    AutonomousCountry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub qpc: u64,
    pub hour: i32,
    pub actor: u64,
    pub player: i32,
    pub kind: u32,
    pub command_country: Option<u32>,
    pub origin: Origin,
    pub sequence: u32,
    pub paused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub from_qpc: u64,
    pub to_qpc: u64,
    pub complete: bool,
    pub hour: i32,
    pub paused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Batch {
    pub kinds: Vec<Kind>,
    pub actors: Vec<Actor>,
    pub departures: Vec<(u64, u64)>,
    pub events: Vec<Event>,
    #[serde(alias = "coverage")]
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub start: crate::branches::BranchStart,
    pub campaign: [u8; 36],
    // An hourly context observation can follow this packet's command watermark.
    pub qpc: u64,
    pub hour: i32,
    pub load_signal: i64,
}

// The native load marker remains zero until the first simulation update.
pub fn load_marker(raw: i64) -> Option<i64> {
    (raw != 0).then_some(raw)
}

impl Phase {
    pub fn load_marker(&self) -> Option<i64> {
        load_marker(self.load_signal)
    }

    pub fn validate(&self) -> io::Result<()> {
        self.start.validate()?;
        if self.qpc == 0
            || self.hour < self.start.first_hour
            || !replay_data::identity::is_campaign_uuid(&self.campaign)
        {
            return Err(invalid("invalid command phase identity"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub branch: u64,
    pub capture: u64,
    pub world: u64,
    pub ordinal: u64,
    pub from_qpc: u64,
    pub to_qpc: u64,
    pub lost: u64,
    pub phase: Option<Phase>,
    raw_length: u32,
    compressed: bool,
    data: Vec<u8>,
}

impl Packet {
    pub fn storage_bytes(&self) -> usize {
        self.data.capacity()
    }
    pub fn pack(
        branch: u64,
        capture: u64,
        world: u64,
        ordinal: u64,
        from_qpc: u64,
        to_qpc: u64,
        lost: u64,
        batch: &Batch,
    ) -> io::Result<Self> {
        if from_qpc > to_qpc || world == 0 || capture == 0 {
            return Err(invalid("invalid command packet interval"));
        }
        let raw = encode_batch(batch, from_qpc, to_qpc)?;
        let raw_length =
            u32::try_from(raw.len()).map_err(|_| invalid("command packet too large"))?;
        if raw.len() > crate::spool::MAX_RECORD_BYTES {
            return Err(invalid("command packet exceeds record bound"));
        }
        // Small packets compress better together in the container's controls extent.
        let compressed = if raw.len() > 1024 {
            Some(zstd::bulk::compress(&raw, 3)?)
        } else {
            None
        };
        let (data, compressed) = match compressed {
            Some(data) if data.len() < raw.len() => (data, true),
            _ => (engine_allocation::copy_vec(&raw)?, false),
        };
        Ok(Self {
            branch,
            capture,
            world,
            ordinal,
            from_qpc,
            to_qpc,
            lost,
            phase: None,
            raw_length,
            compressed,
            data,
        })
    }

    pub fn unpack(&self) -> io::Result<Batch> {
        if !self.compressed {
            if self.data.len() != self.raw_length as usize {
                return Err(invalid("command packet length mismatch"));
            }
            return decode_batch(&self.data, self.from_qpc, self.to_qpc);
        }
        let raw = zstd::bulk::decompress(&self.data, self.raw_length as usize)?;
        if raw.len() != self.raw_length as usize {
            return Err(invalid("command packet length mismatch"));
        }
        decode_batch(&raw, self.from_qpc, self.to_qpc)
    }

    pub(crate) fn encoded_bytes(&self) -> io::Result<usize> {
        engine_allocation::add(
            self.header_values()
                .into_iter()
                .map(varint_bytes)
                .sum::<usize>()
                + 2
                + self
                    .phase
                    .as_ref()
                    .map_or(0, |phase| phase.start.encoded_bytes() + 36 + 8 + 4 + 8),
            self.data.len(),
        )
    }

    fn header_values(&self) -> [u128; 9] {
        [
            self.branch.into(),
            self.capture.into(),
            self.world.into(),
            self.ordinal.into(),
            self.from_qpc.into(),
            (self.to_qpc - self.from_qpc).into(),
            self.lost.into(),
            self.raw_length.into(),
            self.data.len() as u128,
        ]
    }

    pub(crate) fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        engine_allocation::reserve(out, self.encoded_bytes()?)?;
        for value in &self.header_values()[..7] {
            put_varint(out, *value);
        }
        match &self.phase {
            None => out.push(0),
            Some(phase) => {
                phase.validate()?;
                if phase.start.id != self.branch {
                    return Err(invalid("command phase differs from packet"));
                }
                out.push(1);
                phase.start.encode(out)?;
                out.extend_from_slice(&phase.campaign);
                out.extend_from_slice(&phase.qpc.to_le_bytes());
                out.extend_from_slice(&phase.hour.to_le_bytes());
                out.extend_from_slice(&phase.load_signal.to_le_bytes());
            }
        }
        out.push(u8::from(self.compressed));
        put_varint(out, self.raw_length.into());
        put_varint(out, self.data.len() as u128);
        out.extend_from_slice(&self.data);
        Ok(())
    }

    pub(crate) fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let branch = narrow(input.varint()?)?;
        let capture = narrow(input.varint()?)?;
        let world = narrow(input.varint()?)?;
        let ordinal = narrow(input.varint()?)?;
        let from_qpc: u64 = narrow(input.varint()?)?;
        let to_qpc = from_qpc
            .checked_add(narrow(input.varint()?)?)
            .ok_or_else(|| invalid("command packet interval overflow"))?;
        let lost = narrow(input.varint()?)?;
        let value = Self {
            branch,
            capture,
            world,
            ordinal,
            from_qpc,
            to_qpc,
            lost,
            phase: match input.u8()? {
                0 => None,
                1 => {
                    let start = crate::branches::BranchStart::decode(input)?;
                    let mut campaign = [0; 36];
                    for byte in &mut campaign {
                        *byte = input.u8()?;
                    }
                    let phase = Phase {
                        start,
                        campaign,
                        qpc: input.u64()?,
                        hour: input.i32()?,
                        load_signal: input.i64()?,
                    };
                    phase.validate()?;
                    Some(phase)
                }
                _ => return Err(invalid("invalid command phase option")),
            },
            compressed: input.bool()?,
            raw_length: narrow(input.varint()?)?,
            data: {
                let length = narrow::<usize>(input.varint()?)?;
                if length > crate::spool::MAX_RECORD_BYTES {
                    return Err(invalid("command packet exceeds record bound"));
                }
                engine_allocation::copy_vec(input.take(length)?)?
            },
        };
        if value.world == 0
            || value.capture == 0
            || value.from_qpc > value.to_qpc
            || value.raw_length as usize > crate::spool::MAX_RECORD_BYTES
            || (!value.compressed && value.data.len() != value.raw_length as usize)
            || value
                .phase
                .as_ref()
                .is_some_and(|phase| phase.start.id != value.branch)
        {
            return Err(invalid("invalid command packet header"));
        }
        Ok(value)
    }
}

fn varint_bytes(value: u128) -> usize {
    ((128 - value.leading_zeros() as usize).max(1) + 6) / 7
}

#[derive(Debug, Serialize)]
pub struct PlayerSummary {
    pub session: u64,
    pub capture: u64,
    pub world: u64,
    pub actor: Actor,
    pub commands: u64,
    pub unresolved_commands: u64,
    pub unclassified_commands: u64,
    pub coverage_complete: bool,
    pub confirmed_orders: u64,
    pub paused_orders: Option<u64>,
    pub paused_unresolved_commands: u64,
    pub categories: BTreeMap<Category, u64>,
    pub unresolved_kinds: BTreeMap<String, u64>,
    pub observed_minutes: Option<f64>,
    pub confirmed_orders_per_minute: Option<f64>,
    pub apm: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct Analysis {
    pub recorded: bool,
    pub lost_observations: u64,
    pub unattributed_commands: u64,
    pub players: Vec<PlayerSummary>,
}

pub fn summarize(replay: &Replay, branch: u64) -> io::Result<Vec<PlayerSummary>> {
    Ok(analyze(&replay.sessions, &replay.controls, branch)?.players)
}

pub fn analyze(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
) -> io::Result<Analysis> {
    Ok(index(sessions, controls, branch)?.analysis)
}

pub fn index(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
) -> io::Result<Index> {
    index_with(sessions, controls, branch, || Ok(()))
}

pub fn index_with(
    sessions: &[replay_data::identity::SessionStart],
    controls: &[ControlRecord],
    branch: u64,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<Index> {
    #[derive(Default)]
    struct Window {
        spans: Vec<Span>,
        intervals: Vec<(u64, u64)>,
        complete: bool,
        next: u64,
        to: Option<u64>,
        actors: BTreeMap<u64, Vec<Actor>>,
        counts: BTreeMap<(u64, usize), CountSeries>,
        unknown_hours: BTreeSet<i32>,
        paused_unknown_hours: BTreeSet<i32>,
        catalogued: bool,
        timing: index::Timing,
    }
    let mut limits: BTreeMap<(u64, u64, u64), u64> = BTreeMap::new();
    for control in controls {
        check()?;
        if let ControlRecord::PlayerCommands { session, packet } = control
            && packet.branch == branch
        {
            limits
                .entry((*session, packet.capture, packet.world))
                .and_modify(|last| *last = (*last).max(packet.ordinal))
                .or_insert(packet.ordinal);
        }
    }
    let mut kinds = BTreeMap::new();
    let mut summaries: BTreeMap<(u64, u64, u64, u64), PlayerSummary> = BTreeMap::new();
    let mut departures = BTreeMap::new();
    let mut windows: BTreeMap<(u64, u64, u64), Window> = BTreeMap::new();
    let mut packets = BTreeMap::new();
    let mut lost_observations = 0_u64;
    let mut unattributed_commands = 0_u64;
    for control in controls {
        let ControlRecord::PlayerCommands { session, packet } = control else {
            continue;
        };
        let key = (*session, packet.capture, packet.world);
        if !limits.get(&key).is_some_and(|last| packet.ordinal <= *last) {
            continue;
        }
        if let Some(previous) = packets.insert((key, packet.ordinal), packet) {
            if previous != packet {
                return Err(invalid("conflicting command packet identity"));
            }
        }
    }
    for ((key, _), packet) in packets {
        check()?;
        let window = windows.entry(key).or_insert_with(|| Window {
            complete: true,
            ..Window::default()
        });
        let packet_complete = packet.ordinal == window.next
            && packet.lost == 0
            && window.to.is_none_or(|to| to == packet.from_qpc);
        window.complete &= packet_complete;
        if window.to.is_some_and(|to| to != packet.from_qpc) {
            window.complete = false;
        }
        window.to = Some(packet.to_qpc);
        window.next = packet
            .ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("command packet ordinal overflow"))?;
        let batch = packet.unpack()?;
        if packet.ordinal == 0 && batch.kinds.is_empty() {
            window.complete = false;
        }
        for kind in batch.kinds {
            window.catalogued = true;
            if kinds.insert((key, kind.id), kind).is_some() {
                return Err(invalid("repeated command kind definition"));
            }
        }
        for actor in batch.actors {
            let actor_key = (key.0, key.1, key.2, actor.id);
            if actor.id == 0 || actor.since_qpc > actor.at_qpc || actor.at_qpc > packet.to_qpc {
                return Err(invalid("invalid command actor identity"));
            }
            let history = window.actors.entry(actor.id).or_default();
            if history.last().is_none_or(|previous| previous != &actor) {
                if history
                    .last()
                    .is_some_and(|previous| previous.at_qpc > actor.at_qpc)
                {
                    return Err(invalid("command actor observations out of order"));
                }
                history.push(actor.clone());
            }
            if let Some(previous) = summaries.get_mut(&actor_key) {
                if previous.actor.player != actor.player
                    || previous.actor.since_qpc != actor.since_qpc
                {
                    return Err(invalid("command actor identity changed"));
                }
                previous.actor = actor;
            } else {
                summaries.insert(
                    actor_key,
                    PlayerSummary {
                        session: key.0,
                        capture: packet.capture,
                        world: packet.world,
                        actor,
                        commands: 0,
                        unresolved_commands: 0,
                        unclassified_commands: 0,
                        coverage_complete: false,
                        confirmed_orders: 0,
                        paused_orders: Some(0),
                        paused_unresolved_commands: 0,
                        categories: BTreeMap::new(),
                        unresolved_kinds: BTreeMap::new(),
                        observed_minutes: None,
                        confirmed_orders_per_minute: None,
                        apm: None,
                    },
                );
            }
        }
        for (actor, time) in batch.departures {
            if time < packet.from_qpc
                || time > packet.to_qpc
                || departures
                    .insert((key.0, key.1, key.2, actor), time)
                    .is_some()
            {
                return Err(invalid("invalid or repeated command player departure"));
            }
        }
        if packet.branch != branch {
            continue;
        }
        lost_observations = lost_observations
            .checked_add(packet.lost)
            .ok_or_else(|| invalid("command loss count overflow"))?;
        if let Some((_, to)) = window.intervals.last_mut()
            && *to == packet.from_qpc
        {
            *to = packet.to_qpc;
        } else {
            window.intervals.push((packet.from_qpc, packet.to_qpc));
        }
        for mut span in batch.spans {
            span.complete &= packet_complete && window.catalogued;
            if window
                .spans
                .last()
                .is_some_and(|previous| previous.to_qpc > span.from_qpc)
            {
                return Err(invalid("overlapping command coverage intervals"));
            }
            window.complete &= span.complete;
            window.spans.push(span);
        }
        for event in batch.events {
            if !window
                .spans
                .iter()
                .rev()
                .take_while(|span| span.to_qpc >= packet.from_qpc)
                .any(|span| {
                    span.from_qpc <= event.qpc
                        && event.qpc <= span.to_qpc
                        && span.paused == event.paused
                        && span.hour == event.hour
                })
            {
                window.complete = false;
                window.unknown_hours.insert(event.hour);
            }
            let actor_key = (key.0, key.1, key.2, event.actor);
            let Some(summary) = summaries.get_mut(&actor_key) else {
                unattributed_commands = unattributed_commands
                    .checked_add(1)
                    .ok_or_else(|| invalid("command count overflow"))?;
                if event.paused {
                    window.paused_unknown_hours.insert(event.hour);
                } else {
                    window.complete = false;
                    window.unknown_hours.insert(event.hour);
                }
                continue;
            };
            if summary.actor.player != event.player
                || event.qpc < summary.actor.since_qpc
                || departures
                    .get(&actor_key)
                    .is_some_and(|&end| event.qpc > end)
            {
                return Err(invalid("command actor outside its recorded lifetime"));
            }
            let Some(kind) = kinds.get(&(key, event.kind)) else {
                if event.paused {
                    window.paused_unknown_hours.insert(event.hour);
                } else {
                    window.complete = false;
                    window.unknown_hours.insert(event.hour);
                }
                continue;
            };
            if event.origin == Origin::AutonomousCountry {
                continue;
            }
            let history = &window.actors[&event.actor];
            let version = history
                .partition_point(|actor| actor.at_qpc <= event.qpc)
                .checked_sub(1)
                .ok_or_else(|| invalid("command precedes its player observation"))?;
            window
                .counts
                .entry((event.actor, version))
                .or_default()
                .observe(&event, kind.category)?;
            if event.paused {
                if kind.category == Category::Other {
                    summary.paused_unresolved_commands += 1;
                }
                if kind.category.counted() {
                    if event.origin == Origin::PlayerOrder {
                        *summary.paused_orders.as_mut().unwrap() += 1;
                    } else {
                        summary.paused_unresolved_commands += 1;
                    }
                }
                continue;
            }
            summary.unclassified_commands += u64::from(kind.category == Category::Other);
            if !kind.category.counted() {
                continue;
            }
            summary.commands += 1;
            if event.origin == Origin::PlayerOrder {
                summary.confirmed_orders += 1;
                *summary.categories.entry(kind.category).or_default() += 1;
            } else {
                summary.unresolved_commands += 1;
                *summary
                    .unresolved_kinds
                    .entry(kind.name.clone())
                    .or_default() += 1;
            }
        }
    }
    for window in windows.values_mut() {
        window.timing = index::Timing::new(&window.spans)?;
    }
    for ((session, capture, world, actor), summary) in &mut summaries {
        let frequency = sessions
            .iter()
            .find(|entry| entry.id == *session)
            .map(|entry| entry.qpc_frequency)
            .filter(|&value| value != 0)
            .ok_or_else(|| invalid("command session frequency missing"))?;
        let window = &windows[&(*session, *capture, *world)];
        let until = departures
            .get(&(*session, *capture, *world, *actor))
            .copied()
            .unwrap_or(u64::MAX);
        let mut ticks = 0_u64;
        let history = &window.actors[actor];
        for (version, observation) in history.iter().enumerate() {
            if observation.country == 0 {
                continue;
            }
            let end = history
                .get(version + 1)
                .map_or(until, |next| next.at_qpc.min(until));
            let duration = window
                .timing
                .between(&window.spans, observation.at_qpc, end);
            ticks = ticks
                .checked_add(
                    u64::try_from(duration).map_err(|_| invalid("command duration overflow"))?,
                )
                .ok_or_else(|| invalid("command duration overflow"))?;
        }
        let minutes = ticks as f64 / frequency as f64 / 60.0;
        summary.observed_minutes = window.complete.then_some(minutes);
        summary.coverage_complete = window.complete;
        if !window.complete
            || !window.paused_unknown_hours.is_empty()
            || summary.paused_unresolved_commands != 0
        {
            summary.paused_orders = None;
        }
        summary.confirmed_orders_per_minute =
            (window.complete && ticks != 0).then(|| summary.confirmed_orders as f64 / minutes);
        summary.apm = summary
            .confirmed_orders_per_minute
            .filter(|_| summary.unresolved_commands == 0 && summary.unclassified_commands == 0);
    }
    let analysis = Analysis {
        recorded: !limits.is_empty(),
        lost_observations,
        unattributed_commands,
        players: summaries
            .into_iter()
            .filter(|(key, summary)| {
                let until = departures.get(key).copied().unwrap_or(u64::MAX);
                windows[&(key.0, key.1, key.2)]
                    .intervals
                    .iter()
                    .any(|&(from, to)| summary.actor.since_qpc <= to && until >= from)
            })
            .map(|(_, summary)| summary)
            .collect(),
    };
    let mut timelines = engine_allocation::vec(windows.len())?;
    for ((session, capture, world), window) in windows {
        let frequency = sessions
            .iter()
            .find(|entry| entry.id == session)
            .map(|entry| entry.qpc_frequency)
            .filter(|&value| value != 0)
            .ok_or_else(|| invalid("command session frequency missing"))?;
        let actor_departures = departures
            .iter()
            .filter_map(|(&(s, c, w, actor), &time)| {
                ((s, c, w) == (session, capture, world)).then_some((actor, time))
            })
            .collect();
        let mut timeline = Timeline {
            session,
            capture,
            world,
            frequency,
            complete: window.complete,
            spans: window.spans,
            actors: window.actors,
            departures: actor_departures,
            counts: window.counts,
            unknown_hours: window.unknown_hours,
            paused_unknown_hours: window.paused_unknown_hours,
            timing: window.timing,
        };
        timeline.prepare()?;
        timelines.push(timeline);
    }
    Ok(Index {
        analysis,
        timelines,
    })
}
