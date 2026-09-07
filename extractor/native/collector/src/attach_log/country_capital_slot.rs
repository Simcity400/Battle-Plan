use compact_contract::country_capital_slot::{CountryGroup, Failure, Operands, Outcome, TemplateFailure, MAX_GROUPS};

pub(super) const GROUP_BYTES: usize = 360;
pub(super) const BATCH_BYTES: usize = GROUP_BYTES * MAX_GROUPS;
const _: () = assert!(BATCH_BYTES == 23_592_960);

#[derive(Default)]
pub(super) struct Budget { bytes: usize, groups: usize }

struct GroupRead<'a, F> {
    budget: &'a mut Budget,
    read: &'a mut F,
    bytes: usize,
    seals: Vec<(usize, Vec<u8>)>,
}
impl<F: FnMut(usize, &mut [u8]) -> bool> GroupRead<'_, F> {
    fn bytes<const N: usize>(&mut self, address: usize, failure: Failure) -> Result<[u8; N], Failure> {
        address.checked_add(N).ok_or(Failure::Bounds)?;
        let group = self.bytes.checked_add(N).ok_or(Failure::Bounds)?;
        let batch = self.budget.bytes.checked_add(N).ok_or(Failure::Bounds)?;
        if group > GROUP_BYTES || batch > BATCH_BYTES { return Err(Failure::Bounds); }
        self.bytes = group;
        self.budget.bytes = batch;
        let mut out = [0; N];
        if !(self.read)(address, &mut out) { return Err(failure); }
        Ok(out)
    }
    fn observe<const N: usize>(&mut self, address: usize, failure: Failure) -> Result<[u8; N], Failure> {
        let out = self.bytes::<N>(address, failure)?;
        self.seals.push((address, out.to_vec()));
        Ok(out)
    }
    fn close(&mut self) -> Result<(), Failure> {
        for (address, expected) in std::mem::take(&mut self.seals).into_iter().rev() {
            // Every retained span is at most one header; the 0x5c state view is split locally.
            if expected.len() > 16 { return Err(Failure::Bounds); }
            let group = self.bytes.checked_add(expected.len()).ok_or(Failure::Bounds)?;
            let batch = self.budget.bytes.checked_add(expected.len()).ok_or(Failure::Bounds)?;
            if group > GROUP_BYTES || batch > BATCH_BYTES { return Err(Failure::Bounds); }
            self.bytes = group; self.budget.bytes = batch;
            let mut out = [0; 16];
            if !(self.read)(address, &mut out[..expected.len()]) { return Err(Failure::Incoherent); }
            if out[..expected.len()] != expected { return Err(Failure::Incoherent); }
        }
        Ok(())
    }
}
fn add(base: usize, offset: usize) -> Result<usize, Failure> { base.checked_add(offset).ok_or(Failure::Bounds) }
fn pointer(bytes: &[u8]) -> Result<usize, Failure> {
    usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap())).map_err(|_| Failure::Bounds)
}
fn signed(bytes: &[u8]) -> i32 { i32::from_le_bytes(bytes.try_into().unwrap()) }

pub(super) struct Input {
    pub country: usize,
    pub tag_db_id: u32,
    pub country_vtable: usize,
    pub state_vtable: usize,
    pub gs_cell: usize,
    pub expected_gs: usize,
    pub template_db_cell: usize,
    pub operands: Operands,
}

pub(super) fn capture(
    input: Input, budget: &mut Budget, mut read: impl FnMut(usize, &mut [u8]) -> bool,
) -> CountryGroup {
    let mut group = CountryGroup { tag_db_id: input.tag_db_id, operands: input.operands,
        outcome: Outcome::Failed(Failure::Bounds) };
    if budget.groups >= MAX_GROUPS { return group; }
    budget.groups += 1;
    let mut r = GroupRead { budget, read: &mut read, bytes: 0, seals: Vec::with_capacity(16) };
    let observed = (|| {
        if input.tag_db_id == 0 || input.tag_db_id > i32::MAX as u32 { return Err(Failure::CountryIdentity); }
        let class = r.observe::<8>(input.country, Failure::CountryIdentity)?;
        let id = r.observe::<4>(add(input.country, 8)?, Failure::CountryIdentity)?;
        if pointer(&class)? != input.country_vtable || u32::from_le_bytes(id) != input.tag_db_id {
            return Err(Failure::CountryIdentity);
        }
        // Compare against the exact operands already used by the politics observation.
        for (offset, value) in [(0x1018, input.operands.base_raw), (0x101c, input.operands.override_raw)] {
            if let Some(value) = value {
                let bytes = r.observe::<4>(add(input.country, offset)?, Failure::OperandUnobtained)?;
                if signed(&bytes) != value { return Err(Failure::Incoherent); }
            }
        }
        let selected = input.operands.effective().ok_or(Failure::OperandUnobtained)?;
        if selected <= 0 { return Ok(Outcome::NonPositive); }
        let gs = pointer(&r.observe::<8>(input.gs_cell, Failure::GameStateUnreadable)?)?;
        if gs == 0 { return Err(Failure::GameStateUnreadable); }
        if gs != input.expected_gs { return Err(Failure::Incoherent); }
        let header = r.observe::<16>(add(gs, 0x2c8)?, Failure::SlotHeaderUnreadable)?;
        let slots = pointer(&header[..8])?;
        let count = signed(&header[12..16]);
        if count < 0 { return Err(Failure::SlotHeaderUnreadable); }
        if selected >= count { return Ok(Outcome::OutOfRange { slot_count: count }); }
        if slots == 0 { return Err(Failure::SlotUnreadable); }
        let cell = add(slots, (selected as usize).checked_mul(8).ok_or(Failure::Bounds)?)?;
        let state = pointer(&r.observe::<8>(cell, Failure::SlotUnreadable)?)?;
        if state == 0 { return Ok(Outcome::NullSlot { slot_count: count }); }
        let view = r.bytes::<0x5c>(state, Failure::StateUnreadable)?;
        if pointer(&view[..8])? != input.state_vtable { return Err(Failure::StateUnreadable); }
        for (offset, length) in [(0, 8), (0x30, 8), (0x58, 4)] {
            r.seals.push((add(state, offset)?, view[offset..offset + length].to_vec()));
        }
        let state_id = signed(&view[0x58..0x5c]);
        let template = pointer(&view[0x30..0x38])?;
        let unavailable = |reason| Outcome::TemplateUnavailable { slot_count: count, state_id, reason };
        if template == 0 { return Ok(unavailable(TemplateFailure::NullPointer)); }
        let template_word = match r.observe::<4>(add(template, 0xa0)?, Failure::StateUnreadable) {
            Ok(value) => value, Err(Failure::Bounds) => return Err(Failure::Bounds),
            Err(_) => return Ok(unavailable(TemplateFailure::Unreadable)),
        };
        let template_id = signed(&template_word);
        let db = match r.observe::<8>(input.template_db_cell, Failure::StateUnreadable) {
            Ok(bytes) => pointer(&bytes)?, Err(Failure::Bounds) => return Err(Failure::Bounds),
            Err(_) => return Ok(unavailable(TemplateFailure::DatabaseUnavailable)),
        };
        if db == 0 { return Ok(unavailable(TemplateFailure::DatabaseUnavailable)); }
        let db_header = match r.observe::<16>(add(db, 0x28)?, Failure::StateUnreadable) {
            Ok(bytes) => bytes, Err(Failure::Bounds) => return Err(Failure::Bounds),
            Err(_) => return Ok(unavailable(TemplateFailure::DatabaseUnavailable)),
        };
        let db_count = signed(&db_header[12..16]);
        let table = pointer(&db_header[..8])?;
        if template_id < 0 || template_id >= db_count || table == 0 {
            return Ok(unavailable(TemplateFailure::MembershipUnverified));
        }
        let entry = add(table, (template_id as usize).checked_mul(8).ok_or(Failure::Bounds)?)?;
        let member = match r.observe::<8>(entry, Failure::StateUnreadable) {
            Ok(bytes) => pointer(&bytes)?, Err(Failure::Bounds) => return Err(Failure::Bounds),
            Err(_) => return Ok(unavailable(TemplateFailure::MembershipUnverified)),
        };
        if member != template { return Ok(unavailable(TemplateFailure::MembershipUnverified)); }
        Ok(Outcome::Present { slot_count: count, state_id, template_id })
    })();
    group.outcome = match observed {
        Ok(outcome) => match r.close() { Ok(()) => outcome, Err(reason) => Outcome::Failed(reason) },
        Err(reason) => Outcome::Failed(reason),
    };
    group
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct Image(BTreeMap<usize, u8>);
    impl Image {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            for (i, byte) in bytes.iter().enumerate() { self.0.insert(address + i, *byte); }
        }
        fn u64(&mut self, address: usize, value: u64) { self.put(address, &value.to_le_bytes()); }
        fn i32(&mut self, address: usize, value: i32) { self.put(address, &value.to_le_bytes()); }
        fn read(&self, address: usize, out: &mut [u8]) -> bool {
            for (i, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + i)) else { return false; };
                *byte = *value;
            }
            true
        }
    }
    fn fixture() -> (Image, Input) {
        let mut m = Image(BTreeMap::new());
        m.u64(0x1000, 0x111); m.i32(0x1008, 27);
        m.i32(0x2018, 4); m.i32(0x201c, 0);
        m.u64(0x500, 0x3000);
        m.put(0x32c8, &[0; 16]); m.u64(0x32c8, 0x4000); m.i32(0x32d4, 100);
        m.u64(0x4020, 0x5000);
        m.put(0x5000, &[0; 0x5c]); m.u64(0x5000, 0x222); m.u64(0x5030, 0x6000); m.i32(0x5058, 73);
        m.i32(0x60a0, 21); m.u64(0x508, 0x7000);
        m.put(0x7028, &[0; 16]); m.u64(0x7028, 0x8000); m.i32(0x7034, 30); m.u64(0x80a8, 0x6000);
        (m, Input { country: 0x1000, tag_db_id: 27, country_vtable: 0x111, state_vtable: 0x222,
            gs_cell: 0x500, expected_gs: 0x3000, template_db_cell: 0x508,
            operands: Operands { base_raw: Some(4), override_raw: Some(0) } })
    }
    #[test]
    fn actual_selected_reads_keep_three_distinct_ids_and_close_original_objects() {
        let (m, input) = fixture(); let mut trace = Vec::new(); let mut budget = Budget::default();
        let group = capture(input, &mut budget, |a, out| { trace.push((a, out.len())); m.read(a, out) });
        assert_eq!(group.outcome, Outcome::Present { slot_count: 100, state_id: 73, template_id: 21 });
        assert_eq!(trace.iter().filter(|(a, _)| *a == 0x5000).map(|(_, n)| *n).collect::<Vec<_>>(), [0x5c, 8]);
        assert_eq!(trace.iter().filter(|(a, _)| *a == 0x4020).count(), 2);
        assert_eq!(trace.iter().filter(|(a, _)| *a == 0x80a8).count(), 2);
        assert_eq!(budget.bytes, trace.iter().map(|(_, n)| *n).sum::<usize>());
        assert!(budget.bytes <= GROUP_BYTES);
    }
    #[test]
    fn each_closing_identity_header_slot_and_template_mutation_refuses_without_following() {
        for address in [0x1000, 0x1008, 0x2018, 0x201c, 0x500, 0x32c8, 0x4020, 0x5030, 0x5058, 0x60a0, 0x508, 0x7028, 0x80a8] {
            let (m, input) = fixture(); let mut hits = 0; let mut budget = Budget::default();
            let group = capture(input, &mut budget, |a, out| {
                if !m.read(a, out) { return false; }
                // State member opening is contained in the one actual state-prefix read.
                if a == address { hits += 1; if hits == 2 || address == 0x5030 || address == 0x5058 { out[0] ^= 1; } }
                true
            });
            assert_eq!(group.outcome, Outcome::Failed(Failure::Incoherent), "{address:x}");
            assert!(budget.bytes <= GROUP_BYTES);
        }
    }
    #[test]
    fn zero_unknown_negative_range_and_null_are_independent_and_stop_before_dependent_reads() {
        for (override_raw, base_raw, expected, reads_gs) in [
            (Some(-3), None, Outcome::NonPositive, false),
            (Some(0), Some(0), Outcome::NonPositive, false),
            (None, Some(4), Outcome::Failed(Failure::OperandUnobtained), false),
            (Some(0), None, Outcome::Failed(Failure::OperandUnobtained), false),
            (Some(100), None, Outcome::OutOfRange { slot_count: 100 }, true),
        ] {
            let (mut m, mut input) = fixture(); input.operands = Operands { override_raw, base_raw };
            if let Some(value) = override_raw { m.i32(0x201c, value); }
            if let Some(value) = base_raw { m.i32(0x2018, value); }
            let mut addresses = Vec::new();
            let group = capture(input, &mut Budget::default(), |a, out| { addresses.push(a); m.read(a, out) });
            assert_eq!(group.outcome, expected); assert_eq!(addresses.contains(&0x500), reads_gs);
            assert!(!addresses.contains(&0x4020));
        }
        let (mut m, input) = fixture(); m.u64(0x4020, 0);
        assert_eq!(capture(input, &mut Budget::default(), |a, out| m.read(a,out)).outcome,
            Outcome::NullSlot { slot_count: 100 });
    }
    #[test]
    fn template_membership_is_required_and_wrong_object_prefix_never_substitutes() {
        let (mut m, input) = fixture(); m.u64(0x80a8, 0x9000);
        assert_eq!(capture(input, &mut Budget::default(), |a, out| m.read(a,out)).outcome,
            Outcome::TemplateUnavailable { slot_count: 100, state_id: 73, reason: TemplateFailure::MembershipUnverified });
        let (mut m, input) = fixture(); m.u64(0x5000, 0x333);
        assert_eq!(capture(input, &mut Budget::default(), |a, out| m.read(a,out)).outcome,
            Outcome::Failed(Failure::StateUnreadable));
    }
    #[test]
    fn bounds_precede_reads_and_failed_reads_are_not_refunded() {
        let (_, input) = fixture(); let mut budget = Budget { bytes: BATCH_BYTES - 4, groups: 0 }; let mut calls = 0;
        assert_eq!(capture(input, &mut budget, |_, _| { calls += 1; false }).outcome, Outcome::Failed(Failure::Bounds));
        assert_eq!(calls, 0);
        let (_, input) = fixture(); let mut budget = Budget::default();
        assert_eq!(capture(input, &mut budget, |_, _| false).outcome, Outcome::Failed(Failure::CountryIdentity));
        assert_eq!(budget.bytes, 8); assert_eq!(budget.groups, 1);
        let (_, input) = fixture(); budget.groups = MAX_GROUPS;
        assert_eq!(capture(input, &mut budget, |_, _| panic!("read beyond group bound")).outcome, Outcome::Failed(Failure::Bounds));
    }
}
