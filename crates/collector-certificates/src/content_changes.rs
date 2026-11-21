use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    Equipment,
    Technology,
}
pub type Original = unsafe extern "system" fn(usize, usize, usize, usize) -> usize;
pub struct Forwarder {
    pub call: unsafe fn(Writer, [usize; 4], Original) -> usize,
    pub enabled: fn(bool),
}
static RECEIVER: AtomicPtr<Forwarder> = AtomicPtr::new(std::ptr::null_mut());
static ENABLED: [AtomicBool; 2] = [const { AtomicBool::new(false) }; 2];
pub fn forwarder(receiver: Option<&'static Forwarder>) {
    RECEIVER.store(
        receiver.map_or(std::ptr::null_mut(), |receiver| {
            std::ptr::from_ref(receiver).cast_mut()
        }),
        Ordering::Release,
    );
    if let Some(receiver) = receiver {
        (receiver.enabled)(ENABLED.iter().all(|flag| flag.load(Ordering::Acquire)));
    }
}
pub fn enabled(writer: Writer, enabled: bool) {
    ENABLED[writer as usize].store(enabled, Ordering::Release);
    if let Some(receiver) = unsafe { RECEIVER.load(Ordering::Acquire).as_ref() } {
        (receiver.enabled)(ENABLED.iter().all(|flag| flag.load(Ordering::Acquire)));
    }
}
/// Preserve every argument and the original result. An absent receiver costs
/// only its null atomic check; no native reads or allocation occur here.
///
/// # Safety
/// `original` must be the native reload function for `writer`, and `args` must
/// satisfy its call contract and that of the registered forwarder.
pub unsafe fn invoke(writer: Writer, args: [usize; 4], original: Original) -> usize {
    if let Some(receiver) = unsafe { RECEIVER.load(Ordering::Acquire).as_ref() } {
        unsafe { (receiver.call)(writer, args, original) }
    } else {
        unsafe { original(args[0], args[1], args[2], args[3]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static STATE: AtomicUsize = AtomicUsize::new(0);
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    fn changed(enabled: bool) {
        STATE.store(usize::from(enabled), Ordering::SeqCst);
    }
    unsafe fn receive(writer: Writer, args: [usize; 4], original: Original) -> usize {
        assert_eq!(writer, Writer::Technology);
        CALLS.fetch_add(1, Ordering::SeqCst);
        unsafe { original(args[0], args[1], args[2], args[3]) }
    }
    static FORWARD: Forwarder = Forwarder {
        call: receive,
        enabled: changed,
    };
    unsafe extern "system" fn original(a: usize, b: usize, c: usize, d: usize) -> usize {
        a ^ b ^ c ^ d
    }
    #[test]
    fn both_patch_owners_are_required_and_disabled_forwarding_preserves_the_original() {
        let _serial = LOCK.lock().unwrap();
        forwarder(None);
        enabled(Writer::Equipment, false);
        enabled(Writer::Technology, false);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(
                unsafe { invoke(Writer::Technology, [1, 2, 4, 8], original) },
                15
            )
        });
        forwarder(Some(&FORWARD));
        enabled(Writer::Equipment, true);
        assert_eq!(STATE.load(Ordering::SeqCst), 0);
        enabled(Writer::Technology, true);
        assert_eq!(STATE.load(Ordering::SeqCst), 1);
        collector_test_support::with_allocations_denied(|| {
            assert_eq!(
                unsafe { invoke(Writer::Technology, [1, 2, 4, 8], original) },
                15
            )
        });
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
        enabled(Writer::Equipment, false);
        assert_eq!(STATE.load(Ordering::SeqCst), 0);
        forwarder(None);
    }
}
