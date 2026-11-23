use crate::*;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(
            std::env::temp_dir().join("hoi4-core-indexes").join(format!(
                "{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
        )
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        for name in ["indexes.0", "indexes.1"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn memory() -> Memory {
    Memory {
        allowance: 2_000_000,
        cache_bytes: 16_384,
        metadata_bytes: 65_536,
    }
}
fn column(table: TableId, field: &str) -> Column {
    Column::named(table, field).unwrap()
}
fn hours(hour: u64) -> Hours {
    Hours {
        first: hour,
        last: hour,
    }
}
fn status(table: TableId, state: TableState, hour: u64, reason: &str) -> Input<'_> {
    Input::Status(TableStatus {
        table,
        state,
        hours: hours(hour),
        reason,
    })
}

enum Value {
    Known(Vec<u8>),
    Unknown(Missing, Hours, String),
}
impl Value {
    fn from(cell: Cell<'_>) -> Self {
        match cell {
            Cell::Value(b) => Self::Known(b.to_vec()),
            Cell::Unknown(c) => Self::Unknown(c.kind, c.hours, c.reason.into()),
        }
    }
    fn cell(&self) -> Cell<'_> {
        match self {
            Self::Known(b) => Cell::Value(b),
            Self::Unknown(k, h, r) => Cell::Unknown(Cause {
                kind: *k,
                hours: *h,
                reason: r,
            }),
        }
    }
}
#[derive(Default)]
struct Reader(
    BTreeMap<TableId, BTreeMap<Vec<u8>, BTreeMap<usize, Value>>>,
    BTreeMap<TableId, Vec<Vec<u8>>>,
);
impl Reader {
    fn set(&mut self, column: Column, key: &[u8], cell: Cell<'_>) {
        self.0
            .entry(column.table)
            .or_default()
            .entry(key.to_vec())
            .or_default()
            .insert(column.field, Value::from(cell));
    }
    fn apply(&mut self, input: Input<'_>) {
        match input {
            Input::Changed {
                column, key, cell, ..
            } => self.set(column, key, cell),
            Input::Appeared { table, key, fields } => {
                self.0.entry(table).or_default().insert(
                    key.to_vec(),
                    fields.iter().map(|(f, c)| (*f, Value::from(*c))).collect(),
                );
            }
            Input::Gone { table, key, .. } => {
                self.0.entry(table).or_default().remove(key);
            }
            Input::Reread { table } => {
                self.0.entry(table).or_default().clear();
            }
            Input::Status(s)
                if matches!(
                    s.state,
                    TableState::Lost
                        | TableState::Unavailable
                        | TableState::NotRecorded
                        | TableState::Refused
                ) =>
            {
                self.0.entry(s.table).or_default().clear();
            }
            _ => {}
        }
    }
}
impl Columns for Reader {
    fn out_of_view(
        &self,
        table: TableId,
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(keys) = self.1.get(&table) {
            for key in keys {
                visit(key)?;
            }
        }
        Ok(())
    }
    fn cell<'r>(&'r self, column: Column, key: &mut dyn Key) -> io::Result<Option<Cell<'r>>> {
        let Some(rows) = self.0.get(&column.table) else {
            return Ok(None);
        };
        let row = if let Some(b) = key.as_slice() {
            rows.get(b)
        } else {
            let mut found = None;
            for (b, row) in rows {
                if key.equals(b)? {
                    found = Some(row);
                    break;
                }
            }
            found
        };
        Ok(row.and_then(|r| r.get(&column.field)).map(Value::cell))
    }
    fn keys(
        &self,
        table: TableId,
        prefix: &[u8],
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(rows) = self.0.get(&table) {
            for k in rows.keys().filter(|k| k.starts_with(prefix)) {
                visit(k)?;
            }
        }
        Ok(())
    }
}
fn make<'a>(path: &Scratch, chains: &[&'a [Link<'a>]], reader: &Reader, hour: u64) -> Indexes<'a> {
    let statuses: Vec<_> = chains
        .iter()
        .flat_map(|c| c.iter())
        .map(|l| TableStatus {
            table: l.column.table,
            state: TableState::Complete,
            hours: hours(hour),
            reason: "complete restore",
        })
        .collect();
    let mut i = Indexes::new(
        &path.0,
        memory(),
        &chains
            .iter()
            .map(|chain| RelationSpec::secondary(chain))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    i.bootstrap(hour, reader, &statuses).unwrap();
    i
}
fn feed(i: &mut Indexes<'_>, r: &mut Reader, input: Input<'_>) {
    r.apply(input);
    i.apply(r, input, |_, _| Ok(())).unwrap();
}
#[derive(Debug)]
enum NoticeCause {
    Changed,
    Fresh,
    Recovered,
    Missing,
    Hidden,
}
#[derive(Debug)]
struct Notice {
    relation: usize,
    kind: Membership,
    value: Vec<u8>,
    key: Vec<u8>,
    cause: NoticeCause,
    evidence: Option<(Missing, Hours, Vec<u8>)>,
}
fn notices(i: &mut Indexes<'_>, r: &Reader) -> Vec<Notice> {
    let mut out = Vec::new();
    i.finish_hour(r, |i, n| {
        if let Notification::Membership {
            relation,
            kind,
            value,
            key,
            cause,
            ..
        } = n
        {
            let mut v = Vec::new();
            let mut k = Vec::new();
            i.copy(value, &mut v)?;
            i.copy(key, &mut k)?;
            let evidence = match cause {
                MembershipCause::Recovered(e)
                | MembershipCause::Missing(e)
                | MembershipCause::Hidden(e) => {
                    let mut reason = Vec::new();
                    i.copy(e.reason, &mut reason)?;
                    Some((e.kind, e.hours, reason))
                }
                _ => None,
            };
            out.push(Notice {
                relation,
                kind,
                value: v,
                key: k,
                cause: match cause {
                    MembershipCause::Changed => NoticeCause::Changed,
                    MembershipCause::Fresh => NoticeCause::Fresh,
                    MembershipCause::Recovered(_) => NoticeCause::Recovered,
                    MembershipCause::Missing(_) => NoticeCause::Missing,
                    MembershipCause::Hidden(_) => NoticeCause::Hidden,
                },
                evidence,
            });
        }
        Ok(())
    })
    .unwrap();
    out
}
fn members(i: &mut Indexes<'_>, value: &[u8]) -> Vec<Vec<u8>> {
    let mut rows = Vec::new();
    assert!(
        i.secondary(0, value, |i, row| {
            let mut b = Vec::new();
            i.copy(Bytes::Stored(row), &mut b)?;
            rows.push(b);
            Ok(())
        })
        .unwrap()
        .is_none()
    );
    rows
}

#[test]
fn reader_forwards_status_before_table_events_and_memberships_at_step_eight() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"S", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 90);
    assert_eq!(members(&mut i, b"A"), [b"S".to_vec()]);
    i.begin_hour(91).unwrap();
    let mut delivered = Vec::new();
    let input = status(
        c.table,
        TableState::Truncated { missing: 1 },
        91,
        "one state outside view",
    );
    r.apply(input);
    i.apply(&r, input, |_, n| {
        assert!(matches!(n, Notification::Status { .. }));
        delivered.push("index status");
        Ok(())
    })
    .unwrap();
    delivered.push("reader status");
    let input = Input::Changed {
        column: c,
        key: b"S",
        previous: Cell::Value(b"A"),
        cell: Cell::Value(b"B"),
    };
    r.apply(input);
    i.apply(&r, input, |_, _| panic!("membership before step 8"))
        .unwrap();
    delivered.push("reader changed");
    i.finish_hour(&r, |_, n| {
        assert!(matches!(n, Notification::Membership { .. }));
        delivered.push("membership");
        Ok(())
    })
    .unwrap();
    assert_eq!(
        delivered,
        [
            "index status",
            "reader status",
            "reader changed",
            "membership",
            "membership"
        ]
    );
}

#[test]
fn composed_changes_in_either_order_publish_only_the_hour_boundary() {
    for reversed in [false, true] {
        let path = Scratch::new();
        let owner = column(TableId::States, "owner");
        let slot = column(TableId::CountryLinks, "canonical_slot");
        let chain = [owner.into(), slot.into()];
        let mut r = Reader::default();
        r.set(owner, b"S", Cell::Value(b"L1"));
        r.set(slot, b"L1", Cell::Value(b"A"));
        r.set(slot, b"L2", Cell::Value(b"X"));
        let mut i = make(&path, &[&chain], &r, 100);
        i.begin_hour(101).unwrap();
        let mut inputs = [
            Input::Changed {
                column: owner,
                key: b"S",
                previous: Cell::Value(b"L1"),
                cell: Cell::Value(b"L2"),
            },
            Input::Changed {
                column: slot,
                key: b"L2",
                previous: Cell::Value(b"X"),
                cell: Cell::Value(b"Y"),
            },
        ];
        if reversed {
            inputs.reverse();
        }
        for input in inputs {
            feed(&mut i, &mut r, input);
        }
        let mut out = Vec::new();
        i.finish_hour(&r, |i, n| {
            assert_eq!(members(i, b"Y"), [b"S".to_vec()]);
            if let Notification::Membership { kind, value, .. } = n {
                let mut v = Vec::new();
                i.copy(value, &mut v)?;
                out.push((kind, v));
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(
            out,
            [
                (Membership::Left, b"A".to_vec()),
                (Membership::Joined, b"Y".to_vec())
            ]
        );
        assert!(members(&mut i, b"X").is_empty());
    }
}

#[test]
fn gone_then_appeared_with_same_value_has_no_net_change() {
    let path = Scratch::new();
    let c = column(TableId::Ships, "task_force");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"ship", Cell::Value(b"force"));
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Gone {
            table: c.table,
            key: b"ship",
            previous: &[(c.field, Cell::Value(b"force"))],
        },
    );
    feed(
        &mut i,
        &mut r,
        Input::Appeared {
            table: c.table,
            key: b"ship",
            fields: &[(c.field, Cell::Value(b"force"))],
        },
    );
    assert!(notices(&mut i, &r).is_empty());
    assert_eq!(members(&mut i, b"force"), [b"ship".to_vec()]);
}

#[test]
fn unknown_and_recovery_carry_exact_evidence_to_composed_roots() {
    let path = Scratch::new();
    let owner = column(TableId::States, "owner");
    let slot = column(TableId::CountryLinks, "canonical_slot");
    let chain = [owner.into(), slot.into()];
    let mut r = Reader::default();
    r.set(owner, b"S", Cell::Value(b"L"));
    r.set(slot, b"L", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 700);
    i.begin_hour(701).unwrap();
    let unknown = Cell::Unknown(Cause {
        kind: Missing::NotRecorded,
        hours: Hours {
            first: 699,
            last: 701,
        },
        reason: "link slot not recorded",
    });
    feed(
        &mut i,
        &mut r,
        Input::Changed {
            column: slot,
            key: b"L",
            previous: Cell::Value(b"A"),
            cell: unknown,
        },
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Membership::Left);
    assert_eq!(out[0].key, b"S");
    assert!(matches!(out[0].cause, NoticeCause::Missing));
    assert_eq!(
        out[0].evidence,
        Some((
            Missing::NotRecorded,
            Hours {
                first: 699,
                last: 701
            },
            b"link slot not recorded".to_vec()
        ))
    );
    assert_eq!(
        i.unresolved(0, b"S").unwrap().unwrap().kind,
        Missing::NotRecorded
    );
    assert_eq!(
        i.secondary(0, b"A", |_, _| panic!("unknown never yields rows"))
            .unwrap()
            .unwrap()
            .kind,
        Missing::NotRecorded
    );
    i.begin_hour(702).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Changed {
            column: slot,
            key: b"L",
            previous: unknown,
            cell: Cell::Value(b"A"),
        },
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0].cause, NoticeCause::Recovered));
    assert_eq!(out[0].value, b"A");
    assert_eq!(out[0].evidence.as_ref().unwrap().0, Missing::NotRecorded);
}

#[test]
fn lost_target_recovers_in_the_status_hour_after_reread() {
    let path = Scratch::new();
    let owner = column(TableId::States, "owner");
    let slot = column(TableId::CountryLinks, "canonical_slot");
    let chain = [owner.into(), slot.into()];
    let mut r = Reader::default();
    r.set(owner, b"S", Cell::Value(b"L"));
    r.set(slot, b"L", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 400);
    i.begin_hour(401).unwrap();
    feed(
        &mut i,
        &mut r,
        status(slot.table, TableState::Lost, 401, "link block damaged"),
    );
    let out = notices(&mut i, &r);
    assert_eq!(
        out[0].evidence,
        Some((Missing::Lost, hours(401), b"link block damaged".to_vec()))
    );
    i.begin_hour(402).unwrap();
    feed(
        &mut i,
        &mut r,
        status(slot.table, TableState::Complete, 402, "complete again"),
    );
    feed(&mut i, &mut r, Input::Reread { table: slot.table });
    feed(
        &mut i,
        &mut r,
        Input::Appeared {
            table: slot.table,
            key: b"L",
            fields: &[(slot.field, Cell::Value(b"A"))],
        },
    );
    let out = notices(&mut i, &r);
    assert!(matches!(out[0].cause, NoticeCause::Recovered));
    assert_eq!(out[0].evidence.as_ref().unwrap().1, hours(401));
}

#[test]
fn null_owner_is_unowned_and_missing_reference_is_dated_at_restart() {
    let path = Scratch::new();
    let owner = column(TableId::States, "owner");
    let slot = column(TableId::CountryLinks, "canonical_slot");
    let chain = [
        Link {
            column: owner,
            no_values: &[b"0"],
        },
        slot.into(),
    ];
    let mut r = Reader::default();
    r.set(owner, b"unowned", Cell::Value(b"0"));
    r.set(owner, b"owned", Cell::Value(b"L"));
    r.set(slot, b"L", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 8640);
    assert_eq!(members(&mut i, b"A"), [b"owned".to_vec()]);
    assert!(i.unresolved(0, b"unowned").unwrap().is_none());
    let other = Scratch::new();
    r.set(owner, b"missing", Cell::Value(b"not-a-link"));
    let mut i = make(&other, &[&chain], &r, 8640);
    let e = i.unresolved(0, b"missing").unwrap().unwrap();
    assert_eq!(e.hours, hours(8640));
    assert_eq!(e.kind, Missing::MissingReference);
}

#[test]
fn prefix_refuses_only_its_own_unknown_or_hidden_keys() {
    let path = Scratch::new();
    let c = column(TableId::WingEquipment, "variant");
    let chain = [c.into()];
    let mut r = Reader::default();
    for key in [b"V/1", b"W/1", b"W/2"] {
        r.set(c, key, Cell::Value(b"variant"));
    }
    let mut i = Indexes::new(&path.0, memory(), &[RelationSpec::prefix(&chain)]).unwrap();
    i.bootstrap(
        0,
        &r,
        &[TableStatus {
            table: c.table,
            state: TableState::Complete,
            hours: hours(0),
            reason: "complete restore",
        }],
    )
    .unwrap();
    assert_eq!(i.shared_stages(), 0);
    assert_eq!(i.relation_stats(0).unwrap().owned_entries, 0);
    i.begin_hour(1).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Changed {
            column: c,
            key: b"V/1",
            previous: Cell::Value(b"variant"),
            cell: Cell::Unknown(Cause {
                kind: Missing::Unknown,
                hours: hours(1),
                reason: "V variant unreadable",
            }),
        },
    );
    notices(&mut i, &r);
    assert_eq!(
        i.prefix(0, &r, b"V/", |_, _, _| panic!("unknown prefix"))
            .unwrap()
            .unwrap()
            .kind,
        Missing::Unknown
    );
    let mut keys = Vec::new();
    assert!(
        i.prefix(0, &r, b"W/", |_, k, _| {
            keys.push(k.to_vec());
            Ok(())
        })
        .unwrap()
        .is_none()
    );
    assert_eq!(keys, [b"W/1".to_vec(), b"W/2".to_vec()]);
    i.begin_hour(2).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Visibility {
            table: c.table,
            key: b"W/2",
            included: false,
        },
    );
    notices(&mut i, &r);
    assert_eq!(
        i.prefix(0, &r, b"W/", |_, _, _| panic!("hidden prefix"))
            .unwrap()
            .unwrap()
            .kind,
        Missing::Hidden
    );
}

#[test]
fn complete_and_reread_both_unhide_rows_and_partial_lookups_refuse() {
    for reread in [false, true] {
        let path = Scratch::new();
        let c = column(TableId::States, "owner");
        let chain = [c.into()];
        let mut r = Reader::default();
        r.set(c, b"S", Cell::Value(b"A"));
        let mut i = make(&path, &[&chain], &r, 0);
        i.begin_hour(1).unwrap();
        feed(
            &mut i,
            &mut r,
            status(
                c.table,
                TableState::Truncated { missing: 1 },
                1,
                "S outside view",
            ),
        );
        feed(
            &mut i,
            &mut r,
            Input::Visibility {
                table: c.table,
                key: b"S",
                included: false,
            },
        );
        let out = notices(&mut i, &r);
        assert!(matches!(out[0].cause, NoticeCause::Hidden));
        assert_eq!(
            i.secondary(0, b"A", |_, _| panic!("partial secondary"))
                .unwrap()
                .unwrap()
                .kind,
            Missing::Truncated
        );
        i.begin_hour(2).unwrap();
        if reread {
            feed(
                &mut i,
                &mut r,
                Input::Appeared {
                    table: c.table,
                    key: b"S",
                    fields: &[(c.field, Cell::Value(b"A"))],
                },
            );
        }
        feed(
            &mut i,
            &mut r,
            status(c.table, TableState::Complete, 2, "complete again"),
        );
        let out = notices(&mut i, &r);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].cause, NoticeCause::Recovered));
        assert_eq!(members(&mut i, b"A"), [b"S".to_vec()]);
    }
}

#[test]
fn unchanged_status_allocates_nothing_and_invalid_adapters_are_rejected() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let r = Reader::default();
    let mut i = make(&path, &[&chain], &r, 500);
    i.begin_hour(501).unwrap();
    let before = i.storage_stats();
    i.apply(
        &r,
        status(c.table, TableState::Complete, 501, "complete restore"),
        |_, _| panic!("unchanged status"),
    )
    .unwrap();
    let after = i.storage_stats();
    assert_eq!(before.backing_bytes, after.backing_bytes);
    assert_eq!(after.tree_operations - before.tree_operations, 0);
    let path = Scratch::new();
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    assert!(
        i.apply(
            &r,
            Input::Appeared {
                table: c.table,
                key: b"S",
                fields: &[]
            },
            |_, _| Ok(())
        )
        .is_err()
    );
}

#[test]
fn shared_stage_stores_one_simple_edge_and_compaction_bounds_churn() {
    let path = Scratch::new();
    let c = column(TableId::Ships, "task_force");
    let chain = [c.into()];
    let mut r = Reader::default();
    for key in [b"a", b"b", b"c"] {
        r.set(c, key, Cell::Value(b"A"));
    }
    let mut i = make(&path, &[&chain, &chain], &r, 0);
    assert_eq!(i.shared_stages(), 1);
    assert_eq!(i.relation_stats(0).unwrap().owned_entries, 3);
    assert_eq!(i.relation_stats(1).unwrap().owned_entries, 0);
    for h in 1..400 {
        i.begin_hour(h).unwrap();
        let old = if h % 2 == 1 { b"A" } else { b"B" };
        let new = if h % 2 == 1 { b"B" } else { b"A" };
        feed(
            &mut i,
            &mut r,
            Input::Changed {
                column: c,
                key: b"a",
                previous: Cell::Value(old),
                cell: Cell::Value(new),
            },
        );
        notices(&mut i, &r);
    }
    i.compact().unwrap();
    let s = i.storage_stats();
    assert_eq!(s.compactions, 1);
    assert!(s.backing_bytes < 40_000);
    assert!(s.peak_resident_bytes <= memory().allowance);
}

#[test]
fn arbitrary_width_keys_and_values_stream_through_overflow_and_compaction() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut huge = Vec::new();
    write_wide_integer(std::iter::repeat_n(1, 300_000), &mut huge).unwrap();
    let mut r = Reader::default();
    r.set(c, &huge, Cell::Value(&huge));
    let mut i = make(&path, &[&chain], &r, 0);
    assert_eq!(members(&mut i, &huge), [huge.clone()]);
    i.begin_hour(1).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Changed {
            column: c,
            key: &huge,
            previous: Cell::Value(&huge),
            cell: Cell::Value(b"small"),
        },
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].value, b"small");
    assert_eq!(out[0].kind, Membership::Joined);
    assert_eq!(out[1].value, huge);
    assert_eq!(out[1].kind, Membership::Left);
    i.compact().unwrap();
    assert_eq!(members(&mut i, b"small"), [huge]);
    let values = [0, 1, 127, 128, 129, 16383, 16384, u64::MAX];
    let encoded: Vec<_> = values
        .iter()
        .map(|&v| {
            let mut b = Vec::new();
            write_integer(v, &mut b).unwrap();
            b
        })
        .collect();
    assert!(encoded.windows(2).all(|p| p[0] < p[1]));
    assert_eq!(encoded[1].len(), 1);
}

#[test]
fn inline_tree_splits_evictions_deletes_and_compaction_match_sorted_map() {
    use crate::store::{EMPTY, Store, Tree};
    let path = Scratch::new();
    let mut s = Store::new(
        &path.0,
        Memory {
            cache_bytes: 4200,
            ..memory()
        },
        0,
        1,
    )
    .unwrap();
    let tree = Tree(0);
    let mut expected = BTreeMap::new();
    let mut random = 91u64;
    for round in 0..10_000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let key = ((random >> 32) % 3000).to_be_bytes();
        if round % 3 != 0 {
            s.put(tree, Bytes::Borrowed(&key), EMPTY, Bytes::Borrowed(&key))
                .unwrap();
            expected.insert(key, key);
        } else {
            s.remove(tree, Bytes::Borrowed(&key), EMPTY).unwrap();
            expected.remove(&key);
        }
        if round % 900 == 0 {
            s.compact().unwrap();
        }
    }
    let mut actual = Vec::new();
    let before = s.stats();
    let mut cursor = s.cursor(tree, EMPTY, EMPTY, true).unwrap();
    while let Some(e) = s.advance(&mut cursor).unwrap() {
        let mut k = Vec::new();
        s.copy(Bytes::Stored(e.a), &mut k).unwrap();
        actual.push(k);
    }
    assert_eq!(s.stats().tree_operations - before.tree_operations, 1);
    assert!(
        s.stats().page_decodes - before.page_decodes <= s.tree_stats(tree).page_bytes / 4096 + 1
    );
    assert_eq!(
        actual,
        expected.keys().map(|k| k.to_vec()).collect::<Vec<_>>()
    );
    for key in expected.keys() {
        s.remove(tree, Bytes::Borrowed(key), EMPTY).unwrap();
    }
    s.compact().unwrap();
    assert_eq!(s.stats().backing_bytes, 0);
    assert!(s.stats().page_writes > 0);
}

#[test]
fn reread_after_not_recorded_is_recovery_and_admission_supplies_the_budget() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    let mut i = Indexes::new(
        &path.0,
        Memory {
            allowance: 1_000_000_000,
            ..memory()
        },
        &[RelationSpec::secondary(&chain)],
    )
    .unwrap();
    i.bootstrap(
        100,
        &r,
        &[TableStatus {
            table: c.table,
            state: TableState::NotRecorded,
            hours: hours(100),
            reason: "not restored at restart",
        }],
    )
    .unwrap();
    i.begin_hour(101).unwrap();
    feed(
        &mut i,
        &mut r,
        status(c.table, TableState::Complete, 101, "first complete read"),
    );
    feed(
        &mut i,
        &mut r,
        Input::Appeared {
            table: c.table,
            key: b"S",
            fields: &[(c.field, Cell::Value(b"A"))],
        },
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0].cause, NoticeCause::Recovered));
    assert_eq!(
        out[0].evidence,
        Some((
            Missing::NotRecorded,
            hours(100),
            b"not restored at restart".to_vec()
        ))
    );
}

#[test]
fn derived_two_stage_chain_runs_in_dependency_order_only_on_changed_days() {
    const ROOT: Publication = Publication(0);
    const ONE: Publication = Publication(1);
    const TWO: Publication = Publication(2);
    const INPUT_ONE: [Subscription<'static>; 1] = [Subscription {
        source: Source::Table(TableId::States),
        fields: &[0],
        filter: KeyFilter::All,
    }];
    const INPUT_TWO: [Subscription<'static>; 1] = [Subscription {
        source: Source::Publication(ONE),
        fields: &[0],
        filter: KeyFilter::All,
    }];
    struct Stage {
        output: Publication,
        inputs: &'static [Subscription<'static>],
        factor: u64,
    }
    impl DerivedStage<u64> for Stage {
        fn declaration(&self) -> Declaration<'_> {
            Declaration {
                output: self.output,
                inputs: self.inputs,
            }
        }
        fn run(&mut self, day: Day, publications: &mut dyn Publications<u64>) -> io::Result<()> {
            let mut value = 0;
            publications.read(self.inputs[0], day, &mut |record| {
                value = *record;
                Ok(())
            })?;
            publications.publish(self.output, day, &(value * self.factor))
        }
    }
    struct Logs {
        values: [u64; 3],
        changed: [bool; 3],
        runs: Vec<Publication>,
    }
    impl Publications<u64> for Logs {
        fn ready(&self, _: Source, _: Day) -> bool {
            true
        }
        fn changed(&self, input: Subscription<'_>, _: Day) -> bool {
            self.changed[match input.source {
                Source::Table(_) => 0,
                Source::Publication(p) => p.0 as usize,
            }]
        }
        fn read(
            &mut self,
            input: Subscription<'_>,
            _: Day,
            visit: &mut dyn FnMut(&u64) -> io::Result<()>,
        ) -> io::Result<()> {
            visit(
                &self.values[match input.source {
                    Source::Table(_) => 0,
                    Source::Publication(p) => p.0 as usize,
                }],
            )
        }
        fn publish(&mut self, publication: Publication, _: Day, record: &u64) -> io::Result<()> {
            let index = publication.0 as usize;
            self.changed[index] = self.values[index] != *record;
            self.values[index] = *record;
            self.runs.push(publication);
            Ok(())
        }
        fn finish(&mut self, _: Publication, _: Day) -> io::Result<()> {
            Ok(())
        }
    }
    let mut one = Stage {
        output: ONE,
        inputs: &INPUT_ONE,
        factor: 2,
    };
    let mut two = Stage {
        output: TWO,
        inputs: &INPUT_TWO,
        factor: 3,
    };
    let plan = StageOrder::new(&[two.declaration(), one.declaration()], &[ROOT], 1024).unwrap();
    assert_eq!(plan.indices(), [1, 0]);
    let mut logs = Logs {
        values: [4, 0, 0],
        changed: [true, false, false],
        runs: Vec::new(),
    };
    plan.run_day(
        Day {
            segment: 7,
            day: 0,
            first: true,
        },
        &mut [&mut two, &mut one],
        &mut logs,
    )
    .unwrap();
    assert_eq!(logs.values, [4, 8, 24]);
    assert_eq!(logs.runs, [ONE, TWO]);
    logs.changed.fill(false);
    logs.runs.clear();
    plan.run_day(
        Day {
            segment: 7,
            day: 1,
            first: false,
        },
        &mut [&mut two, &mut one],
        &mut logs,
    )
    .unwrap();
    assert!(logs.runs.is_empty());
    let cycle = [
        Declaration {
            output: ROOT,
            inputs: &INPUT_TWO,
        },
        Declaration {
            output: ONE,
            inputs: &[Subscription {
                source: Source::Publication(ROOT),
                fields: &[0],
                filter: KeyFilter::All,
            }],
        },
    ];
    assert!(StageOrder::new(&cycle, &[], 1024).is_err());
}

#[test]
fn vanished_roots_are_absent_after_the_same_hour_restoration() {
    for state in [
        TableState::Lost,
        TableState::Unavailable,
        TableState::Refused,
    ] {
        let path = Scratch::new();
        let c = column(TableId::Ships, "task_force");
        let chain = [c.into()];
        let mut r = Reader::default();
        for key in [b"sank".as_slice(), b"survived"] {
            r.set(c, key, Cell::Value(b"force"));
        }
        let mut i = make(&path, &[&chain], &r, 0);
        i.begin_hour(1).unwrap();
        feed(&mut i, &mut r, status(c.table, state, 1, "table lost"));
        assert_eq!(notices(&mut i, &r).len(), 2);
        i.begin_hour(2).unwrap();
        feed(
            &mut i,
            &mut r,
            status(c.table, TableState::Complete, 2, "complete again"),
        );
        feed(&mut i, &mut r, Input::Reread { table: c.table });
        feed(
            &mut i,
            &mut r,
            Input::Appeared {
                table: c.table,
                key: b"survived",
                fields: &[(c.field, Cell::Value(b"force"))],
            },
        );
        let out = notices(&mut i, &r);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0].cause, NoticeCause::Recovered));
        assert!(i.unresolved(0, b"sank").unwrap().is_none());
        assert_eq!(members(&mut i, b"force"), [b"survived".to_vec()]);
        i.begin_hour(3).unwrap();
        assert!(notices(&mut i, &r).is_empty());
        assert_eq!(members(&mut i, b"force"), [b"survived".to_vec()]);
    }
}

#[test]
fn vanished_target_becomes_a_missing_reference_after_restoration() {
    let path = Scratch::new();
    let owner = column(TableId::States, "owner");
    let slot = column(TableId::CountryLinks, "canonical_slot");
    let chain = [owner.into(), slot.into()];
    let mut r = Reader::default();
    r.set(owner, b"S", Cell::Value(b"L"));
    r.set(slot, b"L", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    feed(
        &mut i,
        &mut r,
        status(slot.table, TableState::Lost, 1, "lost target"),
    );
    assert_eq!(notices(&mut i, &r).len(), 1);
    i.begin_hour(2).unwrap();
    feed(
        &mut i,
        &mut r,
        status(slot.table, TableState::Complete, 2, "complete again"),
    );
    feed(&mut i, &mut r, Input::Reread { table: slot.table });
    assert!(notices(&mut i, &r).is_empty());
    let e = i.unresolved(0, b"S").unwrap().unwrap();
    assert_eq!(e.kind, Missing::MissingReference);
    assert_eq!(e.hours, hours(2));
    i.begin_hour(3).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Gone {
            table: owner.table,
            key: b"S",
            previous: &[(owner.field, Cell::Value(b"L"))],
        },
    );
    assert!(notices(&mut i, &r).is_empty());
    assert!(members(&mut i, b"A").is_empty());
}

#[test]
fn availability_counts_and_cadence_do_no_membership_work() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    for key in 0u32..4096 {
        r.set(c, &key.to_be_bytes(), Cell::Value(b"A"));
    }
    let mut i = make(&path, &[&chain], &r, 0);
    for (hour, state) in [
        (1, TableState::Truncated { missing: 1 }),
        (2, TableState::Truncated { missing: 23 }),
        (3, TableState::ReadInPart),
        (4, TableState::Complete),
    ] {
        i.begin_hour(hour).unwrap();
        let before = i.storage_stats();
        feed(
            &mut i,
            &mut r,
            status(c.table, state, hour, "complete restore"),
        );
        assert!(notices(&mut i, &r).is_empty());
        let after = i.storage_stats();
        assert_eq!(after.tree_operations, before.tree_operations);
        assert_eq!(after.reader_lookups, before.reader_lookups);
        assert_eq!(after.page_allocations, before.page_allocations);
    }
}

#[test]
fn complete_touches_only_roots_reached_through_hidden_keys() {
    let path = Scratch::new();
    let owner = column(TableId::States, "owner");
    let slot = column(TableId::CountryLinks, "canonical_slot");
    let chain = [owner.into(), slot.into()];
    let mut r = Reader::default();
    r.set(slot, b"hidden", Cell::Value(b"A"));
    r.set(slot, b"visible", Cell::Value(b"B"));
    for key in 0u32..4096 {
        r.set(
            owner,
            &key.to_be_bytes(),
            Cell::Value(if key == 0 { b"hidden" } else { b"visible" }),
        );
    }
    let mut i = make(&path, &[&chain], &r, 0);
    assert_eq!(i.shared_stages(), 1);
    i.begin_hour(1).unwrap();
    feed(
        &mut i,
        &mut r,
        status(
            slot.table,
            TableState::Truncated { missing: 1 },
            1,
            "complete restore",
        ),
    );
    feed(
        &mut i,
        &mut r,
        Input::Visibility {
            table: slot.table,
            key: b"hidden",
            included: false,
        },
    );
    assert_eq!(notices(&mut i, &r).len(), 1);
    i.begin_hour(2).unwrap();
    let before = i.storage_stats();
    feed(
        &mut i,
        &mut r,
        status(slot.table, TableState::Complete, 2, "complete restore"),
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].key, 0u32.to_be_bytes());
    assert!(i.storage_stats().reader_lookups - before.reader_lookups < 10);
}

#[test]
fn active_hours_reuse_working_pages_without_compaction() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"S", Cell::Value(b"A"));
    let mut i = make(&path, &[&chain], &r, 0);
    let mut warmed = None;
    for hour in 1..=2160 {
        let (old, new) = if hour % 2 == 1 {
            (b"A", b"B")
        } else {
            (b"B", b"A")
        };
        i.begin_hour(hour).unwrap();
        feed(
            &mut i,
            &mut r,
            Input::Changed {
                column: c,
                key: b"S",
                previous: Cell::Value(old),
                cell: Cell::Value(new),
            },
        );
        assert_eq!(notices(&mut i, &r).len(), 2);
        assert!(!i.maintain().unwrap());
        let current = i.storage_stats();
        if hour == 2 {
            warmed = Some(current);
        }
        if let Some(warmed) = warmed {
            assert_eq!(current.page_allocations, warmed.page_allocations);
            assert_eq!(current.backing_bytes, warmed.backing_bytes);
            assert_eq!(current.live_bytes, warmed.live_bytes);
        }
        assert_eq!(current.compactions, 0);
    }
    assert!(i.storage_stats().page_reuses > 2160);
}

#[test]
fn truncated_bootstrap_excludes_held_out_of_view_rows() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"visible", Cell::Value(b"A"));
    r.set(c, b"hidden", Cell::Value(b"A"));
    r.1.insert(c.table, vec![b"hidden".to_vec()]);
    let mut i = Indexes::new(&path.0, memory(), &[RelationSpec::secondary(&chain)]).unwrap();
    i.bootstrap(
        50,
        &r,
        &[TableStatus {
            table: c.table,
            state: TableState::Truncated { missing: 1 },
            hours: hours(50),
            reason: "complete restore",
        }],
    )
    .unwrap();
    assert_eq!(i.relation_stats(0).unwrap().result_edges, 1);
    assert_eq!(
        i.unresolved(0, b"hidden").unwrap().unwrap().kind,
        Missing::Hidden
    );
    i.begin_hour(51).unwrap();
    feed(
        &mut i,
        &mut r,
        status(c.table, TableState::Complete, 51, "complete restore"),
    );
    let out = notices(&mut i, &r);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].key, b"hidden");
    assert_eq!(
        members(&mut i, b"A"),
        [b"hidden".to_vec(), b"visible".to_vec()]
    );
}

#[test]
fn a_complete_reread_replaces_exactly_its_delivered_rows() {
    let path = Scratch::new();
    let c = column(TableId::Ships, "task_force");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"gone", Cell::Value(b"old"));
    r.set(c, b"stays", Cell::Value(b"old"));
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    feed(&mut i, &mut r, Input::Reread { table: c.table });
    feed(
        &mut i,
        &mut r,
        Input::Appeared {
            table: c.table,
            key: b"stays",
            fields: &[(c.field, Cell::Value(b"new"))],
        },
    );
    assert_eq!(notices(&mut i, &r).len(), 3);
    assert!(members(&mut i, b"old").is_empty());
    assert_eq!(members(&mut i, b"new"), [b"stays".to_vec()]);
}

#[test]
fn changed_before_image_needs_only_the_final_reader_lookup() {
    let path = Scratch::new();
    let c = column(TableId::Ships, "task_force");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"ship", Cell::Value(b"old"));
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    let before = i.storage_stats();
    feed(
        &mut i,
        &mut r,
        Input::Changed {
            column: c,
            key: b"ship",
            previous: Cell::Value(b"old"),
            cell: Cell::Value(b"new"),
        },
    );
    assert_eq!(notices(&mut i, &r).len(), 2);
    assert_eq!(i.storage_stats().reader_lookups - before.reader_lookups, 1);
}

#[test]
fn a_prefix_subscription_does_not_own_a_secondary_subscribers_tree() {
    let path = Scratch::new();
    let c = column(TableId::WingEquipment, "variant");
    let chain = [c.into()];
    let mut r = Reader::default();
    r.set(c, b"wing/1", Cell::Value(b"variant"));
    let mut i = Indexes::new(
        &path.0,
        memory(),
        &[
            RelationSpec::prefix(&chain),
            RelationSpec::secondary(&chain),
        ],
    )
    .unwrap();
    i.bootstrap(
        0,
        &r,
        &[TableStatus {
            table: c.table,
            state: TableState::Complete,
            hours: hours(0),
            reason: "complete restore",
        }],
    )
    .unwrap();
    assert_eq!(i.shared_stages(), 1);
    assert_eq!(i.relation_stats(0).unwrap().owned_entries, 0);
    assert_eq!(i.relation_stats(1).unwrap().owned_entries, 1);
}

#[test]
fn clearing_working_trees_reuses_overflow_extents_with_one_cache_frame() {
    use crate::store::{EMPTY, Store, Tree};
    let path = Scratch::new();
    let mut s = Store::new(
        &path.0,
        Memory {
            cache_bytes: 4200,
            ..memory()
        },
        0,
        1,
    )
    .unwrap();
    let tree = Tree(0);
    let mut key = vec![1; 4096];
    let value = vec![7; 8192];
    let mut cold = None;
    let mut warmed = None;
    for round in 0..20 {
        for row in 0..7 {
            key[0] = row;
            s.put(tree, Bytes::Borrowed(&key), EMPTY, Bytes::Borrowed(&value))
                .unwrap();
        }
        s.clear(tree).unwrap();
        assert_eq!(s.tree_stats(tree).page_bytes, 0);
        assert_eq!(s.tree_stats(tree).overflow_bytes, 0);
        assert!(s.stats().live_bytes <= 4096);
        let allocations = s.stats().page_allocations;
        if let Some(warmed) = warmed {
            assert_eq!(allocations, warmed);
        } else if let Some(cold) = cold {
            assert!(allocations <= cold + 1);
            warmed = Some(allocations);
        } else {
            assert_eq!(round, 0);
            cold = Some(allocations);
        }
    }
    assert_eq!(s.stats().compactions, 0);
}

#[test]
fn membership_notices_follow_relation_value_member_order_for_moves_and_loss() {
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    for (key, value) in [
        (b"a".as_slice(), b"Z"),
        (b"a\0", b"Z"),
        (b"b", b"A"),
        (b"c", b"M"),
        (b"d", b"Z"),
    ] {
        r.set(c, key, Cell::Value(value));
    }
    let mut i = make(&path, &[&chain, &chain], &r, 0);
    i.begin_hour(1).unwrap();
    for (key, previous, cell) in [
        (b"a".as_slice(), b"Z", b"A"),
        (b"a\0", b"Z", b"A"),
        (b"b", b"A", b"M"),
        (b"c", b"M", b"Z"),
    ] {
        feed(
            &mut i,
            &mut r,
            Input::Changed {
                column: c,
                key,
                previous: Cell::Value(previous),
                cell: Cell::Value(cell),
            },
        );
    }
    let out = notices(&mut i, &r);
    let expected = [
        (b"A".as_slice(), b"a".as_slice(), Membership::Joined),
        (b"A", b"a\0", Membership::Joined),
        (b"A", b"b", Membership::Left),
        (b"M", b"b", Membership::Joined),
        (b"M", b"c", Membership::Left),
        (b"Z", b"a", Membership::Left),
        (b"Z", b"a\0", Membership::Left),
        (b"Z", b"c", Membership::Joined),
    ];
    assert_eq!(out.len(), expected.len() * 2);
    for (relation, group) in out.chunks_exact(expected.len()).enumerate() {
        for (notice, &(value, key, kind)) in group.iter().zip(&expected) {
            assert_eq!(
                (
                    notice.relation,
                    notice.value.as_slice(),
                    notice.key.as_slice(),
                    notice.kind
                ),
                (relation, value, key, kind)
            );
        }
    }
    i.begin_hour(2).unwrap();
    feed(
        &mut i,
        &mut r,
        Input::Status(TableStatus {
            table: c.table,
            state: TableState::Lost,
            hours: Hours { first: 1, last: 2 },
            reason: "lost membership inputs",
        }),
    );
    let out = notices(&mut i, &r);
    let expected = [
        (b"A".as_slice(), b"a".as_slice()),
        (b"A", b"a\0"),
        (b"M", b"b"),
        (b"Z", b"c"),
        (b"Z", b"d"),
    ];
    assert_eq!(out.len(), expected.len() * 2);
    for (relation, group) in out.chunks_exact(expected.len()).enumerate() {
        for (notice, &(value, key)) in group.iter().zip(&expected) {
            assert_eq!(
                (
                    notice.relation,
                    notice.value.as_slice(),
                    notice.key.as_slice(),
                    notice.kind
                ),
                (relation, value, key, Membership::Left)
            );
            assert!(matches!(notice.cause, NoticeCause::Missing));
            assert_eq!(
                notice.evidence,
                Some((
                    Missing::Lost,
                    Hours { first: 1, last: 2 },
                    b"lost membership inputs".to_vec()
                ))
            );
        }
    }
}

#[test]
fn rereads_reconcile_terminal_nonmembers_without_inventing_old_memberships() {
    for present in [false, true] {
        let path = Scratch::new();
        let owner = column(TableId::States, "owner");
        let slot = column(TableId::CountryLinks, "canonical_slot");
        let chain = [
            owner.into(),
            Link {
                column: slot,
                no_values: &[b"0"],
            },
        ];
        let mut r = Reader::default();
        r.set(owner, b"S", Cell::Value(b"L"));
        r.set(slot, b"L", Cell::Value(b"0"));
        let mut i = make(&path, &[&chain], &r, 0);
        assert!(members(&mut i, b"A").is_empty());
        i.begin_hour(1).unwrap();
        feed(&mut i, &mut r, Input::Reread { table: slot.table });
        if present {
            feed(
                &mut i,
                &mut r,
                Input::Appeared {
                    table: slot.table,
                    key: b"L",
                    fields: &[(slot.field, Cell::Value(b"0"))],
                },
            );
        }
        assert!(notices(&mut i, &r).is_empty());
        if present {
            assert!(i.unresolved(0, b"S").unwrap().is_none());
            assert!(members(&mut i, b"A").is_empty());
        } else {
            let e = i.unresolved(0, b"S").unwrap().unwrap();
            assert_eq!((e.kind, e.hours), (Missing::MissingReference, hours(1)));
            assert_eq!(
                i.secondary(0, b"A", |_, _| panic!("unknown lookup delivered a member"))
                    .unwrap(),
                Some(e)
            );
            i.begin_hour(2).unwrap();
            feed(
                &mut i,
                &mut r,
                Input::Appeared {
                    table: slot.table,
                    key: b"L",
                    fields: &[(slot.field, Cell::Value(b"0"))],
                },
            );
            assert!(notices(&mut i, &r).is_empty());
            assert!(i.unresolved(0, b"S").unwrap().is_none());
        }
        i.begin_hour(3).unwrap();
        feed(
            &mut i,
            &mut r,
            Input::Changed {
                column: slot,
                key: b"L",
                previous: Cell::Value(b"0"),
                cell: Cell::Value(b"A"),
            },
        );
        let out = notices(&mut i, &r);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, Membership::Joined);
        assert!(matches!(out[0].cause, NoticeCause::Fresh));
    }
}

#[test]
fn availability_transitions_reconsider_terminal_nonmembers() {
    for state in [
        TableState::Lost,
        TableState::Unavailable,
        TableState::Refused,
        TableState::NotRecorded,
    ] {
        let path = Scratch::new();
        let owner = column(TableId::States, "owner");
        let slot = column(TableId::CountryLinks, "canonical_slot");
        let chain = [
            owner.into(),
            Link {
                column: slot,
                no_values: &[b"0"],
            },
        ];
        let mut r = Reader::default();
        r.set(owner, b"S", Cell::Value(b"L"));
        r.set(slot, b"L", Cell::Value(b"0"));
        let mut i = make(&path, &[&chain], &r, 0);
        i.begin_hour(1).unwrap();
        feed(
            &mut i,
            &mut r,
            status(slot.table, state, 1, "target unavailable"),
        );
        assert!(notices(&mut i, &r).is_empty());
        let e = i.unresolved(0, b"S").unwrap().unwrap();
        assert_eq!(e.hours, hours(1));
        assert_eq!(
            i.secondary(0, b"A", |_, _| panic!(
                "unavailable target delivered a member"
            ))
            .unwrap(),
            Some(e)
        );
        i.begin_hour(2).unwrap();
        feed(
            &mut i,
            &mut r,
            status(slot.table, TableState::Complete, 2, "target restored"),
        );
        feed(&mut i, &mut r, Input::Reread { table: slot.table });
        assert!(notices(&mut i, &r).is_empty());
        let e = i.unresolved(0, b"S").unwrap().unwrap();
        assert_eq!((e.kind, e.hours), (Missing::MissingReference, hours(2)));
    }
}

#[test]
fn scattered_single_page_reclamation_has_constant_cost_with_one_cache_frame() {
    use crate::store::{EMPTY, Store, Tree};
    let path = Scratch::new();
    let mut s = Store::new(
        &path.0,
        Memory {
            cache_bytes: 4200,
            metadata_bytes: 262_144,
            ..memory()
        },
        0,
        2048,
    )
    .unwrap();
    for tree in 0..2048 {
        s.put(
            Tree(tree),
            Bytes::Borrowed(b"key"),
            EMPTY,
            Bytes::Borrowed(b"value"),
        )
        .unwrap();
    }
    let allocated = s.stats().page_allocations;
    for tree in (0..2048).step_by(2) {
        let before = s.stats();
        s.clear(Tree(tree)).unwrap();
        let after = s.stats();
        assert_eq!(after.tree_operations, before.tree_operations);
        assert_eq!(after.page_decodes - before.page_decodes, 1);
        assert!(after.page_reads - before.page_reads <= 1);
        assert!(after.page_writes - before.page_writes <= 1);
    }
    for tree in (0..2048).step_by(2).rev() {
        s.put(
            Tree(tree),
            Bytes::Borrowed(b"key"),
            EMPTY,
            Bytes::Borrowed(b"reused"),
        )
        .unwrap();
    }
    assert_eq!(s.stats().page_allocations, allocated);
    assert_eq!(s.stats().compactions, 0);
    for tree in (1..2048).step_by(2) {
        let entry = s
            .get(Tree(tree), Bytes::Borrowed(b"key"), EMPTY)
            .unwrap()
            .unwrap();
        let mut value = Vec::new();
        s.copy(Bytes::Stored(entry.value), &mut value).unwrap();
        assert_eq!(value, b"value");
    }
}

#[test]
fn fragmented_overflow_extents_reuse_the_admitted_pager() {
    use crate::store::{EMPTY, Store, Tree};
    let path = Scratch::new();
    let mut s = Store::new(
        &path.0,
        Memory {
            cache_bytes: 4200,
            ..memory()
        },
        0,
        512,
    )
    .unwrap();
    let value = vec![7; 7 * 4096];
    let mut cold = None;
    let mut warmed = None;
    for round in 0..4 {
        for tree in (0..512).rev() {
            let len = (2 + tree % 5) * 4096;
            s.put(
                Tree(tree),
                Bytes::Borrowed(b"key"),
                EMPTY,
                Bytes::Borrowed(&value[..len]),
            )
            .unwrap();
        }
        for tree in 0..512 {
            s.clear(Tree(tree)).unwrap();
        }
        let current = s.stats();
        if let Some(warmed) = warmed {
            assert_eq!(current.page_allocations, warmed);
        } else if let Some(cold) = cold {
            assert!(current.page_allocations <= cold + 1);
            warmed = Some(current.page_allocations);
        } else {
            assert_eq!(round, 0);
            cold = Some(current.page_allocations);
        }
        assert!(current.peak_resident_bytes <= memory().allowance);
        assert!(current.metadata_bytes <= memory().metadata_bytes);
        assert_eq!(current.compactions, 0);
    }
    s.compact().unwrap();
    assert_eq!(s.stats().live_bytes, 0);
    assert_eq!(s.stats().backing_bytes, 0);
}

#[test]
fn membership_order_is_numeric_for_integer_keys_across_encoding_widths() {
    let int = |v: u64| {
        let mut b = Vec::new();
        write_integer(v, &mut b).unwrap();
        b
    };
    let path = Scratch::new();
    let c = column(TableId::States, "owner");
    let chain = [c.into()];
    let mut r = Reader::default();
    let members = [300_000u64, 16_384, 16_383, 200, 128, 127, 5];
    let old = int(1);
    for &m in &members {
        r.set(c, &int(m), Cell::Value(&old));
    }
    let mut i = make(&path, &[&chain], &r, 0);
    i.begin_hour(1).unwrap();
    let target = |m: u64| if m % 2 == 0 { int(129) } else { int(256) };
    for &m in &members {
        let (key, new) = (int(m), target(m));
        feed(
            &mut i,
            &mut r,
            Input::Changed {
                column: c,
                key: &key,
                previous: Cell::Value(&old),
                cell: Cell::Value(&new),
            },
        );
    }
    let mut sorted = members;
    sorted.sort_unstable();
    let mut expected = Vec::new();
    for &m in &sorted {
        expected.push((int(1), int(m), Membership::Left));
    }
    for value in [129, 256] {
        for &m in sorted.iter().filter(|&&m| target(m) == int(value)) {
            expected.push((int(value), int(m), Membership::Joined));
        }
    }
    let out = notices(&mut i, &r);
    let got: Vec<_> = out
        .iter()
        .map(|n| (n.value.clone(), n.key.clone(), n.kind))
        .collect();
    assert_eq!(got, expected);
}
