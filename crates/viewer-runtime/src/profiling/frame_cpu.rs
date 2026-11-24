use super::current_thread_cpu_time;
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    thread::ThreadId,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Record {
    name: &'static str,
    wall_ms: f64,
    cpu_ms: Option<f64>,
    calls: u64,
}

#[derive(Default)]
struct Frame {
    epoch: u64,
    spans: Vec<Record>,
    counts: Vec<(&'static str, u64)>,
    owners: Vec<OwnerRecord>,
}

std::thread_local! {
    static FRAME: RefCell<Frame> = RefCell::new(Frame::default());
}

pub fn begin_thread_frame() {
    if crate::profile_writer::profiling() {
        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            frame.epoch = frame.epoch.wrapping_add(1);
            frame.spans.clear();
            frame.counts.clear();
            frame.owners.clear();
        });
    }
}

#[derive(Clone, Copy)]
pub struct FrameOwners {
    origin: Option<(ThreadId, u64)>,
}

impl Default for FrameOwners {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameOwners {
    pub fn new() -> Self {
        Self::with_enabled(crate::profile_writer::profiling())
    }

    fn with_enabled(enabled: bool) -> Self {
        Self {
            origin: enabled.then(|| {
                (
                    std::thread::current().id(),
                    FRAME.with(|frame| frame.borrow().epoch),
                )
            }),
        }
    }

    pub fn measure<T>(
        self,
        name: &'static str,
        work: impl FnOnce() -> T,
    ) -> (T, Option<OwnerSample>) {
        let Some((origin, epoch)) = self.origin else {
            return (work(), None);
        };
        let main = std::thread::current().id() == origin;
        let started = current_thread_cpu_time();
        let value = work();
        let cpu_ms = started
            .zip(current_thread_cpu_time())
            .and_then(|(before, after)| after.checked_sub(before))
            .map(|used| used.as_secs_f64() * 1000.0);
        (
            value,
            Some(OwnerSample {
                name,
                origin,
                epoch,
                main,
                cpu_ms,
            }),
        )
    }
}

#[derive(Clone, Copy)]
pub struct OwnerSample {
    name: &'static str,
    origin: ThreadId,
    epoch: u64,
    main: bool,
    cpu_ms: Option<f64>,
}

struct OwnerUsage {
    calls: u64,
    cpu_ms: Option<f64>,
}

impl Default for OwnerUsage {
    fn default() -> Self {
        Self {
            calls: 0,
            cpu_ms: Some(0.0),
        }
    }
}

struct OwnerRecord {
    name: &'static str,
    main: OwnerUsage,
    helper: OwnerUsage,
}

impl OwnerRecord {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            main: OwnerUsage::default(),
            helper: OwnerUsage::default(),
        }
    }

    fn add(&mut self, main: bool, cpu_ms: Option<f64>) {
        let usage = if main {
            &mut self.main
        } else {
            &mut self.helper
        };
        usage.calls += 1;
        usage.cpu_ms = usage.cpu_ms.zip(cpu_ms).map(|(a, b)| a + b);
    }
}

fn adopt_owner(frame: &mut Frame, receiver: ThreadId, sample: OwnerSample) {
    if receiver != sample.origin || frame.epoch != sample.epoch {
        return;
    }
    if let Some(owner) = frame
        .owners
        .iter_mut()
        .find(|owner| owner.name == sample.name)
    {
        owner.add(sample.main, sample.cpu_ms);
    } else {
        let mut owner = OwnerRecord::new(sample.name);
        owner.add(sample.main, sample.cpu_ms);
        frame.owners.push(owner);
    }
}

pub fn record_owner_sample(sample: Option<OwnerSample>) {
    if let Some(sample) = sample {
        let receiver = std::thread::current().id();
        FRAME.with(|frame| adopt_owner(&mut frame.borrow_mut(), receiver, sample));
    }
}

#[must_use = "a thread span records when finished or dropped"]
pub struct ThreadSpan {
    name: &'static str,
    clock: Option<(u64, Instant, Option<Duration>)>,
    same_thread: PhantomData<Rc<()>>,
}

pub fn thread_span(name: &'static str) -> ThreadSpan {
    ThreadSpan::new(name, crate::profile_writer::profiling())
}

impl ThreadSpan {
    fn new(name: &'static str, enabled: bool) -> Self {
        let clock = enabled.then(|| {
            let cpu = current_thread_cpu_time();
            (
                FRAME.with(|frame| frame.borrow().epoch),
                Instant::now(),
                cpu,
            )
        });
        Self {
            name,
            clock,
            same_thread: PhantomData,
        }
    }

    pub fn finish(self) {}
}

impl Drop for ThreadSpan {
    fn drop(&mut self) {
        let Some((epoch, started, cpu)) = self.clock else {
            return;
        };
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let cpu_ms = cpu
            .zip(current_thread_cpu_time())
            .and_then(|(before, after)| after.checked_sub(before))
            .map(|used| used.as_secs_f64() * 1000.0);
        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            if frame.epoch == epoch {
                add(&mut frame.spans, self.name, wall_ms, cpu_ms);
            }
        });
    }
}

pub(super) struct FrameRecords {
    spans: Vec<Record>,
    counts: Vec<(&'static str, u64)>,
    owners: Vec<OwnerRecord>,
}

pub(super) fn take_frame_records() -> FrameRecords {
    FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        FrameRecords {
            spans: std::mem::take(&mut frame.spans),
            counts: std::mem::take(&mut frame.counts),
            owners: std::mem::take(&mut frame.owners),
        }
    })
}

pub(super) fn adopt_frame_records(records: FrameRecords) {
    FRAME.with(|frame| merge_frame_records(&mut frame.borrow_mut(), records));
}

fn merge_frame_records(frame: &mut Frame, records: FrameRecords) {
    for span in records.spans {
        match frame.spans.iter_mut().find(|own| own.name == span.name) {
            Some(own) => {
                own.wall_ms += span.wall_ms;
                own.cpu_ms = own.cpu_ms.zip(span.cpu_ms).map(|(a, b)| a + b);
                own.calls += span.calls;
            }
            None => frame.spans.push(span),
        }
    }
    for (name, value) in records.counts {
        match frame.counts.iter_mut().find(|(key, _)| *key == name) {
            Some((_, count)) => *count += value,
            None => frame.counts.push((name, value)),
        }
    }
    for owner in records.owners {
        match frame.owners.iter_mut().find(|own| own.name == owner.name) {
            Some(own) => {
                own.main.merge(owner.main);
                own.helper.merge(owner.helper);
            }
            None => frame.owners.push(owner),
        }
    }
}

impl OwnerUsage {
    fn merge(&mut self, other: Self) {
        self.calls += other.calls;
        self.cpu_ms = self.cpu_ms.zip(other.cpu_ms).map(|(a, b)| a + b);
    }
}

pub fn record_count(name: &'static str, value: u64) {
    if crate::profile_writer::profiling() {
        FRAME.with(|frame| {
            let mut frame = frame.borrow_mut();
            if let Some((_, count)) = frame.counts.iter_mut().find(|(key, _)| *key == name) {
                *count += value;
            } else {
                frame.counts.push((name, value));
            }
        });
    }
}

fn add(spans: &mut Vec<Record>, name: &'static str, wall_ms: f64, cpu_ms: Option<f64>) {
    if let Some(span) = spans.iter_mut().find(|span| span.name == name) {
        span.wall_ms += wall_ms;
        span.cpu_ms = span.cpu_ms.zip(cpu_ms).map(|(a, b)| a + b);
        span.calls += 1;
    } else {
        spans.push(Record {
            name,
            wall_ms,
            cpu_ms,
            calls: 1,
        });
    }
}

fn line(spans: &[Record], counts: &[(&'static str, u64)], owners: &[OwnerRecord]) -> String {
    let mut fields = Vec::new();
    for span in spans {
        fields.push(format!(
            "mt_{}={:.3} mt_{}_cpu={} mt_{}_calls={}",
            span.name,
            span.wall_ms,
            span.name,
            span.cpu_ms
                .map_or_else(|| "unavailable".to_owned(), |ms| format!("{ms:.3}")),
            span.name,
            span.calls
        ));
    }
    for (name, value) in counts {
        fields.push(format!("{name}={value}"));
    }
    for owner in owners {
        let placement = match (owner.main.calls > 0, owner.helper.calls > 0) {
            (true, false) => "main",
            (false, true) => "helper",
            (true, true) => "mixed",
            (false, false) => "not_run",
        };
        let cpu_ms = owner
            .main
            .cpu_ms
            .zip(owner.helper.cpu_ms)
            .map(|(a, b)| a + b);
        fields.push(format!(
            "owner_{}={} owner_{}_cpu={} owner_{}_calls={}",
            owner.name,
            placement,
            owner.name,
            cpu_ms.map_or_else(|| "unavailable".to_owned(), |ms| format!("{ms:.3}")),
            owner.name,
            owner.main.calls + owner.helper.calls,
        ));
        if placement == "mixed" {
            fields.push(format!(
                "owner_{}_main_cpu={} owner_{}_helper_cpu={}",
                owner.name,
                owner
                    .main
                    .cpu_ms
                    .map_or_else(|| "unavailable".to_owned(), |ms| format!("{ms:.3}")),
                owner.name,
                owner
                    .helper
                    .cpu_ms
                    .map_or_else(|| "unavailable".to_owned(), |ms| format!("{ms:.3}")),
            ));
        }
    }
    fields.join(" ")
}

pub fn take_thread_spans() -> String {
    FRAME.with(|frame| {
        let mut frame = frame.borrow_mut();
        for name in [
            "frame",
            "render",
            "overlay_draw",
            "sprite_upload",
            "units_upload",
            "overlay_prepare",
        ] {
            if !frame.spans.iter().any(|span| span.name == name) {
                frame.spans.push(Record {
                    name,
                    cpu_ms: Some(0.0),
                    ..Default::default()
                });
            }
        }
        for name in [
            "overlay_reprepare",
            "overlay_reason_key",
            "overlay_reason_artwork",
            "overlay_reason_engagement",
            "overlay_reason_raid_missing",
        ] {
            if !frame.counts.iter().any(|(key, _)| *key == name) {
                frame.counts.push((name, 0));
            }
        }
        for name in [
            "counters",
            "other",
            "labels",
            "raids",
            "intel",
            "resources",
            "occupation",
            "adjacency",
        ] {
            if !frame.owners.iter().any(|owner| owner.name == name) {
                frame.owners.push(OwnerRecord::new(name));
            }
        }
        let result = line(&frame.spans, &frame.counts, &frame.owners);
        frame.spans.clear();
        frame.counts.clear();
        frame.owners.clear();
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_owner_measurement_preserves_errors_without_touching_frame_storage() {
        let profiler = FrameOwners::with_enabled(false);
        assert!(profiler.origin.is_none());
        let mut calls = 0;
        let (value, sample) = profiler.measure("counters", || {
            calls += 1;
            Err::<(), _>("unchanged error")
        });
        assert_eq!(value, Err("unchanged error"));
        assert_eq!(calls, 1);
        assert!(sample.is_none());
        record_owner_sample(sample);
        FRAME.with(|frame| {
            let frame = frame.borrow();
            assert_eq!(frame.spans.capacity(), 0);
            assert_eq!(frame.counts.capacity(), 0);
            assert_eq!(frame.owners.capacity(), 0);
        });
    }

    #[test]
    fn owner_cpu_formats_main_helper_mixed_zero_and_unavailable() {
        let mut main = OwnerRecord::new("counters");
        main.add(true, Some(0.012));
        let mut helper = OwnerRecord::new("labels");
        helper.add(false, None);
        let mut mixed = OwnerRecord::new("other");
        mixed.add(false, Some(0.2));
        mixed.add(true, Some(0.1));
        assert_eq!(
            line(&[], &[], &[main, helper, mixed, OwnerRecord::new("raids")]),
            "owner_counters=main owner_counters_cpu=0.012 owner_counters_calls=1 owner_labels=helper owner_labels_cpu=unavailable owner_labels_calls=1 owner_other=mixed owner_other_cpu=0.300 owner_other_calls=2 owner_other_main_cpu=0.100 owner_other_helper_cpu=0.200 owner_raids=not_run owner_raids_cpu=0.000 owner_raids_calls=0"
        );
    }

    #[test]
    fn returned_owner_samples_classify_workers_and_reject_stale_or_wrong_receivers() {
        let origin = std::thread::current().id();
        let profiler = FrameOwners {
            origin: Some((origin, 7)),
        };
        let (main_value, main) = profiler.measure("counters", || 23);
        assert_eq!(main_value, 23);
        let main = main.unwrap();
        assert!(main.main);
        let (worker, helper) = std::thread::spawn(move || {
            let (value, sample) = profiler.measure("other", || Err::<(), _>("same error"));
            assert_eq!(value, Err("same error"));
            (std::thread::current().id(), sample.unwrap())
        })
        .join()
        .unwrap();
        assert!(!helper.main);
        let mut frame = Frame {
            epoch: 7,
            ..Default::default()
        };
        adopt_owner(&mut frame, worker, main);
        adopt_owner(&mut frame, origin, OwnerSample { epoch: 6, ..main });
        assert!(frame.owners.is_empty());
        adopt_owner(&mut frame, origin, main);
        adopt_owner(&mut frame, origin, helper);
        assert_eq!(frame.owners.len(), 2);
        assert_eq!(frame.owners[0].main.calls, 1);
        assert_eq!(frame.owners[0].helper.calls, 0);
        assert_eq!(frame.owners[1].main.calls, 0);
        assert_eq!(frame.owners[1].helper.calls, 1);
    }

    #[test]
    fn taken_thread_records_merge_calls_counts_and_owners() {
        let mut frame = Frame::default();
        add(&mut frame.spans, "sprite_upload", 0.1, Some(0.1));
        frame.counts.push(("synthetic_work", 3));
        let mut taken = Frame::default();
        add(&mut taken.spans, "sprite_upload", 0.2, Some(0.15));
        add(&mut taken.spans, "sprite_upload", 0.3, None);
        add(&mut taken.spans, "rc_terrain", 0.0, Some(0.0));
        taken.counts.push(("synthetic_work", 4));
        taken.counts.push(("synthetic_hits", 1));
        let mut owner = OwnerRecord::new("render_owner");
        owner.add(false, Some(2.5));
        taken.owners.push(owner);
        merge_frame_records(
            &mut frame,
            FrameRecords {
                spans: taken.spans,
                counts: taken.counts,
                owners: taken.owners,
            },
        );
        assert_eq!(
            line(&frame.spans, &frame.counts, &frame.owners),
            "mt_sprite_upload=0.600 mt_sprite_upload_cpu=unavailable mt_sprite_upload_calls=3 mt_rc_terrain=0.000 mt_rc_terrain_cpu=0.000 mt_rc_terrain_calls=1 synthetic_work=7 synthetic_hits=1 owner_render_owner=helper owner_render_owner_cpu=2.500 owner_render_owner_calls=1"
        );
    }

    #[test]
    fn frame_cpu_formats_zero_short_and_unavailable_without_filtering() {
        let mut spans = Vec::new();
        add(&mut spans, "render", 0.0, Some(0.0));
        add(&mut spans, "render", 0.012, Some(0.001));
        add(&mut spans, "overlay_prepare", 0.02, None);
        add(&mut spans, "overlay_prepare", 0.03, Some(0.01));
        assert_eq!(
            line(&spans, &[("overlay_reprepare", 1)], &[]),
            "mt_render=0.012 mt_render_cpu=0.001 mt_render_calls=2 mt_overlay_prepare=0.050 mt_overlay_prepare_cpu=unavailable mt_overlay_prepare_calls=2 overlay_reprepare=1"
        );
    }
}
