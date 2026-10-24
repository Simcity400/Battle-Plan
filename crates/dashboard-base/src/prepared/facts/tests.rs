use super::*;
use crate::prepared::series::{self, tests::Fixture};

fn limits() -> Limits {
    Limits {
        write_bytes: 8192,
        cache_bytes: 1 << 20,
        page_bytes: 4096,
    }
}
fn facts<V: PartialEq + Serialize + DeserializeOwned + Resident + 'static>() -> Facts<u32, V> {
    Facts::new(
        Namespace::new("synthetic", 7, "values").unwrap(),
        MemoryBudget::new(64 << 20),
        limits(),
    )
    .unwrap()
}
fn value<V: Clone, R: ReadPages>(
    reader: &mut Reader<'_, u32, V, R>,
    key: u32,
    hour: i32,
) -> Option<Status<V>>
where
    V: PartialEq + Serialize + DeserializeOwned + Resident + 'static,
{
    reader.at(&key, hour).unwrap().map(|row| row.status.clone())
}
#[test]
fn gaps_absence_unavailable_and_recorded_zero_remain_distinct_at_native_extremes() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<i64>();
    facts
        .append(&mut writer, 1, i32::MIN, -10, Status::Value(0))
        .unwrap();
    facts
        .append(&mut writer, 1, -9, -1, Status::Value(0))
        .unwrap();
    facts
        .append(&mut writer, 1, 1000, 1005, Status::Absent)
        .unwrap();
    facts
        .append(
            &mut writer,
            1,
            1006,
            i32::MAX,
            Status::Unavailable("mod-specific source".into()),
        )
        .unwrap();
    let mut reader = facts.reader(&mut writer).unwrap();
    assert_eq!(value(&mut reader, 1, i32::MIN), Some(Status::Value(0)));
    assert_eq!(value(&mut reader, 1, 0), Some(Status::Gap));
    assert_eq!(value(&mut reader, 1, 1000), Some(Status::Absent));
    assert_eq!(value(&mut reader, 2, 1000), None);
    assert_eq!(
        value(&mut reader, 1, i32::MAX),
        Some(Status::Unavailable("mod-specific source".into()))
    );
    let mut rows = Vec::new();
    reader
        .visit(&1, -20, 1010, |a, b, v| {
            rows.push((a, b, v.clone()));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (-20, -1, Status::Value(0)),
            (0, 999, Status::Gap),
            (1000, 1005, Status::Absent),
            (
                1006,
                1010,
                Status::Unavailable("mod-specific source".into())
            )
        ]
    );
}
#[test]
fn private_spill_is_independent_of_publication_and_one_change_does_not_copy_the_world() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut names = facts::<String>();
    let mut reads = Facts::<u32, bool>::new(
        Namespace::new("synthetic", 7, "read-evidence").unwrap(),
        names.budget.clone(),
        limits(),
    )
    .unwrap();
    for key in 0..1500 {
        names
            .append(
                &mut writer,
                key,
                0,
                1000,
                Status::Value(format!("country-{key:04}")),
            )
            .unwrap();
    }
    for hour in 0..1000 {
        reads
            .append(&mut writer, 0, hour, hour, Status::Value(hour % 2 == 0))
            .unwrap();
        assert!(reads.buffered_bytes() < limits().write_bytes);
    }
    names
        .append(&mut writer, 7, 1001, 1001, Status::Value("new-name".into()))
        .unwrap();
    assert_eq!(
        names
            .columns
            .values()
            .filter(|c| !c.pending.is_empty())
            .count(),
        1
    );
    assert_eq!(names.active_keys(), 1500);
    assert!(writer.bytes_written() > 0);
    assert!(writer.read_at("public-values", 0).unwrap().is_none());
    let mut reader = names.reader(&mut writer).unwrap();
    for key in 0..1500 {
        assert_eq!(
            value(&mut reader, key, 500),
            Some(Status::Value(format!("country-{key:04}")))
        );
    }
}
#[test]
fn long_gapped_history_matches_an_independent_hour_map_before_and_after_restore() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<u64>();
    let mut oracle = BTreeMap::new();
    let mut previous = None;
    for step in 0..8192_i32 {
        let first = step * 37;
        if let Some(last) = previous {
            for hour in last + 1..first {
                oracle.insert(hour, Status::Gap);
            }
        }
        let status = match step % 7 {
            0 => Status::Absent,
            1 => Status::Unavailable("recorded refusal".into()),
            _ => Status::Value((step % 19) as u64),
        };
        for hour in first..=first + 2 {
            oracle.insert(hour, status.clone());
        }
        previous = Some(first + 2);
        facts
            .append(&mut writer, 9, first, first + 2, status)
            .unwrap();
    }
    let checkpoint = facts.checkpoint(&mut writer, 8192).unwrap();
    let small = serde_json::to_vec(&checkpoint).unwrap();
    assert!(small.len() < 256);
    writer.commit().unwrap();
    drop(facts);
    let mut session = fixture.store.session(&fixture.key).unwrap().unwrap();
    let facts = Facts::<u32, u64>::restore(
        &mut session,
        &checkpoint,
        MemoryBudget::new(64 << 20),
        limits(),
    )
    .unwrap();
    let mut reader = facts.reader(&mut session).unwrap();
    for (&hour, expected) in oracle.iter().step_by(23) {
        assert_eq!(value(&mut reader, 9, hour).as_ref(), Some(expected));
    }
    let mut cursor = reader.cursor(&9, 0, i32::MAX).unwrap();
    let mut represented = 0usize;
    while let Some(row) = reader.next(&mut cursor).unwrap() {
        for hour in row.first..=row.last {
            assert_eq!(oracle.get(&hour), Some(&row.status));
            represented += 1;
        }
    }
    assert_eq!(represented, oracle.len());
    assert!(reader.cached_bytes() <= limits().cache_bytes);
}
#[test]
fn retirement_keeps_closed_keys_on_disk_and_reopening_preserves_gaps() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<String>();
    for key in 0..1500 {
        facts
            .append(
                &mut writer,
                key,
                0,
                3,
                Status::Value(format!("country-{key}")),
            )
            .unwrap();
        facts.retire(&mut writer, &key).unwrap();
        assert_eq!(facts.active_keys(), 0);
    }
    let checkpoint = facts.checkpoint(&mut writer, 1).unwrap();
    writer.commit().unwrap();
    drop(facts);
    let mut writer = fixture.writer();
    let mut facts = Facts::<u32, String>::restore(
        &mut writer,
        &checkpoint,
        MemoryBudget::new(64 << 20),
        limits(),
    )
    .unwrap();
    assert_eq!(facts.active_keys(), 0);
    facts
        .append(&mut writer, 1499, 10, 11, Status::Value("later".into()))
        .unwrap();
    let mut reader = facts.reader(&mut writer).unwrap();
    for key in 0..1500 {
        assert_eq!(
            value(&mut reader, key, 2),
            Some(Status::Value(format!("country-{key}")))
        );
    }
    assert_eq!(value(&mut reader, 1499, 7), Some(Status::Gap));
    assert_eq!(
        value(&mut reader, 1499, 10),
        Some(Status::Value("later".into()))
    );
    assert_eq!(value(&mut reader, 1498, 10), None);
}
#[test]
fn a_nonflushing_checkpoint_and_continued_observation_produce_identical_public_payloads() {
    fn run(resume: bool) -> series::tests::PublishedPayloads {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        let mut facts = facts::<u64>();
        let mut public = series::Accumulator::default();
        let mut pending: Vec<(i32, Option<i128>, Vec<u8>)> = Vec::new();
        for hour in 0..20 {
            facts
                .append(&mut writer, 1, hour, hour, Status::Value((hour / 2) as u64))
                .unwrap();
        }
        for hour in 0..20 {
            let mut reader = facts.reader(&mut writer).unwrap();
            let row = reader.at(&1, hour).unwrap().unwrap();
            let value = match row.status {
                Status::Value(v) => Some(v as i128),
                _ => None,
            };
            pending.push((hour, value, hour.to_le_bytes().to_vec()));
        }
        let checkpoint = facts.checkpoint(&mut writer, 20).unwrap();
        assert!(writer.read_at("public-values", 0).unwrap().is_none());
        if resume {
            let bytes = serde_json::to_vec(&(checkpoint, &pending)).unwrap();
            writer.commit().unwrap();
            drop(facts);
            writer = fixture.writer();
            let (checkpoint, restored_pending): (Checkpoint, Vec<(i32, Option<i128>, Vec<u8>)>) =
                serde_json::from_slice(&bytes).unwrap();
            pending = restored_pending;
            facts = Facts::restore(
                &mut writer,
                &checkpoint,
                MemoryBudget::new(64 << 20),
                limits(),
            )
            .unwrap();
        }
        for hour in 20..40 {
            facts
                .append(
                    &mut writer,
                    1,
                    hour,
                    hour,
                    if hour == 31 {
                        Status::Unavailable("refused".into())
                    } else {
                        Status::Value((hour / 2) as u64)
                    },
                )
                .unwrap();
        }
        let mut reader = facts.reader(&mut writer).unwrap();
        for hour in 20..40 {
            let row = reader.at(&1, hour).unwrap().unwrap();
            let value = match row.status {
                Status::Value(v) => Some(v as i128),
                _ => None,
            };
            pending.push((hour, value, hour.to_le_bytes().to_vec()));
        }
        for (hour, value, event) in pending {
            public.advance(hour).unwrap();
            public.observe(1, hour, value, true);
            public.event_raw(2, hour, event).unwrap();
        }
        for hour in 0..20 {
            let row = reader.at(&1, hour).unwrap().unwrap();
            public
                .event(3, 39, &(row.first, row.last, &row.status))
                .unwrap();
        }
        drop(reader);
        public
            .flush(
                &mut writer,
                "public-values",
                0,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        writer.commit().unwrap();
        fixture
            .payloads()
            .into_iter()
            .filter(|((name, _), _)| !name.starts_with("preparation-facts-v2/"))
            .collect()
    }
    fn reference() -> series::tests::PublishedPayloads {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        let mut public = series::Accumulator::default();
        for hour in 0..40_i32 {
            public.advance(hour).unwrap();
            public.observe(
                1,
                hour,
                if hour == 31 {
                    None
                } else {
                    Some((hour / 2) as i128)
                },
                true,
            );
            public
                .event_raw(2, hour, hour.to_le_bytes().to_vec())
                .unwrap();
        }
        for hour in 0..20 {
            let first = (hour / 2) * 2;
            public
                .event(3, 39, &(first, first + 1, Status::Value((hour / 2) as u64)))
                .unwrap();
        }
        public
            .flush(
                &mut writer,
                "public-values",
                0,
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        writer.commit().unwrap();
        fixture.payloads()
    }
    let expected = reference();
    assert_eq!(run(false), expected);
    assert_eq!(run(true), expected);
}
#[test]
fn published_pending_and_open_values_are_visible_in_forks_and_detached_nested_stages() {
    for detached in [false, true] {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        let mut facts = facts::<String>();
        facts
            .append(&mut writer, 1, 10, 12, Status::Value("published".into()))
            .unwrap();
        facts
            .append(&mut writer, 1, 20, 22, Status::Value("pending".into()))
            .unwrap();
        let checkpoint = facts.checkpoint(&mut writer, 1).unwrap();
        writer.commit().unwrap();
        drop(facts);
        let mut writer = fixture.writer();
        let mut stage = if detached {
            writer.stage().unwrap()
        } else {
            writer.fork().unwrap()
        };
        let mut facts = Facts::<u32, String>::restore(
            &mut stage,
            &checkpoint,
            MemoryBudget::new(64 << 20),
            limits(),
        )
        .unwrap();
        facts
            .append(&mut stage, 1, 30, 32, Status::Value("open".into()))
            .unwrap();
        {
            let mut reader = facts.reader(&mut stage).unwrap();
            for (hour, expected) in [(10, "published"), (20, "pending"), (32, "open")] {
                assert_eq!(
                    value(&mut reader, 1, hour),
                    Some(Status::Value(expected.into()))
                );
            }
            for _ in 0..1000 {
                assert_eq!(
                    value(&mut reader, 1, 11),
                    Some(Status::Value("published".into()))
                );
            }
        }
        assert!(stage.read_dependency_counts().0 <= 4);
        assert!(stage.read_dependency_counts().1 <= 16);
        let saved = facts.checkpoint(&mut stage, 2).unwrap();
        if detached {
            let mut nested = stage.fork().unwrap();
            {
                let mut reader = facts.reader(&mut nested).unwrap();
                assert_eq!(
                    value(&mut reader, 1, 11),
                    Some(Status::Value("published".into()))
                );
            }
            stage.adopt_all(nested.into_staged().unwrap()).unwrap();
            writer = writer.commit_continue().unwrap();
        }
        writer.adopt_all(stage.into_staged().unwrap()).unwrap();
        writer.commit().unwrap();
        let mut session = fixture.store.session(&fixture.key).unwrap().unwrap();
        let facts = Facts::<u32, String>::restore(
            &mut session,
            &saved,
            MemoryBudget::new(64 << 20),
            limits(),
        )
        .unwrap();
        let mut reader = facts.reader(&mut session).unwrap();
        assert_eq!(
            value(&mut reader, 1, 32),
            Some(Status::Value("open".into()))
        );
    }
}
#[test]
fn abandoned_and_conflicting_writers_leave_the_committed_checkpoint_unchanged() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<u64>();
    facts
        .append(&mut writer, 1, 0, 10, Status::Value(7))
        .unwrap();
    let checkpoint = facts.checkpoint(&mut writer, 1).unwrap();
    writer.commit().unwrap();
    drop(facts);
    for detached in [false, true] {
        let mut writer = fixture.writer();
        let mut stage = if detached {
            writer.stage().unwrap()
        } else {
            writer.fork().unwrap()
        };
        let mut facts = Facts::<u32, u64>::restore(
            &mut stage,
            &checkpoint,
            MemoryBudget::new(64 << 20),
            limits(),
        )
        .unwrap();
        facts
            .append(&mut stage, 1, 11, 12, Status::Value(9))
            .unwrap();
        facts.checkpoint(&mut stage, 2).unwrap();
        drop(stage);
        drop(facts);
        drop(writer);
        let mut session = fixture.store.session(&fixture.key).unwrap().unwrap();
        let facts = Facts::<u32, u64>::restore(
            &mut session,
            &checkpoint,
            MemoryBudget::new(64 << 20),
            limits(),
        )
        .unwrap();
        let mut reader = facts.reader(&mut session).unwrap();
        assert_eq!(value(&mut reader, 1, 10), Some(Status::Value(7)));
        assert_eq!(value(&mut reader, 1, 11), None);
    }
    let mut writer = fixture.writer();
    let mut stage = writer.stage().unwrap();
    let mut facts = Facts::<u32, u64>::restore(
        &mut stage,
        &checkpoint,
        MemoryBudget::new(64 << 20),
        limits(),
    )
    .unwrap();
    let section = format!("{}/checkpoints", facts.namespace.section());
    writer
        .write_at(&section, checkpoint.range, b"changed".as_slice())
        .unwrap();
    facts
        .append(&mut stage, 1, 11, 11, Status::Value(9))
        .unwrap();
    facts.checkpoint(&mut stage, 2).unwrap();
    assert!(writer.adopt_all(stage.into_staged().unwrap()).is_err());
}
#[test]
fn admission_failure_does_not_truncate_or_mutate_observations_and_credits_return() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<String>();
    facts
        .append(&mut writer, 1, 0, 0, Status::Value("first".into()))
        .unwrap();
    let before = facts.budget.used();
    let external = facts.budget.admit(facts.budget.limit() - before).unwrap();
    assert_eq!(
        facts
            .append(&mut writer, 1, 1, 1, Status::Value("next".into()))
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(facts.columns[&1].open.as_ref().unwrap().interval.last, 0);
    drop(external);
    facts
        .append(&mut writer, 1, 1, 1, Status::Value("next".into()))
        .unwrap();
    let budget = facts.budget.clone();
    drop(facts);
    assert_eq!(budget.used(), 0);
}
#[test]
fn family_column_and_branch_namespaces_do_not_alias_and_overlaps_are_refused() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    for (family, branch, column, number) in [
        ("a", 1, "x", 1),
        ("a", 2, "x", 2),
        ("b", 1, "x", 3),
        ("a", 1, "y", 4),
    ] {
        let mut facts = Facts::<u32, u64>::new(
            Namespace::new(family, branch, column).unwrap(),
            MemoryBudget::new(1 << 20),
            limits(),
        )
        .unwrap();
        facts
            .append(&mut writer, 1, 0, 10, Status::Value(number))
            .unwrap();
        assert!(
            facts
                .append(&mut writer, 1, 9, 11, Status::Value(20))
                .is_err()
        );
        facts.retire(&mut writer, &1).unwrap();
        let mut reader = facts.reader(&mut writer).unwrap();
        assert_eq!(value(&mut reader, 1, 5), Some(Status::Value(number)));
    }
    assert!(Namespace::new("../bad", 1, "x").is_err());
}

#[test]
fn paged_block_extents_select_a_prefix_even_when_the_requested_hour_is_missing() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut presence = facts::<()>();
    let mut extents = Facts::<u32, i32>::new(
        Namespace::new("synthetic", 7, "block-extents").unwrap(),
        presence.budget.clone(),
        limits(),
    )
    .unwrap();
    extents
        .append(&mut writer, 0, 100, 102, Status::Value(100))
        .unwrap();
    extents
        .append(&mut writer, 0, 103, 104, Status::Value(103))
        .unwrap();
    for hour in [100, 102, 103, 104] {
        presence
            .append(&mut writer, 0, hour, hour, Status::Value(()))
            .unwrap();
    }
    let blocks = extents.checkpoint(&mut writer, 2).unwrap();
    let hours = presence.checkpoint(&mut writer, 2).unwrap();
    writer.commit().unwrap();
    drop(extents);
    drop(presence);
    let mut writer = fixture.writer();
    let extents =
        Facts::<u32, i32>::restore(&mut writer, &blocks, MemoryBudget::new(1 << 20), limits())
            .unwrap();
    {
        let mut reader = extents.reader(&mut writer).unwrap();
        assert_eq!(value(&mut reader, 0, 101), Some(Status::Value(100)));
    }
    let presence =
        Facts::<u32, ()>::restore(&mut writer, &hours, MemoryBudget::new(1 << 20), limits())
            .unwrap();
    let mut reader = presence.reader(&mut writer).unwrap();
    assert_eq!(value(&mut reader, 0, 101), Some(Status::Gap));
}
#[test]
fn a_fifty_year_quiet_campaign_retains_one_open_value_per_country() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<u64>();
    for key in 0..1500 {
        facts
            .append(&mut writer, key, 0, 719, Status::Value(key as u64))
            .unwrap();
    }
    let baseline = facts.budget.used();
    for first in (720..438_000).step_by(720) {
        for key in 0..1500 {
            facts
                .append(
                    &mut writer,
                    key,
                    first,
                    (first + 719).min(437_999),
                    Status::Value(key as u64),
                )
                .unwrap();
        }
    }
    assert_eq!(facts.budget.used(), baseline);
    assert_eq!(facts.buffered_bytes(), 0);
    assert_eq!(writer.bytes_written(), 0);
    let mut reader = facts.reader(&mut writer).unwrap();
    for key in 0..1500 {
        let row = reader.at(&key, 437_999).unwrap().unwrap();
        assert_eq!((row.first, row.last), (0, 437_999));
    }
}
#[test]
fn zero_cache_and_oversized_pages_keep_spooled_dependencies_bounded() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<String>();
    facts
        .append(&mut writer, 1, 0, 20, Status::Value("x".repeat(16 << 10)))
        .unwrap();
    facts
        .append(&mut writer, 1, 30, 30, Status::Value("later".into()))
        .unwrap();
    facts.checkpoint(&mut writer, 1).unwrap();
    facts.limits.cache_bytes = 0;
    let mut stage = writer.stage().unwrap();
    {
        let mut reader = facts.reader(&mut stage).unwrap();
        for _ in 0..128 {
            assert!(
                matches!(&value(&mut reader,1,10),Some(Status::Value(text)) if text.len()==16<<10)
            );
            assert_eq!(reader.cached_bytes(), 0);
        }
    }
    assert_eq!(stage.read_dependency_counts().0, 1);
    assert!(stage.read_dependency_counts().1 >= 128);
    writer.adopt_all(stage.into_staged().unwrap()).unwrap();
}

#[test]
fn resident_accounting_includes_unused_capacity_and_fixed_decode_allocations() {
    #[derive(PartialEq)]
    struct Owned(Vec<u8>);
    impl Serialize for Owned {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            0u8.serialize(serializer)
        }
    }
    impl<'de> Deserialize<'de> for Owned {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            let _ = u8::deserialize(deserializer)?;
            Ok(Self(vec![0; 4096]))
        }
    }
    impl Resident for Owned {
        fn heap_bytes(&self) -> usize {
            self.0.capacity()
        }
        fn decode_budget() -> DecodeBudget {
            DecodeBudget {
                fixed_bytes: size_of::<Self>() + 4096,
                bytes_per_encoded_byte: 0,
            }
        }
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut owned = facts::<Vec<Owned>>();
    owned
        .append(
            &mut writer,
            1,
            0,
            0,
            Status::Value((0..100).map(|_| Owned(vec![0; 4096])).collect()),
        )
        .unwrap();
    let before = owned.budget.used();
    owned.append(&mut writer, 1, 1, 1, Status::Absent).unwrap();
    assert!(owned.budget.used() + 400_000 < before);
    let mut reader = owned.reader(&mut writer).unwrap();
    let old = reader.at(&1, 0).unwrap().unwrap();
    assert!(
        matches!(&old.status,Status::Value(rows) if rows.len()==100 && rows.iter().all(|row|row.0.len()==4096))
    );
    drop(old);
    drop(reader);
    drop(owned);
    let mut names = facts::<String>();
    let mut name = String::with_capacity(1 << 20);
    name.push_str("small");
    names
        .append(&mut writer, 2, 0, 0, Status::Value(name))
        .unwrap();
    let before = names.budget.used();
    names
        .append(&mut writer, 2, 1, 1, Status::Value("changed".into()))
        .unwrap();
    assert!(names.budget.used() + (1 << 19) < before);
    let mut reader = names.reader(&mut writer).unwrap();
    assert_eq!(
        value(&mut reader, 2, 0),
        Some(Status::Value("small".into()))
    );
}

#[test]
fn retired_key_roots_and_new_keys_are_isolated_by_each_checkpoint() {
    let fixture = Fixture::new();
    let budget = MemoryBudget::new(16 << 20);
    let mut writer = fixture.writer();
    let mut facts = Facts::<u32, String>::new(
        Namespace::new("isolated", 1, "names").unwrap(),
        budget.clone(),
        limits(),
    )
    .unwrap();
    facts
        .append(&mut writer, 7, 0, 3, Status::Value("old".into()))
        .unwrap();
    facts.retire(&mut writer, &7).unwrap();
    let a = facts.checkpoint(&mut writer, 1).unwrap();
    writer = writer.commit_continue().unwrap();
    facts
        .append(&mut writer, 7, 10, 11, Status::Value("new".into()))
        .unwrap();
    facts.retire(&mut writer, &7).unwrap();
    facts
        .append(&mut writer, 8, 10, 11, Status::Absent)
        .unwrap();
    facts.retire(&mut writer, &8).unwrap();
    let b = facts.checkpoint(&mut writer, 2).unwrap();
    writer = writer.commit_continue().unwrap();
    drop(facts);
    let old = Facts::<u32, String>::restore(&mut writer, &a, budget.clone(), limits()).unwrap();
    let latest = Facts::<u32, String>::restore(&mut writer, &b, budget.clone(), limits()).unwrap();
    let phase = ReadPhase::new(&mut writer, budget, 1 << 20).unwrap();
    let mut reader = phase.reader(&old).unwrap();
    assert_eq!(value(&mut reader, 7, 2), Some(Status::Value("old".into())));
    for hour in [4, 7, 10, 11] {
        assert_eq!(value(&mut reader, 7, hour), None);
    }
    assert_eq!(value(&mut reader, 8, 10), None);
    let mut keys = reader.keys(None, None).unwrap();
    assert_eq!(*reader.next_key(&mut keys).unwrap().unwrap(), 7);
    assert!(reader.next_key(&mut keys).unwrap().is_none());
    let mut reader = phase.reader(&latest).unwrap();
    assert_eq!(value(&mut reader, 7, 7), Some(Status::Gap));
    assert_eq!(value(&mut reader, 7, 10), Some(Status::Value("new".into())));
    assert_eq!(value(&mut reader, 8, 10), Some(Status::Absent));
}

#[test]
fn checkpoint_inside_an_equal_run_resumes_without_splitting_it() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<u64>();
    facts
        .append(&mut writer, 1, 0, 3, Status::Value(9))
        .unwrap();
    let a = facts.checkpoint(&mut writer, 1).unwrap();
    writer = writer.commit_continue().unwrap();
    let mut resumed =
        Facts::<u32, u64>::restore(&mut writer, &a, facts.budget.clone(), limits()).unwrap();
    for f in [&mut facts, &mut resumed] {
        f.append(&mut writer, 1, 4, 9, Status::Value(9)).unwrap();
        f.append(&mut writer, 1, 10, 10, Status::Absent).unwrap();
    }
    for f in [&facts, &resumed] {
        let mut r = f.reader(&mut writer).unwrap();
        let interval = r.at(&1, 5).unwrap().unwrap();
        assert_eq!(
            (interval.first, interval.last, &interval.status),
            (0, 9, &Status::Value(9))
        );
    }
    let old = Facts::<u32, u64>::restore(&mut writer, &a, facts.budget.clone(), limits()).unwrap();
    let mut r = old.reader(&mut writer).unwrap();
    assert_eq!(value(&mut r, 1, 4), None);
}

#[test]
fn historical_projects_are_discovered_in_order_across_key_pages_without_a_roster() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let budget = MemoryBudget::new(16 << 20);
    let small = Limits {
        page_bytes: 128,
        ..limits()
    };
    let mut facts = Facts::<(u32, u32), u64>::new(
        Namespace::new("projects", 1, "identities").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    for actor in (0..32).rev() {
        for project in (0..24).rev() {
            facts
                .append(
                    &mut writer,
                    (actor, project),
                    project as i32,
                    project as i32 + 1,
                    Status::Value(actor as u64 * 100 + project as u64),
                )
                .unwrap();
            facts.retire(&mut writer, &(actor, project)).unwrap();
        }
    }
    // Active-only keys must merge with archived keys without duplicate catalogue entries.
    facts
        .append(&mut writer, (11, 3), 100, 100, Status::Absent)
        .unwrap();
    facts
        .append(&mut writer, (11, 24), 100, 100, Status::Absent)
        .unwrap();
    {
        let r = facts.reader(&mut writer).unwrap();
        let mut keys = r.keys(Some(&(11, 0)), Some(&(11, u32::MAX))).unwrap();
        for project in 0..25 {
            assert_eq!(*r.next_key(&mut keys).unwrap().unwrap(), (11, project));
        }
        assert!(r.next_key(&mut keys).unwrap().is_none());
    }
    let checkpoint = facts.checkpoint(&mut writer, 1).unwrap();
    writer = writer.commit_continue().unwrap();
    drop(facts);
    let facts =
        Facts::<(u32, u32), u64>::restore(&mut writer, &checkpoint, budget.clone(), small).unwrap();
    let baseline = budget.used();
    let phase = ReadPhase::new(&mut writer, budget.clone(), 32 << 10).unwrap();
    let mut r = phase.reader(&facts).unwrap();
    let mut keys = r.keys(None, None).unwrap();
    for actor in 0..32 {
        for project in 0..if actor == 11 { 25 } else { 24 } {
            let key = r.next_key(&mut keys).unwrap().unwrap();
            assert_eq!(*key, (actor, project));
            if project < 24 {
                assert_eq!(
                    r.at(&key, project as i32).unwrap().unwrap().status,
                    Status::Value(actor as u64 * 100 + project as u64)
                );
            }
        }
    }
    assert!(r.next_key(&mut keys).unwrap().is_none());
    assert!(phase.cached_bytes() <= 32 << 10);
    assert!(budget.used() - baseline < 256 << 10);
}

#[test]
fn production_latest_summaries_ordinals_and_journals_use_shared_paged_primitives() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let budget = MemoryBudget::new(16 << 20);
    let small = Limits {
        page_bytes: 128,
        ..limits()
    };
    let mut summaries = PagedMap::<(u32, u64), (u64, String)>::new(
        Namespace::new("production", 1, "ended").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    let mut ordinals = PagedMap::<String, u64>::new(
        Namespace::new("production", 1, "appearances").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    let mut journal = Records::<(i32, String)>::new(
        Namespace::new("yards", 1, "records").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    for handle in (0..256).rev() {
        summaries
            .put(&mut writer, (3, handle), (1, format!("ended-{handle}")))
            .unwrap();
    }
    let a = summaries.checkpoint(1).unwrap();
    writer = writer.commit_continue().unwrap();
    {
        let phase = ReadPhase::new(&mut writer, budget.clone(), 64 << 10).unwrap();
        assert_eq!(summaries.get(&phase, &(3, 51)).unwrap().unwrap().0, 1);
        assert!(ordinals.get(&phase, &"line".to_string()).unwrap().is_none());
    }
    ordinals.put(&mut writer, "line".into(), 0).unwrap();
    summaries
        .put(&mut writer, (3, 51), (2, "latest".into()))
        .unwrap();
    for step in 0..300 {
        assert_eq!(
            journal
                .append(&mut writer, (10, format!("same-hour-{step}")))
                .unwrap(),
            step
        );
    }
    let b = summaries.checkpoint(2).unwrap();
    let records = journal.checkpoint(2).unwrap();
    writer = writer.commit_continue().unwrap();
    let old = PagedMap::<(u32, u64), (u64, String)>::restore(&a, budget.clone(), small).unwrap();
    let latest = PagedMap::<(u32, u64), (u64, String)>::restore(&b, budget.clone(), small).unwrap();
    let journal = Records::<(i32, String)>::restore(&records, budget.clone(), small).unwrap();
    let phase = ReadPhase::new(&mut writer, budget.clone(), 64 << 10).unwrap();
    assert_eq!(
        &*old.get(&phase, &(3, 51)).unwrap().unwrap(),
        &(1, "ended-51".into())
    );
    assert_eq!(
        &*latest.get(&phase, &(3, 51)).unwrap().unwrap(),
        &(2, "latest".into())
    );
    let mut seen = 0;
    latest
        .visit(
            &phase,
            Some(&(3, 0)),
            Some(&(3, u64::MAX)),
            |&(owner, handle), summary| {
                assert_eq!(owner, 3);
                assert_eq!(handle, seen);
                assert_eq!(summary.0, if handle == 51 { 2 } else { 1 });
                seen += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(seen, 256);
    let mut cursor = journal.cursor(&phase, 0, u64::MAX).unwrap();
    for step in 0..300 {
        let row = journal.next(&phase, &mut cursor).unwrap().unwrap();
        assert_eq!(*row.key(), step);
        assert_eq!(row.value(), &(10, format!("same-hour-{step}")));
    }
    assert!(journal.next(&phase, &mut cursor).unwrap().is_none());
    assert_eq!(
        *ordinals.get(&phase, &"line".to_string()).unwrap().unwrap(),
        0
    );
}

#[test]
fn multiple_typed_cursors_share_one_read_phase_and_keep_evicted_facts_admitted() {
    use std::cell::Cell;
    struct Counted<'a> {
        writer: &'a mut store::Writer,
        reads: &'a Cell<usize>,
    }
    impl ReadPages for Counted<'_> {
        fn open(&mut self, section: &str, range: u64) -> io::Result<Option<store::Range>> {
            self.reads.set(self.reads.get() + 1);
            self.writer.read_at_spooled(section, range)
        }
    }
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let budget = MemoryBudget::new(32 << 20);
    let small = Limits {
        page_bytes: 512,
        write_bytes: 512,
        ..limits()
    };
    let mut names = Facts::<u32, String>::new(
        Namespace::new("shared", 1, "names").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    let mut evidence = Facts::<u32, bool>::new(
        Namespace::new("shared", 1, "evidence").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    let mut presence = Facts::<u32, ()>::new(
        Namespace::new("shared", 1, "presence").unwrap(),
        budget.clone(),
        small,
    )
    .unwrap();
    for hour in 0..128 {
        names
            .append(
                &mut writer,
                1,
                hour,
                hour,
                Status::Value(format!("name-{hour}")),
            )
            .unwrap();
        evidence
            .append(&mut writer, 1, hour, hour, Status::Value(hour % 2 == 0))
            .unwrap();
        presence
            .append(
                &mut writer,
                1,
                hour,
                hour,
                if hour % 2 == 0 {
                    Status::Value(())
                } else {
                    Status::Absent
                },
            )
            .unwrap();
    }
    names.retire(&mut writer, &1).unwrap();
    evidence.retire(&mut writer, &1).unwrap();
    presence.retire(&mut writer, &1).unwrap();
    writer = writer.commit_continue().unwrap();
    for detached in [false, true] {
        let mut stage = if detached {
            writer.stage().unwrap()
        } else {
            writer.fork().unwrap()
        };
        let reads = Cell::new(0);
        {
            let mut counted = Counted {
                writer: &mut stage,
                reads: &reads,
            };
            let phase = ReadPhase::new(&mut counted, budget.clone(), 1 << 20).unwrap();
            for prefix in 0..4 {
                let before = reads.get();
                let mut nr = phase.reader(&names).unwrap();
                let mut er = phase.reader(&evidence).unwrap();
                let mut pr = phase.reader(&presence).unwrap();
                let mut nc = nr.cursor(&1, 0, 127).unwrap();
                let mut ec = er.cursor(&1, 0, 127).unwrap();
                let mut pc = pr.cursor(&1, 0, 127).unwrap();
                drop(nr);
                let mut nr = phase.reader(&names).unwrap();
                for hour in 0..128 {
                    assert_eq!(
                        nr.next(&mut nc).unwrap().unwrap().status,
                        Status::Value(format!("name-{hour}"))
                    );
                    assert_eq!(
                        er.next(&mut ec).unwrap().unwrap().status,
                        Status::Value(hour % 2 == 0)
                    );
                    assert_eq!(
                        pr.next(&mut pc).unwrap().unwrap().status,
                        if hour % 2 == 0 {
                            Status::Value(())
                        } else {
                            Status::Absent
                        }
                    );
                }
                assert!(nr.next(&mut nc).unwrap().is_none());
                assert!(er.next(&mut ec).unwrap().is_none());
                assert!(pr.next(&mut pc).unwrap().is_none());
                if prefix > 0 {
                    assert_eq!(reads.get(), before);
                }
                assert!(phase.cached_bytes() <= 1 << 20);
            }
        }
        assert_eq!(stage.read_dependency_counts().0, 1);
        assert!(stage.read_dependency_counts().1 < 512);
        writer.adopt_all(stage.into_staged().unwrap()).unwrap();
    }
    {
        let phase = ReadPhase::new(&mut writer, budget.clone(), 1024).unwrap();
        let mut r = phase.reader(&names).unwrap();
        let pinned = r.at(&1, 0).unwrap().unwrap();
        let held = budget.used();
        for hour in 1..128 {
            assert!(r.at(&1, hour).unwrap().is_some());
        }
        assert_eq!(pinned.status, Status::Value("name-0".into()));
        assert!(budget.used() >= held.min(1024));
        let before = budget.used();
        drop(pinned);
        assert!(budget.used() < before);
    }
    let other = MemoryBudget::new(32 << 20);
    let phase = ReadPhase::new(&mut writer, other, 4096).unwrap();
    assert!(phase.reader(&names).is_err());
}

#[test]
fn spill_scratch_cannot_bypass_exhausted_caller_credit() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let budget = MemoryBudget::new(8191);
    let error =
        pages::write(&mut writer, &budget, "private-scratch", b"data".as_slice()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(writer.bytes_written(), 0);
    assert_eq!(budget.used(), 0);
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<String>();
    facts
        .append(&mut writer, 1, 0, 0, Status::Value("first".into()))
        .unwrap();
    let reserved = facts
        .budget
        .admit(facts.budget.limit() - facts.budget.used())
        .unwrap();
    assert_eq!(
        facts.retire(&mut writer, &1).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(writer.bytes_written(), 0);
    drop(reserved);
    assert!(facts.checkpoint(&mut writer, 1).is_err());
}

#[test]
fn exhausted_read_admission_keeps_the_same_fact_and_map_cursor_position() {
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let mut facts = facts::<String>();
    facts
        .append(&mut writer, 1, 0, 0, Status::Value("x".repeat(4096)))
        .unwrap();
    facts.retire(&mut writer, &1).unwrap();
    let budget = facts.budget.clone();
    let mut map = PagedMap::<u64, String>::new(
        Namespace::new("retry", 1, "map").unwrap(),
        budget.clone(),
        limits(),
    )
    .unwrap();
    map.put(&mut writer, 0, "y".repeat(4096)).unwrap();
    let phase = ReadPhase::new(&mut writer, budget.clone(), 0).unwrap();
    let mut reader = phase.reader(&facts).unwrap();
    let mut facts_cursor = reader.cursor(&1, 0, 0).unwrap();
    let mut map_cursor = map.cursor(&phase, None, None).unwrap();
    let reserved = budget.admit(budget.limit() - budget.used()).unwrap();
    assert_eq!(
        reader.next(&mut facts_cursor).err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    assert_eq!(
        map.next(&phase, &mut map_cursor).err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(reserved);
    assert_eq!(reader.next(&mut facts_cursor).unwrap().unwrap().first, 0);
    assert_eq!(
        *map.next(&phase, &mut map_cursor).unwrap().unwrap().key(),
        0
    );
    assert!(reader.next(&mut facts_cursor).unwrap().is_none());
    assert!(map.next(&phase, &mut map_cursor).unwrap().is_none());
}
