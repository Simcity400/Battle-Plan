use super::*;
use collector_air::extended::roster_replay::RosterReplay;
use engine_allocation::{reserve, reserve_exact, resize_default};

#[cfg(test)]
mod oob_entry_tests;
mod order_group_source;
pub use order_group_source::OrderGroupSource;

const THEATRE_MASK: u32 = 0x3F;
const LAND_THEATRE_MASK: u32 = 0x01;
const LAND_DETAILS_MASK: u32 = 0x06;
const LAND_MEMBERS_MASK: u32 = 0x04;
const AIR_DETAILS_MASK: u32 = 0x38;
const OOB_MASK: u32 = 0x1C0;

#[derive(Debug, Default)]
pub struct NavalJob {
    pub log: RawLog<NAVAL_PACK_N>,
    pub(super) reserved_index: collector_raw::raw::RecordedCache,
    pub game_state: usize,
    pub(super) theatres_active: bool,
    pub(super) wing_sources: Vec<ctheatre::WingMembership>,
    pub(super) wing_outcome: Option<Outcome>,
    pub(super) wing_required: usize,
    pub(super) wings_follow: bool,
    pub(super) oob_theatres: Vec<usize>,
    pub(super) oob_theatre_positions: Vec<usize>,
    pub(super) oob_theatres_required: usize,
    pub(super) roster: engine_native::ccountry::RosterSnapshot,
    pub(super) roots: Vec<corders_group::Root>,
    pub(super) roots_len: Option<usize>,
    pub(super) roots_outcome: Option<Outcome>,
    pub(super) roots_required: usize,
    pub(super) capacity_charge: collector_primitives::raw_memory::Charge,
    pub(super) positive_span_end: usize,
    pub(super) positive_raw_complete: bool,
    pub(super) reserved_first_failure: Option<&'static str>,
    pub(super) reserved_capacity_short: bool,
    pub(super) reserved_required_rows: usize,
    pub(super) reserved_raw_demand: Option<(usize, usize)>,
    pub(super) reserved_growth_rows: usize,
}

impl NavalJob {
    pub fn defer_wings(&mut self) {
        self.wings_follow = true;
    }

    pub fn install_wings_after(
        &mut self,
        wings: Option<&mut collector_air::extended::air::AirWingJob>,
    ) -> (u64, bool) {
        self.wings_follow = false;
        let active = self.theatres_active;
        let ticks = match wings {
            Some(wings) => self.install_wings(wings),
            None if active => {
                self.clear_reserved();
                self.roster.clear();
                self.update_capacity_charge();
                return (0, false);
            }
            None => 0,
        };
        self.theatres_active = active;
        self.update_capacity_charge();
        (ticks, true)
    }

    pub fn install_wings(&mut self, job: &mut collector_air::extended::air::AirWingJob) -> u64 {
        self.theatres_active = job.theatres_active;
        if !job.theatres_active {
            self.wing_sources.clear();
            self.wing_outcome = Some(SKIPPED);
            self.wing_required = 0;
            return 0;
        }
        let begin = qpc();
        let outcome = job.naval_membership_outcome;
        std::mem::swap(&mut self.wing_sources, &mut job.naval_memberships);
        self.wing_sources.truncate(outcome.len());
        self.wing_outcome = Some(outcome);
        self.wing_required = job.required_memberships;
        elapsed(begin)
    }

    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn fixture() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 4 << 20),
            wing_sources: Vec::with_capacity(4096),
            oob_theatres: Vec::with_capacity(2048),
            oob_theatre_positions: Vec::with_capacity(2048),
            roster: engine_native::ccountry::RosterSnapshot::try_with_capacity(1024)
                .map_err(collector_memory::terminal::io)
                .unwrap_or_default(),
            roots: Vec::with_capacity(2048),
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.theatres_active = false;
        self.wing_sources.clear();
        self.wing_outcome = None;
        self.wing_required = 0;
        self.wings_follow = false;
        self.oob_theatres.clear();
        self.oob_theatre_positions.clear();
        self.oob_theatres_required = 0;
        self.roster.clear();
        self.roots.clear();
        self.roots_len = None;
        self.roots_outcome = None;
        self.roots_required = 0;
        self.clear_reserved();
    }

    fn clear_reserved(&mut self) {
        self.positive_span_end = 0;
        self.positive_raw_complete = false;
        self.reserved_first_failure = None;
        self.reserved_capacity_short = false;
        self.reserved_required_rows = 0;
        self.reserved_raw_demand = None;
        self.reserved_growth_rows = 0;
    }

    pub fn capture_roots(&mut self, roots: &[corders_group::Root], outcome: Outcome) -> u64 {
        if let Outcome::Rows { len } = outcome
            && len <= roots.len()
        {
            let additional = len.saturating_sub(self.roots.len());
            if reserve_exact(&mut self.roots, additional).is_err() {
                self.fail_roots("orders registry snapshot allocation failed");
                return 0;
            }
        }
        let begin = qpc();
        self.roots.clear();
        self.roots_len = None;
        self.roots_outcome = Some(outcome);
        let required = match outcome {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => 0,
        };
        self.roots_required = self.roots_required.max(required);
        if let Outcome::Rows { len } = outcome
            && let Some(roots) = roots.get(..len)
            && len <= self.roots.capacity()
        {
            self.roots.extend_from_slice(roots);
            self.roots_len = Some(len);
        }
        elapsed(begin)
    }

    pub fn fail_roots(&mut self, reason: &'static str) {
        self.roots.clear();
        self.roots_len = None;
        self.roots_outcome = Some(Outcome::Unavailable { reason });
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
            || self.reserved_capacity_short
            || self.wing_required > self.wing_sources.capacity()
            || self.oob_theatres_required > self.oob_theatres.capacity()
            || self.roster.required() > self.roster.capacity()
            || self.roots_required > self.roots.capacity()
            || matches!(self.wing_outcome, Some(Outcome::Truncated { .. }))
    }

    pub fn grow(&mut self) {
        self.log.grow();
        let required = match self.wing_outcome {
            Some(Outcome::Truncated { len, unread_rows }) => self
                .wing_required
                .max(len.saturating_add(unread_rows as usize)),
            _ => self.wing_required,
        };
        if required > self.wing_sources.capacity() {
            let target = required;
            let additional = target.saturating_sub(self.wing_sources.len());
            if reserve_exact(&mut self.wing_sources, additional).is_err() {
                return;
            }
        }
        if self.oob_theatres_required > self.oob_theatres.capacity() {
            let target = self.oob_theatres_required;
            let additional = target.saturating_sub(self.oob_theatres.len());
            if reserve_exact(&mut self.oob_theatres, additional).is_err() {
                return;
            }
            let additional = target.saturating_sub(self.oob_theatre_positions.len());
            if reserve_exact(&mut self.oob_theatre_positions, additional).is_err() {
                return;
            }
        }
        if self
            .roster
            .reserve(self.roster.required())
            .map_err(collector_memory::terminal::io)
            .is_err()
        {
            return;
        }
        if self.roots_required > self.roots.capacity() {
            let target = self.roots_required;
            let additional = target.saturating_sub(self.roots.len());
            if reserve_exact(&mut self.roots, additional).is_err() {
                return;
            }
        }
        self.update_capacity_charge();
    }

    pub fn presize_countries(&mut self) {
        if self
            .roster
            .reserve(collector_primitives::country_slots::census_rows(0))
            .map_err(collector_memory::terminal::io)
            .is_err()
        {
            return;
        }
        self.update_capacity_charge();
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use collector_primitives::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.wing_sources)
                + vector_bytes(&self.oob_theatres)
                + vector_bytes(&self.oob_theatre_positions)
                + vector_bytes(&self.roots)
                + self.roster.allocated_bytes() as u64,
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NavalCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; NAVAL_PACK_N],
}

#[derive(Debug, Clone, Copy)]
pub struct NavalCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn npool_u64(log: &RawLog<NAVAL_PACK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn npool_u32(log: &RawLog<NAVAL_PACK_N>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

fn nspan(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn theatre_link(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
    mask: u32,
) -> Option<u32> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = nspan(log, memory, links.data.checked_add(offset), 4, mask)?;
    let slot = npool_u32(log, start);
    if slot == 0 || slot >= countries {
        return None;
    }
    Some(slot)
}

fn copy_naval_air_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    theatres: Array,
    links: &Array,
) {
    for ordinal in 0..theatres.count {
        let entry_address = theatres
            .data
            .checked_add((ordinal as usize).saturating_mul(8));
        let theatre =
            match nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK).and_then(|start| {
                usize::try_from(npool_u64(log, start))
                    .ok()
                    .filter(|&p| p != 0)
            }) {
                Some(theatre) => theatre,
                None => continue,
            };
        let _ = nspan(log, memory, Some(theatre), 16, AIR_DETAILS_MASK);
        let owner = nspan(
            log,
            memory,
            theatre.checked_add(cair_theatre::COUNTRY),
            4,
            AIR_DETAILS_MASK,
        );
        if let Some(owner) = owner {
            let owner = npool_u32(log, owner);
            if owner != 0 && owner < links.count {
                let address = (owner as usize)
                    .checked_mul(4)
                    .and_then(|offset| links.data.checked_add(offset));
                let _ = nspan(log, memory, address, 4, AIR_DETAILS_MASK);
            }
        }
        copy_theatre_name(
            log,
            memory,
            theatre.checked_add(cair_theatre::NAME),
            AIR_DETAILS_MASK,
        );
        let groups_header = match nspan(
            log,
            memory,
            theatre.checked_add(cair_theatre::GROUPS),
            16,
            AIR_DETAILS_MASK,
        ) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0; 16];
        header.copy_from_slice(&log.bytes[groups_header..groups_header + 16]);
        let groups = match Array::from_header(header, 8) {
            Some(groups) => groups,
            None => continue,
        };
        for group_ordinal in 0..groups.count {
            let entry_address = groups
                .data
                .checked_add((group_ordinal as usize).saturating_mul(8));
            let group =
                match nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK).and_then(|start| {
                    usize::try_from(npool_u64(log, start))
                        .ok()
                        .filter(|&p| p != 0)
                }) {
                    Some(group) => group,
                    None => continue,
                };
            let _ = nspan(log, memory, Some(group), 16, AIR_DETAILS_MASK);
            let _ = nspan(
                log,
                memory,
                group.checked_add(cair_group::THEATRE_HANDLE),
                8,
                AIR_DETAILS_MASK,
            );
            copy_theatre_name(
                log,
                memory,
                group.checked_add(cair_group::NAME),
                AIR_DETAILS_MASK,
            );
            let wings_header = match nspan(
                log,
                memory,
                group.checked_add(cair_group::WINGS),
                16,
                AIR_DETAILS_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let mut header = [0; 16];
            header.copy_from_slice(&log.bytes[wings_header..wings_header + 16]);
            let wings = match Array::from_header(header, 8) {
                Some(wings) => wings,
                None => continue,
            };
            for wing_ordinal in 0..wings.count {
                let entry_address = wings
                    .data
                    .checked_add((wing_ordinal as usize).saturating_mul(8));
                let _ = nspan(log, memory, entry_address, 8, AIR_DETAILS_MASK);
                if log.overflowed {
                    return;
                }
            }
        }
    }
}

pub fn copy_naval_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    roster_snapshot: &mut engine_native::ccountry::RosterSnapshot,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
) {
    let root = engine_native::ccountry::read_reserved_observation(memory, base, game_state);
    copy_naval_theatres_with_root(
        log,
        roster_snapshot,
        memory,
        base,
        game_state,
        details_active,
        root,
    );
    roster_snapshot.capture_reserved_observation(base, game_state, root);
}

fn copy_naval_theatres_with_root(
    log: &mut RawLog<NAVAL_PACK_N>,
    roster_snapshot: &mut engine_native::ccountry::RosterSnapshot,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
    reserved: engine_native::ccountry::ReservedCountryObservation,
) {
    use engine_native::{carmy_group, ccountry};
    let mask = if details_active {
        THEATRE_MASK
    } else {
        LAND_THEATRE_MASK
    };
    roster_snapshot.clear();
    if let Some(roster) = memory.authenticated_countries(base, game_state)
        && roster.slots().len() == roster.arrays()[0].count as usize
    {
        if roster_snapshot
            .reserve(roster.slots().len())
            .map_err(collector_memory::terminal::io)
            .is_err()
        {
            return;
        }
        if roster_snapshot.capture(roster, base, game_state) {
            let [countries, links] = roster.arrays();
            let slots = roster.slots();
            if links.count == 0 {
                return;
            }
            for slot in 1..countries.count {
                let Some(country) = slots[slot as usize].filter(|c| c.canonical_slot == slot)
                else {
                    continue;
                };
                if !copy_country_theatres(
                    log,
                    memory,
                    country.pointer,
                    &links,
                    Some(slots),
                    details_active,
                    mask,
                ) {
                    return;
                }
            }
            return;
        }
    }
    let countries = match nspan(
        log,
        memory,
        game_state.checked_add(engine_native::cgame_state::COUNTRIES),
        16,
        mask,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(
        log,
        memory,
        game_state.checked_add(engine_native::cgame_state::COUNTRY_LINKS),
        16,
        mask,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries..countries + 16]);
    let countries_array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links..links + 16]);
    let links_array = match Array::from_header(raw, 4) {
        Some(array) => array,
        None => return,
    };
    if countries_array.count < 2 || links_array.count == 0 {
        return;
    }
    let country_vtable = match engine_sites::at(base, ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    for slot in 1..countries_array.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let pointer = match nspan(log, memory, pointer_addr, 8, mask)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(pointer) => pointer,
            None => continue,
        };
        if let Some(alias) = reserved.physical_alias(pointer) {
            match alias {
                Ok(()) => continue,
                Err(_) => return,
            }
        }
        let vtable = match nspan(log, memory, Some(pointer), 8, mask) {
            Some(start) => start,
            None => continue,
        };
        if npool_u64(log, vtable) != country_vtable {
            return;
        }
        let id_start = match nspan(log, memory, pointer.checked_add(ccountry::ID), 4, mask) {
            Some(start) => start,
            None => continue,
        };
        let id = npool_u32(log, id_start);
        let canonical =
            match theatre_link(log, memory, &links_array, countries_array.count, id, mask) {
                Some(canonical) => canonical,
                None => return,
            };
        if theatre_link(log, memory, &links_array, countries_array.count, slot, mask)
            != Some(canonical)
        {
            return;
        }
        if theatre_link(
            log,
            memory,
            &links_array,
            countries_array.count,
            canonical,
            mask,
        ) != Some(canonical)
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries_array.data.checked_add(o));
        let canonical_pointer = match nspan(log, memory, canonical_addr, 8, mask) {
            Some(start) => start,
            None => return,
        };
        if npool_u64(log, canonical_pointer) != pointer as u64 {
            return;
        }
        if !copy_country_theatres(
            log,
            memory,
            pointer,
            &links_array,
            None,
            details_active,
            mask,
        ) {
            return;
        }
    }
    let _ = carmy_group::VTABLE_RVA;
}

#[allow(clippy::too_many_arguments)]
fn copy_country_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    pointer: usize,
    links_array: &Array,
    roster: Option<&[Option<engine_native::ccountry::Country>]>,
    details_active: bool,
    mask: u32,
) -> bool {
    use engine_native::{ccountry, ctheater_group};
    for theatres_off in [ccountry::THEATRES, ccountry::AIR_THEATRES] {
        if !details_active && theatres_off == ccountry::AIR_THEATRES {
            break;
        }
        let header_mask = if theatres_off == ccountry::THEATRES {
            mask
        } else {
            AIR_DETAILS_MASK
        };
        let theatres_header = match nspan(
            log,
            memory,
            pointer.checked_add(theatres_off),
            16,
            header_mask,
        ) {
            Some(start) => start,
            None => continue,
        };
        let mut header = [0u8; 16];
        header.copy_from_slice(&log.bytes[theatres_header..theatres_header + 16]);
        let theatres = match Array::from_header(header, 8) {
            Some(array) => array,
            None => continue,
        };
        if theatres_off == ccountry::AIR_THEATRES {
            copy_naval_air_theatres(log, memory, theatres, links_array);
            continue;
        }
        for ordinal in 0..theatres.count {
            let entry_addr = theatres
                .data
                .checked_add((ordinal as usize).saturating_mul(8));
            let theatre = match nspan(log, memory, entry_addr, 8, header_mask)
                .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
            {
                Some(theatre) => theatre,
                None => continue,
            };
            let [
                (_, identity),
                (owner, owner_len),
                (volunteers, volunteers_len),
            ] = ctheatre::HEADER_SPANS;
            let owner_start = match (theatre.checked_add(owner), theatre.checked_add(volunteers)) {
                (Some(owner), Some(volunteers)) => log
                    .take_scattered(
                        memory,
                        [theatre, owner, volunteers],
                        [identity, owner_len, volunteers_len],
                        [header_mask; 3],
                    )
                    .filter(|&(_, completed)| completed >= 2)
                    .map(|(starts, _)| starts[1]),
                _ => {
                    let _ = nspan(log, memory, Some(theatre), identity, header_mask);
                    nspan(
                        log,
                        memory,
                        theatre.checked_add(owner),
                        owner_len,
                        header_mask,
                    )
                }
            };
            if let Some(owner_start) = owner_start {
                let owner = npool_u32(log, owner_start);
                let resolved = roster
                    .and_then(|slots| slots.get(owner as usize))
                    .is_some_and(Option::is_some);
                if owner != 0 && owner < links_array.count && !resolved {
                    let offset = (owner as usize).checked_mul(4);
                    let _ = nspan(
                        log,
                        memory,
                        offset.and_then(|o| links_array.data.checked_add(o)),
                        4,
                        header_mask,
                    );
                }
            }
            if !details_active {
                continue;
            }
            let mask = LAND_DETAILS_MASK;
            let groups_header = match nspan(
                log,
                memory,
                theatre.checked_add(ctheatre::THEATER_GROUP_DATA),
                16,
                mask,
            ) {
                Some(start) => start,
                None => continue,
            };
            let mut groups_raw = [0u8; 16];
            groups_raw.copy_from_slice(&log.bytes[groups_header..groups_header + 16]);
            let groups = match Array::from_header(groups_raw, 8) {
                Some(array) => array,
                None => continue,
            };
            for group_ordinal in 0..groups.count {
                let group_addr = groups
                    .data
                    .checked_add((group_ordinal as usize).saturating_mul(8));
                let group = match nspan(log, memory, group_addr, 8, mask)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                {
                    Some(group) => group,
                    None => continue,
                };
                let _ = nspan(log, memory, Some(group), 16, mask);
                let _ = nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::THEATRE),
                    8,
                    mask,
                );
                copy_theatre_name(log, memory, group.checked_add(ctheater_group::NAME), mask);
                let _ = nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::PRIORITY),
                    4,
                    mask,
                );
                let members_header = match nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::ORDERS_GROUP_DATA),
                    16,
                    mask,
                ) {
                    Some(start) => start,
                    None => continue,
                };
                let mut members_raw = [0u8; 16];
                members_raw.copy_from_slice(&log.bytes[members_header..members_header + 16]);
                let members = match Array::from_header(members_raw, 8) {
                    Some(array) => array,
                    None => continue,
                };
                for member_ordinal in 0..members.count {
                    let member_addr = members
                        .data
                        .checked_add((member_ordinal as usize).saturating_mul(8));
                    let member = match nspan(log, memory, member_addr, 8, mask)
                        .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                    {
                        Some(member) => member,
                        None => continue,
                    };
                    let _ = nspan(log, memory, Some(member), 8, mask);
                    let _ = nspan(log, memory, Some(member), 16, mask);
                    let _ = nspan(
                        log,
                        memory,
                        member.checked_add(corders_group::THEATER_GROUP),
                        8,
                        mask,
                    );
                    let _ = nspan(
                        log,
                        memory,
                        member.checked_add(corders_group::THEATRE),
                        8,
                        mask,
                    );
                    if log.overflowed {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn copy_theatre_name(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    name_address: Option<usize>,
    mask: u32,
) {
    use engine_native::msvc_string;
    let header_addr = match name_address {
        Some(addr) => addr,
        None => return,
    };
    let header_start = match nspan(log, memory, Some(header_addr), 32, mask) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(header[16..24].try_into().unwrap())) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        header_addr
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(source) => source,
            None => return,
        }
    };
    let Some(end) = source.checked_add(len) else {
        return;
    };
    if nspan(log, memory, Some(end), 1, mask).is_none() {
        return;
    }
    if len != 0 {
        let _ = nspan(log, memory, Some(source), len, mask);
    }
}

const OOB_ROOT_SPANS: [(usize, usize); 8] = [
    (corders_group::IS_ARMY_GROUP, 1),
    (corders_group::MOTORIZATION_LEVEL, 1),
    (corders_group::THEATRE, 8),
    (corders_group::THEATER_GROUP, 8),
    (corders_group::PARENT, 8),
    (corders_group::LEADER, 8),
    (corders_group::HQ_PROVIDER, 8),
    (corders_group::MEMBER_DATA, 16),
];

fn copy_oob_root_spans(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine_native::Memory,
    pointer: usize,
) -> [Option<usize>; OOB_ROOT_SPANS.len()] {
    let mut starts = [None; OOB_ROOT_SPANS.len()];
    let mut next = 0;
    let addresses = OOB_ROOT_SPANS.map(|(offset, _)| pointer.checked_add(offset));
    #[cfg(test)]
    let gather = !naval_test_kit::OOB_PER_FIELD.get();
    #[cfg(not(test))]
    let gather = true;
    if gather
        && addresses.iter().all(Option::is_some)
        && let Some((taken, completed)) = log.take_scattered(
            memory,
            addresses.map(|address| address.unwrap_or(0)),
            OOB_ROOT_SPANS.map(|(_, len)| len),
            [OOB_MASK; OOB_ROOT_SPANS.len()],
        )
    {
        for (start, taken) in starts.iter_mut().zip(&taken[..completed]) {
            *start = Some(*taken);
        }
        next = (completed + 1).min(OOB_ROOT_SPANS.len());
    }
    for (index, (offset, len)) in OOB_ROOT_SPANS.into_iter().enumerate().skip(next) {
        starts[index] = nspan(log, memory, pointer.checked_add(offset), len, OOB_MASK);
    }
    starts
}

fn copy_oob_root(
    job: &mut NavalJob,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    #[cfg(test)]
    if oob_entry_tests::SERIAL.get() {
        return oob_entry_tests::copy_oob_root_serial(job, memory, base, pointer);
    }
    let log = &mut job.log;
    let own = copy_oob_root_spans(log, memory, pointer);
    copy_theatre_name(log, memory, pointer.checked_add(corders_group::NAME), 0);
    let is_army_group = own[0].map(|start| log.bytes[start]);
    for (index, offset) in [
        corders_group::THEATRE,
        corders_group::THEATER_GROUP,
        corders_group::PARENT,
        corders_group::LEADER,
        corders_group::HQ_PROVIDER,
    ]
    .into_iter()
    .enumerate()
    {
        let target = own[index + 2].and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(target) = target.filter(|&p| p != 0) {
            if offset != corders_group::HQ_PROVIDER {
                let identity = nspan(log, memory, Some(target), 16, OOB_MASK);
                if offset == corders_group::THEATRE
                    && identity.is_some_and(|start| {
                        engine_sites::at(base, ctheatre::VTABLE_RVA)
                            .is_some_and(|expected| npool_u64(log, start) == expected as u64)
                            && npool_u64(log, start + 8) as u32 == ctheatre::REFERENCE_TYPE
                    })
                {
                    job.oob_theatres_required = job.oob_theatres.len().saturating_add(1);
                    if reserve(&mut job.oob_theatres, 1).is_err() {
                        return false;
                    }
                    let required = job
                        .oob_theatres_required
                        .saturating_sub(job.oob_theatre_positions.len());
                    if reserve(&mut job.oob_theatre_positions, required).is_err() {
                        return false;
                    }

                    if job.oob_theatres.len() == job.oob_theatres.capacity()
                        || job.oob_theatres.len() == job.oob_theatre_positions.capacity()
                    {
                        log.overflowed = true;
                        return false;
                    }
                    job.oob_theatres.push(target);
                }
            }
            if offset == corders_group::PARENT {
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(corders_group::IS_ARMY_GROUP),
                    1,
                    OOB_MASK,
                );
            }
            if offset == corders_group::HQ_PROVIDER {
                if let Some(unit) =
                    target.checked_sub(engine_native::corders_group_member::HOST_OFFSET)
                {
                    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                }
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(engine_native::corders_group_member::GROUP),
                    8,
                    OOB_MASK,
                );
            }
        }
    }
    let Some(members_header) = own[7] else {
        return true;
    };
    let mut members_raw = [0u8; 16];
    members_raw.copy_from_slice(&log.bytes[members_header..members_header + 16]);
    let members = match Array::from_header(members_raw, 8) {
        Some(array) => array,
        None => return true,
    };
    let walked = take_oob_entries(
        log,
        memory,
        &members,
        OOB_MEMBER_WORST,
        |log, member, batched| {
            if let Some(unit) = member.checked_sub(engine_native::corders_group_member::HOST_OFFSET)
            {
                let backlink = member.checked_add(engine_native::corders_group_member::GROUP);
                if batched {
                    take_oob_member(log, memory, unit, backlink);
                } else {
                    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                    let _ = nspan(log, memory, backlink, 8, OOB_MASK);
                }
            }
            !log.overflowed
        },
    );
    if !walked {
        return false;
    }
    if is_army_group.is_some_and(|flag| flag != 0) {
        let children_header = match nspan(
            log,
            memory,
            pointer.checked_add(engine_native::carmy_group::MEMBER_DATA),
            16,
            OOB_MASK,
        ) {
            Some(start) => start,
            None => return true,
        };
        let mut children_raw = [0u8; 16];
        children_raw.copy_from_slice(&log.bytes[children_header..children_header + 16]);
        if let Some(children) = Array::from_header(children_raw, 8) {
            let walked = take_oob_entries(log, memory, &children, (1, 16), |log, child, _| {
                let _ = nspan(log, memory, Some(child), 16, OOB_MASK);
                !log.overflowed
            });
            if !walked {
                return false;
            }
        }
    }
    if job.theatres_active {
        let _reads = read_accounting::table(TableId::LandTheatreMembers);
        let theater_group = nspan(
            log,
            memory,
            pointer.checked_add(corders_group::THEATER_GROUP),
            8,
            LAND_MEMBERS_MASK,
        )
        .and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(theater_group) = theater_group.filter(|&p| p != 0) {
            let _ = nspan(log, memory, Some(theater_group), 16, LAND_MEMBERS_MASK);
        }
    }
    if log.overflowed {
        return false;
    }
    true
}

const OOB_MEMBER_WORST: (usize, usize) = (2, 40);

fn take_oob_entries<M: engine_native::Memory>(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &M,
    array: &Array,
    visit_worst: (usize, usize),
    mut visit: impl FnMut(&mut RawLog<NAVAL_PACK_N>, usize, bool) -> bool,
) -> bool {
    let count = array.count as usize;
    let mut pointers = [0usize; 64];
    let mut index = 0;
    while index < count {
        let run = (count - index).min(pointers.len());
        let Some(address) = array.data.checked_add(index.saturating_mul(8)) else {
            return true;
        };
        let (visit_spans, visit_bytes) = visit_worst;
        let worst_spans = run + 1 + run * visit_spans;
        let worst_bytes = run * (8 + visit_bytes);
        let batched = if log.fits(worst_spans, worst_bytes) {
            log.take_array_entries(memory, address, 8, run, OOB_MASK)
        } else {
            None
        };
        let Some((start, completed)) = batched else {
            for slot in 0..run {
                let pointer = nspan(log, memory, address.checked_add(slot * 8), 8, OOB_MASK)
                    .and_then(|start| usize::try_from(npool_u64(log, start)).ok())
                    .unwrap_or(0);
                if pointer != 0 && !visit(log, pointer, false) {
                    return false;
                }
            }
            index += run;
            continue;
        };
        for (slot, pointer) in pointers[..completed].iter_mut().enumerate() {
            *pointer = usize::try_from(npool_u64(log, start + slot * 8)).unwrap_or(0);
        }
        let stopped = completed < run;
        if stopped {
            engine_native::batch_fallback::record(
                engine_native::batch_fallback::Batch::OrderOfBattle,
            );
        }
        for &pointer in &pointers[..completed] {
            if pointer != 0 && !visit(log, pointer, true) {
                return false;
            }
        }
        index += completed + usize::from(stopped);
    }
    true
}

fn take_oob_member<M: engine_native::Memory>(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &M,
    unit: usize,
    backlink: Option<usize>,
) {
    if let Some(backlink) = backlink {
        match log.take_scattered(memory, [unit, backlink], [32, 8], [OOB_MASK; 2]) {
            Some((_, 1..)) => return,
            Some(_) => {
                engine_native::batch_fallback::record(
                    engine_native::batch_fallback::Batch::OrderOfBattle,
                );
                let _ = nspan(log, memory, Some(backlink), 8, OOB_MASK);
                return;
            }
            None => {}
        }
    }
    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
    let _ = nspan(log, memory, backlink, 8, OOB_MASK);
}

fn copy_oob_registry(job: &mut NavalJob, memory: &impl engine_native::Memory, base: usize) -> bool {
    use engine_native::{creference_registry as registry, reference_bucket as bucket};
    let log = &mut job.log;
    let table = match nspan(
        log,
        memory,
        engine_sites::at(base, registry::LOW_ROOTS_RVA)
            .and_then(|p| p.checked_add(corders_group::REFERENCE_TYPE as usize * 8)),
        8,
        OOB_MASK,
    )
    .and_then(|start| {
        usize::try_from(npool_u64(log, start))
            .ok()
            .filter(|&p| p != 0)
    }) {
        Some(table) => table,
        None => return false,
    };
    let header = match nspan(
        log,
        memory,
        table.checked_add(registry::ENTRIES),
        17,
        OOB_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let mut header_bytes = [0u8; 17];
    header_bytes.copy_from_slice(&log.bytes[header..header + 17]);
    let entries = match usize::try_from(u64::from_le_bytes(header_bytes[0..8].try_into().unwrap()))
        .ok()
        .filter(|&e| e != 0)
    {
        Some(entries) => entries,
        None => return false,
    };
    let mask = u32::from_le_bytes(header_bytes[12..16].try_into().unwrap());
    let distance = header_bytes[16];
    let capacity = match mask.checked_add(1) {
        Some(capacity) => capacity,
        None => return false,
    };
    let terminal = match (capacity as usize).checked_add(distance as usize) {
        Some(terminal) => terminal,
        None => return false,
    };
    let terminal_addr = entries
        .checked_add(terminal * bucket::STRIDE)
        .and_then(|p| p.checked_add(bucket::DISTANCE));
    let _ = nspan(log, memory, terminal_addr, 1, OOB_MASK);
    let group_vtable = match engine_sites::at(base, corders_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let army_group_vtable = match engine_sites::at(base, engine_native::carmy_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let mut first = 0_usize;
    while first < terminal {
        let log = &mut job.log;
        let slots = (terminal - first).min(64);
        let address = match entries.checked_add(first * bucket::STRIDE) {
            Some(address) => address,
            None => break,
        };
        let batch_start = match nspan(log, memory, Some(address), slots * bucket::STRIDE, OOB_MASK)
        {
            Some(start) => start,
            None => break,
        };
        for slot_in_batch in 0..slots {
            let log = &mut job.log;
            let bucket_start = batch_start + slot_in_batch * bucket::STRIDE;
            let distance = log.bytes[bucket_start + bucket::DISTANCE];
            if distance == 0 {
                continue;
            }
            let handle = u64::from_le_bytes(
                log.bytes[bucket_start + bucket::TYPE..bucket_start + bucket::OBJECT]
                    .try_into()
                    .unwrap(),
            );
            if handle as u32 != corders_group::REFERENCE_TYPE {
                continue;
            }
            let pointer = match usize::try_from(u64::from_le_bytes(
                log.bytes[bucket_start + bucket::OBJECT..bucket_start + bucket::STRIDE]
                    .try_into()
                    .unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0)
            {
                Some(pointer) => pointer,
                None => continue,
            };
            let identity = match nspan(log, memory, Some(pointer), 16, OOB_MASK) {
                Some(start) => start,
                None => continue,
            };
            if npool_u64(log, identity) != group_vtable
                && npool_u64(log, identity) != army_group_vtable
            {
                continue;
            }
            if npool_u64(log, identity + 8) != handle {
                continue;
            }
            let country_start = match nspan(
                log,
                memory,
                pointer.checked_add(corders_group::COUNTRY),
                4,
                OOB_MASK,
            ) {
                Some(start) => start,
                None => continue,
            };
            let _country = npool_u32(log, country_start);
            if !copy_oob_root(job, memory, base, pointer) {
                return false;
            }
        }
        first += slots;
    }
    true
}

pub fn copy_naval_oob(job: &mut NavalJob, memory: &impl engine_native::Memory, base: usize) {
    job.oob_theatres.clear();
    job.oob_theatre_positions.clear();
    job.oob_theatres_required = 0;
    match job.roots_outcome {
        Some(Outcome::Rows { len }) if job.roots_len == Some(len) => {
            for index in 0..len {
                let pointer = job.roots[index].pointer();
                if !copy_oob_root(job, memory, base, pointer) {
                    return;
                }
            }
        }
        Some(_) => return,
        None => {
            if !copy_oob_registry(job, memory, base) {
                return;
            }
        }
    }
    let log = &mut job.log;
    dedup_first_seen(&mut job.oob_theatres, &mut job.oob_theatre_positions);
    for &theatre in &job.oob_theatres {
        for offset in [ctheatre::ORDERS_GROUP_DATA, ctheatre::ARMY_GROUP_DATA] {
            let Some(start) = nspan(log, memory, theatre.checked_add(offset), 16, OOB_MASK) else {
                return;
            };
            let mut header = [0; 16];
            header.copy_from_slice(&log.bytes[start..start + 16]);
            let Some(roster) = Array::from_header(header, 8) else {
                return;
            };
            for first in (0..roster.count).step_by(64) {
                let count = (roster.count - first).min(64) as usize;
                let Some(address) = roster.element(first, 8) else {
                    return;
                };
                match log.take_array_entries(memory, address, 8, count, OOB_MASK) {
                    Some((_, completed)) if completed == count => {}
                    _ => return,
                }
            }
            if log.overflowed {
                return;
            }
        }
    }
}

pub fn dedup_first_seen(values: &mut Vec<usize>, positions: &mut Vec<usize>) {
    positions.clear();
    positions.extend(0..values.len());
    positions.sort_unstable_by_key(|&position| (values[position], position));
    positions.dedup_by_key(|position| values[*position]);
    positions.sort_unstable();
    for (kept, &position) in positions.iter().enumerate() {
        values[kept] = values[position];
    }
    values.truncate(positions.len());
}

pub(super) fn prior_roster_header(
    log: &RawLog<NAVAL_PACK_N>,
    end: usize,
    address: usize,
) -> Result<Option<[u8; 16]>, &'static str> {
    let spans = log
        .spans
        .get(..end)
        .ok_or("reserved land prior span range invalid")?;
    let mut observed = None;
    for span in spans
        .iter()
        .filter(|span| span.addr == address && span.len == 16)
    {
        if observed.is_some() {
            return Err("reserved land prior roster header ambiguous");
        }
        if !span.ok {
            return Err("reserved land prior roster header unreadable");
        }
        let start = span.start as usize;
        let end = start
            .checked_add(16)
            .ok_or("reserved land prior byte range overflow")?;
        let source = log
            .bytes
            .get(start..end)
            .ok_or("reserved land prior roster bytes unavailable")?;
        let mut header = [0; 16];
        header.copy_from_slice(source);
        observed = Some(header);
    }
    Ok(observed)
}

fn copy_reserved_land_headers(job: &mut NavalJob, memory: &impl Memory, base: usize) {
    let mut bounded_demand = false;
    let result = (|| {
        let root = job
            .roster
            .reserved(base, job.game_state)
            .ok_or("reserved land root observation absent")??;
        job.log
            .bytes
            .len()
            .checked_add(16)
            .filter(|end| *end <= u32::MAX as usize)
            .ok_or("reserved land raw byte demand overflow")?;
        job.reserved_raw_demand = Some((16, 1));
        bounded_demand = true;
        job.log.reserve(16);
        if job.log.overflowed {
            return Err("reserved land raw capacity insufficient");
        }
        let header = nspan(
            &mut job.log,
            memory,
            root.pointer.checked_add(engine_native::ccountry::THEATRES),
            16,
            0,
        )
        .ok_or("reserved land vector unreadable or capacity insufficient")?;
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&job.log.bytes[header..header + 16]);
        let theatres = Array::from_header(bytes, 8).ok_or("reserved land vector invalid")?;
        job.reserved_required_rows = theatres.count as usize;
        job.reserved_raw_demand = super::reserved_raw_demand(Some(theatres.count));
        bounded_demand = false;
        let remaining = (theatres.count as usize)
            .checked_mul(32)
            .ok_or("reserved land raw byte demand overflow")?;
        job.log
            .bytes
            .len()
            .checked_add(remaining)
            .filter(|end| *end <= u32::MAX as usize)
            .ok_or("reserved land raw byte demand overflow")?;
        (theatres.count as usize)
            .checked_mul(4)
            .and_then(|spans| spans.checked_add(1))
            .ok_or("reserved land raw span demand overflow")?;
        bounded_demand = true;
        job.reserved_growth_rows = theatres.count as usize;
        job.log.reserve(remaining);
        if job.log.overflowed {
            return Err("reserved land raw capacity insufficient");
        }
        let expected = engine_sites::at(base, ctheatre::VTABLE_RVA)
            .ok_or("reserved land module address overflow")? as u64;
        for ordinal in 0..theatres.count {
            let entry = nspan(&mut job.log, memory, theatres.element(ordinal, 8), 8, 0)
                .ok_or("reserved land theatre pointer unreadable or capacity insufficient")?;
            let theatre = usize::try_from(npool_u64(&job.log, entry))
                .ok()
                .filter(|pointer| *pointer != 0)
                .ok_or("reserved land theatre pointer null or overflowing")?;
            let owner_address = theatre
                .checked_add(ctheatre::OWNER)
                .ok_or("reserved land theatre owner address overflow")?;
            let volunteers_address = theatre
                .checked_add(ctheatre::VOLUNTEERS_THEATRE)
                .ok_or("reserved land theatre volunteers address overflow")?;
            let (taken, completed) = job
                .log
                .take_scattered(
                    memory,
                    [theatre, owner_address, volunteers_address],
                    [16, 4, 4],
                    [0; 3],
                )
                .ok_or("reserved land theatre header capacity or copy form refused")?;
            if completed != ctheatre::HEADER_SPANS.len() {
                return Err("reserved land theatre header unreadable");
            }
            let identity = taken[0];
            let owner = taken[1];
            let handle = npool_u64(&job.log, identity + 8);
            if npool_u64(&job.log, identity) != expected
                || handle as u32 != ctheatre::REFERENCE_TYPE
                || handle >> 32 == 0
            {
                return Err("reserved land theatre identity mismatch");
            }
            if npool_u32(&job.log, owner) != 0 {
                return Err("reserved land theatre owner is not zero");
            }
            for offset in [ctheatre::ORDERS_GROUP_DATA, ctheatre::ARMY_GROUP_DATA] {
                let address = theatre
                    .checked_add(offset)
                    .ok_or("reserved land group roster address overflow")?;
                let prior = prior_roster_header(&job.log, job.positive_span_end, address)?;
                let bytes = match prior {
                    Some(bytes) => bytes,
                    None => {
                        bounded_demand = false;
                        let (bytes, spans) = job
                            .reserved_raw_demand
                            .ok_or("reserved land raw demand unavailable")?;
                        let demand = (
                            bytes
                                .checked_add(16)
                                .ok_or("reserved land raw byte demand overflow")?,
                            spans
                                .checked_add(1)
                                .ok_or("reserved land raw span demand overflow")?,
                        );
                        job.log
                            .bytes
                            .len()
                            .checked_add(16)
                            .filter(|end| *end <= u32::MAX as usize)
                            .ok_or("reserved land raw byte demand overflow")?;
                        job.reserved_raw_demand = Some(demand);
                        bounded_demand = true;
                        job.log.reserve(16);
                        if job.log.overflowed {
                            return Err("reserved land raw capacity insufficient");
                        }
                        let start = nspan(&mut job.log, memory, Some(address), 16, 0).ok_or(
                            "reserved land group roster header unreadable or capacity insufficient",
                        )?;
                        let mut bytes = [0; 16];
                        bytes.copy_from_slice(&job.log.bytes[start..start + 16]);
                        bytes
                    }
                };
                Array::from_header(bytes, 8).ok_or("reserved land group roster header invalid")?;
            }
        }
        Ok::<_, &'static str>(())
    })();
    job.reserved_first_failure = result.err();
    job.reserved_capacity_short = bounded_demand && job.log.overflowed;
}

fn copy_naval_pass(
    job: &mut NavalJob,
    memory: &impl engine_native::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; NAVAL_PACK_N],
) {
    job.log.clear();
    job.clear_reserved();
    job.game_state = game_state;
    let details_active = job.theatres_active;
    let started = qpc();
    read_accounting::with_table(TableId::LandTheatres, || {
        copy_naval_theatres(
            &mut job.log,
            &mut job.roster,
            memory,
            base,
            game_state,
            details_active,
        )
    });
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    if job.log.overflowed {
        return;
    }
    let started = qpc();
    read_accounting::with_table(TableId::OrdersGroups, || copy_naval_oob(job, memory, base));
    ticks[6] += qpc().saturating_sub(started).max(0) as u64;
    if job.log.overflowed {
        return;
    }
    job.positive_span_end = job.log.spans.len();
    job.positive_raw_complete = true;
    let started = qpc();
    read_accounting::with_table(TableId::LandTheatres, || {
        copy_reserved_land_headers(job, memory, base)
    });
    ticks[0] += qpc().saturating_sub(started).max(0) as u64;
}

pub fn copy_naval_refresh(
    memory: &impl Memory,
    ctx: NavalCopyContext,
    job: &mut NavalJob,
) -> NavalCopyReport {
    job.theatres_active =
        replay_data::cadence::should_walk(TableId::LandTheatreGroups, ctx.hour, ctx.force_full);
    if job.theatres_active && job.wing_outcome.is_none() && !job.wings_follow {
        job.clear_reserved();
        job.roster.clear();
        return NavalCopyReport::default();
    }
    let mut ticks = [0u64; NAVAL_PACK_N];
    copy_naval_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    job.update_capacity_charge();
    NavalCopyReport {
        raw_ok: job.positive_raw_complete,
        copy_ticks: ticks,
    }
}

impl Captured {
    pub fn fail_naval_capture(&mut self, reason: &'static str) {
        self.reserved_land_count = None;
        self.reserved_land_required_rows = 0;
        macro_rules! reject {
            ($($field:ident = $slot:literal),+ $(,)?) => {
                $({
                    self.$field.outcome = failed(
                        $slot >= 6 || ($slot == 0 && self.active[4]) || self.theatres_active(),
                        reason,
                    );
                })+
            };
        }
        reject!(
            land = 0,
            land_groups = 1,
            land_members = 2,
            air = 3,
            air_groups = 4,
            air_members = 5,
            groups = 6,
            group_members = 7,
            group_children = 8,
        );
        self.theatre_names.reset();
        self.raw_mode = false;
    }
}

pub fn decode_naval_payload(
    captured: &mut Captured,
    base: usize,
    job: &NavalJob,
    decode_ticks: &mut [u64; NAVAL_PACK_N],
) {
    captured.reserved_land_count = None;
    captured.reserved_land_required_rows = 0;
    captured.active[4] = true;
    captured.active[5..10].fill(job.theatres_active);
    let recorded =
        match collector_raw::raw::Recorded::build_range(&job.log, 0..job.positive_span_end) {
            Ok(recorded) => recorded,
            Err(reason) => {
                captured.fail_naval_capture(reason);
                return;
            }
        };
    let started = qpc();
    let mut walked: Vec<corders_group::Root> = Vec::new();
    let (roots, roots_outcome) = match job.roots_outcome {
        Some(Outcome::Rows { len }) if job.roots_len == Some(len) => {
            (&job.roots[..len], Outcome::Rows { len })
        }
        Some(Outcome::Rows { .. }) => (
            &[][..],
            Outcome::Unavailable {
                reason: "orders registry snapshot capacity insufficient",
            },
        ),
        Some(outcome) => (&[][..], outcome),
        None => {
            let outcome = corders_group::walk_roots(&recorded, base, &mut walked);
            (&walked[..], outcome)
        }
    };
    if let Outcome::Rows { len } = roots_outcome {
        captured.roster_scratch.required = captured.roster_scratch.required.max(len);
        if captured.roster_scratch.first.len() < len
            && resize_default(&mut captured.roster_scratch.first, len).is_err()
        {
            return;
        }
    }
    captured.set_order_sources(&recorded, base, roots, roots_outcome);

    if !job.theatres_active {
        captured.skip_theatre_details();
    }
    let theatre_source = RosterReplay {
        inner: &recorded,
        roster: job
            .roster
            .roster(base, job.game_state)
            .map(|roster| (base, job.game_state, roster)),
        reserved: job
            .roster
            .reserved_observation(base, job.game_state)
            .map(|root| (base, job.game_state, root)),
    };
    let first = ctheatre::walk_buffers(
        &theatre_source,
        base,
        job.game_state,
        job.theatres_active,
        ctheatre::Storage {
            land: storage_buffer(&mut captured.land.rows),
            land_groups: storage_buffer(&mut captured.land_groups.rows),
            land_members: storage_buffer(&mut captured.land_members.rows),
            air: storage_buffer(&mut captured.air.rows),
            air_groups: storage_buffer(&mut captured.air_groups.rows),
            air_members: storage_buffer(&mut captured.air_members.rows),
            names: storage_buffer(&mut captured.theatre_names.first),
        },
    );
    captured.land.outcome = bounded(first.land, captured.land.rows.len());
    if job.theatres_active {
        captured.land_groups.outcome = bounded(first.land_groups, captured.land_groups.rows.len());
        captured.land_members.outcome =
            bounded(first.land_members, captured.land_members.rows.len());
        captured.air.outcome = bounded(first.air, captured.air.rows.len());
        captured.air_groups.outcome = bounded(first.air_groups, captured.air_groups.rows.len());
        captured.air_members.outcome = bounded(first.air_members, captured.air_members.rows.len());
    }
    captured.theatre_names.len = first.names_len;
    captured.theatre_names.required = first.names_required;
    drop(recorded);
    if let Ok(recorded) = collector_raw::raw::Recorded::build_range_cached(
        &job.log,
        job.positive_span_end..job.log.spans.len(),
        &job.reserved_index,
    ) {
        let source = RosterReplay {
            inner: &recorded,
            roster: None,
            reserved: job
                .roster
                .reserved_observation(base, job.game_state)
                .map(|root| (base, job.game_state, root)),
        };
        captured.capture_reserved_land_inner(
            &source,
            base,
            source
                .authenticated_reserved_country(base, job.game_state)
                .unwrap_or(Err("reserved land root observation absent")),
            Some((&job.log, job.positive_span_end)),
            job.reserved_first_failure,
        );
    }
    captured.reserved_land_required_rows = job.reserved_growth_rows;
    if !job.theatres_active {
        captured.wing_sources.outcome = SKIPPED;
        captured.wings_finished = true;
        decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
        captured.raw_mode = false;
        return;
    }
    let source_len = job.wing_sources.len();
    collector_primitives::table_capture::grow_vec(&mut captured.wing_sources.rows, source_len);

    let copied = source_len.min(captured.wing_sources.rows.len());
    captured.wing_sources.rows[..copied].copy_from_slice(&job.wing_sources[..copied]);
    captured.wing_sources.outcome = match job.wing_outcome {
        Some(Outcome::Rows { .. } | Outcome::Truncated { .. }) => {
            let missing = match job.wing_outcome {
                Some(Outcome::Truncated { unread_rows, .. }) => unread_rows,
                _ => 0,
            };
            match u32::try_from(source_len - copied)
                .ok()
                .and_then(|omitted| missing.checked_add(omitted))
            {
                Some(0) => Outcome::Rows { len: copied },
                Some(unread_rows) => Outcome::Truncated {
                    len: copied,
                    unread_rows,
                },
                None => Outcome::Unavailable {
                    reason: "air-wing omitted source count overflow",
                },
            }
        }
        Some(outcome) => outcome,
        None => source_unavailable(),
    };
    captured.wings_finished = true;
    decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
    captured.raw_mode = false;
}
