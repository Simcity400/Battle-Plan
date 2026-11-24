use core_cursor::reference::{Shape, ShapedLog};
use core_cursor::{
    Cursor, CursorOptions, CursorStats, FrameBudgets, KeyFilter, Memory, Owner, PLAYBACK_BYTES,
    thread_cycles,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

thread_local! {static TRACK:Cell<bool>=const{Cell::new(false)};static ALLOCATIONS:Cell<usize>=const{Cell::new(0)};}
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn allocation() {
    let _ = TRACK.try_with(|track| {
        if track.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(pointer, layout, bytes) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[cfg(windows)]
mod pacing {
    use super::*;
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateWaitableTimerExW(
            attributes: *mut c_void,
            name: *const u16,
            flags: u32,
            access: u32,
        ) -> *mut c_void;
        fn SetWaitableTimer(
            timer: *mut c_void,
            due: *const i64,
            period: i32,
            completion: *mut c_void,
            argument: *mut c_void,
            resume: i32,
        ) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    pub struct Pacer(*mut c_void);
    impl Pacer {
        pub fn new() -> io::Result<Self> {
            let timer = unsafe {
                CreateWaitableTimerExW(std::ptr::null_mut(), std::ptr::null(), 2, 0x001f0003)
            };
            if timer.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(timer))
            }
        }
        pub fn until(&self, deadline: Instant) -> io::Result<()> {
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                let ticks = wait.as_nanos().div_ceil(100).min(i64::MAX as u128) as i64;
                let due = -ticks.max(1);
                if unsafe {
                    SetWaitableTimer(
                        self.0,
                        &raw const due,
                        0,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                    )
                } == 0
                {
                    return Err(io::Error::last_os_error());
                }
                if unsafe { WaitForSingleObject(self.0, u32::MAX) } != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }
    impl Drop for Pacer {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
#[cfg(not(windows))]
mod pacing {
    use super::*;
    pub struct Pacer;
    impl Pacer {
        pub fn new() -> io::Result<Self> {
            Ok(Self)
        }
        pub fn until(&self, deadline: Instant) -> io::Result<()> {
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            Ok(())
        }
    }
}

struct Measurement {
    samples: Vec<Duration>,
    switches: Vec<Duration>,
    full_refreshes: usize,
    allocations: usize,
    cycles: Option<u64>,
    drops: usize,
    skipped: usize,
    over: usize,
    window_min: f64,
    window_sum: f64,
    window_max: f64,
    bytes_min: usize,
    bytes_sum: u128,
    bytes_max: usize,
    stats: CursorStats,
    start: CursorStats,
}
impl Measurement {
    fn new(frames: usize, start: CursorStats) -> Self {
        Self {
            samples: Vec::with_capacity(frames),
            switches: Vec::with_capacity(frames),
            full_refreshes: 0,
            allocations: 0,
            cycles: thread_cycles().map(|_| 0),
            drops: 0,
            skipped: 0,
            over: 0,
            window_min: f64::INFINITY,
            window_sum: 0.0,
            window_max: 0.0,
            bytes_min: usize::MAX,
            bytes_sum: 0,
            bytes_max: 0,
            stats: start,
            start,
        }
    }
}
struct Client {
    name: String,
    cursor: Cursor<ShapedLog>,
    map: bool,
    result: Measurement,
    origin: i32,
}
fn options(memory: &Memory, speed: f64, refresh: f64) -> CursorOptions {
    CursorOptions {
        memory: memory.clone(),
        working_set: memory.limit(),
        speed,
        refresh_hz: refresh,
    }
}
fn cycles(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".into(), |value| value.to_string())
}
fn p99(samples: &mut [Duration]) -> Duration {
    samples.sort_unstable();
    if samples.is_empty() {
        Duration::ZERO
    } else {
        samples[((samples.len() - 1) as f64 * 0.99).ceil() as usize]
    }
}

fn paced(clients: &mut [Client], refresh: f64, hours: i32, reverse: bool) -> io::Result<()> {
    let budgets = FrameBudgets::from_refresh(refresh)?;
    let speed = if reverse { -1440.0 } else { 1440.0 };
    let duration = Duration::from_secs_f64(f64::from(hours) / 1440.0);
    let capacity = (duration.as_secs_f64() * refresh).ceil() as usize + 1;
    for client in clients.iter_mut() {
        client.origin = client.cursor.values().hour();
        client.cursor.working_memory().reset_peaks();
        client.cursor.playback(speed, refresh)?;
        client.result = Measurement::new(capacity, client.cursor.stats());
    }
    let pacer = pacing::Pacer::new()?;
    let start = Instant::now();
    let mut tick = 1u64;
    loop {
        let deadline = start + budgets.period.mul_f64(tick as f64);
        if deadline > start + duration {
            break;
        }
        pacer.until(deadline)?;
        let elapsed = start.elapsed();
        if elapsed > duration + budgets.period {
            break;
        }
        for client in clients.iter_mut() {
            let distance = (start.elapsed().as_secs_f64() * 1440.0)
                .floor()
                .min(f64::from(hours)) as i32;
            let target = if reverse {
                client.origin - distance
            } else {
                client.origin + distance
            };
            ALLOCATIONS.with(|count| count.set(0));
            TRACK.with(|track| track.set(true));
            let cpu = thread_cycles();
            let wall = Instant::now();
            let outcome = if reverse {
                client.cursor.rewind(target)
            } else {
                client.cursor.advance(target)
            };
            let elapsed = wall.elapsed();
            let end_cpu = thread_cycles();
            let applied =
                outcome.map(|delta| (delta.applied, delta.scope_switch, delta.full_refresh));
            TRACK.with(|track| track.set(false));
            let (applied, scope_switch, full_refresh) = applied?;
            if scope_switch {
                client.result.switches.push(elapsed);
            }
            client.result.full_refreshes += usize::from(full_refresh);
            client.result.allocations += ALLOCATIONS.with(Cell::get);
            client.result.cycles = client
                .result
                .cycles
                .zip(cpu.zip(end_cpu))
                .map(|(total, (first, last))| total + last.saturating_sub(first));
            let budget = if client.map {
                budgets.map_decode
            } else {
                budgets.widget_advance
            };
            client.result.over += usize::from(elapsed > budget);
            client.result.samples.push(elapsed);
            let lag = (i64::from(target) - i64::from(client.cursor.values().hour())).unsigned_abs()
                as f64;
            client.result.drops +=
                usize::from(!applied || lag > 1440.0 / refresh || elapsed > budget);
            let stats = client.cursor.stats();
            let window = stats.window_wall_seconds;
            client.result.window_min = client.result.window_min.min(window);
            client.result.window_max = client.result.window_max.max(window);
            client.result.window_sum += window;
            client.result.bytes_min = client.result.bytes_min.min(stats.window_bytes);
            client.result.bytes_max = client.result.bytes_max.max(stats.window_bytes);
            client.result.bytes_sum += stats.window_bytes as u128;
            client.result.stats = stats;
        }
        let next =
            (start.elapsed().as_secs_f64() / budgets.period.as_secs_f64()).floor() as u64 + 1;
        let skipped = next.saturating_sub(tick + 1) as usize;
        for client in clients.iter_mut() {
            client.result.skipped += skipped;
        }
        tick = next.max(tick + 1);
    }
    Ok(())
}
fn report_client(report: &mut String, client: &mut Client, refresh: f64, reverse: bool) -> bool {
    header(report);
    let result = &mut client.result;
    let frame_p99 = p99(&mut result.samples);
    let budget = FrameBudgets::from_refresh(refresh).unwrap();
    let budget = if client.map {
        budget.map_decode
    } else {
        budget.widget_advance
    };
    let stats = result.stats;
    let start = result.start;
    let window_min = if result.samples.is_empty() {
        0.0
    } else {
        result.window_min
    };
    let window_mean = if result.samples.is_empty() {
        0.0
    } else {
        result.window_sum / result.samples.len() as f64
    };
    let pass = !result.samples.is_empty()
        && frame_p99 <= budget
        && result.allocations == 0
        && result.drops == 0
        && result.skipped == 0
        && (!client.map || (!result.switches.is_empty() && result.full_refreshes == 0))
        && window_min >= 1.0 / refresh;
    let _ = writeln!(
        report,
        "| {} | {} | {:.0} | {} | {:.3} | {:.3} | {} | {} | {} | {} | {} | {} | {} | {:.4}/{:.4}/{:.4} | {} | {} | {} |",
        client.name,
        if reverse { "reverse" } else { "forward" },
        refresh,
        result.samples.len(),
        frame_p99.as_secs_f64() * 1e6,
        budget.as_secs_f64() * 1e6,
        result.over,
        result.drops,
        result.skipped,
        result.allocations,
        stats.not_resident.saturating_sub(start.not_resident),
        stats.capacity_misses.saturating_sub(start.capacity_misses),
        stats.catch_up_drops.saturating_sub(start.catch_up_drops),
        window_min,
        window_mean,
        result.window_max,
        stats.window_bytes,
        cycles(result.cycles),
        if pass { "PASS" } else { "FAIL" }
    );
    let switch_count = result.switches.len();
    let switch_p99 = p99(&mut result.switches);
    let switch_max = result.switches.last().copied().unwrap_or_default();
    let _ = writeln!(
        report,
        "\n{} {} scope-switch frames: {}; p99 {:.3} us, max {:.3} us / {:.3} us budget; full refreshes {}. All rows use the change-log typed-varint column layout; no fixed-width decoder rows contribute to PASS.\n",
        client.name,
        if reverse { "reverse" } else { "forward" },
        switch_count,
        switch_p99.as_secs_f64() * 1e6,
        switch_max.as_secs_f64() * 1e6,
        budget.as_secs_f64() * 1e6,
        result.full_refreshes
    );
    let _ = writeln!(
        report,
        "\n{} {} charged page bytes min/mean/max: {}/{}/{}; final {}; source directory upper bound for the final window: {} bytes.\n",
        client.name,
        if reverse { "reverse" } else { "forward" },
        if result.samples.is_empty() {
            0
        } else {
            result.bytes_min
        },
        result
            .bytes_sum
            .checked_div(result.samples.len() as u128)
            .unwrap_or(0),
        result.bytes_max,
        stats.window_bytes,
        stats.directory_bound_bytes,
    );
    let _ = writeln!(
        report,
        "\n{} {} reader: page-read {} cycles / {:.3} ms wall, inverse-fill {} cycles / {:.3} ms wall, catch-up restore {} cycles, {} compact bytes read, {} pages, {} catch-ups, {} window shortenings, achieved final {} game hours.\n",
        client.name,
        if reverse { "reverse" } else { "forward" },
        cycles(thread_cycles().map(|_| {
            stats
                .page_read_cycles
                .saturating_sub(start.page_read_cycles)
        })),
        stats
            .page_read_wall_ns
            .saturating_sub(start.page_read_wall_ns) as f64
            / 1e6,
        cycles(thread_cycles().map(|_| {
            stats
                .reverse_build_cycles
                .saturating_sub(start.reverse_build_cycles)
        })),
        stats
            .reverse_build_wall_ns
            .saturating_sub(start.reverse_build_wall_ns) as f64
            / 1e6,
        cycles(thread_cycles().map(|_| stats.restore_cycles.saturating_sub(start.restore_cycles))),
        stats.decoded_bytes.saturating_sub(start.decoded_bytes),
        stats.pages_read.saturating_sub(start.pages_read),
        stats.catch_ups.saturating_sub(start.catch_ups),
        stats
            .shortened_windows
            .saturating_sub(start.shortened_windows),
        stats.window_hours
    );
    let _ = writeln!(
        report,
        "\n{} reader cycles: total {}, planning {}, snapshot rebuild/publish {}, frame preparation {}, publish retries {}, scope handoffs {}.\n",
        client.name,
        stats.worker_cycles.saturating_sub(start.worker_cycles),
        stats.planning_cycles.saturating_sub(start.planning_cycles),
        stats
            .publication_cycles
            .saturating_sub(start.publication_cycles),
        stats
            .preparation_cycles
            .saturating_sub(start.preparation_cycles),
        stats.publish_retries.saturating_sub(start.publish_retries),
        stats.scope_changes.saturating_sub(start.scope_changes)
    );
    let _ = writeln!(
        report,
        "\n{} cursor owners (shared forward pages are charged to the playback root):",
        client.name
    );
    owners(report, client.cursor.working_memory());
    pass
}
fn header(report: &mut String) {
    report.push_str("\n| Client | Direction | Hz | Frames | p99 us | Budget us | Over | Drops | Skipped presents | Allocs | Not resident | Capacity | Catch-up drops | Achieved window s min/mean/max | Resident bytes | Frame cycles | Result |\n|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|\n");
}
fn owners(report: &mut String, memory: &Memory) {
    report.push_str("\nMap state residency and peaks measure the fixture's packed `NumericState` and do not establish the real map store's memory cost.\n");
    let _ = writeln!(
        report,
        "\nResident bytes by owner: state {}, forward {}, reverse {}, delta {}, metadata {}, transient {}.\n",
        memory.owner_used(Owner::State),
        memory.owner_used(Owner::Forward),
        memory.owner_used(Owner::Reverse),
        memory.owner_used(Owner::Delta),
        memory.owner_used(Owner::Metadata),
        memory.owner_used(Owner::Transient)
    );
    let _ = writeln!(
        report,
        "\nLedger peak {} / {} bytes; owner peaks: state {}, compact forward {}, inverse {}, delta {}, metadata {}, transient {}.\n",
        memory.peak(),
        memory.limit(),
        memory.owner_peak(Owner::State),
        memory.owner_peak(Owner::Forward),
        memory.owner_peak(Owner::Reverse),
        memory.owner_peak(Owner::Delta),
        memory.owner_peak(Owner::Metadata),
        memory.owner_peak(Owner::Transient)
    );
}
fn seeks(
    report: &mut String,
    name: &str,
    source: &Arc<ShapedLog>,
    keys: &KeyFilter,
    memory: &Memory,
    count: usize,
    map: bool,
) -> io::Result<bool> {
    let mut samples = Vec::with_capacity(count);
    let mut total_cycles = thread_cycles().map(|_| 0);
    let initial_reads = source
        .checkpoint_reads
        .load(std::sync::atomic::Ordering::Relaxed);
    for index in 0..count {
        let hour = ((index as u64 * (source.shape().hours - 1) as u64) / count as u64) as i32;
        let cpu = thread_cycles();
        let wall = Instant::now();
        let cursor = Cursor::open_with(source, keys.clone(), hour, options(memory, 0.0, 0.0))?;
        samples.push(wall.elapsed());
        total_cycles = total_cycles
            .zip(cpu.zip(thread_cycles()))
            .map(|(total, (first, last))| total + last.saturating_sub(first));
        drop(cursor);
    }
    let p99 = p99(&mut samples);
    let reads = source
        .checkpoint_reads
        .load(std::sync::atomic::Ordering::Relaxed)
        - initial_reads;
    let budget_ms = if map { 50 } else { 10 };
    let _ = writeln!(
        report,
        "\n{name} seeks: {count} opens; p99 {:.3} ms / {budget_ms} ms, max {:.3} ms; {} frame-thread cycles; {reads} checkpoint-version reads plus compact block replay.\n",
        p99.as_secs_f64() * 1e3,
        samples.last().copied().unwrap_or_default().as_secs_f64() * 1e3,
        cycles(total_cycles)
    );
    Ok(p99 <= Duration::from_millis(budget_ms) && reads >= count)
}
fn isolated(
    report: &mut String,
    name: &str,
    source: &Arc<ShapedLog>,
    keys: KeyFilter,
    map: bool,
    memory: &Memory,
    refresh: f64,
    hours: i32,
    seek_count: usize,
) -> io::Result<bool> {
    memory.reset_peaks();
    let mut pass = seeks(report, name, source, &keys, memory, seek_count, map)?;
    let origin = (source.shape().hours - hours) / 2;
    let cursor = Cursor::open_with(source, keys, origin, options(memory, 1440.0, refresh))?;
    cursor.wait_read_ahead()?;
    let mut clients = [Client {
        name: name.into(),
        origin,
        cursor,
        map,
        result: Measurement::new(0, CursorStats::default()),
    }];
    for reverse in [false, true] {
        memory.reset_peaks();
        paced(&mut clients, refresh, hours, reverse)?;
        pass &= report_client(report, &mut clients[0], refresh, reverse);
        owners(report, memory);
    }
    Ok(pass)
}
fn concurrent(
    report: &mut String,
    widgets: &Arc<ShapedLog>,
    map: &Arc<ShapedLog>,
    memory: &Memory,
    refresh: f64,
    hours: i32,
) -> io::Result<bool> {
    memory.reset_peaks();
    let origin = (map.shape().hours - hours) / 2;
    let mut clients = Vec::with_capacity(11);
    for index in 0..10 {
        let size = if index % 2 == 0 { 10 } else { 1000 };
        let cursor = Cursor::open_with(
            widgets,
            widgets.filter(size)?,
            origin,
            options(memory, 1440.0, refresh),
        )?;
        clients.push(Client {
            name: format!("page-widget-{index}-{size}"),
            origin,
            cursor,
            map: false,
            result: Measurement::new(0, CursorStats::default()),
        });
    }
    clients.push(Client {
        name: "page-map".into(),
        origin,
        cursor: Cursor::open_with(
            map,
            KeyFilter::All,
            origin,
            options(memory, 1440.0, refresh),
        )?,
        map: true,
        result: Measurement::new(0, CursorStats::default()),
    });
    for client in &clients {
        client.cursor.wait_read_ahead()?;
    }
    let mut pass = true;
    for reverse in [false, true] {
        memory.reset_peaks();
        paced(&mut clients, refresh, hours, reverse)?;
        for client in &mut clients {
            pass &= report_client(report, client, refresh, reverse);
        }
        owners(report, memory);
    }
    Ok(pass && memory.peak() <= memory.limit())
}
fn main() -> io::Result<()> {
    let mut output = std::env::temp_dir().join("hoi4-core-cursor-bench");
    let mut hours = 43_200;
    let mut seek_count = 16usize;
    let mut arguments = std::env::args().skip(1);
    while let Some(option) = arguments.next() {
        let value = arguments
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing option value"))?;
        match option.as_str() {
            "--out" => output = value.into(),
            "--hours" => hours = value.parse().map_err(io::Error::other)?,
            "--seeks" => seek_count = value.parse().map_err(io::Error::other)?,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected --out, --hours or --seeks",
                ));
            }
        }
    }
    let campaign = 13 * 365 * 24;
    if hours < 43_200 || hours > campaign / 2 || seek_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid measurement span",
        ));
    }
    std::fs::create_dir_all(&output)?;
    let run = output.join(format!(
        "fixtures-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos()
    ));
    std::fs::create_dir(&run)?;
    let memory = Memory::new(PLAYBACK_BYTES)?;
    let mut report = String::from(
        "# core-cursor measurement\n\n13-year synthetic compact logs (113,880 game hours). Factory products hold 51,000 addresses: daily updates of all addresses, and an hourly product changing 90% (45,900). Widgets are genuine spaced filters of 10 or 1,000 addresses from those products. The map has 600,000 live field addresses (60,000 static addresses plus 540,000 unit field addresses) and 3,000 changes per hour, including ten retirements and ten newly created addresses per hour: 1,138,800 births and retirements over the campaign. Every timing row uses the change-log numeric schema: 12 typed columns, delta-varint hours, key and field varints, type and break flags, inline length-prefixed zigzag fixed-point values (scale 8), and a worker-built 17-byte row index. These synthetic records have no provenance. Per-hour runs are key sorted. The worker maps and validates the source key column once per selection; the frame gallops those varint keys against the page key column without an identity shortcut or per-frame conversion; map frames merge about 15 interleaved key-sorted runs at 95 Hz.\n\nSetup writes a version root at each byte-bounded product block and every 168 hours for the map. Repeating synthetic checkpoint columns are shared by those roots; their stored prior/current records and split-key ordinal resolve a block cut. All checkpoint columns are serialized before timing. Opens read the checkpoint file for selected addresses and replay compact pages to the cut; they never calculate current values directly. Playback is paced by absolute display deadlines at 95 and 144 Hz (experiment inputs). Each presentation uses the clock's current hour, never retries a miss, and includes catch-up and direction-change drops. Windows are measured from the published resident set on every frame. Frame CPU and worker CPU use QueryThreadCycleTime, reported as cycles without a guessed frequency. Windows pacing uses a high-resolution waitable timer.\n\nThe concurrent page runs ten widgets sharing one product and reader, together with the map, under one 400,000,000-byte root. These are core decode and memory measurements; rendering and real recording I/O belong to the app playback gate. Every direction runs at least 30 seconds (15 requested windows). PASS requires the p99 budget, zero allocations, no drops or skipped presentations and a minimum achieved window of at least one display period. Requested windows are 2 seconds; memory pressure may shorten them. Owner bytes and peaks are reported after each direction. Map state residency and peaks measure the fixture's packed `NumericState` and do not establish the real map store's memory cost. All timing rows include the typed layout decode, rather than the former simple fixed-width decoder. Scope-switch frame p99 and maximum are reported separately; the map also requires observed scope switches and zero full refreshes. Dense reverse chunks span at least a display frame of game hours, restore once at their first block, and replay through intervening block versions; map chunks also keep a checkpoint run together. The fixture still cuts a version at every product block start (section 2.5), including blocks that split an hour, so the hourly 1,000-key reverse rows measure the cursor over that layout.\n",
    );
    if thread_cycles().is_none() {
        report.push_str("\nPer-thread cycle counters unavailable on this platform; CPU is reported as unavailable.\n");
    }
    let daily = Arc::new(ShapedLog::create(
        Shape {
            hours: campaign,
            keys: 51_000,
            changes_per_tick: 51_000,
            tick_hours: 24,
            lifetimes: false,
            live_units: 0,
            churn_per_tick: 0,
        },
        &memory,
        &run.join("daily.checkpoints"),
    )?);
    let hourly = Arc::new(ShapedLog::create(
        Shape {
            hours: campaign,
            keys: 51_000,
            changes_per_tick: 45_900,
            tick_hours: 1,
            lifetimes: false,
            live_units: 0,
            churn_per_tick: 0,
        },
        &memory,
        &run.join("hourly.checkpoints"),
    )?);
    let map = Arc::new(ShapedLog::create(
        Shape {
            hours: campaign,
            keys: 600_000,
            changes_per_tick: 3000,
            tick_hours: 1,
            lifetimes: true,
            live_units: 540_000,
            churn_per_tick: 10,
        },
        &memory,
        &run.join("map.checkpoints"),
    )?);
    let mut pass = true;
    for refresh in [95.0, 144.0] {
        for (product, source) in [("daily", &daily), ("hourly-90pct", &hourly)] {
            for size in [10, 1000] {
                let name = format!("{product}-widget-{size}");
                match isolated(
                    &mut report,
                    &name,
                    source,
                    source.filter(size)?,
                    false,
                    &memory,
                    refresh,
                    hours,
                    seek_count,
                ) {
                    Ok(ok) => pass &= ok,
                    Err(error) => {
                        pass = false;
                        let _ = writeln!(report, "\n{name} {refresh} Hz ERROR: {error}\n");
                    }
                }
            }
        }
        match isolated(
            &mut report,
            "map",
            &map,
            KeyFilter::All,
            true,
            &memory,
            refresh,
            hours,
            seek_count,
        ) {
            Ok(ok) => pass &= ok,
            Err(error) => {
                pass = false;
                let _ = writeln!(report, "\nmap {refresh} Hz ERROR: {error}\n");
            }
        }
        for (name, widgets) in [("daily", &daily), ("hourly-90pct", &hourly)] {
            let _ = writeln!(report, "\nConcurrent page: {name}, {refresh} Hz.\n");
            match concurrent(&mut report, widgets, &map, &memory, refresh, hours) {
                Ok(ok) => pass &= ok,
                Err(error) => {
                    pass = false;
                    let _ = writeln!(report, "\nConcurrent page ERROR: {error}\n");
                }
            }
        }
    }
    let _ = writeln!(
        report,
        "\nBudget result: {}.\nCheckpoint fixtures: {}.\n",
        if pass { "PASS" } else { "FAIL" },
        run.display()
    );
    let path = output.join("report.md");
    std::fs::write(&path, report)?;
    println!("{}: {}", if pass { "PASS" } else { "FAIL" }, path.display());
    if !pass {
        std::process::exit(1);
    }
    Ok(())
}
