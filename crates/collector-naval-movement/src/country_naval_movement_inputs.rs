//! The hourly original pass over the strategic navy manager's physical slots,
//! held by country counters for the Ordinary job. The naval mission ledgers
//! borrow each slot's authenticated navy between [`Captured::begin`] and
//! [`Captured::finish`]; slot zero supplies the reserved country's launch
//! naval bases ([`Launch`], held by country appearance). The controller-area
//! province domain of the retired selected-route preview is not read.
//! Attach/live hints prepare the physical slot count; worker storage grows
//! from the original header before the same sample is published.
use crate::table_capture::Arena;
use engine_native::{Memory, ccountry};
use engine_naval_history::country_naval_movement_inputs as native;
use replay_data::rows::PacketHour;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub navies: usize,
}

collector_primitives::capacity_max!(Capacities { navies });

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct LaunchCapacities {
    pub launch_bases: usize,
}

collector_primitives::capacity_max!(LaunchCapacities { launch_bases });

impl Capacities {
    /// Hint floors are observed from the live manager.
    pub fn raise(&mut self, required: Self) {
        crate::country_scale::raise(&mut self.navies, required.navies);
    }
}

impl LaunchCapacities {
    pub fn raise(&mut self, required: Self) {
        crate::country_scale::raise(&mut self.launch_bases, required.launch_bases);
    }
}

#[derive(Default)]
struct NavyRow {
    slot: u32,
    present: bool,
    // Consume all controls and clear before the callback loan is returned.
    inputs: Option<native::NavyInputs>,
    identity_reason: Option<&'static str>,
}

/// The reserved country's launch naval bases from physical slot zero of the
/// same pass. Country appearance holds it with the reserved singleton.
pub struct Launch {
    sample_hour: Option<i32>,
    launch_bases: Arena<ccountry::launch_sources::LaunchNavalBase>,
    launch_scratch: Arena<ccountry::launch_sources::Occurrence>,
    launch_result: Result<ccountry::launch_sources::NavalProvider, &'static str>,
}

impl Launch {
    pub fn new(capacity: LaunchCapacities) -> Self {
        Self {
            sample_hour: None,
            launch_bases: Arena::new(capacity.launch_bases),
            launch_scratch: Arena::new(capacity.launch_bases),
            launch_result: Err("reserved launch sources not supplied"),
        }
    }

    pub fn reset(&mut self) {
        self.sample_hour = None;
        self.launch_bases.reset();
        self.launch_scratch.reset();
        self.launch_result = Err("reserved launch sources not supplied");
    }

    pub fn allocated_bytes(&self) -> usize {
        self.launch_bases.allocated_bytes() + self.launch_scratch.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut LaunchCapacities) {
        self.launch_bases.grow(&mut capacity.launch_bases);
        self.launch_scratch.grow(&mut capacity.launch_bases);
    }

    pub fn required(&self) -> LaunchCapacities {
        LaunchCapacities {
            launch_bases: self.launch_bases.required.max(self.launch_scratch.required),
        }
    }

    fn capacity(&self) -> LaunchCapacities {
        LaunchCapacities {
            launch_bases: self
                .launch_bases
                .first
                .len()
                .min(self.launch_scratch.first.len()),
        }
    }

    pub fn launch_sample_hour(&self) -> Option<i32> {
        self.sample_hour
    }

    pub fn projection_launch(
        &self,
        owned_states: Option<Vec<u32>>,
    ) -> std::io::Result<Option<replay_data::rows::ReservedLaunchSources>> {
        if self.sample_hour.is_none() {
            return Ok(None);
        }
        let naval_bases = match self.launch_result {
            Ok(ccountry::launch_sources::NavalProvider::NativeNull) => {
                replay_data::rows::LaunchNavalBases::NativeNull
            }
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => {
                let values = self
                    .launch_bases
                    .first
                    .get(..self.launch_bases.len)
                    .and_then(|values| values.get(..len))
                    .ok_or_else(|| std::io::Error::other("reserved launch base arena invalid"))?;
                replay_data::rows::LaunchNavalBases::Bases(engine_allocation::collect_vec(
                    values
                        .iter()
                        .map(|value| replay_data::rows::LaunchNavalBase {
                            base_province: value.base_province,
                            country: value.country,
                        }),
                )?)
            }
            Err(_) => replay_data::rows::LaunchNavalBases::Unavailable,
        };
        Ok(Some(replay_data::rows::ReservedLaunchSources {
            sample_hour: PacketHour::Host,
            naval_bases,
            owned_states,
        }))
    }

    fn capture_launch(
        &mut self,
        memory: &impl Memory,
        base: usize,
        observed: &mut Result<native::NavyInputs, &'static str>,
    ) {
        let captured = match observed {
            Ok(inputs) => inputs.capture_launch_growing(
                memory,
                base,
                &mut self.launch_scratch.first,
                &mut self.launch_bases.first,
            ),
            Err(reason) => ccountry::launch_sources::NavalCapture {
                provider: Err(*reason),
                required: 0,
            },
        };
        self.launch_bases.required = captured.required;
        self.launch_scratch.required = captured.required;
        self.launch_result = captured.provider;
        self.launch_bases.len = match captured.provider {
            Ok(ccountry::launch_sources::NavalProvider::Bases(len)) => len,
            _ => 0,
        };
    }
}

pub struct Captured {
    sample_hour: Option<i32>,
    physical_count: Result<usize, &'static str>,
    navies: Arena<NavyRow>,
}

impl Captured {
    pub fn new(capacity: Capacities) -> Self {
        Self {
            sample_hour: None,
            physical_count: Err("physical navy pass not begun"),
            navies: Arena::new(capacity.navies),
        }
    }

    fn clear_controls(&mut self) {
        for row in &mut self.navies.first {
            row.inputs = None;
        }
    }

    pub fn reset(&mut self) {
        self.clear_controls();
        self.sample_hour = None;
        self.physical_count = Err("physical navy pass not begun");
        self.navies.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.navies.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.navies.grow(&mut capacity.navies);
    }

    pub fn required(&self) -> Capacities {
        Capacities {
            navies: self.navies.required,
        }
    }

    pub fn retain_required(&mut self, demand: Capacities) {
        self.navies.required = self.navies.required.max(demand.navies);
    }

    fn capacity(&self) -> Capacities {
        Capacities {
            navies: self.navies.first.len(),
        }
    }

    /// One original holder acquires all physical manager slots before the
    /// mission pass, including aliases and native nulls. Controls survive
    /// until [`Self::finish`]; no borrower may reacquire a failed slot.
    pub fn begin<'a, M: Memory>(
        &mut self,
        memory: &'a M,
        base: usize,
        state: usize,
        hour: Option<i32>,
        launch: &mut Launch,
    ) -> native::Reader<'a, M> {
        self.reset();
        launch.reset();
        self.sample_hour = hour;
        launch.sample_hour = hour;
        let reader = native::Reader::new(memory, base, state);
        self.physical_count = reader.bounds().navy_slots;
        if let Ok(count) = self.physical_count {
            self.navies.required = count;
            if let Err(reason) =
                engine_native::walk::Buffer::Growing(&mut self.navies.first).ensure(count)
            {
                self.physical_count = Err(reason);
                return reader;
            }
            for index in 0..count {
                let Ok(slot) = u32::try_from(index) else {
                    self.physical_count = Err("physical navy slot overflow");
                    break;
                };
                let mut observed = reader.observe_navy(slot);
                if slot == 0 {
                    launch.capture_launch(memory, base, &mut observed);
                }
                if let Some(row) = self.navies.first.get_mut(index) {
                    *row = NavyRow {
                        slot,
                        ..Default::default()
                    };
                    match observed {
                        Ok(inputs) => {
                            row.present = inputs.present;
                            row.inputs = Some(inputs);
                        }
                        Err(reason) => row.identity_reason = Some(reason),
                    }
                    self.navies.len += 1;
                }
            }
            // Empty/null manager returns native null for any requested slot.
            if count == 0 {
                let mut zero = reader.observe_navy(0);
                launch.capture_launch(memory, base, &mut zero);
            }
        }
        reader
    }

    /// Retire every borrowed control once the mission borrowers returned.
    pub fn finish(&mut self) {
        self.clear_controls();
    }

    pub fn physical_count(&self) -> Result<usize, &'static str> {
        self.physical_count
    }

    pub fn borrow_navy(&mut self, slot: u32) -> Result<&mut native::NavyInputs, &'static str> {
        self.physical_count?;
        let row = self
            .navies
            .first
            .get_mut(slot as usize)
            .filter(|row| row.slot == slot && (slot as usize) < self.navies.len)
            .ok_or("physical navy original arena exhausted")?;
        if let Some(reason) = row.identity_reason {
            return Err(reason);
        }
        row.inputs
            .as_mut()
            .ok_or("physical navy original control retired")
    }

    fn refusals(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.physical_count.err().into_iter().chain(
            self.navies.first[..self.navies.len]
                .iter()
                .filter_map(|row| row.identity_reason),
        )
    }
}

/// Spool-thread scalar diagnostics of the physical navy pass and launch bases.
#[derive(Default)]
pub struct Work {
    sampled: u64,
    launch_available: u64,
    launch_native_null: u64,
    launch_refused: u64,
    refusals: Vec<(&'static str, u64)>,
    required: Capacities,
    launch_required: LaunchCapacities,
    capacity: Capacities,
    launch_capacity: LaunchCapacities,
}

impl Work {
    pub fn observe(&mut self, capture: &Captured, launch: &Launch) -> std::io::Result<()> {
        if capture.sample_hour.is_none() {
            return Ok(());
        }
        self.sampled = self.sampled.saturating_add(1);
        self.required.raise(capture.required());
        self.launch_required.raise(launch.required());
        self.capacity = capture.capacity();
        self.launch_capacity = launch.capacity();
        match launch.launch_result {
            Ok(provider) => {
                self.launch_available = self.launch_available.saturating_add(1);
                if matches!(
                    provider,
                    ccountry::launch_sources::NavalProvider::NativeNull
                ) {
                    self.launch_native_null = self.launch_native_null.saturating_add(1);
                }
            }
            Err(_) => self.launch_refused = self.launch_refused.saturating_add(1),
        }
        for reason in capture.refusals().chain(launch.launch_result.err()) {
            let index = match self.refusals.binary_search_by_key(&reason, |entry| entry.0) {
                Ok(index) => index,
                Err(index) => {
                    engine_allocation::reserve(&mut self.refusals, 1)?;
                    self.refusals.insert(index, (reason, 0));
                    index
                }
            };
            self.refusals[index].1 = self.refusals[index].1.saturating_add(1);
        }
        Ok(())
    }

    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "sampled_hours": self.sampled,
            "reserved_launch_naval_available_hours": self.launch_available,
            "reserved_launch_naval_native_null_hours": self.launch_native_null,
            "reserved_launch_naval_refused_hours": self.launch_refused,
            "refusals": crate::status::map(self.refusals.iter().map(|&(reason, count)| (reason, count))),
            "physical_slots_required": self.required.navies,
            "reserved_launch_naval_bases_required": self.launch_required.launch_bases,
            "physical_slots_capacity": self.capacity.navies,
            "reserved_launch_naval_bases_capacity": self.launch_capacity.launch_bases,
        })
    }
}

#[cfg(test)]
mod tests;
