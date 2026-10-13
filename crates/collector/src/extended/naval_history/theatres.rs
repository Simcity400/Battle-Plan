use super::*;
use engine::allocation::{reserve, reserve_exact, resize_default};

#[cfg(test)]
mod oob_entry_tests;
mod order_group_source;
pub(crate) use order_group_source::OrderGroupSource;

/// Fault-attribution masks in closure order: [LandTheatres,
/// LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups,
/// AirTheatreMembers, OrdersGroups, OrdersGroupMembers, ArmyGroupMembers].
/// Shared country roots block the active theatre tables. Land headers, land
/// details and air details have separate fault masks, matching the typed
/// walk's independent outcomes. Shared OOB roots block all three OOB tables.
const THEATRE_MASK: u32 = 0x3F;
const LAND_THEATRE_MASK: u32 = 0x01;
const LAND_DETAILS_MASK: u32 = 0x06;
const LAND_MEMBERS_MASK: u32 = 0x04;
const AIR_DETAILS_MASK: u32 = 0x38;
const OOB_MASK: u32 = 0x1C0;

#[derive(Debug, Default)]
pub(crate) struct NavalJob {
    pub log: RawLog<NAVAL_PACK_N>,
    pub game_state: usize,
    pub(super) growing: bool,
    pub(super) theatres_active: bool,
    pub(super) wing_sources: Vec<ctheatre::WingMembership>,
    pub(super) wing_outcome: Option<Outcome>,
    pub(super) wing_growing: bool,
    pub(super) wing_required: usize,
    /// The copy left before the wing census joined: the callback installs it
    /// after the copy returns ([`Self::install_wings_after`]).
    pub(super) wings_follow: bool,
    /// OOB-only memo of freshly authenticated theatre pointers, in root order.
    /// It never shares observations with theatre/member source families. The
    /// root walk pushes every root's theatre; [`dedup_first_seen`] then keeps
    /// each theatre's first occurrence, so demand counts theatre references.
    pub(super) oob_theatres: Vec<usize>,
    /// Scratch positions for that dedup, with the same capacity as
    /// `oob_theatres`; filling it within that capacity never allocates on the
    /// callback.
    pub(super) oob_theatre_positions: Vec<usize>,
    pub(super) oob_theatres_required: usize,
    /// The callback's authenticated country roster as the theatre copy used
    /// it, for this pass's decode only; see [`copy_naval_theatres`].
    pub(super) roster: engine::ccountry::RosterSnapshot,
    /// This pass's original worker registry census, including its verdict.
    /// A complete prefix supplies OOB copy and decode; an attempted failure
    /// remains terminal. Only standalone fixtures without any attempted census
    /// read the registry themselves. Dropped by [`Self::clear`].
    pub(super) roots: Vec<corders_group::Root>,
    pub(super) roots_len: Option<usize>,
    pub(super) roots_outcome: Option<Outcome>,
    pub(super) roots_required: usize,
    pub(super) capacity_charge: crate::raw_memory::Charge,
    pub(super) positive_span_end: usize,
    pub(super) positive_raw_complete: bool,
    pub(super) reserved_native_count: Option<u32>,
    pub(super) reserved_first_failure: Option<&'static str>,
    pub(super) reserved_capacity_short: bool,
    pub(super) reserved_required_rows: usize,
    pub(super) reserved_raw_demand: Option<(usize, usize)>,
    pub(super) reserved_growth_rows: usize,
}

impl NavalJob {
    /// The copy leaves with the root census, before the wing census joins:
    /// its raw phase ([`copy_naval_pass`]) reads no wing value, so the
    /// callback installs the wing census after it returns
    /// ([`Self::install_wings_after`]).
    pub(crate) fn defer_wings(&mut self) {
        self.wings_follow = true;
    }

    /// This callback's wing census, into a copy that left before it
    /// ([`Self::defer_wings`]): the install ticks, and whether the copy
    /// stands. A copy that walked the theatres without a wing census (the
    /// wing job failed) is refused, as a copy that found none installed is
    /// ([`copy_naval_refresh`]). The copy's own theatre cadence is kept.
    pub(crate) fn install_wings_after(
        &mut self,
        wings: Option<&mut super::super::air::AirWingJob>,
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

    /// Callback-owned values from the one shared complete wing census.
    pub(crate) fn install_wings(&mut self, job: &mut super::super::air::AirWingJob) -> u64 {
        self.theatres_active = job.theatres_active;
        self.wing_growing = job.growing;
        if !job.theatres_active {
            self.wing_sources.clear();
            self.wing_outcome = Some(SKIPPED);
            self.wing_required = 0;
            return 0;
        }
        let begin = qpc();
        if job.growing {
            let outcome = job.naval_membership_outcome;
            std::mem::swap(&mut self.wing_sources, &mut job.naval_memberships);
            self.wing_sources.truncate(outcome.len());
            self.wing_outcome = Some(outcome);
            self.wing_required = job.required_memberships;
            return elapsed(begin);
        }
        self.wing_sources.clear();
        let (source, rows) = job.memberships();
        // Capacity demand is independent of failure: a bad tail leaf in the
        // complete shared census must not hide a smaller consumer's shortage.
        self.wing_required = match source {
            Outcome::Rows { len } => len,
            Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
            Outcome::Unavailable { .. } => rows.len().saturating_add(1),
        };
        let mut failure = None;
        let mut omitted = 0_u32;
        for row in rows {
            if let Some(reason) = row.error {
                failure.get_or_insert(reason);
            } else if self.wing_sources.len() < self.wing_sources.capacity() {
                self.wing_sources.push(row.value);
            } else {
                omitted = omitted.saturating_add(1);
            }
        }
        self.wing_outcome = Some(match source {
            Outcome::Rows { .. } => match failure {
                Some(reason) => Outcome::Unavailable { reason },
                None if omitted != 0 => Outcome::Truncated {
                    len: self.wing_sources.len(),
                    unread_rows: omitted,
                },
                None => Outcome::Rows {
                    len: self.wing_sources.len(),
                },
            },
            Outcome::Truncated { .. } => {
                let len = self.wing_sources.len();
                match u32::try_from(self.wing_required.saturating_sub(len)) {
                    Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
                    Err(_) => Outcome::Unavailable {
                        reason: "air-wing omitted source count overflow",
                    },
                }
            }
            other => other,
        });
        elapsed(begin)
    }

    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(super) fn fixture() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 4 << 20),
            wing_sources: Vec::with_capacity(4096),
            oob_theatres: Vec::with_capacity(2048),
            oob_theatre_positions: Vec::with_capacity(2048),
            roster: engine::ccountry::RosterSnapshot::try_with_capacity(1024)
                .map_err(crate::allocation::io)
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
        self.growing = false;
        self.theatres_active = false;
        self.wing_sources.clear();
        self.wing_outcome = None;
        self.wing_growing = false;
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
        self.reserved_native_count = None;
        self.reserved_first_failure = None;
        self.reserved_capacity_short = false;
        self.reserved_required_rows = 0;
        self.reserved_raw_demand = None;
        self.reserved_growth_rows = 0;
    }

    /// Retains the original census verdict as well as a complete fitted prefix.
    /// This fixed entry preserves allocation-free fixture/callback storage;
    /// a failed or short attempted snapshot never authorizes a registry retry.
    pub(crate) fn capture_roots(&mut self, roots: &[corders_group::Root], outcome: Outcome) -> u64 {
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

    /// Worker-only snapshot of the original, complete registry census.
    pub(crate) fn capture_roots_growing(
        &mut self,
        roots: &[corders_group::Root],
        outcome: Outcome,
    ) -> u64 {
        if let Outcome::Rows { len } = outcome
            && len <= roots.len()
        {
            let additional = len.saturating_sub(self.roots.len());
            if reserve_exact(&mut self.roots, additional).is_err() {
                self.fail_roots("orders registry snapshot allocation failed");
                return 0;
            }
        }
        self.capture_roots(roots, outcome)
    }

    /// A rejected or failed accepted producer supplies no retryable census.
    pub(crate) fn fail_roots(&mut self, reason: &'static str) {
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
            .map_err(crate::allocation::io)
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

    /// Off-thread live country hint. OOB roots and references use their own
    /// original registry demand rather than a chosen country-slot ratio.
    pub fn presize_countries(&mut self) {
        if self
            .roster
            .reserve(crate::country_scale::census_rows(0))
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        self.update_capacity_charge();
    }

    pub(super) fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.wing_sources)
                + vector_bytes(&self.oob_theatres)
                + vector_bytes(&self.oob_theatre_positions)
                + vector_bytes(&self.roots)
                + self.roster.allocated_bytes() as u64,
            ..Default::default()
        });
    }

    pub(crate) fn wing_limit(&self) -> usize {
        self.wing_sources.capacity()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NavalCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; NAVAL_PACK_N],
    pub raw_bytes: [u64; NAVAL_PACK_N],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NavalCopyContext {
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
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn theatre_link(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
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
    memory: &impl engine::Memory,
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

pub(crate) fn copy_naval_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    roster_snapshot: &mut engine::ccountry::RosterSnapshot,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
) {
    let root = engine::ccountry::read_reserved_observation(memory, base, game_state);
    copy_naval_theatres_with_root(
        log,
        roster_snapshot,
        memory,
        base,
        game_state,
        details_active,
        root,
    );
    // Positive capture clears this snapshot even when it cannot fit. Keep the
    // independent selection and result after that operation, including failure.
    roster_snapshot.capture_reserved_observation(base, game_state, root);
}

fn copy_naval_theatres_with_root(
    log: &mut RawLog<NAVAL_PACK_N>,
    roster_snapshot: &mut engine::ccountry::RosterSnapshot,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    details_active: bool,
    reserved: engine::ccountry::ReservedCountryObservation,
) {
    use engine::{carmy_group, ccountry};
    let mask = if details_active {
        THEATRE_MASK
    } else {
        LAND_THEATRE_MASK
    };
    roster_snapshot.clear();
    // Reuse within the pass: the callback authenticated the complete country
    // census this hour before any job ran, and holds the game thread until
    // every job has finished, so authenticating each country again here would
    // read the same values. The copy then takes no census span, and the
    // snapshot hands the same roster to this pass's decode, whose country
    // reader resolves through it exactly as the typed walk does over the
    // callback's memory. Each pass clears the snapshot first; without a
    // roster, or room for it, the census is taken as before.
    if let Some(roster) = memory.authenticated_countries(base, game_state)
        && roster.slots().len() == roster.arrays()[0].count as usize
    {
        if log.worker_growth_enabled()
            && roster_snapshot
                .reserve(roster.slots().len())
                .map_err(crate::allocation::io)
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
    // Country census exactly like the fleet/wing censuses.
    let countries = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRIES),
        16,
        mask,
    ) {
        Some(start) => start,
        None => return,
    };
    let links = match nspan(
        log,
        memory,
        game_state.checked_add(engine::cgame_state::COUNTRY_LINKS),
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
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
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

/// One authenticated country's theatre takes: the land theatre headers every
/// hour, and on the daily slot the land details and air theatres. False when
/// the log overflowed and the pass must stop.
///
/// `roster` is the callback's authenticated country roster when the pass
/// resolves countries through it; a theatre owner's link entry is then taken
/// only where the replay's country reader reads it: for an id whose roster
/// slot is null.
#[allow(clippy::too_many_arguments)]
fn copy_country_theatres(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
    pointer: usize,
    links_array: &Array,
    roster: Option<&[Option<engine::ccountry::Country>]>,
    details_active: bool,
    mask: u32,
) -> bool {
    use engine::{ccountry, ctheater_group};
    // Land theatre headers refresh hourly; the five nested/detail tables
    // refresh only on their shared daily slot.
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
            // Theatre identity (vtable + handle 16), owner and volunteers in
            // one guarded copy, each field its own exact span as the typed
            // walk's gathered record reads them, stopping at the first
            // failure as its per-field reads do; then the owner link resolve
            // and the group vector.
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
                // The roster resolves an id whose slot it authenticated
                // without reading its link.
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
                // Group identity (vtable + handle 16), theatre backlink,
                // name string, priority.
                let _ = nspan(log, memory, Some(group), 16, mask);
                let _ = nspan(
                    log,
                    memory,
                    group.checked_add(ctheater_group::THEATRE),
                    8,
                    mask,
                );
                // Name string header + payload + terminator (theatre arena).
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
                    // Member vtable (8) for the normal/army-group check plus
                    // identity (vtable + handle 16) plus parents. Both
                    // lengths are recorded because the walk requests both
                    // exact spans (exact-match serving, no sub-ranges).
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
    memory: &impl engine::Memory,
    name_address: Option<usize>,
    mask: u32,
) {
    use engine::msvc_string;
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

/// A root's own one-entry order-of-battle spans, as `(offset, len)`, in the
/// order the typed walk reads them: army-group flag, motorization, the five
/// references (theatre, theater group, parent, leader, HQ provider) and the
/// member vector header.
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

/// Logs `pointer`'s [`OOB_ROOT_SPANS`] with one guarded copy, each field its
/// own exact span with its own success or failure, as the per-field takes
/// log them. Those takes are independent, so after a failed field each later
/// one is taken on its own, as before. Returns each field's log start.
fn copy_oob_root_spans(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &impl engine::Memory,
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
        // The failed field is logged as the failure its own take logs.
        next = (completed + 1).min(OOB_ROOT_SPANS.len());
    }
    for (index, (offset, len)) in OOB_ROOT_SPANS.into_iter().enumerate().skip(next) {
        starts[index] = nspan(log, memory, pointer.checked_add(offset), len, OOB_MASK);
    }
    starts
}

/// One authenticated root's order-of-battle spans, from its army-group flag
/// on; the registry census has read and authenticated its identity and
/// country. Returns whether the copy goes on: false when the log overflowed.
///
/// The root's own one-entry spans are logged first, in one guarded copy
/// ([`copy_oob_root_spans`]), then what they reference. The log holds the
/// same exact spans, bytes and failures as when each field was taken just
/// before its targets: the replay serves each read from its exact key, and
/// every key's occurrences keep their order, because within one pass the
/// game thread is held and two occurrences of a key hold the same bytes.
/// Only a log overflow can come at a different field; the copy then stops,
/// as before.
fn copy_oob_root(
    job: &mut NavalJob,
    memory: &impl engine::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    #[cfg(test)]
    if oob_entry_tests::SERIAL.get() {
        return oob_entry_tests::copy_oob_root_serial(job, memory, base, pointer);
    }
    let log = &mut job.log;
    // OOB identity fields: army-group flag, motorization, country
    // (already above), theatre/theater-group/parent/leader/provider
    // references with target authentication for non-null pointers.
    let own = copy_oob_root_spans(log, memory, pointer);
    // Name-only failures retain the unchanged roster prefix; its suffix is None.
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
            // References use a vtable + handle identity, except the
            // HQ provider: it is an embedded member whose host is
            // authenticated below, not a referenced object itself.
            if offset != corders_group::HQ_PROVIDER {
                let identity = nspan(log, memory, Some(target), 16, OOB_MASK);
                if offset == corders_group::THEATRE
                    && identity.is_some_and(|start| {
                        base.checked_add(ctheatre::VTABLE_RVA)
                            .is_some_and(|expected| npool_u64(log, start) == expected as u64)
                            && npool_u64(log, start + 8) as u32 == ctheatre::REFERENCE_TYPE
                    })
                {
                    job.oob_theatres_required = job.oob_theatres.len().saturating_add(1);
                    if job.growing {
                        if reserve(&mut job.oob_theatres, 1).is_err() {
                            return false;
                        }
                        let required = job
                            .oob_theatres_required
                            .saturating_sub(job.oob_theatre_positions.len());
                        if reserve(&mut job.oob_theatre_positions, required).is_err() {
                            return false;
                        }
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
                // Provider is a member pointer: host identity span
                // plus group backlink, like member_unit.
                if let Some(unit) = target.checked_sub(engine::corders_group_member::HOST_OFFSET) {
                    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                }
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(engine::corders_group_member::GROUP),
                    8,
                    OOB_MASK,
                );
            }
        }
    }
    // Member vector with per-member host identity (32-byte header)
    // and group backlink, exactly like `member_unit`: the entries in runs
    // of up to 64 per guarded copy, each member's two spans in one.
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
            // Host identity span (vtable/class/secondary/handle 32 bytes)
            // at the embedded base, plus group backlink at the member.
            if let Some(unit) = member.checked_sub(engine::corders_group_member::HOST_OFFSET) {
                let backlink = member.checked_add(engine::corders_group_member::GROUP);
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
    // The typed walk accepts any nonzero discriminator. The child
    // header starts beyond an ordinary COrdersGroup's allocation,
    // so neither it nor its entries may be read for a zero/failed flag.
    if is_army_group.is_some_and(|flag| flag != 0) {
        let children_header = match nspan(
            log,
            memory,
            pointer.checked_add(engine::carmy_group::MEMBER_DATA),
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
    // Order-membership reverse census for this root: theatre-group
    // pointer plus target identity when non-zero.
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

/// A member visit's worst case: its host identity and backlink spans and
/// their 40 bytes ([`take_oob_member`]).
const OOB_MEMBER_WORST: (usize, usize) = (2, 40);

/// Takes `array`'s 8-byte entries in runs of up to 64 per guarded copy
/// ([`RawLog::take_array_entries`]), each entry its own exact span with its
/// own failure, as taking each on its own logs them, then hands each read
/// nonzero entry to `visit` in array order, with `true` (it may batch its
/// own reads). A run that stops at a failed entry goes on with a new run
/// after it, as the per-entry loop goes on, and is counted in
/// `batch_fallbacks` (`order_of_battle`). False from `visit` ends the walk
/// with false. The log holds the per-entry loop's spans, and each key's
/// occurrences keep their order unless two keys alias: a run's entries are
/// logged before the spans their visits take, and the replay serves each
/// read from its exact key.
///
/// A run is batched only when its whole worst case fits the storage the log
/// already holds: an entry span and eight bytes for each entry, a span for
/// a failed one, and `visit_worst` (spans, bytes) for each entry's visit.
/// Then neither this run nor the per-entry loop can overflow within it.
/// Otherwise the run's entries are taken and visited one by one, with
/// `false`, as the per-entry loop takes them, its visits too.
fn take_oob_entries<M: engine::Memory>(
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
        // An entry address that overflows is no read, as for the
        // per-entry take; every later one overflows too.
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
            // No room for the run's worst case, or no run admitted here:
            // the per-entry loop's takes and visits, request by request.
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
            engine::batch_fallback::record(engine::batch_fallback::Batch::OrderOfBattle);
        }
        for &pointer in &pointers[..completed] {
            if pointer != 0 && !visit(log, pointer, true) {
                return false;
            }
        }
        // The failed entry is logged; the next run starts after it.
        index += completed + usize::from(stopped);
    }
    true
}

/// A member's host identity (32 bytes at `unit`) and group backlink (8
/// bytes at `backlink`) in one guarded copy, each its own exact span with
/// its own failure, as their two takes log them: after a failed host
/// identity the backlink is taken on its own (counted in `batch_fallbacks`
/// as `order_of_battle`). Only a batched run calls it, whose admission
/// already holds its worst case ([`OOB_MEMBER_WORST`]).
fn take_oob_member<M: engine::Memory>(
    log: &mut RawLog<NAVAL_PACK_N>,
    memory: &M,
    unit: usize,
    backlink: Option<usize>,
) {
    if let Some(backlink) = backlink {
        match log.take_scattered(memory, [unit, backlink], [32, 8], [OOB_MASK; 2]) {
            // Both read, or the backlink's failure logged.
            Some((_, 1..)) => return,
            Some(_) => {
                engine::batch_fallback::record(engine::batch_fallback::Batch::OrderOfBattle);
                let _ = nspan(log, memory, Some(backlink), 8, OOB_MASK);
                return;
            }
            // No scatter admitted here: the two takes below.
            None => {}
        }
    }
    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
    let _ = nspan(log, memory, backlink, 8, OOB_MASK);
}

/// The order-of-battle roots from the orders-group registry itself: its
/// header, buckets and each root's identity and country, then the root's own
/// spans. Returns whether the copy goes on to stage the rosters.
fn copy_oob_registry(job: &mut NavalJob, memory: &impl engine::Memory, base: usize) -> bool {
    use engine::{creference_registry as registry, reference_bucket as bucket};
    let log = &mut job.log;
    let table = match nspan(
        log,
        memory,
        base.checked_add(registry::LOW_ROOTS_RVA)
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
    let group_vtable = match base.checked_add(corders_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let army_group_vtable = match base.checked_add(engine::carmy_group::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    let mut first = 0_usize;
    // Registry growth allocates the complete mask/overflow extent.
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

pub(crate) fn copy_naval_oob(job: &mut NavalJob, memory: &impl engine::Memory, base: usize) {
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
        // A standalone fixed fixture may acquire its own first census.
        None => {
            if !copy_oob_registry(job, memory, base) {
                return;
            }
        }
    }
    let log = &mut job.log;
    dedup_first_seen(&mut job.oob_theatres, &mut job.oob_theatre_positions);
    // The engine matches these pointers against the complete authenticated
    // root census. Stage each native roster once, including an unused subtype,
    // and retain failures as observations rather than retrying for later roots.
    // This memo belongs only to this OOB pass: identical addresses in another
    // source family never stand in for this roster's own acquisition.
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
            // Up to 64 entries per guarded copy, each recorded as the exact
            // span its per-entry read would record, failure included: the
            // replay walks the theatres in this same first-seen root order
            // and reads each roster entry by entry, so every read is served
            // from the next recorded span.
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

/// Keeps the first occurrence of each value in its original order, in
/// O(n log n): positions sorted by `(value, position)` keep their first
/// position per value, then return to position order. A sorted insert per
/// value would move O(n) entries each. `positions` needs `values.len()`
/// capacity to stay allocation-free.
pub(crate) fn dedup_first_seen(values: &mut Vec<usize>, positions: &mut Vec<usize>) {
    positions.clear();
    positions.extend(0..values.len());
    positions.sort_unstable_by_key(|&position| (values[position], position));
    positions.dedup_by_key(|position| values[*position]);
    positions.sort_unstable();
    // Kept positions ascend, so each is at or after its destination.
    for (kept, &position) in positions.iter().enumerate() {
        values[kept] = values[position];
    }
    values.truncate(positions.len());
}

/// Optional suffix only: first failure stops acquisition, and mask zero never
/// changes positive fault bits. Full count demand is not a row certificate.
/// Reuse only one exact observation in the completed positive acquisition.
/// An observed failure or ambiguous occurrence never admits a fresh retry.
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
            root.pointer.checked_add(engine::ccountry::THEATRES),
            16,
            0,
        )
        .ok_or("reserved land vector unreadable or capacity insufficient")?;
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&job.log.bytes[header..header + 16]);
        let theatres = Array::from_header(bytes, 8).ok_or("reserved land vector invalid")?;
        job.reserved_native_count = Some(theatres.count);
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
        let expected = base
            .checked_add(ctheatre::VTABLE_RVA)
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
                // Entry payloads already belong to the complete OOB walk.
                // Unknown nonempty rosters are refused by the logical callback.
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
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; NAVAL_PACK_N],
) {
    // This independent raw phase reads no shared wing value: they are
    // installed before it, or after it returns (`NavalJob::defer_wings`).
    job.log.clear();
    if job.growing {
        job.log.enable_worker_growth();
    }
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

pub(crate) fn copy_naval_refresh(
    memory: &impl Memory,
    ctx: NavalCopyContext,
    job: &mut NavalJob,
) -> NavalCopyReport {
    job.theatres_active =
        replay::cadence::should_walk(TableId::LandTheatreGroups, ctx.hour, ctx.force_full);
    // A copy whose wing census follows it is refused when that census is
    // installed (`NavalJob::install_wings_after`).
    if job.theatres_active && job.wing_outcome.is_none() && !job.wings_follow {
        job.clear_reserved();
        job.roster.clear();
        return NavalCopyReport::default();
    }
    let mut ticks = [0u64; NAVAL_PACK_N];
    copy_naval_pass(job, memory, ctx.base, ctx.game_state, &mut ticks);
    NavalCopyReport {
        raw_ok: job.positive_raw_complete,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

/// Acquisition-worker entry: grow from this pass's original observations.
pub(crate) fn copy_naval_refresh_growing(
    memory: &impl Memory,
    ctx: NavalCopyContext,
    job: &mut NavalJob,
) -> NavalCopyReport {
    job.growing = true;
    let report = copy_naval_refresh(memory, ctx, job);
    job.update_capacity_charge();
    report
}

impl Captured {
    /// Finish an accepted theatre/OOB acquisition failure after independent
    /// capture. The separately acquired wing census and other naval families
    /// retain their outcomes; no raw payload or typed retry may follow this.
    pub(crate) fn fail_naval_capture(&mut self, reason: &'static str) {
        self.reserved_land_count = None;
        self.reserved_land_required_rows = 0;
        self.reserved_land_first_failure = Some(reason);
        self.reserved_land_native_count = None;
        self.reserved_land_typed_capacity_short = false;
        self.reserved_land_raw_capacity_short = false;
        self.reserved_land_raw_demand = None;
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

/// A recorded log replayed with the authenticated country roster its copy
/// resolved countries through; every read still goes to the log.
struct RosterReplay<'a, M> {
    inner: &'a M,
    roster: Option<(usize, usize, engine::ccountry::AuthenticatedRoster<'a>)>,
    reserved: Option<(usize, usize, engine::ccountry::ReservedCountryObservation)>,
}

impl<M: Memory> Memory for RosterReplay<'_, M> {
    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<engine::ccountry::Country, &'static str>> {
        self.authenticated_reserved_country_observation(base, game_state)
            .map(engine::ccountry::ReservedCountryObservation::result)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::ReservedCountryObservation> {
        self.reserved
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, root)| root)
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }

    fn immutable_view(
        &self,
        address: usize,
        len: usize,
    ) -> Option<engine::memory::SliceMemory<'_>> {
        self.inner.immutable_view(address, len)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine::ccountry::AuthenticatedRoster<'_>> {
        self.roster
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, roster)| roster)
    }
}

pub(crate) fn decode_naval_payload(
    captured: &mut Captured,
    base: usize,
    job: &NavalJob,
    decode_ticks: &mut [u64; NAVAL_PACK_N],
) {
    captured.reserved_land_count = None;
    captured.reserved_land_required_rows = 0;
    captured.reserved_land_first_failure = None;
    captured.reserved_land_native_count = job.reserved_native_count;
    captured.reserved_land_typed_capacity_short = false;
    captured.reserved_land_raw_capacity_short = job.reserved_capacity_short;
    captured.reserved_land_raw_demand = job.reserved_raw_demand;
    captured.active[4] = true;
    captured.active[5..10].fill(job.theatres_active);
    let recorded = match crate::raw::Recorded::build_range(&job.log, 0..job.positive_span_end) {
        Ok(recorded) => recorded,
        Err(reason) => {
            captured.fail_naval_capture(reason);
            return;
        }
    };
    let started = qpc();
    // Interpretation retains the original census's terminal verdict.
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
            let outcome = corders_group::walk_roots_growing(&recorded, base, &mut walked);
            (&walked[..], outcome)
        }
    };
    // Decoding runs off the game thread. Keep the scratch census complete even
    // when the independent group-row output intentionally truncates.
    if let Outcome::Rows { len } = roots_outcome {
        captured.roster_scratch.required = captured.roster_scratch.required.max(len);
        if captured.roster_scratch.first.len() < len
            && resize_default(&mut captured.roster_scratch.first, len).is_err()
        {
            return;
        }
    }
    // Order sources + OOB through the existing setters (identical rows,
    // outcomes and ticks to the game thread's source callbacks).
    if job.growing {
        captured.set_order_sources_growing(&recorded, base, roots, roots_outcome);
    } else {
        captured.set_order_sources(&recorded, base, roots, roots_outcome);
    }
    // The hourly header walk uses the same country and owner checks as the
    // daily detail walk, but never requires wing membership or nested arenas.
    if !job.theatres_active {
        captured.skip_theatre_details();
    }
    // The copy resolved countries through the callback's roster when it had
    // one, so the replay does too: this pass's snapshot, never another's.
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
            land: storage_buffer(&mut captured.land.rows, job.growing),
            land_groups: storage_buffer(&mut captured.land_groups.rows, job.growing),
            land_members: storage_buffer(&mut captured.land_members.rows, job.growing),
            air: storage_buffer(&mut captured.air.rows, job.growing),
            air_groups: storage_buffer(&mut captured.air_groups.rows, job.growing),
            air_members: storage_buffer(&mut captured.air_members.rows, job.growing),
            names: storage_buffer(&mut captured.theatre_names.first, job.growing),
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
    // Finish all positive byte consumers before recycling the one index for
    // the independently acquired optional suffix. Never share address cursors.
    drop(recorded);
    // Replay the logical failure order even when a later staged header failed:
    // its callback must first retain an earlier OOB refusal. A staging refusal
    // can veto a successful whole batch, but can never authorize publication.
    match crate::raw::Recorded::build_range(&job.log, job.positive_span_end..job.log.spans.len()) {
        Ok(recorded) => {
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
                job.growing,
            );
        }
        Err(reason) => {
            captured.reserved_land_first_failure = Some(reason);
        }
    }
    // Keep observed raw demand independent from logical certificate refusal.
    // Only the callback's checked representable demand may schedule growth.
    captured.reserved_land_native_count = job.reserved_native_count;
    captured.reserved_land_required_rows = job.reserved_growth_rows;
    captured.reserved_land_raw_capacity_short = job.reserved_capacity_short;
    captured.reserved_land_raw_demand = job.reserved_raw_demand;
    if !job.theatres_active {
        captured.wing_sources.outcome = SKIPPED;
        captured.wings_finished = true;
        decode_ticks[0] += qpc().saturating_sub(started).max(0) as u64;
        captured.raw_mode = false;
        return;
    }
    // Preserve the complete authoritative reverse census without attempting
    // an unrelated full wing decode from an incomplete mirror.
    let source_len = job.wing_sources.len();
    if job.wing_growing {
        crate::table_capture::grow_vec(&mut captured.wing_sources.rows, source_len);
    }
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
