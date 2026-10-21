use super::*;
use engine_allocation::{owned_vec, reserve, vec};

/// One refresh of raw character payload. Trait occurrences reference owned,
/// verified definitions; all other observations retain the raw replay order.
#[derive(Debug, Default)]
pub(crate) struct CharactersJob {
    pub(super) worker_growing: bool,
    pub(super) capture_growing: bool,
    pub(super) leader_rows: usize,
    pub(super) character_demand: AdvisorDemand,
    pub(super) unit_demand: AdvisorDemand,
    pub part: usize,
    pub log: crate::raw::RawLog<N>,
    pub game_state: usize,
    pub active: [bool; N],
    pub(super) traits: TraitCapture,
    pub(super) advisor_demand: AdvisorDemand,
    // Authenticated roles whose appended fields are copied after every old
    // field. Worker growth follows the authenticated roles; callback storage
    // records shortage for idle growth.
    pub(super) appended_roles: Vec<AppendedRole>,
    pub(super) appended_required: usize,
    pub(super) appended_overflowed: bool,
    pub(super) appended_charge: crate::raw_memory::Charge,
    pub(super) skills: [SkillCopy; 32],
    pub(super) portrait_census: Option<Arc<ccharacter_manager::PortraitCensus>>,
    pub(super) census_current: bool,
    pub(super) census_matches_plan: bool,
    pub(super) source_bindings: Option<CharacterSourceBindings>,
    // Advisor-only jobs authenticate both headers once, then acquire the two
    // rosters sequentially with independent fresh-read programs. The child has
    // no child of its own; neither root authentication nor values are cached.
    pub(super) advisor_roots: crate::raw::RawLog<N>,
    pub(super) advisor_dynamic: Option<Box<CharactersJob>>,
    pub(super) advisor_cache: Option<advisors::Cache>,
    pub(super) advisor_section_complete: bool,
    /// The leader role job's held rows ([`leaders::LeaderCache`]); its
    /// payload came from them when `leader_cached`.
    pub(super) leader_cache: Option<leaders::LeaderCache>,
    pub(super) leader_cached: bool,
    pub(super) leader_charge: crate::raw_memory::Charge,
    /// Characters rows the roster job carries ([`carry::Carry`]).
    pub(super) carry: carry::Carry,
    pub(super) carry_charge: crate::raw_memory::Charge,
    /// The roster job's held roster token and quiet hours ([`quiet`]).
    pub(super) roster: quiet::QuietRoster,
    /// The Characters rows of this job's latest quiet decode.
    pub(super) published: quiet::Published,
    /// UnitLeaders rows held between quiet roster hours.
    pub(super) unit_carry: leader_carry::UnitCarry,
    pub(super) experience: daily_experience::Store,
    pub(super) daily_training: Option<bool>,
    /// UnitLeaderTraits rows held between the traits job's captures.
    pub(super) trait_carry: leader_carry::TraitCarry,
    /// The role census kept between gathers.
    pub(super) census_carry: census::CensusCarry,
    pub(super) census_charge: crate::raw_memory::Charge,
    // The roster job gathers the role census on role hours. It persists
    // like the portrait census; the flag admits only this pass's gather.
    pub(super) role_census: Option<Arc<census::RoleCensus>>,
    pub(super) role_census_current: bool,
    /// Historical roster length of the retained portrait census, or of the
    /// loaned census this payload was acquired from.
    pub(super) census_historical: usize,
    /// The generation of the census a role job's capture borrowed; zero for
    /// none.
    pub(super) census_generation: u64,
    /// A role payload acquired from a loaned census, the occurrences its
    /// decode visits, and the demand recorded when that storage ran out.
    pub(super) census_capture: bool,
    pub(super) census_visited: Vec<census::Occurrence>,
    pub(super) census_required: usize,
    /// Template identities this job's earlier decodes read. It survives
    /// `clear`: the copy consults it, the decode rebuilds it.
    pub(super) templates: TemplateCache,
    pub(super) template_charge: crate::raw_memory::Charge,
}

/// Auxiliary bindings whose selecting character/unit bytes are controls in the
/// raw read program. Only addresses and capacity demand survive a refresh;
/// trait vectors and definitions are acquired on every refresh.
#[derive(Debug)]
pub(super) struct CharacterSourceBindings {
    pub(super) sources: Vec<TraitVectorCopy>,
    pub(super) overflowed: bool,
    pub(super) ready: bool,
    pub(super) rows: usize,
    pub(super) bytes: usize,
    pub(super) advisor: AdvisorDemand,
    pub(super) advisor_section_complete: bool,
    pub(super) charge: crate::raw_memory::Charge,
}

impl CharacterSourceBindings {
    pub(super) fn new() -> Self {
        let mut value = Self {
            sources: owned_vec(vec(census::initial_len())),
            overflowed: false,
            ready: false,
            rows: 0,
            bytes: 0,
            advisor: AdvisorDemand::default(),
            advisor_section_complete: false,
            charge: crate::raw_memory::Charge::default(),
        };
        value.update_charge();
        value
    }

    pub(super) fn update_charge(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.sources),
            ..Default::default()
        });
    }

    pub(super) fn matches(&self, context: CharactersCopyContext) -> bool {
        self.ready && self.rows == context.trait_rows && self.bytes == context.trait_bytes
    }

    pub(super) fn begin(&mut self, context: CharactersCopyContext) {
        self.sources.clear();
        self.overflowed = false;
        self.ready = false;
        self.rows = context.trait_rows;
        self.bytes = context.trait_bytes;
    }

    pub(super) fn remember(&mut self, source: TraitVectorCopy, growing: bool) {
        if growing {
            let capacity = self.sources.capacity();
            if reserve(&mut self.sources, 1).is_err() {
                self.overflowed = true;
                return;
            }
            if self.sources.capacity() != capacity {
                self.update_charge();
            }
        }
        if self.sources.len() == self.sources.capacity() {
            self.overflowed = true;
        } else {
            self.sources.push(source);
        }
    }

    pub(super) fn grow(&mut self) {
        if self.overflowed {
            let _ = engine_allocation::reserve(&mut self.sources, 1);
            self.update_charge();
        }
    }
}

/// Successful shared skill definitions, retained only for this acquisition.
/// Experience changes select skill definitions from the category/level
/// database and can change the leader's selection.
/// Collisions simply acquire again. Every use still appends its scoped raw
/// observations so unrelated aliases and their faults keep separate cursors.
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
        // Owned bytes need no native-address prefetch.
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
}

impl CharactersJob {
    /// Whether recycling keeps the job. A leader role job that carried its
    /// rows from the census copied no raw log, but its held rows are its
    /// storage: dropping it would start every later hour cold.
    pub(crate) fn has_storage(&self) -> bool {
        self.worker_growing
            || self.advisor_cache.is_some()
            || self.leader_cache.is_some()
            || self.log.spans.capacity() != 0
    }

    /// A pool miss's empty job, allocation-free: its part's caches that
    /// need no storage until a worker fills them.
    pub(crate) fn vacant(part: usize) -> Self {
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
    #[cfg(test)]
    pub fn new() -> Self {
        let mut job = Self::with_storage(0, 1 << 19, 24 << 20, true);
        job.traits = TraitCapture::with_test_capacities(4096, 4096, 4096, 1 << 20);
        job.source_bindings = Some(super::tests::fixture_bindings());
        job
    }

    pub(super) fn with_storage(part: usize, spans: usize, bytes: usize, has_traits: bool) -> Self {
        let mut log = crate::raw::RawLog::new(spans, bytes);
        let trace_capacity = spans.min(1 << 18);
        log.initialize_scattered_read_plan(trace_capacity, trace_capacity, bytes.min(4 << 20));
        let census = (part == 0)
            .then(|| {
                ccharacter_manager::PortraitCensus::try_new(census::initial_len())
                    .map_err(crate::allocation::io)
                    .ok()
            })
            .flatten();
        let role_census = (part == 0).then(|| census::RoleCensus::new(census::initial_len()));
        // A pass visits at most every roster occurrence. Attach/cadence hints
        // seed the list; original acquired occurrences grow worker storage.
        let census_visited = owned_vec(vec(if part == 0 { 0 } else { census::initial_len() }));
        let mut census_charge = crate::raw_memory::Charge::default();
        census_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: census
                .as_ref()
                .map_or(0, |census| census.allocated_bytes() as u64)
                + role_census
                    .as_ref()
                    .map_or(0, |census| census.allocated_bytes() as u64)
                + crate::raw_memory::vector_bytes(&census_visited),
            ..Default::default()
        });
        // Live role counts seed the suffix queue with raw-span headroom.
        // Worker acquisition reserves new occurrences before appending.
        let appended_roles = owned_vec(vec(demand::initial_roles(part).max(spans / 8)));
        let mut appended_charge = crate::raw_memory::Charge::default();
        appended_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&appended_roles),
            ..Default::default()
        });
        Self {
            worker_growing: false,
            capture_growing: false,
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
            portrait_census: census.map(Arc::new),
            census_current: false,
            census_matches_plan: false,
            source_bindings: Some(CharacterSourceBindings::new()),
            advisor_roots: crate::raw::RawLog::default(),
            advisor_dynamic: None,
            advisor_cache: None,
            advisor_section_complete: false,
            leader_cache: None,
            leader_cached: false,
            leader_charge: crate::raw_memory::Charge::default(),
            carry: carry::Carry::default(),
            carry_charge: crate::raw_memory::Charge::default(),
            roster: quiet::QuietRoster::default(),
            published: quiet::Published::default(),
            unit_carry: leader_carry::UnitCarry::default(),
            experience: daily_experience::Store::default(),
            daily_training: None,
            trait_carry: leader_carry::TraitCarry::default(),
            census_carry: census::CensusCarry::default(),
            census_charge,
            role_census: role_census.map(Arc::new),
            role_census_current: false,
            census_historical: 0,
            census_generation: 0,
            census_capture: false,
            census_visited,
            census_required: 0,
            templates: TemplateCache::default(),
            template_charge: crate::raw_memory::Charge::default(),
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
        job.worker_growing = true;
        if part == 1 && cfg!(feature = "character_changes") {
            job.leader_cache = Some(leaders::LeaderCache::default());
        }
        if part == 2 {
            job.advisor_roots = crate::raw::RawLog::new(4, 48);
            job.advisor_dynamic = Some(Box::new(Self::with_storage(part, 0, 0, false)));
        }
        job
    }

    pub fn clear(&mut self) {
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
        self.advisor_roots.clear();
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.clear();
        }
    }

    pub(crate) fn portrait_census(&self) -> Option<Arc<ccharacter_manager::PortraitCensus>> {
        self.census_current
            .then(|| self.portrait_census.clone())
            .flatten()
    }

    /// This pass's role census; role jobs return the loan at their join.
    pub(crate) fn role_census(&self) -> Option<Arc<census::RoleCensus>> {
        self.role_census_current
            .then(|| self.role_census.clone())
            .flatten()
    }

    pub(super) fn update_census_charge(&mut self) {
        self.census_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: self
                .portrait_census
                .as_ref()
                .map_or(0, |census| census.allocated_bytes() as u64)
                + self
                    .role_census
                    .as_ref()
                    .map_or(0, |census| census.allocated_bytes() as u64)
                + crate::raw_memory::vector_bytes(&self.census_visited)
                + self.roster.allocated_bytes()
                + self.census_carry.allocated_bytes() as u64,
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

    /// Visits the next role pass may need: its recorded demand, and every
    /// occurrence of the live census (`live`).
    pub(super) fn visits_required(&self, live: usize) -> usize {
        if self.part == 0 {
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
        // Every portrait lease returns before recycle. The grown census was
        // incomplete, so retire any prepared program: the next pass must
        // rediscover the roster to publish it.
        if let Some(census) = self.portrait_census.as_mut().and_then(Arc::get_mut)
            && census.needs_growth()
        {
            if census.grow().map_err(crate::allocation::io).is_err() {
                return;
            }
            self.log.invalidate_read_plan();
        }
        // Role loans also return before recycle.
        if let Some(census) = self.role_census.as_mut().and_then(Arc::get_mut) {
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
            // A prefix-only program from a short queue has no requests for
            // the omitted fields. Rediscover them after idle growth.
            self.log.invalidate_read_plan();
        }
        self.appended_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.appended_roles),
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

    pub(crate) fn prepare_read_plans(&mut self) {
        if let Some(cache) = &mut self.advisor_cache {
            cache.prepare();
        }
        self.log.prepare_read_plan();
        if let Some(dynamic) = &mut self.advisor_dynamic {
            dynamic.prepare_read_plans();
        }
    }

    pub(crate) fn note_growth(&self) {
        crate::raw::note_growth("characters", &self.log);
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
        if self.worker_growing {
            if reserve(&mut self.appended_roles, 1).is_err() {
                self.appended_overflowed = true;
                return;
            }
            self.appended_charge.set(crate::raw_memory::Capacity {
                auxiliary_vectors: crate::raw_memory::vector_bytes(&self.appended_roles),
                ..Default::default()
            });
        }
        if self.appended_roles.len() < self.appended_roles.capacity() {
            self.appended_roles.push(role);
        }
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
pub(crate) struct CharactersCopyReport {
    /// False when the existing field prefix exceeded staging. A short
    /// appended suffix keeps the prefix payload and refuses only that field.
    pub raw_ok: bool,
    /// A stopped prepared pass may not fall back to another native capture.
    pub plan_failed: bool,
    pub copy_ticks: [u64; N],
    pub raw_bytes: [u64; N],
}

/// Acquisition parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CharactersCopyContext {
    pub base: usize,
    pub game_state: usize,
    /// A full hour reads every character template again.
    pub force_full: bool,
    pub active: [bool; N],
    pub trait_rows: usize,
    pub trait_bytes: usize,
}

/// Per-batch failure flags mirroring `Batch::active`: once a typed batch
/// fails, its later appends are skipped without reads, and so are the
/// mirror's later sub-copies.
#[derive(Debug, Default)]
pub(super) struct Dead {
    pub(super) characters: bool,
    pub(super) leaders: bool,
    pub(super) advisors: bool,
    pub(super) units: bool,
    pub(super) traits: bool,
}

/// Successful view fields remain in the current capture's append-only raw
/// arena. Each roster occurrence owns its offsets; aliases never share them.
#[derive(Clone, Copy)]
pub(super) struct ViewCopy {
    pub(super) starts: [u32; 8],
    pub(super) unit_id: Option<u32>,
    /// The roster job carries this character's Characters row: its
    /// name-token span, names and gender were not copied.
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
    /// Failed and inactive spans retain the old zero-field semantics. Borrow
    /// only while parsing; later raw appends may move the arena's allocation.
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

/// Mirror of `View::read`'s scatter plus single-resume sequence with no
/// decoding. Returns `None` only when the identity span fails, which aborts
/// the whole copy exactly like the typed cursor error.
#[cfg(test)]
pub(super) fn copy_view(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Option<ViewCopy> {
    copy_view_carrying(job, memory, base, pointer, false)
}

/// The Characters-only spans a carried character omits: the name-token span
/// (which holds the template pointer), both name headers and the gender.
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
            // RawLog admits start + len <= u32::MAX; a successful nonempty
            // field's start therefore cannot equal the absent marker.
            view.starts[index] = start as u32;
        }
    }
    if completed == 0 {
        return None;
    }
    // Resume only later independent fields, each at most once; the failed
    // field is never retried.
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
    // Local vtable and reference-type authentication, checked after the
    // scatter exactly like the typed view.
    let expected = base.checked_add(ccharacter::VTABLE_RVA)? as u64;
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

/// Parsed native string header shared by both string domains: payload length,
/// payload source address and inline flag. `None` means the typed walk fails
/// the enclosing row with its own header reason from the same header bytes.
pub(super) struct Lit {
    pub(super) len: usize,
    pub(super) source: usize,
    pub(super) inline: bool,
}

/// Hint only the first line of a potential upcoming read. The guard is
/// advisory: rejecting a hint never changes the later guarded read or error.
#[inline]
pub(super) fn hint_character_target(
    address: usize,
    len: usize,
    hint: &mut impl FnMut(usize, usize),
) {
    if address != 0
        && address.checked_add(len).is_some()
        && crate::raw::is_canonical_range(address, len)
    {
        hint(address, 1);
    }
}

/// These pointers come only from the fresh, authenticated character view.
/// Targets remain unauthenticated until their original reads below. Empty
/// maps still read their sentinel; failed view fields contain only zeros.
#[inline]
pub(super) fn hint_character_targets(
    job: &CharactersJob,
    view: &ViewCopy,
    dead: &Dead,
    mut hint: impl FnMut(usize, usize),
) {
    if !dead.characters {
        // The page's views are staged before its bodies. Hint heap names now,
        // so their independent cache misses overlap the remaining view reads.
        // Inline sources need no hint; their header address is immaterial here.
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
        && crate::raw::is_canonical_range(unit, 16)
    {
        hint_character_target(unit, 16, &mut hint);
        if !dead.units {
            // These fixed base fields are independent of the leader subclass.
            // Skill, XP and category occupy one small span. Do not follow
            // the skill pointer before its ordinary validation.
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

/// Called only after both local string parses succeeded. Inline bytes are
/// already owned. Heap strings need their terminator even when empty; hint
/// a separate payload-first line only when it differs from the terminator.
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
        || !crate::raw::is_canonical_range(literal.source, extent)
    {
        return;
    }
    let terminator = literal.source + literal.len;
    hint(terminator, 1);
    if literal.len != 0 && literal.source >> 6 != terminator >> 6 {
        hint(literal.source, 1);
    }
}

/// Mirror of `ccharacter::Literal::from_header` (character name and token).
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

/// Mirror of `cunit_leader`'s string header validation: heap strings read
/// their terminator (and payload when nonempty) from live memory, inline
/// strings are served from the already-copied header.
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

/// Heap payload and terminator spans for one parsed character-domain string.
/// Inline strings need no live reads beyond the copied header.
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

/// The original terminator-then-payload observations share one guard. Tight
/// staging uses the original scalar prefix so capacity cannot hide a fault.
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

/// Traversal state mirroring `msvc_tree_node::Cursor`'s read set: the mirror
/// issues the identical sentinel, link and node reads in the identical
/// successor order, parsing link values locally to choose branches.
pub(super) struct TreeWalk {
    pub(super) head: usize,
    pub(super) next: usize,
    pub(super) remaining: u32,
    pub(super) count: u32,
}

/// Abort the whole copy (`false`) or kill only this map's batch (`true`).
pub(super) enum TreeFate {
    Abort,
    Dead,
}

/// How a map's sentinel observation is acquired.
#[derive(Clone, Copy)]
pub(super) enum Sentinel<'a> {
    /// Read it now.
    Read,
    /// A batch already gathered it; `None` is that read's failure.
    Gathered(Option<&'a [u8]>),
    /// It lies inside the character view, which fails it without a read.
    Owned,
}

/// Mirror of `RoleTree::read`: sentinel plus size/coherence parses.
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

/// `RoleTree::read`'s checks on an acquired sentinel; returns the first node.
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

/// Mirror of `RoleTree::next`: the identical bounded successor sequence,
/// returning native node addresses with no decoding.
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

/// One mirrored span read: `None` kills the enclosing batch. The caller maps
/// `None` to a whole-copy abort when the job overflowed, matching the staged
/// fallback rule.
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

/// Fixed scalar sequences with either prefix failure or independent-field
/// continuation. Tight storage and absent checked addresses use the scalar
/// path before any reads, preserving its exact retained prefix and growth.
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
            // The failed field is already recorded. Resume only later
            // independent fields, without retrying the failed observation.
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

/// Mirror of `ccharacter::walk_entry`'s string reads: header parses are local
/// (the headers ride in the view span), heap payloads and terminators are
/// copied by length. Any failure kills only the characters batch.
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
    // The staged view already hinted both heap strings before this page's
    // bodies began. Repeating those hints here adds no useful read lead.
    if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
        || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
    {
        dead.characters = true;
    }
}

/// Mirror of `ccharacter::walk_entry`'s template reads after both names: the
/// pointer rides in the view's name token span; a pointer this job already
/// holds an identity for reads nothing, exactly as the decode then supplies
/// it. Otherwise the header and generated byte in one scatter, then the file's
/// heap terminator and bytes. Any failure kills only the characters batch.
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
    let Some(expected) = base.checked_add(ccharacter_template::VTABLE_RVA) else {
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

/// Mirror of `ccountry_leader::walk_entry`'s read set: the role-map traversal
/// plus one ideology/role-header pair per node. Order, duplicate and auth
/// checks are local parses; any failure kills only the leaders batch.
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
    let expected = match base.checked_add(ccountry_leader::VTABLE_RVA) {
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
    // Retain the full ordered leader map.
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
            || pool_u64(job, header + ccountry_leader::CHARACTER) != pointer as u64
        {
            dead.leaders = true;
            return true;
        }
        job.remember_appended(AppendedRole::Leader(role.unwrap()));
    }
    true
}

/// Exact advisor field prefix. Address overflow keeps the scalar mirror's
/// earlier observations; the normal path uses one guard for the same spans.
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

/// Mirror of `cadvisor::walk_entry`'s read set: the role-map traversal plus
/// node, role and key/idea/slot string spans per node. Hash order and auth
/// checks are local parses; any failure kills only the advisors batch.
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
    let expected = match base.checked_add(cadvisor::VTABLE_RVA) {
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
        // Include the failing attempt so a short destination cannot hide its
        // recorded successor/body failure. Do not allocate from an unvisited
        // tree tail merely because its header advertises a large count.
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
                // The first heap read is its terminator. These failures occur
                // before the typed walk checks output capacity, so no byte
                // allocation is needed to retain their original failure.
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
        let Some(array) = Array::from_header(header, engine_native::modifier_entry::STRIDE) else {
            dead.advisors = true;
            return true;
        };
        let required = (array.count as usize)
            .checked_mul(engine_native::modifier_entry::STRIDE)
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
                array.element(first, engine_native::modifier_entry::STRIDE),
                count * engine_native::modifier_entry::STRIDE,
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

/// Authenticate a unit leader header locally, mirroring `cunit_leader`'s
/// identity read. Returns the kind and leader id for the copy below.
pub(super) fn parse_unit_identity(
    job: &CharactersJob,
    base: usize,
    start: usize,
) -> Option<(u8, u32)> {
    let vtable = pool_u64(job, start);
    let relative = usize::try_from(vtable).ok()?.checked_sub(base)?;
    let kind = match relative {
        cunit_leader::VTABLE_RVA => cunit_leader::KIND_BASE,
        carmy_leader::VTABLE_RVA => cunit_leader::KIND_ARMY,
        cnavy_leader::VTABLE_RVA => cunit_leader::KIND_NAVY,
        coperative_leader::VTABLE_RVA => cunit_leader::KIND_OPERATIVE,
        _ => return None,
    };
    if pool_u32(job, start + cunit_leader::TYPE) != cunit_leader::REFERENCE_TYPE {
        return None;
    }
    Some((kind, pool_u32(job, start + cunit_leader::ID)))
}

/// Mirror of `read_unit_selected` plus `cunit_leader::read`/`read_text`: identity,
/// state, skill, kind components and both text strings. The identity-fallback
/// spans are always recorded so the trait replay finds them when the scalar
/// read fails. Any failure kills only the units batch.
pub(super) fn copy_unit(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    unit: Option<usize>,
    character_id: impl FnOnce(&CharactersJob) -> u32,
    unit_id: &mut Option<u32>,
    dead: &mut Dead,
) -> bool {
    let Some(unit) = unit else {
        // Null role pointers are handled by the caller: no rows, no reads.
        return true;
    };
    if dead.units && dead.traits {
        // The typed walk `continue`s without reads once both batches are
        // dead, and so does the mirror.
        return true;
    }
    // The replay calls the identity fallback while its traits batch is still
    // alive at this character, which is exactly the entry state here: later
    // kills in this function happen after the replay's call point, so they
    // must not close this gate.
    let need_fallback = !dead.traits;
    // Scalar and trait identities are independent native observations. The
    // trait acquisition authenticates its own identity even after a scalar
    // failure, provided the fallback can still resolve the parent handle.
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
        // State scatter in `cunit_leader::read` order: character backlink,
        // category, skill pointer, exile country and residual experience.
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
            // These are independent registries: a leader's id is not the
            // id of the character whose role points to it.
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
    // Identity-fallback spans for the trait replay. Recorded whenever the
    // replay could still call the fallback (entry gate above); a live
    // failure here kills only traits, exactly like the typed fallback
    // error, and never the units outcome.
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

/// Mirror of the skill block in `cunit_leader::read`. Kills only the units
/// batch; the vtable predicate is a local parse.
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
            let skill_expected = match base.checked_add(cunit_leader_skill::VTABLE_RVA) {
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
                // These raw occurrences are owned copies, not native reads.
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
            // The typed walk authenticates the vtable from the first span
            // and the remaining fields from the later spans; a completed
            // zero prefix still fails here with the vtable reason.
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

/// Mirror of the kind-component scatter in `cunit_leader::read`.
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
    // Then `cunit_leader::read`'s owner and visibility scatter, with the
    // army leader's captured byte.
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

/// Mirror of `cunit_leader::read_text`: both string headers plus heap
/// payloads and terminators by header length.
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

/// Records independent appended-reader spans and their first failed read.
/// An unavailable suffix does not kill the role's existing field batch.
struct AppendedCopy<'a, M> {
    job: std::cell::RefCell<&'a mut CharactersJob>,
    memory: &'a M,
    mask: u32,
}

/// Appended reads cannot consume staging needed by a later existing row.
/// A short suffix leaves the completed prefix admitted; the raw log retains
/// its actual overflow and growth demand, and missing owned reads refuse only
/// the corresponding appended field during decode.
pub(super) fn copy_appended_roles(job: &mut CharactersJob, memory: &impl Memory, base: usize) {
    if job.overflowed() || job.log.planned_capture_failed() {
        return;
    }
    for index in 0..job.appended_roles.len() {
        let role = job.appended_roles[index];
        let source = AppendedCopy {
            job: std::cell::RefCell::new(&mut *job),
            memory,
            mask: match role {
                AppendedRole::Leader(_) => MASK_LEADERS,
                AppendedRole::Advisor(_) => MASK_ADVISORS,
                AppendedRole::Unit { .. } => MASK_UNITS,
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
    // Appended naming faults are replayed as a local refusal by read_unit;
    // they do not kill the existing unit fields or the other leader rows.
    let member = unit
        .checked_add(coperative_leader::CODENAME)
        .and_then(|member| job.log.take(memory, member, naming::INPUT_SPAN, MASK_UNITS));
    let Some(start) = member else { return };
    let mut bytes = [0_u8; naming::INPUT_SPAN];
    bytes.copy_from_slice(&job.log.bytes[start..start + naming::INPUT_SPAN]);
    if Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        != base
            .checked_add(naming::VTABLE_RVA)
            .map(|address| address as u64)
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
            override_shown = match unit.checked_add(coperative_leader::CODENAME) {
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

/// Retain ordered occurrences and share successful current-pass definitions.
/// Any native failure kills only this table; staging exhaustion aborts the job.
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
        plan.remember(params, job.worker_growing);
    }
    *dead = job.traits.acquire(memory, base, params).is_err();
    !job.traits.overflowed()
}

/// Mirror of one staged `walk_family` entry's body read set across every
/// batch. Returns `false` only to abort the whole copy (pool overflow or a
/// census-level fault); batch kills are recorded in `dead` and the loop
/// continues exactly like the typed walk.
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
        // A carried pointer names another character: read its omitted
        // spans now and rediscover the program without it.
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
        // Decode supplies the held row; its text joins this pass's demand.
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
        character_id,
        &mut view.unit_id,
        dead,
    ) {
        return false;
    }
    // The trait vector shares the unit leader pointer resolved view-locally
    // above; a null role skips it silently like the typed walk.
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

/// The assigned trait vector of one unit leader, as the typed walk reads it.
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

/// Mirror of the roster census plus the cursor's 32-entry page loop. Page and
/// pointer parses are local; any census failure stops the copy and the replay
/// fails identically from the same recorded spans.
pub(super) fn copy_pass(
    memory: &impl Memory,
    context: CharactersCopyContext,
    job: &mut CharactersJob,
    section: Option<Array>,
) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    if job.worker_growing {
        job.log.enable_worker_growth();
    }
    job.advisor_demand = AdvisorDemand::default();
    job.appended_overflowed = false;
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin(context, job.worker_growing);
    job.game_state = context.game_state;
    job.census_current = false;
    job.role_census_current = false;
    job.advisor_section_complete = false;
    // Each table owner retains its own raw program. Parts beyond the core
    // also preserve capacity demand and freshly reacquire bound trait sources.
    let hourly = roster_part(context.active);
    let bound = !hourly && job.source_bindings.is_some();
    if bound && !job.source_bindings.as_ref().unwrap().matches(context) {
        job.log.invalidate_read_plan();
    }
    if context.active[0] {
        job.templates
            .begin(context.base, context.game_state, context.force_full);
    }
    // A quiet roster attempt ([`quiet`]) already certified this hour's carry.
    if job.part == 0 && section.is_none() && !std::mem::take(&mut job.roster.carry_begun) {
        // Characters rows of certified characters are carried, not read.
        job.carry.begin_hour(
            context.active[0] && !context.force_full && cfg!(feature = "character_changes"),
        );
        job.carry.sample(memory, context.base);
    }
    // Which templates and names the program reads depends on the identities
    // this job already holds and the characters it carries, so their
    // generations key it.
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
    // Daily XP may be served from held bytes. Such a pass must not replay a
    // native-only program that would reread the residual column.
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
            job.source_bindings.as_mut().unwrap().begin(context);
        }
        // The root returns every portrait lease before this job can be recycled.
        // A retained lease disables sharing; never allocate or wait here.
        let mut owned = job.portrait_census.take();
        let census = owned.as_mut().and_then(Arc::get_mut);
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
        // Partial diagnostic passes do not rebuild the retained raw program.
        // They may preserve its census only when every occurrence stayed identical.
        // Otherwise retire that program before an A -> B -> A roster change
        // could certify B's identities using A's successful prepared reads.
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
            // A failed vector kills the table; later sources are not read.
            for &source in &plan.sources {
                if job.traits.acquire(memory, context.base, source).is_err()
                    || job.traits.overflowed()
                {
                    break;
                }
            }
        }
        // The prepared program freshly checked both roster pointer pages and
        // every identity header. It also freshly copied every country field.
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
    // Demand the whole native roster even if staging stops this pass early.
    if let Some(census) = census.as_mut() {
        census.expect(arrays[0].count as usize + arrays[1].count as usize);
        if job.worker_growing && census.grow().map_err(crate::allocation::io).is_err() {
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
    let expected = base.checked_add(ccharacter_manager::VTABLE_RVA)? as u64;
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
            // Look ahead only within the already-copied roster page. These
            // hints cover the same seven view fields; the loop below still
            // performs every actual read and failure check in native order.
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
                    // Registration precedes every read it certifies.
                    job.carry.visit(pointer, carried);
                }
                let Some(view) = copy_view_carrying(job, memory, base, pointer, carried) else {
                    // A fatal later slot is observed now, but its predecessors'
                    // bodies must run before the corresponding typed error.
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

/// Role jobs that take their roster from a loaned census: country-leader
/// roles alone, or unit-leader traits alone.
pub(super) fn census_part(active: [bool; N]) -> bool {
    let leaders = active[1] && !active[4];
    let traits = !active[1] && active[4];
    !active[0] && !active[2] && !active[3] && (leaders || traits)
}

/// On role hours the roster job gathers the role fields of its complete,
/// freshly certified census in one guarded call. Its reads are the roster
/// job's; a retained loan disables sharing without waiting or allocating.
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
    if let (Some(rows), Some(roles)) = (rows, job.role_census.as_mut().and_then(Arc::get_mut)) {
        let (admitted, counts) = job.census_carry.acquire(
            memory,
            roles,
            scope,
            rows,
            stamp,
            job.census_historical,
            job.worker_growing,
        );
        job.role_census_current = admitted;
        if admitted {
            let gathered = if counts.gathered {
                rows.len()
            } else {
                counts.compared as usize
            };
            job.log.raw_bytes[0] += (gathered * (census::ROW - 4)) as u64;
        }
        crate::census_changes::record(&counts);
    }
}

/// A role job's acquisition from the loaned census: the independent walk's
/// reads and failure order from each occurrence's map header or unit pointer
/// on, without the roster, pointer pages or character views.
pub(super) fn copy_census(
    memory: &impl Memory,
    context: CharactersCopyContext,
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    if job.worker_growing {
        job.log.enable_worker_growth();
    }
    job.leader_rows = 0;
    job.advisor_demand = AdvisorDemand::default();
    job.appended_roles.clear();
    job.appended_required = 0;
    job.appended_overflowed = false;
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin(context, job.worker_growing);
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
    // Trait-source bindings belong to the independent walk's prepared program.
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

/// Retain one occurrence for decode. Exhausted storage records the demand of
/// every remaining occurrence that could need one and stops the acquisition.
pub(super) fn visit_census(
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
    index: usize,
    needs: impl Fn(census::Occurrence) -> bool,
) -> bool {
    if job.worker_growing && reserve(&mut job.census_visited, 1).is_err() {
        return false;
    }
    if job.census_visited.len() < job.census_visited.capacity() {
        job.census_visited.push(census.occurrence(index));
        return true;
    }
    let remaining = (index..census.len())
        .filter(|&index| needs(census.occurrence(index)))
        .count();
    job.census_required = job.census_visited.len() + remaining.max(1);
    false
}

/// Leader-map sentinels are gathered in batches ahead of their maps. An empty
/// map whose sentinel passes produces no row and no decode read, so neither
/// its occurrence nor its sentinel is retained; every other occurrence records
/// exactly the independent walk's observations.
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
    let omissible = base.checked_add(ccountry_leader::VTABLE_RVA).is_some();
    let mut heads = [0; SENTINEL_BATCH];
    let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
    let mut next = 0;
    while next < census.len() && !dead.leaders {
        // A header that fails before its sentinel read, or whose sentinel
        // lies inside the character view, ends the batch.
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

/// Unit leaders in roster order, from each occurrence's unit pointer on.
pub(super) fn copy_census_units(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    census: census::Admitted<'_>,
    dead: &mut Dead,
) {
    /// Occurrences ahead whose leader lines are hinted.
    const LOOKAHEAD: usize = 16;
    let unit = |occurrence: census::Occurrence| {
        usize::try_from(u64::from_le_bytes(occurrence.unit()))
            .ok()
            .filter(|&unit| unit != 0)
    };
    let needs = |occurrence: census::Occurrence| unit(occurrence).is_some();
    // The traits job carries the leaders no forwarded writer touched
    // ([`leader_carry`]); the roster job has none to carry here.
    job.trait_carry.begin_hour();
    // A fixed-capacity capture's trait budget counts only fresh rows.
    if !dead.traits && dead.units && job.worker_growing {
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
            // Decode publishes the held rows.
            continue;
        }
        let id = occurrence.id();
        let mut unit_id = None;
        if !copy_unit(job, memory, base, Some(pointer), |_| id, &mut unit_id, dead)
            || !copy_trait_vectors(job, memory, base, pointer, unit_id.unwrap_or(id), dead)
        {
            return;
        }
    }
}

/// Both roots are observed before either body, preserving the shared census
/// failure boundary. Only their current, validated array geometry enters the
/// two programs; a changed dynamic header cannot retire the historical plan.
pub(super) fn copy_advisor_sections(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
) {
    use std::cell::RefCell;
    use std::sync::atomic::Ordering::Relaxed;
    struct Roots<'a, M> {
        memory: &'a M,
        log: RefCell<&'a mut crate::raw::RawLog<N>>,
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
    if job.worker_growing {
        job.advisor_roots.enable_worker_growth();
    }
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
    dynamic.worker_growing = job.worker_growing;
    for (index, (array, _)) in roster.sections().into_iter().enumerate() {
        if index == 1 && !job.advisor_section_complete {
            break;
        }
        let part = if index == 0 { &mut *job } else { &mut *dynamic };
        let started = std::time::Instant::now();
        copy_pass(memory, ctx, part, Some(array));
        ADVISOR_SECTION_NS[index].fetch_add(
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            Relaxed,
        );
        ADVISOR_SECTION_CALLS[index].fetch_add(1, Relaxed);
        if part.log.planned_capture_succeeded() {
            ADVISOR_SECTION_PLANNED[index].fetch_add(1, Relaxed);
        }
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
