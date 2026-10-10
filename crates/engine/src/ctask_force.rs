//! Task-force complete-object layout with primary and secondary bases.
//! Reference type is separate from class: fleet-space and ship-space handles
//! can both occur. Location follows the stored province pointer, independently
//! of inherited supply cache. Shared unit handle, supply, owner, location and
//! definition fields are in `cunit`.
//! `cfleet::walk_counter_inputs` visits authenticated task forces for movement
//! target, first combat role and both routes through `cunit::read_map_motion`,
//! shared with railway guns. Fixtures cover routes, roles and truncation.

use crate::{
    Memory, ccountry, cfleet,
    cpdx_array::Array,
    cunit, read,
    walk::{Buffer, Outcome},
};

mod fresh_eta;
pub use fresh_eta::{FreshEtaInputs, MissionRiskInputs, OriginalRiskShips};
mod cached_name;
pub(crate) use cached_name::cached_name_buffer;
pub use cached_name::{cached_name, cached_name_demand};
pub mod port_mission;

/// Primary vtable at displacement zero.
pub const VTABLE_RVA: usize = 0x0294_c480;
/// Secondary vtable at displacement +0x10.
pub const SECONDARY_VTABLE_RVA: usize = 0x0294_c660;
/// Supply-consumer/reference-object secondary base.
pub const SECONDARY_BASE: usize = 0x10;
/// Default reference type assigned through fleet-ID minting.
pub const DEFAULT_HANDLE_TYPE: u32 = 61;
/// CTaskForce ship-space reference type also present in engine saves.
pub const OBSERVED_SHIP_SPACE_HANDLE_TYPE: u32 = 51;
/// Parent fleet pointer.
pub const FLEET: usize = 0x340;
/// Ship-pointer array.
pub const SHIPS: usize = 0x348;
/// Ship-array count, u32.
pub const SHIP_COUNT: usize = 0x354;
/// Mission enum, u32, including the reserve discriminator.
pub const MISSION: usize = 0x374;
/// Embedded naval mission initialized by task-force construction.
pub const NAVAL_MISSION: usize = 0x360;
/// Repair-parent full handle, serialized as `repair_parent`; linked forces
/// are excluded from roster presentation.
pub const REPAIR_PARENT: usize = 0x4b0;
/// Detached mission, i32, saved as `repair_last_mission`, restored on return
/// and selected while the detached marker is nonzero.
pub const REPAIR_LAST_MISSION: usize = 0x4f0;
/// Effective mission query.
pub const EFFECTIVE_MISSION_RVA: usize = 0x00d5_a660;
/// Counter-strength dispatch offset; ratio is total current ship strength
/// over total maximum strength with each maximum floored at 100.
pub const STRENGTH_BAR_SLOT: usize = 0x118;
/// Counter-organisation dispatch offset; ratio averages each ship's current
/// organisation over its effective maximum.
pub const ORGANISATION_BAR_SLOT: usize = 0x138;
/// Counter-strength ratio query.
pub const STRENGTH_RATIO_RVA: usize = 0x00d5_b0e0;
/// Counter-organisation ratio query.
pub const ORGANISATION_RATIO_RVA: usize = 0x00d5_a160;
/// Detached-activity enum, u32.
pub const DETACHED_ACTIVITY: usize = 0x4b8;
/// Repair-target province pointer.
pub const REPAIR_TARGET: usize = 0x4c0;
/// Current fuel usage, i64, read from fleet-vector task forces.
pub const CURRENT_FUEL_USAGE: usize = 0x500;
/// Reserve mission enum value.
pub const MISSION_RESERVE: u32 = 8;
/// Constructor installing all task-force interface identities.
pub const CONSTRUCTOR_RVA: usize = 0x00d4_eaf0;
/// Destructor restoring the same interface identities.
pub const DESTRUCTOR_RVA: usize = 0x00d4_edf0;
/// Task-force creation and fleet-ID assignment.
pub const CREATE_RVA: usize = 0x00d5_4560;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: u32,
    pub logical_owner: u32,
    pub expeditionary_owner: u32,
    pub province: u32,
    pub mission: u32,
    /// [`cunit::ORDINARY_ROUTE_COUNT`], the signed route length.
    pub route_count: i32,
    /// Combat presence tests the signed count for positive.
    pub combat_references: i32,
}

pub(crate) struct Reader<'a, M: Memory> {
    pub memory: &'a M,
    pub base: usize,
    pub countries: &'a ccountry::Reader<'a, M>,
    pub provinces: Array,
}

impl<M: Memory> Reader<'_, M> {
    pub(crate) fn identity(
        &self,
        pointer: usize,
        fleet_pointer: usize,
    ) -> Result<u64, &'static str> {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("task force address overflow")
        };
        let primary = self
            .base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let secondary = self
            .base
            .checked_add(SECONDARY_VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(self.memory, pointer) != Some(primary as u64)
            || read::<u64>(self.memory, at(SECONDARY_BASE)?) != Some(secondary as u64)
        {
            return Err("task force vtable mismatch");
        }
        let id =
            read::<u64>(self.memory, at(cunit::HANDLE)?).ok_or("task force handle unreadable")?;
        if id == 0 {
            return Err("non-null task force has null handle");
        }
        if read::<u64>(self.memory, at(FLEET)?) != Some(fleet_pointer as u64) {
            return Err("task force parent differs from fleet roster");
        }
        Ok(id)
    }

    pub(crate) fn row(
        &self,
        pointer: usize,
        id: u64,
        fleet: u64,
        owner: u32,
    ) -> Result<Row, &'static str> {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("task force address overflow")
        };
        let mut owners = [0; 12];
        let mut location = [0; 8];
        let mut mission = [0; 4];
        let mut route = [0; 4];
        let mut combats = [0; 4];
        if self.memory.copy_scattered(
            [
                at(cunit::ROSTER_OWNER)?,
                at(cunit::LOCATION)?,
                at(MISSION)?,
                at(cunit::ORDINARY_ROUTE_COUNT)?,
                at(cunit::COMBAT_REFERENCE_COUNT)?,
            ],
            &mut [
                &mut owners,
                &mut location,
                &mut mission,
                &mut route,
                &mut combats,
            ],
        ) != 5
        {
            return Err("task force fields unreadable");
        }
        let roster_owner = u32::from_le_bytes(owners[0..4].try_into().expect("fixed span"));
        if self.countries.resolve(roster_owner)?.canonical_slot != owner {
            return Err("task force owner differs from fleet country");
        }
        let province = cfleet::province(
            self.memory,
            self.base,
            self.provinces,
            usize::try_from(u64::from_le_bytes(location))
                .map_err(|_| "task force location overflow")?,
        )?;
        Ok(Row {
            id,
            fleet,
            roster_owner,
            expeditionary_owner: u32::from_le_bytes(owners[4..8].try_into().expect("fixed span")),
            logical_owner: u32::from_le_bytes(owners[8..12].try_into().expect("fixed span")),
            province,
            mission: u32::from_le_bytes(mission),
            route_count: i32::from_le_bytes(route),
            combat_references: i32::from_le_bytes(combats),
        })
    }
}

/// Task-force presentation operands beyond the TaskForces row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CounterRow {
    pub id: u64,
    pub is_in_regions: u8,
    pub running_away: u8,
    pub engagement_rule: i32,
    pub mission_region_count: i32,
    pub bombardment_region: u32,
    pub repair_parent: u64,
    pub detached_activity: i32,
    pub repair_last_mission: i32,
    /// Received-supply ratio, signed fixed5, read by the low-supply child.
    pub received_supply_ratio: i64,
    /// Attached naval-HQ count; nonzero shows the combat control for the viewer's
    /// task force when it is in combat.
    pub attached_headquarters: i32,
    /// Uncommitted retained CUnit+0x258 bytes, independently copied after the
    /// established counter prefix. None refuses only the cached-name field.
    pub name: Option<crate::cname_group_member::Span>,
    /// Stored supply footer operands independent of motion prefix and cached
    /// name. Ship count/demand are read through the secondary supply object.
    /// Origin failure withholds only that fragment.
    pub supply: Option<crate::cunit_supply_inputs::Inputs>,
    pub supply_origins_start: usize,
    pub supply_failure: Option<crate::cunit_supply_inputs::Failure>,
    /// Ordered port searching/status operands, independently captured after
    /// the established prefix, retained name and supply inputs.
    pub port_mission: Option<port_mission::Inputs>,
}

/// Caller-owned origin storage for admitted task-force supply suffixes.
/// Workers grow from the original distributed-supply header; fixed callback
/// storage retains an explicit shortage.
pub struct SupplyOrigins<'a> {
    pub origins: Buffer<'a, crate::cunit_supply_inputs::Origin>,
    pub len: usize,
    pub required: usize,
}

/// Attached naval-HQ array whose count gates combat presentation.
pub const ATTACHED_HEADQUARTERS: usize = 0x728;

/// Authenticate embedded naval mission from its vtable and task-force
/// back-pointer in the first 16 bytes. Every reader checks each observed
/// force independently of sibling buffer capacity.
pub(crate) fn authenticate_mission(
    header: &[u8],
    base: usize,
    task: usize,
) -> Result<(), &'static str> {
    use crate::cnaval_mission as mission;
    let vtable = base
        .checked_add(mission::VTABLE_RVA)
        .ok_or("module address overflow")?;
    let word = |at: usize| {
        header
            .get(at..at + 8)
            .map(|bytes| u64::from_le_bytes(bytes.try_into().expect("eight bytes")))
    };
    if word(0) != Some(vtable as u64) || word(mission::TASK_FORCE) != Some(task as u64) {
        return Err("naval mission identity mismatch");
    }
    Ok(())
}

/// Reads an already-authenticated task force's embedded CNavalMission and link fields.
/// The mission is authenticated ([`authenticate_mission`]); a bombardment region must be the
/// registered CStrategicRegion at its id. Null pointers stay zero.
pub(crate) fn read_counter_inputs(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    task: usize,
    id: u64,
) -> Result<CounterRow, &'static str> {
    use crate::{cnaval_mission as mission, cstrategic_region};
    let at = |offset| {
        task.checked_add(offset)
            .ok_or("task force address overflow")
    };
    let root = at(NAVAL_MISSION)?;
    let header = read::<[u8; 16]>(memory, root).ok_or("naval mission identity unreadable")?;
    authenticate_mission(&header, base, task)?;
    let mut status = [0; 8];
    let mut region = [0; 8];
    let mut links = [0; 16];
    let mut detached = [0; 4];
    let mut supply = [0; 8];
    let mut headquarters = [0; 4];
    if memory.copy_scattered(
        [
            root + mission::IS_IN_REGIONS,
            root + mission::BOMBARDMENT_REGION,
            at(REPAIR_PARENT)?,
            at(REPAIR_LAST_MISSION)?,
            at(cunit::RECEIVED_SUPPLY_RATIO)?,
            at(ATTACHED_HEADQUARTERS + 0xc)?,
        ],
        &mut [
            &mut status,
            &mut region,
            &mut links,
            &mut detached,
            &mut supply,
            &mut headquarters,
        ],
    ) != 6
    {
        return Err("naval counter fields unreadable");
    }
    let regions = Array::read(memory, root + mission::ACCESSIBLE_REGIONS, 8)
        .ok_or("naval mission accessible regions invalid")?;
    let region =
        usize::try_from(u64::from_le_bytes(region)).map_err(|_| "naval region pointer invalid")?;
    let bombardment_region = if region == 0 {
        0
    } else {
        let expected = base
            .checked_add(cstrategic_region::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(memory, region) != Some(expected as u64) {
            return Err("bombardment region vtable mismatch");
        }
        let region_id = region
            .checked_add(cstrategic_region::ID)
            .and_then(|p| read::<u32>(memory, p))
            .ok_or("bombardment region id unreadable")?;
        let registered = game_state
            .checked_add(crate::cgame_state::STRATEGIC_REGIONS)
            .and_then(|p| Array::read(memory, p, 8))
            .and_then(|regions| regions.element(region_id, 8))
            .and_then(|p| read::<u64>(memory, p));
        if region_id == 0 || registered != Some(region as u64) {
            return Err("bombardment region differs from registered slot");
        }
        region_id
    };
    Ok(CounterRow {
        id,
        is_in_regions: status[0],
        running_away: status[1],
        engagement_rule: i32::from_le_bytes(status[4..8].try_into().unwrap()),
        mission_region_count: regions.count as i32,
        bombardment_region,
        repair_parent: u64::from_le_bytes(links[..8].try_into().unwrap()),
        detached_activity: i32::from_le_bytes(links[8..12].try_into().unwrap()),
        repair_last_mission: i32::from_le_bytes(detached),
        received_supply_ratio: i64::from_le_bytes(supply),
        attached_headquarters: i32::from_le_bytes(headquarters),
        name: None,
        supply: None,
        supply_origins_start: 0,
        supply_failure: None,
        port_mission: None,
    })
}

/// One task force's movement target, first-combat role and routes, the routes
/// as ranges of the caller's arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapMotionRow {
    pub id: u64,
    pub target: u32,
    /// [`cunit::first_combat_side`] of the first combat reference: 0 without a
    /// combat, 1 when the logical owner is on the attacking side, else 2.
    pub combat_role: u8,
    pub ordinary_start: usize,
    pub ordinary_len: usize,
    pub alternate_len: usize,
    pub movement_numerator: Option<i64>,
    pub retreat: Option<u8>,
    pub withdrawal: Option<u8>,
    pub naval_transfer: Option<u8>,
    /// First raw combat and first reached native naval combat, full references.
    pub combat_links: Option<[u64; 2]>,
    /// Current-hour pure counter-search inputs; absent outside its native demand
    /// or when the caller did not supply an authenticated sample hour.
    pub fresh: Option<FreshEtaInputs>,
    /// Existing-prefix logical owner; no second native owner read.
    pub logical_owner: u32,
    /// Transient same-pass roster-owner loan for shared naval/theatre operands.
    /// The collector clears it before the Ordinary acquisition returns.
    pub fresh_owner: Option<Result<ccountry::Country, &'static str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapMotionWalk {
    pub outcome: Outcome,
    pub paths_len: usize,
    pub required_paths: usize,
}

#[derive(Clone, Copy)]
enum MissionRiskDemand {
    Observed(Result<bool, &'static str>),
    OriginalCounter(Result<u8, &'static str>),
    OriginalCounterContributor(Result<u8, &'static str>, bool),
}

impl MissionRiskDemand {
    fn resolve(
        self,
        memory: &impl Memory,
        pointer: usize,
        count: &mut Option<Result<i32, &'static str>>,
    ) -> Result<bool, &'static str> {
        match self {
            Self::Observed(demand) => demand,
            Self::OriginalCounterContributor(in_regions, _) => {
                Self::OriginalCounter(in_regions).resolve(memory, pointer, count)
            }
            Self::OriginalCounter(in_regions) => {
                if in_regions? == 0 {
                    return Ok(false);
                }
                // The old motion prefix may have stopped before this header.
                // A failed original header is retained, never read as a word
                // again to hide its failure. Signed negative is native idle.
                let count = *count.get_or_insert_with(|| {
                    pointer
                        .checked_add(cunit::COMBAT_REFERENCE_COUNT)
                        .and_then(|address| read::<i32>(memory, address))
                        .ok_or("task-force risk combat count unreadable")
                });
                count.map(|count| count <= 0)
            }
        }
    }

    fn contributor(self) -> bool {
        matches!(self, Self::OriginalCounterContributor(_, true))
    }
}

/// Collects [`MapMotionRow`]s for the task forces the fleet walk authenticates,
/// through [`cunit::read_map_motion`], the reader railway guns share. A full row
/// buffer or route arena truncates the table; a bad read makes it unavailable.
/// Nothing of the map entity, the camera or a selection is read.
pub struct MapMotionCollector<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    game_state: usize,
    provinces: Array,
    rows: Buffer<'a, MapMotionRow>,
    paths: Buffer<'a, u32>,
    combats: &'a mut cunit::FirstCombatMemo,
    combat_registry: crate::creference_registry::HeaderResolver,
    len: usize,
    paths_len: usize,
    required_paths: usize,
    missing: u32,
    error: Option<&'static str>,
    fresh_hour: Option<i32>,
    fresh: fresh_eta::Reader<'a, M>,
}

impl<'a, M: Memory> MapMotionCollector<'a, M> {
    /// `combats` is the capture's first-combat memo, cleared by the caller for
    /// each capture and reserved off the game thread.
    pub fn new(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: &'a mut [MapMotionRow],
        paths: &'a mut [u32],
        combats: &'a mut cunit::FirstCombatMemo,
    ) -> Self {
        Self::with_buffers(
            memory,
            base,
            game_state,
            Buffer::Fixed(rows),
            Buffer::Fixed(paths),
            combats,
        )
    }

    /// Worker motion capture retains serialized unit routes. Each original
    /// header grows its arena before copying.
    pub fn new_growing(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: &'a mut Vec<MapMotionRow>,
        paths: &'a mut Vec<u32>,
        combats: &'a mut cunit::FirstCombatMemo,
    ) -> Self {
        Self::with_buffers(
            memory,
            base,
            game_state,
            Buffer::Growing(rows),
            Buffer::Growing(paths),
            combats,
        )
    }

    fn with_buffers(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: Buffer<'a, MapMotionRow>,
        paths: Buffer<'a, u32>,
        combats: &'a mut cunit::FirstCombatMemo,
    ) -> Self {
        let provinces = game_state
            .checked_add(crate::cgame_state::PROVINCES)
            .and_then(|at| Array::read(memory, at, 8))
            .filter(|a| a.count >= 2);
        Self {
            memory,
            base,
            game_state,
            provinces: provinces.unwrap_or(Array { data: 0, count: 0 }),
            rows,
            paths,
            combats,
            combat_registry: crate::creference_registry::HeaderResolver::default(),
            len: 0,
            paths_len: 0,
            required_paths: 0,
            missing: 0,
            error: provinces
                .is_none()
                .then_some("task-force motion province roster invalid"),
            fresh_hour: None,
            fresh: fresh_eta::Reader::new(memory, base, game_state),
        }
    }

    /// Supplies this callback's simulation hour. The legacy constructor has no
    /// clock and therefore cannot claim a fresh ETA packet.
    pub fn with_fresh_hour(mut self, hour: i32) -> Self {
        self.fresh_hour = Some(hour);
        self
    }

    /// Attachment-owned fixed-key verification, checked once before dispatch.
    /// A refused/unverified guard withholds only modifier-derived ETA/range;
    /// native stats, simulation bytes and the owner loan remain independent.
    pub fn with_fresh_modifier_guard(mut self, guard: Result<(), &'static str>) -> Self {
        self.fresh = self.fresh.with_modifier_guard(guard);
        self
    }

    /// Bind an off-thread prepared sink to the existing original SHIPS walk.
    /// Its immediate pointer loans retire before this owner callback returns.
    pub fn with_risk_ship_observer(mut self, observer: &'a mut dyn OriginalRiskShips) -> Self {
        self.fresh = self.fresh.with_risk_ship_observer(observer);
        self
    }

    /// `pointer` and `id` are a task force the fleet walk has authenticated.
    pub fn visit(&mut self, pointer: usize, id: u64) {
        if self.error.is_none()
            && let Err(reason) = self.read(pointer, id, None, &mut None, &mut None)
        {
            self.error = Some(reason);
        }
    }

    /// Borrows risk for an authenticated receiver outside the old motion
    /// census, using this same current-pass observer and its failure memos.
    /// The loan pairs the original primary pointer with the full handle from
    /// `counter_identity`, after its same-world class-1 and registry checks.
    /// The caller retains that full handle for the packet join and deduplicates
    /// receiver acquisition while preserving ledger occurrences separately. Null ledger
    /// or backlink skips never call this method. Do not manufacture a successful
    /// loan or retry a failed identity/source observation.
    /// This method neither reprobes the loan nor touches motion rows, routes,
    /// capacity accounting or the old census outcome. No pointer is retained.
    pub fn borrow_mission_risk(
        &mut self,
        receiver: Result<(usize, u64), &'static str>,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
    ) -> Result<MissionRiskInputs, &'static str> {
        self.borrow_mission_contribution(receiver, demand, false, guard)
    }

    pub fn borrow_mission_contribution(
        &mut self,
        receiver: Result<(usize, u64), &'static str>,
        demand: Result<bool, &'static str>,
        contributor: bool,
        guard: Result<(), &'static str>,
    ) -> Result<MissionRiskInputs, &'static str> {
        let (pointer, handle) = receiver?;
        self.fresh.set_receiver(handle);
        let hour = self
            .fresh_hour
            .ok_or("fresh task-force risk sample hour unavailable")?;
        Ok(self
            .fresh
            .capture_mission_risk_union(pointer, hour, demand, guard, contributor))
    }

    /// Visits this authenticated receiver once for motion and an independent
    /// original same-pass mission-risk demand. Use instead of `visit`, never
    /// after it. The caller owns nullable-ledger/backlink and original +30 /
    /// signed-combat-count admission; failures must not be retried or guessed.
    /// `guard` authenticates the risk keys and speed/weather keys.
    /// Motion/path/row-capacity failure is retained by the normal census while
    /// a valid risk-only observation remains available. No native pointer is
    /// retained. Missing current-hour authentication is an outer refusal.
    pub fn visit_with_mission_risk(
        &mut self,
        pointer: usize,
        id: u64,
        demand: Result<bool, &'static str>,
        guard: Result<(), &'static str>,
    ) -> Result<MissionRiskInputs, &'static str> {
        self.fresh.set_receiver(id);
        if self.error.is_none() {
            match self.read(
                pointer,
                id,
                Some((MissionRiskDemand::Observed(demand), guard)),
                &mut None,
                &mut None,
            ) {
                Ok(Some(packet)) => return Ok(packet),
                Ok(None) => {}
                Err(reason) => self.error = Some(reason),
            }
        }
        let hour = self
            .fresh_hour
            .ok_or("fresh task-force risk sample hour unavailable")?;
        Ok(self
            .fresh
            .capture_mission_risk(pointer, hour, demand, guard))
    }

    /// The ordinary visitor's risk demand from its original counter +30 fact
    /// and signed +1B4 observation. Its combat count shares the old motion
    /// header attempt even when vector validation, paths or capacity refuses
    /// motion. A stopped prefix may read the independently reached word once;
    /// a failed original header cannot be retried. Use instead of either visit.
    pub fn visit_with_original_counter_risk(
        &mut self,
        pointer: usize,
        id: u64,
        in_regions: Result<u8, &'static str>,
        guard: Result<(), &'static str>,
    ) -> Result<MissionRiskInputs, &'static str> {
        self.visit_with_original_counter_risk_observed(
            pointer, id, in_regions, guard, &mut None, &mut None,
        )
    }

    /// Also loans original signed count and logical-owner attempts for the
    /// receiver packet. None means the old prefix/native risk demand did not
    /// reach the count, or the old motion prefix never attempted the owner.
    pub fn visit_with_original_counter_risk_observed(
        &mut self,
        pointer: usize,
        id: u64,
        in_regions: Result<u8, &'static str>,
        guard: Result<(), &'static str>,
        count: &mut Option<Result<i32, &'static str>>,
        logical_owner: &mut Option<Result<u32, &'static str>>,
    ) -> Result<MissionRiskInputs, &'static str> {
        self.visit_with_original_counter_contribution_observed(
            pointer,
            id,
            in_regions,
            false,
            guard,
            count,
            logical_owner,
        )
    }

    /// Same-pass contribution union for an original authenticated region-ledger
    /// receiver. The contributor must come from that ledger. Its graph is
    /// independently demanded without a combat gate; own mission facts retain
    /// their original failures without withholding the contribution.
    #[allow(clippy::too_many_arguments)]
    pub fn visit_with_original_counter_contribution_observed(
        &mut self,
        pointer: usize,
        id: u64,
        in_regions: Result<u8, &'static str>,
        contributor: bool,
        guard: Result<(), &'static str>,
        count: &mut Option<Result<i32, &'static str>>,
        logical_owner: &mut Option<Result<u32, &'static str>>,
    ) -> Result<MissionRiskInputs, &'static str> {
        self.fresh.set_receiver(id);
        let demand = MissionRiskDemand::OriginalCounterContributor(in_regions, contributor);
        *count = None;
        *logical_owner = None;
        if self.error.is_none() {
            match self.read(pointer, id, Some((demand, guard)), count, logical_owner) {
                Ok(Some(packet)) => return Ok(packet),
                Ok(None) => {}
                Err(reason) => self.error = Some(reason),
            }
        }
        let hour = self
            .fresh_hour
            .ok_or("fresh task-force risk sample hour unavailable")?;
        let contributor = demand.contributor();
        let demand = demand.resolve(self.memory, pointer, count);
        Ok(self
            .fresh
            .capture_mission_risk_union(pointer, hour, demand, guard, contributor))
    }

    fn read(
        &mut self,
        pointer: usize,
        id: u64,
        risk: Option<(MissionRiskDemand, Result<(), &'static str>)>,
        count: &mut Option<Result<i32, &'static str>>,
        logical_owner: &mut Option<Result<u32, &'static str>>,
    ) -> Result<Option<MissionRiskInputs>, &'static str> {
        if matches!(self.rows, Buffer::Growing(_)) {
            let required = self
                .len
                .checked_add(1)
                .ok_or("task-force motion sizing overflow")?;
            self.rows.ensure(required)?;
            self.combats
                .reserve(required)
                .map_err(|_| "task-force combat sizing allocation failed")?;
        }
        self.fresh.set_receiver(id);
        let owner = pointer
            .checked_add(cunit::LOGICAL_OWNER)
            .and_then(|at| read::<u32>(self.memory, at))
            .ok_or("task-force logical owner unreadable");
        *logical_owner = Some(owner);
        let owner = owner?;
        let motion = cunit::read_map_motion_with_combat_count(
            self.memory,
            self.base,
            self.game_state,
            self.provinces,
            pointer,
            owner,
            self.combats,
            count,
        )?;
        let needed = motion.required_paths();
        self.required_paths = self
            .required_paths
            .checked_add(needed)
            .ok_or("task-force motion route demand overflow")?;
        self.paths.ensure(
            self.paths_len
                .checked_add(needed)
                .ok_or("task-force motion route sizing overflow")?,
        )?;
        if self.len == self.rows.len() || needed > self.paths.len() - self.paths_len {
            self.missing = self
                .missing
                .checked_add(1)
                .ok_or("task-force motion unread count overflow")?;
            return Ok(None);
        }
        motion.copy_paths(
            self.memory,
            &mut self.paths[self.paths_len..self.paths_len + needed],
        )?;
        // The old row and both route copies succeeded before these isolated
        // scalar reads. None cannot affect this census or the next unit. The
        // authenticated visitor pointer is not retained after this call.
        let arrow = cunit::read_movement_arrow_scalars(self.memory, pointer);
        let combat_links = cunit::read_map_motion_combat_links(
            self.memory,
            self.base,
            pointer,
            motion,
            &mut self.combat_registry,
        )
        .ok();
        let fresh = self.fresh_hour.map(|hour| {
            if let Some((demand, guard)) = risk {
                let contributor = demand.contributor();
                let demand = demand.resolve(self.memory, pointer, count);
                let (eta, owner, packet) = self.fresh.capture_with_mission_risk_union(
                    pointer,
                    hour,
                    demand,
                    guard,
                    contributor,
                );
                (eta, owner, Some(packet))
            } else {
                let (eta, owner) = self.fresh.capture(pointer, hour);
                (eta, owner, None)
            }
        });
        self.rows[self.len] = MapMotionRow {
            id,
            target: motion.target,
            combat_role: motion.combat_role,
            ordinary_start: self.paths_len,
            ordinary_len: motion.ordinary.count as usize,
            alternate_len: motion.alternate.count as usize,
            movement_numerator: arrow.numerator,
            retreat: arrow.retreat,
            withdrawal: arrow.withdrawal,
            naval_transfer: arrow.naval_transfer,
            combat_links,
            fresh: fresh.map(|(inputs, _, _)| inputs),
            logical_owner: owner,
            fresh_owner: fresh.map(|(_, owner, _)| owner),
        };
        self.paths_len += needed;
        self.len += 1;
        Ok(fresh.and_then(|(_, _, packet)| packet))
    }

    /// `graph_error` is the fleet walk's own failure, which leaves the census unknown.
    pub fn finish(self, graph_error: Option<&'static str>) -> MapMotionWalk {
        let outcome = if let Some(reason) = graph_error.or(self.error) {
            Outcome::Unavailable { reason }
        } else if self.missing != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.missing,
            }
        } else {
            Outcome::Rows { len: self.len }
        };
        MapMotionWalk {
            outcome,
            paths_len: self.paths_len,
            required_paths: self.required_paths,
        }
    }
}

/// Mission country whose strategic navy registers the mission.
pub const MISSION_OWNER: usize = 0x10;
/// Currently patrolled strategic region; patrol display counts it active
/// when it is the icon region.
pub const MISSION_CURRENT_REGION: usize = 0x70;

/// Registered mission regions come from original accessible-region lists
/// after removing earlier occurrences. These rows retain the country ledger
/// used by naval mission-map presentation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MissionRow {
    pub id: u64,
    /// Raw country reference at `CNavalMission+0x10`.
    pub owner: u32,
    /// Strategic region id at `+0x70`, zero for none.
    pub current_region: u32,
    /// Range of accessible region ids (`+0xC8`, count `+0xD4`) in the caller's arena, in order.
    pub regions_start: usize,
    pub regions_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissionWalk {
    pub outcome: Outcome,
    pub regions_len: usize,
    pub required_regions: usize,
}

/// Collects [`MissionRow`]s for task forces with a current region or an accessible region;
/// a row buffer or region arena that is too short truncates, a bad read makes it unavailable.
pub struct MissionCollector<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    regions: Array,
    rows: Buffer<'a, MissionRow>,
    arena: Buffer<'a, u32>,
    len: usize,
    arena_len: usize,
    required: usize,
    missing: u32,
    error: Option<&'static str>,
}

impl<'a, M: Memory> MissionCollector<'a, M> {
    pub fn new(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: &'a mut [MissionRow],
        arena: &'a mut [u32],
    ) -> Self {
        Self::with_buffers(
            memory,
            base,
            game_state,
            Buffer::Fixed(rows),
            Buffer::Fixed(arena),
        )
    }

    /// Worker mission capture grows regions from the original mission header
    /// before following its pointers.
    pub fn new_growing(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: &'a mut Vec<MissionRow>,
        arena: &'a mut Vec<u32>,
    ) -> Self {
        Self::with_buffers(
            memory,
            base,
            game_state,
            Buffer::Growing(rows),
            Buffer::Growing(arena),
        )
    }

    fn with_buffers(
        memory: &'a M,
        base: usize,
        game_state: usize,
        rows: Buffer<'a, MissionRow>,
        arena: Buffer<'a, u32>,
    ) -> Self {
        let regions = game_state
            .checked_add(crate::cgame_state::STRATEGIC_REGIONS)
            .and_then(|at| Array::read(memory, at, 8))
            .filter(|a| a.count >= 2);
        Self {
            memory,
            base,
            regions: regions.unwrap_or(Array { data: 0, count: 0 }),
            rows,
            arena,
            len: 0,
            arena_len: 0,
            required: 0,
            missing: 0,
            error: regions
                .is_none()
                .then_some("task-force mission region roster invalid"),
        }
    }

    /// `pointer` and `id` are a task force the fleet walk has authenticated; its mission is
    /// authenticated here, whether or not a row is kept for it.
    pub fn visit(&mut self, pointer: usize, id: u64) {
        if self.error.is_none()
            && let Err(reason) = self.read(pointer, id)
        {
            self.error = Some(reason);
        }
    }

    /// The registered id of a `CStrategicRegion*`, which must sit at its own roster slot.
    fn region_id(&self, region: usize) -> Result<u32, &'static str> {
        use crate::cstrategic_region as sr;
        let id = region
            .checked_add(sr::ID)
            .and_then(|at| read::<u32>(self.memory, at))
            .ok_or("mission region id unreadable")?;
        let registered = self
            .regions
            .element(id, 8)
            .and_then(|at| read::<u64>(self.memory, at));
        if id == 0 || registered != Some(region as u64) {
            return Err("mission region differs from its registered slot");
        }
        Ok(id)
    }

    fn read(&mut self, pointer: usize, id: u64) -> Result<(), &'static str> {
        use crate::cnaval_mission as mission;
        let root = pointer
            .checked_add(NAVAL_MISSION)
            .ok_or("task force address overflow")?;
        let field = |offset: usize| root.checked_add(offset).ok_or("mission address overflow");
        // The identity header and the owner after it, in one read.
        let header = read::<[u8; MISSION_OWNER + 4]>(self.memory, root)
            .ok_or("naval mission identity unreadable")?;
        authenticate_mission(&header, self.base, pointer)?;
        let owner = u32::from_le_bytes(
            header[MISSION_OWNER..]
                .try_into()
                .expect("four owner bytes"),
        );
        let current = read::<u64>(self.memory, field(MISSION_CURRENT_REGION)?)
            .and_then(|p| usize::try_from(p).ok())
            .ok_or("mission current region unreadable")?;
        let list = Array::read(self.memory, field(mission::ACCESSIBLE_REGIONS)?, 8)
            .ok_or("mission accessible regions invalid")?;
        if current == 0 && list.count == 0 {
            return Ok(());
        }
        let needed = list.count as usize;
        self.required = self
            .required
            .checked_add(needed)
            .ok_or("mission region demand overflow")?;
        self.rows
            .ensure(self.len.checked_add(1).ok_or("mission sizing overflow")?)?;
        self.arena.ensure(
            self.arena_len
                .checked_add(needed)
                .ok_or("mission region sizing overflow")?,
        )?;
        if self.len == self.rows.len() || needed > self.arena.len() - self.arena_len {
            self.missing = self
                .missing
                .checked_add(1)
                .ok_or("mission unread count overflow")?;
            return Ok(());
        }
        let current_region = if current == 0 {
            0
        } else {
            self.region_id(current)?
        };
        for index in 0..list.count {
            let region = list
                .element(index, 8)
                .and_then(|at| read::<u64>(self.memory, at))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("mission accessible region pointer unreadable or null")?;
            self.arena[self.arena_len + index as usize] = self.region_id(region)?;
        }
        self.rows[self.len] = MissionRow {
            id,
            owner,
            current_region,
            regions_start: self.arena_len,
            regions_len: needed,
        };
        self.arena_len += needed;
        self.len += 1;
        Ok(())
    }

    pub fn finish(self, graph_error: Option<&'static str>) -> MissionWalk {
        let outcome = if let Some(reason) = graph_error.or(self.error) {
            Outcome::Unavailable { reason }
        } else if self.missing != 0 {
            Outcome::Truncated {
                len: self.len,
                unread_rows: self.missing,
            }
        } else {
            Outcome::Rows { len: self.len }
        };
        MissionWalk {
            outcome,
            regions_len: self.arena_len,
            required_regions: self.required,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows, target_env = "msvc", target_arch = "x86_64"))]
mod bench;
