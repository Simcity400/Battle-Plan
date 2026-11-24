use super::*;
use engine_events::cnuke;
use engine_native::{ccountry, cgame_state, cprovince, cstate};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static OBSERVATION_OWNER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(super) fn owns_observation() -> bool {
    OBSERVATION_OWNER.get()
}
struct Owner {
    _serial: std::sync::MutexGuard<'static, ()>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::SeqCst);
        OPEN.store(0, Ordering::SeqCst);
        OBSERVATION_OWNER.set(false);
    }
}

fn clock(hour: i32) -> Option<Clock> {
    Some(Clock {
        hour,
        play_session_started_at: 4,
        game_state: 0,
        play_time: 0,
        seed: 0,
        campaign_id: Default::default(),
        speed: 0,
        tick_busy: 0,
    })
}

fn ready() -> Owner {
    let serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    OBSERVATION_OWNER.set(true);
    LEDGER.reset();
    OPEN.store(0, Ordering::SeqCst);
    EQUIPMENT.store(IDLE, Ordering::SeqCst);
    LEDGER.prepare(64);
    assert!(unsafe { LEDGER.drain(&mut Vec::new()) }.is_err());
    unsafe { LEDGER.boundary(clock(1)) };
    unsafe { LEDGER.boundary(clock(2)) };
    Owner { _serial: serial }
}

fn drained() -> Vec<Item> {
    let mut entries = Vec::with_capacity(64);
    unsafe { LEDGER.boundary(clock(3)) };
    unsafe { LEDGER.drain(&mut entries) }.unwrap();
    entries.into_iter().map(|entry| entry.item).collect()
}

#[test]
fn forwarded_equipment_is_recorded_only_on_the_open_strike_thread() {
    let _serial = ready();
    assert!(equipment_call(None).is_none());
    OPEN.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    std::thread::spawn(|| {
        assert!(equipment_call(None).is_none());
        equipment(false, 1, 1, 1);
    })
    .join()
    .unwrap();
    {
        let _call = equipment_call(None).unwrap();
        equipment(false, 3, 7, 200_000);
        equipment(true, 3, 8, -5);
        equipment_observed();
    }
    drop(equipment_call(None).unwrap());
    close();
    assert!(equipment_call(None).is_none());
    assert_eq!(
        drained(),
        [
            Item::Equipment {
                wing: false,
                country: 3,
                variant: 7,
                amount_raw: 200_000
            },
            Item::Equipment {
                wing: true,
                country: 3,
                variant: 8,
                amount_raw: -5
            },
            Item::Refused {
                part: Part::Equipment,
                reason: "equipment destruction did not observe a strike's pool writer"
            },
        ]
    );
}

struct World {
    storage: Vec<u64>,
    root: Box<u64>,
    base: usize,
}
const GAME: usize = 0x100;
const NUKE: usize = 0x600;
const COUNTRY: usize = 0x800;
const PROVINCE: usize = 0xc00;
const STATE: usize = 0x1000;
const SLOTS: usize = 0x1400;
const LINKS: usize = 0x1500;
const RESERVED: usize = 0x1600;
const OWNER: usize = 0x1700;
impl World {
    fn new() -> Self {
        let mut world = Self {
            storage: vec![0; 0x2000 / 8],
            root: Box::new(0),
            base: 0,
        };
        world.base = (&raw const *world.root as usize) - cgame_state::SINGLETON_RVA.0;
        *world.root = world.at(GAME) as u64;
        world.u32(GAME + cgame_state::CURRENT_HOUR, 60_827_795);
        world.vtable(NUKE, cnuke::VTABLE_RVA.0);
        world.u64(NUKE + cnuke::COUNTRY, world.at(COUNTRY) as u64);
        world.u32(NUKE + cnuke::TYPE, 0);
        world.array(GAME + cgame_state::COUNTRIES, SLOTS, 3);
        world.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 3);
        for (slot, country) in [RESERVED, COUNTRY, OWNER].into_iter().enumerate() {
            world.u64(SLOTS + slot * 8, world.at(country) as u64);
            world.vtable(country, ccountry::VTABLE_RVA.0);
            world.u32(country + ccountry::ID, slot as u32);
            world.u32(country + ccountry::SURRENDER_TARGET, 0x100 + slot as u32);
            world.u32(LINKS + slot * 4, slot as u32);
        }
        world.vtable(PROVINCE, cprovince::VTABLE_RVA.0);
        world.u32(PROVINCE + cprovince::ID, 13_000);
        world.u64(PROVINCE + cprovince::STATE, world.at(STATE) as u64);
        world.u32(PROVINCE + cprovince::CONTROLLER, 77);
        world.vtable(STATE, cstate::VTABLE_RVA.0);
        world.u32(STATE + cstate::ID, 1_204);
        world.u32(STATE + cstate::OWNER, 2);
        world
    }
    fn array(&mut self, offset: usize, data: usize, count: u32) {
        self.u64(offset, self.at(data) as u64);
        self.u32(offset + 8, count);
        self.u32(offset + 12, count);
    }
    fn at(&self, offset: usize) -> usize {
        self.storage.as_ptr() as usize + offset
    }
    fn bytes(&mut self) -> &mut [u8] {
        unsafe {
            std::slice::from_raw_parts_mut(self.storage.as_mut_ptr().cast(), self.storage.len() * 8)
        }
    }
    fn u32(&mut self, offset: usize, value: u32) {
        self.bytes()[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn u64(&mut self, offset: usize, value: u64) {
        self.bytes()[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn vtable(&mut self, offset: usize, rva: usize) {
        self.u64(offset, (self.base + rva) as u64);
    }
}

#[test]
fn a_strike_records_its_target_and_refuses_each_part_not_forwarded() {
    let _serial = ready();
    let world = World::new();
    BASE.store(world.base, Ordering::SeqCst);
    ENABLED.store(true, Ordering::SeqCst);
    for part in [
        Part::WarSupport,
        Part::Buildings,
        Part::Units,
        Part::Equipment,
    ] {
        forwarder(part, part == Part::Buildings);
    }
    open(world.at(NUKE), world.at(PROVINCE));
    assert!(current());
    close();
    assert!(!current());
    ENABLED.store(false, Ordering::SeqCst);
    assert_eq!(
        drained(),
        [
            Item::Strike {
                hour: 60_827_795,
                strike: native::Strike {
                    launcher: engine_events::event_feed_sites::CountryIdentity {
                        reference: 1,
                        id: 1,
                        instance: 0x101,
                    },
                    nuke_type: 0,
                    province: 13_000,
                    state: 1_204,
                    owner: engine_events::event_feed_sites::CountryIdentity {
                        reference: 2,
                        id: 2,
                        instance: 0x102,
                    },
                    controller: 77,
                },
            },
            Item::Refused {
                part: Part::WarSupport,
                reason: "war support writer not forwarded"
            },
            Item::Refused {
                part: Part::Equipment,
                reason: "equipment destruction writers not forwarded"
            },
        ]
    );
}

#[test]
fn an_unreadable_strike_makes_its_interval_unavailable() {
    let _serial = ready();
    let mut world = World::new();
    world.u64(NUKE, 0);
    BASE.store(world.base, Ordering::SeqCst);
    ENABLED.store(true, Ordering::SeqCst);
    open(world.at(NUKE), world.at(PROVINCE));
    ENABLED.store(false, Ordering::SeqCst);
    assert!(!current());
    unsafe { LEDGER.boundary(clock(3)) };
    assert!(unsafe { LEDGER.drain(&mut Vec::with_capacity(8)) }.is_err());
}
