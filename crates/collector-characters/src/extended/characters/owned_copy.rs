use super::*;
use engine_allocation::{SharedBox, owned_vec, reserve, vec};

#[derive(Debug, Default)]
pub struct CharactersJob {
    pub(super) leader_rows: usize,
    pub(super) character_demand: AdvisorDemand,
    pub(super) unit_demand: AdvisorDemand,
    pub part: usize,
    pub log: collector_raw::raw::RawLog<N>,
    pub game_state: usize,
    pub active: [bool; N],
    pub(super) traits: TraitCapture,
    pub(super) advisor_demand: AdvisorDemand,
    pub(super) appended_roles: Vec<AppendedRole>,
    pub(super) appended_required: usize,
    pub(super) appended_overflowed: bool,
    pub(super) appended_charge: collector_primitives::raw_memory::Charge,
    pub(super) skills: [SkillCopy; 32],
    pub(super) portrait_census: Option<SharedBox<ccharacter_manager::PortraitCensus>>,
    pub(super) census_current: bool,
    pub(super) census_matches_plan: bool,
    pub(super) source_bindings: Option<CharacterSourceBindings>,
    pub(super) advisor_roots: collector_raw::raw::RawLog<N>,
    pub(super) advisor_dynamic: Option<Box<CharactersJob>>,
    pub(super) advisor_cache: Option<advisors::Cache>,
    pub(super) advisor_section_complete: bool,
    pub(super) leader_cache: Option<leaders::LeaderCache>,
    pub(super) leader_cached: bool,
    pub(super) leader_charge: collector_primitives::raw_memory::Charge,
    pub(super) carry: carry::Carry,
    pub(super) carry_charge: collector_primitives::raw_memory::Charge,
    pub(super) roster: quiet::QuietRoster,
    pub(super) published: quiet::Published,
    pub(super) unit_carry: leader_carry::UnitCarry,
    pub(super) experience: daily_experience::Store,
    pub(super) daily_training: Option<bool>,
    pub(super) trait_carry: leader_carry::TraitCarry,
    pub(super) census_carry: census::CensusCarry,
    pub(super) census_charge: collector_primitives::raw_memory::Charge,
    pub(super) role_census: Option<SharedBox<census::RoleCensus>>,
    pub(super) role_census_current: bool,
    pub(super) census_historical: usize,
    pub(super) census_generation: u64,
    pub(super) census_capture: bool,
    pub(super) census_visited: Vec<census::Visited>,
    pub(super) census_required: usize,
    pub(super) templates: TemplateCache,
    pub(super) template_charge: collector_primitives::raw_memory::Charge,
}

#[derive(Debug)]
pub(super) struct CharacterSourceBindings {
    pub(super) sources: Vec<TraitVectorCopy>,
    pub(super) overflowed: bool,
    pub(super) ready: bool,
    pub(super) advisor: AdvisorDemand,
    pub(super) advisor_section_complete: bool,
    pub(super) charge: collector_primitives::raw_memory::Charge,
}

impl CharacterSourceBindings {
    pub(super) fn new() -> Self {
        let mut value = Self {
            sources: owned_vec(vec(census::initial_len())),
            overflowed: false,
            ready: false,
            advisor: AdvisorDemand::default(),
            advisor_section_complete: false,
            charge: collector_primitives::raw_memory::Charge::default(),
        };
        value.update_charge();
        value
    }

    pub(super) fn update_charge(&mut self) {
        self.charge.set(collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(&self.sources),
            ..Default::default()
        });
    }

    pub(super) fn begin(&mut self) {
        self.sources.clear();
        self.overflowed = false;
        self.ready = false;
    }

    pub(super) fn remember(&mut self, source: TraitVectorCopy) {
        let capacity = self.sources.capacity();
        if reserve(&mut self.sources, 1).is_err() {
            self.overflowed = true;
            return;
        }
        if self.sources.capacity() != capacity {
            self.update_charge();
        }
        self.sources.push(source);
    }

    pub(super) fn grow(&mut self) {
        if self.overflowed {
            let _ = engine_allocation::reserve(&mut self.sources, 1);
            self.update_charge();
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SkillCopy {
    pub(super) pointer: usize,
    pub(super) bytes: [u8; 21],
}

impl Memory for SkillCopy {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let source = match (address.checked_sub(self.pointer), output.len()) {
            (Some(0), 8) => &self.bytes[..8],
            (Some(cunit_leader_skill::VALID), 1) => &self.bytes[8..9],
            (Some(cunit_leader_skill::LEVEL), 12) => &self.bytes[9..],
            _ => return false,
        };
        output.copy_from_slice(source);
        true
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        self.copy_scattered(addresses, outputs)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct AdvisorDemand {
    pub(super) rows: usize,
    pub(super) bytes: usize,
    pub(super) overflowed: bool,
}

impl AdvisorDemand {
    pub(super) fn add(&mut self, rows: usize, bytes: usize) {
        self.overflowed |=
            self.rows.checked_add(rows).is_none() || self.bytes.checked_add(bytes).is_none();
        self.rows = self.rows.saturating_add(rows);
        self.bytes = self.bytes.saturating_add(bytes);
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum AppendedRole {
    Leader(usize),
    Advisor(usize),
    Unit {
        pointer: usize,
        kind: u8,
        category: i32,
    },
    Visibility {
        leader: usize,
        character: usize,
    },
}

impl CharactersJob {
    pub fn has_storage(&self) -> bool {
        if self.part == 2 && cfg!(feature = "advisor_changes") {
            return self.advisor_cache.is_some();
        }
        self.source_bindings.is_some()
    }

    pub fn vacant(part: usize) -> Self {
        let mut job = Self {
            part,
            ..Self::default()
        };
        if part == 1 && cfg!(feature = "character_changes") {
            job.leader_cache = Some(leaders::LeaderCache::default());
        }
        job
    }
    pub(super) fn source_mask(&self) -> u32 {
        self.active
            .iter()
            .enumerate()
            .fold(0, |mask, (index, &active)| {
                mask | (u32::from(active) << index)
            })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn new() -> Self {
        let mut job = Self::with_storage(0, 1 << 19, 24 << 20, true);
        job.traits = TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20);
        job.source_bindings = Some(super::tests::fixture_bindings());
        job.roster = quiet::QuietRoster::unobserved();
        job
    }

    pub(super) fn with_storage(part: usize, spans: usize, bytes: usize, has_traits: bool) -> Self {
        let mut log = collector_raw::raw::RawLog::new(spans, bytes);
        let trace_capacity = spans.min(1 << 18);
        log.initialize_scattered_read_plan(trace_capacity, trace_capacity, bytes.min(4 << 20));
        let census = (part == 0)
            .then(|| {
                ccharacter_manager::PortraitCensus::try_new(census::initial_len())
                    .map_err(collector_memory::terminal::io)
                    .ok()
            })
            .flatten();
        let role_census = (part == 0).then(|| census::RoleCensus::new(census::initial_len()));
        let census_visited = owned_vec(vec(if part == 0 { 0 } else { census::initial_len() }));
        let mut census_charge = collector_primitives::raw_memory::Charge::default();
        census_charge.set(collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: census
                .as_ref()
                .map_or(0, |census| census.allocated_bytes() as u64)
                + role_census
                    .as_ref()
                    .map_or(0, |census| census.allocated_bytes() as u64)
                + collector_primitives::raw_memory::vector_bytes(&census_visited),
            ..Default::default()
        });
        let appended_roles = owned_vec(vec(demand::initial_roles(part).max(spans / 8)));
        let mut appended_charge = collector_primitives::raw_memory::Charge::default();
        appended_charge.set(collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(&appended_roles),
            ..Default::default()
        });
        Self {
            leader_rows: 0,
            character_demand: AdvisorDemand::default(),
            unit_demand: AdvisorDemand::default(),
            part,
            log,
            game_state: 0,
            active: [true; N],
            traits: if has_traits {
                TraitCapture::new()
            } else {
                TraitCapture::default()
            },
            advisor_demand: AdvisorDemand::default(),
            appended_roles,
            appended_required: 0,
            appended_overflowed: false,
            appended_charge,
            skills: [SkillCopy::default(); 32],
            portrait_census: census.and_then(|census| engine_allocation::shared(census).ok()),
            census_current: false,
            census_matches_plan: false,
            source_bindings: Some(CharacterSourceBindings::new()),
            advisor_roots: collector_raw::raw::RawLog::default(),
            advisor_dynamic: None,
            advisor_cache: None,
            advisor_section_complete: false,
            leader_cache: None,
            leader_cached: false,
            leader_charge: collector_primitives::raw_memory::Charge::default(),
            carry: carry::Carry::default(),
            carry_charge: collector_primitives::raw_memory::Charge::default(),
            roster: quiet::QuietRoster::default(),
            published: quiet::Published::default(),
            unit_carry: leader_carry::UnitCarry::default(),
            experience: daily_experience::Store::default(),
            daily_training: None,
            trait_carry: leader_carry::TraitCarry::default(),
            census_carry: census::CensusCarry::default(),
            census_charge,
            role_census: role_census.and_then(|census| engine_allocation::shared(census).ok()),
            role_census_current: false,
            census_historical: 0,
            census_generation: 0,
            census_capture: false,
            census_visited,
            census_required: 0,
            templates: TemplateCache::default(),
            template_charge: collector_primitives::raw_memory::Charge::default(),
        }
    }

    pub fn new_part(part: usize) -> Self {
        if part == 2 && cfg!(feature = "advisor_changes") {
            return Self {
                part,
                advisor_cache: Some(advisors::Cache::new()),
                ..Default::default()
            };
        }
        assert!(part < PARTS, "invalid character part");
        let mask = PART_MASKS[part];
        let mut job = Self::with_storage(part, 0, 0, mask[4]);
        if part == 1 && cfg!(feature = "character_changes") {
            job.leader_cache = Some(leaders::LeaderCache::default());
        }
        if part == 2 {
            job.advisor_roots = collector_raw::raw::RawLog::new(4, 48);
            job.advisor_dynamic =
                engine_allocation::boxed(Self::with_storage(part, 0, 0, false)).ok();
        }
        job
    }

    pub fn clear(&mut self) {
        self.log.drop_index();
        self.log.clear();
        self.game_state = 0;
        self.active = [false; N];
        self.traits.clear();
        self.advisor_demand = AdvisorDemand::default();
        self.appended_roles.clear();
        self.appended_required = 0;
        self.appended_overflowed = false;
        self.skills.fill(SkillCopy::default());
        self.census_current = false;
        self.role_census_current = false;
        self.census_capture = false;
        self.census_visited.clear();
        self.census_required = 0;
        self.advisor_section_complete = false;
        self.leader_cached = false;
        self.roster.quiet = false;
        self.advisor_roots.drop_index();
        self.advisor_roots.clear();
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.clear();
        }
    }

    pub fn portrait_census(&self) -> Option<SharedBox<ccharacter_manager::PortraitCensus>> {
        self.census_current
            .then(|| self.portrait_census.clone())
            .flatten()
    }

    pub fn role_census(&self) -> Option<SharedBox<census::RoleCensus>> {
        self.role_census_current
            .then(|| self.role_census.clone())
            .flatten()
    }

    pub(super) fn update_census_charge(&mut self) {
        self.census_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: self
                    .portrait_census
                    .as_ref()
                    .map_or(0, |census| census.allocated_bytes() as u64)
                    + self
                        .role_census
                        .as_ref()
                        .map_or(0, |census| census.allocated_bytes() as u64)
                    + collector_primitives::raw_memory::vector_bytes(&self.census_visited),
                ..Default::default()
            });
    }

    pub fn grow_for_next(&self) -> bool {
        self.advisor_cache
            .as_ref()
            .is_some_and(advisors::Cache::needs_growth)
            || self.log.grow_for_next()
            || self.traits.grow_for_next()
            || self
                .portrait_census
                .as_ref()
                .is_some_and(|census| census.needs_growth())
            || self
                .source_bindings
                .as_ref()
                .is_some_and(|plan| plan.overflowed)
            || self
                .advisor_dynamic
                .as_ref()
                .is_some_and(|job| job.grow_for_next())
            || self
                .role_census
                .as_ref()
                .is_some_and(|census| census.needs_growth())
            || self.visits_required(census::live_len()) > self.census_visited.capacity()
            || self.appended_required > self.appended_roles.capacity()
    }

    pub fn trim_quiet_logs(&mut self) -> bool {
        let quiet = self.log.trim_if_quiet();
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.log.trim_if_quiet();
        }
        quiet
    }

    pub(super) fn visits_required(&self, live: usize) -> usize {
        if self.part == 0 || self.advisor_cache.is_some() {
            return 0;
        }
        self.census_required.max(live)
    }

    pub(super) fn overflowed(&self) -> bool {
        (self.log.overflowed && !self.appended_overflowed)
            || self.traits.overflowed()
            || self.advisor_roots.overflowed
            || self
                .advisor_dynamic
                .as_ref()
                .is_some_and(|job| job.overflowed())
            || self.census_required > self.census_visited.capacity()
    }

    pub fn grow(&mut self) {
        engine_allocation::advisory(|| self.grow_hints());
    }

    fn grow_hints(&mut self) {
        if let Some(census) = self.portrait_census.as_mut().and_then(SharedBox::get_mut)
            && census.needs_growth()
        {
            if census
                .grow()
                .map_err(collector_memory::terminal::io)
                .is_err()
            {
                return;
            }
            self.log.invalidate_read_plan();
        }
        if let Some(census) = self.role_census.as_mut().and_then(SharedBox::get_mut) {
            census.grow();
        }
        let visits = self
            .visits_required(census::live_len())
            .saturating_sub(self.census_visited.len());
        if reserve(&mut self.census_visited, visits).is_err() {
            return;
        }
        self.update_census_charge();
        let appended_grew = self.appended_required > self.appended_roles.capacity();
        let appended = self
            .appended_required
            .saturating_sub(self.appended_roles.len());
        if reserve(&mut self.appended_roles, appended).is_err() {
            return;
        }
        if appended_grew {
            self.log.invalidate_read_plan();
        }
        self.appended_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(
                    &self.appended_roles,
                ),
                ..Default::default()
            });
        if let Some(cache) = &mut self.advisor_cache {
            cache.grow();
        }
        self.log.grow();
        self.traits.grow();
        if let Some(plan) = &mut self.source_bindings {
            plan.grow();
        }
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.grow();
        }
    }

    pub fn prepare_read_plans(&mut self) {
        if let Some(cache) = &mut self.advisor_cache {
            cache.prepare();
        }
        self.log.prepare_read_plan();
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.prepare_read_plans();
        }
    }

    pub fn note_growth(&self) {
        collector_raw::raw::note_growth("characters", &self.log);
        if let Some(dynamic) = &self.advisor_dynamic {
            dynamic.note_growth();
        }
    }

    pub(super) fn requires_no_retry(&self) -> bool {
        let mut observed = self.log.planned_source_observed();
        let mut failed = self.log.planned_capture_failed()
            || (!self.appended_overflowed && self.log.planned_capture_requires_no_retry());
        if let Some(dynamic) = &self.advisor_dynamic {
            observed |= dynamic.log.planned_source_observed();
            failed |= dynamic.requires_no_retry();
        }
        failed || (self.overflowed() && observed)
    }

    fn remember_appended(&mut self, role: AppendedRole) {
        self.appended_required = self.appended_required.saturating_add(1);
        if reserve(&mut self.appended_roles, 1).is_err() {
            self.appended_overflowed = true;
            return;
        }
        self.appended_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(
                    &self.appended_roles,
                ),
                ..Default::default()
            });
        self.appended_roles.push(role);
    }

    pub(super) fn fate(&self) -> TreeFate {
        if self.log.overflowed {
            TreeFate::Abort
        } else {
            TreeFate::Dead
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CharactersCopyReport {
    pub raw_ok: bool,
    pub plan_failed: bool,
    pub copy_ticks: [u64; N],
}

#[derive(Debug, Clone, Copy)]
pub struct CharactersCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub force_full: bool,
    pub active: [bool; N],
}

#[derive(Debug, Default)]
pub(super) struct Dead {
    pub(super) characters: bool,
    pub(super) leaders: bool,
    pub(super) advisors: bool,
    pub(super) units: bool,
    pub(super) traits: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ViewCopy {
    pub(super) starts: [u32; 8],
    pub(super) unit_id: Option<u32>,
    pub(super) carried: bool,
}

impl Default for ViewCopy {
    fn default() -> Self {
        Self {
            starts: [u32::MAX; 8],
            unit_id: None,
            carried: false,
        }
    }
}

impl ViewCopy {
    #[inline]
    pub(super) fn field<'a>(&self, job: &'a CharactersJob, index: usize) -> &'a [u8] {
        static ZERO: [u8; 64] = [0; 64];
        let len = VIEW_LENS[index];
        let start = self.starts[index];
        if start == u32::MAX {
            &ZERO[..len]
        } else {
            let start = start as usize;
            &job.log.bytes[start..start + len]
        }
    }
}

#[cfg(test)]
pub(super) fn copy_view(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Option<ViewCopy> {
    copy_view_carrying(job, memory, base, pointer, false)
}

const CARRIED_SPANS: [usize; 3] = [2, 3, 7];

pub(super) fn copy_view_carrying(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    carried: bool,
) -> Option<ViewCopy> {
    let mut view = ViewCopy {
        carried,
        ..ViewCopy::default()
    };
    let active = job.source_mask();
    let mut masks = VIEW_MASKS.map(|mask| mask & active);
    if carried {
        for index in CARRIED_SPANS {
            masks[index] = 0;
        }
    }
    let lengths = std::array::from_fn(|index| {
        if masks[index] != 0 {
            VIEW_LENS[index]
        } else {
            0
        }
    });
    let (starts, completed) = job.log.take_scattered(
        memory,
        VIEW_OFFSETS.map(|offset| pointer.checked_add(offset).unwrap_or(0)),
        lengths,
        masks,
    )?;
    for index in 0..completed {
        let (start, len) = (starts[index], lengths[index]);
        if len != 0 {
            view.starts[index] = start as u32;
        }
    }
    if completed == 0 {
        return None;
    }
    for index in completed + 1..VIEW_OFFSETS.len() {
        if lengths[index] == 0 {
            continue;
        }
        let addr = pointer.checked_add(VIEW_OFFSETS[index]).unwrap_or(0);
        let len = lengths[index];
        if let Some(start) = job.log.take(memory, addr, len, masks[index]) {
            view.starts[index] = start as u32;
        }
    }
    let expected = engine_sites::at(base, ccharacter::VTABLE_RVA)? as u64;
    let identity = view.field(job, 0);
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected
        || u32::from_le_bytes(
            identity[ccharacter::TYPE..ccharacter::TYPE + 4]
                .try_into()
                .unwrap(),
        ) != ccharacter::REFERENCE_TYPE
    {
        return None;
    }
    Some(view)
}

pub(super) struct Lit {
    pub(super) len: usize,
    pub(super) source: usize,
    pub(super) inline: bool,
}

#[inline]
pub(super) fn hint_character_target(
    address: usize,
    len: usize,
    hint: &mut impl FnMut(usize, usize),
) {
    if address != 0
        && address.checked_add(len).is_some()
        && collector_raw::raw::is_canonical_range(address, len)
    {
        hint(address, 1);
    }
}

#[inline]
pub(super) fn hint_character_targets(
    job: &CharactersJob,
    view: &ViewCopy,
    dead: &Dead,
    mut hint: impl FnMut(usize, usize),
) {
    if !dead.characters {
        let headers = view.field(job, 3);
        if let (Some(name), Some(token)) = (
            parse_character_literal(&headers[..32], 0),
            parse_character_literal(&headers[32..], 0),
        ) {
            hint_character_literal(&name, &mut hint);
            hint_character_literal(&token, &mut hint);
        }
    }
    for (field, active) in [(4, !dead.leaders), (6, !dead.advisors)] {
        if !active {
            continue;
        }
        let header = view.field(job, field);
        if u32::try_from(u64::from_le_bytes(header[8..16].try_into().unwrap())).is_ok()
            && let Ok(head) = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        {
            hint_character_target(head, 26, &mut hint);
        }
    }
    if !(dead.units && dead.traits)
        && let Ok(unit) =
            usize::try_from(u64::from_le_bytes(view.field(job, 5).try_into().unwrap()))
        && unit != 0
        && unit.checked_add(16).is_some()
        && collector_raw::raw::is_canonical_range(unit, 16)
    {
        hint_character_target(unit, 16, &mut hint);
        if !dead.units {
            for (offset, len) in [
                (cunit_leader::NAME, 32),
                (cunit_leader::SKILL, 32),
                (cunit_leader::CHARACTER_TYPE, 8),
                (cunit_leader::EXILE_COUNTRY, 4),
            ] {
                if let Some(address) = unit.checked_add(offset) {
                    hint_character_target(address, len, &mut hint);
                }
            }
        }
    }
}

#[inline]
pub(super) fn hint_character_literal(literal: &Lit, mut hint: impl FnMut(usize, usize)) {
    if literal.inline {
        return;
    }
    let Some(extent) = literal.len.checked_add(1) else {
        return;
    };
    if literal.source == 0
        || literal.source.checked_add(extent).is_none()
        || !collector_raw::raw::is_canonical_range(literal.source, extent)
    {
        return;
    }
    let terminator = literal.source + literal.len;
    hint(terminator, 1);
    if literal.len != 0 && literal.source >> 6 != terminator >> 6 {
        hint(literal.source, 1);
    }
}

pub(super) fn parse_character_literal(header: &[u8], header_addr: usize) -> Option<Lit> {
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    let source = if inline {
        header_addr
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0)?
    };
    if inline && header[len] != 0 {
        return None;
    }
    Some(Lit {
        len,
        source,
        inline,
    })
}

pub(super) fn copy_unit_literal(
    job: &mut CharactersJob,
    memory: &impl Memory,
    header: &[u8],
    mask: u32,
) -> Option<usize> {
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    if capacity == msvc_string::INLINE_CAPACITY {
        if header[len] != 0 {
            return None;
        }
        return Some(len);
    }
    let source = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&pointer| pointer != 0)?;
    copy_heap_literal_bytes(job, memory, source, len, mask).then_some(len)
}

pub(super) fn copy_character_literal_bytes(
    job: &mut CharactersJob,
    memory: &impl Memory,
    literal: &Lit,
    mask: u32,
) -> bool {
    if literal.inline {
        return true;
    }
    copy_heap_literal_bytes(job, memory, literal.source, literal.len, mask)
}

pub(super) fn copy_heap_literal_bytes(
    job: &mut CharactersJob,
    memory: &impl Memory,
    source: usize,
    len: usize,
    mask: u32,
) -> bool {
    let Some(terminator) = source.checked_add(len) else {
        return false;
    };
    let fits = len != 0
        && len <= u32::MAX as usize
        && job.log.spans.capacity() - job.log.spans.len() >= 2
        && job
            .log
            .bytes
            .len()
            .checked_add(1)
            .and_then(|end| end.checked_add(len))
            .is_some_and(|end| end <= job.log.bytes.capacity() && end <= u32::MAX as usize);
    if fits {
        return matches!(
            job.log
                .take_scattered(memory, [terminator, source], [1, len], [mask; 2]),
            Some((_, 2))
        );
    }
    if job.log.take(memory, terminator, 1, mask).is_none() {
        return false;
    }
    if len != 0 && job.log.take(memory, source, len, mask).is_none() {
        return false;
    }
    true
}

pub(super) struct TreeWalk {
    pub(super) head: usize,
    pub(super) next: usize,
    pub(super) remaining: u32,
    pub(super) count: u32,
}

pub(super) enum TreeFate {
    Abort,
    Dead,
}

#[derive(Clone, Copy)]
pub(super) enum Sentinel<'a> {
    Read,
    Gathered(Option<&'a [u8]>),
    Owned,
}

pub(super) fn tree_read(
    job: &mut CharactersJob,
    memory: &impl Memory,
    head: u64,
    count: u64,
    mask: u32,
    source: Sentinel<'_>,
) -> Result<TreeWalk, TreeFate> {
    let head = usize::try_from(head)
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or_else(|| job.fate())?;
    let count = u32::try_from(count).map_err(|_| job.fate())?;
    let start = match source {
        Sentinel::Read => job.log.take(memory, head, census::SENTINEL, mask),
        Sentinel::Gathered(observed) => job.log.record(head, census::SENTINEL, observed, mask),
        Sentinel::Owned => return Err(TreeFate::Dead),
    }
    .ok_or_else(|| job.fate())?;
    let mut sentinel = [0u8; census::SENTINEL];
    sentinel.copy_from_slice(&job.log.bytes[start..start + census::SENTINEL]);
    let next = tree_sentinel(head, count, &sentinel).ok_or(TreeFate::Dead)?;
    Ok(TreeWalk {
        head,
        next,
        remaining: count,
        count,
    })
}

pub(super) fn tree_sentinel(head: usize, count: u32, sentinel: &[u8]) -> Option<usize> {
    if sentinel[msvc_tree_node::NIL] != 1 {
        return None;
    }
    let link = |offset: usize| {
        usize::try_from(u64::from_le_bytes(
            sentinel[offset..offset + 8].try_into().unwrap(),
        ))
        .ok()
        .filter(|&pointer| pointer != 0)
    };
    let next = link(msvc_tree_node::LEFT)?;
    if (count == 0) != (next == head) {
        return None;
    }
    if count == 0 && (link(msvc_tree_node::PARENT)? != head || link(msvc_tree_node::RIGHT)? != head)
    {
        return None;
    }
    Some(next)
}

pub(super) fn tree_link(
    job: &mut CharactersJob,
    memory: &impl Memory,
    node: usize,
    offset: usize,
    mask: u32,
) -> Result<usize, TreeFate> {
    let addr = node.checked_add(offset).ok_or(TreeFate::Dead)?;
    let start = job
        .log
        .take(memory, addr, 8, mask)
        .ok_or_else(|| job.fate())?;
    usize::try_from(u64::from_le_bytes(
        job.log.bytes[start..start + 8].try_into().unwrap(),
    ))
    .ok()
    .filter(|&pointer| pointer != 0)
    .ok_or(TreeFate::Dead)
}

pub(super) fn tree_next(
    job: &mut CharactersJob,
    memory: &impl Memory,
    walk: &mut TreeWalk,
    mask: u32,
) -> Result<Option<usize>, TreeFate> {
    if walk.remaining == 0 {
        if walk.next != walk.head {
            return Err(TreeFate::Dead);
        }
        return Ok(None);
    }
    let current = walk.next;
    if current == walk.head {
        return Err(TreeFate::Dead);
    }
    let nil_addr = current
        .checked_add(msvc_tree_node::NIL)
        .ok_or(TreeFate::Dead)?;
    let nil = job
        .log
        .take(memory, nil_addr, 1, mask)
        .ok_or_else(|| job.fate())?;
    if job.log.bytes[nil] != 0 {
        return Err(TreeFate::Dead);
    }
    let right = tree_link(job, memory, current, msvc_tree_node::RIGHT, mask)?;
    let mut next = right;
    let mut steps = 0_u64;
    if right != walk.head {
        loop {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            let nil_addr = next
                .checked_add(msvc_tree_node::NIL)
                .ok_or(TreeFate::Dead)?;
            let nil = job
                .log
                .take(memory, nil_addr, 1, mask)
                .ok_or_else(|| job.fate())?;
            if job.log.bytes[nil] != 0 {
                return Err(TreeFate::Dead);
            }
            let left = tree_link(job, memory, next, msvc_tree_node::LEFT, mask)?;
            if left == walk.head {
                break;
            }
            if tree_link(job, memory, left, msvc_tree_node::PARENT, mask)? != next {
                return Err(TreeFate::Dead);
            }
            next = left;
        }
        if tree_link(job, memory, right, msvc_tree_node::PARENT, mask)? != current {
            return Err(TreeFate::Dead);
        }
    } else {
        let mut child = current;
        next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        while next != walk.head {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            if tree_link(job, memory, next, msvc_tree_node::LEFT, mask)? == child {
                break;
            }
            if tree_link(job, memory, next, msvc_tree_node::RIGHT, mask)? != child {
                return Err(TreeFate::Dead);
            }
            child = next;
            next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        }
    }
    walk.next = next;
    walk.remaining -= 1;
    Ok(Some(current))
}

pub(super) fn take_or_kill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
    dead: &mut bool,
) -> Option<usize> {
    match addr.and_then(|addr| job.log.take(memory, addr, len, mask)) {
        Some(start) => Some(start),
        None => {
            *dead = true;
            None
        }
    }
}

#[inline]
pub(super) fn take_fixed_fields<const M: usize, const CONTINUE: bool>(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    mask: u32,
    dead: &mut bool,
) -> [Option<usize>; M] {
    let mut starts = [None; M];
    let total = lengths.iter().try_fold(job.log.bytes.len(), |end, &len| {
        end.checked_add(len)
            .filter(|&end| len <= u32::MAX as usize && end <= u32::MAX as usize)
    });
    if addresses.iter().any(Option::is_none)
        || job.log.overflowed
        || lengths.iter().filter(|&&len| len != 0).count()
            > job.log.spans.capacity() - job.log.spans.len()
        || total.is_none_or(|end| end > job.log.bytes.capacity())
    {
        for index in 0..M {
            starts[index] = take_or_kill(job, memory, addresses[index], lengths[index], mask, dead);
            if !CONTINUE && starts[index].is_none() {
                break;
            }
        }
        return starts;
    }
    let Some((copied, completed)) = job.log.take_scattered_prefetched(
        memory,
        addresses.map(Option::unwrap),
        lengths,
        [mask; M],
    ) else {
        *dead = true;
        return starts;
    };
    for index in 0..completed {
        starts[index] = Some(copied[index]);
    }
    if completed != M {
        *dead = true;
        if CONTINUE {
            for index in completed + 1..M {
                starts[index] =
                    take_or_kill(job, memory, addresses[index], lengths[index], mask, dead);
            }
        }
    }
    starts
}

pub(super) fn pool_u64(job: &CharactersJob, start: usize) -> u64 {
    u64::from_le_bytes(job.log.bytes[start..start + 8].try_into().unwrap())
}

pub(super) fn pool_u32(job: &CharactersJob, start: usize) -> u32 {
    u32::from_le_bytes(job.log.bytes[start..start + 4].try_into().unwrap())
}

pub(super) fn copy_character_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let name_addr = pointer.checked_add(ccharacter::NAME).unwrap_or(0);
    let token_addr = pointer.checked_add(ccharacter::TOKEN).unwrap_or(0);
    let headers = view.field(job, 3);
    let name = parse_character_literal(&headers[..32], name_addr);
    let token = parse_character_literal(&headers[32..64], token_addr);
    let (Some(name), Some(token)) = (name, token) else {
        dead.characters = true;
        return;
    };
    job.character_demand.add(1, name.len);
    job.character_demand.add(0, token.len);
    if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
        || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
    {
        dead.characters = true;
    }
}

pub(super) fn copy_character_template(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    id: u32,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let at = ccharacter::TEMPLATE - ccharacter::NAME_TOKEN_ID;
    let Ok(template) = usize::try_from(u64::from_le_bytes(
        view.field(job, 2)[at..at + 8].try_into().unwrap(),
    )) else {
        dead.characters = true;
        return;
    };
    if template == 0 {
        return;
    }
    if let Some(known) = job.templates.known(id, template) {
        let bytes = known.file.len();
        job.character_demand.add(0, bytes);
        return;
    }
    let [header, _] = take_fixed_fields::<2, false>(
        job,
        memory,
        [
            Some(template),
            template.checked_add(ccharacter_template::GENERATED),
        ],
        [ccharacter_template::HEADER, 1],
        MASK_CHARACTERS,
        &mut dead.characters,
    );
    if dead.characters {
        return;
    }
    let Some(header) = header else {
        dead.characters = true;
        return;
    };
    let Some(expected) = engine_sites::at(base, ccharacter_template::VTABLE_RVA) else {
        dead.characters = true;
        return;
    };
    let file = {
        let bytes = &job.log.bytes[header..header + ccharacter_template::HEADER];
        (u64::from_le_bytes(bytes[..8].try_into().unwrap()) == expected as u64)
            .then(|| {
                parse_character_literal(
                    &bytes[ccharacter_template::SOURCE_FILE..ccharacter_template::SOURCE_LINE],
                    template + ccharacter_template::SOURCE_FILE,
                )
            })
            .flatten()
    };
    match file {
        Some(file) if copy_character_literal_bytes(job, memory, &file, MASK_CHARACTERS) => {
            job.character_demand.add(0, file.len);
        }
        _ => dead.characters = true,
    }
}

pub(super) fn copy_leaders(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    header: impl FnOnce(&CharactersJob) -> [u8; 16],
    pointer: usize,
    sentinel: Sentinel<'_>,
    dead: &mut Dead,
) -> bool {
    if dead.leaders {
        return true;
    }
    let expected = match engine_sites::at(base, engine_native_parts::ccountry_leader_layout::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.leaders = true;
            return true;
        }
    };
    let header = header(job);
    let head = u64::from_le_bytes(header[..8].try_into().unwrap());
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut walk = match tree_read(job, memory, head, count, MASK_LEADERS, sentinel) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.leaders = true;
            return true;
        }
    };
    let Some(required) = job.leader_rows.checked_add(walk.count as usize) else {
        dead.leaders = true;
        return true;
    };
    job.leader_rows = required;
    let mut previous = None;
    loop {
        let node = match tree_next(job, memory, &mut walk, MASK_LEADERS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.leaders = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some(ideo) = take_or_kill(
            job,
            memory,
            node.checked_add(character_leader_node::IDEOLOGY),
            16,
            MASK_LEADERS,
            &mut dead.leaders,
        ) else {
            return !job.log.overflowed;
        };
        let ideology = i32::from_le_bytes(job.log.bytes[ideo..ideo + 4].try_into().unwrap());
        if previous.is_some_and(|key| key >= ideology) {
            dead.leaders = true;
            return true;
        }
        previous = Some(ideology);
        let role = usize::try_from(pool_u64(job, ideo + 8))
            .ok()
            .filter(|&role| role != 0);
        let Some(header) = take_or_kill(job, memory, role, 16, MASK_LEADERS, &mut dead.leaders)
        else {
            return !job.log.overflowed;
        };
        if pool_u64(job, header) != expected
            || pool_u64(job, header + engine_native_parts::ccountry_leader_layout::CHARACTER) != pointer as u64
        {
            dead.leaders = true;
            return true;
        }
        job.remember_appended(AppendedRole::Leader(role.unwrap()));
    }
    true
}

pub(super) fn take_advisor_fields<const M: usize>(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    dead: &mut bool,
) -> Option<[usize; M]> {
    if addresses.iter().any(Option::is_none) {
        let mut starts = [0; M];
        for index in 0..M {
            starts[index] = take_or_kill(
                job,
                memory,
                addresses[index],
                lengths[index],
                MASK_ADVISORS,
                dead,
            )?;
        }
        return Some(starts);
    }
    match job.log.take_scattered_prefetched(
        memory,
        addresses.map(Option::unwrap),
        lengths,
        [MASK_ADVISORS; M],
    ) {
        Some((starts, completed)) if completed == M => Some(starts),
        _ => {
            *dead = true;
            None
        }
    }
}

pub(super) fn copy_advisors(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) -> bool {
    if dead.advisors {
        return true;
    }
    let expected = match engine_sites::at(base, cadvisor::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.advisors = true;
            return true;
        }
    };
    let header = view.field(job, 6);
    let head = u64::from_le_bytes(header[..8].try_into().unwrap());
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut walk = match tree_read(job, memory, head, count, MASK_ADVISORS, Sentinel::Read) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.advisors = true;
            return true;
        }
    };
    let mut previous_hash = None;
    loop {
        if walk.remaining != 0 {
            match job.advisor_demand.rows.checked_add(1) {
                Some(rows) => job.advisor_demand.rows = rows,
                None => job.advisor_demand.overflowed = true,
            }
        }
        let node = match tree_next(job, memory, &mut walk, MASK_ADVISORS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.advisors = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some([role_ptr, key_header, hash]) = take_advisor_fields(
            job,
            memory,
            [
                node.checked_add(character_advisor_node::ADVISOR),
                node.checked_add(character_advisor_node::KEY),
                node.checked_add(character_advisor_node::KEY_HASH),
            ],
            [8, 32, 4],
            &mut dead.advisors,
        ) else {
            return !job.log.overflowed;
        };
        let role = usize::try_from(pool_u64(job, role_ptr))
            .ok()
            .filter(|&role| role != 0);
        if role.is_none() {
            dead.advisors = true;
            return true;
        }
        let role = role.unwrap();
        let Some([vtable, parent, idea_header, slot_header]) = take_advisor_fields(
            job,
            memory,
            [
                Some(role),
                role.checked_add(cadvisor::CHARACTER),
                role.checked_add(cadvisor::IDEA_TOKEN),
                role.checked_add(cadvisor::SLOT),
            ],
            [8, 8, 32, 32],
            &mut dead.advisors,
        ) else {
            return !job.log.overflowed;
        };
        if pool_u64(job, vtable) != expected || pool_u64(job, parent) != pointer as u64 {
            dead.advisors = true;
            return true;
        }
        let hash = pool_u32(job, hash);
        if previous_hash.is_some_and(|previous| previous > hash) || previous_hash == Some(hash) {
            dead.advisors = true;
            return true;
        }
        previous_hash = Some(hash);
        let key_addr = node.checked_add(character_advisor_node::KEY).unwrap_or(0);
        let idea_addr = role.checked_add(cadvisor::IDEA_TOKEN).unwrap_or(0);
        let slot_addr = role.checked_add(cadvisor::SLOT).unwrap_or(0);
        let literal = |start: usize, addr: usize| {
            let mut header = [0u8; 32];
            header.copy_from_slice(&job.log.bytes[start..start + 32]);
            parse_character_literal(&header, addr)
        };
        let (Some(key), Some(idea), Some(slot)) = (
            literal(key_header, key_addr),
            literal(idea_header, idea_addr),
            literal(slot_header, slot_addr),
        ) else {
            dead.advisors = true;
            return true;
        };
        let required_bytes = [key.len, idea.len, slot.len]
            .into_iter()
            .try_fold(job.advisor_demand.bytes, usize::checked_add);
        let mut copied = true;
        let mut validation_failed = false;
        for literal in [&key, &idea, &slot] {
            let first_span = job.log.spans.len();
            if !copy_character_literal_bytes(job, memory, literal, MASK_ADVISORS) {
                let terminator = literal.source.checked_add(literal.len);
                validation_failed = terminator.is_none()
                    || job.log.spans.get(first_span).is_some_and(|span| {
                        !span.ok && Some(span.addr) == terminator && span.len == 1
                    });
                copied = false;
                break;
            }
        }
        if !validation_failed {
            match required_bytes {
                Some(bytes) => job.advisor_demand.bytes = bytes,
                None => job.advisor_demand.overflowed = true,
            }
        }
        if !copied {
            dead.advisors = true;
            return true;
        }
        let Some(cache_header) = take_or_kill(
            job,
            memory,
            role.checked_add(cadvisor::MODIFIER_ENTRIES),
            16,
            MASK_ADVISORS,
            &mut dead.advisors,
        ) else {
            return !job.log.overflowed;
        };
        let header = job.log.bytes[cache_header..cache_header + 16]
            .try_into()
            .unwrap();
        let Some(array) = Array::from_header(header, engine_native_parts::modifier_entry::STRIDE) else {
            dead.advisors = true;
            return true;
        };
        let required = (array.count as usize)
            .checked_mul(engine_native_parts::modifier_entry::STRIDE)
            .and_then(|bytes| job.advisor_demand.bytes.checked_add(bytes));
        match required {
            Some(bytes) => job.advisor_demand.bytes = bytes,
            None => job.advisor_demand.overflowed = true,
        }
        let mut first = 0;
        while first < array.count {
            let count = (array.count - first).min(64) as usize;
            if take_or_kill(
                job,
                memory,
                array.element(first, engine_native_parts::modifier_entry::STRIDE),
                count * engine_native_parts::modifier_entry::STRIDE,
                MASK_ADVISORS,
                &mut dead.advisors,
            )
            .is_none()
            {
                return !job.log.overflowed;
            }
            first += count as u32;
        }
        job.remember_appended(AppendedRole::Advisor(role));
    }
    true
}

pub(super) fn parse_unit_identity(
    job: &CharactersJob,
    base: usize,
    start: usize,
) -> Option<(u8, u32)> {
    let vtable = pool_u64(job, start);
    let kind = match engine_sites::which(
        base,
        usize::try_from(vtable).ok()?,
        &[
            cunit_leader::VTABLE_RVA,
            carmy_leader::VTABLE_RVA,
            cnavy_leader::VTABLE_RVA,
            engine_native_parts::coperative_leader_layout::VTABLE_RVA,
        ],
    )? {
        cunit_leader::VTABLE_RVA => cunit_leader::KIND_BASE,
        carmy_leader::VTABLE_RVA => cunit_leader::KIND_ARMY,
        cnavy_leader::VTABLE_RVA => cunit_leader::KIND_NAVY,
        engine_native_parts::coperative_leader_layout::VTABLE_RVA => cunit_leader::KIND_OPERATIVE,
        _ => return None,
    };
    if pool_u32(job, start + cunit_leader::TYPE) != cunit_leader::REFERENCE_TYPE {
        return None;
    }
    Some((kind, pool_u32(job, start + cunit_leader::ID)))
}

#[expect(clippy::too_many_arguments, reason = "A unit copy takes its character's pointer and id beside the job, memory and outputs.")]
pub(super) fn copy_unit(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    unit: Option<usize>,
    character: usize,
    character_id: impl FnOnce(&CharactersJob) -> u32,
    unit_id: &mut Option<u32>,
    dead: &mut Dead,
) -> bool {
    let Some(unit) = unit else {
        return true;
    };
    if dead.units && dead.traits {
        return true;
    }
    let need_fallback = !dead.traits;
    let source_mask = job.source_mask();
    let identity = take_or_kill(
        job,
        memory,
        Some(unit),
        16,
        MASK_UNIT_LINK & source_mask,
        &mut dead.units,
    );
    let parsed = identity.and_then(|start| parse_unit_identity(job, base, start));
    *unit_id = parsed.map(|(_, id)| id);
    if identity.is_some() && parsed.is_none() {
        dead.units = true;
    }
    let kind = parsed.map(|(kind, _)| kind);
    if !dead.units {
        let state = take_fixed_fields::<4, false>(
            job,
            memory,
            [
                cunit_leader::CHARACTER_TYPE,
                cunit_leader::CATEGORY,
                cunit_leader::SKILL,
                cunit_leader::EXILE_COUNTRY,
            ]
            .map(|offset| unit.checked_add(offset)),
            [8, 4, 8, 4],
            MASK_UNITS,
            &mut dead.units,
        );
        if !job.log.overflowed
            && let [Some(backlink), Some(_category), Some(skill), Some(_exile)] = state
        {
            let character_id = character_id(job);
            if pool_u32(job, backlink) != ccharacter::REFERENCE_TYPE
                || pool_u32(job, backlink + 4) != character_id
            {
                dead.units = true;
            } else {
                if matches!(
                    kind,
                    Some(cunit_leader::KIND_ARMY | cunit_leader::KIND_NAVY)
                ) {
                    let key = (unit, character_id, parsed.unwrap().1);
                    let held = job.experience.get(key);
                    let address = unit.checked_add(cunit_leader::EXPERIENCE);
                    let value = match (address, held) {
                        (Some(address), Some(value)) => {
                            job.log
                                .record(address, 8, Some(&value.to_le_bytes()), MASK_UNITS)
                        }
                        _ => take_or_kill(job, memory, address, 8, MASK_UNITS, &mut dead.units),
                    };
                    if let Some(at) = value {
                        if held.is_none() {
                            let value =
                                i64::from_le_bytes(job.log.bytes[at..at + 8].try_into().unwrap());
                            job.experience.put(key, value);
                        }
                    } else {
                        dead.units = true;
                    }
                }
                if !dead.units {
                    copy_unit_skill(job, memory, base, skill, dead);
                }
            }
        }
        if job.log.overflowed {
            return false;
        }
        if !dead.units {
            if let Some(kind) = kind {
                copy_unit_components(job, memory, unit, kind, dead);
            }
            if job.log.overflowed {
                return false;
            }
        }
        if !dead.units {
            copy_unit_text(job, memory, unit, dead);
            if job.log.overflowed {
                return false;
            }
        }
        if !dead.units && kind.is_some_and(cunit_leader::has_codename) {
            copy_unit_codename(job, memory, base, unit);
            if job.log.overflowed {
                return false;
            }
        }
        if !dead.units {
            job.remember_appended(AppendedRole::Visibility { leader: unit, character });
        }
        if !dead.units
            && job.active[5]
            && matches!(
                kind,
                Some(cunit_leader::KIND_ARMY | cunit_leader::KIND_NAVY)
            )
        {
            let source = AppendedCopy {
                job: std::cell::RefCell::new(job),
                memory,
                mask: MASK_TRAINING,
            };
            let mut count = 0_usize;
            let _ = cunit_leader::visit_trait_progress(&source, base, unit, |_, _, _| {
                count += 1;
            });
            let _ = cunit_leader::visit_trait_factors(&source, base, unit, |_, _, _| {
                count += 1;
            });
            source
                .job
                .borrow_mut()
                .unit_demand
                .add(0, count.saturating_mul(16));
            if source.job.borrow().log.overflowed {
                return false;
            }
        }
        if !dead.units
            && let (Some(kind), Some(category)) = (kind, state[1])
            && kind == cunit_leader::KIND_ARMY
            && pool_u32(job, category) < 2
        {
            job.remember_appended(AppendedRole::Unit {
                pointer: unit,
                kind,
                category: pool_u32(job, category) as i32,
            });
        }
    }
    if need_fallback {
        let mut fallback_dead = false;
        let [first, second] = take_fixed_fields::<2, false>(
            job,
            memory,
            [
                unit.checked_add(cunit_leader::TYPE),
                unit.checked_add(cunit_leader::CHARACTER_TYPE),
            ],
            [8, 8],
            MASK_TRAIT_FALLBACK & source_mask,
            &mut fallback_dead,
        );
        if job.log.overflowed {
            return false;
        }
        if dead.units && (first.is_none() || second.is_none()) {
            dead.traits = true;
        }
        if dead.units
            && let Some(first) = first
        {
            *unit_id = Some(pool_u32(job, first + 4));
        }
    }
    true
}

pub(super) fn copy_unit_skill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    skill_start: usize,
    dead: &mut Dead,
) {
    let skill = usize::try_from(pool_u64(job, skill_start)).ok();
    match skill {
        None => {
            dead.units = true;
        }
        Some(0) => {}
        Some(skill) => {
            let skill_expected = match engine_sites::at(base, cunit_leader_skill::VTABLE_RVA) {
                Some(expected) => expected as u64,
                None => {
                    dead.units = true;
                    return;
                }
            };
            let addresses = [
                Some(skill),
                skill.checked_add(cunit_leader_skill::VALID),
                skill.checked_add(cunit_leader_skill::LEVEL),
            ];
            let cache_slot = (skill >> 3) % job.skills.len();
            let cached = job.skills[cache_slot];
            let [vtable, valid, scalars] = if cached.pointer == skill && !job.log.planning() {
                let native_bytes = job.log.raw_bytes[3];
                let starts = take_fixed_fields::<3, true>(
                    job,
                    &cached,
                    addresses,
                    [8, 1, 12],
                    MASK_UNITS,
                    &mut dead.units,
                );
                job.log.raw_bytes[3] = native_bytes;
                starts
            } else {
                take_fixed_fields::<3, true>(
                    job,
                    memory,
                    addresses,
                    [8, 1, 12],
                    MASK_UNITS,
                    &mut dead.units,
                )
            };
            if let Some(vtable) = vtable
                && pool_u64(job, vtable) != skill_expected
            {
                dead.units = true;
            }
            if !dead.units
                && cached.pointer != skill
                && let [Some(vtable), Some(valid), Some(scalars)] = [vtable, valid, scalars]
            {
                let mut bytes = [0; 21];
                bytes[..8].copy_from_slice(&job.log.bytes[vtable..vtable + 8]);
                bytes[8] = job.log.bytes[valid];
                bytes[9..].copy_from_slice(&job.log.bytes[scalars..scalars + 12]);
                job.skills[cache_slot] = SkillCopy {
                    pointer: skill,
                    bytes,
                };
            }
        }
    }
}

pub(super) fn copy_unit_components(
    job: &mut CharactersJob,
    memory: &impl Memory,
    unit: usize,
    kind: u8,
    dead: &mut Dead,
) {
    let components = match kind {
        cunit_leader::KIND_ARMY => Some([
            carmy_leader::ATTACK,
            carmy_leader::DEFENSE,
            carmy_leader::PLANNING,
            carmy_leader::LOGISTICS,
        ]),
        cunit_leader::KIND_NAVY => Some([
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]),
        _ => None,
    };
    if let Some(offsets) = components {
        take_fixed_fields::<4, false>(
            job,
            memory,
            offsets.map(|offset| unit.checked_add(offset)),
            [4; 4],
            MASK_UNITS,
            &mut dead.units,
        );
    }
    let at = |offset| unit.checked_add(offset);
    let owner = [
        at(cunit_leader::OWNER),
        at(cunit_leader::VISIBILITY_CHOICE),
        at(cunit_leader::TEMPLATE_VISIBILITY),
    ];
    if kind == cunit_leader::KIND_ARMY {
        let [a, b, c] = owner;
        take_fixed_fields::<4, false>(
            job,
            memory,
            [a, b, c, at(carmy_leader::CAPTURED)],
            [4, 8, 1, 1],
            MASK_UNITS,
            &mut dead.units,
        );
    } else {
        take_fixed_fields::<3, false>(job, memory, owner, [4, 8, 1], MASK_UNITS, &mut dead.units);
    }
}

pub(super) fn copy_unit_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    unit: usize,
    dead: &mut Dead,
) {
    let [name_header, token_header] = take_fixed_fields::<2, true>(
        job,
        memory,
        [
            unit.checked_add(cunit_leader::NAME),
            unit.checked_add(cunit_leader::TOKEN),
        ],
        [32, 32],
        MASK_UNITS,
        &mut dead.units,
    );
    let (Some(name_header), Some(token_header)) = (name_header, token_header) else {
        return;
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&job.log.bytes[name_header..name_header + 32]);
    let name = copy_unit_literal(job, memory, &header, MASK_UNITS);
    header.copy_from_slice(&job.log.bytes[token_header..token_header + 32]);
    let token = copy_unit_literal(job, memory, &header, MASK_UNITS);
    match (name, token) {
        (Some(name), Some(token)) => {
            job.unit_demand.add(1, name);
            job.unit_demand.add(0, token);
        }
        _ => dead.units = true,
    }
}

struct AppendedCopy<'a, M> {
    job: std::cell::RefCell<&'a mut CharactersJob>,
    memory: &'a M,
    mask: u32,
}

pub(super) fn copy_appended_roles(job: &mut CharactersJob, memory: &impl Memory, base: usize) {
    if job.overflowed() || job.log.planned_capture_failed() {
        return;
    }
    let mut names = Vec::new();
    for index in 0..job.appended_roles.len() {
        let role = job.appended_roles[index];
        let source = AppendedCopy {
            job: std::cell::RefCell::new(&mut *job),
            memory,
            mask: match role {
                AppendedRole::Leader(_) => MASK_LEADERS,
                AppendedRole::Advisor(_) => MASK_ADVISORS,
                AppendedRole::Unit { .. } | AppendedRole::Visibility { .. } => MASK_UNITS,
            },
        };
        match role {
            AppendedRole::Leader(pointer) => {
                let _ = ccountry_leader::current_ideology(&source, base, pointer);
            }
            AppendedRole::Advisor(pointer) => {
                let _ = cadvisor::visit_tooltip_keys(&source, pointer, |_, header| {
                    let mut job = source.job.borrow_mut();
                    let len = copy_unit_literal(&mut job, memory, &header, MASK_ADVISORS)
                        .ok_or("advisor tooltip key unreadable or invalid")?;
                    let Some(bytes) = len
                        .checked_add(8)
                        .and_then(|bytes| job.advisor_demand.bytes.checked_add(bytes))
                    else {
                        return Err("advisor tooltip byte count overflow");
                    };
                    job.advisor_demand.bytes = bytes;
                    Ok(())
                });
            }
            AppendedRole::Unit {
                pointer,
                kind,
                category,
            } => {
                let _ = cunit_leader::read_formation_reference(&source, pointer, kind, category);
            }
            AppendedRole::Visibility { leader, character } => {
                let mut used = 0;
                if let Ok((_, Some((_, len, _)))) = engine_characters::unit_leader_visibility::visibility(
                    &source,
                    base,
                    leader,
                    character,
                    &mut engine_native_memory::walk::Buffer::Growing(&mut names),
                    &mut used,
                ) {
                    source.job.borrow_mut().unit_demand.add(0, len);
                }
            }
        }
        if source.job.borrow().log.overflowed {
            source.job.borrow_mut().appended_overflowed = true;
            break;
        }
    }
}

impl<M: Memory> Memory for AppendedCopy<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let mut job = self.job.borrow_mut();
        let Some(start) = job.log.take(self.memory, address, output.len(), self.mask) else {
            return false;
        };
        output.copy_from_slice(&job.log.bytes[start..start + output.len()]);
        true
    }
}

fn copy_unit_codename(job: &mut CharactersJob, memory: &impl Memory, base: usize, unit: usize) {
    use engine_native::cname_group_member as naming;
    let member = unit
        .checked_add(engine_native_parts::coperative_leader_layout::CODENAME)
        .and_then(|member| job.log.take(memory, member, naming::INPUT_SPAN, MASK_UNITS));
    let Some(start) = member else { return };
    let mut bytes = [0_u8; naming::INPUT_SPAN];
    bytes.copy_from_slice(&job.log.bytes[start..start + naming::INPUT_SPAN]);
    if Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        != engine_sites::at(base, naming::VTABLE_RVA).map(|address| address as u64)
        || i32::from_le_bytes(bytes[naming::TYPE..naming::TYPE + 4].try_into().unwrap()) != 2
    {
        return;
    }
    let mut override_shown = bytes[naming::FLAG_B] == 0;
    for offset in [naming::FIRST_NAME_TEXT, naming::OVERRIDE_NAME] {
        if offset == naming::OVERRIDE_NAME && !override_shown {
            let length = u64::from_le_bytes(bytes[0x98..0xa0].try_into().unwrap());
            if length == 0 {
                continue;
            }
            let source = AppendedCopy {
                job: std::cell::RefCell::new(&mut *job),
                memory,
                mask: MASK_UNITS,
            };
            override_shown = match unit.checked_add(engine_native_parts::coperative_leader_layout::CODENAME) {
                Some(member) => {
                    match naming::operative_group_is_null(&source, base, member, &bytes) {
                        Ok(null) => null,
                        Err(_) => return,
                    }
                }
                None => return,
            };
            if !override_shown {
                continue;
            }
        }
        let header = &bytes[offset..offset + 32];
        let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().unwrap());
        let (len, capacity) = (word(msvc_string::LENGTH), word(msvc_string::CAPACITY));
        if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
            return;
        }
        let Ok(len) = usize::try_from(len) else {
            return;
        };
        job.unit_demand.add(0, len);
        if capacity == msvc_string::INLINE_CAPACITY {
            if len != 0 && header[len] != 0 {
                return;
            }
        } else {
            let Some(source) = usize::try_from(word(0)).ok().filter(|&source| source != 0) else {
                return;
            };
            if len != 0 {
                let Some(last) = source
                    .checked_add(len - 1)
                    .filter(|_| source.checked_add(len).is_some())
                else {
                    return;
                };
                let validation = job.log.take_scattered_prefetched(
                    memory,
                    [source, last],
                    [1, 2],
                    [MASK_UNITS; 2],
                );
                let Some((starts, 2)) = validation else {
                    return;
                };
                if job.log.bytes[starts[1] + 1] != 0
                    || job.log.take(memory, source, len, MASK_UNITS).is_none()
                {
                    return;
                }
            }
        }
    }
}

pub(super) fn copy_trait_vector(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    params: TraitVectorCopy,
    dead: &mut bool,
) -> bool {
    if *dead {
        return true;
    }
    if let Some(plan) = &mut job.source_bindings {
        plan.remember(params);
    }
    *dead = job.traits.acquire(memory, base, params).is_err();
    !job.traits.overflowed()
}

pub(super) fn copy_character_body(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    view: &mut ViewCopy,
    dead: &mut Dead,
) -> bool {
    if job.log.overflowed {
        return false;
    }
    let id = u32::from_le_bytes(
        view.field(job, 0)[ccharacter::ID..ccharacter::ID + 4]
            .try_into()
            .unwrap(),
    );
    if view.carried && job.carry.carried_row(pointer, id).is_none() {
        job.carry.uncarry(pointer);
        job.log.invalidate_read_plan();
        view.carried = false;
        let mask = job.source_mask();
        for index in CARRIED_SPANS {
            let len = VIEW_LENS[index];
            if VIEW_MASKS[index] & mask == 0 {
                continue;
            }
            let address = pointer.checked_add(VIEW_OFFSETS[index]).unwrap_or(0);
            if let Some(start) = job.log.take(memory, address, len, VIEW_MASKS[index] & mask) {
                view.starts[index] = start as u32;
            }
        }
        if job.log.overflowed {
            return false;
        }
    }
    if view.carried {
        job.character_demand
            .add(1, job.carry.carried_bytes(pointer));
    } else {
        copy_character_text(job, memory, view, pointer, dead);
        if job.log.overflowed {
            return false;
        }
        copy_character_template(job, memory, base, view, id, dead);
        if job.log.overflowed {
            return false;
        }
    }
    let fields = *view;
    let header = |job: &CharactersJob| fields.field(job, 4).try_into().unwrap();
    if !copy_leaders(job, memory, base, header, pointer, Sentinel::Read, dead) {
        return false;
    }
    if !copy_advisors(job, memory, base, view, pointer, dead) {
        return false;
    }
    let unit = view_unit(job, view);
    let character_id = |job: &CharactersJob| {
        u32::from_le_bytes(
            fields.field(job, 0)[ccharacter::ID..ccharacter::ID + 4]
                .try_into()
                .unwrap(),
        )
    };
    if !copy_unit(
        job,
        memory,
        base,
        unit,
        pointer,
        character_id,
        &mut view.unit_id,
        dead,
    ) {
        return false;
    }
    match view_unit(job, view) {
        Some(unit) => copy_trait_vectors(job, memory, base, unit, view.unit_id.unwrap_or(id), dead),
        None => true,
    }
}

pub(super) fn view_unit(job: &CharactersJob, view: &ViewCopy) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(view.field(job, 5).try_into().unwrap()))
        .ok()
        .filter(|&unit| unit != 0)
}

pub(super) fn copy_trait_vectors(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    unit: usize,
    leader_id: u32,
    dead: &mut Dead,
) -> bool {
    copy_trait_vector(
        job,
        memory,
        base,
        TraitVectorCopy { unit, leader_id },
        &mut dead.traits,
    )
}

pub(super) fn copy_pass(
    memory: &impl Memory,
    context: CharactersCopyContext,
    job: &mut CharactersJob,
    section: Option<Array>,
) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    job.advisor_demand = AdvisorDemand::default();
    job.appended_overflowed = false;
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin();
    job.game_state = context.game_state;
    job.census_current = false;
    job.role_census_current = false;
    job.advisor_section_complete = false;
    let hourly = roster_part(context.active);
    let bound = !hourly && job.source_bindings.is_some();
    if bound && !job.source_bindings.as_ref().unwrap().ready {
        job.log.invalidate_read_plan();
    }
    if context.active[0] {
        job.templates
            .begin(context.base, context.game_state, context.force_full);
    }
    if job.part == 0 && section.is_none() && !std::mem::take(&mut job.roster.carry_begun) {
        job.carry.begin_hour(
            context.active[0] && !context.force_full && cfg!(feature = "character_changes"),
        );
    }
    let key = section.map_or(
        [
            context.base,
            context.game_state,
            job.source_mask() as usize,
            job.templates.generation as usize
                ^ (job.carry.generation as usize).rotate_left(usize::BITS / 2),
        ],
        |array| {
            [
                context.base,
                context.game_state,
                array.data,
                array.count as usize,
            ]
        },
    );
    if context.active[3] && !context.active[5]
        || !(hourly || bound)
        || !job.log.begin_planned_capture(memory, key)
    {
        job.leader_rows = 0;
        job.character_demand = AdvisorDemand::default();
        job.unit_demand = AdvisorDemand::default();
        job.appended_roles.clear();
        job.appended_required = 0;
        if bound {
            job.source_bindings.as_mut().unwrap().begin();
        }
        let mut owned = job.portrait_census.take();
        let census = owned.as_mut().and_then(SharedBox::get_mut);
        copy_roster(
            memory,
            context.base,
            context.game_state,
            job,
            census,
            section,
        );
        copy_appended_roles(job, memory, context.base);
        job.carry.finish_pass(!job.overflowed());
        if bound {
            let complete = !job.overflowed()
                && !job.advisor_demand.overflowed
                && !job.log.fault.iter().any(|fault| *fault)
                && job
                    .traits
                    .vectors
                    .iter()
                    .all(|v| matches!(v.outcome, Outcome::Rows { .. }));
            let plan = job.source_bindings.as_mut().unwrap();
            plan.ready = complete && !plan.overflowed;
            plan.advisor = job.advisor_demand;
            plan.advisor_section_complete = job.advisor_section_complete;
        }
        job.census_matches_plan = job.census_current
            && (hourly
                || bound
                || (job.census_matches_plan
                    && owned.as_ref().is_some_and(|census| census.unchanged())));
        if !(hourly || bound || job.census_matches_plan) {
            job.log.invalidate_read_plan();
        }
        job.portrait_census = owned;
    } else {
        if bound {
            let plan = job.source_bindings.as_ref().unwrap();
            job.advisor_demand = plan.advisor;
            job.advisor_section_complete = plan.advisor_section_complete;
            for &source in &plan.sources {
                if job.traits.acquire(memory, context.base, source).is_err()
                    || job.traits.overflowed()
                {
                    break;
                }
            }
        }
        job.census_current = job.census_matches_plan
            && job
                .portrait_census
                .as_ref()
                .is_some_and(|census| census.rows(context.base, context.game_state).is_some());
    }
    job.log.finish_planned_capture();
    job.traits.finish();
}

pub(super) fn copy_roster(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut CharactersJob,
    mut census: Option<&mut ccharacter_manager::PortraitCensus>,
    section: Option<Array>,
) {
    if let Some(census) = census.as_mut() {
        census.begin(base, game_state);
    }
    job.game_state = game_state;
    let source_mask = job.source_mask();
    let mut dead = Dead {
        characters: !job.active[0],
        leaders: !job.active[1],
        advisors: !job.active[2],
        units: !job.active[3],
        traits: !job.active[4],
    };
    let arrays = if let Some(array) = section {
        [array, Array { data: 0, count: 0 }]
    } else {
        let Some(arrays) = copy_roster_headers(memory, base, game_state, job, source_mask) else {
            return;
        };
        arrays
    };
    if let Some(census) = census.as_mut() {
        census.expect(arrays[0].count as usize + arrays[1].count as usize);
        if census
            .grow()
            .map_err(collector_memory::terminal::io)
            .is_err()
        {
            return;
        }
        job.census_historical = arrays[0].count as usize;
    }
    copy_roster_entries(memory, base, game_state, job, census, arrays, &mut dead);
}

pub(super) fn copy_roster_headers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut CharactersJob,
    source_mask: u32,
) -> Option<[Array; 2]> {
    let manager_addr = game_state.checked_add(cgame_state::CHARACTER_MANAGER)?;
    let manager_start = job.log.take(memory, manager_addr, 8, source_mask)?;
    let manager = usize::try_from(pool_u64(job, manager_start))
        .ok()
        .filter(|&manager| manager != 0)?;
    let vtable_start = job.log.take(memory, manager, 8, source_mask)?;
    let expected = engine_sites::at(base, ccharacter_manager::VTABLE_RVA)? as u64;
    if pool_u64(job, vtable_start) != expected {
        return None;
    }
    let mut arrays = [Array { data: 0, count: 0 }; 2];
    for (slot, offset) in [
        ccharacter_manager::HISTORICAL_CHARACTERS,
        ccharacter_manager::DYNAMIC_CHARACTERS,
    ]
    .iter()
    .enumerate()
    {
        let addr = manager.checked_add(*offset)?;
        let header_start = job.log.take(memory, addr, 16, source_mask)?;
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&job.log.bytes[header_start..header_start + 16]);
        arrays[slot] = Array::from_header(raw, 8)?;
    }
    arrays[0].count.checked_add(arrays[1].count)?;
    Some(arrays)
}

pub(super) fn copy_roster_entries(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut CharactersJob,
    mut census: Option<&mut ccharacter_manager::PortraitCensus>,
    arrays: [Array; 2],
    dead: &mut Dead,
) {
    let source_mask = job.source_mask();
    for array in arrays.iter() {
        let mut ordinal = 0u32;
        while ordinal < array.count {
            let chunk = (array.count - ordinal).min(ROSTER_PAGE);
            let Some(page_addr) = array.element(ordinal, 8) else {
                return;
            };
            let Some(page_start) = job
                .log
                .take(memory, page_addr, chunk as usize * 8, source_mask)
            else {
                return;
            };
            for bytes in job.log.bytes[page_start..page_start + chunk as usize * 8].chunks_exact(8)
            {
                let Some(pointer) = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                else {
                    break;
                };
                let carried = job.carry.carries(pointer);
                for (index, (offset, len)) in VIEW_OFFSETS.into_iter().zip(VIEW_LENS).enumerate() {
                    if VIEW_MASKS[index] & source_mask == 0
                        || (carried && CARRIED_SPANS.contains(&index))
                    {
                        continue;
                    }
                    if let Some(address) = pointer.checked_add(offset) {
                        job.log.prefetch(address, len);
                    }
                }
            }
            let mut views: [Option<ViewCopy>; ROSTER_PAGE as usize] = std::array::from_fn(|_| None);
            let mut staged = 0;
            let mut terminal = false;
            for index in 0..chunk {
                let at = page_start + index as usize * 8;
                let pointer = usize::try_from(u64::from_le_bytes(
                    job.log.bytes[at..at + 8].try_into().unwrap(),
                ))
                .ok()
                .filter(|&pointer| pointer != 0);
                let Some(pointer) = pointer else {
                    terminal = true;
                    break;
                };
                let carried = job.carry.carries(pointer);
                if job.carry.active {
                    job.carry.visit(pointer, carried);
                }
                let Some(view) = copy_view_carrying(job, memory, base, pointer, carried) else {
                    terminal = true;
                    break;
                };
                if job.log.overflowed {
                    return;
                }
                if view.starts[1] != u32::MAX
                    && let Some(census) = census.as_mut()
                {
                    let identity = view.field(job, 0);
                    census.push(ccharacter_manager::PortraitIdentity {
                        pointer,
                        character: u32::from_le_bytes(
                            identity[ccharacter::ID..ccharacter::ID + 4]
                                .try_into()
                                .unwrap(),
                        ),
                    });
                }
                hint_character_targets(job, &view, dead, |address, len| {
                    job.log.prefetch(address, len)
                });
                views[staged] = Some(view);
                staged += 1;
            }
            if job.log.overflowed {
                return;
            }
            for (index, view) in views[..staged].iter_mut().enumerate() {
                let at = page_start + index * 8;
                let pointer =
                    u64::from_le_bytes(job.log.bytes[at..at + 8].try_into().unwrap()) as usize;
                if !copy_character_body(
                    job,
                    memory,
                    base,
                    pointer,
                    view.as_mut().expect("staged character view"),
                    dead,
                ) {
                    return;
                }
            }
            if terminal {
                return;
            }
            ordinal += chunk;
        }
    }
    job.advisor_section_complete = !dead.advisors && !job.log.fault[2] && !job.overflowed();
    if !job.overflowed()
        && let Some(census) = census
    {
        census.finish(arrays[0].count as usize + arrays[1].count as usize);
        job.census_current = census.rows(base, game_state).is_some();
    }
}

pub(super) fn census_part(active: [bool; N]) -> bool {
    let leaders = active[1] && !active[4];
    let traits = !active[1] && active[4];
    !active[0] && !active[2] && !active[3] && (leaders || traits)
}

pub(super) fn gather_role_census(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
) {
    if !job.census_current {
        return;
    }
    let _reads = read_accounting::table(TABLES[anchor(ctx.active)]);
    let scope = (ctx.base, ctx.game_state);
    let portraits = job.portrait_census.as_ref();
    let rows = portraits.and_then(|census| census.rows(ctx.base, ctx.game_state));
    let stamp = portraits.map_or(0, |census| census.stamp());
    let roles = job.role_census.as_mut().and_then(SharedBox::get_mut);
    if let (Some(rows), Some(roles)) = (rows, roles) {
        let (admitted, counts) =
            job.census_carry
                .acquire(memory, roles, scope, rows, stamp, job.census_historical);
        job.role_census_current = admitted;
        if admitted && counts.gathered {
            job.log.raw_bytes[0] += (rows.len() * (census::ROW - 4)) as u64;
        }
        collector_observers::census_changes::record(&counts);
    }
}

pub(super) fn copy_census(
    memory: &impl Memory,
    context: CharactersCopyContext,
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    job.leader_rows = 0;
    job.advisor_demand = AdvisorDemand::default();
    job.appended_roles.clear();
    job.appended_required = 0;
    job.appended_overflowed = false;
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin();
    job.game_state = context.game_state;
    job.census_current = false;
    job.role_census_current = false;
    job.advisor_section_complete = false;
    job.census_capture = true;
    job.census_visited.clear();
    job.census_required = 0;
    job.census_historical = census.historical;
    job.census_generation = census.generation;
    let mut dead = Dead {
        characters: true,
        leaders: !context.active[1],
        advisors: true,
        units: true,
        traits: !context.active[4],
    };
    let bindings = job.source_bindings.take();
    if context.active[1] {
        copy_census_leaders(job, memory, context.base, census, &mut dead);
    } else {
        copy_census_units(job, memory, context.base, census, &mut dead);
    }
    copy_appended_roles(job, memory, context.base);
    job.source_bindings = bindings;
    job.traits.finish();
}

pub(super) fn visit_census(
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
    index: usize,
    needs: impl Fn(census::Occurrence) -> bool,
) -> bool {
    if reserve(&mut job.census_visited, 1).is_err() {
        let remaining = (index..census.len())
            .filter(|&index| needs(census.occurrence(index)))
            .count();
        job.census_required = job.census_visited.len() + remaining.max(1);
        return false;
    }
    job.census_visited.push(census.occurrence(index).visited());
    true
}

pub(super) fn copy_census_leaders(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    census: census::Admitted<'_>,
    dead: &mut Dead,
) {
    use census::{SENTINEL, SENTINEL_BATCH};
    let needs = |occurrence: census::Occurrence| {
        census::map_head(occurrence.leaders()).is_none_or(|(_, count)| count != 0)
    };
    let omissible = engine_sites::at(base, engine_native_parts::ccountry_leader_layout::VTABLE_RVA).is_some();
    let mut heads = [0; SENTINEL_BATCH];
    let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
    let mut next = 0;
    while next < census.len() && !dead.leaders {
        let mut staged = 0;
        while staged < SENTINEL_BATCH && next + staged < census.len() {
            let occurrence = census.occurrence(next + staged);
            match census::map_head(occurrence.leaders()) {
                Some((head, _)) if !census::inside_view(occurrence.pointer, head, SENTINEL) => {
                    heads[staged] = head;
                }
                _ => break,
            }
            staged += 1;
        }
        let completed = census::gather_sentinels(memory, &heads[..staged], &mut sentinels);
        job.log.raw_bytes[1] += (completed * SENTINEL) as u64;
        let end = if staged == SENTINEL_BATCH {
            next + staged
        } else {
            census.len().min(next + staged + 1)
        };
        for index in next..end {
            if dead.leaders {
                return;
            }
            let occurrence = census.occurrence(index);
            let header = occurrence.leaders();
            let slot = index - next;
            let gathered = &sentinels[slot * SENTINEL..][..SENTINEL];
            let sentinel = if slot < staged {
                Sentinel::Gathered((slot < completed).then_some(gathered))
            } else if census::map_head(header).is_some() {
                Sentinel::Owned
            } else {
                Sentinel::Read
            };
            if omissible
                && let Sentinel::Gathered(Some(bytes)) = sentinel
                && let Some((head, 0)) = census::map_head(header)
                && tree_sentinel(head, 0, bytes).is_some()
            {
                continue;
            }
            if !visit_census(job, census, index, needs)
                || !copy_leaders(
                    job,
                    memory,
                    base,
                    |_| header,
                    occurrence.pointer,
                    sentinel,
                    dead,
                )
            {
                return;
            }
        }
        next = end;
    }
}

pub(super) fn copy_census_units(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    census: census::Admitted<'_>,
    dead: &mut Dead,
) {
    const LOOKAHEAD: usize = 16;
    let unit = |occurrence: census::Occurrence| {
        usize::try_from(u64::from_le_bytes(occurrence.unit()))
            .ok()
            .filter(|&unit| unit != 0)
    };
    let needs = |occurrence: census::Occurrence| unit(occurrence).is_some();
    job.trait_carry.begin_hour();
    if !dead.traits && dead.units {
        job.trait_carry
            .certify(census.generation, census.len(), || quiet::leaders(census));
    }
    for index in 0..census.len() {
        if dead.traits {
            return;
        }
        if let Some(ahead) = (index + LOOKAHEAD < census.len())
            .then(|| unit(census.occurrence(index + LOOKAHEAD)))
            .flatten()
            .filter(|&ahead| !job.trait_carry.carries(ahead))
        {
            for offset in [0, cunit_leader::TRAIT_DATA] {
                if let Some(address) = ahead.checked_add(offset) {
                    memory.prefetch(address, 16);
                }
            }
        }
        let occurrence = census.occurrence(index);
        let Some(pointer) = unit(occurrence) else {
            continue;
        };
        if !visit_census(job, census, index, needs) {
            return;
        }
        if job.trait_carry.carries(pointer) {
            continue;
        }
        let id = occurrence.id();
        let mut unit_id = None;
        if !copy_unit(job, memory, base, Some(pointer), occurrence.pointer, |_| id, &mut unit_id, dead)
            || !copy_trait_vectors(job, memory, base, pointer, unit_id.unwrap_or(id), dead)
        {
            return;
        }
    }
}

pub(super) fn copy_advisor_sections(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
) {
    use std::cell::RefCell;
    struct Roots<'a, M> {
        memory: &'a M,
        log: RefCell<&'a mut collector_raw::raw::RawLog<N>>,
    }
    impl<M: Memory> Memory for Roots<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut log = self.log.borrow_mut();
            let Some(start) = log.take(self.memory, address, output.len(), MASK_ADVISORS) else {
                return false;
            };
            output.copy_from_slice(&log.bytes[start..start + output.len()]);
            true
        }
    }
    debug_assert_eq!(ctx.active, PART_MASKS[2]);
    let _reads = read_accounting::table(TableId::AdvisorRoles);
    job.clear();
    job.active = ctx.active;
    job.game_state = ctx.game_state;
    let roster = ccharacter_manager::Roster::read(
        &Roots {
            memory,
            log: RefCell::new(&mut job.advisor_roots),
        },
        ctx.base,
        ctx.game_state,
    );
    let Ok(roster) = roster else {
        return;
    };
    let mut dynamic = job.advisor_dynamic.take().expect("advisor section storage");
    for (index, (array, _)) in roster.sections().into_iter().enumerate() {
        if index == 1 && !job.advisor_section_complete {
            break;
        }
        let part = if index == 0 { &mut *job } else { &mut *dynamic };
        copy_pass(memory, ctx, part, Some(array));
    }
    job.advisor_dynamic = Some(dynamic);
}

pub(super) fn walk_advisor_section(
    memory: &impl Memory,
    base: usize,
    roster: ccharacter_manager::Roster,
    advisors: &mut Batch<'_, cadvisor::Row>,
) {
    let result = (|| {
        let mut cursor = roster.cursor();
        let mut page = ccharacter_manager::ViewPage::default();
        while advisors.active() {
            let view = loop {
                if let Some(view) = page.next() {
                    break Some(view?);
                }
                if !cursor.refill_page(memory, &mut page, [false, false, false, true])? {
                    break None;
                }
            };
            let Some(view) = view else {
                break;
            };
            let entry = view.entry()?;
            advisors.append(
                |rows, bytes| cadvisor::walk_prefix_entry(&view, base, entry, rows, bytes).into(),
                |row, offset| {
                    relocate(&mut row.key_start, offset)
                        && relocate(&mut row.idea_start, offset)
                        && relocate(&mut row.slot_start, offset)
                        && relocate(&mut row.modifier_start, offset)
                },
            );
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        advisors.fail(reason);
    }
}
