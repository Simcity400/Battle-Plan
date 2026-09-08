//! Underlying D20 scalars, sampled in the existing registered reader bracket.
use super::*;
use compact_contract::division_routes::{
    TrainingArmy, TrainingCount, TrainingObservation as Obs, TrainingRefusal as Cause,
    TrainingScalars,
};
use country_roster_readset::Budget;
use std::mem::size_of;

struct Witness {
    address: usize,
    width: usize,
    bytes: [u8; 8],
    obtained: bool,
}
struct Block {
    rows: [Option<Witness>; 32],
    used: usize,
    next: Option<Box<Block>>,
}
#[derive(Default)]
struct Trace {
    head: Option<Box<Block>>,
}
impl Trace {
    fn field<const N: usize>(
        &mut self,
        budget: &Budget,
        address: usize,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
    ) -> Result<[u8; N], Cause> {
        if N == 0 || N > 8 || address.checked_add(N).is_none() {
            return Err(Cause::BoundExceeded);
        }
        if self.head.as_ref().is_none_or(|b| b.used == 32) {
            budget
                .retain(size_of::<Block>())
                .map_err(|_| Cause::BoundExceeded)?;
            let next = self.head.take();
            self.head = Some(Box::new(Block {
                rows: std::array::from_fn(|_| None),
                used: 0,
                next,
            }));
        }
        budget
            .reserve_verification(N * 2)
            .map_err(|_| Cause::BoundExceeded)?;
        let mut bytes = [0; 8];
        let obtained = read(address, &mut bytes[..N]);
        let b = self.head.as_mut().unwrap();
        b.rows[b.used] = Some(Witness {
            address,
            width: N,
            bytes,
            obtained,
        });
        b.used += 1;
        if !obtained {
            return Err(Cause::Unreadable);
        }
        Ok(bytes[..N].try_into().unwrap())
    }
    fn close(&mut self, read: &mut impl FnMut(usize, &mut [u8]) -> bool) -> Result<(), Cause> {
        let mut failure = None;
        while let Some(mut block) = self.head.take() {
            self.head = block.next.take();
            for row in block.rows.iter_mut().take(block.used) {
                let w = row.take().unwrap();
                let mut after = [0; 8];
                let obtained = read(w.address, &mut after[..w.width]);
                if !obtained || !w.obtained {
                    failure.get_or_insert(Cause::Unreadable);
                } else if after[..w.width] != w.bytes[..w.width] {
                    failure = Some(Cause::Changed);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
fn at(p: usize, o: usize) -> Result<usize, Cause> {
    p.checked_add(o).ok_or(Cause::BoundExceeded)
}
fn pointer(v: [u8; 8]) -> Result<usize, Cause> {
    usize::try_from(u64::from_le_bytes(v)).map_err(|_| Cause::BoundExceeded)
}
fn admit<T>(
    value: Result<T, Cause>,
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Obs<T> {
    match trace.close(read).and(value) {
        Ok(v) => Obs::Measured(v),
        Err(c) => Obs::Unavailable(c),
    }
}
pub(super) fn sample(
    module: usize,
    body: usize,
    kind: u32,
    id: u32,
    budget: &Budget,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> TrainingScalars {
    let Some(primary) = body.checked_sub(DIVISION_WRAPPER_TO_BODY) else {
        return TrainingScalars::unavailable(Cause::InvalidIdentity);
    };
    let mut identity = Trace::default();
    let authenticated = (|| {
        if !is_map_control_heap_pointer(primary) {
            return Err(Cause::InvalidIdentity);
        }
        let p = u64::from_le_bytes(identity.field::<8>(budget, primary, read)?);
        let b = u64::from_le_bytes(identity.field::<8>(budget, body, read)?);
        let pair = identity.field::<8>(budget, at(body, DIVISION_OFF_ID_TYPE)?, read)?;
        if p != at(module, DIVISION_WRAPPER_VTABLE_RVA)? as u64
            || b != at(module, DIVISION_VTABLE_RVA)? as u64
            || !is_compact_division_body_type(kind)
            || u32::from_le_bytes(pair[..4].try_into().unwrap()) != kind
            || u32::from_le_bytes(pair[4..].try_into().unwrap()) != id
        {
            return Err(Cause::InvalidIdentity);
        }
        Ok(())
    })();
    if let Err(c) = authenticated {
        let closing = identity.close(read);
        return TrainingScalars::unavailable(closing.err().unwrap_or(c));
    }
    let mut traces: [Trace; 6] = std::array::from_fn(|_| Trace::default());
    let experience = (|| {
        Ok(i64::from_le_bytes(traces[0].field::<8>(
            budget,
            at(primary, 0x430)?,
            read,
        )?))
    })();
    let current_hp = (|| {
        Ok(i64::from_le_bytes(traces[1].field::<8>(
            budget,
            at(primary, 0x420)?,
            read,
        )?))
    })();
    let maximum_hp = (|| {
        let stats = pointer(traces[2].field::<8>(budget, at(primary, 0x138)?, read)?)?;
        if !is_map_control_heap_pointer(stats) {
            return Err(Cause::InvalidIdentity);
        }
        if u64::from_le_bytes(traces[2].field::<8>(budget, stats, read)?)
            != at(module, DIVISION_COUNTER_STATS_VTABLE_RVA)? as u64
        {
            return Err(Cause::InvalidIdentity);
        }
        Ok(i64::from_le_bytes(traces[2].field::<8>(
            budget,
            at(stats, 0x288)?,
            read,
        )?))
    })();
    let template_count = (|| {
        let entry_count =
            i32::from_le_bytes(traces[3].field::<4>(budget, at(primary, 0x3ec)?, read)?);
        let mut sum_raw = 0i32;
        if entry_count > 0 {
            let data = pointer(traces[3].field::<8>(budget, at(primary, 0x3e0)?, read)?)?;
            if !is_map_control_heap_pointer(data) {
                return Err(Cause::InvalidIdentity);
            }
            for index in 0..entry_count as usize {
                let offset = index
                    .checked_mul(8)
                    .and_then(|v| v.checked_add(4))
                    .ok_or(Cause::BoundExceeded)?;
                sum_raw = sum_raw.wrapping_add(i32::from_le_bytes(traces[3].field::<4>(
                    budget,
                    at(data, offset)?,
                    read,
                )?));
            }
        }
        Ok(TrainingCount {
            entry_count,
            sum_raw,
        })
    })();
    let army = (|| {
        let army =
            pointer(traces[4].field::<8>(budget, at(body, DIVISION_OFF_ARMY_PARENT)?, read)?)?;
        if army == 0 {
            return Ok(None);
        }
        if !is_map_control_heap_pointer(army) {
            return Err(Cause::InvalidIdentity);
        }
        if u64::from_le_bytes(traces[4].field::<8>(budget, army, read)?)
            != at(module, ARMY_VTABLE_RVA)? as u64
        {
            return Err(Cause::InvalidIdentity);
        }
        let raw = traces[4].field::<8>(budget, at(army, ARMY_OFF_HANDLE)?, read)?;
        let kind = u32::from_le_bytes(raw[..4].try_into().unwrap());
        let id = u32::from_le_bytes(raw[4..].try_into().unwrap());
        if kind != ARMY_TYPE || id == 0 || id > MAX_ARMY_ID {
            return Err(Cause::InvalidIdentity);
        }
        Ok(Some(TrainingArmy { kind, id }))
    })();
    let country_operand = (|| {
        Ok(i32::from_le_bytes(traces[5].field::<4>(
            budget,
            at(primary, 0x1e0)?,
            read,
        )?))
    })();
    let out = TrainingScalars {
        experience: admit(experience, &mut traces[0], read),
        current_hp: admit(current_hp, &mut traces[1], read),
        maximum_hp: admit(maximum_hp, &mut traces[2], read),
        template_count: admit(template_count, &mut traces[3], read),
        army: admit(army, &mut traces[4], read),
        country_operand: admit(country_operand, &mut traces[5], read),
    };
    match identity.close(read) {
        Ok(()) => out,
        Err(c) => TrainingScalars::unavailable(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    const BASE: usize = 0x140000000;
    const PRIMARY: usize = HEAP_LO + 0x2000000;
    const STATS: usize = HEAP_LO + 0x3000000;
    const VALUES: usize = HEAP_LO + 0x4000000;
    const ARMY: usize = HEAP_LO + 0x5000000;
    #[derive(Default)]
    struct Memory {
        bytes: BTreeMap<usize, u8>,
        calls: Vec<(usize, usize)>,
        visits: BTreeMap<usize, usize>,
        change: Option<(usize, usize, Vec<u8>)>,
    }
    impl Memory {
        fn put(&mut self, p: usize, b: &[u8]) {
            for (i, b) in b.iter().enumerate() {
                self.bytes.insert(p + i, *b);
            }
        }
        fn u64(&mut self, p: usize, v: u64) {
            self.put(p, &v.to_le_bytes())
        }
        fn i32(&mut self, p: usize, v: i32) {
            self.put(p, &v.to_le_bytes())
        }
        fn read(&mut self, p: usize, out: &mut [u8]) -> bool {
            self.calls.push((p, out.len()));
            let visit = self.visits.entry(p).or_default();
            *visit += 1;
            if let Some((address, n, value)) = &self.change {
                if *address == p && *n == *visit {
                    let value = value.clone();
                    self.put(p, &value);
                }
            }
            for (i, b) in out.iter_mut().enumerate() {
                let Some(v) = self.bytes.get(&(p + i)) else {
                    return false;
                };
                *b = *v;
            }
            true
        }
    }
    fn fixture() -> Memory {
        let mut m = Memory::default();
        m.u64(PRIMARY, (BASE + DIVISION_WRAPPER_VTABLE_RVA) as u64);
        m.u64(PRIMARY + 0x10, (BASE + DIVISION_VTABLE_RVA) as u64);
        m.u64(PRIMARY + 0x18, (7u64 << 32) | u64::from(DIVISION_TYPE));
        m.u64(PRIMARY + 0x430, i64::MAX as u64);
        m.u64(PRIMARY + 0x420, i64::MIN as u64);
        m.u64(PRIMARY + 0x138, STATS as u64);
        m.u64(STATS, (BASE + DIVISION_COUNTER_STATS_VTABLE_RVA) as u64);
        m.u64(STATS + 0x288, (1u64 << 53) + 1);
        m.i32(PRIMARY + 0x3ec, 3);
        m.u64(PRIMARY + 0x3e0, VALUES as u64);
        for (i, v) in [i32::MAX, 1, -1].into_iter().enumerate() {
            m.i32(VALUES + i * 8 + 4, v);
        }
        m.u64(PRIMARY + 0xc0, 0);
        m.i32(PRIMARY + 0x1e0, -7);
        m
    }
    fn capture(m: &mut Memory, b: &Budget) -> TrainingScalars {
        sample(BASE, PRIMARY + 0x10, DIVISION_TYPE, 7, b, &mut |p, o| {
            m.read(p, o)
        })
    }
    #[test]
    fn exact_signed_operands_and_wrapping_count_survive_real_seals_and_wire() {
        let mut m = fixture();
        let value = capture(&mut m, &Budget::default());
        assert_eq!(value.experience, Obs::Measured(i64::MAX));
        assert_eq!(value.current_hp, Obs::Measured(i64::MIN));
        assert_eq!(value.maximum_hp, Obs::Measured((1i64 << 53) + 1));
        assert_eq!(value.country_operand, Obs::Measured(-7));
        assert_eq!(
            value.template_count,
            Obs::Measured(TrainingCount {
                entry_count: 3,
                sum_raw: i32::MAX
            })
        );
        assert_eq!(value.army, Obs::Measured(None));
        let mut row = super::super::deleted_division_sample(7, DIVISION_TYPE);
        row.deleted = false;
        row.training_scalars = value;
        let bytes = super::super::compact_divisions_data_payload(&[row], 0, 0, 1, 1, None, None);
        let decoded = compact_contract::division_routes::DivisionRoutes::parse(&bytes).unwrap();
        assert_eq!(decoded.training_scalars(0), value);
        let before = m.calls.len() / 2;
        assert_eq!(
            m.calls[..before].iter().map(|(_, n)| n).sum::<usize>(),
            m.calls[before..].iter().map(|(_, n)| n).sum::<usize>()
        );
    }
    #[test]
    fn nonpositive_count_does_not_read_data_pointer_and_null_army_is_not_unreadable() {
        for count in [0, -1, i32::MIN] {
            let mut m = fixture();
            m.i32(PRIMARY + 0x3ec, count);
            for i in 0..8 {
                m.bytes.remove(&(PRIMARY + 0x3e0 + i));
            }
            let v = capture(&mut m, &Budget::default());
            assert_eq!(
                v.template_count,
                Obs::Measured(TrainingCount {
                    entry_count: count,
                    sum_raw: 0
                })
            );
            assert!(!m.calls.iter().any(|(p, _)| *p == PRIMARY + 0x3e0));
            assert_eq!(v.army, Obs::Measured(None));
            m.bytes.remove(&(PRIMARY + 0xc0));
            assert_eq!(
                capture(&mut m, &Budget::default()).army,
                Obs::Unavailable(Cause::Unreadable)
            );
        }
    }
    #[test]
    fn independent_unreadable_and_changed_fields_keep_other_measurements() {
        let mut m = fixture();
        m.bytes.remove(&(STATS + 0x288));
        let v = capture(&mut m, &Budget::default());
        assert_eq!(v.maximum_hp, Obs::Unavailable(Cause::Unreadable));
        assert_eq!(v.current_hp, Obs::Measured(i64::MIN));
        assert_eq!(
            m.visits[&(STATS + 0x288)],
            2,
            "failed opening still attempts funded close"
        );
        let mut m = fixture();
        m.change = Some((VALUES + 4, 2, 0i32.to_le_bytes().to_vec()));
        let v = capture(&mut m, &Budget::default());
        assert_eq!(v.template_count, Obs::Unavailable(Cause::Changed));
        assert_eq!(v.experience, Obs::Measured(i64::MAX));
    }
    #[test]
    fn army_and_primary_identity_changes_never_follow_replacement() {
        let mut m = fixture();
        m.u64(PRIMARY + 0xc0, ARMY as u64);
        m.u64(ARMY, (BASE + ARMY_VTABLE_RVA) as u64);
        m.u64(ARMY + ARMY_OFF_HANDLE, (19u64 << 32) | u64::from(ARMY_TYPE));
        assert_eq!(
            capture(&mut m, &Budget::default()).army,
            Obs::Measured(Some(TrainingArmy {
                kind: ARMY_TYPE,
                id: 19
            }))
        );
        let mut m = fixture();
        m.change = Some((
            PRIMARY + 0x18,
            2,
            ((8u64 << 32) | u64::from(DIVISION_TYPE))
                .to_le_bytes()
                .to_vec(),
        ));
        assert_eq!(
            capture(&mut m, &Budget::default()),
            TrainingScalars::unavailable(Cause::Changed)
        );
        assert!(m.calls.iter().filter(|(p, _)| *p == PRIMARY + 0x18).count() == 2);
    }
    #[test]
    fn budget_exhaustion_keeps_every_reserved_close_without_inventing_count() {
        let mut m = fixture();
        let b = Budget::test_limits(100, 16 * 1024 * 1024);
        let v = capture(&mut m, &b);
        assert!(matches!(
            v.template_count,
            Obs::Unavailable(Cause::BoundExceeded)
        ));
        let mut counts = BTreeMap::new();
        for call in &m.calls {
            *counts.entry(*call).or_insert(0) += 1;
        }
        assert!(counts.values().all(|n| *n == 2));
        assert!(m.calls.iter().map(|(_, n)| n).sum::<usize>() <= 100);
        assert_eq!(
            b.verification_bytes(),
            101,
            "failed reservation poisons accounting, not extra physical read"
        );
    }

    fn observed_row(m: &mut Memory) -> DivisionMovementSample {
        let mut row = super::super::deleted_division_sample(7, DIVISION_TYPE);
        row.deleted = false;
        row.location = 1;
        row.raw_max_organisation = Some(100_000);
        row.training_scalars = capture(m, &Budget::default());
        row
    }
    fn emit(
        m: &mut Memory,
        tracker: &mut FrameJournalled<BTreeMap<u32, DivisionMovementSample>>,
    ) -> Vec<u8> {
        let (rows, changed, flags) =
            compact_division_emitted_rows(vec![observed_row(m)], 1, 0, tracker);
        compact_divisions_data_payload(&rows, 0, flags, changed, 1, None, None)
    }
    #[test]
    fn journal_rollback_reemits_changed_raw_operand_and_unavailable_overwrites() {
        let mut m = fixture();
        let mut tracker = FrameJournalled::default();
        let first = emit(&mut m, &mut tracker);
        assert_eq!(u32::from_le_bytes(first[8..12].try_into().unwrap()), 1);
        tracker.close_frame_transaction();
        m.u64(PRIMARY + 0x430, 19);
        let rejected = emit(&mut m, &mut tracker);
        tracker.restore_frame_entry();
        assert_eq!(
            emit(&mut m, &mut tracker),
            rejected,
            "withheld frame cannot advance the baseline"
        );
        tracker.close_frame_transaction();
        m.bytes.remove(&(PRIMARY + 0x430));
        let unavailable = emit(&mut m, &mut tracker);
        let decoded =
            compact_contract::division_routes::DivisionRoutes::parse(&unavailable).unwrap();
        assert_eq!(
            u32::from_le_bytes(unavailable[8..12].try_into().unwrap()),
            1
        );
        assert_eq!(
            decoded.training_scalars(0).experience,
            Obs::Unavailable(Cause::Unreadable)
        );
    }
    #[test]
    #[ignore = "exports exclusive hash-bound sampler/publisher/fold input for the core consumer"]
    fn actual_sampler_publisher_fold_training_stages() {
        use super::super::province_occupants::tests::stage;
        use std::io::Write;
        const HOUR: i32 = DATE_EPOCH_CONST + 16_959_367;
        let destination = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_D2R9_JOINT_OUTPUT").or_else(|| std::env::var_os("HOI4_D2R9_JOINT_OUTPUT")).expect("exclusive output required"),
        );
        assert!(destination.is_absolute());
        let mut m = fixture();
        let mut tracker = FrameJournalled::default();
        let mut stream = Vec::new();
        let mut append = |sequence, hour, bytes: &[u8]| {
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                20,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                bytes,
            );
            stage(&stream, sequence + 1, hour)
        };
        let emit_experience = |m: &mut Memory,
                               tracker: &mut FrameJournalled<BTreeMap<u32, DivisionMovementSample>>,
                               expected| {
            let payload = emit(m, tracker);
            assert_eq!(u32::from_le_bytes(payload[8..12].try_into().unwrap()), 1,
                "the fixture must emit an actual division row before corruption");
            assert_eq!(u32::from_le_bytes(payload[12..16].try_into().unwrap()), 1,
                "the observed operand change must reach the delta publisher");
            let parsed = compact_contract::division_routes::DivisionRoutes::parse(&payload)
                .expect("uncorrupted producer payload");
            assert_eq!(parsed.training_scalars(0).experience, expected);
            payload
        };
        let measured = append(1, HOUR,
            &emit_experience(&mut m, &mut tracker, Obs::Measured(i64::MAX)));
        tracker.close_frame_transaction();
        m.bytes.remove(&(PRIMARY + 0x430));
        let unavailable = append(2, HOUR,
            &emit_experience(&mut m, &mut tracker, Obs::Unavailable(Cause::Unreadable)));
        tracker.close_frame_transaction();
        m.u64(PRIMARY + 0x430, 19);
        let mut hostile = emit_experience(&mut m, &mut tracker, Obs::Measured(19));
        assert_ne!(u32::from_le_bytes(hostile[16..20].try_into().unwrap())
            & COMPACT_STATUS_DELTA_DATA, 0, "corrupt an actual changed DATA delta");
        hostile[32 + 216..32 + 220].copy_from_slice(&99u32.to_le_bytes());
        let refused = append(3, HOUR, &hostile);
        // Model a DATA receiver rejection after the producer advanced; this is not a sink rollback.
        tracker.close_frame_transaction();
        let empty = emit(&mut m, &mut tracker);
        assert_eq!(u32::from_le_bytes(empty[8..12].try_into().unwrap()), 0);
        let empty_after_refusal = append(4, HOUR, &empty);
        let full = emit(&mut m, &mut FrameJournalled::default());
        let raw_recovery = append(5, HOUR, &full);
        drop(append);
        append_compact_record(
            &mut stream,
            6,
            HOUR,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        append_compact_record(
            &mut stream,
            7,
            HOUR + 1,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &full,
        );
        let next_hour = stage(&stream, 8, HOUR + 1);
        assert_eq!(
            measured["snapshot"]["divisions"][0]["training_scalars"]["experience"]["raw_i64"],
            i64::MAX
        );
        assert_eq!(
            unavailable["snapshot"]["divisions"][0]["training_scalars"]["experience"]["reason"],
            "unreadable"
        );
        for value in [&refused, &empty_after_refusal, &raw_recovery] {
            assert!(value["snapshot"]["divisions"].is_null());
            assert!(
                value["snapshot"]["provenance"]["collector"]["domain_problems"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["domain_id"] == 20)
            );
        }
        assert_eq!(
            next_hour["snapshot"]["divisions"][0]["training_scalars"]["experience"]["raw_i64"],
            19
        );
        let output = serde_json::to_vec(&serde_json::json!({"schema":1,"stages":[measured,unavailable,refused,empty_after_refusal,raw_recovery,next_hour]})).unwrap();
        assert!(output.len() <= 1024 * 1024);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .unwrap();
        file.write_all(&output).unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn output_layout_is_reserved_before_sampling_and_exact_limit_admits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let roots = (0..1024)
            .map(|i| DivisionBodyBridgeEntry {
                div_id: i + 1,
                body_ptr: 0,
                owner_tag_db_id: 0,
            })
            .collect::<Vec<_>>();
        let workers = compact_division_sample_worker_count(roots.len());
        let bytes = roots.len()
            * (size_of::<Option<DivisionMovementSample>>() + size_of::<DivisionMovementSample>())
            + if workers > 1 {
                workers * size_of::<thread::ScopedJoinHandle<'_, ()>>()
            } else {
                0
            };
        let calls = AtomicUsize::new(0);
        let sample = |_, row: &DivisionBodyBridgeEntry, _: &Budget| {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(super::super::deleted_division_sample(
                row.div_id,
                DIVISION_TYPE,
            ))
        };
        let limited = Budget::test_limits(0, bytes - 1);
        assert!(matches!(
            sample_compact_division_roots_with(BASE, &roots, &limited, &sample),
            Err(DivisionSampleFailure::OutputStorageUnavailable)
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no body read before output reservation"
        );
        assert_eq!(limited.retained_bytes(), bytes);
        let exact = Budget::test_limits(0, bytes);
        let rows = sample_compact_division_roots_with(BASE, &roots, &exact, &sample).unwrap();
        assert_eq!(rows.len(), roots.len());
        assert_eq!(
            rows.iter().map(|r| r.div_id).collect::<Vec<_>>(),
            (1..=1024).collect::<Vec<_>>()
        );
        assert_eq!(calls.load(Ordering::SeqCst), roots.len());
        assert_eq!(exact.retained_bytes(), bytes);
        assert_eq!(exact.verification_bytes(), 0);
    }
    #[test]
    fn simultaneous_output_reservations_share_quota_without_refund() {
        let bytes =
            size_of::<Option<DivisionMovementSample>>() + size_of::<DivisionMovementSample>();
        let budget = Budget::test_limits(0, bytes);
        let root = [DivisionBodyBridgeEntry {
            div_id: 7,
            body_ptr: 0,
            owner_tag_db_id: 0,
        }];
        let results = thread::scope(|scope| {
            let first = scope.spawn(|| {
                sample_compact_division_roots_with(BASE, &root, &budget, |_, r, _| {
                    Some(super::super::deleted_division_sample(
                        r.div_id,
                        DIVISION_TYPE,
                    ))
                })
            });
            let second = scope.spawn(|| {
                sample_compact_division_roots_with(BASE, &root, &budget, |_, r, _| {
                    Some(super::super::deleted_division_sample(
                        r.div_id,
                        DIVISION_TYPE,
                    ))
                })
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(DivisionSampleFailure::OutputStorageUnavailable)))
                .count(),
            1
        );
        assert_eq!(budget.retained_bytes(), bytes + 1);
    }
}
