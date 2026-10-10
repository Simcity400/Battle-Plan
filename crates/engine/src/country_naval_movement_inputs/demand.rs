//! Advisory sizing from demanded moving owners and their stored foreign slots.
//! Only these navies' headers are counted; no nearest payload is copied.
use super::*;

/// Caller-owned observer controls indexed by physical slot. Their cheap root
/// bound is independent of payload demand. Clear before and after each pass.
#[derive(Default)]
pub struct DemandSlot {
    own: bool,
    navy: Option<Result<NavyInputs, &'static str>>,
}

impl DemandSlot {
    pub fn clear(&mut self) {
        self.own = false;
        self.navy = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemandedDemand {
    pub live: LiveDemand,
    /// max(actual country count, physical navy count), never a payload bound.
    pub slot_scratch_required: usize,
    /// Maximum observed own foreign-list count for one reusable scratch list.
    pub foreign_scratch_required: usize,
    /// Maximum one-owner/reserved theatre roster count for a reusable arena.
    pub theatre_header_scratch_required: usize,
    /// Unique requested physical own/foreign keys. A short foreign scratch
    /// leaves this a known floor rather than an invented all-manager count.
    pub physical_slots: HeaderDemand,
    /// Independent of any capacity-first aggregate failure. A scratch-only
    /// advisory pass may be rerun only when this sticky diagnostic is absent.
    pub first_native_failure: Option<&'static str>,
    pub owners: HeaderDemand,
}

impl<M: Memory> Reader<'_, M> {
    /// Advisory off-thread sizing from the caller's moving-TF census. Own
    /// slots and actual stored foreign physical slots are deduplicated. Each
    /// navy's controls are observed once. Only foreign-list payloads identify
    /// dependencies; no nearest payload/getter/search runs.
    ///
    /// A short scratch arena keeps all known header floors and reports its
    /// complete requirement. An incomplete dependency graph must not become
    /// an all-navies nearest allocation. Acquisition observes a separate pass.
    pub fn live_demand(
        &self,
        countries: &ccountry::Reader<'_, M>,
        owners: impl IntoIterator<Item = Result<ccountry::Country, &'static str>>,
        slots: &mut [DemandSlot],
        foreign_scratch: &mut [u32],
    ) -> DemandedDemand {
        for slot in slots.iter_mut() {
            slot.clear();
        }
        let bounds = self.bounds();
        let mut observed = DemandedDemand {
            live: LiveDemand {
                bounds,
                navy: NavyDemand::default(),
                theatres: HeaderDemand::default(),
                theatre_areas: HeaderDemand::default(),
            },
            slot_scratch_required: countries.count() as usize,
            foreign_scratch_required: 0,
            theatre_header_scratch_required: 0,
            physical_slots: HeaderDemand::default(),
            first_native_failure: None,
            owners: HeaderDemand::default(),
        };
        match bounds.navy_slots {
            Ok(count) => observed.slot_scratch_required = observed.slot_scratch_required.max(count),
            Err(reason) => {
                observed.live.navy.nearest_bases.observe(Err(reason));
                observed.first_native_failure.get_or_insert(reason);
            }
        }
        if let Err(reason) = bounds.provinces {
            observed.first_native_failure.get_or_insert(reason);
        }
        for owner in owners {
            let owner = match owner {
                Ok(owner) if owner.pointer != 0 && owner.id != 0 && owner.canonical_slot != 0 => {
                    owner
                }
                Ok(_) => {
                    observed.owners.observe(Err(
                        "naval movement demand owner is not authenticated positive country",
                    ));
                    observed.first_native_failure.get_or_insert(
                        "naval movement demand owner is not authenticated positive country",
                    );
                    continue;
                }
                Err(reason) => {
                    observed.owners.observe(Err(reason));
                    observed.first_native_failure.get_or_insert(reason);
                    continue;
                }
            };
            let Some(control) = slots.get_mut(owner.canonical_slot as usize) else {
                observed
                    .owners
                    .observe(Err("naval movement demand slot scratch exhausted"));
                observed
                    .live
                    .navy
                    .nearest_bases
                    .observe(Err("naval movement demand slot scratch exhausted"));
                continue;
            };
            if control.own {
                continue;
            }
            control.own = true;
            observed.owners.observe(Ok(1));
            self.theatre_demand(
                countries,
                Owner::Positive(owner),
                &mut observed.live,
                &mut observed.theatre_header_scratch_required,
            );
            self.observe_demanded_slot(
                owner.canonical_slot,
                slots,
                &mut observed.live.navy.nearest_bases,
                &mut observed.physical_slots,
                &mut observed.first_native_failure,
            );
            let foreign = {
                let control = &mut slots[owner.canonical_slot as usize];
                match control.navy.as_mut().expect("slot observed above") {
                    Ok(inputs) => {
                        let demand = inputs.demand();
                        for field in [
                            demand.nearest_bases,
                            demand.foreign_navy_slots,
                            demand.supply_hubs,
                        ] {
                            if let Some(reason) = field.first_failure {
                                observed.first_native_failure.get_or_insert(reason);
                            }
                        }
                        add(
                            &mut observed.live.navy.foreign_navy_slots,
                            demand.foreign_navy_slots,
                        );
                        add(&mut observed.live.navy.supply_hubs, demand.supply_hubs);
                        observed.foreign_scratch_required = observed
                            .foreign_scratch_required
                            .max(demand.foreign_navy_slots.required);
                        let result = inputs
                            .capture_foreign_navy_slots(self.memory, foreign_scratch)
                            .len;
                        if demand.foreign_navy_slots.required <= foreign_scratch.len()
                            && let Err(reason) = result
                        {
                            observed.first_native_failure.get_or_insert(reason);
                        }
                        result
                    }
                    Err(reason) => {
                        observed.live.navy.foreign_navy_slots.observe(Err(*reason));
                        observed.live.navy.supply_hubs.observe(Err(*reason));
                        Err(*reason)
                    }
                }
            };
            match foreign {
                Ok(count) => {
                    for &foreign in &foreign_scratch[..count] {
                        self.observe_demanded_slot(
                            foreign,
                            slots,
                            &mut observed.live.navy.nearest_bases,
                            &mut observed.physical_slots,
                            &mut observed.first_native_failure,
                        );
                    }
                }
                Err(reason) => {
                    observed.live.navy.nearest_bases.observe(Err(reason));
                    observed.physical_slots.observe(Err(reason));
                }
            }
        }
        match self.reserved_owner() {
            Ok(owner) => self.theatre_demand(
                countries,
                owner,
                &mut observed.live,
                &mut observed.theatre_header_scratch_required,
            ),
            Err(reason) => {
                observed.live.theatres.observe(Err(reason));
                observed.live.theatre_areas.observe(Err(reason));
            }
        }
        for field in [observed.live.theatres, observed.live.theatre_areas] {
            if let Some(reason) = field.first_failure {
                observed.first_native_failure.get_or_insert(reason);
            }
        }
        for slot in slots {
            slot.clear();
        }
        foreign_scratch.fill(0);
        observed
    }

    fn observe_demanded_slot(
        &self,
        slot: u32,
        controls: &mut [DemandSlot],
        demand: &mut HeaderDemand,
        physical_slots: &mut HeaderDemand,
        first_native_failure: &mut Option<&'static str>,
    ) {
        let Some(control) = controls.get_mut(slot as usize) else {
            demand.observe(Err("naval movement demand physical slot outside scratch"));
            physical_slots.observe(Err("naval movement demand physical slot outside scratch"));
            if self
                .bounds()
                .navy_slots
                .is_ok_and(|count| count != 0 && slot as usize >= count)
            {
                first_native_failure
                    .get_or_insert("naval movement physical navy slot outside manager");
            }
            return;
        };
        if control.navy.is_some() {
            return;
        }
        physical_slots.observe(Ok(1));
        let input = self.observe_navy(slot);
        match &input {
            Ok(input) => {
                let observed = input.demand().nearest_bases;
                add(demand, observed);
                if let Some(reason) = observed.first_failure {
                    first_native_failure.get_or_insert(reason);
                }
            }
            Err(reason) => {
                demand.observe(Err(*reason));
                physical_slots.observe(Err(*reason));
                first_native_failure.get_or_insert(*reason);
            }
        }
        control.navy = Some(input);
    }

    fn theatre_demand(
        &self,
        countries: &ccountry::Reader<'_, M>,
        owner: Owner,
        demand: &mut LiveDemand,
        header_scratch_required: &mut usize,
    ) {
        let theatres = match owner
            .pointer()
            .and_then(|country| array(self.memory, country, ccountry::THEATRES, 8))
        {
            Ok(theatres) => theatres,
            Err(reason) => {
                demand.theatres.observe(Err(reason));
                demand.theatre_areas.observe(Err(reason));
                return;
            }
        };
        demand.theatres.observe(Ok(theatres.count as usize));
        *header_scratch_required = (*header_scratch_required).max(theatres.count as usize);
        // A bad identity/header preserves later sibling demand; a bad pointer
        // batch stops only this owner's roster, not subsequent owners.
        let result =
            cpdx_array::pointers(
                self.memory,
                theatres,
                "naval movement demand theatre pointer unreadable",
                |_, theatre| {
                    match self.theatre_header(Some(countries), owner, theatre) {
                        Ok(input) => demand.theatre_areas.observe(
                            input.areas.as_ref().ok_or(CONSUMED).and_then(|areas| {
                                areas
                                    .as_ref()
                                    .map(|areas| areas.count as usize)
                                    .map_err(|&reason| reason)
                            }),
                        ),
                        Err(reason) => demand.theatre_areas.observe(Err(reason)),
                    }
                    Ok(())
                },
            );
        if let Err(reason) = result {
            demand.theatre_areas.observe(Err(reason));
        }
    }
}

fn add(destination: &mut HeaderDemand, source: HeaderDemand) {
    destination.observe(Ok(source.required));
    if let Some(reason) = source.first_failure {
        destination.observe(Err(reason));
    }
}
