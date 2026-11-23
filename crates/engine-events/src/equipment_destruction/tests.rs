use super::*;
const ADD_QUANTITY_BOUND: i64 = 0x53e2d620fb40;
fn row(design: u64, amount: i64) -> equipment_entry::Row {
    equipment_entry::Row {
        variant_handle: design,
        amount_raw: amount,
        ..Default::default()
    }
}
#[test]
fn signed_removals_keep_designs_duplicates_and_noops() {
    let mut before = [row(70, 450000), row(0xabc00000046, 300000), row(70, 50000)];
    let mut after = [row(70, 370000), row(0xabc00000046, 300000), row(70, 30000)];
    let mut events = Vec::new();
    removals(&mut before, &mut after, |d, a| events.push((d, a))).unwrap();
    assert_eq!(events, [(70, 100000)]);
    events.clear();
    removals(&mut after.clone(), &mut after, |d, a| events.push((d, a))).unwrap();
    assert!(events.is_empty());
    removals(&mut [row(70, -1)], &mut [row(70, 2), row(71, 3)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, -3), (71, -3)]);
}

#[test]
fn signed_removals_accept_unclamped_native_quantities_and_refuse_sum_overflow() {
    for quantity in [i64::MAX / 100000, i64::MIN] {
        let mut events = Vec::new();
        removals(&mut [row(70, quantity)], &mut [], |d, q| {
            events.push((d, q))
        })
        .unwrap();
        assert_eq!(events, [(70, quantity)]);
    }
    let count = (i64::MAX / ADD_QUANTITY_BOUND + 1) as usize;
    let mut before = vec![row(70, ADD_QUANTITY_BOUND); count];
    assert_eq!(
        removals(&mut before, &mut [], |_, _| {}),
        Err("destruction design quantity overflow")
    );
}
#[test]
fn callers_are_classified_semantically_and_unknown_is_preserved() {
    assert_eq!(land_cause(0, LAND_COMBAT_RETURN.0), Cause::LandCombat);
    assert_eq!(
        land_cause(0, NAVAL_TRANSPORT_RETURN.0),
        Cause::NavalTransport
    );
    assert_eq!(land_cause(0, IMMEDIATE_RETURN.0), Cause::Immediate);
    assert_eq!(land_cause(0, LAND_COMBAT_RETURN.0 + 1), Cause::Unknown);
    for (source, expected) in AIR_CAUSES {
        assert_eq!(air_cause(0, source.0), expected);
        assert_eq!(air_cause(0, source.0 + 1), Cause::Unknown);
    }
    assert_eq!(air_cause(0, 0), Cause::Unknown);
    assert_eq!(Cause::default(), Cause::Unknown);
}

#[test]
fn bounded_variant_copy_sums_signed_duplicates_and_refuses_short_or_unreadable() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0xc0];
    bytes[..8].copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x20..0x28].copy_from_slice(&0x1080u64.to_le_bytes());
    bytes[0x28..0x2c].copy_from_slice(&3i32.to_le_bytes());
    bytes[0x2c..0x30].copy_from_slice(&3i32.to_le_bytes());
    for (i, (design, amount)) in [(0x2000u64, 450001i64), (0x3000, 7), (0x2000, 50000)]
        .into_iter()
        .enumerate()
    {
        bytes[0x80 + i * 16..0x88 + i * 16].copy_from_slice(&design.to_le_bytes());
        bytes[0x88 + i * 16..0x90 + i * 16].copy_from_slice(&amount.to_le_bytes());
    }
    let mem = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        variant_amount(&mem, 0, 0x1000, 0x2000, &mut [0; 48]),
        Ok(500001)
    );
    assert_eq!(variant_amount(&mem, 0, 0x1000, 0x4000, &mut [0; 48]), Ok(0));
    assert!(variant_amount(&mem, 0, 0x1000, 0x2000, &mut [0; 47]).is_err());
    assert!(
        variant_amount(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes[..0xaf]
            },
            0,
            0x1000,
            0x2000,
            &mut [0; 48]
        )
        .is_err()
    );
    bytes[0x88..0x90].copy_from_slice(&(-1i64).to_le_bytes());
    assert_eq!(
        variant_amount(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000,
            0x2000,
            &mut [0; 48]
        ),
        Ok(49999)
    );
    for quantity in [i64::MAX / 100000, i64::MIN] {
        bytes[0x88..0x90].copy_from_slice(&quantity.to_le_bytes());
        assert_eq!(
            variant_amount(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                0x2000,
                &mut [0; 48]
            ),
            Ok(quantity + 50000)
        );
    }
    bytes[0x88..0x90].copy_from_slice(&(-ADD_QUANTITY_BOUND - 1).to_le_bytes());
    assert_eq!(
        variant_amount(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000,
            0x2000,
            &mut [0; 48]
        ),
        Ok(-ADD_QUANTITY_BOUND - 1 + 50000)
    );
}

#[test]
fn original_pool_demand_survives_short_storage_without_extra_native_reads() {
    use engine_native_memory::memory::SliceMemory;
    use std::cell::{Cell, RefCell};
    struct Traced<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.memory.copy(address, output)
        }
    }
    for count in [5000i32, 10003] {
        let mut bytes = [0u8; 0x40];
        bytes[..8].copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x20..0x28].copy_from_slice(&0x2000u64.to_le_bytes());
        bytes[0x28..0x2c].copy_from_slice(&count.to_le_bytes());
        bytes[0x2c..0x30].copy_from_slice(&count.to_le_bytes());
        let memory = Traced {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            reads: RefCell::new(Vec::new()),
        };
        let expected = snapshot(&memory, 0, 0x1000, &mut []);
        let reads = memory.reads.take();
        let demand = Cell::new(0);
        assert_eq!(
            snapshot_with_demand(&memory, 0, 0x1000, &mut [], |count| demand.set(count)),
            expected
        );
        assert_eq!(demand.get(), count as usize);
        assert_eq!(memory.reads.take(), reads);
        let expected = variant_amount(&memory, 0, 0x1000, 0x3000, &mut []);
        let reads = memory.reads.take();
        demand.set(0);
        assert_eq!(
            variant_amount_with_demand(&memory, 0, 0x1000, 0x3000, &mut [], |count| demand
                .set(count)),
            expected
        );
        assert_eq!(demand.get(), count as usize);
        assert_eq!(memory.reads.take(), reads);
    }
}

#[test]
fn signed_snapshot_preserves_full_native_signed_quantities() {
    use engine_native::cequipment_variant;
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0x180];
    bytes[..8].copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x20..0x28].copy_from_slice(&0x1080u64.to_le_bytes());
    bytes[0x28..0x2c].copy_from_slice(&2i32.to_le_bytes());
    bytes[0x2c..0x30].copy_from_slice(&2i32.to_le_bytes());
    bytes[0x100..0x108].copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
    let handle = (5000u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE);
    bytes[0x108..0x110].copy_from_slice(&handle.to_le_bytes());
    for (index, quantity) in [ADD_QUANTITY_BOUND, -ADD_QUANTITY_BOUND]
        .into_iter()
        .enumerate()
    {
        let at = 0x80 + index * equipment_entry::STRIDE;
        bytes[at..at + 8].copy_from_slice(&0x1100u64.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&quantity.to_le_bytes());
    }
    let mut output = [equipment_entry::Row::default(); 2];
    assert_eq!(
        snapshot(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000,
            &mut output
        ),
        Ok(2)
    );
    assert_eq!(output[0].amount_raw, ADD_QUANTITY_BOUND);
    assert_eq!(output[1].amount_raw, -ADD_QUANTITY_BOUND);
    for invalid in [i64::MAX / 100000, i64::MAX, i64::MIN] {
        bytes[0x88..0x90].copy_from_slice(&invalid.to_le_bytes());
        assert_eq!(
            snapshot(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut output
            ),
            Ok(2)
        );
        assert_eq!(output[0].amount_raw, invalid);
    }
}

#[test]
fn native_owners_follow_expeditionary_roster_wing_and_controller() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0xc00];
    let write32 = |bytes: &mut [u8], at: usize, value: i32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
    };
    write32(&mut bytes, engine_native::cunit::ROSTER_OWNER, 4999);
    write32(&mut bytes, engine_native::cunit::LOGICAL_OWNER, 1234);
    for expeditionary in [-1, 0, 1500] {
        write32(&mut bytes, engine_native::cunit::EXPEDITIONARY_OWNER, expeditionary);
        assert_eq!(
            army_owner(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000
            )
            .unwrap(),
            if expeditionary > 0 { 1500 } else { 4999 }
        );
        assert_eq!(
            army_expeditionary_owner(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000
            ),
            Ok(expeditionary as u32)
        );
    }
    bytes[0x38..0x40].copy_from_slice(&0x1100u64.to_le_bytes());
    bytes[0x140..0x148].copy_from_slice(&0x1200u64.to_le_bytes());
    write32(&mut bytes, 0x290, 5000);
    for direct in [0, 1500] {
        write32(&mut bytes, 0x9c4, direct);
        assert_eq!(
            air_owner(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000
            )
            .unwrap(),
            if direct > 0 { 1500 } else { 5000 }
        );
    }
    bytes[0x50..0x58].copy_from_slice(&0x1300u64.to_le_bytes());
    write32(&mut bytes, 0x3cc, 4999);
    assert_eq!(
        garrison_owner(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0x1000
        ),
        Ok(4999)
    );
    assert!(
        garrison_owner(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes[..0x3cf]
            },
            0x1000
        )
        .is_err()
    );
}

#[test]
fn deletion_paths_cleanup_transfers_and_unknown_are_explicit() {
    for (pc, cause) in DELETION_CAUSES {
        assert_eq!(deletion_context(0, pc.0), DeletionContext::Loss(cause));
    }
    for pc in IGNORED_DELETIONS {
        assert_eq!(deletion_context(0, pc.0), DeletionContext::Ignore);
    }
    assert_eq!(
        deletion_context(0, QUEUED_DELETE_RETURN.0),
        DeletionContext::Queue
    );
    assert_eq!(
        deletion_context(0, usize::MAX),
        DeletionContext::Loss(Cause::Unknown)
    );
    for (pc, cause) in QUEUED_DELETION_CAUSES {
        assert_eq!(queued_context(0, pc.0), QueuedContext::Loss(cause));
    }
    assert_eq!(
        deletion_context(0, MERGED_DELETION_RETURN.0),
        DeletionContext::Transfer(Disposition::Merged)
    );
    assert_eq!(
        queued_context(0, HQ_PROVIDER_RETURN.0),
        QueuedContext::Transfer(Disposition::HqTransferred)
    );
    assert_eq!(
        queued_context(0, FAKE_INTEL_QUEUE_RETURN.0),
        QueuedContext::Ignore
    );
    assert_eq!(
        queued_context(0, FAKE_INTEL_QUEUE_RETURN.0 + 1),
        QueuedContext::Loss(Cause::Unknown)
    );
}

#[test]
fn site_metadata_keeps_call_targets_and_install_groups_explicit() {
    for site in SITES {
        assert_eq!(SITES[site.id as usize].id, site.id);
    }
    assert_eq!(
        SITES
            .iter()
            .filter(|site| !site.id.is_supplemental())
            .count(),
        4
    );
    assert_eq!(
        SITES
            .iter()
            .filter(|site| site.id.is_supplemental())
            .count(),
        SITES.len() - 4
    );
}

#[test]
fn fake_intel_uses_the_recorded_census_flag_and_refuses_short_reads() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0x700];
    let army = 0x1000;
    let template = army + 0x400;
    let flag_at = 0x400 + engine_native::creferenced_division_template::REVEAL_SKIP;
    assert_eq!(
        fake_intel_division(
            &SliceMemory {
                base: army,
                bytes: &bytes
            },
            army
        ),
        Ok(false)
    );
    bytes[carmy::STANDARD_TEMPLATE..carmy::STANDARD_TEMPLATE + 8]
        .copy_from_slice(&(template as u64).to_le_bytes());
    for flag in [0, 1, 255] {
        bytes[flag_at] = flag;
        assert_eq!(
            fake_intel_division(
                &SliceMemory {
                    base: army,
                    bytes: &bytes
                },
                army
            ),
            Ok(flag != 0)
        );
    }
    assert!(
        fake_intel_division(
            &SliceMemory {
                base: army,
                bytes: &bytes[..flag_at]
            },
            army
        )
        .is_err()
    );
}

#[test]
fn division_identity_authenticates_both_vtables_and_dynamic_handles() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0x200];
    bytes[..8].copy_from_slice(&(carmy::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[0x10..0x18].copy_from_slice(&(carmy::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes());
    for kind in [0u32, 17, 4713, u32::MAX] {
        let handle = (5000u64 << 32) | u64::from(kind);
        bytes[0x18..0x20].copy_from_slice(&handle.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(army_identity(&memory, 0, 0x1000), Ok(handle));
        assert_eq!(is_army(&memory, 0, 0x1000), Ok(true));
    }
    bytes[0x10] ^= 1;
    assert!(
        army_identity(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000
        )
        .is_err()
    );
    bytes[0x10] ^= 1;
    bytes[0x18..0x20].fill(0);
    assert!(
        army_identity(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000
        )
        .is_err()
    );
    bytes[8..12].copy_from_slice(&13u32.to_le_bytes());
    assert_eq!(
        is_army(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000
        ),
        Ok(false)
    );
}

#[test]
fn reset_and_current_game_state_reads_fail_closed() {
    struct Fields(Vec<(usize, Vec<u8>)>);
    impl Memory for Fields {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (start, source) in &self.0 {
                if let Some(offset) = address.checked_sub(*start)
                    && let Some(bytes) =
                        source.get(offset..offset.saturating_add(destination.len()))
                {
                    destination.copy_from_slice(bytes);
                    return true;
                }
            }
            false
        }
    }
    let mut memory = Fields(vec![
        (
            engine_native::cgame_state::SINGLETON_RVA.0,
            0x1000u64.to_le_bytes().to_vec(),
        ),
        (0x1000 + RESETTING, vec![0]),
    ]);
    assert_eq!(current_state(&memory, 0, 0x1000), Ok(()));
    assert!(current_state(&memory, 0, 0x2000).is_err());
    assert_eq!(cleanup(&memory, 0), Ok(false));
    memory.0[1].1[0] = 1;
    assert_eq!(cleanup(&memory, 0), Ok(true));
    memory.0[1].1[0] = 2;
    assert!(cleanup(&memory, 0).is_err());
    memory.0[1].1.clear();
    assert!(cleanup(&memory, 0).is_err());
}

#[test]
fn paradrop_records_actual_division_and_flooring_loss() {
    let design = 0xabc00000046;
    let mut before = [row(design, 450001), row(70, 300000)];
    let mut after = [row(design, 200000), row(70, 100000)];
    let mut events = Vec::new();
    removals(&mut before, &mut after, |d, q| events.push((d, q))).unwrap();
    assert_eq!(events, [(70, 200000), (design, 250001)]);
}

#[test]
fn live_pool_signed_nets_include_rounding_gains_and_real_removals() {
    let mut before = [row(70, 450001), row(71, 199999), row(72, 59999)];
    let mut after = [row(70, 400000), row(71, 200000), row(72, 100000)];
    let mut events = Vec::new();
    removals(&mut before, &mut after, |d, q| events.push((d, q))).unwrap();
    assert_eq!(events, [(70, 50001), (71, -1), (72, -40001)]);

    events.clear();
    removals(&mut [row(70, 400001)], &mut [row(70, 400001)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert!(events.is_empty());
    removals(&mut [row(70, 400001)], &mut [row(70, 600001)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, -200000)]);
    events.clear();
    removals(&mut [row(70, 400001)], &mut [row(70, 160000)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, 240001)]);
}

#[test]
fn negative_damage_gain_is_signed() {
    let mut events = Vec::new();
    removals(&mut [row(70, 1000000)], &mut [row(70, 1100000)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, -100000)]);
}

#[test]
fn fractional_pool_rounding_gain_is_signed() {
    let mut events = Vec::new();
    removals(&mut [row(70, 1030000)], &mut [row(70, 1100000)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, -70000)]);
}

#[test]
fn scripted_factor_above_one_and_sign_flip_are_signed() {
    let mut events = Vec::new();
    removals(&mut [row(70, 400001)], &mut [row(70, 800002)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, -400001)]);
    events.clear();
    removals(&mut [row(70, 400001)], &mut [row(70, -400001)], |d, q| {
        events.push((d, q));
    })
    .unwrap();
    assert_eq!(events, [(70, 800002)]);
}

#[test]
fn scripted_reserve_owner_authenticates_both_objects_and_reverse_link() {
    use engine_native::{ccountry, cproduction_status};
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0u8; 0x2400];
    let country = 0x1000;
    let manager = 0x3000;
    let pool = manager + cproduction_status::STOCKPILE_POOL;
    bytes[..8].copy_from_slice(&(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[ccountry::ID..ccountry::ID + 4].copy_from_slice(&5000u32.to_le_bytes());
    bytes[ccountry::PRODUCTION_STATUS..ccountry::PRODUCTION_STATUS + 8]
        .copy_from_slice(&(manager as u64).to_le_bytes());
    bytes[0x2000..0x2008].copy_from_slice(&(cproduction_status::VTABLE_RVA.0 as u64).to_le_bytes());
    let owner_at = 0x2000 + cproduction_status::COUNTRY;
    bytes[owner_at..owner_at + 8].copy_from_slice(&(country as u64).to_le_bytes());
    let check = |bytes: &[u8]| {
        reserve_owner(
            &SliceMemory {
                base: country,
                bytes,
            },
            0,
            pool,
        )
    };
    assert_eq!(check(&bytes), Ok(5000));
    bytes[0x2000] ^= 1;
    assert!(check(&bytes).is_err());
    bytes[0x2000] ^= 1;
    bytes[0] ^= 1;
    assert!(check(&bytes).is_err());
    bytes[0] ^= 1;
    bytes[ccountry::PRODUCTION_STATUS] ^= 1;
    assert!(check(&bytes).is_err());
    bytes[ccountry::PRODUCTION_STATUS] ^= 1;
    assert!(check(&bytes[..owner_at + 7]).is_err());
    assert!(
        reserve_owner(
            &SliceMemory {
                base: country,
                bytes: &bytes
            },
            0,
            1
        )
        .is_err()
    );
}
