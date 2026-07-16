//! Real WMI `ProcessEventSource` (design §2 "Watcher: WMI plus bounded
//! reconciliation").
//!
//! This is the lower-latency half of the dual-order watcher. It subscribes to
//! standard-user `__InstanceCreationEvent` and `__InstanceDeletionEvent` queries
//! via `IWbemServices::ExecNotificationQuery`, each restricted to a
//! `TargetInstance` that is a `Win32_Process`. Two enumerators deliver start and
//! stop hints and the embedded target supplies the PID and executable name. The
//! privileged `Win32_ProcessTrace` provider is deliberately not used because it
//! returns `WBEM_E_ACCESS_DENIED` for a normal user on the supported host.
//! Bounded process reconciliation remains authoritative.
//!
//! ## No new dependency
//!
//! Per this lane's fence and the machine-wide network ban, NO `windows`/
//! `windows-sys`/`wmi` crate is added (none is in `Cargo.lock`). The COM binding
//! is hand-written raw `extern "system"` FFI against the system `ole32` /
//! `oleaut32` import libraries, exactly mirroring the established style in
//! `modset.rs` and `sources.rs`. Design §9 asked for a pinned crate binding; the
//! FORBIDDEN-network constraint makes raw FFI the only expressible path, and it
//! is functionally sufficient here.
//!
//! ## Testability seam
//!
//! Everything that touches COM lives behind the [`WmiEventPump`] trait. The pump
//! loop ([`pump_iteration`]), health transitions, queueing, WMI-loss degradation,
//! and recovery are driven by a fake pump in unit tests with zero live WMI. The
//! real COM pump is exercised only by an `#[ignore]`d live smoke test.
//!
//! ## Fail-closed / honest degradation
//!
//! Health starts `Degraded` and only flips to `Healthy` after a subscription is
//! proven established. If the provider is unavailable the source stays `Degraded`
//! forever (the coordinator then runs on the bounded 2 s snapshot reconcile — the
//! design's WMI-loss fallback). A lost enumerator degrades to `Degraded` and the
//! loop attempts bounded-backoff re-subscription; recovery flips back to
//! `Healthy`. A timeout is a timeout: no event is ever fabricated.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{ProcessEventSource, RawProcessEvent};
use crate::auto_attach::policy::WatcherHealth;

/// Blocking poll timeout for the notification enumerator. Bounds both cancel
/// responsiveness and CPU when idle (the real `Next` blocks for this long).
const POLL_TIMEOUT_MS: u32 = 500;
/// Bounded backoff between re-subscription attempts after a provider loss. Kept
/// at the degraded reconcile cadence so the watcher's failure cost is bounded.
const RESUBSCRIBE_BACKOFF_MS: u64 = 2_000;
/// Upper bound on how long `new()` blocks for the first subscription attempt.
/// On timeout the source returns with whatever health has been set so far
/// (`Degraded` by default) — fail-closed; the thread keeps trying and may flip
/// to `Healthy` later.
const READY_TIMEOUT_MS: u64 = 5_000;

const HEALTH_HEALTHY: u8 = 0;
const HEALTH_DEGRADED: u8 = 1;

fn encode_health(h: WatcherHealth) -> u8 {
    match h {
        WatcherHealth::Healthy => HEALTH_HEALTHY,
        WatcherHealth::Degraded => HEALTH_DEGRADED,
    }
}

fn decode_health(v: u8) -> WatcherHealth {
    if v == HEALTH_HEALTHY {
        WatcherHealth::Healthy
    } else {
        WatcherHealth::Degraded
    }
}

/// One raw poll result from the WMI enumerator, before `hoi4.exe` filtering (the
/// coordinator filters names). `Lost` means the subscription/connection is dead
/// and the caller must degrade and re-subscribe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WmiPoll {
    Event {
        class: String,
        pid: u32,
        name: String,
    },
    Timeout,
    Lost,
}

/// The COM-touching seam. The real implementation is the raw-FFI COM pump; tests
/// inject a scripted fake. NOT `Send`: the real pump's COM pointers live entirely
/// on the watcher thread (constructed, used, and dropped there), so a pump value
/// never crosses a thread boundary — only the `Send` factory closure does.
pub trait WmiEventPump {
    /// Establish (or re-establish) the notification subscription. `true` means
    /// the enumerator is live; `false` means the provider is currently
    /// unavailable (the loop degrades and backs off).
    fn resubscribe(&mut self) -> bool;
    /// Block up to `timeout_ms` for the next raw process-trace event.
    fn poll(&mut self, timeout_ms: u32) -> WmiPoll;
}

/// Map a raw WMI process event to a coordinator event by its `__CLASS`.
/// Pure and cross-platform so it is unit-tested directly.
pub(crate) fn classify_process_trace(class: &str, pid: u32, name: &str) -> Option<RawProcessEvent> {
    match class {
        "Win32_ProcessStartTrace" | "__InstanceCreationEvent" => Some(RawProcessEvent::Start {
            pid,
            name: name.to_string(),
        }),
        "Win32_ProcessStopTrace" | "__InstanceDeletionEvent" => Some(RawProcessEvent::Stop {
            pid,
            name: name.to_string(),
        }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IterOutcome {
    /// Made progress or idled; loop again immediately (the pump's own poll
    /// blocks, so this is not a busy loop with the real pump).
    Progressed,
    /// Disconnected / lost; caller should back off before retrying.
    Backoff,
}

/// One watcher iteration over an injected pump. Synchronously testable: no
/// threads, no sleeps, no COM.
fn pump_iteration<P: WmiEventPump>(
    pump: &mut P,
    connected: &mut bool,
    shared: &SharedWmiState,
) -> IterOutcome {
    if !*connected {
        if pump.resubscribe() {
            *connected = true;
            shared.set_health(WatcherHealth::Healthy);
            return IterOutcome::Progressed;
        }
        shared.set_health(WatcherHealth::Degraded);
        return IterOutcome::Backoff;
    }
    match pump.poll(POLL_TIMEOUT_MS) {
        WmiPoll::Timeout => IterOutcome::Progressed,
        WmiPoll::Event { class, pid, name } => {
            if let Some(ev) = classify_process_trace(&class, pid, &name) {
                shared.push(ev);
            }
            IterOutcome::Progressed
        }
        WmiPoll::Lost => {
            *connected = false;
            shared.set_health(WatcherHealth::Degraded);
            IterOutcome::Backoff
        }
    }
}

/// State shared between the watcher thread and the public source handle.
struct SharedWmiState {
    queue: Mutex<VecDeque<RawProcessEvent>>,
    health: AtomicU8,
}

impl SharedWmiState {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            // Fail-closed: unknown until a subscription is proven established.
            health: AtomicU8::new(HEALTH_DEGRADED),
        }
    }
    fn set_health(&self, h: WatcherHealth) {
        self.health.store(encode_health(h), Ordering::Release);
    }
    fn health(&self) -> WatcherHealth {
        decode_health(self.health.load(Ordering::Acquire))
    }
    fn push(&self, ev: RawProcessEvent) {
        if let Ok(mut q) = self.queue.lock() {
            q.push_back(ev);
        }
    }
    fn drain(&self) -> Vec<RawProcessEvent> {
        match self.queue.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Sleep up to `total_ms`, waking early if cancellation is requested.
fn interruptible_sleep(total_ms: u64, cancel: &AtomicBool) {
    const STEP_MS: u64 = 50;
    let mut slept = 0u64;
    while slept < total_ms {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let chunk = STEP_MS.min(total_ms - slept);
        std::thread::sleep(Duration::from_millis(chunk));
        slept += chunk;
    }
}

/// The watcher loop over a pump built by `factory` (run ON the watcher thread so
/// a non-`Send` COM pump never crosses a boundary). Signals `ready` exactly once,
/// after the first subscription attempt, so the constructor can block until the
/// subscribe/enumerate race is closed.
fn run_pump_loop<F, P>(
    factory: F,
    shared: Arc<SharedWmiState>,
    cancel: Arc<AtomicBool>,
    ready: SyncSender<()>,
) where
    F: FnOnce() -> Option<P>,
    P: WmiEventPump,
{
    let mut pump = match factory() {
        Some(p) => p,
        None => {
            shared.set_health(WatcherHealth::Degraded);
            let _ = ready.try_send(());
            return;
        }
    };
    let mut connected = false;
    let mut signalled = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }
        let outcome = pump_iteration(&mut pump, &mut connected, &shared);
        if !signalled {
            let _ = ready.try_send(());
            signalled = true;
        }
        if matches!(outcome, IterOutcome::Backoff) {
            interruptible_sleep(RESUBSCRIBE_BACKOFF_MS, &cancel);
        }
    }
    // `pump` drops here (releasing COM pointers) BEFORE any caller-side COM
    // uninitialize on the same thread.
}

/// A resident WMI process-event watcher. Owns a dedicated thread that holds the
/// blocking notification enumerator and pushes classified start/stop events into
/// a shared queue drained by [`ProcessEventSource::drain_events`].
pub struct WmiProcessEventSource {
    shared: Arc<SharedWmiState>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl WmiProcessEventSource {
    /// Spawn the watcher thread with a caller-supplied thread body. The body runs
    /// on the new thread and must (eventually) call `ready.try_send(())` so the
    /// constructor can stop blocking. Bounded: the constructor waits at most
    /// [`READY_TIMEOUT_MS`].
    fn spawn_thread<G>(body: G) -> Self
    where
        G: FnOnce(Arc<SharedWmiState>, Arc<AtomicBool>, SyncSender<()>) + Send + 'static,
    {
        let shared = Arc::new(SharedWmiState::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = sync_channel::<()>(1);
        let handle = {
            let shared = shared.clone();
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("wmi-process-events".to_string())
                .spawn(move || body(shared, cancel, tx))
                .ok()
        };
        // Block until the first subscription attempt reports in (Healthy or
        // Degraded), closing the design §2 subscribe/enumerate race: once `new()`
        // returns, any process that starts afterwards is caught by WMI and the
        // coordinator's startup snapshot catches anything already running.
        let _ = rx.recv_timeout(Duration::from_millis(READY_TIMEOUT_MS));
        Self {
            shared,
            cancel,
            handle,
        }
    }

    /// Generic constructor for tests: drive the loop with an injected pump
    /// factory (no COM). Also usable by any caller that provides its own pump.
    pub fn with_pump_factory<F, P>(factory: F) -> Self
    where
        F: FnOnce() -> Option<P> + Send + 'static,
        P: WmiEventPump + 'static,
    {
        Self::spawn_thread(move |shared, cancel, ready| {
            run_pump_loop(factory, shared, cancel, ready)
        })
    }

    /// Production constructor: the real standard-user COM process subscriptions.
    #[cfg(windows)]
    pub fn new() -> Self {
        Self::spawn_thread(|shared, cancel, ready| com::run_com_watcher(shared, cancel, ready))
    }
}

impl ProcessEventSource for WmiProcessEventSource {
    fn drain_events(&mut self) -> Vec<RawProcessEvent> {
        self.shared.drain()
    }
    fn health(&self) -> WatcherHealth {
        self.shared.health()
    }
}

impl Drop for WmiProcessEventSource {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// =====================================================================
// Real COM implementation (raw FFI; Windows only).
// =====================================================================

#[cfg(windows)]
#[allow(dead_code)] // vtable/GUID/VARIANT fields exist for layout, not all read.
mod com {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::SyncSender;
    use std::sync::Arc;

    use super::{run_pump_loop, SharedWmiState, WmiEventPump, WmiPoll};

    type Hresult = i32;
    type Bstr = *mut u16;

    // --- COINIT / CLSCTX / RPC / WBEM constants -----------------------
    const COINIT_MULTITHREADED: u32 = 0x0;
    const RPC_E_CHANGED_MODE: Hresult = 0x8001_0106u32 as i32;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const RPC_C_AUTHN_WINNT: u32 = 10;
    const RPC_C_AUTHZ_NONE: u32 = 0;
    const RPC_C_AUTHN_LEVEL_DEFAULT: u32 = 0;
    const RPC_C_AUTHN_LEVEL_CALL: u32 = 3;
    const RPC_C_IMP_LEVEL_IMPERSONATE: u32 = 3;
    const EOAC_NONE: u32 = 0;
    const WBEM_FLAG_RETURN_IMMEDIATELY: i32 = 0x10;
    const WBEM_FLAG_FORWARD_ONLY: i32 = 0x20;
    const WBEM_S_TIMEDOUT: Hresult = 0x0004_0004;

    // VARIANT type tags we read.
    const VT_I2: u16 = 2;
    const VT_I4: u16 = 3;
    const VT_BSTR: u16 = 8;
    const VT_UNKNOWN: u16 = 13;
    const VT_UI2: u16 = 18;
    const VT_UI4: u16 = 19;
    const VT_INT: u16 = 22;
    const VT_UINT: u16 = 23;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    // CLSID_WbemLocator {4590F811-1D3A-11D0-891F-00AA004B2E24}
    const CLSID_WBEM_LOCATOR: Guid = Guid {
        data1: 0x4590_F811,
        data2: 0x1D3A,
        data3: 0x11D0,
        data4: [0x89, 0x1F, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };
    // IID_IWbemLocator {DC12A687-737F-11CF-884D-00AA004B2E24}
    const IID_IWBEM_LOCATOR: Guid = Guid {
        data1: 0xDC12_A687,
        data2: 0x737F,
        data3: 0x11CF,
        data4: [0x88, 0x4D, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };

    /// VARIANT (x64: 24 bytes). We read only the tag and the first union word.
    #[repr(C)]
    struct Variant {
        vt: u16,
        _r1: u16,
        _r2: u16,
        _r3: u16,
        val0: u64,
        val1: u64,
    }

    // --- Interface + vtable layouts (only the slots we call are typed) ---

    #[repr(C)]
    struct IWbemLocator {
        vtbl: *const IWbemLocatorVtbl,
    }
    #[repr(C)]
    struct IWbemLocatorVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut IWbemLocator) -> u32,
        connect_server: unsafe extern "system" fn(
            *mut IWbemLocator,
            Bstr,
            Bstr,
            Bstr,
            Bstr,
            i32,
            Bstr,
            *mut c_void,
            *mut *mut IWbemServices,
        ) -> Hresult,
    }

    #[repr(C)]
    struct IWbemServices {
        vtbl: *const IWbemServicesVtbl,
    }
    #[repr(C)]
    struct IWbemServicesVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut IWbemServices) -> u32,
        open_namespace: usize,
        cancel_async_call: usize,
        query_object_sink: usize,
        get_object: usize,
        get_object_async: usize,
        put_class: usize,
        put_class_async: usize,
        delete_class: usize,
        delete_class_async: usize,
        create_class_enum: usize,
        create_class_enum_async: usize,
        put_instance: usize,
        put_instance_async: usize,
        delete_instance: usize,
        delete_instance_async: usize,
        create_instance_enum: usize,
        create_instance_enum_async: usize,
        exec_query: usize,
        exec_query_async: usize,
        exec_notification_query: unsafe extern "system" fn(
            *mut IWbemServices,
            Bstr,
            Bstr,
            i32,
            *mut c_void,
            *mut *mut IEnumWbemClassObject,
        ) -> Hresult,
        // exec_notification_query_async, exec_method, exec_method_async follow
        // but are never called, so they are omitted from the layout tail.
    }

    #[repr(C)]
    struct IEnumWbemClassObject {
        vtbl: *const IEnumWbemClassObjectVtbl,
    }
    #[repr(C)]
    struct IEnumWbemClassObjectVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut IEnumWbemClassObject) -> u32,
        reset: usize,
        next: unsafe extern "system" fn(
            *mut IEnumWbemClassObject,
            i32,
            u32,
            *mut *mut IWbemClassObject,
            *mut u32,
        ) -> Hresult,
        // next_async, clone, skip follow but are never called.
    }

    #[repr(C)]
    struct IWbemClassObject {
        vtbl: *const IWbemClassObjectVtbl,
    }
    #[repr(C)]
    struct IWbemClassObjectVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut IWbemClassObject) -> u32,
        get_qualifier_set: usize,
        get: unsafe extern "system" fn(
            *mut IWbemClassObject,
            *const u16,
            i32,
            *mut Variant,
            *mut i32,
            *mut i32,
        ) -> Hresult,
        // put, delete, ... follow but are never called.
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, co_init: u32) -> Hresult;
        fn CoUninitialize();
        fn CoInitializeSecurity(
            sec_desc: *mut c_void,
            auth_svc: i32,
            as_auth_svc: *mut c_void,
            reserved1: *mut c_void,
            authn_level: u32,
            imp_level: u32,
            auth_list: *mut c_void,
            capabilities: u32,
            reserved3: *mut c_void,
        ) -> Hresult;
        fn CoCreateInstance(
            rclsid: *const Guid,
            unk_outer: *mut c_void,
            cls_context: u32,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> Hresult;
        fn CoSetProxyBlanket(
            proxy: *mut c_void,
            authn_svc: u32,
            authz_svc: u32,
            server_princ_name: *mut u16,
            authn_level: u32,
            imp_level: u32,
            auth_info: *mut c_void,
            capabilities: u32,
        ) -> Hresult;
    }

    #[link(name = "oleaut32")]
    extern "system" {
        fn SysAllocString(psz: *const u16) -> Bstr;
        fn SysFreeString(bstr: Bstr);
        fn VariantClear(var: *mut Variant) -> Hresult;
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    unsafe fn bstr_to_string(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0usize;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    unsafe fn get_variant(obj: *mut IWbemClassObject, name: &str) -> Option<Variant> {
        let wname = wide(name);
        let mut var: Variant = std::mem::zeroed();
        let hr = ((*(*obj).vtbl).get)(
            obj,
            wname.as_ptr(),
            0,
            &mut var,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        if hr < 0 {
            None
        } else {
            Some(var)
        }
    }

    unsafe fn get_string(obj: *mut IWbemClassObject, name: &str) -> Option<String> {
        let mut var = get_variant(obj, name)?;
        let out = if var.vt == VT_BSTR {
            Some(bstr_to_string(var.val0 as *const u16))
        } else {
            None
        };
        VariantClear(&mut var);
        out
    }

    unsafe fn get_u32(obj: *mut IWbemClassObject, name: &str) -> Option<u32> {
        let mut var = get_variant(obj, name)?;
        let out = match var.vt {
            VT_I2 | VT_I4 | VT_UI2 | VT_UI4 | VT_INT | VT_UINT => Some(var.val0 as u32),
            VT_BSTR => bstr_to_string(var.val0 as *const u16)
                .trim()
                .parse::<u32>()
                .ok(),
            _ => None,
        };
        VariantClear(&mut var);
        out
    }

    /// The real COM pump. All pointers are created, used, and dropped on the
    /// watcher thread; this type is intentionally not `Send`.
    struct ComProcessTracePump {
        loc: *mut IWbemLocator,
        svc: *mut IWbemServices,
        start_en: *mut IEnumWbemClassObject,
        stop_en: *mut IEnumWbemClassObject,
        poll_start_next: bool,
    }

    impl ComProcessTracePump {
        fn empty() -> Self {
            Self {
                loc: std::ptr::null_mut(),
                svc: std::ptr::null_mut(),
                start_en: std::ptr::null_mut(),
                stop_en: std::ptr::null_mut(),
                poll_start_next: true,
            }
        }

        unsafe fn subscribe(
            svc: *mut IWbemServices,
            query_text: &str,
        ) -> *mut IEnumWbemClassObject {
            let lang = SysAllocString(wide("WQL").as_ptr());
            let query = SysAllocString(wide(query_text).as_ptr());
            let mut enumerator: *mut IEnumWbemClassObject = std::ptr::null_mut();
            let hr = ((*(*svc).vtbl).exec_notification_query)(
                svc,
                lang,
                query,
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                std::ptr::null_mut(),
                &mut enumerator,
            );
            SysFreeString(lang);
            SysFreeString(query);
            if hr < 0 {
                std::ptr::null_mut()
            } else {
                enumerator
            }
        }

        unsafe fn teardown(&mut self) {
            if !self.start_en.is_null() {
                ((*(*self.start_en).vtbl).release)(self.start_en);
                self.start_en = std::ptr::null_mut();
            }
            if !self.stop_en.is_null() {
                ((*(*self.stop_en).vtbl).release)(self.stop_en);
                self.stop_en = std::ptr::null_mut();
            }
            if !self.svc.is_null() {
                ((*(*self.svc).vtbl).release)(self.svc);
                self.svc = std::ptr::null_mut();
            }
            if !self.loc.is_null() {
                ((*(*self.loc).vtbl).release)(self.loc);
                self.loc = std::ptr::null_mut();
            }
            self.poll_start_next = true;
        }
    }

    impl Drop for ComProcessTracePump {
        fn drop(&mut self) {
            unsafe { self.teardown() };
        }
    }

    impl WmiEventPump for ComProcessTracePump {
        fn resubscribe(&mut self) -> bool {
            unsafe {
                self.teardown();

                let mut loc: *mut IWbemLocator = std::ptr::null_mut();
                let hr = CoCreateInstance(
                    &CLSID_WBEM_LOCATOR,
                    std::ptr::null_mut(),
                    CLSCTX_INPROC_SERVER,
                    &IID_IWBEM_LOCATOR,
                    &mut loc as *mut *mut IWbemLocator as *mut *mut c_void,
                );
                if hr < 0 || loc.is_null() {
                    return false;
                }
                self.loc = loc;

                let ns = SysAllocString(wide("ROOT\\CIMV2").as_ptr());
                let mut svc: *mut IWbemServices = std::ptr::null_mut();
                let hr = ((*(*loc).vtbl).connect_server)(
                    loc,
                    ns,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut svc,
                );
                SysFreeString(ns);
                if hr < 0 || svc.is_null() {
                    return false;
                }
                self.svc = svc;

                let hr = CoSetProxyBlanket(
                    svc as *mut c_void,
                    RPC_C_AUTHN_WINNT,
                    RPC_C_AUTHZ_NONE,
                    std::ptr::null_mut(),
                    RPC_C_AUTHN_LEVEL_CALL,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    std::ptr::null_mut(),
                    EOAC_NONE,
                );
                if hr < 0 {
                    return false;
                }

                self.start_en = Self::subscribe(
                    svc,
                    "SELECT * FROM __InstanceCreationEvent WITHIN 1 WHERE TargetInstance ISA 'Win32_Process'",
                );
                if self.start_en.is_null() {
                    return false;
                }
                self.stop_en = Self::subscribe(
                    svc,
                    "SELECT * FROM __InstanceDeletionEvent WITHIN 1 WHERE TargetInstance ISA 'Win32_Process'",
                );
                if self.stop_en.is_null() {
                    return false;
                }
                self.poll_start_next = true;
                true
            }
        }

        fn poll(&mut self, timeout_ms: u32) -> WmiPoll {
            if self.start_en.is_null() || self.stop_en.is_null() {
                return WmiPoll::Lost;
            }
            unsafe {
                let enumerator = if self.poll_start_next {
                    self.start_en
                } else {
                    self.stop_en
                };
                self.poll_start_next = !self.poll_start_next;
                let mut obj: *mut IWbemClassObject = std::ptr::null_mut();
                let mut returned: u32 = 0;
                let hr = ((*(*enumerator).vtbl).next)(
                    enumerator,
                    timeout_ms as i32,
                    1,
                    &mut obj,
                    &mut returned,
                );
                if hr == WBEM_S_TIMEDOUT {
                    return WmiPoll::Timeout;
                }
                if hr < 0 {
                    return WmiPoll::Lost;
                }
                if returned == 0 || obj.is_null() {
                    return WmiPoll::Timeout;
                }
                let class = get_string(obj, "__CLASS").unwrap_or_default();
                let mut target = match get_variant(obj, "TargetInstance") {
                    Some(value) => value,
                    None => {
                        ((*(*obj).vtbl).release)(obj);
                        return WmiPoll::Lost;
                    }
                };
                if target.vt != VT_UNKNOWN || target.val0 == 0 {
                    VariantClear(&mut target);
                    ((*(*obj).vtbl).release)(obj);
                    return WmiPoll::Lost;
                }
                let target_obj = target.val0 as *mut IWbemClassObject;
                let pid = get_u32(target_obj, "ProcessId").unwrap_or(0);
                let name = get_string(target_obj, "Name").unwrap_or_default();
                VariantClear(&mut target);
                ((*(*obj).vtbl).release)(obj);
                WmiPoll::Event { class, pid, name }
            }
        }
    }

    /// The COM watcher thread body: initialize COM (MTA) on this thread, run the
    /// generic pump loop over a real COM pump, then uninitialize COM AFTER the
    /// pump has dropped its interface pointers.
    pub fn run_com_watcher(
        shared: Arc<SharedWmiState>,
        cancel: Arc<AtomicBool>,
        ready: SyncSender<()>,
    ) {
        unsafe {
            let hr = CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);
            // S_OK(0)/S_FALSE(1) are success; a fresh thread will not hit
            // RPC_E_CHANGED_MODE, but tolerate it (COM usable, don't uninit).
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                shared.set_health(super::WatcherHealth::Degraded);
                let _ = ready.try_send(());
                return;
            }
            let uninit = hr != RPC_E_CHANGED_MODE;
            // Best-effort process-wide security. If already set (RPC_E_TOO_LATE)
            // or it fails, defaults/prior settings apply; the real gate is
            // whether the query + Next below succeed (else we degrade honestly).
            let _ = CoInitializeSecurity(
                std::ptr::null_mut(),
                -1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                RPC_C_AUTHN_LEVEL_DEFAULT,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                std::ptr::null_mut(),
                EOAC_NONE,
                std::ptr::null_mut(),
            );

            run_pump_loop(|| Some(ComProcessTracePump::empty()), shared, cancel, ready);

            if uninit {
                CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classify (pure) ---------------------------------------------

    #[test]
    fn classify_maps_start_and_stop() {
        assert_eq!(
            classify_process_trace("Win32_ProcessStartTrace", 42, "hoi4.exe"),
            Some(RawProcessEvent::Start {
                pid: 42,
                name: "hoi4.exe".into()
            })
        );
        assert_eq!(
            classify_process_trace("Win32_ProcessStopTrace", 42, "hoi4.exe"),
            Some(RawProcessEvent::Stop {
                pid: 42,
                name: "hoi4.exe".into()
            })
        );
        assert_eq!(
            classify_process_trace("__InstanceCreationEvent", 43, "hoi4.exe"),
            Some(RawProcessEvent::Start {
                pid: 43,
                name: "hoi4.exe".into()
            })
        );
        assert_eq!(
            classify_process_trace("__InstanceDeletionEvent", 43, "hoi4.exe"),
            Some(RawProcessEvent::Stop {
                pid: 43,
                name: "hoi4.exe".into()
            })
        );
    }

    #[test]
    fn classify_ignores_unknown_class() {
        assert_eq!(
            classify_process_trace("Win32_ProcessTrace", 42, "hoi4.exe"),
            None
        );
        assert_eq!(classify_process_trace("", 1, "x").as_ref(), None);
    }

    // ---- fake WMI layer ----------------------------------------------

    struct FakePump {
        resubscribe_results: VecDeque<bool>,
        poll_results: VecDeque<WmiPoll>,
        resubscribe_calls: usize,
        poll_calls: usize,
    }

    impl FakePump {
        fn new(resub: Vec<bool>, polls: Vec<WmiPoll>) -> Self {
            Self {
                resubscribe_results: resub.into(),
                poll_results: polls.into(),
                resubscribe_calls: 0,
                poll_calls: 0,
            }
        }
    }

    impl WmiEventPump for FakePump {
        fn resubscribe(&mut self) -> bool {
            self.resubscribe_calls += 1;
            self.resubscribe_results.pop_front().unwrap_or(false)
        }
        fn poll(&mut self, _timeout_ms: u32) -> WmiPoll {
            self.poll_calls += 1;
            self.poll_results.pop_front().unwrap_or(WmiPoll::Timeout)
        }
    }

    fn start(pid: u32) -> WmiPoll {
        WmiPoll::Event {
            class: "Win32_ProcessStartTrace".into(),
            pid,
            name: "hoi4.exe".into(),
        }
    }

    #[test]
    fn iteration_subscribes_then_marks_healthy() {
        let shared = SharedWmiState::new();
        let mut pump = FakePump::new(vec![true], vec![]);
        let mut connected = false;
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Progressed);
        assert!(connected);
        assert_eq!(shared.health(), WatcherHealth::Healthy);
    }

    #[test]
    fn iteration_failed_subscribe_stays_degraded_and_backs_off() {
        let shared = SharedWmiState::new();
        let mut pump = FakePump::new(vec![false], vec![]);
        let mut connected = false;
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Backoff);
        assert!(!connected);
        assert_eq!(shared.health(), WatcherHealth::Degraded);
    }

    #[test]
    fn iteration_queues_classified_event() {
        let shared = SharedWmiState::new();
        let mut pump = FakePump::new(vec![], vec![start(1234)]);
        let mut connected = true;
        shared.set_health(WatcherHealth::Healthy);
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Progressed);
        let drained = shared.drain();
        assert_eq!(
            drained,
            vec![RawProcessEvent::Start {
                pid: 1234,
                name: "hoi4.exe".into()
            }]
        );
        // Health untouched by a plain event.
        assert_eq!(shared.health(), WatcherHealth::Healthy);
    }

    #[test]
    fn iteration_drops_unknown_class_event() {
        let shared = SharedWmiState::new();
        let ev = WmiPoll::Event {
            class: "Win32_ProcessTrace".into(),
            pid: 9,
            name: "hoi4.exe".into(),
        };
        let mut pump = FakePump::new(vec![], vec![ev]);
        let mut connected = true;
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Progressed);
        assert!(shared.drain().is_empty());
    }

    #[test]
    fn iteration_lost_degrades_and_disconnects() {
        // Models WMI provider loss (design §2): the source degrades so the
        // coordinator falls back to the bounded 2 s snapshot reconcile.
        let shared = SharedWmiState::new();
        shared.set_health(WatcherHealth::Healthy);
        let mut pump = FakePump::new(vec![], vec![WmiPoll::Lost]);
        let mut connected = true;
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Backoff);
        assert!(!connected);
        assert_eq!(shared.health(), WatcherHealth::Degraded);
    }

    #[test]
    fn iteration_recovers_after_loss() {
        // Lost -> Degraded, then a successful resubscribe -> Healthy (design §2
        // "WMI loss/recovery"). No duplicate/fabricated events across the gap.
        let shared = SharedWmiState::new();
        let mut pump = FakePump::new(vec![true, true], vec![WmiPoll::Lost, start(7)]);
        let mut connected = false;
        // 1: subscribe -> Healthy
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Progressed
        );
        assert_eq!(shared.health(), WatcherHealth::Healthy);
        // 2: poll -> Lost -> Degraded, disconnected
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Backoff
        );
        assert_eq!(shared.health(), WatcherHealth::Degraded);
        assert!(!connected);
        // 3: re-subscribe -> Healthy again
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Progressed
        );
        assert_eq!(shared.health(), WatcherHealth::Healthy);
        assert!(connected);
        // 4: poll -> the recovered event
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Progressed
        );
        assert_eq!(
            shared.drain(),
            vec![RawProcessEvent::Start {
                pid: 7,
                name: "hoi4.exe".into()
            }]
        );
    }

    // ---- threaded source over the fake pump --------------------------

    #[test]
    fn threaded_source_drains_event_and_reports_healthy() {
        // A fake pump that subscribes, emits one start, then idles forever.
        let mut polls = vec![start(555)];
        for _ in 0..64 {
            polls.push(WmiPoll::Timeout);
        }
        let factory = move || Some(FakePump::new(vec![true], polls));
        let mut source = WmiProcessEventSource::with_pump_factory(factory);

        // Drain with a bounded deadline (no fixed sleep assumption).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while std::time::Instant::now() < deadline {
            got.extend(source.drain_events());
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            got,
            vec![RawProcessEvent::Start {
                pid: 555,
                name: "hoi4.exe".into()
            }]
        );
        assert_eq!(source.health(), WatcherHealth::Healthy);
        // Drop joins the watcher thread (cancellation path).
    }

    #[test]
    fn threaded_source_none_factory_is_degraded() {
        let factory = || None::<FakePump>;
        let mut source = WmiProcessEventSource::with_pump_factory(factory);
        assert_eq!(source.health(), WatcherHealth::Degraded);
        assert!(source.drain_events().is_empty());
    }

    // ---- live smoke (manual only) ------------------------------------

    /// Live COM gate: the standard-user subscriptions must become healthy and
    /// deliver both sides of a real child-process lifecycle. Run manually with
    /// `cargo test -p recorder_host wmi_live_smoke -- --ignored --nocapture`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn wmi_live_smoke() {
        let mut source = WmiProcessEventSource::new();
        assert_eq!(source.health(), WatcherHealth::Healthy);

        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 10",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("launch WMI lifecycle fixture");
        let child_pid = child.id();

        let start_deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut saw_start = false;
        while std::time::Instant::now() < start_deadline && !saw_start {
            for event in source.drain_events() {
                eprintln!("wmi event: {event:?}");
                if matches!(event, RawProcessEvent::Start { pid, ref name } if pid == child_pid && name.eq_ignore_ascii_case("powershell.exe"))
                {
                    saw_start = true;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        child.kill().expect("terminate WMI lifecycle fixture");
        child.wait().expect("reap WMI lifecycle fixture");

        let stop_deadline = std::time::Instant::now() + Duration::from_secs(8);
        let mut saw_stop = false;
        while std::time::Instant::now() < stop_deadline && !saw_stop {
            for event in source.drain_events() {
                eprintln!("wmi event: {event:?}");
                if matches!(event, RawProcessEvent::Stop { pid, ref name } if pid == child_pid && name.eq_ignore_ascii_case("powershell.exe"))
                {
                    saw_stop = true;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert_eq!(source.health(), WatcherHealth::Healthy);
        assert!(
            saw_start,
            "WMI did not deliver fixture start for PID {child_pid}"
        );
        assert!(
            saw_stop,
            "WMI did not deliver fixture stop for PID {child_pid}"
        );
    }
}
