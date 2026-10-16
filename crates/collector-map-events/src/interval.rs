//! The hourly interval: its coverage, sealed on the game thread in constant
//! time, and its row, assembled on the spool thread from the copied frames.
use crate::{hooks, lanes};
use replay::rows::{
    MapEvent, MapEventBuildingDamage, MapEventBuildingReason, MapEventCriticalBombardment,
    MapEventFamily as Family, MapEventFamilyHealth, MapEventFault as Fault, MapEventFaults,
    MapEventGeneralCapture, MapEventIntelEntry, MapEventParadrop, MapEventPayload,
    MapEventResistance, MapEventUnitIntel, MapEventsRow,
};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

static EPOCH: AtomicU64 = AtomicU64::new(0);
/// Observation starts again at the next seal: attach, world reset or load.
static START: AtomicBool = AtomicBool::new(true);
static OUTPUT_LOST: AtomicBool = AtomicBool::new(false);
static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
static NEXT: AtomicU64 = AtomicU64::new(1);
static HOUR: AtomicI32 = AtomicI32::new(0);
static SEALED: AtomicBool = AtomicBool::new(false);
/// The admitted world occurrence of the last seal; zero for none.
static WORLD: AtomicU64 = AtomicU64::new(0);
/// Events an older world occurrence published after its last cut.
static STALE: AtomicU64 = AtomicU64::new(0);

/// Events dropped because their world occurrence ended before a cut.
pub fn stale() -> u64 {
    STALE.load(Ordering::Relaxed)
}

/// Before installing the hooks: continue the recording's epoch and sequence.
pub fn begin(previous_epoch: u64, previous_sequence: u64) -> Result<(), &'static str> {
    lanes::restore_sequence(previous_sequence)?;
    EPOCH.store(previous_epoch, Ordering::SeqCst);
    NEXT.store(previous_sequence + 1, Ordering::SeqCst);
    START.store(true, Ordering::SeqCst);
    SEALED.store(false, Ordering::SeqCst);
    Ok(())
}

/// The world was reset or loaded: coverage starts again.
pub fn invalidate() {
    START.store(true, Ordering::SeqCst);
}

/// An interval's events were lost before they were written.
pub fn lost_output() {
    OUTPUT_LOST.store(true, Ordering::SeqCst);
}

/// One sealed interval's boundaries and coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub epoch: u64,
    pub from_hour: i32,
    pub to_hour: i32,
    pub first_sequence: u64,
    pub next_sequence: u64,
    pub health: [MapEventFamilyHealth; Family::COUNT],
    pub seal: lanes::Seal,
    /// The admitted world occurrence the interval lists events of; `None`
    /// lists none.
    pub world: Option<u64>,
}

/// Game thread, at the hourly cut of `hour`, after the cut's world boundary
/// admitted `world` (`world_lifetime::token`). Constant time. A new or
/// unadmitted occurrence starts the observation again.
pub fn seal(hour: i32, world: Option<u64>) -> Interval {
    let seal = lanes::seal();
    let changed = WORLD.swap(world.unwrap_or(0), Ordering::SeqCst) != world.unwrap_or(0);
    let start = START.swap(false, Ordering::SeqCst) | changed | world.is_none();
    if start {
        EPOCH.fetch_add(1, Ordering::SeqCst);
        hooks::reset_missed();
    }
    let lost = OUTPUT_LOST.swap(false, Ordering::SeqCst);
    // An event left in flight is listed by a later interval, which is then
    // incomplete as well.
    let in_flight = IN_FLIGHT.swap(!seal.complete, Ordering::SeqCst) || !seal.complete;
    let health = Family::ALL.map(|family| {
        let installed = hooks::installed(family);
        let enabled = installed && hooks::enabled(family);
        let mut faults =
            MapEventFaults::from_bits(hooks::take_faults(family)).unwrap_or_else(|_| {
                let mut all = MapEventFaults::default();
                all.insert(Fault::Read);
                all
            });
        for (applies, fault) in [
            (!installed, Fault::Unavailable),
            (!enabled, Fault::Disabled),
            (start, Fault::ObservationStart),
            (lost, Fault::OutputLost),
            (in_flight, Fault::InFlight),
        ] {
            if applies {
                faults.insert(fault);
            }
        }
        MapEventFamilyHealth {
            installed,
            enabled,
            complete: enabled && faults.is_empty(),
            faults,
            missed: hooks::missed(family),
        }
    });
    let from_hour = if SEALED.swap(true, Ordering::SeqCst) && !start {
        HOUR.load(Ordering::SeqCst).min(hour)
    } else {
        hour.saturating_sub(1)
    };
    HOUR.store(hour, Ordering::SeqCst);
    let first_sequence = NEXT.load(Ordering::SeqCst);
    let next_sequence = seal.last + 1;
    NEXT.store(next_sequence.max(first_sequence), Ordering::SeqCst);
    Interval {
        epoch: EPOCH.load(Ordering::SeqCst),
        from_hour,
        to_hour: hour,
        first_sequence,
        next_sequence: next_sequence.max(first_sequence),
        health,
        seal,
        world,
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], &'static str> {
        if self.bytes.len() < len {
            return Err("map event frame short");
        }
        let (head, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bool(&mut self) -> Result<bool, &'static str> {
        Ok(self.u8()? != 0)
    }
}

/// One frame's payload, written by the family's callback.
pub fn payload(family: u8, bytes: &[u8]) -> Result<MapEventPayload, &'static str> {
    let family = Family::from_index(family).map_err(|_| "map event frame family unknown")?;
    let mut input = Reader { bytes };
    let payload = match family {
        Family::FakeDivisionRevealed | Family::RailwayGunOverrun => {
            let owner = input.u32()?;
            let anchor_province = input.u32()?;
            let visibility_province = input.u32()?;
            let fog = input.bool()?;
            let count = input.u32()? as usize;
            if input.bytes.len() != count.checked_mul(5).ok_or("map event intel overflow")? {
                return Err("map event intel length differs");
            }
            let mut intel = Vec::new();
            engine::allocation::reserve_exact(&mut intel, count)
                .map_err(|_| "map event assembly allocation failed")?;
            for _ in 0..count {
                intel.push(MapEventIntelEntry {
                    country: input.u32()?,
                    level: input.u8()? as i8,
                });
            }
            let unit = MapEventUnitIntel {
                owner,
                anchor_province,
                visibility_province,
                fog,
                intel,
            };
            if family == Family::FakeDivisionRevealed {
                MapEventPayload::FakeDivisionRevealed(unit)
            } else {
                MapEventPayload::RailwayGunOverrun(unit)
            }
        }
        Family::CriticalShoreBombardment => {
            MapEventPayload::CriticalShoreBombardment(MapEventCriticalBombardment {
                province: input.u32()?,
            })
        }
        Family::GeneralCaptured => MapEventPayload::GeneralCaptured(MapEventGeneralCapture {
            owner: input.u32()?,
            captor: input.u32()?,
            province: input.u32()?,
        }),
        Family::ParadropLanding => MapEventPayload::ParadropLanding(MapEventParadrop {
            province: input.u32()?,
            country: input.u32()?,
            lost: input.bool()?,
            fog: input.bool()?,
        }),
        Family::ResistanceActivity => {
            let province = input.u32()?;
            let controller = input.u32()?;
            let overlord = input.u32()?;
            let len = input.u32()? as usize;
            let key = engine::allocation::copy_vec(input.take(len)?)
                .map_err(|_| "map event assembly allocation failed")?;
            MapEventPayload::ResistanceActivity(MapEventResistance {
                province,
                controller,
                overlord,
                key,
            })
        }
        Family::BuildingDamage => MapEventPayload::BuildingDamage(MapEventBuildingDamage {
            building_type: input.u32()?,
            province: input.u32()?,
            controller: input.u32()?,
            reason: MapEventBuildingReason::from_native(u32::from(input.u8()?))
                .ok_or("map event building reason unknown")?,
        }),
    };
    if !input.bytes.is_empty() {
        return Err("map event frame has trailing bytes");
    }
    Ok(payload)
}

/// The interval's row from its copied frames.
pub fn row(
    interval: &Interval,
    assembled: &lanes::Assembled,
) -> Result<MapEventsRow, &'static str> {
    let mut events = Vec::new();
    engine::allocation::reserve_exact(&mut events, assembled.frames.len())
        .map_err(|_| "map event assembly allocation failed")?;
    for frame in &assembled.frames {
        // An event of an ended occurrence belongs to no interval of this one.
        if interval.world != Some(frame.world) {
            STALE.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        events.push(MapEvent {
            sequence: frame.sequence,
            hour: frame.hour,
            payload: payload(
                frame.family,
                &assembled.bytes[frame.start..frame.start + frame.len],
            )?,
        });
    }
    let row = MapEventsRow {
        epoch: interval.epoch,
        from_hour: interval.from_hour,
        to_hour: interval.to_hour,
        first_sequence: interval.first_sequence,
        next_sequence: interval.next_sequence,
        health: interval.health,
        events,
    };
    row.validate()
        .map_err(|_| "map event interval failed its own validation")?;
    Ok(row)
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    EPOCH.store(0, Ordering::SeqCst);
    START.store(true, Ordering::SeqCst);
    OUTPUT_LOST.store(false, Ordering::SeqCst);
    IN_FLIGHT.store(false, Ordering::SeqCst);
    NEXT.store(1, Ordering::SeqCst);
    HOUR.store(0, Ordering::SeqCst);
    SEALED.store(false, Ordering::SeqCst);
    WORLD.store(0, Ordering::SeqCst);
}
