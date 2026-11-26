use collector_hooks::hook::Hook;
use collector_hooks::hook::Registers;
use collector_hooks::hook::Tap;
use collector_primitives::clock::qpc;
use collector_primitives::first_reason::FirstReason;
use collector_primitives::shared_lane::SharedLane;
use engine_executable::executable_tables::commands::{self, Target, Token};
use engine_native_memory::{Memory, cpdx_array::Array, memory::LiveMemory, read};
use engine_native::cgame_state;
use engine_native_air::command_inputs as native;
use replay_recording::command_packet::{Actor, Batch, Category, Event, Kind, Origin, Span};
use std::{
    borrow::Cow,
    cell::UnsafeCell,
    collections::{HashMap, HashSet},
    io,
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering, fence},
    },
};

mod classes;

const EVENT_ALLOCATION: &str = "command event allocation failed";
const WORLD_CONTEXT_ALLOCATION: &str = "command world context allocation failed";
const PLAYER_STORAGE_ALLOCATION: &str = "command player storage allocation failed";
const OVERLAP: &str = "command observation overlapped another";
const ADMISSION_GAP: &str = "command observer coverage interrupted";

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
pub struct WorldContext {
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

impl Descriptor {
    fn captured(&self) -> bool {
        !matches!(
            self.category,
            Category::System | Category::Session | Category::Automatic
        )
    }
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
    sample_roster: unsafe fn(&LiveMemory, usize),
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
    coverage_uncertain: AtomicBool,
    coverage_uncertain_until: AtomicU64,
    transition_epoch: AtomicU64,
    first_failure: FirstReason,
    capacity_demand: AtomicUsize,
    name_demand: AtomicUsize,
}

unsafe impl Sync for Live {}
unsafe impl Send for Live {}

static LIVE: AtomicPtr<Live> = AtomicPtr::new(std::ptr::null_mut());
static STORE: Mutex<Option<Box<Live>>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);

impl Live {
    fn new(
        memory: &impl Memory,
        base: usize,
        sample_roster: unsafe fn(&LiveMemory, usize),
    ) -> io::Result<Self> {
        let tables = engine_executable::executable_tables::commands()?;
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
        let state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
            .and_then(|at| read::<u64>(memory, at))
            .unwrap_or(0) as usize;
        let roster = Array::read(
            memory,
            state.saturating_add(cgame_state::PLAYER_RECORDS),
            engine_native_parts::chuman_layout::STRIDE,
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
                    .element(index, engine_native_parts::chuman_layout::STRIDE)
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
        Ok(Self {
            base,
            capture: u64::try_from(qpc())
                .ok()
                .filter(|&time| time != 0)
                .ok_or(io::ErrorKind::InvalidData)?,
            sample_roster,
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
            coverage_uncertain: AtomicBool::new(false),
            coverage_uncertain_until: AtomicU64::new(0),
            transition_epoch: AtomicU64::new(0),
            first_failure: FirstReason::new(),
            capacity_demand: AtomicUsize::new(slots),
            name_demand: AtomicUsize::new(names),
        })
    }

    fn events(&self) -> &SharedLane<Entry> {
        unsafe { &*self.events.get() }
    }
    fn metadata(&self) -> &SharedLane<Metadata> {
        unsafe { &*self.metadata.get() }
    }

    fn grow(&self) -> (bool, bool) {
        let events = self.events();
        let event_capacity = events.required_entries().max(events.len());
        let next_events = (event_capacity > events.len())
            .then(|| engine_allocation::advisory(|| SharedLane::new(event_capacity, 0)).ok())
            .flatten();
        let actors = self.metadata();
        let actor_capacity = self
            .capacity_demand
            .load(Ordering::Relaxed)
            .max(1)
            .checked_next_power_of_two()
            .and_then(|slots| slots.checked_mul(2))
            .map(|slots| actors.required_entries().max(actors.len()).max(slots));
        let stride = self
            .name_demand
            .load(Ordering::Relaxed)
            .max(actors.stride());
        let next_actors = actor_capacity
            .filter(|&capacity| capacity > actors.len() || stride > actors.stride())
            .and_then(|capacity| {
                engine_allocation::advisory(|| SharedLane::new(capacity, stride)).ok()
            });
        if next_events.is_none() && next_actors.is_none() {
            return (false, false);
        }
        if self
            .occupied
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return (false, false);
        }
        self.version.fetch_add(1, Ordering::AcqRel);
        let mut retired_events = None;
        let mut retired_actors = None;
        let mut changed = (false, false);
        if self.events().is_drained() && self.metadata().is_drained() {
            if let Some(next) = next_events {
                retired_events = Some(unsafe { std::mem::replace(&mut *self.events.get(), next) });
                changed.0 = true;
            }
            if let Some(next) = next_actors {
                retired_actors =
                    Some(unsafe { std::mem::replace(&mut *self.metadata.get(), next) });
                changed.1 = true;
            }
            unsafe { (*self.producer.get()).dirty = true };
        }
        self.version.fetch_add(1, Ordering::Release);
        self.occupied.store(false, Ordering::Release);
        drop((retired_events, retired_actors));
        changed
    }

    fn fail(&self, reason: &'static str) {
        self.first_failure.record(reason);
        self.failed_qpc
            .fetch_max(qpc().max(0) as u64, Ordering::Relaxed);
        self.failed.fetch_add(1, Ordering::Release);
    }

    fn status(&self) -> impl serde::Serialize + use<> {
        collector_status::status::status!({
            "lost": self.failed.load(Ordering::Acquire),
            "first_loss": self.first_failure.get(),
        })
    }

    fn publish(&self, entry: Entry) -> Result<(), &'static str> {
        let slot = self.events().reserve().ok_or(EVENT_ALLOCATION)?;
        slot.publish(entry);
        Ok(())
    }

    fn grow_players(&self, producer: &mut Producer, players: usize) -> Result<(), &'static str> {
        let slots = players
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(PLAYER_STORAGE_ALLOCATION)?;
        let (mut next, previous) = engine_allocation::advisory(|| {
            Ok::<_, io::Error>((
                engine_allocation::default_vec::<CachedPlayer>(slots)?,
                engine_allocation::default_vec::<CachedPlayer>(slots)?,
            ))
        })
        .map_err(|_| PLAYER_STORAGE_ALLOCATION)?;
        for &player in &producer.players {
            if player.actor == 0 || !player.seen {
                continue;
            }
            let mut index = hash(player.key as u32 as u64, slots);
            while next[index].actor != 0 {
                index = (index + 1) & (slots - 1);
            }
            next[index] = player;
        }
        producer.players = next;
        producer.previous_players = previous;
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
            engine_native_parts::chuman_layout::STRIDE,
        )
        .ok_or("command roster invalid")?;
        if !producer.dirty && producer.roster == (rows.data, rows.count) {
            return Ok(());
        }
        self.capacity_demand
            .fetch_max(rows.count as usize * 2, Ordering::Relaxed);
        if rows.count as usize > producer.players.len() / 2 {
            self.grow_players(producer, rows.count as usize)?;
        }
        producer.previous_players.fill(CachedPlayer::default());
        for index in 0..rows.count {
            let pointer = rows
                .element(index, engine_native_parts::chuman_layout::STRIDE)
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
                        .reserve_bytes(needed)
                        .ok_or("command player allocation failed")?;
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
        let paused = if world == 0 {
            None
        } else if let Some(paused) = native::paused(memory, self.base)? {
            let state = read::<i32>(
                memory,
                session
                    .checked_add(native::SESSION_STATE)
                    .ok_or("command session overflow")?,
            )
            .ok_or("command session state unreadable")?;
            matches!(state, 2 | 11 | 12).then_some(paused)
        } else {
            None
        };
        let active = paused.is_some();
        if producer.active && (!active || producer.world != world || producer.session != session) {
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
        if let Some(paused) = paused {
            if producer.active {
                let hour = read(memory, producer.state + cgame_state::CURRENT_HOUR)
                    .ok_or("command hour unreadable")?;
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
                return Ok(());
            }
            let state = engine_sites::at(self.base, cgame_state::SINGLETON_RVA)
                .and_then(|at| read::<u64>(memory, at))
                .filter(|&pointer| pointer != 0)
                .ok_or("command world unreadable")? as usize;
            let campaign = engine_native_memory::msvc_string::read_text::<36>(
                memory,
                state + cgame_state::CAMPAIGN_ID,
            )
            .filter(|text| text.len == 36)
            .ok_or("command campaign identity unreadable")?;
            let context = WorldContext {
                campaign: campaign.bytes,
                hour: read(memory, state + cgame_state::CURRENT_HOUR)
                    .ok_or("command phase hour unreadable")?,
                load_signal: engine_sites::at(self.base, cgame_state::PLAY_SESSION_STARTED_AT_RVA)
                    .and_then(|at| read(memory, at))
                    .ok_or("command phase load signal unreadable")?,
                qpc: time,
            };
            let metadata = self.metadata().reserve().ok_or(WORLD_CONTEXT_ALLOCATION)?;
            let event = self.events().reserve().ok_or(EVENT_ALLOCATION)?;
            metadata.publish(Metadata::World {
                order,
                world,
                context,
            });
            event.publish(Entry {
                order,
                world,
                qpc: time,
                code: 1,
                hour: context.hour,
                paused,
                ..Entry::default()
            });
            producer.players.fill(CachedPlayer::default());
            producer.previous_players.fill(CachedPlayer::default());
            producer.dirty = true;
            producer.world = world;
            producer.session = session;
            producer.state = state;
            producer.hour = context.hour;
            producer.paused = paused;
            producer.active = true;
        }
        Ok(())
    }

    fn observe(&self, registers: &Registers, site: usize) -> Result<(), &'static str> {
        let memory = unsafe { LiveMemory::new() };
        let time = u64::try_from(qpc()).map_err(|_| "command clock negative")?;
        let world = collector_lifetime::world_lifetime::observed_generation().unwrap_or(0);
        self.observe_at(&memory, registers, site, world, time, |state| {
            // SAFETY: The installed command callback uses this session's live shared roster.
            unsafe { (self.sample_roster)(&memory, state) }
        })
    }

    fn kind(&self, vtable: usize) -> Option<&Descriptor> {
        self.kinds
            .binary_search_by_key(&vtable, |kind| kind.vtable)
            .ok()
            .map(|index| &self.kinds[index])
    }

    fn observe_at(
        &self,
        memory: &impl Memory,
        registers: &Registers,
        site: usize,
        world: u64,
        time: u64,
        sample_roster: impl FnOnce(usize),
    ) -> Result<(), &'static str> {
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
        self.mode(memory, producer, world, session, time, order)?;
        let command = match site {
            1 => Some(
                read::<u64>(memory, registers.rsi).ok_or("accepted command pointer unreadable")?
                    as usize,
            ),
            2 => Some(registers.rdi),
            _ => None,
        };
        if !producer.active
            && let Some(command) = command
        {
            let vtable = read::<u64>(memory, command).ok_or("command envelope unreadable")?;
            let vtable = usize::try_from(vtable).map_err(|_| "command vtable overflow")?;
            if self.kind(vtable).is_none_or(Descriptor::captured) {
                return Err("command accepted before the game is in play");
            }
        }
        if producer.active {
            sample_roster(producer.state);
            if site == 0 {
                self.roster(memory, producer, order, time)?;
            } else if let Some(command) = command {
                let header = native::header(memory, command)?;
                let kind = self.kind(header.vtable);
                if kind.is_some_and(|kind| kind.roster) {
                    producer.dirty = true;
                }
                if kind.is_some_and(|kind| kind.remove_player) {
                    let removed = read::<i32>(memory, command + native::REMOVED_PLAYER_KEY)
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
                if kind.is_none_or(Descriptor::captured) {
                    let roster_ready = match self.roster(memory, producer, order, time) {
                        Ok(()) => true,
                        Err(reason) => {
                            self.fail(reason);
                            false
                        }
                    };
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
                        memory,
                        self.base,
                        producer.state,
                        &self.inputs,
                        command,
                        kind.map_or(Target::Unknown, |kind| kind.target),
                        header.player,
                        found.country,
                    )
                    .unwrap_or(native::Resolution::Unknown);
                    let target = if let native::Resolution::Country(country) = resolution {
                        Some(country)
                    } else {
                        None
                    };
                    let origin = match resolution {
                        native::Resolution::Country(country) => {
                            match native::autonomous(memory, self.base, producer.state, country) {
                                Ok(true) => Origin::AutonomousCountry,
                                Ok(false)
                                    if found.actor != 0
                                        && native::manual_country(
                                            memory,
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
                    if origin != Origin::AutonomousCountry {
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
    collector_primitives::table_hash::slot(key, slots)
}

unsafe extern "system-unwind" fn observe(registers: &Registers, site: usize) {
    let _timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::PlayerCommands);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    struct Call;
    impl Drop for Call {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _call = Call;
    if !ENABLED.load(Ordering::Acquire) || collector_memory::terminal::failed() {
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
        live.fail(OVERLAP);
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
    if let Err(reason) = live.observe(registers, site) {
        live.fail(reason);
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

pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

pub fn status() -> impl serde::Serialize {
    let store = STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    store.as_ref().map(|live| live.status())
}

/// # Safety
/// Only the originating spool may query the live command store. It must finish the query before the
/// runtime releases the installation's store at detach.
pub unsafe fn urgent() -> bool {
    let pointer = LIVE.load(Ordering::Acquire);
    if pointer.is_null() {
        return false;
    }
    let live = unsafe { &*pointer };
    live.events().pending() >= live.events().len() * 3 / 4
        || live.metadata().pending() >= live.metadata().len() * 3 / 4
}

pub struct Drained {
    pub capture: u64,
    pub world: u64,
    pub from: u64,
    pub to: u64,
    pub lost: u64,
    pub context: Option<WorldContext>,
    pub batch: Batch,
}

#[derive(Default)]
pub struct Drain {
    events: Vec<(u64, u64)>,
    actors: Vec<(u64, u64)>,
    committed: Vec<(u64, u64)>,
    windows: HashMap<u64, Option<Span>>,
    failure: u64,
    loss_until: u64,
    last_qpc: u64,
    last_world: u64,
    known_actors: HashMap<(u64, u64), Actor>,
    retired: HashSet<(u64, u64)>,
    contexts: HashMap<u64, WorldContext>,
}

fn copy_actor(entry: ActorEntry, bytes: &[u8]) -> io::Result<Actor> {
    let user_end = entry.user_len;
    let name_end = user_end + entry.name_len;
    let tag_end = name_end + entry.tag_len;
    Ok(Actor {
        id: entry.actor,
        player: entry.player,
        country: entry.country,
        since_qpc: entry.since_qpc,
        at_qpc: entry.at_qpc,
        user: engine_allocation::copy_vec(&bytes[..user_end])?,
        name: engine_allocation::copy_vec(&bytes[user_end..name_end])?,
        tag: engine_allocation::copy_vec(&bytes[name_end..tag_end])?,
    })
}

fn group_for<'a>(
    groups: &'a mut Vec<Drained>,
    index: &mut HashMap<u64, usize>,
    world: u64,
    from: u64,
    to: u64,
    capture: u64,
) -> io::Result<&'a mut Drained> {
    if let Some(&slot) = index.get(&world) {
        return Ok(&mut groups[slot]);
    }
    engine_allocation::reserve(groups, 1)?;
    engine_allocation::reserve_map(index, 1)?;
    let slot = groups.len();
    groups.push(Drained {
        capture,
        world,
        from,
        to,
        lost: 0,
        context: None,
        batch: Batch::default(),
    });
    index.insert(world, slot);
    Ok(&mut groups[slot])
}

impl Drain {
    /// # Safety
    /// Only the originating installation's spool consumer may drain these lanes. Keep that
    /// installation live, and use a final drain only after command callbacks are quiet.
    pub unsafe fn take(&mut self, final_drain: bool) -> io::Result<Vec<Drained>> {
        let mut out = Vec::new();
        // SAFETY: The caller holds the same installation and final-drain guarantees.
        unsafe { self.take_into(final_drain, &mut out) }?;
        Ok(out)
    }

    /// # Safety
    /// Only the originating installation's spool consumer may drain these lanes. Keep that
    /// installation live, and use a final drain only after command callbacks are quiet.
    pub unsafe fn take_into(
        &mut self,
        final_drain: bool,
        out: &mut Vec<Drained>,
    ) -> io::Result<()> {
        let pointer = LIVE.load(Ordering::Acquire);
        if pointer.is_null() {
            return Ok(());
        }
        let live = unsafe { &*pointer };
        self.take_live_into(live, final_drain, out)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn take_live(&mut self, live: &Live, final_drain: bool) -> io::Result<Vec<Drained>> {
        let mut out = Vec::new();
        self.take_live_into(live, final_drain, &mut out)?;
        Ok(out)
    }

    fn take_live_into(
        &mut self,
        live: &Live,
        final_drain: bool,
        out: &mut Vec<Drained>,
    ) -> io::Result<()> {
        let transition = live.transition_epoch.load(Ordering::Acquire);
        if transition & 1 != 0 {
            return if final_drain {
                Err(io::Error::other(
                    "command observer transition is still pending at final drain",
                ))
            } else {
                Ok(())
            };
        }
        let version = live.version.load(Ordering::Acquire);
        if version & 1 != 0 {
            return if final_drain {
                Err(io::Error::other(
                    "command observer is still active at final drain",
                ))
            } else {
                Ok(())
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
                Ok(())
            };
        }
        let failed = live.failed.load(Ordering::Acquire);
        let failed_qpc = live.failed_qpc.load(Ordering::Relaxed);
        let uncertain = live.coverage_uncertain.load(Ordering::Acquire);
        let uncertain_until = live.coverage_uncertain_until.load(Ordering::Acquire);
        fence(Ordering::Acquire);
        if transition != live.transition_epoch.load(Ordering::Relaxed) {
            return if final_drain {
                Err(io::Error::other(
                    "command observer transition changed during final drain",
                ))
            } else {
                Ok(())
            };
        }
        if time == 0 {
            return Ok(());
        }
        live.events()
            .published_into(&mut self.events, |entry| entry.order)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        live.metadata()
            .published_into(&mut self.actors, Metadata::order)
            .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        self.committed.clear();
        for &(_, index) in &self.events {
            let (entry, _) = live.events().record(index);
            if entry.order <= order && entry.code == 4 {
                engine_allocation::reserve(&mut self.committed, 1)?;
                self.committed.push((entry.world, entry.order));
            }
        }
        self.committed.sort_unstable();
        self.committed.dedup();
        let mut groups = Vec::new();
        let mut group_index = HashMap::new();
        let mut staged_contexts = HashMap::new();
        let mut staged_actors: Vec<(u64, Actor)> = Vec::new();
        let mut staged_known = HashMap::new();
        let mut staged_windows = HashMap::new();
        let mut staged_retired = HashSet::new();
        for &(_, index) in &self.actors {
            let (entry, bytes) = live.metadata().record(index);
            if entry.order() > order {
                continue;
            }
            let entry = match entry {
                Metadata::World {
                    order: world_order,
                    world,
                    context,
                } => {
                    if !self.contexts.contains_key(&world) && !staged_contexts.contains_key(&world) {
                        let start = self
                            .events
                            .partition_point(|&(entry_order, _)| entry_order < world_order);
                        if self.events[start..]
                            .iter()
                            .take_while(|&&(entry_order, _)| entry_order == world_order)
                            .any(|&(_, event_index)| {
                                let (entry, _) = live.events().record(event_index);
                                entry.world == world && entry.code == 1
                            })
                        {
                            engine_allocation::reserve_map(&mut staged_contexts, 1)?;
                            staged_contexts.insert(world, context);
                        }
                    }
                    continue;
                }
                Metadata::Actor(entry) => entry,
            };
            if self
                .committed
                .binary_search(&(entry.world, entry.order))
                .is_err()
            {
                continue;
            }
            let key = (entry.world, entry.actor);
            let existing = staged_known
                .get(&key)
                .map(|&slot: &usize| &staged_actors[slot].1)
                .or_else(|| self.known_actors.get(&key));
            if existing.is_some_and(|old| {
                old.player == entry.player
                    && old.country == entry.country
                    && old.since_qpc == entry.since_qpc
                    && old.user == bytes[..entry.user_len]
                    && old.name == bytes[entry.user_len..entry.user_len + entry.name_len]
                    && old.tag
                        == bytes[entry.user_len + entry.name_len
                            ..entry.user_len + entry.name_len + entry.tag_len]
            }) {
                continue;
            }
            let batch_actor = copy_actor(entry, bytes)?;
            let known_actor = copy_actor(entry, bytes)?;
            let group = group_for(
                &mut groups,
                &mut group_index,
                entry.world,
                self.last_qpc.min(time),
                time,
                live.capture,
            )?;
            engine_allocation::reserve(&mut group.batch.actors, 1)?;
            engine_allocation::reserve(&mut staged_actors, 1)?;
            let slot = staged_actors.len();
            if let Some(existing_slot) = staged_known.get_mut(&key) {
                *existing_slot = slot;
            } else {
                engine_allocation::reserve_map(&mut staged_known, 1)?;
                staged_known.insert(key, slot);
            }
            staged_actors.push((entry.world, known_actor));
            group.batch.actors.push(batch_actor);
        }
        for &(_, index) in &self.events {
            let (entry, _) = live.events().record(index);
            if entry.order > order {
                continue;
            }
            let from = if self.last_qpc == 0 {
                entry.qpc
            } else {
                self.last_qpc
            };
            let group = group_for(
                &mut groups,
                &mut group_index,
                entry.world,
                from,
                time,
                live.capture,
            )?;
            match entry.code {
                4 => {}
                3 => {
                    let key = (entry.world, entry.actor);
                    if !self.retired.contains(&key) && !staged_retired.contains(&key) {
                        engine_allocation::reserve_set(&mut staged_retired, 1)?;
                        engine_allocation::reserve(&mut group.batch.departures, 1)?;
                        staged_retired.insert(key);
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
                    let previous = match staged_windows.get_mut(&entry.world) {
                        Some(slot) => std::mem::replace(slot, next),
                        None => {
                            engine_allocation::reserve_map(&mut staged_windows, 1)?;
                            let previous = self.windows.get(&entry.world).copied().flatten();
                            staged_windows.insert(entry.world, next);
                            previous
                        }
                    };
                    if let Some(previous) = previous {
                        engine_allocation::reserve(&mut group.batch.spans, 1)?;
                        group.batch.spans.push(Span {
                            from_qpc: previous.from_qpc.max(group.from),
                            to_qpc: entry.qpc,
                            ..previous
                        });
                    }
                }
                _ => {
                    engine_allocation::reserve(&mut group.batch.events, 1)?;
                    group.batch.events.push(Event {
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
                    });
                }
            }
        }
        let mut loss_until = if failed != self.failure {
            self.loss_until.max(failed_qpc)
        } else {
            self.loss_until
        }
        .max(uncertain_until);
        if uncertain {
            loss_until = loss_until.max(time);
        }
        let event_refused = live.events().rejected();
        let actor_refused = live.metadata().rejected();
        let lost = failed.saturating_sub(self.failure);
        let world = if world == 0 && groups.is_empty() && lost != 0 {
            self.last_world
        } else {
            world
        };
        if world != 0 {
            group_for(
                &mut groups,
                &mut group_index,
                world,
                self.last_qpc.min(time),
                time,
                live.capture,
            )?;
        }
        groups.sort_unstable_by_key(|group| group.world);
        for group in &mut groups {
            group.context = staged_contexts
                .get(&group.world)
                .or_else(|| self.contexts.get(&group.world))
                .copied();
            let current_window = staged_windows
                .get(&group.world)
                .copied()
                .or_else(|| self.windows.get(&group.world).copied())
                .flatten();
            let incomplete =
                uncertain || lost != 0 || (loss_until != 0 && loss_until >= group.from);
            if let Some(mut window) = current_window {
                engine_allocation::reserve(&mut group.batch.spans, 1)?;
                group.batch.spans.push(Span {
                    from_qpc: window.from_qpc.max(group.from),
                    to_qpc: time,
                    complete: !incomplete,
                    ..window
                });
                window.from_qpc = time;
                if let Some(slot) = staged_windows.get_mut(&group.world) {
                    *slot = Some(window);
                } else {
                    engine_allocation::reserve_map(&mut staged_windows, 1)?;
                    staged_windows.insert(group.world, Some(window));
                }
            }
            if incomplete {
                for span in &mut group.batch.spans {
                    span.complete = false;
                }
            }
        }
        if let Some(latest) = groups.last_mut() {
            latest.lost = lost;
        }
        engine_allocation::reserve_map(&mut self.contexts, staged_contexts.len())?;
        engine_allocation::reserve_map(&mut self.known_actors, staged_actors.len())?;
        engine_allocation::reserve_set(&mut self.retired, staged_retired.len())?;
        engine_allocation::reserve_map(&mut self.windows, staged_windows.len())?;
        engine_allocation::reserve(out, groups.len())?;
        for (world, context) in staged_contexts {
            self.contexts.entry(world).or_insert(context);
        }
        for (world, actor) in staged_actors {
            let key = (world, actor.id);
            if let Some(slot) = self.known_actors.get_mut(&key) {
                *slot = actor;
            } else {
                self.known_actors.insert(key, actor);
            }
        }
        for key in staged_retired {
            self.retired.insert(key);
        }
        for (world, window) in staged_windows {
            if let Some(slot) = self.windows.get_mut(&world) {
                *slot = window;
            } else {
                self.windows.insert(world, window);
            }
        }
        self.loss_until = loss_until;
        if let Some(latest) = groups.last() {
            self.last_world = latest.world;
            self.last_qpc = time;
            self.failure = failed;
        }
        for &(_, index) in &self.actors {
            if live.metadata().record(index).0.order() <= order {
                live.metadata().release(index);
            }
        }
        for &(_, index) in &self.events {
            if live.events().record(index).0.order <= order {
                live.events().release(index);
            }
        }
        live.events().advance(event_refused);
        live.metadata().advance(actor_refused);
        out.extend(groups);
        let _ = live.grow();
        Ok(())
    }

    /// # Safety
    /// Read the catalog only for the current installation's first command packet, before retiring
    /// its store and before any later attachment.
    pub unsafe fn catalog() -> io::Result<Vec<Kind>> {
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
    live: *const Live,
    owns_live: bool,
    publication_attempted: bool,
    was_enabled: bool,
    transition_open: bool,
}

impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    /// Publish the state required by `sample_roster` before enabling the taps and retain it until
    /// every admitted callback has returned. Keep exclusive installation ownership until this
    /// handle drops, including after releasing its store.
    pub unsafe fn new(
        memory: &impl Memory,
        base: usize,
        sample_roster: unsafe fn(&LiveMemory, usize),
    ) -> Self {
        let mut live_pointer: *const Live = std::ptr::null();
        let result = (|| -> Result<Vec<Tap>, Cow<'static, str>> {
            let mut located = [(0, [0; 16]); native::SITES.len()];
            for (site, slot) in native::SITES.iter().zip(&mut located) {
                *slot = engine_sites::hook(base, site.rva)
                    .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))?;
                if collector_hooks::hook::tap_span(&slot.1) != Ok(site.span) {
                    return Err("command observer site instruction span differs".into());
                }
            }
            let live = engine_allocation::boxed(
                Live::new(memory, base, sample_roster)
                    .map_err(|error| Cow::Owned(error.to_string()))?,
            )
            .map_err(|_| "command observer storage allocation failed")?;
            let pointer = std::ptr::from_ref(&*live).cast_mut();
            live_pointer = pointer;
            *STORE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(live);
            LIVE.store(pointer, Ordering::Release);
            located
                .into_iter()
                .enumerate()
                .map(|(index, (at, original))| unsafe {
                    Tap::new(at, observe, index, original).map_err(|reason| {
                        Cow::Owned(format!("player command site {index}: {reason}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })();
        let owns_live = !live_pointer.is_null();
        Self {
            taps: result.unwrap_or_else(|reason| {
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Construction,
                    None,
                    reason.as_ref(),
                );
                Vec::new()
            }),
            live: live_pointer,
            owns_live,
            publication_attempted: false,
            was_enabled: false,
            transition_open: false,
        }
    }

    fn live(&self) -> Option<&Live> {
        (!self.live.is_null()).then(|| unsafe { &*self.live })
    }

    fn begin_transition(&mut self) {
        if !self.transition_open {
            if let Some(live) = self.live() {
                live.transition_epoch.fetch_add(1, Ordering::SeqCst);
            }
            self.transition_open = true;
        }
    }

    fn end_transition(&mut self) {
        if self.transition_open {
            self.transition_open = false;
            if let Some(live) = self.live() {
                live.transition_epoch.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    fn record_coverage_loss(&self, reason: &'static str) {
        if let Some(live) = self.live() {
            live.fail(reason);
        }
    }

    fn mark_coverage_uncertain(&self, reason: &'static str) {
        if let Some(live) = self.live()
            && !live.coverage_uncertain.swap(true, Ordering::AcqRel)
        {
            live.fail(reason);
        }
    }

    fn coverage_uncertain(&self) -> bool {
        self.live()
            .is_some_and(|live| live.coverage_uncertain.load(Ordering::Acquire))
    }

    fn clear_coverage_uncertain(&self) {
        if let Some(live) = self.live()
            && live.coverage_uncertain.load(Ordering::Acquire)
        {
            let until = (qpc().max(0) as u64).max(live.closed_qpc.load(Ordering::Relaxed));
            live.coverage_uncertain_until
                .fetch_max(until, Ordering::Release);
            live.coverage_uncertain.store(false, Ordering::Release);
        }
    }

    fn taps_published(&self) -> bool {
        !self.taps.is_empty() && self.taps.iter().all(Tap::is_published)
    }

    fn any_tap_enabled(&self) -> bool {
        self.taps.iter().any(Tap::is_enabled)
    }

    /// # Safety
    /// Only the owning runtime may toggle these taps in attach or detach order. Keep them alive
    /// until any shared publication round finishes and all admitted callbacks are quiet.
    pub unsafe fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        let outcome = if enabled {
            collector_hooks::hook::shared::attach_outcome()
        } else {
            None
        };
        if enabled && outcome.is_none() && self.taps_published() && ENABLED.load(Ordering::Acquire)
        {
            return Ok(true);
        }
        let previous = ENABLED.load(Ordering::Acquire);
        self.begin_transition();
        if enabled && outcome.is_none() && !previous {
            let _ = collector_hooks::hook::shared::defer_preparation(|| {
                ENABLED.store(false, Ordering::SeqCst);
            });
        }
        let result = match outcome {
            Some(outcome) => outcome.map_err(|_| "command observer publication failed"),
            None => {
                self.publication_attempted |= enabled;
                Hook::set_enabled_iter(
                    self.taps.iter_mut().map(Tap::publication_hook),
                    enabled,
                )
                .map_err(|_| "command observer publication failed")
            }
        };
        let deferred = (collector_hooks::hook::shared::attach_collecting()
            && (enabled || !collector_hooks::hook::restoration::active())
            && matches!(result, Ok(true)))
            || (!enabled
                && collector_hooks::hook::restoration::collecting()
                && matches!(result, Ok(false)));
        if deferred {
            return result;
        }
        match result {
            Ok(true) => {
                if enabled {
                    if self.taps_published() {
                        let was_uncertain = self.coverage_uncertain();
                        if self.was_enabled && !was_uncertain {
                            self.record_coverage_loss(ADMISSION_GAP);
                        }
                        self.clear_coverage_uncertain();
                        self.was_enabled = true;
                        ENABLED.store(true, Ordering::Release);
                    }
                } else if !self.any_tap_enabled() {
                    ENABLED.store(false, Ordering::SeqCst);
                    self.clear_coverage_uncertain();
                }
                self.end_transition();
                Ok(true)
            }
            Ok(false) => {
                if previous {
                    if self.taps_published() {
                        self.clear_coverage_uncertain();
                    } else {
                        self.mark_coverage_uncertain(ADMISSION_GAP);
                    }
                }
                self.end_transition();
                Ok(false)
            }
            Err(error) => {
                if previous {
                    if self.taps_published() {
                        self.clear_coverage_uncertain();
                    } else {
                        self.mark_coverage_uncertain(ADMISSION_GAP);
                    }
                }
                self.end_transition();
                Err(error)
            }
        }
    }

    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps.iter().flat_map(Tap::executable_ranges)
    }

    /// # Safety
    /// Restore taps and wait for callback and publication quiescence before release. Drop this
    /// handle before any new player-command installation can publish or it can clear that
    /// installation's store.
    pub unsafe fn release_after_quiet(&mut self) {
        self.end_transition();
        self.publication_attempted = false;
        self.live = std::ptr::null();
        if self.owns_live {
            self.owns_live = false;
            LIVE.store(std::ptr::null_mut(), Ordering::Release);
            STORE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        self.end_transition();
        if !self.publication_attempted {
            // SAFETY: The owning hook handle has no published taps or live callbacks here.
            unsafe { self.release_after_quiet() };
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use engine_native_politics::chuman;
    use super::*;
    use std::collections::BTreeMap;
    fn live() -> Live {
        Live {
            base: 0,
            capture: 1,
            sample_roster: |_, _| {},
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
            coverage_uncertain: AtomicBool::new(false),
            coverage_uncertain_until: AtomicU64::new(0),
            transition_epoch: AtomicU64::new(0),
            first_failure: FirstReason::new(),
            capacity_demand: AtomicUsize::new(2),
            name_demand: AtomicUsize::new(0),
        }
    }

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
    impl Fixture {
        fn new() -> Self {
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
            Self(BTreeMap::from([
                (native::IDLER_RVA.0, 0x5000_u64.to_le_bytes().to_vec()),
                (
                    0x5000,
                    (native::IN_GAME_IDLER_VTABLE_RVA.0 as u64)
                        .to_le_bytes()
                        .to_vec(),
                ),
                (0x5000 + native::IDLER_PAUSED, vec![1]),
                (0x1000 + native::SESSION_STATE, 2_i32.to_le_bytes().to_vec()),
                (0x2000, state),
                (0x4000, text),
                (
                    cgame_state::SINGLETON_RVA.0,
                    0x2000_u64.to_le_bytes().to_vec(),
                ),
                (
                    cgame_state::PLAY_SESSION_STARTED_AT_RVA.0,
                    5_i64.to_le_bytes().to_vec(),
                ),
            ]))
        }
    }

    fn attempt_mode(
        live: &Live,
        memory: &Fixture,
        world: u64,
        time: u64,
        order: u64,
    ) -> Result<(), &'static str> {
        let producer = unsafe { &mut *live.producer.get() };
        let result = live.mode(memory, producer, world, 0x1000, time, order);
        if let Err(reason) = result {
            live.fail(reason);
        }
        live.closed_qpc.store(time, Ordering::Relaxed);
        live.closed_order.store(order, Ordering::Relaxed);
        live.closed_world.store(
            if producer.active { producer.world } else { 0 },
            Ordering::Relaxed,
        );
        result
    }

    #[cfg(test)]
    fn accepted_fixture(category: Option<Category>) -> (Live, Fixture) {
        let mut live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        if let Some(category) = category {
            live.kinds.push(Descriptor {
                vtable: 0x8000,
                id: 1,
                category,
                target: Target::Unknown,
                roster: false,
                remove_player: false,
            });
        }
        let mut memory = Fixture::new();
        memory.0.insert(0x6000, 0x7000_u64.to_le_bytes().to_vec());
        memory.0.insert(0x7000, 0x8000_u64.to_le_bytes().to_vec());
        (live, memory)
    }

    #[cfg(test)]
    struct LoadingMemory<'a> {
        fixture: &'a Fixture,
        headers: std::cell::Cell<usize>,
    }

    #[cfg(test)]
    impl Memory for LoadingMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            assert!(address == native::IDLER_RVA.0 || matches!(address, 0x5000 | 0x6000 | 0x7000));
            assert_eq!(output.len(), 8);
            if address == 0x7000 {
                self.headers.set(self.headers.get() + 1);
            }
            self.fixture.copy(address, output)
        }
    }

    #[cfg(test)]
    fn attempt_observe(
        live: &Live,
        memory: &impl Memory,
        site: usize,
        world: u64,
        time: u64,
    ) -> Result<(), &'static str> {
        let mut registers: Registers = unsafe { std::mem::zeroed() };
        registers.rcx = 0x1000;
        registers.r14 = 0x1000;
        registers.rsi = if site == 1 { 0x6000 } else { 0x1000 };
        registers.rdi = 0x7000;
        let result = live.observe_at(memory, &registers, site, world, time, |_| {});
        if let Err(reason) = result {
            live.fail(reason);
            live.failed_qpc.store(time, Ordering::Relaxed);
            let producer = unsafe { &*live.producer.get() };
            live.closed_qpc.store(time, Ordering::Relaxed);
            live.closed_order
                .store(producer.next_order - 1, Ordering::Relaxed);
            live.closed_world.store(
                if producer.active { producer.world } else { 0 },
                Ordering::Relaxed,
            );
        }
        result
    }

    #[test]
    fn accepted_captured_or_unknown_commands_before_play_report_one_named_loss() {
        for site in [1, 2] {
            for category in [Some(Category::Army), None] {
                for world in [0, 1] {
                    let (live, mut memory) = accepted_fixture(category);
                    memory.0.remove(&native::IDLER_RVA.0);
                    let loading = LoadingMemory {
                        fixture: &memory,
                        headers: std::cell::Cell::new(0),
                    };
                    let reason = "command accepted before the game is in play";
                    assert_eq!(
                        attempt_observe(&live, &loading, site, world, 90),
                        Err(reason)
                    );
                    assert_eq!(loading.headers.get(), 1);
                    assert_eq!(live.failed.load(Ordering::Acquire), 1);
                    assert_eq!((live.metadata().pending(), live.events().pending()), (0, 0));
                    let mut drain = Drain::default();
                    assert!(drain.take_live(&live, false).unwrap().is_empty());
                    attempt_mode(&live, &Fixture::new(), 1, 100, 3).unwrap();
                    live.closed_qpc.store(110, Ordering::Relaxed);
                    let groups = drain.take_live(&live, false).unwrap();
                    assert_eq!(groups.len(), 1);
                    assert_eq!(groups[0].lost, 1);
                    assert!(groups[0].batch.spans.iter().all(|span| !span.complete));
                    live.closed_qpc.store(120, Ordering::Relaxed);
                    let groups = drain.take_live(&live, true).unwrap();
                    assert_eq!(groups[0].lost, 0);
                    assert!(groups[0].batch.spans.iter().all(|span| span.complete));
                }
            }
        }
    }

    #[test]
    fn loading_nonaccepted_sites_and_excluded_categories_remain_quiet() {
        for category in [Category::System, Category::Session, Category::Automatic] {
            for absent in [false, true] {
                let (live, mut memory) = accepted_fixture(Some(category));
                if absent {
                    memory.0.remove(&native::IDLER_RVA.0);
                } else {
                    memory.0.insert(0x5000, 1_u64.to_le_bytes().to_vec());
                }
                let loading = LoadingMemory {
                    fixture: &memory,
                    headers: std::cell::Cell::new(0),
                };
                for site in 0..native::SITES.len() {
                    attempt_observe(&live, &loading, site, 1, 90 + site as u64).unwrap();
                }
                assert_eq!(loading.headers.get(), 2);
                assert_eq!(live.failed.load(Ordering::Acquire), 0);
                assert_eq!((live.metadata().pending(), live.events().pending()), (0, 0));
            }
        }
    }

    #[test]
    fn accepted_command_after_idler_disappears_invalidates_coverage_across_reactivation() {
        for drain_between in [false, true] {
            let (live, mut memory) = accepted_fixture(Some(Category::Army));
            attempt_mode(&live, &memory, 1, 100, 1).unwrap();
            memory.0.remove(&native::IDLER_RVA.0);
            assert_eq!(
                attempt_observe(&live, &memory, 2, 1, 105),
                Err("command accepted before the game is in play")
            );
            assert!(!unsafe { (*live.producer.get()).active });
            let mut drain = Drain::default();
            let mut groups = if drain_between {
                drain.take_live(&live, false).unwrap()
            } else {
                Vec::new()
            };
            attempt_mode(&live, &Fixture::new(), 1, 110, 3).unwrap();
            live.closed_qpc.store(115, Ordering::Relaxed);
            groups.extend(drain.take_live(&live, false).unwrap());
            assert_eq!(groups.iter().map(|group| group.lost).sum::<u64>(), 1);
            let spans: Vec<_> = groups.iter().flat_map(|group| &group.batch.spans).collect();
            assert_eq!(spans.len(), 2);
            assert_eq!((spans[0].from_qpc, spans[0].to_qpc), (100, 105));
            assert_eq!((spans[1].from_qpc, spans[1].to_qpc), (110, 115));
            assert!(spans.iter().all(|span| !span.complete));
            live.closed_qpc.store(120, Ordering::Relaxed);
            let groups = drain.take_live(&live, true).unwrap();
            assert_eq!(groups[0].lost, 0);
            assert!(groups[0].batch.spans.iter().all(|span| span.complete));
        }
    }

    #[cfg(test)]
    fn command_session() -> replay_data::identity::SessionStart {
        use replay_data::identity::*;
        SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            capture_mode: CaptureMode::Full,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "synthetic".into(),
                recorder_version: "synthetic".into(),
                collector_hash: [3; 32],
                verification: Verification::Verified {
                    launcher: Default::default(),
                    memory: Default::default(),
                },
            },
        }
    }

    #[cfg(test)]
    fn command_control(mut group: Drained, ordinal: u64) -> replay_recording::control::ControlRecord {
        use replay_recording::command_packet::{Packet, Phase};
        use replay_recording::control::ControlRecord;
        use replay_capture_spool::spool::{Record, decode_record, encode_record};
        use replay_frame::branches::{BranchReason, BranchStart, Parentage};
        if ordinal == 0 {
            group.batch.kinds = vec![Kind {
                id: 1,
                token: 1,
                name: "synthetic command".into(),
                category: Category::Army,
            }];
        }
        let mut packet = Packet::pack(
            1,
            group.capture,
            group.world,
            ordinal,
            group.from,
            group.to,
            group.lost,
            &group.batch,
        )
        .unwrap();
        if ordinal == 0 {
            let context = group.context.unwrap();
            packet.phase = Some(Phase {
                start: BranchStart {
                    id: 1,
                    session: 1,
                    loaded_hour: None,
                    first_hour: context.hour,
                    parent: Parentage::Unknown,
                    reason: BranchReason::SessionStart,
                },
                campaign: context.campaign,
                qpc: context.qpc,
                hour: context.hour,
                load_signal: context.load_signal,
            });
            packet.phase.as_ref().unwrap().validate().unwrap();
        }
        let record = Record::Control(ControlRecord::PlayerCommands { session: 1, packet: Box::new(packet) });
        let Record::Control(control) = decode_record(&encode_record(&record).unwrap()).unwrap()
        else {
            panic!("command control changed record kind");
        };
        control
    }

    #[test]
    fn refusal_after_draining_phase_closure_reaches_a_loss_only_packet() {
        for site in [1, 2] {
            let (live, mut memory) = accepted_fixture(Some(Category::Army));
            memory.0.insert(0x5000 + native::IDLER_PAUSED, vec![0]);
            attempt_mode(&live, &memory, 7, 100, 1).unwrap();
            live.metadata()
                .reserve()
                .unwrap()
                .publish(Metadata::Actor(ActorEntry {
                    order: 1,
                    world: 7,
                    actor: 1,
                    player: 4,
                    country: 3,
                    since_qpc: 100,
                    at_qpc: 100,
                    ..Default::default()
                }));
            for entry in [
                Entry {
                    order: 1,
                    world: 7,
                    qpc: 100,
                    code: 4,
                    ..Default::default()
                },
                Entry {
                    order: 1,
                    world: 7,
                    qpc: 102,
                    hour: 10,
                    actor: 1,
                    player: 4,
                    country: 3,
                    target_present: true,
                    kind: 1,
                    origin: 1,
                    ..Default::default()
                },
            ] {
                live.publish(entry).unwrap();
            }
            let producer = unsafe { &mut *live.producer.get() };
            producer.players[0] = CachedPlayer {
                actor: 1,
                seen: true,
                ..Default::default()
            };
            memory.0.remove(&native::IDLER_RVA.0);
            attempt_mode(&live, &memory, 7, 105, 2).unwrap();
            let mut drain = Drain::default();
            let mut groups = drain.take_live(&live, false).unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].batch.departures, vec![(1, 105)]);
            assert!(drain.windows[&7].is_none());
            let mut controls = vec![command_control(groups.remove(0), 0)];
            let session = command_session();
            let before =
                replay_capture_spool::commands::analyze(std::slice::from_ref(&session), &controls, 1).unwrap();
            assert_eq!(before.lost_observations, 0);
            assert_eq!(before.players.len(), 1);
            assert!(before.players[0].coverage_complete);
            assert!(before.players[0].apm.is_some());
            live.closed_qpc.store(108, Ordering::Relaxed);
            assert!(drain.take_live(&live, false).unwrap().is_empty());
            assert_eq!(
                attempt_observe(&live, &memory, site, 0, 110),
                Err("command accepted before the game is in play")
            );
            assert_eq!((live.metadata().pending(), live.events().pending()), (0, 0));
            assert_eq!(live.closed_world.load(Ordering::Relaxed), 0);
            let mut groups = drain.take_live(&live, true).unwrap();
            assert_eq!(groups.len(), 1);
            let group = groups.remove(0);
            assert_eq!(
                (group.world, group.from, group.to, group.lost),
                (7, 105, 110, 1)
            );
            let context = group.context.unwrap();
            assert_eq!(
                (context.qpc, context.hour, context.load_signal),
                (100, 10, 5)
            );
            assert_eq!(group.batch, Batch::default());
            let control = command_control(group, 1);
            let replay_recording::control::ControlRecord::PlayerCommands { packet, .. } = &control else {
                unreachable!();
            };
            assert!(packet.phase.is_none());
            assert_eq!(packet.unpack().unwrap(), Batch::default());
            controls.push(control);
            let after =
                replay_capture_spool::commands::index(std::slice::from_ref(&session), &controls, 1).unwrap();
            assert_eq!(after.analysis.lost_observations, 1);
            assert_eq!(after.analysis.players[0].confirmed_orders, 1);
            assert!(!after.analysis.players[0].coverage_complete);
            assert!(after.analysis.players[0].observed_minutes.is_none());
            assert!(after.analysis.players[0].apm.is_none());
            assert_eq!(after.timelines.len(), 1);
            assert!(!after.timelines[0].complete);
            assert_eq!(after.timelines[0].spans.len(), 1);
            assert!(drain.take_live(&live, true).unwrap().is_empty());
            assert_eq!(drain.failure, 1);
        }
    }

    #[test]
    fn refusals_without_an_emitted_world_stay_pending_until_activation() {
        for watermark in [false, true] {
            let (live, mut memory) = accepted_fixture(Some(Category::Army));
            live.closed_qpc.store(0, Ordering::Relaxed);
            live.closed_world.store(0, Ordering::Relaxed);
            let mut drain = Drain::default();
            assert!(drain.take_live(&live, true).unwrap().is_empty());
            memory.0.remove(&native::IDLER_RVA.0);
            assert_eq!(
                attempt_observe(&live, &memory, 2, 0, 90),
                Err("command accepted before the game is in play")
            );
            if !watermark {
                live.closed_qpc.store(0, Ordering::Relaxed);
            }
            assert!(drain.take_live(&live, false).unwrap().is_empty());
            assert!(drain.take_live(&live, true).unwrap().is_empty());
            assert_eq!(drain.failure, 0);
            assert_eq!(drain.last_world, 0);
            attempt_mode(&live, &Fixture::new(), 1, 100, 3).unwrap();
            live.closed_qpc.store(110, Ordering::Relaxed);
            let mut groups = drain.take_live(&live, false).unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].lost, 1);
            assert!(groups[0].batch.spans.iter().all(|span| !span.complete));
            let mut controls = vec![command_control(groups.remove(0), 0)];
            live.closed_qpc.store(120, Ordering::Relaxed);
            let mut groups = drain.take_live(&live, true).unwrap();
            assert_eq!(groups[0].lost, 0);
            controls.push(command_control(groups.remove(0), 1));
            let index = replay_capture_spool::commands::index(&[command_session()], &controls, 1).unwrap();
            assert_eq!(index.analysis.lost_observations, 1);
            assert!(!index.timelines[0].complete);
            assert_eq!(drain.failure, 1);
        }
    }

    pub fn reactivated_world() -> Vec<Drained> {
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        let mut memory = Fixture::new();
        attempt_mode(&live, &memory, 1, 100, 1).unwrap();
        live.publish(Entry {
            order: 1,
            world: 1,
            qpc: 102,
            hour: 10,
            paused: true,
            ..Default::default()
        })
        .unwrap();
        memory.0.remove(&native::IDLER_RVA.0);
        attempt_mode(&live, &memory, 1, 105, 2).unwrap();
        memory
            .0
            .insert(native::IDLER_RVA.0, 0x5000_u64.to_le_bytes().to_vec());
        memory.0.get_mut(&0x2000).unwrap()
            [cgame_state::CURRENT_HOUR..cgame_state::CURRENT_HOUR + 4]
            .copy_from_slice(&12_i32.to_le_bytes());
        memory.0.insert(
            cgame_state::PLAY_SESSION_STARTED_AT_RVA.0,
            6_i64.to_le_bytes().to_vec(),
        );
        attempt_mode(&live, &memory, 1, 110, 3).unwrap();
        live.publish(Entry {
            order: 3,
            world: 1,
            qpc: 112,
            hour: 12,
            paused: true,
            ..Default::default()
        })
        .unwrap();
        live.closed_qpc.store(115, Ordering::Relaxed);
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, false).unwrap();
        live.closed_qpc.store(120, Ordering::Relaxed);
        let continued = drain.take_live(&live, false).unwrap();
        let context = continued[0].context.unwrap();
        assert_eq!(
            (context.qpc, context.hour, context.load_signal),
            (100, 10, 5)
        );
        attempt_mode(&live, &memory, 2, 125, 4).unwrap();
        let next = drain.take_live(&live, true).unwrap();
        let context = next
            .iter()
            .find(|group| group.world == 2)
            .unwrap()
            .context
            .unwrap();
        assert_eq!(
            (context.qpc, context.hour, context.load_signal),
            (125, 12, 6)
        );
        groups
    }

    #[test]
    fn same_world_reactivation_preserves_the_first_context() {
        let groups = reactivated_world();
        assert_eq!(groups.len(), 1);
        let context = groups[0].context.unwrap();
        assert_eq!(
            (context.qpc, context.hour, context.load_signal),
            (100, 10, 5)
        );
        assert_eq!(groups[0].lost, 0);
        assert_eq!(
            groups[0]
                .batch
                .events
                .iter()
                .map(|event| event.hour)
                .collect::<Vec<_>>(),
            vec![10, 12]
        );
        assert_eq!(
            groups[0]
                .batch
                .spans
                .iter()
                .map(|span| span.hour)
                .collect::<Vec<_>>(),
            vec![10, 12]
        );
        assert!(groups[0].batch.spans.iter().all(|span| span.complete));
    }

    #[test]
    fn loading_idlers_leave_activation_lanes_and_coverage_untouched() {
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        for variant in 0..4 {
            let mut memory = Fixture::new();
            if variant == 0 {
                memory.0.remove(&(0x1000 + native::SESSION_STATE));
            }
            match variant {
                0 => {
                    memory.0.remove(&native::IDLER_RVA.0);
                }
                1 => {
                    memory
                        .0
                        .insert(native::IDLER_RVA.0, 0_u64.to_le_bytes().to_vec());
                }
                2 => {
                    memory.0.insert(0x5000, 1_u64.to_le_bytes().to_vec());
                }
                _ => {
                    memory.0.remove(&0x5000);
                }
            }
            for order in 1..=8 {
                attempt_mode(&live, &memory, 1, 90, order).unwrap();
                assert!(!unsafe { (*live.producer.get()).active });
                assert_eq!((live.metadata().pending(), live.events().pending()), (0, 0));
                assert_eq!(
                    (live.metadata().rejected(), live.events().rejected()),
                    (0, 0)
                );
                assert_eq!(
                    (
                        live.metadata().required_entries(),
                        live.events().required_entries()
                    ),
                    (0, 0)
                );
                assert_eq!(live.failed.load(Ordering::Acquire), 0);
            }
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 9).unwrap();
        assert!(unsafe { (*live.producer.get()).active });
        assert_eq!((live.metadata().pending(), live.events().pending()), (1, 1));
        live.closed_qpc.store(110, Ordering::Relaxed);
        let groups = Drain::default().take_live(&live, true).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lost, 0);
        assert!(groups[0].context.is_some());
        assert_eq!(groups[0].batch.spans.len(), 1);
        assert_eq!(
            (
                groups[0].batch.spans[0].from_qpc,
                groups[0].batch.spans[0].to_qpc
            ),
            (100, 110)
        );
        assert!(groups[0].batch.spans[0].complete);
    }

    #[test]
    fn invalid_in_game_pause_state_refuses_once_without_publishing_activation() {
        for pause in [Some(2), None] {
            let live = live();
            unsafe {
                (*live.producer.get()).active = false;
            }
            let mut memory = Fixture::new();
            if let Some(pause) = pause {
                memory.0.insert(0x5000 + native::IDLER_PAUSED, vec![pause]);
            } else {
                memory.0.remove(&(0x5000 + native::IDLER_PAUSED));
            }
            assert_eq!(
                attempt_mode(&live, &memory, 1, 100, 1),
                Err("command pause state unreadable")
            );
            assert_eq!((live.metadata().pending(), live.events().pending()), (0, 0));
            assert!(
                live.metadata()
                    .published(Metadata::order)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                live.events()
                    .published(|entry| entry.order)
                    .unwrap()
                    .is_empty()
            );
            assert!(!unsafe { (*live.producer.get()).active });
            assert_eq!(live.failed.load(Ordering::Acquire), 1);
        }
    }

    #[test]
    fn activation_past_full_lanes_publishes_both_and_a_refused_allocation_neither() {
        for metadata_full in [false, true] {
            for denied in [false, true] {
                let live = live();
                unsafe {
                    (*live.producer.get()).active = false;
                }
                if metadata_full {
                    for _ in 0..live.metadata().len() {
                        live.metadata()
                            .reserve()
                            .unwrap()
                            .publish(Metadata::default());
                    }
                } else {
                    for _ in 0..live.events().len() {
                        live.publish(Entry::default()).unwrap();
                    }
                }
                let metadata = live.metadata().published(Metadata::order).unwrap().len();
                let events = live.events().published(|entry| entry.order).unwrap().len();
                let fixture = Fixture::new();
                let attempt = || attempt_mode(&live, &fixture, 1, 100, 1);
                if denied {
                    let reason = if metadata_full {
                        WORLD_CONTEXT_ALLOCATION
                    } else {
                        EVENT_ALLOCATION
                    };
                    assert_eq!(
                        collector_test_support::with_allocations_denied(attempt),
                        Err(reason)
                    );
                    assert_eq!(
                        live.metadata().published(Metadata::order).unwrap().len(),
                        metadata
                    );
                    assert_eq!(
                        live.events().published(|entry| entry.order).unwrap().len(),
                        events
                    );
                    assert_eq!(
                        (live.metadata().rejected(), live.events().rejected()),
                        if metadata_full { (1, 0) } else { (0, 1) }
                    );
                    assert!(!unsafe { (*live.producer.get()).active });
                    assert_eq!(live.failed.load(Ordering::Acquire), 1);
                    assert_eq!(
                        serde_json::to_value(live.status()).unwrap(),
                        serde_json::json!({"lost": 1, "first_loss": reason})
                    );
                } else {
                    attempt().unwrap();
                    let metadata_after = live.metadata().published(Metadata::order).unwrap();
                    let events_after = live.events().published(|entry| entry.order).unwrap();
                    assert_eq!(
                        (metadata_after.len(), events_after.len()),
                        (metadata + 1, events + 1)
                    );
                    assert!(matches!(
                        live.metadata().record(metadata_after.last().unwrap().1).0,
                        Metadata::World {
                            order: 1,
                            world: 1,
                            ..
                        }
                    ));
                    assert_eq!(
                        live.events().record(events_after.last().unwrap().1).0.code,
                        1
                    );
                    assert_eq!(
                        (live.metadata().rejected(), live.events().rejected()),
                        (0, 0)
                    );
                    assert!(unsafe { (*live.producer.get()).active });
                    assert_eq!(live.failed.load(Ordering::Acquire), 0);
                }
                if metadata_full {
                    assert!(live.metadata().required_entries() > live.metadata().len());
                } else {
                    assert!(live.events().required_entries() > live.events().len());
                }
            }
        }
    }

    #[test]
    fn a_command_burst_past_the_prepared_lane_keeps_every_command_in_order() {
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let burst = 4 * live.events().len() as u32 + 3;
        for sequence in 0..burst {
            live.publish(Entry {
                order: 2 + u64::from(sequence),
                world: 1,
                qpc: 101 + u64::from(sequence),
                hour: 10,
                player: 4,
                sequence,
                kind: 1,
                origin: 1,
                ..Entry::default()
            })
            .unwrap();
        }
        live.closed_order
            .store(1 + u64::from(burst), Ordering::Relaxed);
        live.closed_qpc.store(200, Ordering::Relaxed);
        let groups = Drain::default().take_live(&live, true).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lost, 0);
        assert_eq!(
            groups[0]
                .batch
                .events
                .iter()
                .map(|event| (event.sequence, event.qpc))
                .collect::<Vec<_>>(),
            (0..burst)
                .map(|sequence| (sequence, 101 + u64::from(sequence)))
                .collect::<Vec<_>>()
        );
        assert!(groups[0].batch.spans.iter().all(|span| span.complete));
        assert_eq!(live.events().rejected(), 0);
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        assert!(live.events().len() > burst as usize);
    }

    #[test]
    fn a_roster_past_the_prepared_lanes_and_tables_commits_every_player_whole() {
        use engine_native_memory::msvc_string;
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        let mut memory = Fixture::new();
        let humans = 0x10_0000_usize;
        let count = 12;
        let texts: Vec<(String, String)> = (0..count)
            .map(|index| {
                if index % 2 == 0 {
                    (format!("u{index}"), format!("p{index}"))
                } else {
                    (
                        format!("user-name-{index:02}"),
                        format!("player-{index:02}"),
                    )
                }
            })
            .collect();
        let mut block = vec![0_u8; count * engine_native_parts::chuman_layout::STRIDE];
        for (index, (user, name)) in texts.iter().enumerate() {
            let human = &mut block[index * engine_native_parts::chuman_layout::STRIDE..][..engine_native_parts::chuman_layout::STRIDE];
            human[..8].copy_from_slice(&(engine_native_parts::chuman_layout::VTABLE_RVA.0 as u64).to_le_bytes());
            human[chuman::PLAYER_KEY..][..4].copy_from_slice(&(index as i32 + 1).to_le_bytes());
            for (at, text) in [(chuman::FIRST_STRING, user), (chuman::NAME, name)] {
                let string = &mut human[at..][..msvc_string::STRIDE];
                string[..text.len()].copy_from_slice(text.as_bytes());
                string[msvc_string::LENGTH..][..8]
                    .copy_from_slice(&(text.len() as u64).to_le_bytes());
                string[msvc_string::CAPACITY..][..8]
                    .copy_from_slice(&msvc_string::INLINE_CAPACITY.to_le_bytes());
            }
        }
        memory.0.insert(humans, block);
        let set_roster = |memory: &mut Fixture, rows: u32| {
            let header = &mut memory.0.get_mut(&0x2000).unwrap()[cgame_state::PLAYER_RECORDS..];
            header[..8].copy_from_slice(&(humans as u64).to_le_bytes());
            header[8..12].copy_from_slice(&(count as u32).to_le_bytes());
            header[12..16].copy_from_slice(&rows.to_le_bytes());
        };
        set_roster(&mut memory, count as u32);
        attempt_mode(&live, &memory, 1, 100, 1).unwrap();
        assert!(live.metadata().len() < count && live.metadata().stride() < 16);
        let producer = unsafe { &mut *live.producer.get() };
        assert!(producer.players.len() < 2 * count);
        live.roster(&memory, producer, 2, 101).unwrap();
        live.closed_order.store(2, Ordering::Relaxed);
        live.closed_qpc.store(110, Ordering::Relaxed);
        let mut drain = Drain::default();
        let groups = drain.take_live(&live, false).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lost, 0);
        assert_eq!(
            groups[0]
                .batch
                .actors
                .iter()
                .map(|actor| (actor.player, actor.user.clone(), actor.name.clone()))
                .collect::<Vec<_>>(),
            texts
                .iter()
                .enumerate()
                .map(|(index, (user, name))| {
                    (
                        index as i32 + 1,
                        user.clone().into_bytes(),
                        name.clone().into_bytes(),
                    )
                })
                .collect::<Vec<_>>()
        );
        let producer = unsafe { &mut *live.producer.get() };
        let departures: Vec<_> = producer
            .players
            .iter()
            .filter(|player| player.actor != 0 && player.seen)
            .map(|player| (player.actor, 115))
            .collect();
        assert_eq!(departures.len(), count);
        set_roster(&mut memory, 0);
        live.roster(&memory, producer, 3, 115).unwrap();
        live.closed_order.store(3, Ordering::Relaxed);
        live.closed_qpc.store(120, Ordering::Relaxed);
        let groups = drain.take_live(&live, true).unwrap();
        assert_eq!(groups[0].batch.departures, departures);
        assert_eq!(groups[0].lost, 0);
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        assert_eq!(
            (live.metadata().rejected(), live.events().rejected()),
            (0, 0)
        );
    }

    #[test]
    fn losing_the_in_game_idler_closes_the_phase_and_retires_players() {
        for absent in [false, true] {
            let live = live();
            unsafe {
                (*live.producer.get()).active = false;
            }
            let mut memory = Fixture::new();
            attempt_mode(&live, &memory, 1, 100, 1).unwrap();
            let players = 4 * live.events().len() as u64;
            {
                let producer = unsafe { &mut *live.producer.get() };
                producer.players = (1..=players)
                    .map(|actor| CachedPlayer {
                        actor,
                        seen: true,
                        ..Default::default()
                    })
                    .collect();
            }
            if absent {
                memory.0.remove(&native::IDLER_RVA.0);
            } else {
                memory.0.insert(0x5000, 1_u64.to_le_bytes().to_vec());
            }
            memory.0.remove(&(0x1000 + native::SESSION_STATE));
            attempt_mode(&live, &memory, 1, 105, 2).unwrap();
            attempt_mode(&live, &memory, 1, 110, 3).unwrap();
            assert!(!unsafe { (*live.producer.get()).active });
            let groups = Drain::default().take_live(&live, true).unwrap();
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].lost, 0);
            assert_eq!(
                groups[0].batch.departures,
                (1..=players).map(|actor| (actor, 105)).collect::<Vec<_>>()
            );
            assert_eq!(groups[0].batch.spans.len(), 1);
            assert_eq!(groups[0].batch.spans[0].to_qpc, 105);
            assert!(groups[0].batch.spans[0].complete);
            assert!(groups[0].batch.events.is_empty());
            assert_eq!(live.failed.load(Ordering::Acquire), 0);
        }
    }

    #[test]
    fn two_worlds_charge_refusals_once_and_both_lose_complete_coverage() {
        for leave in [false, true] {
            let live = live();
            unsafe {
                (*live.producer.get()).active = false;
            }
            let memory = Fixture::new();
            attempt_mode(&live, &memory, 1, 100, 1).unwrap();
            attempt_mode(&live, &memory, 2, 105, 2).unwrap();
            for _ in 0..3 {
                live.fail(OVERLAP);
            }
            live.failed_qpc.store(107, Ordering::Relaxed);
            if leave {
                attempt_mode(&live, &memory, 0, 108, 3).unwrap();
            }
            live.closed_qpc.store(110, Ordering::Relaxed);
            let mut drain = Drain::default();
            let groups = drain.take_live(&live, false).unwrap();
            assert_eq!(groups.len(), 2);
            assert_eq!(groups.iter().map(|group| group.lost).sum::<u64>(), 3);
            assert_eq!(
                (
                    groups[0].world,
                    groups[0].lost,
                    groups[1].world,
                    groups[1].lost
                ),
                (1, 0, 2, 3)
            );
            for group in &groups {
                assert!(group.context.is_some());
                assert!(!group.batch.spans.is_empty());
                assert!(group.batch.spans.iter().all(|span| !span.complete));
            }
            live.closed_qpc.store(120, Ordering::Relaxed);
            let groups = drain.take_live(&live, true).unwrap();
            assert_eq!(groups.iter().map(|group| group.lost).sum::<u64>(), 0);
            assert!(
                groups
                    .iter()
                    .flat_map(|group| &group.batch.spans)
                    .all(|span| span.complete)
            );
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
        let live = live();
        let producer = unsafe { &mut *live.producer.get() };
        producer.active = false;
        let memory = Fixture::new();
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
        assert_eq!(context.campaign, *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6");
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

    #[cfg(test)]
    fn make_test_hooks(code: &collector_hooks::hook::TestCode, live: &Live) -> (Hooks, usize) {
        unsafe extern "system" fn target() -> usize {
            7
        }
        let (site, sentinel) = code.stub(0, target as *const () as usize);
        let tap = unsafe { Tap::new(site, observe, 0, sentinel) }.unwrap();
        (
            Hooks {
                taps: vec![tap],
                live: std::ptr::from_ref(live),
                owns_live: false,
                publication_attempted: false,
                was_enabled: false,
                transition_open: false,
            },
            site,
        )
    }

    #[cfg(test)]
    fn build_actor_update_burst_live() -> (Live, Vec<(i32, u32, Vec<u8>, Vec<u8>)>, u32) {
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let initial_actors = [
            (1_u64, 1_i32, 1_u32, b"alpha-user".as_slice(), b"alpha-name".as_slice()),
            (2_u64, 2_i32, 2_u32, b"bravo-user".as_slice(), b"bravo-name".as_slice()),
            (3_u64, 3_i32, 3_u32, b"charlie-user".as_slice(), b"charlie-name".as_slice()),
        ];
        let publish_actor = |order: u64, actor: u64, player: i32, country: u32, user: &[u8], name: &[u8]| {
            let needed = user.len() + name.len();
            let mut reservation = live.metadata().reserve_bytes(needed).unwrap();
            let bytes = unsafe { std::slice::from_raw_parts_mut(reservation.bytes(), needed) };
            bytes[..user.len()].copy_from_slice(user);
            bytes[user.len()..].copy_from_slice(name);
            reservation.publish(Metadata::Actor(ActorEntry {
                order,
                world: 1,
                actor,
                player,
                country,
                since_qpc: 101,
                at_qpc: 100 + order,
                user_len: user.len(),
                name_len: name.len(),
                tag_len: 0,
            }));
        };
        for &(actor, player, country, user, name) in &initial_actors {
            publish_actor(2, actor, player, country, user, name);
        }
        live.publish(Entry {
            order: 2,
            world: 1,
            qpc: 101,
            code: 4,
            ..Entry::default()
        })
        .unwrap();
        publish_actor(3, 1, 1, 9, b"alpha-user", b"alpha-renamed");
        live.publish(Entry {
            order: 3,
            world: 1,
            qpc: 102,
            code: 4,
            ..Entry::default()
        })
        .unwrap();
        live.publish(Entry {
            order: 4,
            world: 1,
            qpc: 103,
            hour: 10,
            paused: false,
            code: 5,
            ..Entry::default()
        })
        .unwrap();
        let expected_actors = vec![
            (1, 1, b"alpha-user".to_vec(), b"alpha-name".to_vec()),
            (2, 2, b"bravo-user".to_vec(), b"bravo-name".to_vec()),
            (3, 3, b"charlie-user".to_vec(), b"charlie-name".to_vec()),
            (1, 9, b"alpha-user".to_vec(), b"alpha-renamed".to_vec()),
        ];
        let burst = 2 * live.events().len() as u32 + 3;
        for sequence in 0..burst {
            live.publish(Entry {
                order: 5 + u64::from(sequence),
                world: 1,
                qpc: 104 + u64::from(sequence),
                hour: 10,
                actor: 1,
                player: 1,
                sequence,
                kind: 1,
                origin: 1,
                ..Entry::default()
            })
            .unwrap();
        }
        live.closed_order
            .store(4 + u64::from(burst), Ordering::Relaxed);
        live.closed_qpc.store(200, Ordering::Relaxed);
        (live, expected_actors, burst)
    }

    #[test]
    fn failed_lane_grow_and_metadata_copy_never_drop_drained_commands_or_actors() {
        let (baseline_live, _, _) = build_actor_update_burst_live();
        let mut baseline_drain = Drain::default();
        let total_attempts = {
            let _guard = collector_test_support::NullGuard::new(usize::MAX);
            baseline_drain.take_live(&baseline_live, false).unwrap();
            collector_test_support::ATTEMPTS.load(Ordering::Relaxed)
        };
        assert!(total_attempts > 0);
        let mut saw_grow_denial = false;
        for ordinal in 1..=total_attempts {
            let (live, expected_actors, burst) = build_actor_update_burst_live();
            let initial_events_len = live.events().len();
            let mut drain = Drain::default();
            let first = {
                let _deny = collector_test_support::NullGuard::new(ordinal);
                drain.take_live(&live, false)
            };
            if first.is_ok() && live.events().len() == initial_events_len {
                saw_grow_denial = true;
            }
            let mut groups = first.unwrap_or_default();
            live.closed_qpc.store(210, Ordering::Relaxed);
            groups.extend(drain.take_live(&live, true).unwrap());
            let actors: Vec<_> = groups
                .iter()
                .flat_map(|group| &group.batch.actors)
                .map(|actor| {
                    (
                        actor.player,
                        actor.country,
                        actor.user.clone(),
                        actor.name.clone(),
                    )
                })
                .collect();
            assert_eq!(
                actors, expected_actors,
                "lost or duplicated actor when allocation ordinal {ordinal} failed"
            );
            let events: Vec<_> = groups
                .iter()
                .flat_map(|group| &group.batch.events)
                .map(|event| (event.sequence, event.qpc))
                .collect();
            let expected_events: Vec<_> = (0..burst)
                .map(|sequence| (sequence, 104 + u64::from(sequence)))
                .collect();
            assert_eq!(
                events, expected_events,
                "lost or duplicated commands when allocation ordinal {ordinal} failed"
            );
            assert!(
                live.events().len() > burst as usize,
                "lane did not expand on retry after allocation ordinal {ordinal} failed"
            );
        }
        assert!(saw_grow_denial);
    }

    #[test]
    fn reenabling_enabled_taps_is_idempotent_without_closing_admission() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        let (mut hooks, _) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        assert!(ENABLED.load(Ordering::Acquire));
        collector_hooks::hook::fail_flushes(1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        assert!(ENABLED.load(Ordering::Acquire));
        collector_hooks::hook::fail_flushes(0);
        {
            let mut round = collector_hooks::hook::shared::Round::collect_attach().unwrap();
            collector_hooks::hook::publication::select_family(Some("player_commands"));
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
            assert!(ENABLED.load(Ordering::Acquire));
            round.publish();
            assert!(ENABLED.load(Ordering::Acquire));
            collector_hooks::hook::publication::select_family(Some("player_commands"));
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
            assert!(ENABLED.load(Ordering::Acquire));
            collector_hooks::hook::publication::select_family(None);
        }
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
    }

    #[test]
    fn refused_disable_keeps_admission_open_and_real_disable_reenable_records_loss() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let first = drain.take_live(&live, false).unwrap();
        assert_eq!(first[0].lost, 0);
        assert!(first[0].batch.spans.iter().all(|span| span.complete));
        let (mut hooks, _) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        assert!(ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        collector_hooks::hook::fail_flushes(1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(false) }.is_err());
        assert!(ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 0);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        assert!(ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        let loss_qpc = live.failed_qpc.load(Ordering::Relaxed);
        live.closed_qpc
            .store(loss_qpc.max(120) + 10, Ordering::Relaxed);
        let after = drain.take_live(&live, false).unwrap();
        assert_eq!(after[0].lost, 1);
        assert!(after[0].batch.spans.iter().all(|span| !span.complete));
        collector_hooks::hook::fail_flushes(2);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(false) }.is_err());
        assert!(ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 2);
        let rollback_qpc = live.failed_qpc.load(Ordering::Relaxed);
        let t1 = rollback_qpc.max(loss_qpc + 20) + 10;
        live.closed_qpc.store(t1, Ordering::Relaxed);
        let uncertain_one = drain.take_live(&live, false).unwrap();
        assert_eq!(uncertain_one[0].lost, 1);
        assert!(
            uncertain_one[0]
                .batch
                .spans
                .iter()
                .all(|span| !span.complete)
        );
        let t2 = t1 + 10;
        live.closed_qpc.store(t2, Ordering::Relaxed);
        let uncertain_two = drain.take_live(&live, false).unwrap();
        assert!(
            uncertain_two[0]
                .batch
                .spans
                .iter()
                .all(|span| !span.complete)
        );
        let t3 = t2 + 10;
        live.closed_qpc.store(t3, Ordering::Relaxed);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
        let detached = drain.take_live(&live, true).unwrap();
        assert!(detached[0].batch.spans.iter().all(|span| !span.complete));
    }

    #[test]
    fn detected_tap_mismatch_records_loss_and_blocks_idempotent_fast_path() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let _ = drain.take_live(&live, false).unwrap();
        let (mut hooks, site) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        assert!(ENABLED.load(Ordering::Acquire));
        let saved = unsafe { std::ptr::read(site as *const u8) };
        unsafe { std::ptr::write(site as *mut u8, saved ^ 0xff) };
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        let result = unsafe { hooks.set_enabled(false) };
        assert!(result.is_err());
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(true) }.is_err());
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        let mismatch_qpc = live.failed_qpc.load(Ordering::Relaxed);
        let t1 = mismatch_qpc.max(120) + 10;
        live.closed_qpc.store(t1, Ordering::Relaxed);
        let d1 = drain.take_live(&live, false).unwrap();
        assert!(d1[0].batch.spans.iter().all(|span| !span.complete));
        let t2 = t1 + 10;
        live.closed_qpc.store(t2, Ordering::Relaxed);
        let d2 = drain.take_live(&live, false).unwrap();
        assert!(d2[0].batch.spans.iter().all(|span| !span.complete));
        unsafe { std::ptr::write(site as *mut u8, saved) };
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
    }

    #[test]
    fn batched_restore_defers_drain_until_report_records_refusal() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let initial = drain.take_live(&live, false).unwrap();
        assert!(initial[0].batch.spans.iter().all(|span| span.complete));
        let (mut hooks, _) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        let _scratch = collector_hooks::hook::restoration::Scratch::prepare(4)
            .unwrap()
            .enter();
        {
            let mut batch = collector_hooks::hook::restoration::Batch::collect().unwrap();
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(false));
            collector_hooks::hook::fail_flushes(2);
            assert_eq!(batch.publish(), 0);
            live.closed_qpc.store(120, Ordering::Relaxed);
            let between = drain.take_live(&live, false).unwrap();
            assert!(between.is_empty());
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(false));
        }
        let reported_qpc = live.failed_qpc.load(Ordering::Relaxed);
        live.closed_qpc
            .store(reported_qpc.max(120) + 10, Ordering::Relaxed);
        let after = drain.take_live(&live, false).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].lost, 1);
        assert!(after[0].batch.spans.iter().all(|span| !span.complete));
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
    }

    #[test]
    fn verified_rollback_closes_prior_coverage_uncertainty() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let initial = drain.take_live(&live, false).unwrap();
        assert!(initial[0].batch.spans.iter().all(|span| span.complete));
        let (mut hooks, _) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        collector_hooks::hook::fail_flushes(2);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(false) }.is_err());
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        let t1 = live.failed_qpc.load(Ordering::Relaxed).max(120) + 10;
        live.closed_qpc.store(t1, Ordering::Relaxed);
        let uncertain = drain.take_live(&live, false).unwrap();
        assert_eq!(uncertain[0].lost, 1);
        assert!(uncertain[0].batch.spans.iter().all(|span| !span.complete));
        collector_hooks::hook::fail_flushes(1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(false) }.is_err());
        assert!(ENABLED.load(Ordering::Acquire));
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        let closed_until = live.coverage_uncertain_until.load(Ordering::Acquire);
        let t2 = closed_until.max(t1) + 10;
        live.closed_qpc.store(t2, Ordering::Relaxed);
        let straddling = drain.take_live(&live, false).unwrap();
        assert!(straddling[0].batch.spans.iter().all(|span| !span.complete));
        let t3 = t2 + 10;
        live.closed_qpc.store(t3, Ordering::Relaxed);
        let healthy = drain.take_live(&live, false).unwrap();
        assert_eq!(healthy[0].lost, 0);
        assert!(healthy[0].batch.spans.iter().all(|span| span.complete));
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
    }

    #[test]
    fn batched_restore_other_family_rollback_preserves_command_byte_mismatch() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        unsafe extern "system" fn other_target() -> usize {
            11
        }
        unsafe extern "system" fn other_callback() -> usize {
            13
        }
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let initial = drain.take_live(&live, false).unwrap();
        assert!(initial[0].batch.spans.iter().all(|span| span.complete));
        let (mut hooks, site) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        let (other_site, other_bytes) = code.stub(64, other_target as *const () as usize);
        let mut other =
            unsafe { Hook::new(other_site, other_callback as *const () as usize, other_bytes) }
                .unwrap();
        assert_eq!(other.set_enabled(true), Ok(true));
        let saved = unsafe { std::ptr::read(site as *const u8) };
        unsafe { std::ptr::write(site as *mut u8, saved ^ 0xff) };
        {
            let _scratch = collector_hooks::hook::restoration::Scratch::prepare(8)
                .unwrap()
                .enter();
            {
                let mut batch = collector_hooks::hook::restoration::Batch::collect().unwrap();
                // SAFETY: The fixture owns the taps and keeps them live through publication.
                assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(false));
                assert_eq!(other.set_enabled(false), Ok(false));
                collector_hooks::hook::fail_flushes(1);
                assert_eq!(batch.publish(), 0);
                // SAFETY: The fixture owns the taps and keeps them live through publication.
                assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(false));
                assert_eq!(other.set_enabled(false), Ok(false));
            }
        }
        assert_eq!(live.failed.load(Ordering::Acquire), 1);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert!(unsafe { hooks.set_enabled(true) }.is_err());
        let t1 = live.failed_qpc.load(Ordering::Relaxed).max(120) + 10;
        live.closed_qpc.store(t1, Ordering::Relaxed);
        let after = drain.take_live(&live, false).unwrap();
        assert_eq!(after[0].lost, 1);
        assert!(after[0].batch.spans.iter().all(|span| !span.complete));
        unsafe { std::ptr::write(site as *mut u8, saved) };
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert_eq!(other.set_enabled(false), Ok(true));
    }

    #[test]
    fn shared_attach_disable_defers_drain_until_report_records_refusal() {
        let _guard = collector_hooks::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                collector_hooks::hook::fail_flushes(0);
                collector_hooks::hook::publication::select_family(None);
                ENABLED.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset;
        let code = collector_hooks::hook::TestCode::new();
        let live = live();
        unsafe {
            (*live.producer.get()).active = false;
        }
        attempt_mode(&live, &Fixture::new(), 1, 100, 1).unwrap();
        let mut drain = Drain::default();
        live.closed_qpc.store(110, Ordering::Relaxed);
        let initial = drain.take_live(&live, false).unwrap();
        assert!(initial[0].batch.spans.iter().all(|span| span.complete));
        let (mut hooks, _) = make_test_hooks(&code, &live);
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(true) }, Ok(true));
        {
            let mut round = collector_hooks::hook::shared::Round::collect_attach().unwrap();
            collector_hooks::hook::publication::select_family(Some("player_commands"));
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
            collector_hooks::hook::fail_flushes(2);
            round.publish();
            live.closed_qpc.store(120, Ordering::Relaxed);
            let between = drain.take_live(&live, false).unwrap();
            assert!(between.is_empty());
            collector_hooks::hook::publication::select_family(Some("player_commands"));
            // SAFETY: The fixture owns the taps and keeps them live through publication.
            assert!(unsafe { hooks.set_enabled(false) }.is_err());
            collector_hooks::hook::publication::select_family(None);
        }
        let reported_qpc = live.failed_qpc.load(Ordering::Relaxed);
        live.closed_qpc
            .store(reported_qpc.max(120) + 10, Ordering::Relaxed);
        let after = drain.take_live(&live, false).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].lost, 1);
        assert!(after[0].batch.spans.iter().all(|span| !span.complete));
        // SAFETY: The fixture owns the taps and keeps them live through publication.
        assert_eq!(unsafe { hooks.set_enabled(false) }, Ok(true));
        assert!(!ENABLED.load(Ordering::Acquire));
    }
}
