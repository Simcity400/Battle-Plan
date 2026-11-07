use dashboard_base::prepared::facts::{Facts, Limits, Namespace, Status};
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
    fs,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct MeasuredAllocator;
#[global_allocator]
static ALLOCATOR: MeasuredAllocator = MeasuredAllocator;
thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; }
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static MEASUREMENTS: Mutex<()> = Mutex::new(());
#[derive(Clone, Copy)]
struct Header {
    tracked: bool,
    bytes: usize,
}
fn layout(original: Layout) -> (Layout, usize) {
    let align = original.align().max(align_of::<Header>());
    let offset = size_of::<Header>().next_multiple_of(align);
    (
        Layout::from_size_align(offset + original.size(), align).unwrap(),
        offset,
    )
}
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, original: Layout) -> *mut u8 {
        let (layout, offset) = layout(original);
        let pointer = unsafe { engine_heap::Heap.alloc(layout) };
        if pointer.is_null() {
            return pointer;
        }
        let tracked = TRACK.try_with(Cell::get).unwrap_or(false);
        unsafe {
            pointer.cast::<Header>().write(Header {
                tracked,
                bytes: layout.size(),
            });
        }
        if tracked {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        unsafe { pointer.add(offset) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, original: Layout) {
        let (layout, offset) = layout(original);
        let base = unsafe { pointer.sub(offset) };
        let header = unsafe { base.cast::<Header>().read() };
        if header.tracked {
            LIVE.fetch_sub(header.bytes, Ordering::Relaxed);
        }
        unsafe {
            engine_heap::Heap.dealloc(base, layout);
        }
    }
}
struct Tracking;
impl Tracking {
    fn new() -> Self {
        TRACK.set(true);
        Self
    }
}
impl Drop for Tracking {
    fn drop(&mut self) {
        TRACK.set(false);
    }
}

#[test]
fn real_allocations_and_checkpoint_lengths_stay_flat_while_history_grows() {
    let _serial = MEASUREMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let allocation_baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(allocation_baseline, Ordering::Relaxed);
    let id = uuid::Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("private-fact-allocation-{id}"));
    fs::create_dir(&root).unwrap();
    let recording = root.join("recording");
    fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(id).unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let limits = Limits {
        write_bytes: 8192,
        page_bytes: 4096,
    };
    let mut resident = Vec::new();
    let mut sizes = Vec::new();
    let mut checkpoint_payloads = Vec::new();
    {
        let _tracking = Tracking::new();
        let mut facts = Facts::<u32, String>::new(
            Namespace::new("allocations", 1, "values").unwrap(),
            limits,
        )
        .unwrap();
        for batch in 0..96 {
            let mut writer = if batch == 0 {
                store.replace(key.clone()).unwrap()
            } else {
                store.extend(&key).unwrap()
            };
            if batch == 0 {
                for country in 0..1500 {
                    facts
                        .append(
                            &mut writer,
                            country,
                            0,
                            if country == 1 { 2 } else { 438_000 },
                            Status::Value(format!("name-{:08}", country)),
                        )
                        .unwrap();
                }
            }
            for step in 0..128 {
                let hour = (batch * 128 + step + 1) * 37;
                facts
                    .append(
                        &mut writer,
                        1,
                        hour,
                        hour + 2,
                        Status::Value(format!("name-{:08}", hour)),
                    )
                    .unwrap();
            }
            let checkpoint = facts.checkpoint(&mut writer, batch as u64).unwrap();
            let serialized = serde_json::to_vec(&checkpoint).unwrap();
            sizes.push(serialized.len());
            drop(serialized);
            checkpoint_payloads.push(
                writer
                    .read_at_spooled(
                        "preparation-facts-v2/allocations/1/values/checkpoints",
                        batch as u64,
                    )
                    .unwrap()
                    .unwrap()
                    .len(),
            );
            {
                let mut reader = facts.reader(&mut writer).unwrap();
                let mut cursor = reader.cursor(&1, (batch * 128 + 1) * 37, i32::MAX).unwrap();
                assert!(reader.at(&1, 0).unwrap().is_some());
                while reader.next(&mut cursor).unwrap().is_some() {}
            }
            writer.commit().unwrap();
            if [0, 7, 31, 95].contains(&batch) {
                resident.push(LIVE.load(Ordering::Relaxed));
            }
        }
    }
    assert!(
        resident.iter().max().unwrap() - resident.iter().min().unwrap() < 8192,
        "actual resident heap grew: {resident:?}"
    );
    assert!(
        sizes.iter().max().unwrap() - sizes.iter().min().unwrap() < 32,
        "checkpoint roots grew: {sizes:?}"
    );
    assert!(
        checkpoint_payloads.iter().max().unwrap() - checkpoint_payloads.iter().min().unwrap()
            < 1024,
        "private checkpoint payloads grew with closed history: {checkpoint_payloads:?}"
    );
    assert!(
        PEAK.load(Ordering::Relaxed) - allocation_baseline < 16 << 20,
        "actual allocation peak grew past 16 MiB"
    );
    store.clear().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retired_key_churn_does_not_create_resident_historical_sections_after_reopen() {
    let _serial = MEASUREMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let id = uuid::Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("private-fact-retirement-{id}"));
    fs::create_dir(&root).unwrap();
    let recording = root.join("recording");
    fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(id).unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let limits = Limits {
        write_bytes: 4096,
        page_bytes: 2048,
    };
    {
        let _tracking = Tracking::new();
        let mut facts = Facts::<u32, u64>::new(
            Namespace::new("churn", 1, "careers").unwrap(),
            limits,
        )
        .unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        for country in 0..1500 {
            facts
                .append(&mut writer, country, 0, 0, Status::Value(country as u64))
                .unwrap();
        }
        facts.checkpoint(&mut writer, 0).unwrap();
        writer = writer.commit_continue().unwrap();
        let resident = LIVE.load(Ordering::Relaxed);
        let mut current: Vec<u32> = (0..1500).collect();
        for batch in 1..=96_u32 {
            for offset in 0..16 {
                let country = ((batch - 1) * 16 + offset) % 1500;
                facts
                    .retire(&mut writer, &current[country as usize])
                    .unwrap();
                let new = 1500 + (batch - 1) * 16 + offset;
                facts
                    .append(
                        &mut writer,
                        new,
                        batch as i32,
                        batch as i32,
                        Status::Value(country as u64),
                    )
                    .unwrap();
                current[country as usize] = new;
            }
            let checkpoint = facts.checkpoint(&mut writer, batch as u64).unwrap();
            writer = writer.commit_continue().unwrap();
            if batch % 8 == 0 {
                drop(facts);
                facts = Facts::restore(&mut writer, &checkpoint, limits).unwrap();
            }
            assert_eq!(facts.active_keys(), 1500);
            let (sections, runs, files, buffer) = writer.resident_metadata();
            assert_eq!(sections, 4);
            assert!(runs <= 4 * 32);
            assert_eq!(buffer, 0);
            let live = LIVE.load(Ordering::Relaxed);
            assert!(
                live <= resident + (128 << 10) + files * 4096,
                "resident key metadata grew beyond backend file tables: live={live}, base={resident}, files={files}"
            );
            assert!(serde_json::to_vec(&checkpoint).unwrap().len() < 256);
        }
        drop(writer);
    }
    store.clear().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_indivisible_spill_has_no_extra_pack_buffer() {
    let _serial = MEASUREMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let id = uuid::Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("private-fact-spill-{id}"));
    fs::create_dir(&root).unwrap();
    let recording = root.join("recording");
    fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(id).unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    {
        let _tracking = Tracking::new();
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        let limit = (16 << 20) + (64 << 10);
        let limits = Limits {
            write_bytes: 128,
            page_bytes: 128,
        };
        let mut facts = Facts::<u32, String>::new(
            Namespace::new("tight", 1, "names").unwrap(),
            limits,
        )
        .unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        facts
            .append(&mut writer, 1, 0, 0, Status::Value("x".repeat(8 << 20)))
            .unwrap();
        facts.retire(&mut writer, &1).unwrap();
        assert_eq!(facts.active_keys(), 0);
        assert_eq!(writer.resident_metadata().3, 0);
        assert!(
            PEAK.load(Ordering::Relaxed) - baseline <= limit,
            "actual spill allocation held more than the value spilled"
        );
        writer.commit().unwrap();
    }
    store.clear().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn commit_peak_stays_bounded_while_a_fact_history_grows() {
    let _serial = MEASUREMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let id = uuid::Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("private-fact-commit-{id}"));
    fs::create_dir(&root).unwrap();
    let recording = root.join("recording");
    fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(id).unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    {
        let _tracking = Tracking::new();
        let limits = Limits {
            write_bytes: 4096,
            page_bytes: 2048,
        };
        let mut facts = Facts::<u32, u64>::new(
            Namespace::new("compaction", 1, "latest").unwrap(),
            limits,
        )
        .unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        let mut scratch = [0; 8192];
        for batch in 1..=128_i32 {
            for step in 0..256 {
                let hour = (batch - 1) * 256 + step;
                facts
                    .append(&mut writer, 1, hour, hour, Status::Value(hour as u64))
                    .unwrap();
            }
            facts.checkpoint(&mut writer, batch as u64).unwrap();
            let baseline = LIVE.load(Ordering::Relaxed);
            PEAK.store(baseline, Ordering::Relaxed);
            writer = writer.commit_continue_with_scratch(&mut scratch).unwrap();
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
            let (_, _, files, buffer) = writer.resident_metadata();
            assert_eq!(buffer, 0);
            assert!(
                peak <= (128 << 10) + files * 4096,
                "commit peak follows historical ranges: batch={batch}, peak={peak}, files={files}"
            );
        }
        let mut reader = facts.reader(&mut writer).unwrap();
        assert!(reader.at(&1, 0).unwrap().is_some());
        assert!(reader.at(&1, 128 * 256 - 1).unwrap().is_some());
    }
    store.clear().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pack_relocation_and_scattered_rewrites_do_not_materialize_historical_entries() {
    let _serial = MEASUREMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let id = uuid::Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("private-fact-relocation-{id}"));
    fs::create_dir(&root).unwrap();
    let recording = root.join("recording");
    fs::write(&recording, b"synthetic").unwrap();
    let store = store::Store::local(id).unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    {
        use std::io::Read;
        let _tracking = Tracking::new();
        let mut scratch = [0; 8192];
        let mut writer = store.replace(key.clone()).unwrap();
        writer
            .write_with_scratch("heavy", std::io::repeat(0).take(2 << 20), &mut scratch)
            .unwrap();
        for range in 0..32768u64 {
            writer
                .write_with_scratch("pages", &(range.to_le_bytes())[..], &mut scratch)
                .unwrap();
        }
        writer = writer.commit_continue_with_scratch(&mut scratch).unwrap();
        let original = writer.read_at("pages", 0).unwrap().unwrap().id();
        writer.write_at("heavy", 0, &b"new"[..]).unwrap();
        for pass in 0..2 {
            if pass != 0 {
                for range in (0..32768u64).step_by(512) {
                    writer.write_at("pages", range, &b"changed"[..]).unwrap();
                }
            }
            let baseline = LIVE.load(Ordering::Relaxed);
            PEAK.store(baseline, Ordering::Relaxed);
            writer = writer.commit_continue_with_scratch(&mut scratch).unwrap();
            let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
            let (_, _, files, buffer) = writer.resident_metadata();
            assert_eq!(buffer, 0);
            assert!(
                peak <= (128 << 10) + files * 4096,
                "history-sized relocation/rewrite: pass={pass}, peak={peak}"
            );
            assert_ne!(
                writer
                    .read_at("pages", 0)
                    .unwrap()
                    .unwrap()
                    .id()
                    .cache_key()[0],
                original.cache_key()[0]
            );
            let mut input = writer.read_at("pages", 32767).unwrap().unwrap();
            let mut bytes = [0; 8];
            input.read_exact(&mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), 32767);
        }
    }
    store.clear().unwrap();
    fs::remove_dir_all(root).unwrap();
}
