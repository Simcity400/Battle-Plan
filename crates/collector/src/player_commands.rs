use crate::{
    clock::qpc,
    hook::{Hook, Registers, Tap},
    shared_lane::SharedLane,
};
use engine_executable::executable_tables::commands::{self, Target, Token};
use engine_native::{
    Memory, cgame_state, chuman, command_inputs as native, cpdx_array::Array, memory::LiveMemory,
    read,
};
use replay::commands::{Actor, Batch, Category, Event, Kind, Origin, Span};
use std::{
    borrow::Cow,
    cell::UnsafeCell,
    collections::{BTreeMap, BTreeSet},
    io,
    ops::Range,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence},
    },
};

mod classes;

#[derive(Clone, Copy, Default)]
struct Entry {
    order: u64,
    world: u64,
    qpc: u64,
    actor: u64,
    player: i32,
    country: u32,
    sequence: u32,
    kind: u32,
    hour: i32,
    code: u8,
    origin: u8,
    target_present: bool,
    paused: bool,
}

#[derive(Clone, Copy, Default)]
struct ActorEntry {
    order: u64,
    world: u64,
    actor: u64,
    player: i32,
    country: u32,
    since_qpc: u64,
    at_qpc: u64,
    user_len: usize,
    name_len: usize,
    tag_len: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct WorldContext {
    pub campaign: [u8; 36],
    pub hour: i32,
    pub load_signal: i64,
    pub qpc: u64,
}

#[derive(Clone, Copy)]
enum Metadata {
    Actor(ActorEntry),
    World {
        order: u64,
        world: u64,
        context: WorldContext,
    },
}

impl Default for Metadata {
    fn default() -> Self {
        Self::Actor(ActorEntry::default())
    }
}

impl Metadata {
    fn order(&self) -> u64 {
        match self {
            Self::Actor(entry) => entry.order,
            Self::World { order, .. } => *order,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CachedPlayer {
    key: i32,
    actor: u64,
    country: u32,
    seen: bool,
    since_qpc: u64,
}

struct Descriptor {
    vtable: usize,
    id: u32,
    category: Category,
    target: Target,
    roster: bool,
    remove_player: bool,
}

struct Producer {
    world: u64,
    session: usize,
    state: usize,
    active: bool,
    hour: i32,
    paused: bool,
    dirty: bool,
    roster: (usize, u32),
    players: Vec<CachedPlayer>,
    previous_players: Vec<CachedPlayer>,
    next_actor: u64,
    next_order: u64,
}

struct Live {
    base: usize,
    capture: u64,
    kinds: Vec<Descriptor>,
    catalog: Vec<Kind>,
    inputs: native::Inputs,
    events: UnsafeCell<SharedLane<Entry>>,
    metadata: UnsafeCell<SharedLane<Metadata>>,
    producer: UnsafeCell<Producer>,
    occupied: AtomicBool,
    version: AtomicU64,
    closed_qpc: AtomicU64,
    closed_order: AtomicU64,
    closed_world: AtomicU64,
    failed: AtomicU64,
    failed_qpc: AtomicU64,
    capacity_demand: AtomicUsize,
    name_demand: AtomicUsize,
    player_capacity: AtomicUsize,
    buffer_bytes: AtomicUsize,
}

// The callback's exclusive, nonwaiting admission owns producer; lanes publish immutable records.
unsafe impl Sync for Live {}
unsafe impl Send for Live {}

static LIVE: AtomicPtr<Live> = AtomicPtr::new(std::ptr::null_mut());
static STORE: Mutex<Option<Box<Live>>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static UNHOOKED: OnceLock<Cow<'static, str>> = OnceLock::new();
static CAPTURED: AtomicU64 = AtomicU64::new(0);
static AUTOMATIC: AtomicU64 = AtomicU64::new(0);
static UNRESOLVED_FIELDS: AtomicU64 = AtomicU64::new(0);

impl Live {
    fn new(memory: &impl Memory, base: usize) -> io::Result<Self> {
        let tables = &engine_executable::executable_tables::get().commands;
        let mut kinds = engine_allocation::vec(tables.kinds.len())?;
        let mut catalog = engine_allocation::vec(tables.kinds.len())?;
        engine_allocation::reserve(&mut catalog, 1)?;
        catalog.push(Kind {
            id: 0,
            token: 0,
            name: engine_allocation::string("Unclassified command")?,
            category: Category::Other,
        });
        let mut known = engine_allocation::vec(
            classes::CATEGORIES
                .iter()
                .map(|(_, names)| names.len())
                .sum(),
        )?;
        for &(category, names) in &classes::CATEGORIES {
            known.extend(names.iter().map(|&name| (name, category, false)));
        }
        known.sort_unstable_by_key(|&(name, ..)| name);
        if known.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "player command classes repeat",
            ));
        }
        for (index, native) in tables.kinds.iter().enumerate() {
            let id = u32::try_from(index + 1).map_err(|_| io::ErrorKind::InvalidData)?;
            let name = commands::class_hash(&native.name);
            let class = known
                .binary_search_by_key(&name, |&(name, ..)| name)
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("player command kind unsupported: {}", native.name),
                    )
                })?;
            known[class].2 = true;
            let category = known[class].1;
            if category.counted() && matches!(native.target, Target::Unknown) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("player command ownership unsupported: {}", native.name),
                ));
            }
            kinds.push(Descriptor {
                vtable: base + native.vtable as usize,
                id,
                category,
                target: native.target,
                roster: classes::ROSTER.contains(&name),
                remove_player: name == classes::REMOVE_PLAYER,
            });
            let token = match native.token {
                Token::Constant(value) => value,
                Token::Word(offset) => {
                    read(memory, base + offset as usize).ok_or(io::ErrorKind::InvalidData)?
                }
            };
            catalog.push(Kind {
                id,
                token,
                name: engine_allocation::string(&native.name)?,
                category,
            });
        }
        if known.iter().any(|&(.., used)| !used) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "player command classes name kinds this executable lacks",
            ));
        }
        let state = read::<u64>(memory, base + cgame_state::SINGLETON_RVA).unwrap_or(0) as usize;
        let roster = Array::read(
            memory,
            state.saturating_add(cgame_state::PLAYER_RECORDS),
            chuman::STRIDE,
        );
        let count = roster.map_or(0, |array| array.count as usize);
        let slots = count
            .max(1)
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(engine_allocation::oom)?;
        let mut names = 0;
        if let Some(roster) = roster {
            for index in 0..roster.count {
                let at = roster
                    .element(index, chuman::STRIDE)
                    .ok_or(io::ErrorKind::InvalidData)?;
                if let Ok(player) = native::player_at(memory, base, at) {
                    let tag = native::country_tag(memory, base, state, player.country)
                        .map_err(io::Error::other)?;
                    names = names.max(
                        player
                            .user
                            .len
                            .checked_add(player.name.len)
                            .and_then(|len| len.checked_add(tag.map_or(0, |tag| tag.len)))
                            .ok_or_else(engine_allocation::oom)?,
                    );
                }
            }
        }
        let mut players = engine_allocation::vec(slots)?;
        players.resize(slots, CachedPlayer::default());
        let events = SharedLane::new(16 * 1024, 0)?;
        let actors = SharedLane::new(
            slots.checked_mul(2).ok_or_else(engine_allocation::oom)?,
            names.max(64),
        )?;
        let buffer_bytes = events.allocated_bytes() + actors.allocated_bytes();
        Ok(Self {
            base,
            capture: u64::try_from(qpc())
                .ok()
                .filter(|&time| time != 0)
                .ok_or(io::ErrorKind::InvalidData)?,
            kinds,
            catalog,
            inputs: native::Inputs::new(memory, base)?,
            events: UnsafeCell::new(events),
            metadata: UnsafeCell::new(actors),
            producer: UnsafeCell::new(Producer {
                world: 0,
                session: 0,
                state: 0,
                active: false,
                hour: 0,
                paused: false,
                dirty: true,
                roster: (0, 0),
                previous_players: engine_allocation::copy_vec(&players)?,
                players,
                next_actor: 1,
                next_order: 1,
            }),
            occupied: AtomicBool::new(false),
            version: AtomicU64::new(0),
            closed_qpc: AtomicU64::new(0),
            closed_order: AtomicU64::new(0),
            closed_world: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            failed_qpc: AtomicU64::new(0),
            capacity_demand: AtomicUsize::new(slots),
            name_demand: AtomicUsize::new(names),
            player_capacity: AtomicUsize::new(slots),
            buffer_bytes: AtomicUsize::new(buffer_bytes),
        })
    }

    fn events(&self) -> &SharedLane<Entry> {
        unsafe { &*self.events.get() }
    }
    fn metadata(&self) -> &SharedLane<Metadata> {
        unsafe { &*self.metadata.get() }
    }

    fn grow(&self) -> io::Result<(bool, bool)> {
        let slots = self
            .capacity_demand
            .load(Ordering::Relaxed)
            .max(1)
            .checked_next_power_of_two()
            .ok_or_else(engine_allocation::oom)?;
        let player_capacity = self.player_capacity.load(Ordering::Relaxed);
        let events = self.events();
        let event_capacity = events.required_entries().max(events.len());
        let next_events = (event_capacity > events.len())
            .then(|| SharedLane::new(event_capacity, 0))
            .transpose()?;
        let actors = self.metadata();
        let actor_capacity = actors
            .required_entries()
            .max(actors.len())
            .max(slots.checked_mul(2).ok_or_else(engine_allocation::oom)?);
        let stride = self
            .name_demand
            .load(Ordering::Relaxed)
            .max(actors.stride());
        let next_actors = (actor_capacity > actors.len() || stride > actors.stride())
            .then(|| SharedLane::new(actor_capacity, stride))
            .transpose()?;
        let mut players = (slots > player_capacity)
            .then(|| engine_allocation::default_vec::<CachedPlayer>(slots))
            .transpose()?;
        let mut previous = (slots > player_capacity)
            .then(|| engine_allocation::default_vec::<CachedPlayer>(slots))
            .transpose()?;
        if next_events.is_none() && next_actors.is_none() && players.is_none() {
            return Ok((false, false));
        }
        if self
            .occupied
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Ok((false, false));
        }
        self.version.fetch_add(1, Ordering::AcqRel);
        let mut retired_events = None;
        let mut retired_actors = None;
        let mut changed = (false, false);
        if self.events().is_drained() && self.metadata().is_drained() {
            let producer = unsafe { &mut *self.producer.get() };
            if let Some(mut next) = players.take() {
                for &player in &producer.players {
                    if player.actor == 0 || !player.seen {
                        continue;
                    }
                    let mut index = hash(player.key as u32 as u64, next.len());
                    while next[index].actor != 0 {
                        index = (index + 1) & (next.len() - 1);
                    }
                    next[index] = player;
                }
                players = Some(std::mem::replace(&mut producer.players, next));
                previous = Some(std::mem::replace(
                    &mut producer.previous_players,
                    previous.take().unwrap(),
                ));
                self.player_capacity.store(slots, Ordering::Relaxed);
            }
            if let Some(next) = next_events {
                retired_events = Some(unsafe { std::mem::replace(&mut *self.events.get(), next) });
                changed.0 = true;
            }
            if let Some(next) = next_actors {
                retired_actors =
                    Some(unsafe { std::mem::replace(&mut *self.metadata.get(), next) });
                changed.1 = true;
            }
            producer.dirty = true;
            self.buffer_bytes.store(
                self.events().allocated_bytes() + self.metadata().allocated_bytes(),
                Ordering::Relaxed,
            );
        }
        self.version.fetch_add(1, Ordering::Release);
        self.occupied.store(false, Ordering::Release);
        drop((retired_events, retired_actors, players, previous));
        Ok(changed)
    }

    fn fail(&self) {
        self.failed_qpc
            .fetch_max(qpc().max(0) as u64, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Release);
    }

    fn publish(&self, entry: Entry) -> Result<(), &'static str> {
        let slot = self.events().reserve().ok_or("command event buffer full")?;
        slot.publish(entry);
        Ok(())
    }

    fn roster(
        &self,
        memory: &impl Memory,
        producer: &mut Producer,
        order: u64,
        time: u64,
    ) -> Result<(), &'static str> {
        let rows = Array::read(
            memory,
            producer.state + cgame_state::PLAYER_RECORDS,
            chuman::STRIDE,
        )
        .ok_or("command roster invalid")?;
        if !producer.dirty && producer.roster == (rows.data, rows.count) {
            return Ok(());
        }
        self.capacity_demand
            .fetch_max(rows.count as usize * 2, Ordering::Relaxed);
        if rows.count as usize > producer.players.len() / 2 {
            return Err("command player storage needs growth");
        }
        let departures = producer
            .players
            .iter()
            .filter(|player| player.actor != 0 && player.seen)
            .count();
        if self.events().len().saturating_sub(self.events().pending()) < departures + 1 {
            self.events()
                .require_entries(self.events().pending().saturating_add(departures + 1));
            return Err("command roster commit buffer full");
        }
        producer.previous_players.fill(CachedPlayer::default());
        for index in 0..rows.count {
            let pointer = rows
                .element(index, chuman::STRIDE)
                .ok_or("command player offset overflow")?;
            let player = native::player_at(memory, self.base, pointer)?;
            let key = player.key;
            if key < 0 {
                continue;
            }
            let mut previous = hash(key as u32 as u64, producer.players.len());
            let mut previous_actor = 0;
            let mut since = time;
            for _ in 0..producer.players.len() {
                let slot = producer.players[previous];
                if slot.actor == 0 {
                    break;
                }
                if slot.key == key {
                    if slot.seen {
                        previous_actor = slot.actor;
                        since = slot.since_qpc;
                    }
                    break;
                }
                previous = (previous + 1) & (producer.players.len() - 1);
            }
            let mut index = hash(key as u32 as u64, producer.previous_players.len());
            for _ in 0..producer.previous_players.len() {
                let slot = &mut producer.previous_players[index];
                if slot.actor == 0 || slot.key == key {
                    if slot.seen {
                        return Err("duplicate command player key");
                    }
                    let actor = if previous_actor == 0 {
                        let id = producer.next_actor;
                        producer.next_actor = id
                            .checked_add(1)
                            .ok_or("command actor sequence exhausted")?;
                        id
                    } else {
                        previous_actor
                    };
                    let needed = player
                        .user
                        .len
                        .checked_add(player.name.len)
                        .ok_or("command player text overflow")?;
                    let tag =
                        native::country_tag(memory, self.base, producer.state, player.country)?;
                    let text_len = needed;
                    let needed = needed
                        .checked_add(tag.map_or(0, |tag| tag.len))
                        .ok_or("command country tag length overflow")?;
                    self.name_demand.fetch_max(needed, Ordering::Relaxed);
                    let mut reservation = self
                        .metadata()
                        .reserve()
                        .ok_or("command player buffer full")?;
                    if needed > reservation.stride() {
                        return Err("command player text needs growth");
                    }
                    let bytes =
                        unsafe { std::slice::from_raw_parts_mut(reservation.bytes(), needed) };
                    player.user.copy(memory, &mut bytes[..player.user.len])?;
                    player
                        .name
                        .copy(memory, &mut bytes[player.user.len..text_len])?;
                    if let Some(tag) = tag {
                        tag.copy(memory, &mut bytes[text_len..])?;
                    }
                    reservation.publish(Metadata::Actor(ActorEntry {
                        order,
                        world: producer.world,
                        actor,
                        player: key,
                        country: player.country,
                        since_qpc: since,
                        at_qpc: time,
                        user_len: player.user.len,
                        name_len: player.name.len,
                        tag_len: tag.map_or(0, |tag| tag.len),
                    }));
                    *slot = CachedPlayer {
                        key,
                        actor,
                        country: player.country,
                        seen: true,
                        since_qpc: since,
                    };
                    break;
                }
                index = (index + 1) & (producer.previous_players.len() - 1);
            }
        }
        for old in &producer.players {
            if old.actor == 0 || !old.seen {
                continue;
            }
            let mut index = hash(old.key as u32 as u64, producer.previous_players.len());
            let mut retained = false;
            for _ in 0..producer.previous_players.len() {
                let current = producer.previous_players[index];
                if current.actor == 0 {
                    break;
                }
                if current.key == old.key {
                    retained = current.actor == old.actor;
                    break;
                }
                index = (index + 1) & (producer.previous_players.len() - 1);
            }
            if !retained {
                self.publish(Entry {
                    order,
                    world: producer.world,
                    qpc: time,
                    actor: old.actor,
                    code: 3,
                    ..Entry::default()
                })?;
            }
        }
        self.publish(Entry {
            order,
            world: producer.world,
            qpc: time,
            code: 4,
            ..Entry::default()
        })?;
        std::mem::swap(&mut producer.players, &mut producer.previous_players);
        producer.roster = (rows.data, rows.count);
        producer.dirty = false;
        Ok(())
    }

    fn mode(
        &self,
        memory: &impl Memory,
        producer: &mut Producer,
        world: u64,
        session: usize,
        time: u64,
        order: u64,
    ) -> Result<(), &'static str> {
        let state = read::<i32>(
            memory,
            session
                .checked_add(native::SESSION_STATE)
                .ok_or("command session overflow")?,
        )
        .ok_or("command session state unreadable")?;
        let active = world != 0 && matches!(state, 2 | 11 | 12);
        if producer.active && (!active || producer.world != world || producer.session != session) {
            let departures = producer
                .players
                .iter()
                .filter(|player| player.actor != 0 && player.seen)
                .count();
            if self.events().len().saturating_sub(self.events().pending()) < departures + 1 {
                self.events()
                    .require_entries(self.events().pending().saturating_add(departures + 1));
                return Err("command phase boundary buffer full");
            }
            for player in &producer.players {
                if player.actor != 0 && player.seen {
                    self.publish(Entry {
                        order,
                        world: producer.world,
                        qpc: time,
                        actor: player.actor,
                        code: 3,
                        ..Entry::default()
                    })?;
                }
            }
            producer.active = false;
            producer.dirty = true;
            self.publish(Entry {
                order,
                world: producer.world,
                qpc: time,
                code: 2,
                ..Entry::default()
            })?;
        }
        if active && !producer.active {
            producer.players.fill(CachedPlayer::default());
            producer.previous_players.fill(CachedPlayer::default());
            producer.dirty = true;
            producer.world = world;
            producer.session = session;
            producer.state = read::<u64>(memory, self.base + cgame_state::SINGLETON_RVA)
                .filter(|&pointer| pointer != 0)
                .ok_or("command world unreadable")? as usize;
            let campaign = engine_native::msvc_string::read_text::<36>(
                memory,
                producer.state + cgame_state::CAMPAIGN_ID,
            )
            .filter(|text| text.len == 36)
            .ok_or("command campaign identity unreadable")?;
            let context = WorldContext {
                campaign: campaign.bytes,
                hour: read(memory, producer.state + cgame_state::CURRENT_HOUR)
                    .ok_or("command phase hour unreadable")?,
                load_signal: read(memory, self.base + cgame_state::PLAY_SESSION_STARTED_AT_RVA)
                    .ok_or("command phase load signal unreadable")?,
                qpc: time,
            };
            self.metadata()
                .reserve()
                .ok_or("command world context buffer full")?
                .publish(Metadata::World {
                    order,
                    world,
                    context,
                });
            producer.hour = context.hour;
            producer.paused = native::paused(memory, self.base)?;
            self.publish(Entry {
                order,
                world,
                qpc: time,
                code: 1,
                hour: producer.hour,
                paused: producer.paused,
                ..Entry::default()
            })?;
            producer.active = true;
        }
        Ok(())
    }

    fn observe(&self, registers: &Registers, site: usize) -> Result<(), &'static str> {
        let memory = unsafe { LiveMemory::new() };
        let time = u64::try_from(qpc()).map_err(|_| "command clock negative")?;
        let producer = unsafe { &mut *self.producer.get() };
        let order = producer.next_order;
        producer.next_order = order
            .checked_add(1)
            .ok_or("command observation sequence exhausted")?;
        let session = match site {
            0 => registers.rcx,
            1 => registers.r14,
            2 => registers.rsi,
            _ if producer.active => producer.session,
            _ => return Ok(()),
        };
        // Accepted commands are live before the first hourly snapshot, including while paused.
        let world = crate::world_lifetime::observed_generation().unwrap_or(0);
        self.mode(&memory, producer, world, session, time, order)?;
        if producer.active {
            let hour = read(&memory, producer.state + cgame_state::CURRENT_HOUR)
                .ok_or("command hour unreadable")?;
            let paused = native::paused(&memory, self.base)?;
            if hour != producer.hour || paused != producer.paused {
                self.publish(Entry {
                    order,
                    world,
                    qpc: time,
                    hour,
                    paused,
                    code: 5,
                    ..Entry::default()
                })?;
                producer.hour = hour;
                producer.paused = paused;
            }
            let shared = crate::sampling::SHARED.load(Ordering::Acquire);
            if !shared.is_null() {
                let shared = unsafe { &*shared };
                if !shared.roster_ready.load(Ordering::Acquire) {
                    let hour = read(&memory, producer.state + cgame_state::CURRENT_HOUR);
                    shared.sample_content_roster(&memory, hour);
                }
            }
            if site == 0 {
                self.roster(&memory, producer, order, time)?;
            } else if site < 3 {
                let command = if site == 1 {
                    read::<u64>(&memory, registers.rsi)
                        .ok_or("accepted command pointer unreadable")? as usize
                } else {
                    registers.rdi
                };
                let header = native::header(&memory, command)?;
                let kind = self
                    .kinds
                    .binary_search_by_key(&header.vtable, |kind| kind.vtable)
                    .ok()
                    .map(|index| &self.kinds[index]);
                if kind.is_some_and(|kind| kind.roster) {
                    producer.dirty = true;
                }
                if kind.is_some_and(|kind| kind.remove_player) {
                    let removed = read::<i32>(&memory, command + native::REMOVED_PLAYER_KEY)
                        .ok_or("removed player key unreadable")?;
                    for player in &mut producer.players {
                        if player.actor != 0 && player.key == removed && player.seen {
                            self.publish(Entry {
                                order,
                                world: producer.world,
                                qpc: time,
                                actor: player.actor,
                                code: 3,
                                ..Entry::default()
                            })?;
                            player.seen = false;
                            break;
                        }
                    }
                }
                if !kind.is_some_and(|kind| {
                    matches!(
                        kind.category,
                        Category::System | Category::Session | Category::Automatic
                    )
                }) {
                    let roster_ready = self.roster(&memory, producer, order, time).is_ok();
                    if !roster_ready {
                        self.fail();
                    }
                    let mut found = CachedPlayer::default();
                    let mut index = hash(header.player as u32 as u64, producer.players.len());
                    for _ in 0..producer.players.len() {
                        let player = producer.players[index];
                        if player.actor == 0 {
                            break;
                        }
                        if player.key == header.player {
                            if roster_ready && player.seen {
                                found = player;
                            }
                            break;
                        }
                        index = (index + 1) & (producer.players.len() - 1);
                    }
                    let resolution = native::target(
                        &memory,
                        self.base,
                        producer.state,
                        &self.inputs,
                        command,
                        kind.map_or(Target::Unknown, |kind| kind.target),
                        header.player,
                        found.country,
                    )
                    .unwrap_or_else(|_| {
                        UNRESOLVED_FIELDS.fetch_add(1, Ordering::Relaxed);
                        native::Resolution::Unknown
                    });
                    let target = if let native::Resolution::Country(country) = resolution {
                        Some(country)
                    } else {
                        None
                    };
                    let origin = match resolution {
                        native::Resolution::Country(country) => {
                            match native::autonomous(&memory, self.base, producer.state, country) {
                                Ok(true) => Origin::AutonomousCountry,
                                Ok(false)
                                    if found.actor != 0
                                        && native::manual_country(
                                            &memory,
                                            self.base,
                                            producer.state,
                                            country,
                                            found.country,
                                        )
                                        .unwrap_or(false) =>
                                {
                                    Origin::PlayerOrder
                                }
                                _ => Origin::Unresolved,
                            }
                        }
                        native::Resolution::Automatic => Origin::AutonomousCountry,
                        native::Resolution::Unknown => Origin::Unresolved,
                    };
                    if origin == Origin::AutonomousCountry {
                        AUTOMATIC.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.publish(Entry {
                            order,
                            world: producer.world,
                            qpc: time,
                            actor: found.actor,
                            player: header.player,
                            country: target.unwrap_or(0),
                            sequence: header.sequence,
                            kind: kind.map_or(0, |kind| kind.id),
                            hour: producer.hour,
                            paused: producer.paused,
                            origin: origin as u8,
                            target_present: target.is_some(),
                            ..Entry::default()
                        })?;
                        CAPTURED.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        self.closed_qpc.store(time, Ordering::Relaxed);
        self.closed_order.store(order, Ordering::Relaxed);
        self.closed_world.store(
            if producer.active { producer.world } else { 0 },
            Ordering::Relaxed,
        );
        Ok(())
    }
}

fn hash(key: u64, slots: usize) -> usize {
    crate::table_hash::slot(key, slots)
}

unsafe extern "system-unwind" fn observe(registers: &Registers, site: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::PlayerCommands);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    struct Call;
    impl Drop for Call {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _call = Call;
    if !ENABLED.load(Ordering::Acquire) || crate::allocation::failed() {
        return;
    }
    let pointer = LIVE.load(Ordering::Acquire);
    if pointer.is_null() {
        return;
    }
    let live = unsafe { &*pointer };
    if live
        .occupied
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        live.fail();
        return;
    }
    struct Writer<'a>(&'a Live);
    impl Drop for Writer<'_> {
        fn drop(&mut self) {
            self.0.version.fetch_add(1, Ordering::Release);
            self.0.occupied.store(false, Ordering::Release);
        }
    }
    live.version.fetch_add(1, Ordering::AcqRel);
    let _writer = Writer(live);
    if live.observe(registers, site).is_err() {
        live.fail();
        let producer = unsafe { &mut *live.producer.get() };
        live.closed_qpc
            .store(qpc().max(0) as u64, Ordering::Relaxed);
        live.closed_order
            .store(producer.next_order.saturating_sub(1), Ordering::Relaxed);
        live.closed_world.store(
            if producer.active { producer.world } else { 0 },
            Ordering::Relaxed,
        );
    }
}

pub(crate) fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

pub(crate) fn urgent() -> bool {
    let pointer = LIVE.load(Ordering::Acquire);
    if pointer.is_null() {
        return false;
    }
    let live = unsafe { &*pointer };
    live.events().pending() >= live.events().len() * 3 / 4
        || live.metadata().pending() >= live.metadata().len() * 3 / 4
}

pub(crate) fn status() -> impl serde::Serialize {
    use crate::status::status;
    let pointer = LIVE.load(Ordering::Acquire);
    let live = (!pointer.is_null()).then(|| unsafe { &*pointer });
    status!({ "enabled": ENABLED.load(Ordering::Acquire), "unhooked": UNHOOKED.get().map(|reason| reason.as_ref()), "commands": CAPTURED.load(Ordering::Relaxed), "autonomous_excluded": AUTOMATIC.load(Ordering::Relaxed), "unresolved_fields": UNRESOLVED_FIELDS.load(Ordering::Relaxed), "refusals": live.map_or(0, |live| live.failed.load(Ordering::Relaxed)), "buffer_bytes": live.map_or(0, |live| live.buffer_bytes.load(Ordering::Relaxed)), "scope": "player-issued synchronized orders; individual session keys; automatic execution excluded; ambiguous provenance retained" })
}

pub(crate) struct Drained {
    pub capture: u64,
    pub world: u64,
    pub from: u64,
    pub to: u64,
    pub lost: u64,
    pub context: Option<WorldContext>,
    pub batch: Batch,
}

#[derive(Default)]
pub(crate) struct Drain {
    events: Vec<(u64, u64)>,
    actors: Vec<(u64, u64)>,
    windows: BTreeMap<u64, Option<Span>>,
    failure: u64,
    loss_until: u64,
    event_refused: u64,
    actor_refused: u64,
    last_qpc: u64,
    known_actors: BTreeMap<(u64, u64), Actor>,
    retired: BTreeSet<(u64, u64)>,
    contexts: BTreeMap<u64, WorldContext>,
}

impl Drain {
    pub(crate) fn take(&mut self, final_drain: bool) -> io::Result<Vec<Drained>> {
        let pointer = LIVE.load(Ordering::Acquire);
        if pointer.is_null() {
            return Ok(Vec::new());
        }
        let live = unsafe { &*pointer };
        self.take_live(live, final_drain)
    }

    fn take_live(&mut self, live: &Live, final_drain: bool) -> io::Result<Vec<Drained>> {
        let version = live.version.load(Ordering::Acquire);
        if version & 1 != 0 {
            return if final_drain {
                Err(io::Error::other(
                    "command observer is still active at final drain",
                ))
            } else {
                Ok(Vec::new())
            };
        }
        let (time, order, world) = (
            live.closed_qpc.load(Ordering::Relaxed),
            live.closed_order.load(Ordering::Relaxed),
            live.closed_world.load(Ordering::Relaxed),
        );
        fence(Ordering::Acquire);
        if version != live.version.load(Ordering::Relaxed) {
            return if final_drain {
                Err(io::Error::other(
                    "command observer changed during final drain",
                ))
            } else {
                Ok(Vec::new())
            };
        }
        if time == 0 {
            return Ok(Vec::new());
        }
        live.events()
            .published_into(&mut self.events, |entry| entry.order)
            .map_err(io::Error::other)?;
        live.metadata()
            .published_into(&mut self.actors, Metadata::order)
            .map_err(io::Error::other)?;
        let mut groups: BTreeMap<u64, Drained> = BTreeMap::new();
        let committed: BTreeSet<_> = self
            .events
            .iter()
            .filter_map(|&(_, index)| {
                let (entry, _) = live.events().record(index);
                (entry.order <= order && entry.code == 4).then_some((entry.world, entry.order))
            })
            .collect();
        for &(_, index) in &self.actors {
            let (entry, bytes) = live.metadata().record(index);
            if entry.order() > order {
                continue;
            }
            let entry = match entry {
                Metadata::World {
                    order,
                    world,
                    context,
                } => {
                    if self.events.iter().any(|&(_, index)| {
                        let (entry, _) = live.events().record(index);
                        entry.order == order && entry.world == world && entry.code == 1
                    }) {
                        self.contexts.insert(world, context);
                    }
                    live.metadata().release(index);
                    continue;
                }
                Metadata::Actor(entry) => entry,
            };
            if !committed.contains(&(entry.world, entry.order)) {
                live.metadata().release(index);
                continue;
            }
            if self
                .known_actors
                .get(&(entry.world, entry.actor))
                .is_some_and(|old| {
                    old.player == entry.player
                        && old.country == entry.country
                        && old.since_qpc == entry.since_qpc
                        && old.user == bytes[..entry.user_len]
                        && old.name == bytes[entry.user_len..entry.user_len + entry.name_len]
                        && old.tag
                            == bytes[entry.user_len + entry.name_len
                                ..entry.user_len + entry.name_len + entry.tag_len]
                })
            {
                live.metadata().release(index);
                continue;
            }
            let group = groups.entry(entry.world).or_insert_with(|| Drained {
                capture: live.capture,
                world: entry.world,
                from: self.last_qpc.min(time),
                to: time,
                lost: 0,
                context: None,
                batch: Batch::default(),
            });
            let actor = Actor {
                id: entry.actor,
                player: entry.player,
                country: entry.country,
                since_qpc: entry.since_qpc,
                at_qpc: entry.at_qpc,
                user: engine_allocation::copy_vec(&bytes[..entry.user_len])?,
                name: engine_allocation::copy_vec(
                    &bytes[entry.user_len..entry.user_len + entry.name_len],
                )?,
                tag: engine_allocation::copy_vec(
                    &bytes[entry.user_len + entry.name_len
                        ..entry.user_len + entry.name_len + entry.tag_len],
                )?,
            };
            self.known_actors
                .insert((entry.world, entry.actor), actor.clone());
            group.batch.actors.push(actor);
            live.metadata().release(index);
        }
        for &(_, index) in &self.events {
            let (entry, _) = live.events().record(index);
            if entry.order > order {
                continue;
            }
            let group = groups.entry(entry.world).or_insert_with(|| Drained {
                capture: live.capture,
                world: entry.world,
                from: if self.last_qpc == 0 {
                    entry.qpc
                } else {
                    self.last_qpc
                },
                to: time,
                lost: 0,
                context: None,
                batch: Batch::default(),
            });
            match entry.code {
                4 => {}
                3 => {
                    if self.retired.insert((entry.world, entry.actor)) {
                        group.batch.departures.push((entry.actor, entry.qpc));
                    }
                }
                1 | 2 | 5 => {
                    let next = (entry.code != 2).then_some(Span {
                        from_qpc: entry.qpc,
                        to_qpc: entry.qpc,
                        complete: true,
                        hour: entry.hour,
                        paused: entry.paused,
                    });
                    if let Some(Some(previous)) = self.windows.insert(entry.world, next) {
                        group.batch.spans.push(Span {
                            from_qpc: previous.from_qpc.max(group.from),
                            to_qpc: entry.qpc,
                            ..previous
                        });
                    }
                }
                _ => group.batch.events.push(Event {
                    qpc: entry.qpc,
                    hour: entry.hour,
                    actor: entry.actor,
                    player: entry.player,
                    kind: entry.kind,
                    command_country: entry.target_present.then_some(entry.country),
                    origin: match entry.origin {
                        1 => Origin::PlayerOrder,
                        2 => Origin::AutonomousCountry,
                        _ => Origin::Unresolved,
                    },
                    sequence: entry.sequence,
                    paused: entry.paused,
                }),
            }
            live.events().release(index);
        }
        let failed = live.failed.load(Ordering::Acquire);
        if failed != self.failure {
            self.loss_until = self.loss_until.max(live.failed_qpc.load(Ordering::Relaxed));
        }
        let event_refused = live.events().rejected();
        let actor_refused = live.metadata().rejected();
        let lost = failed.saturating_sub(self.failure);
        if world != 0 {
            groups.entry(world).or_insert_with(|| Drained {
                capture: live.capture,
                world,
                from: self.last_qpc.min(time),
                to: time,
                lost: 0,
                context: None,
                batch: Batch::default(),
            });
        }
        for group in groups.values_mut() {
            group.context = self.contexts.get(&group.world).copied();
            group.lost = lost;
            if let Some(Some(window)) = self.windows.get_mut(&group.world) {
                group.batch.spans.push(Span {
                    from_qpc: window.from_qpc.max(group.from),
                    to_qpc: time,
                    complete: lost == 0 && (self.loss_until == 0 || self.loss_until < group.from),
                    ..*window
                });
                window.from_qpc = time;
            }
            if lost != 0 || (self.loss_until != 0 && self.loss_until >= group.from) {
                for span in &mut group.batch.spans {
                    span.complete = false;
                }
            }
        }
        if !groups.is_empty() {
            self.last_qpc = time;
        }
        if !groups.is_empty() {
            self.failure = failed;
        }
        self.event_refused = event_refused;
        self.actor_refused = actor_refused;
        live.events().advance(event_refused);
        live.metadata().advance(actor_refused);
        let changed = live.grow()?;
        if changed.0 {
            self.event_refused = 0;
        }
        if changed.1 {
            self.actor_refused = 0;
        }
        Ok(groups.into_values().collect())
    }

    pub(crate) fn catalog() -> io::Result<Vec<Kind>> {
        let pointer = LIVE.load(Ordering::Acquire);
        if pointer.is_null() {
            return Ok(Vec::new());
        }
        let live = unsafe { &*pointer };
        let mut kinds = engine_allocation::vec(live.catalog.len())?;
        for kind in &live.catalog {
            kinds.push(Kind {
                id: kind.id,
                token: kind.token,
                name: engine_allocation::string(&kind.name)?,
                category: kind.category,
            });
        }
        Ok(kinds)
    }
}

pub struct Hooks {
    taps: Vec<Tap>,
    publication_attempted: bool,
}

impl Hooks {
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Self {
        let result = (|| -> Result<Vec<Tap>, Cow<'static, str>> {
            let mut sentinels = Vec::new();
            for site in &native::SITES {
                let sentinel = read::<[u8; 16]>(memory, base + site.rva)
                    .ok_or("command observer site unreadable")?;
                if native::fingerprint(&sentinel) != site.fingerprint {
                    return Err("command observer site differs from supported build".into());
                }
                sentinels.push(sentinel);
            }
            let live = engine_allocation::boxed(
                Live::new(memory, base).map_err(|error| Cow::Owned(error.to_string()))?,
            )
            .map_err(|_| "command observer storage allocation failed")?;
            let pointer = std::ptr::from_ref(&*live).cast_mut();
            *STORE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(live);
            LIVE.store(pointer, Ordering::Release);
            native::SITES
                .iter()
                .zip(sentinels)
                .enumerate()
                .map(|(index, (site, sentinel))| unsafe {
                    Tap::new(base + site.rva, observe, index, sentinel).map_err(|reason| {
                        Cow::Owned(format!("player command site {index}: {reason}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })();
        Self {
            taps: result.unwrap_or_else(|reason| {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Construction,
                    None,
                    reason.as_ref(),
                );
                let _ = UNHOOKED.set(reason);
                Vec::new()
            }),
            publication_attempted: false,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !crate::hook::shared::defer_preparation(|| ENABLED.store(false, Ordering::SeqCst)) {
            ENABLED.store(false, Ordering::SeqCst);
        }
        let result = match crate::hook::shared::attach_outcome() {
            Some(outcome) => outcome.map_err(|_| "command observer publication failed")?,
            None => {
                self.publication_attempted |= enabled;
                Hook::set_enabled_iter_attach(
                    self.taps.iter_mut().map(Tap::publication_hook),
                    enabled,
                )
                .map_err(|_| "command observer publication failed")?
            }
        };
        if result {
            ENABLED.store(enabled && !self.taps.is_empty(), Ordering::Release);
        }
        Ok(result)
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }

    pub fn release_after_quiet(&mut self) {
        self.publication_attempted = false;
        LIVE.store(std::ptr::null_mut(), Ordering::Release);
        STORE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            self.release_after_quiet();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn live() -> Live {
        Live {
            base: 0,
            capture: 1,
            kinds: Vec::new(),
            catalog: Vec::new(),
            inputs: native::Inputs::default(),
            events: UnsafeCell::new(SharedLane::new(8, 0).unwrap()),
            metadata: UnsafeCell::new(SharedLane::new(4, 8).unwrap()),
            producer: UnsafeCell::new(Producer {
                paused: false,
                hour: 0,
                world: 1,
                session: 1,
                state: 1,
                active: true,
                dirty: false,
                roster: (0, 0),
                players: vec![CachedPlayer::default(); 2],
                previous_players: vec![CachedPlayer::default(); 2],
                next_actor: 1,
                next_order: 2,
            }),
            occupied: AtomicBool::new(false),
            version: AtomicU64::new(0),
            closed_qpc: AtomicU64::new(110),
            closed_order: AtomicU64::new(1),
            closed_world: AtomicU64::new(1),
            failed: AtomicU64::new(0),
            failed_qpc: AtomicU64::new(0),
            capacity_demand: AtomicUsize::new(2),
            name_demand: AtomicUsize::new(0),
            player_capacity: AtomicUsize::new(2),
            buffer_bytes: AtomicUsize::new(0),
        }
    }

    #[test]
    fn incomplete_roster_transaction_cannot_publish_partial_identities() {
        let live = live();
        live.metadata()
            .reserve()
            .unwrap()
            .publish(Metadata::Actor(ActorEntry {
                order: 1,
                world: 1,
                actor: 1,
                since_qpc: 100,
                ..Default::default()
            }));
        live.publish(Entry {
            order: 1,
            world: 1,
            qpc: 100,
            code: 1,
            ..Default::default()
        })
        .unwrap();
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, false).unwrap();
        assert!(groups[0].batch.actors.is_empty());
        assert!(drain.known_actors.is_empty());
        assert!(live.metadata().is_drained());
    }

    #[test]
    fn paused_phase_context_and_commands_drain_before_any_hourly_snapshot() {
        struct Fixture(BTreeMap<usize, Vec<u8>>);
        impl Memory for Fixture {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some((&start, bytes)) = self.0.range(..=address).next_back() else {
                    return false;
                };
                let offset = address - start;
                let Some(source) = bytes.get(offset..offset + output.len()) else {
                    return false;
                };
                output.copy_from_slice(source);
                true
            }
        }
        let live = live();
        let producer = unsafe { &mut *live.producer.get() };
        producer.active = false;
        let mut state = vec![0; 0x800];
        let campaign = *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        state[cgame_state::CAMPAIGN_ID..cgame_state::CAMPAIGN_ID + 8]
            .copy_from_slice(&0x4000_u64.to_le_bytes());
        state[cgame_state::CAMPAIGN_ID + 16..cgame_state::CAMPAIGN_ID + 24]
            .copy_from_slice(&36_u64.to_le_bytes());
        state[cgame_state::CAMPAIGN_ID + 24..cgame_state::CAMPAIGN_ID + 32]
            .copy_from_slice(&47_u64.to_le_bytes());
        state[cgame_state::CURRENT_HOUR..cgame_state::CURRENT_HOUR + 4]
            .copy_from_slice(&10_i32.to_le_bytes());
        let mut text = campaign.to_vec();
        text.push(0);
        let memory = Fixture(BTreeMap::from([
            (native::IDLER_RVA, 0x5000_u64.to_le_bytes().to_vec()),
            (
                0x5000,
                (native::IN_GAME_IDLER_VTABLE_RVA as u64)
                    .to_le_bytes()
                    .to_vec(),
            ),
            (0x5000 + native::IDLER_PAUSED, vec![1]),
            (0x1000 + native::SESSION_STATE, 2_i32.to_le_bytes().to_vec()),
            (0x2000, state),
            (0x4000, text),
            (
                cgame_state::SINGLETON_RVA,
                0x2000_u64.to_le_bytes().to_vec(),
            ),
            (
                cgame_state::PLAY_SESSION_STARTED_AT_RVA,
                5_i64.to_le_bytes().to_vec(),
            ),
        ]));
        live.mode(&memory, producer, 1, 0x1000, 100, 1).unwrap();
        assert!(producer.active);
        live.publish(Entry {
            order: 1,
            world: 1,
            qpc: 105,
            hour: 10,
            paused: true,
            ..Entry::default()
        })
        .unwrap();
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, true).unwrap();
        let context = groups[0].context.unwrap();
        assert_eq!(context.campaign, campaign);
        assert_eq!((context.hour, context.load_signal), (10, 5));
        assert_eq!(groups[0].batch.events.len(), 1);
        assert_eq!(groups[0].batch.events[0].qpc, 105);
        assert!(groups[0].batch.spans[0].complete);
        assert!(groups[0].batch.spans[0].paused);
        assert!(groups[0].batch.events[0].paused);
    }

    #[test]
    fn committed_roster_and_zero_action_coverage_survive_handoff() {
        let live = live();
        live.metadata()
            .reserve()
            .unwrap()
            .publish(Metadata::Actor(ActorEntry {
                order: 1,
                world: 1,
                actor: 1,
                player: 4,
                country: 3,
                since_qpc: 100,
                ..Default::default()
            }));
        for code in [1, 4] {
            live.publish(Entry {
                order: 1,
                world: 1,
                qpc: 100,
                code,
                ..Default::default()
            })
            .unwrap();
        }
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, false).unwrap();
        assert_eq!(groups[0].batch.actors.len(), 1);
        assert_eq!(
            groups[0].batch.spans,
            vec![Span {
                paused: false,
                hour: 0,
                from_qpc: 100,
                to_qpc: 110,
                complete: true
            }]
        );
        assert!(live.events().is_drained());
        live.closed_qpc.store(120, Ordering::Relaxed);
        let groups = drain.take_live(&live, true).unwrap();
        assert!(groups[0].batch.actors.is_empty());
        assert_eq!(groups[0].batch.spans[0].from_qpc, 110);
        assert!(groups[0].batch.spans[0].complete);
    }

    #[test]
    fn failure_after_watermark_invalidates_the_interval_that_contains_it() {
        let live = live();
        live.publish(Entry {
            order: 1,
            world: 1,
            qpc: 100,
            code: 1,
            ..Default::default()
        })
        .unwrap();
        live.failed_qpc.store(125, Ordering::Relaxed);
        live.failed.store(1, Ordering::Release);
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, false).unwrap();
        assert_eq!(groups[0].lost, 1);
        live.closed_qpc.store(130, Ordering::Relaxed);
        let groups = drain.take_live(&live, false).unwrap();
        assert_eq!(groups[0].lost, 0);
        assert!(!groups[0].batch.spans[0].complete);
        live.closed_qpc.store(140, Ordering::Relaxed);
        assert!(drain.take_live(&live, true).unwrap()[0].batch.spans[0].complete);
    }
}
