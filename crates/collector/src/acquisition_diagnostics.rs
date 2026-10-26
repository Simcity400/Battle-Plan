use crate::{
    acquisition::{Ticket, Timeline},
    acquisition_jobs::{MAX_JOBS, ROLES},
    sampling::EarlyStep,
};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

pub(crate) const TOP_JOBS: usize = 8;
const MAX_PROBE_POLLS: usize = 64;
const MAX_PROBE_INSPECTIONS: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identity {
    pub role: usize,
    pub part: Option<usize>,
    pub partition: Option<(u32, u32)>,
    pub kind: u8,
}
impl Identity {
    fn packed(self) -> u64 {
        self.role as u64
            | ((self.kind as u64) << 16)
            | (self.part.map_or(0, |part| part as u64 + 1) << 32)
    }
    fn unpack(value: u64) -> Self {
        Self {
            role: (value & 0xffff) as usize,
            kind: ((value >> 16) & 0xffff) as u8,
            part: ((value >> 32) != 0).then(|| (value >> 32) as usize - 1),
            partition: None,
        }
    }
    fn status(self) -> impl serde::Serialize {
        crate::status::status!({
            "role": crate::status::DebugValue(ROLES[self.role]),
            "part": self.part,
            "partition": self.partition,
            "kind": ["job", "variant_part", "variant_context_part", "event_feed_seal", "technology_part", "project_part", "cold_setup_part", "variant_context_seed_part", "project_runtime_part", "cold_storage_warmup"][self.kind as usize],
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Job {
    pub identity: Identity,
    pub ticket: Ticket,
    pub timeline: Timeline,
}
impl Job {
    fn body_ticks(self) -> u64 {
        self.timeline
            .body_end_qpc
            .saturating_sub(self.timeline.body_begin_qpc)
            .max(0) as u64
    }
    pub(crate) fn status(self, frequency: u64) -> impl serde::Serialize {
        let duration = |begin: i64, end: i64| {
            (begin != 0 && end >= begin).then(|| ns((end - begin) as u64, frequency))
        };
        crate::status::status!({
            "identity": self.identity.status(),
            "ticket": self.ticket,
            "queued_qpc": (self.timeline.queued_qpc != 0).then_some(self.timeline.queued_qpc),
            "body_begin_qpc": (self.timeline.body_begin_qpc != 0).then_some(self.timeline.body_begin_qpc),
            "body_end_qpc": (self.timeline.body_end_qpc != 0).then_some(self.timeline.body_end_qpc),
            "published_qpc": (self.timeline.published_qpc != 0).then_some(self.timeline.published_qpc),
            "collected_qpc": (self.timeline.collected_qpc != 0).then_some(self.timeline.collected_qpc),
            "queue_delay_ns": duration(self.timeline.queued_qpc, self.timeline.body_begin_qpc),
            "body_ns": duration(self.timeline.body_begin_qpc, self.timeline.body_end_qpc),
            "publication_delay_ns": duration(self.timeline.body_end_qpc, self.timeline.published_qpc),
            "growths": self.timeline.growths,
            "callback_thread": self.timeline.caller,
        })
    }
}

#[cfg(any(test, feature = "callback_profile"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct JoinSpan {
    pub job: Job,
    pub profile: crate::acquisition::Profile,
    pub begin_qpc: i64,
    pub end_qpc: i64,
}
#[cfg(any(test, feature = "callback_profile"))]
impl JoinSpan {
    pub(crate) fn status(self, frequency: u64) -> impl serde::Serialize {
        let profile = self.profile;
        let ticks = |at| (at != 0).then_some(at);
        crate::status::status!({
            "job": self.job.status(frequency),
            "join_begin_qpc": ticks(self.begin_qpc),
            "join_end_qpc": ticks(self.end_qpc),
            "claimed_qpc": ticks(profile.claimed_qpc),
            "runner_thread_id": (profile.runner_thread_id != 0).then_some(profile.runner_thread_id),
            "runner_thread_cycles": profile.runner_thread_cycles,
            "submit_lock_wait_ns": ns(profile.submit_lock_wait_ticks, frequency),
            "finish_lock_wait_ns": ns(profile.finish_lock_wait_ticks, frequency),
        })
    }
}

pub(crate) fn ns(ticks: u64, frequency: u64) -> u64 {
    (u128::from(ticks) * 1_000_000_000 / u128::from(frequency)).min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Jobs {
    pub count: u64,
    pub worker_growths: u64,
    pub top: [Job; TOP_JOBS],
    pub len: usize,
    pub research: Option<Job>,
}
impl Jobs {
    pub(crate) fn record(&mut self, job: Job) {
        self.count = self.count.saturating_add(1);
        if !job.timeline.caller {
            self.worker_growths = self.worker_growths.saturating_add(job.timeline.growths);
        }
        if job.identity.role == crate::acquisition_jobs::Role::Research as usize {
            self.research = Some(job);
        }
        self.insert(job);
    }
    fn insert(&mut self, job: Job) {
        let key = |job: &Job| (std::cmp::Reverse(job.body_ticks()), job.ticket.generation());
        let index = self.top[..self.len].partition_point(|old| key(old) <= key(&job));
        if index == TOP_JOBS {
            return;
        }
        let len = (self.len + 1).min(TOP_JOBS);
        self.top.copy_within(index..len - 1, index + 1);
        self.top[index] = job;
        self.len = len;
    }
    pub(crate) fn merge(&mut self, other: &Self) {
        self.count = self.count.saturating_add(other.count);
        self.worker_growths = self.worker_growths.saturating_add(other.worker_growths);
        if let Some(job) = other.research {
            self.research = Some(job);
        }
        for job in &other.top[..other.len] {
            self.insert(*job);
        }
    }
}

#[derive(Default)]
pub(crate) struct Slot {
    identity: AtomicU64,
    accepted: AtomicBool,
    published_qpc: AtomicU64,
    jobs: UnsafeCell<Jobs>,
    current: UnsafeCell<Identity>,
}
// SAFETY: admission/collection own the slot until its release; snapshots require a settled batch.
unsafe impl Sync for Slot {}
impl Slot {
    pub(crate) unsafe fn admit(&self, identity: Identity, accepted: bool) {
        // Each metadata release follows the previous collector's VACANT, even before PENDING.
        self.accepted.store(accepted, Ordering::Release);
        self.identity.store(identity.packed(), Ordering::Release);
        // SAFETY: the submitter exclusively owns a VACANT cell under the queue mutex.
        unsafe {
            *self.current.get() = identity;
        }
    }
    pub(crate) fn accepted(&self) -> bool {
        self.accepted.load(Ordering::Acquire)
    }
    pub(crate) fn identity(&self) -> Identity {
        Identity::unpack(self.identity.load(Ordering::Acquire))
    }
    pub(crate) fn publish(&self, qpc: i64) {
        self.published_qpc.store(qpc as u64, Ordering::Release);
    }
    pub(crate) fn published_qpc(&self) -> i64 {
        self.published_qpc.load(Ordering::Acquire) as i64
    }
    pub(crate) unsafe fn collected(&self, ticket: Ticket, timeline: Timeline) {
        // SAFETY: the unique collector owns COLLECTING until the following VACANT release.
        unsafe {
            (*self.jobs.get()).record(Job {
                identity: *self.current.get(),
                ticket,
                timeline,
            });
        }
    }
    pub(crate) unsafe fn drain_into(&self, total: &mut Jobs) {
        // SAFETY: the caller has drained and settled all slot owners before the next admission.
        let jobs = unsafe { &mut *self.jobs.get() };
        total.merge(jobs);
        jobs.count = 0;
        jobs.worker_growths = 0;
        jobs.len = 0;
        jobs.research = None;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Completion {
    pub identity: Identity,
    pub ticket: Ticket,
}
impl Completion {
    fn status(self) -> impl serde::Serialize {
        crate::status::status!({"identity": self.identity.status(), "ticket": self.ticket})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Outstanding {
    pub generations: [u64; MAX_JOBS],
    pub identities: [u64; MAX_JOBS],
    pub pool: u64,
    pub changed_slots: u64,
}
impl Default for Outstanding {
    fn default() -> Self {
        Self {
            generations: [0; MAX_JOBS],
            identities: [0; MAX_JOBS],
            pool: 0,
            changed_slots: 0,
        }
    }
}
impl Outstanding {
    pub(crate) fn add(&mut self, slot: usize, generation: u64, identity: Identity) {
        self.generations[slot] = generation;
        self.identities[slot] = identity.packed();
    }
    fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::sequence(
            self.generations
                .iter()
                .enumerate()
                .filter(|(_, generation)| **generation != 0)
                .map(|(slot, generation)| {
                    Completion {
                        identity: Identity::unpack(self.identities[slot]),
                        ticket: Ticket::diagnostic(self.pool, slot, *generation),
                    }
                    .status()
                }),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StepSpan {
    pub step: usize,
    pub wall_ns: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hour {
    pub readiness_total_ns: u64,
    pub readiness_max_ns: u64,
    pub readiness_count: u64,
    pub outstanding: Outstanding,
    pub completion: Option<Completion>,
    pub ending_probe_exhausted: bool,
    pub jobs: Jobs,
    pub callback_growths: u64,
    pub longest_step: Option<StepSpan>,
    pub division_merge_ns: u64,
    pub wing_merge_ns: u64,
}
impl Hour {
    pub(crate) fn readiness(
        &mut self,
        wall_ns: u64,
        outstanding: Outstanding,
        completion: Option<Completion>,
        ending_probe_exhausted: bool,
    ) {
        self.readiness_count = self.readiness_count.saturating_add(1);
        self.readiness_total_ns = self.readiness_total_ns.saturating_add(wall_ns);
        if self.readiness_count == 1 || wall_ns > self.readiness_max_ns {
            self.readiness_max_ns = wall_ns;
            self.outstanding = outstanding;
            self.completion = completion.filter(|_| !ending_probe_exhausted);
            self.ending_probe_exhausted = ending_probe_exhausted;
        }
    }
    pub(crate) fn step(&mut self, step: usize, wall_ns: u64) {
        if self.longest_step.is_none_or(|old| wall_ns > old.wall_ns) {
            self.longest_step = Some(StepSpan { step, wall_ns });
        }
    }
    pub(crate) fn status(&self, frequency: u64) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "readiness_waits": {
                "total_ns": self.readiness_total_ns, "max_ns": self.readiness_max_ns,
                "count": self.readiness_count,
                "longest": (self.readiness_count != 0).then(|| crate::status::status!({
                    "outstanding": self.outstanding.status(),
                    "outstanding_changed_slots": self.outstanding.changed_slots,
                    "ending_completion": self.completion.map(Completion::status),
                    "ending_probe_exhausted": self.ending_probe_exhausted,
                })),
            },
            "jobs": {"count": self.jobs.count, "research": self.jobs.research.map(move |job| job.status(frequency)), "top_by_body": crate::status::sequence(self.jobs.top[..self.jobs.len].iter().map(move |job| job.status(frequency)))},
            "growths": {"callback_thread": self.callback_growths, "workers": self.jobs.worker_growths},
            "longest_early_step": self.longest_step.map(|step| crate::status::status!({
                "step": crate::status::DebugValue(EarlyStep::ALL[step.step]), "wall_ns": step.wall_ns,
            })),
            "inline_census_merges_ns": {"divisions": self.division_merge_ns, "wings": self.wing_merge_ns},
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Probe {
    active: bool,
    remaining: usize,
    latest: Option<(i64, Completion)>,
    exhausted: bool,
}

pub(crate) struct ProbeBudget {
    polls: usize,
    inspections: usize,
}
impl Default for ProbeBudget {
    fn default() -> Self {
        Self {
            polls: MAX_PROBE_POLLS,
            inspections: MAX_PROBE_INSPECTIONS,
        }
    }
}
impl ProbeBudget {
    pub(crate) fn poll<T>(&mut self, ready: impl FnOnce() -> T) -> (T, Option<Completion>, bool) {
        if self.polls == 0 || self.inspections == 0 {
            return (ready(), None, true);
        }
        self.polls -= 1;
        PROBE.set(Probe {
            active: true,
            remaining: self.inspections,
            ..Default::default()
        });
        let result = ready();
        let probe = PROBE.replace(Probe::default());
        self.inspections = probe.remaining;
        let completion = probe
            .latest
            .filter(|_| !probe.exhausted)
            .map(|(_, value)| value);
        (result, completion, probe.exhausted)
    }
}

thread_local! {
    static HOUR: RefCell<Hour> = RefCell::new(Hour::default());
    static PROBE: Cell<Probe> = const { Cell::new(Probe { active: false, remaining: 0, latest: None, exhausted: false }) };
}
pub(crate) fn reset() {
    HOUR.with_borrow_mut(|hour| *hour = Hour::default());
    PROBE.set(Probe::default());
}
pub(crate) fn update(f: impl FnOnce(&mut Hour)) {
    HOUR.with_borrow_mut(f);
}
pub(crate) fn take(callback_growths: u64) -> Hour {
    HOUR.with_borrow_mut(|hour| {
        hour.callback_growths = callback_growths;
        std::mem::take(hour)
    })
}
pub(crate) fn probe_inspection() -> bool {
    let mut probe = PROBE.get();
    if !probe.active {
        return false;
    }
    if probe.remaining == 0 {
        probe.exhausted = true;
        probe.latest = None;
        PROBE.set(probe);
        return false;
    }
    probe.remaining -= 1;
    PROBE.set(probe);
    true
}
pub(crate) fn probe_condition() {
    let mut probe = PROBE.get();
    if probe.active {
        probe.latest = None;
        PROBE.set(probe);
    }
}
pub(crate) fn observed(identity: Identity, ticket: Ticket, published_qpc: i64) {
    let mut probe = PROBE.get();
    if probe.active && !probe.exhausted && probe.latest.is_none_or(|(at, _)| published_qpc > at) {
        probe.latest = Some((published_qpc, Completion { identity, ticket }));
        PROBE.set(probe);
    }
}

#[cfg(feature = "callback_profile")]
pub(crate) struct Status(pub Hour, pub u64);
#[cfg(feature = "callback_profile")]
impl serde::Serialize for Status {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.status(self.1).serialize(serializer)
    }
}

#[cfg(any(test, feature = "callback_profile"))]
pub(crate) const FIELDS: usize = 22 + 2 * MAX_JOBS + 14 * (TOP_JOBS + 1) + 1;
#[cfg(any(test, feature = "callback_profile"))]
impl Hour {
    pub(crate) fn fields(self) -> [u64; FIELDS] {
        let mut fields = [0; FIELDS];
        let completion = self.completion.unwrap_or_default();
        let step = self.longest_step.unwrap_or_default();
        fields[..22].copy_from_slice(&[
            self.readiness_total_ns,
            self.readiness_max_ns,
            self.readiness_count,
            self.outstanding.pool,
            self.callback_growths,
            u64::from(self.longest_step.is_some()),
            step.step as u64,
            step.wall_ns,
            self.division_merge_ns,
            self.wing_merge_ns,
            self.jobs.count,
            self.jobs.worker_growths,
            self.jobs.len as u64,
            u64::from(self.completion.is_some()),
            completion.identity.packed(),
            completion.ticket.pool(),
            completion.ticket.slot() as u64,
            completion.ticket.generation(),
            u64::from(completion.identity.partition.is_some()),
            completion
                .identity
                .partition
                .map_or(0, |(start, end)| start as u64 | ((end as u64) << 32)),
            self.outstanding.changed_slots,
            u64::from(self.ending_probe_exhausted),
        ]);
        fields[22..22 + MAX_JOBS].copy_from_slice(&self.outstanding.generations);
        fields[22 + MAX_JOBS..22 + 2 * MAX_JOBS].copy_from_slice(&self.outstanding.identities);
        for (job, fields) in self
            .jobs
            .top
            .into_iter()
            .chain(std::iter::once(self.jobs.research.unwrap_or_default()))
            .zip(fields[22 + 2 * MAX_JOBS..FIELDS - 1].chunks_exact_mut(14))
        {
            let partition = job.identity.partition.unwrap_or_default();
            fields.copy_from_slice(&[
                job.identity.packed(),
                u64::from(job.identity.partition.is_some()),
                partition.0 as u64 | ((partition.1 as u64) << 32),
                job.ticket.pool(),
                job.ticket.slot() as u64,
                job.ticket.generation(),
                job.timeline.queued_qpc as u64,
                job.timeline.body_begin_qpc as u64,
                job.timeline.body_end_qpc as u64,
                job.timeline.published_qpc as u64,
                job.timeline.collected_qpc as u64,
                job.timeline.finish_lock_ticks,
                job.timeline.growths,
                u64::from(job.timeline.caller),
            ]);
        }
        fields[FIELDS - 1] = u64::from(self.jobs.research.is_some());
        fields
    }
    pub(crate) fn from_fields(fields: [u64; FIELDS]) -> Self {
        let mut hour = Self {
            readiness_total_ns: fields[0],
            readiness_max_ns: fields[1],
            readiness_count: fields[2],
            callback_growths: fields[4],
            longest_step: (fields[5] != 0).then_some(StepSpan {
                step: fields[6] as usize,
                wall_ns: fields[7],
            }),
            ending_probe_exhausted: fields[21] != 0,
            division_merge_ns: fields[8],
            wing_merge_ns: fields[9],
            completion: (fields[13] != 0).then_some(Completion {
                identity: Identity::unpack(fields[14]),
                ticket: Ticket::diagnostic(fields[15], fields[16] as usize, fields[17]),
            }),
            ..Default::default()
        };
        hour.outstanding.pool = fields[3];
        hour.outstanding.changed_slots = fields[20];
        if let Some(completion) = &mut hour.completion {
            completion.identity.partition =
                (fields[18] != 0).then_some((fields[19] as u32, (fields[19] >> 32) as u32));
        }
        hour.outstanding
            .generations
            .copy_from_slice(&fields[22..22 + MAX_JOBS]);
        hour.outstanding
            .identities
            .copy_from_slice(&fields[22 + MAX_JOBS..22 + 2 * MAX_JOBS]);
        hour.jobs.count = fields[10];
        hour.jobs.worker_growths = fields[11];
        hour.jobs.len = fields[12] as usize;
        let mut research = Job::default();
        for (job, fields) in hour
            .jobs
            .top
            .iter_mut()
            .chain(std::iter::once(&mut research))
            .zip(fields[22 + 2 * MAX_JOBS..FIELDS - 1].chunks_exact(14))
        {
            job.identity = Identity::unpack(fields[0]);
            job.identity.partition =
                (fields[1] != 0).then_some((fields[2] as u32, (fields[2] >> 32) as u32));
            job.ticket = Ticket::diagnostic(fields[3], fields[4] as usize, fields[5]);
            job.timeline = Timeline {
                queued_qpc: fields[6] as i64,
                body_begin_qpc: fields[7] as i64,
                body_end_qpc: fields[8] as i64,
                published_qpc: fields[9] as i64,
                collected_qpc: fields[10] as i64,
                finish_lock_ticks: fields[11],
                growths: fields[12],
                caller: fields[13] != 0,
                thread_work: None,
            };
        }
        hour.jobs.research = (fields[FIELDS - 1] != 0).then_some(research);
        hour
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acquisition_jobs::Role;

    fn job(index: usize, body: i64, growths: u64, caller: bool) -> Job {
        Job {
            identity: Identity {
                role: Role::Variants as usize,
                part: Some(index),
                kind: 1,
                partition: Some((index as u32, index as u32 + 1)),
            },
            ticket: Ticket::diagnostic(7, index % 2, index as u64 + 1),
            timeline: Timeline {
                queued_qpc: 100 + index as i64,
                body_begin_qpc: 200,
                body_end_qpc: 200 + body,
                published_qpc: 210 + body,
                growths,
                caller,
                ..Default::default()
            },
        }
    }

    #[test]
    fn a_short_research_join_retains_its_identity_and_all_timestamps() {
        let mut research = job(4, 5, 2, false);
        research.identity.role = Role::Research as usize;
        research.timeline.collected_qpc = 1000;
        let mut hour = Hour::default();
        hour.jobs.record(research);
        for index in 10..30 {
            hour.jobs.record(job(index, index as i64, 0, false));
        }
        assert!(
            !hour
                .jobs
                .top
                .iter()
                .any(|job| job.identity.role == Role::Research as usize)
        );
        assert_eq!(Hour::from_fields(hour.fields()), hour);
        assert_eq!(hour.jobs.research, Some(research));
        let span = JoinSpan {
            job: research,
            profile: crate::acquisition::Profile {
                claimed_qpc: 150,
                runner_thread_id: 7,
                ..Default::default()
            },
            begin_qpc: 110,
            end_qpc: 1010,
        };
        let json = serde_json::to_value(span.status(10_000_000)).unwrap();
        assert_eq!(json["job"]["identity"]["role"], "Research");
        assert_eq!(json["job"]["identity"]["part"], 4);
        assert_eq!(json["job"]["queued_qpc"], 104);
        assert_eq!(json["job"]["body_begin_qpc"], 200);
        assert_eq!(json["job"]["body_end_qpc"], 205);
        assert_eq!(json["job"]["published_qpc"], 215);
        assert_eq!(json["job"]["collected_qpc"], 1000);
        assert_eq!(json["join_begin_qpc"], 110);
        assert_eq!(json["join_end_qpc"], 1010);
        assert_eq!(json["claimed_qpc"], 150);
        let missing = serde_json::to_value(JoinSpan::default().status(1)).unwrap();
        assert!(missing["job"]["body_begin_qpc"].is_null());
        assert!(missing["join_begin_qpc"].is_null());
    }

    #[test]
    fn every_job_counts_and_per_slot_top_eight_merge_is_exact_after_reuse() {
        let mut slots = [Jobs::default(); 2];
        for index in 0..40 {
            slots[index % 2].record(job(index, index as i64, index as u64, index % 3 == 0));
        }
        let mut total = Jobs::default();
        for slot in &slots {
            total.merge(slot);
        }
        assert_eq!(total.count, 40);
        assert_eq!(
            total.worker_growths,
            (0..40).filter(|index| index % 3 != 0).sum::<u64>()
        );
        assert_eq!(
            total.top[..total.len]
                .iter()
                .map(|job| job.identity.part.unwrap())
                .collect::<Vec<_>>(),
            (32..40).rev().collect::<Vec<_>>()
        );
        let mut equal = Jobs::default();
        for index in (0..20).rev() {
            equal.record(job(index, 10, 0, false));
        }
        assert_eq!(
            equal.top.map(|job| job.identity.part.unwrap()),
            std::array::from_fn::<_, TOP_JOBS, _>(|index| index)
        );
    }

    #[test]
    fn slot_history_keeps_children_and_resets_only_at_the_settled_boundary() {
        let slot = Slot::default();
        for index in 0..12 {
            let job = job(index, index as i64, 2, false);
            // SAFETY: the synthetic slot is exclusively owned by this test.
            unsafe {
                slot.admit(job.identity, true);
                slot.collected(job.ticket, job.timeline);
            }
        }
        let mut total = Jobs::default();
        // SAFETY: synthetic slots have no concurrent readers, collectors or submitters.
        unsafe {
            slot.drain_into(&mut total);
        }
        assert_eq!(total.count, 12);
        assert_eq!(total.worker_growths, 24);
        assert_eq!(total.top[0].identity.part, Some(11));
        let mut empty = Jobs::default();
        // SAFETY: the synthetic slot is still exclusively owned here.
        unsafe {
            slot.drain_into(&mut empty);
        }
        assert_eq!(empty, Jobs::default());
        // SAFETY: the synthetic slot remains exclusively owned.
        unsafe {
            slot.admit(Identity::default(), false);
        }
        assert!(!slot.accepted());
    }

    #[test]
    fn longest_readiness_keeps_first_tie_and_unknown_completion_stays_null() {
        let mut hour = Hour::default();
        let completion = Completion {
            identity: job(3, 0, 0, false).identity,
            ticket: Ticket::diagnostic(7, 3, 4),
        };
        let mut outstanding = Outstanding {
            pool: 7,
            ..Default::default()
        };
        outstanding.add(3, 4, completion.identity);
        hour.readiness(50, outstanding, Some(completion), false);
        hour.readiness(50, Outstanding::default(), None, true);
        hour.readiness(20, Outstanding::default(), None, false);
        assert_eq!(
            (
                hour.readiness_total_ns,
                hour.readiness_max_ns,
                hour.readiness_count
            ),
            (120, 50, 3)
        );
        assert_eq!(hour.completion, Some(completion));
        assert!(!hour.ending_probe_exhausted);
        let status = serde_json::to_value(hour.status(10_000_000)).unwrap();
        assert_eq!(
            status["readiness_waits"]["longest"]["outstanding"][0]["ticket"]["generation"],
            4
        );
        assert_eq!(
            status["readiness_waits"]["longest"]["outstanding"][0]["identity"]["role"],
            "Variants"
        );
        hour.readiness(51, Outstanding::default(), Some(completion), true);
        assert!(hour.ending_probe_exhausted);
        assert_eq!(Hour::from_fields(hour.fields()), hour);
        let status = serde_json::to_value(hour.status(10_000_000)).unwrap();
        assert!(status["readiness_waits"]["longest"]["ending_completion"].is_null());
        assert_eq!(
            status["readiness_waits"]["longest"]["ending_probe_exhausted"],
            true
        );
        assert!(
            serde_json::to_value(Hour::default().status(1)).unwrap()["readiness_waits"]["longest"]
                .is_null()
        );
    }

    #[test]
    fn growth_split_steps_merges_and_atomic_fields_round_trip_without_clock_queries() {
        let mut hour = Hour {
            callback_growths: 17,
            division_merge_ns: 13,
            wing_merge_ns: 19,
            ..Default::default()
        };
        hour.step(EarlyStep::Inputs as usize, 70);
        hour.step(EarlyStep::Wings as usize, 70);
        hour.step(EarlyStep::Ships as usize, 60);
        hour.jobs.record(job(0, 50, 23, false));
        hour.jobs.record(job(1, 70, 11, true));
        hour.readiness(
            31,
            Outstanding {
                pool: 7,
                ..Default::default()
            },
            Some(Completion {
                identity: job(0, 0, 0, false).identity,
                ticket: Ticket::diagnostic(7, 0, 1),
            }),
            false,
        );
        assert_eq!(Hour::from_fields(hour.fields()), hour);
        let status = serde_json::to_value(hour.status(10_000_000)).unwrap();
        assert_eq!(
            status["growths"],
            serde_json::json!({"callback_thread": 17, "workers": 23})
        );
        assert_eq!(
            status["longest_early_step"],
            serde_json::json!({"step": "Inputs", "wall_ns": 70})
        );
        assert_eq!(
            status["inline_census_merges_ns"],
            serde_json::json!({"divisions": 13, "wings": 19})
        );
        assert_eq!(status["jobs"]["top_by_body"][1]["queue_delay_ns"], 10_000);
        assert_eq!(status["jobs"]["top_by_body"][1]["body_ns"], 5_000);
        assert_eq!(
            status["jobs"]["top_by_body"][1]["publication_delay_ns"],
            1_000
        );
        let missing = serde_json::to_value(Job::default().status(1)).unwrap();
        assert!(missing["body_ns"].is_null());
        assert!(missing["queue_delay_ns"].is_null());
        assert!(missing["publication_delay_ns"].is_null());
    }

    #[test]
    fn callback_scratch_and_successful_poll_completion_do_not_cross_hours() {
        crate::sampling::reset_hour_joins();
        update(|hour| hour.step(EarlyStep::Wings as usize, 99));
        let mut probe = ProbeBudget::default();
        let completion = Completion {
            identity: job(1, 0, 0, false).identity,
            ticket: Ticket::diagnostic(7, 1, 2),
        };
        assert_eq!(
            probe.poll(|| {
                observed(completion.identity, completion.ticket, 100);
                7
            }),
            (7, Some(completion), false)
        );
        assert_eq!(probe.poll(|| 8), (8, None, false));
        assert_eq!(take(3).callback_growths, 3);
        assert_eq!(take(0), Hour::default());
        update(|hour| hour.step(EarlyStep::Inputs as usize, 10));
        probe.poll(|| {
            crate::sampling::reset_hour_joins();
            assert_eq!(take(0), Hour::default());
            assert!(!probe_inspection());
        });
    }

    #[test]
    fn readiness_attributes_latest_publication_only_in_the_successful_condition() {
        let first = job(1, 0, 0, false);
        let last = job(2, 0, 0, false);
        let mut probe = ProbeBudget::default();
        assert_eq!(
            probe
                .poll(|| {
                    observed(first.identity, first.ticket, 999);
                    probe_condition();
                    observed(last.identity, last.ticket, 200);
                    observed(first.identity, first.ticket, 100);
                })
                .1,
            Some(Completion {
                identity: last.identity,
                ticket: last.ticket
            })
        );
        assert_eq!(
            probe
                .poll(|| {
                    observed(first.identity, first.ticket, 200);
                    observed(last.identity, last.ticket, 200);
                })
                .1,
            Some(Completion {
                identity: first.identity,
                ticket: first.ticket
            })
        );
        assert_eq!(
            probe
                .poll(|| {
                    observed(first.identity, first.ticket, 200);
                    probe_condition();
                })
                .1,
            None
        );
        observed(first.identity, first.ticket, 999);
        assert!(!probe_inspection());
    }

    #[test]
    fn readiness_probe_budgets_bound_work_without_changing_predicate_results() {
        reset();
        let completion = Completion {
            identity: job(1, 0, 0, false).identity,
            ticket: Ticket::diagnostic(7, 1, 2),
        };
        let mut probe = ProbeBudget::default();
        for poll in 0..MAX_PROBE_POLLS + 3 {
            let result = probe.poll(|| {
                if probe_inspection() {
                    observed(completion.identity, completion.ticket, 100);
                }
                poll
            });
            assert_eq!(
                result,
                if poll < MAX_PROBE_POLLS {
                    (poll, Some(completion), false)
                } else {
                    (poll, None, true)
                }
            );
        }
        let mut probe = ProbeBudget::default();
        for poll in 0..2 {
            assert_eq!(
                probe.poll(|| {
                    for _ in 0..MAX_PROBE_INSPECTIONS / 2 {
                        assert!(probe_inspection());
                        observed(completion.identity, completion.ticket, 100);
                    }
                    poll
                }),
                (poll, Some(completion), false)
            );
        }
        assert_eq!(
            probe.poll(|| {
                assert!(!probe_inspection());
                7
            }),
            (7, None, true)
        );
        let mut probe = ProbeBudget::default();
        assert_eq!(
            probe.poll(|| {
                for _ in 0..MAX_PROBE_INSPECTIONS {
                    assert!(probe_inspection());
                    observed(completion.identity, completion.ticket, 100);
                }
                assert!(!probe_inspection());
                observed(completion.identity, completion.ticket, 999);
                8
            }),
            (8, None, true)
        );
        assert!(!probe_inspection());
    }
}
