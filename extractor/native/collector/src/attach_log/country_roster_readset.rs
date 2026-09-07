use compact_contract::country_roster::Cause;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) const VERIFICATION_LIMIT: usize = 10 * 1024 * 1024;
pub(super) const RETAINED_LIMIT: usize = 16 * 1024 * 1024;
pub(super) const FLEET_SLOT_LIMIT: usize = 250_000;
const SCRATCH: usize = 8192;
const MAX_GROUPED_VERIFICATION: usize = 16 * 65_536 + 28 * 65_536 + 24 * (20_000 + FLEET_SLOT_LIMIT) + 24;
const _: () = assert!(MAX_GROUPED_VERIFICATION == 9_363_608);
const _: () = assert!(MAX_GROUPED_VERIFICATION <= VERIFICATION_LIMIT);
const _: () = assert!(SCRATCH <= 192 * 1024);

pub(super) struct Budget {
    verification: AtomicUsize,
    retained: AtomicUsize,
    fleet_slots: AtomicUsize,
    verification_limit: usize,
    retained_limit: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new(VERIFICATION_LIMIT, RETAINED_LIMIT)
    }
}
impl Budget {
    fn new(verification_limit: usize, retained_limit: usize) -> Self {
        Self { verification: AtomicUsize::new(0), retained: AtomicUsize::new(0),
            fleet_slots: AtomicUsize::new(0), verification_limit, retained_limit }
    }
    fn charge(counter: &AtomicUsize, limit: usize, bytes: usize) -> Result<(), Cause> {
        let mut previous = counter.load(Ordering::Relaxed);
        loop {
            let next = previous.checked_add(bytes).filter(|next| *next <= limit);
            let charged = next.unwrap_or(limit + 1);
            match counter.compare_exchange_weak(previous, charged, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return next.map(|_| ()).ok_or(Cause::BoundExceeded),
                Err(actual) => previous = actual,
            }
        }
    }
    #[cfg(test)]
    pub(super) fn test_limits(verification_limit: usize, retained_limit: usize) -> Self {
        Self::new(verification_limit, retained_limit)
    }
    #[cfg(test)]
    pub(super) fn verification_bytes(&self) -> usize { self.verification.load(Ordering::Relaxed) }

    pub(super) fn reserve_read(&self, address: usize, width: usize) -> Result<ReservedRead, Cause> {
        if !(1..=8).contains(&width) { return Err(Cause::BoundExceeded); }
        address.checked_add(width).ok_or(Cause::BoundExceeded)?;
        Self::charge(&self.verification, self.verification_limit, width * 2)?;
        Ok(ReservedRead { address, width })
    }

    pub(super) fn retain(&self, bytes: usize) -> Result<(), Cause> {
        Self::charge(&self.retained, self.retained_limit, bytes)
    }
    pub(super) fn fleet_slots(&self, count: usize) -> Result<(), Cause> {
        Self::charge(&self.fleet_slots, FLEET_SLOT_LIMIT, count)
    }
    pub(super) fn read<const N: usize>(
        &self, address: usize, read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; N], Cause> {
        address.checked_add(N).ok_or(Cause::BoundExceeded)?;
        Self::charge(&self.verification, self.verification_limit, N)?;
        let mut bytes = [0; N];
        if read(address, &mut bytes) { Ok(bytes) } else { Err(Cause::VectorUnreadable) }
    }
    pub(super) fn compare(
        &self, address: usize, expected: &[u8],
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<(), Cause> {
        address.checked_add(expected.len()).ok_or(Cause::BoundExceeded)?;
        Self::charge(&self.verification, self.verification_limit, expected.len())?;
        let mut scratch = [0; SCRATCH];
        for (index, chunk) in expected.chunks(SCRATCH).enumerate() {
            let destination = &mut scratch[..chunk.len()];
            if !read(address + index * SCRATCH, destination) { return Err(Cause::VectorUnreadable); }
            if destination != chunk { return Err(Cause::VectorChanged); }
        }
        Ok(())
    }
}

// Neither capability is Clone: opening and closing each consume their reservation.
pub(super) struct ReservedRead { address: usize, width: usize }
pub(super) struct OpenedRead { address: usize, width: usize, bytes: [u8; 8] }
impl ReservedRead {
    pub(super) fn open(self, read: &mut impl FnMut(usize, &mut [u8]) -> bool) -> Result<OpenedRead, Cause> {
        let mut bytes = [0; 8];
        if !read(self.address, &mut bytes[..self.width]) { return Err(Cause::VectorUnreadable); }
        Ok(OpenedRead { address: self.address, width: self.width, bytes })
    }
}
impl OpenedRead {
    pub(super) fn bytes(&self) -> &[u8] { &self.bytes[..self.width] }
    pub(super) fn width(&self) -> usize { self.width }
    pub(super) fn close(self, read: &mut impl FnMut(usize, &mut [u8]) -> bool) -> Result<(), Cause> {
        let mut current = [0; 8];
        if !read(self.address, &mut current[..self.width]) { return Err(Cause::VectorUnreadable); }
        if current[..self.width] != self.bytes[..self.width] { return Err(Cause::VectorChanged); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_capability_remains_funded_after_shared_ceiling_is_exhausted() {
        let budget = Budget::new(16, 1);
        let mut calls = Vec::new();
        let opening = budget.reserve_read(0x1000, 8).unwrap().open(&mut |address, out| {
            calls.push((address, out.len())); out.copy_from_slice(&7u64.to_le_bytes()); true
        }).unwrap();
        assert!(matches!(budget.reserve_read(0x2000, 1), Err(Cause::BoundExceeded)));
        assert_eq!(opening.close(&mut |address, out| {
            calls.push((address, out.len())); out.copy_from_slice(&7u64.to_le_bytes()); true
        }), Ok(()));
        assert_eq!(calls, [(0x1000, 8), (0x1000, 8)]);
        assert_eq!(budget.verification_bytes(), 17);
    }
    #[test]
    fn unsuccessful_opening_keeps_its_closing_bytes_reserved_and_invalid_widths_do_not_read() {
        let budget = Budget::new(16, 1);
        for width in [0, 9, usize::MAX] {
            assert!(matches!(budget.reserve_read(0x1000, width), Err(Cause::BoundExceeded)));
        }
        assert!(matches!(budget.reserve_read(usize::MAX, 1), Err(Cause::BoundExceeded)));
        assert_eq!(budget.verification_bytes(), 0);
        let mut calls = 0;
        assert!(matches!(budget.reserve_read(0x1000, 8).unwrap().open(&mut |_, out| {
            calls += 1; assert_eq!(out.len(), 8); false
        }), Err(Cause::VectorUnreadable)));
        assert_eq!(budget.verification_bytes(), 16);
        assert!(matches!(budget.reserve_read(0x2000, 1), Err(Cause::BoundExceeded)));
        assert_eq!(calls, 1);
    }

    #[test]
    fn fresh_read_trace_detects_mutation_without_following_new_pointer() {
        let budget = Budget::new(32, 32);
        let opening = 0x2000u64.to_le_bytes();
        let mut calls = Vec::new();
        let mut reader = |address, out: &mut [u8]| {
            calls.push((address, out.len()));
            out.copy_from_slice(&0x3000u64.to_le_bytes()); true
        };
        assert_eq!(budget.compare(0x1000, &opening, &mut reader), Err(Cause::VectorChanged));
        assert_eq!(calls, [(0x1000, 8)]);
        assert_eq!(budget.verification.load(Ordering::Relaxed), 8);
    }
    #[test]
    fn empty_is_measured_without_a_pointer_read_and_failure_is_not_refunded() {
        let budget = Budget::new(8, 1);
        let mut count = 0;
        let mut reader = |_, _: &mut [u8]| { count += 1; false };
        assert_eq!(budget.compare(0, &[], &mut reader), Ok(()));
        assert_eq!(budget.read::<8>(8, &mut reader), Err(Cause::VectorUnreadable));
        assert_eq!(budget.read::<1>(8, &mut reader), Err(Cause::BoundExceeded));
        assert_eq!(budget.compare(0, &[], &mut reader), Err(Cause::BoundExceeded));
        assert_eq!(count, 1);
        assert_eq!(budget.retain(1), Ok(()));
        assert_eq!(budget.retain(1), Err(Cause::BoundExceeded));
        assert_eq!(budget.retain(0), Err(Cause::BoundExceeded));
    }
    #[test]
    fn simultaneous_readers_charge_before_read_and_share_one_ceiling() {
        let budget = Budget::new(128, 1);
        let calls = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let budget = &budget; let calls = &calls;
                scope.spawn(move || {
                    for _ in 0..32 {
                        let _ = budget.read::<8>(8, &mut |_, out| {
                            calls.fetch_add(1, Ordering::Relaxed); out.fill(0); true
                        });
                    }
                });
            }
        });
        assert_eq!(calls.load(Ordering::Relaxed), 16);
        assert_eq!(budget.verification.load(Ordering::Relaxed), 129);
    }
    #[test]
    fn full_vector_streams_with_bounded_scratch_and_rejects_last_byte_change() {
        let bytes = vec![19; 2_000_000];
        let budget = Budget::default();
        let mut largest = 0;
        assert_eq!(budget.compare(16, &bytes, &mut |address, out| {
            largest = largest.max(out.len()); out.fill(19);
            if address + out.len() == 16 + bytes.len() { *out.last_mut().unwrap() = 20; }
            true
        }), Err(Cause::VectorChanged));
        assert!(largest <= SCRATCH);
        assert_eq!(budget.verification.load(Ordering::Relaxed), 2_000_000);
        assert_eq!(budget.fleet_slots(250_000), Ok(()));
        assert_eq!(budget.fleet_slots(1), Err(Cause::BoundExceeded));
    }
}
