use super::*;

pub struct TechnologyDispatch<'a> {
    pub lanes: usize,
    pub acquire:
        &'a mut dyn FnMut(Vec<TechnologyPart>) -> Result<Vec<TechnologyPart>, &'static str>,
}
pub struct TechnologyPart {
    ordinal: usize,
    countries: Vec<engine_native_memory::country_identity::Country>,
    rows: Vec<ctechnology::Row>,
    pointers: Vec<usize>,
    ranges: Vec<std::ops::Range<usize>>,
    entries: Vec<ctechnology_template::Entry>,
    names: Vec<u8>,
    complete: bool,
    outcomes: Vec<Outcome>,
    admissions: Vec<ctechnology_template::Admission>,
}
impl TechnologyPart {
    pub fn capture(&mut self, memory: &impl Memory, base: usize) {
        let _reads = read_accounting::table(TABLES[2]);
        self.complete = false;
        self.ranges.clear();
        self.outcomes.clear();
        let mut dictionary = ctechnology_template::Dictionary::instances(
            &mut self.entries,
            &mut self.names,
            &mut self.admissions,
        );
        let mut end = 0;
        for country in &self.countries {
            let outcome = ctechnology_status::walk_technologies(
                memory,
                base,
                country.pointer,
                country.id,
                (&mut self.rows, &mut self.pointers),
                end,
                &mut dictionary,
            );
            let len = outcome.len();
            self.outcomes.push(outcome);
            self.ranges.push(end..end + len);
            end += len;
        }
        self.complete = true;
    }
}
pub(super) struct Prepared {
    parts: Vec<TechnologyPart>,
    part: usize,
    country: usize,
    failure: Option<&'static str>,
}
impl Prepared {
    fn refused(reason: &'static str) -> Self {
        Self {
            parts: Vec::new(),
            part: 0,
            country: 0,
            failure: Some(reason),
        }
    }
    pub(super) fn copy_country(
        &mut self,
        country: u32,
        rows: &mut Vec<ctechnology::Row>,
        pointers: &mut Vec<usize>,
        first: usize,
    ) -> Outcome {
        if let Some(reason) = self.failure {
            return Outcome::Unavailable { reason };
        }
        let part = &self.parts[self.part];
        assert_eq!(part.countries[self.country].id, country);
        let outcome = part.outcomes[self.country];
        let range = part.ranges[self.country].clone();
        let len = range.len();
        let end = first + len;
        let grown = grow_vec(rows, end) && grow_vec(pointers, end);
        if grown {
            rows[first..end].copy_from_slice(&part.rows[range.clone()]);
            pointers[first..end].copy_from_slice(&part.pointers[range]);
        }
        self.country += 1;
        if self.country == part.countries.len() {
            self.part += 1;
            self.country = 0;
        }
        if grown {
            outcome
        } else {
            Outcome::Unavailable {
                reason: "technology row allocation failed",
            }
        }
    }
}
pub(super) struct Preparation {
    pub(super) definition_ticks: u64,
    pub(super) roster: Vec<Result<Option<engine_native_memory::country_identity::Country>, &'static str>>,
    pub(super) prepared: Option<Prepared>,
}
pub(super) fn prepare(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    dispatch: Option<TechnologyDispatch<'_>>,
    dictionary: &mut ctechnology_template::Dictionary<'_>,
) -> Option<Preparation> {
    let dispatch = dispatch.filter(|dispatch| dispatch.lanes > 1 && countries.count() >= 32)?;
    let mut roster =
        engine_allocation::advisory(|| engine_allocation::vec(countries.count() as usize).ok())?;
    for slot in 1..countries.count() {
        let country = countries.at(slot);
        roster.push(country);
        if country.is_err() {
            break;
        }
    }
    let mut definition_ticks = 0;
    let prepared = if roster.last().is_some_and(Result::is_err) {
        None
    } else {
        prepare_roster(memory, &roster, dispatch, dictionary, &mut definition_ticks)
    };
    Some(Preparation {
        roster,
        prepared,
        definition_ticks,
    })
}
fn prepare_roster(
    memory: &impl Memory,
    roster: &[Result<Option<engine_native_memory::country_identity::Country>, &'static str>],
    dispatch: TechnologyDispatch<'_>,
    dictionary: &mut ctechnology_template::Dictionary<'_>,
    definition_ticks: &mut u64,
) -> Option<Prepared> {
    let parts = engine_allocation::advisory(|| {
        let live = engine_allocation::collect_vec(
            roster.iter().filter_map(|country| country.ok().flatten()),
        )
        .ok()?;
        if live.len() < 2 {
            return None;
        }
        let width = live.len().div_ceil(dispatch.lanes);
        let mut parts = engine_allocation::vec(live.len().div_ceil(width)).ok()?;
        for (ordinal, countries) in live.chunks(width).enumerate() {
            parts.push(TechnologyPart {
                ordinal,
                countries: copy_vec(countries).ok()?,
                rows: Vec::new(),
                pointers: Vec::new(),
                ranges: engine_allocation::vec(countries.len()).ok()?,
                entries: Vec::new(),
                names: Vec::new(),
                complete: false,
                outcomes: engine_allocation::vec(countries.len()).ok()?,
                admissions: Vec::new(),
            });
        }
        Some(parts)
    })?;
    let expected = parts.len();
    let mut parts = match (dispatch.acquire)(parts) {
        Ok(parts) => parts,
        Err(reason) => {
            dictionary.refuse(reason);
            return Some(Prepared::refused(reason));
        }
    };
    parts.sort_unstable_by_key(|part| part.ordinal);
    if parts.len() != expected
        || parts
            .iter()
            .enumerate()
            .any(|(i, part)| part.ordinal != i || !part.complete)
    {
        let reason = "technology partition acquisition incomplete";
        dictionary.refuse(reason);
        return Some(Prepared::refused(reason));
    }
    let _reads = read_accounting::table(TABLES[2]);
    let begin = qpc();
    for part in &parts {
        if let Err(reason) = dictionary.merge_attempts(memory, &part.admissions) {
            *definition_ticks = elapsed(begin);
            dictionary.refuse(reason);
            return Some(Prepared::refused(reason));
        }
    }
    *definition_ticks = elapsed(begin);
    Some(Prepared {
        parts,
        part: 0,
        country: 0,
        failure: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;
    const BASE: usize = 0x1400_0000;
    const MEM: usize = 0x10000;
    struct World {
        bytes: Vec<u8>,
        count: usize,
        objects: usize,
        templates: usize,
    }
    impl World {
        fn new(count: usize) -> Self {
            let objects = (MEM + 0x1000 + (count + 1) * 12).next_multiple_of(0x1000);
            let templates = objects + count * 0x2000;
            let mut this = Self {
                bytes: vec![0; templates + 0x2000 - MEM],
                count,
                objects,
                templates,
            };
            let countries = MEM + 0x1000;
            let links = countries + (count + 1) * 8;
            this.array(MEM + cgame_state::COUNTRIES, countries, count + 1);
            this.array(MEM + cgame_state::COUNTRY_LINKS, links, count + 1);
            for owner in 1..=count {
                let country = this.country(owner);
                let status = country + 0x1000;
                let roster = country + 0x1200;
                this.u64(countries + owner * 8, country as u64);
                this.u32(links + owner * 4, owner as u32);
                this.u64(country, (BASE + ccountry::VTABLE_RVA.0) as u64);
                this.u32(country + ccountry::ID, owner as u32);
                this.u32(country + ccountry::OWNED_STATE_COUNT, 1);
                this.u64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
                this.u64(status, (BASE + ctechnology_status::VTABLE_RVA.0) as u64);
                this.u64(status + ctechnology_status::COUNTRY, country as u64);
                this.array(
                    status + ctechnology_status::TECHNOLOGIES_DATA,
                    roster,
                    owner % 3 + 1,
                );
                for index in 0..owner % 3 + 1 {
                    let technology = country + 0x1400 + index * 0x200;
                    this.u64(roster + index * 8, technology as u64);
                    this.u64(technology, (BASE + ctechnology::VTABLE_RVA.0) as u64);
                    this.u32(technology + ctechnology::DEFINITION_INDEX, 0x80000001);
                    this.u64(technology + ctechnology::STATUS, status as u64);
                    this.u64(
                        technology + ctechnology::TEMPLATE,
                        (templates + index * 0x600) as u64,
                    );
                }
            }
            for index in 0..3 {
                let template = templates + index * 0x600;
                this.u64(
                    template + ctechnology_template::ID,
                    u32::from_le_bytes(*b"tech") as u64,
                );
                this.u64(template + ctechnology_template::ID + 16, 4);
                this.u64(template + ctechnology_template::ID + 24, 15);
                this.u32(
                    template + ctechnology_template::DATABASE_INDEX,
                    index as u32 + 1,
                );
                this.u32(
                    template + ctechnology_template::TECHNOLOGY_HANDLE,
                    0x80000001,
                );
            }
            this
        }
        fn country(&self, owner: usize) -> usize {
            self.objects + (owner - 1) * 0x2000
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.bytes[at - MEM..at - MEM + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.bytes[at - MEM..at - MEM + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: usize) {
            self.u64(at, data as u64);
            self.u32(at + 8, count as u32);
            self.u32(at + 12, count as u32);
        }
        fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: MEM,
                bytes: &self.bytes,
            }
        }
    }
    fn capture(
        world: &World,
        lanes: usize,
        fault: Option<usize>,
    ) -> Vec<([TableView; N], Vec<(bool, bool)>, bool)> {
        struct Fault<'a> {
            inner: SliceMemory<'a>,
            fault: Option<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                !self
                    .fault
                    .is_some_and(|fault| at <= fault && fault < at.saturating_add(out.len()))
                    && self.inner.copy(at, out)
            }
        }
        let memory = Fault {
            inner: world.memory(),
            fault,
        };
        capture_memory(world, &memory, lanes, &[1, 2, 25])
    }
    fn capture_memory(
        world: &World,
        memory: &(impl Memory + Sync),
        lanes: usize,
        hours: &[i32],
    ) -> Vec<([TableView; N], Vec<(bool, bool)>, bool)> {
        let changes = crate::country_input_changes::Domain::enabled(world.count + 1);
        let mut store = SliceFingerprints::new();
        let mut captured = Captured::vacant();
        captured.left_out = std::array::from_fn(|i| ![2, 4].contains(&i));
        let mut output = Vec::new();
        for &hour in hours {
            let mut acquire = |parts: Vec<TechnologyPart>| {
                Ok(std::thread::scope(|scope| {
                    let jobs: Vec<_> = parts
                        .into_iter()
                        .map(|mut part| {
                            scope.spawn(move || {
                                part.capture(memory, BASE);
                                part
                            })
                        })
                        .collect();
                    jobs.into_iter()
                        .rev()
                        .map(|job| job.join().unwrap())
                        .collect()
                }))
            };
            captured.capture_sliced_impl(
                memory,
                BASE,
                MEM,
                hour,
                hour == 1,
                Slices {
                    technologies: &mut store,
                    focus: &mut SliceFingerprints::new(),
                    changes: &changes,
                },
                None,
                true,
                (lanes > 1).then_some(TechnologyDispatch {
                    lanes,
                    acquire: &mut acquire,
                }),
            );
            captured.decode_owned();
            output.push((
                captured.tables(),
                (1..=world.count as u32 + 1)
                    .map(|id| {
                        (
                            captured.technology_selection.live(id),
                            captured.technology_selection.contains(id),
                        )
                    })
                    .collect(),
                captured.technology_changed.certified,
            ));
        }
        output
    }
    #[test]
    fn uneven_and_fifteen_hundred_country_parts_match_serial_rows_and_certificates() {
        for count in [35, 67, 1500] {
            let world = World::new(count);
            let expected = capture(&world, 1, None);
            assert!(matches!(expected[0].0[2], TableView::Complete(_)));
            for lanes in [2, 3, 15] {
                assert_eq!(capture(&world, lanes, None), expected);
            }
        }
    }
    #[test]
    fn cold_parts_keep_serial_first_refusal_and_cross_partition_definition_checks() {
        let mut world = World::new(67);
        for fault in [
            world.country(3),
            world.country(36) + 0x1400,
            world.templates + ctechnology_template::ID,
        ] {
            assert_eq!(
                capture(&world, 15, Some(fault)),
                capture(&world, 1, Some(fault))
            );
        }
        let other = world.templates + 0x1200;
        let original = world.templates - MEM;
        let copy = world.bytes[original..original + 0x600].to_vec();
        world.bytes[other - MEM..other - MEM + 0x600].copy_from_slice(&copy);
        world.u64(
            world.country(66) + 0x1400 + ctechnology::TEMPLATE,
            other as u64,
        );
        let expected = capture(&world, 1, None);
        assert!(matches!(expected[0].0[4], TableView::Unavailable { .. }));
        assert_eq!(capture(&world, 15, None), expected);
    }
    #[test]
    fn one_shot_definition_failure_is_settled_at_its_first_serial_occurrence() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct OneShot<'a> {
            inner: SliceMemory<'a>,
            at: usize,
            reads: AtomicUsize,
            fail: usize,
        }
        impl Memory for OneShot<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at <= self.at
                    && self.at < at.saturating_add(out.len())
                    && self.reads.fetch_add(1, Ordering::Relaxed) == self.fail
                {
                    return false;
                }
                self.inner.copy(at, out)
            }
        }
        let world = World::new(67);
        let memory = |fail| OneShot {
            inner: world.memory(),
            at: world.templates + ctechnology_template::ID + 24,
            reads: AtomicUsize::new(0),
            fail,
        };
        let serial = memory(0);
        let expected = capture_memory(&world, &serial, 1, &[1]);
        for lanes in [2, 3, 15] {
            let parallel = memory(0);
            let actual = capture_memory(&world, &parallel, lanes, &[1]);
            assert_eq!(actual, expected);
            assert!(
                matches!(&actual[0].0[4], TableView::Unavailable { reason } if reason == "technology template fields unreadable")
            );
            assert_eq!(parallel.reads.load(Ordering::Relaxed), 1);
        }
        let country_fault = OneShot {
            inner: world.memory(),
            at: world.country(3),
            reads: AtomicUsize::new(0),
            fail: 0,
        };
        let serial_country_fault = OneShot {
            inner: world.memory(),
            at: world.country(3),
            reads: AtomicUsize::new(0),
            fail: 0,
        };
        assert_eq!(
            capture_memory(&world, &country_fault, 15, &[1]),
            capture_memory(&world, &serial_country_fault, 1, &[1])
        );
        assert_eq!(country_fault.reads.load(Ordering::Relaxed), 1);
        let redundant = memory(1);
        assert_eq!(
            capture_memory(&world, &redundant, 15, &[1])[0],
            capture(&world, 1, None)[0]
        );
    }
    #[test]
    fn required_technology_part_allocation_denial_is_terminal() {
        const NAME: &str = "required_technology_part_allocation_denial_is_terminal";
        const ENV: &str = "BATTLEPLAN_COLD_TECHNOLOGY_TEST";
        if std::env::var(ENV).ok().as_deref() != Some(NAME) {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        &format!("governance::technology_parts::tests::{NAME}")
                    ])
                    .env(ENV, NAME)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        fn failed() {
            crate::allocation::record_failure(
                crate::allocation::Phase::Extended,
                crate::allocation::Code::Reserve,
            );
        }
        let world = World::new(1);
        let mut part = TechnologyPart {
            ordinal: 0,
            countries: vec![engine_native_memory::country_identity::Country {
                id: 1,
                canonical_slot: 1,
                pointer: world.country(1),
            }],
            rows: Vec::new(),
            pointers: Vec::new(),
            ranges: Vec::with_capacity(1),
            entries: Vec::new(),
            names: Vec::new(),
            complete: false,
            outcomes: Vec::with_capacity(1),
            admissions: Vec::new(),
        };
        crate::allocation::reset();
        engine_allocation::set_hooks(None, Some(failed));
        engine_allocation::set_stop_hook(Some(crate::allocation::failed));
        collector_test_support::with_allocations_denied(|| part.capture(&world.memory(), BASE));
        assert!(
            part.outcomes
                .iter()
                .any(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
        assert!(engine_allocation::capture_stopped());
        assert!(crate::allocation::failed());
    }
}
