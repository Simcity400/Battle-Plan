use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{ProcessEventSource, RawProcessEvent};
use crate::auto_attach::policy::WatcherHealth;

const POLL_TIMEOUT_MS: u32 = 500;
const RESUBSCRIBE_BACKOFF_MS: u64 = 2_000;
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

pub trait WmiEventPump {
    fn resubscribe(&mut self) -> bool;
    fn poll(&mut self, timeout_ms: u32) -> WmiPoll;
}

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

pub(crate) fn event_or_lost(class: String, pid: Option<u32>, name: String) -> WmiPoll {
    match pid {
        Some(pid) => WmiPoll::Event { class, pid, name },
        None => WmiPoll::Lost,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IterOutcome {
    Progressed,
    Backoff,
}

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

struct SharedWmiState {
    queue: Mutex<VecDeque<RawProcessEvent>>,
    health: AtomicU8,
}

impl SharedWmiState {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
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
}

pub struct WmiProcessEventSource {
    shared: Arc<SharedWmiState>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl WmiProcessEventSource {
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
        let _ = rx.recv_timeout(Duration::from_millis(READY_TIMEOUT_MS));
        Self {
            shared,
            cancel,
            handle,
        }
    }

    pub fn with_pump_factory<F, P>(factory: F) -> Self
    where
        F: FnOnce() -> Option<P> + Send + 'static,
        P: WmiEventPump + 'static,
    {
        Self::spawn_thread(move |shared, cancel, ready| {
            run_pump_loop(factory, shared, cancel, ready)
        })
    }

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

#[cfg(windows)]
#[allow(dead_code)]
mod com {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::SyncSender;
    use std::sync::Arc;

    use super::{run_pump_loop, SharedWmiState, WmiEventPump, WmiPoll};

    type Hresult = i32;
    type Bstr = *mut u16;

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

    const CLSID_WBEM_LOCATOR: Guid = Guid {
        data1: 0x4590_F811,
        data2: 0x1D3A,
        data3: 0x11D0,
        data4: [0x89, 0x1F, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };
    const IID_IWBEM_LOCATOR: Guid = Guid {
        data1: 0xDC12_A687,
        data2: 0x737F,
        data3: 0x11CF,
        data4: [0x88, 0x4D, 0x00, 0xAA, 0x00, 0x4B, 0x2E, 0x24],
    };

    #[repr(C)]
    struct Variant {
        vt: u16,
        _r1: u16,
        _r2: u16,
        _r3: u16,
        val0: u64,
        val1: u64,
    }

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
                let pid = get_u32(target_obj, "ProcessId");
                let name = get_string(target_obj, "Name").unwrap_or_default();
                VariantClear(&mut target);
                ((*(*obj).vtbl).release)(obj);
                super::event_or_lost(class, pid, name)
            }
        }
    }

    pub fn run_com_watcher(
        shared: Arc<SharedWmiState>,
        cancel: Arc<AtomicBool>,
        ready: SyncSender<()>,
    ) {
        unsafe {
            let hr = CoInitializeEx(std::ptr::null_mut(), COINIT_MULTITHREADED);
            if hr < 0 && hr != RPC_E_CHANGED_MODE {
                shared.set_health(super::WatcherHealth::Degraded);
                let _ = ready.try_send(());
                return;
            }
            let uninit = hr != RPC_E_CHANGED_MODE;
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

    #[test]
    fn event_or_lost_builds_event_for_readable_pid() {
        assert_eq!(
            event_or_lost(
                "Win32_ProcessStartTrace".into(),
                Some(42),
                "hoi4.exe".into()
            ),
            WmiPoll::Event {
                class: "Win32_ProcessStartTrace".into(),
                pid: 42,
                name: "hoi4.exe".into(),
            }
        );
    }

    #[test]
    fn unreadable_pid_degrades_to_lost_never_fabricates_pid() {
        assert_eq!(
            event_or_lost("Win32_ProcessStopTrace".into(), None, "hoi4.exe".into()),
            WmiPoll::Lost
        );
    }

    #[test]
    fn unreadable_pid_emits_no_event_and_degrades_health() {
        let shared = SharedWmiState::new();
        shared.set_health(WatcherHealth::Healthy);
        let poll = event_or_lost("Win32_ProcessStartTrace".into(), None, "hoi4.exe".into());
        let mut pump = FakePump::new(vec![], vec![poll]);
        let mut connected = true;
        let out = pump_iteration(&mut pump, &mut connected, &shared);
        assert_eq!(out, IterOutcome::Backoff);
        assert!(shared.drain().is_empty());
        assert_eq!(shared.health(), WatcherHealth::Degraded);
        assert!(!connected);
    }

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
        let shared = SharedWmiState::new();
        let mut pump = FakePump::new(vec![true, true], vec![WmiPoll::Lost, start(7)]);
        let mut connected = false;
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Progressed
        );
        assert_eq!(shared.health(), WatcherHealth::Healthy);
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Backoff
        );
        assert_eq!(shared.health(), WatcherHealth::Degraded);
        assert!(!connected);
        assert_eq!(
            pump_iteration(&mut pump, &mut connected, &shared),
            IterOutcome::Progressed
        );
        assert_eq!(shared.health(), WatcherHealth::Healthy);
        assert!(connected);
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

    #[test]
    fn threaded_source_drains_event_and_reports_healthy() {
        let mut polls = vec![start(555)];
        for _ in 0..64 {
            polls.push(WmiPoll::Timeout);
        }
        let factory = move || Some(FakePump::new(vec![true], polls));
        let mut source = WmiProcessEventSource::with_pump_factory(factory);

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
    }

    #[test]
    fn threaded_source_none_factory_is_degraded() {
        let factory = || None::<FakePump>;
        let mut source = WmiProcessEventSource::with_pump_factory(factory);
        assert_eq!(source.health(), WatcherHealth::Degraded);
        assert!(source.drain_events().is_empty());
    }

    #[cfg(windows)]
    #[test]
    #[ignore]
    fn wmi_live_smoke() {
        let mut source = WmiProcessEventSource::new();
        assert_eq!(source.health(), WatcherHealth::Healthy);

        // The fixture must remain a real second process -- the test watches
        // its actual WMI start/stop lifecycle; CREATE_NO_WINDOW keeps the
        // console-subsystem child from allocating a visible terminal window
        // on console-less harnesses (lane QI defect class).
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
            .creation_flags(CREATE_NO_WINDOW)
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

        child.wait().expect("await WMI lifecycle fixture");

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
