use crate::*;
use std::{io, sync::Arc};

fn key(bytes: &[u8]) -> Key<'_> {
    Key::new(bytes).unwrap()
}
fn interval(first: Hour, last: Hour) -> Interval {
    Interval::new(first, last).unwrap()
}
fn timer(hour: Hour, id: &[u8]) -> Timer<'_> {
    Timer {
        hour,
        id: key(id),
        key: key(id),
    }
}
fn known(value: i64) -> CellValue<'static> {
    CellValue::Known(Value::I64(value))
}
const REASON: Reason<'static> = Reason {
    name: "synthetic",
    detail: "missing evidence",
};
fn missing(hours: Interval) -> Missing<'static> {
    Missing {
        hours,
        kind: MissingKind::Unknown(REASON),
    }
}

struct TestRows {
    hour: Hour,
    cells: [CellValue<'static>; 4],
}
impl Default for TestRows {
    fn default() -> Self {
        Self {
            hour: 10,
            cells: [
                known(11),
                CellValue::Known(Value::Bytes(7)),
                known(22),
                known(23),
            ],
        }
    }
}
impl RowValues for TestRows {
    fn value(&self, _: Hour, slot: usize, field: FieldId) -> io::Result<CellValue<'_>> {
        if field >= 2 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.cells
            .get(slot * 2 + field)
            .copied()
            .ok_or_else(|| io::ErrorKind::InvalidInput.into())
    }
}
impl TestRows {
    fn row(&self, slot: usize) -> Row<'_> {
        Row {
            key: key(if slot == 0 { &[1] } else { &[2] }),
            slot,
            fields: RowView::new(self.hour, slot, &[0, 1], self),
        }
    }
}
impl Rows for TestRows {
    fn visit(&self, visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>) -> io::Result<()> {
        visitor(self.row(0))?;
        visitor(self.row(1))
    }
}

#[test]
fn every_adjacent_order_step_and_reread_causality() {
    let rows = TestRows::default();
    let table = TableId::Countries;
    let id = key(&[1]);
    let previous = known(2);
    let value = Value::I64(3);
    let unknown = missing(Interval::hour(7));
    let events = [
        Event::Discontinuity(Discontinuity::SegmentStart {
            hours: Interval::hour(7),
        }),
        Event::Status {
            table,
            status: &Status::Complete,
            sample_hour: Some(7),
        },
        Event::Reread { table },
        Event::Appeared {
            table,
            key: id,
            slot: 0,
            fields: rows.row(0).fields,
            source: Appearance::Reread,
        },
        Event::Gone {
            table,
            key: id,
            slot: 0,
        },
        Event::Appeared {
            table,
            key: id,
            slot: 0,
            fields: rows.row(0).fields,
            source: Appearance::Ordinary,
        },
        Event::Changed {
            table,
            key: id,
            field: 0,
            slot: 0,
            previous: &previous,
            value: &value,
        },
        Event::Unknown {
            table,
            key: id,
            field: 0,
            slot: 0,
            missing: &unknown,
        },
        Event::Left {
            cause: MembershipCause::Changed,
            relation: RelationId(0),
            value: id,
            member: id,
        },
        Event::Joined {
            cause: MembershipCause::Fresh,
            relation: RelationId(0),
            value: id,
            member: id,
        },
        Event::Tick {
            hour: 7,
            starts_day: false,
            due: &[],
        },
    ];
    for pair in events.windows(2) {
        assert!(pair[0].order() < pair[1].order());
    }
    let mut shuffled = [10, 3, 7, 1, 6, 9, 0, 4, 8, 5, 2];
    shuffled.sort_unstable_by_key(|&index| events[index].order());
    assert_eq!(shuffled, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    let other = TableId::Provinces;
    assert!(
        Event::Status {
            table: other,
            status: &Status::Complete,
            sample_hour: Some(7)
        }
        .order()
            < events[2].order()
    );
    assert!(Event::Reread { table: other }.order() < events[4].order());
}

#[test]
fn order_breaks_metadata_reverse_and_timer_ties_without_external_sequences() {
    let id = key(&[1]);
    let left = |cause, relation, value, member| Event::Left {
        cause,
        relation,
        value,
        member,
    };
    let a = left(
        MembershipCause::Changed,
        RelationId(9),
        key(&[9]),
        key(&[9]),
    );
    let b = left(MembershipCause::Changed, RelationId(0), id, id);
    assert!(b.order() < a.order());
    assert!(b.order() < left(MembershipCause::Changed, RelationId(1), id, id).order());
    assert!(b.order() < left(MembershipCause::Changed, RelationId(0), key(&[2]), id).order());
    assert!(b.order() < left(MembershipCause::Changed, RelationId(0), id, key(&[2])).order());
    let gap = Event::Discontinuity(Discontinuity::Gap {
        hours: interval(1, 2),
    });
    let damage = Event::Discontinuity(Discontinuity::Damage {
        hours: interval(1, 2),
        reason: REASON,
    });
    assert!(gap.order() < damage.order());
    assert!(
        gap.order()
            < Event::Discontinuity(Discontinuity::Gap {
                hours: interval(1, 3)
            })
            .order()
    );
    let first_due = [timer(1, &[1])];
    let second_due = [timer(1, &[2])];
    assert!(
        Event::Tick {
            hour: 1,
            starts_day: false,
            due: &first_due
        }
        .order()
            < Event::Tick {
                hour: 1,
                starts_day: false,
                due: &second_due
            }
            .order()
    );
}

#[test]
fn all_metadata_precedes_reread_and_changes_are_column_then_slot() {
    let table = TableId::Countries;
    let id = key(&[0]);
    let evidence = ReadEvidence {
        manifest_bits: &[1],
        timing: SpanView::default(),
    };
    let reread = Event::Reread { table };
    for metadata in [
        Event::Status {
            table,
            status: &Status::Complete,
            sample_hour: Some(4),
        },
        Event::Acquired { table, evidence },
        Event::Supplied { table, evidence },
        Event::Observed {
            observation: ObservationId(0),
        },
    ] {
        assert!(metadata.order() < reread.order());
    }
    let previous = known(0);
    let value = Value::I64(1);
    let change = |field, slot| Event::Changed {
        table,
        key: id,
        field,
        slot,
        previous: &previous,
        value: &value,
    };
    assert!(change(0, usize::MAX).order() < change(1, 0).order());
    assert!(change(1, 0).order() < change(1, 1).order());
    assert_eq!(change(1, 1).order(), change(1, 1).order());
}

#[test]
fn quiet_span_refuses_every_append_after_any_break_until_the_call_clears_it() {
    for state in [
        CoverageState::Gap,
        CoverageState::Refused(REASON),
        CoverageState::Lost(REASON),
        CoverageState::NotRecorded,
    ] {
        let mut quiet = QuietSpans::default();
        quiet
            .append(Coverage {
                hours: interval(1, 3),
                state: CoverageState::Accepted,
            })
            .unwrap();
        quiet
            .append(Coverage {
                hours: interval(4, 6),
                state: CoverageState::Accepted,
            })
            .unwrap();
        assert_eq!(
            quiet.append(Coverage {
                hours: interval(7, 9),
                state
            }),
            Err(SpanError::NotQuiet {
                hours: interval(7, 9)
            })
        );
        assert_eq!(
            quiet.append(Coverage {
                hours: interval(10, 12),
                state: CoverageState::Accepted
            }),
            Err(SpanError::NotQuiet {
                hours: interval(7, 9)
            })
        );
        assert_eq!(quiet.view().interval(), Some(interval(1, 6)));
        assert_eq!(quiet.view().hours(), 6);
        quiet.clear();
        quiet
            .append(Coverage {
                hours: interval(10, 12),
                state: CoverageState::Accepted,
            })
            .unwrap();
        assert_eq!(quiet.view().hours(), 3);
    }
}

#[test]
fn quiet_span_also_rejects_an_implicit_gap_and_out_of_order_input() {
    let mut quiet = QuietSpans::default();
    quiet
        .append(Coverage {
            hours: interval(1, 6),
            state: CoverageState::Accepted,
        })
        .unwrap();
    assert_eq!(
        quiet.append(Coverage {
            hours: Interval::hour(5),
            state: CoverageState::Accepted
        }),
        Err(SpanError::OutOfOrder)
    );
    assert_eq!(
        quiet.append(Coverage {
            hours: interval(10, 12),
            state: CoverageState::Accepted
        }),
        Err(SpanError::NotQuiet {
            hours: interval(7, 9)
        })
    );
    assert_eq!(
        quiet.append(Coverage {
            hours: Interval::hour(7),
            state: CoverageState::Accepted
        }),
        Err(SpanError::NotQuiet {
            hours: interval(7, 9)
        })
    );
}

#[test]
fn read_and_observation_runs_coalesce_intersect_and_keep_constant_time_lengths() {
    let mut storage = [Interval::hour(0); 3];
    let mut spans = Spans::new(&mut storage);
    for run in [interval(1, 3), interval(4, 6), interval(10, 12)] {
        spans.append(run).unwrap();
    }
    assert_eq!(spans.view().hours(), 9);
    assert_eq!(spans.view().runs(), &[interval(1, 6), interval(10, 12)]);
    assert_eq!(
        spans
            .view()
            .intersection(interval(5, 11))
            .collect::<Vec<_>>(),
        [interval(5, 6), interval(10, 11)]
    );
    assert_eq!(
        SpanView::new(&[interval(Hour::MIN, Hour::MAX)])
            .unwrap()
            .hours(),
        1u64 << 32
    );
    let mut storage = [Interval::hour(0)];
    let mut spans = Spans::new(&mut storage);
    spans.append(interval(1, 2)).unwrap();
    assert_eq!(
        spans.append(Interval::hour(5)),
        Err(SpanError::Capacity { needed: 2 })
    );
    assert_eq!(spans.view().hours(), 2);
    spans.append(Interval::hour(3)).unwrap();
    assert_eq!(spans.view().hours(), 3);
}

fn subscription(declarations: Vec<Declaration>) -> Subscription {
    Subscription::new(Arc::from(declarations)).unwrap()
}
fn relation(id: usize, target: TableId) -> Relation {
    Relation {
        id: RelationId(id),
        steps: Arc::from([RelationStep {
            source: FieldRef {
                table: TableId::Provinces,
                field: 0,
            },
            target,
        }]),
    }
}
fn union_storage() -> UnionStorage {
    UnionStorage::new(
        UnionCapacity {
            tables: 100,
            fields: 100,
            historical: 20,
            relations: 20,
            observations: 20,
            observation_lookups: 20,
            windows: 20,
        },
        100_000,
    )
    .unwrap()
}

#[test]
fn subscription_errors_name_invalid_tables_fields_filters_and_relation_steps() {
    assert_eq!(
        table_id(u16::MAX),
        Err(SubscriptionError::UnknownTable { table: u16::MAX })
    );
    let table = TableId::Countries;
    assert_eq!(
        Subscription::new(Arc::from([Declaration::Previous {
            table,
            fields: Arc::from([usize::MAX])
        }]))
        .err(),
        Some(SubscriptionError::UnknownField {
            table,
            field: usize::MAX
        })
    );
    let stream = Declaration::Stream {
        table,
        fields: Arc::from([0]),
        filter: Filter::Where {
            field: 1,
            predicate: Arc::new(|_| true),
        },
    };
    assert_eq!(
        Subscription::new(Arc::from([stream.clone()])).err(),
        Some(SubscriptionError::UnsubscribedWhere { table, field: 1 })
    );
    subscription(vec![
        stream.clone(),
        Declaration::Lookup {
            table,
            fields: Arc::from([1]),
            shape: LookupShape::Exact,
        },
    ]);
    assert_eq!(
        Subscription::new(Arc::from([
            stream,
            Declaration::Historical {
                index: IndexId(4),
                fields: Arc::from([ProductFieldId(1)])
            }
        ]))
        .err(),
        Some(SubscriptionError::UnsubscribedWhere { table, field: 1 })
    );
    let relation = Relation {
        id: RelationId(3),
        steps: Arc::from([
            RelationStep {
                source: FieldRef { table, field: 0 },
                target: TableId::Provinces,
            },
            RelationStep {
                source: FieldRef { table, field: 1 },
                target: table,
            },
        ]),
    };
    assert_eq!(
        relation.validate(),
        Err(SubscriptionError::DisconnectedRelation {
            relation: RelationId(3),
            step: 1,
            expected: TableId::Provinces,
            actual: table
        })
    );
}

#[test]
fn union_keeps_previous_copies_and_historical_index_fields_separate_for_two_families() {
    let countries = TableId::Countries;
    let provinces = TableId::Provinces;
    let families = [
        WrittenFamily {
            subscription: subscription(vec![
                Declaration::Stream {
                    table: countries,
                    fields: Arc::from([0, 1]),
                    filter: Filter::All,
                },
                Declaration::Reverse(relation(6, countries)),
            ]),
            calls: 0,
        },
        WrittenFamily {
            subscription: subscription(vec![
                Declaration::Watch {
                    table: countries,
                    fields: Arc::from([1, 2]),
                    filter: Filter::Keys,
                },
                Declaration::Previous {
                    table: provinces,
                    fields: Arc::from([1]),
                },
                Declaration::Lookup {
                    table: countries,
                    fields: Arc::from([1]),
                    shape: LookupShape::Previous,
                },
                Declaration::Historical {
                    index: IndexId(7),
                    fields: Arc::from([ProductFieldId(9999)]),
                },
                Declaration::Lookup {
                    table: provinces,
                    fields: Arc::from([2]),
                    shape: LookupShape::Secondary(relation(1, countries)),
                },
            ]),
            calls: 0,
        },
    ];
    let subscriptions = families.each_ref().map(Family::subscription);
    let mut storage = union_storage();
    let union = Subscription::union(&subscriptions, &mut storage).unwrap();
    assert_eq!(
        union.fields,
        [
            HeldField {
                field: FieldRef {
                    table: countries,
                    field: 0
                },
                previous: false
            },
            HeldField {
                field: FieldRef {
                    table: countries,
                    field: 1
                },
                previous: true
            },
            HeldField {
                field: FieldRef {
                    table: countries,
                    field: 2
                },
                previous: false
            },
            HeldField {
                field: FieldRef {
                    table: provinces,
                    field: 0
                },
                previous: false
            },
            HeldField {
                field: FieldRef {
                    table: provinces,
                    field: 1
                },
                previous: true
            },
            HeldField {
                field: FieldRef {
                    table: provinces,
                    field: 2
                },
                previous: false
            }
        ]
    );
    assert_eq!(
        union.historical,
        [HistoricalField {
            index: IndexId(7),
            field: ProductFieldId(9999)
        }]
    );
    assert_eq!(union.relations.len(), 1);
    assert_eq!(
        union.relation_bindings,
        [
            RelationBinding {
                family: FamilyId(0),
                local: RelationId(6),
                shared: SharedRelationId(0)
            },
            RelationBinding {
                family: FamilyId(1),
                local: RelationId(1),
                shared: SharedRelationId(0)
            }
        ]
    );
}

#[test]
fn union_namespaces_local_ids_and_deduplicates_chains_and_shared_observations() {
    let observation = Observation {
        product: ProductId("fixture"),
        id: ObservationId(1),
        wake: ObserveWake::EachHour,
        tables: Arc::from([TableId::Countries]),
        predicate: Arc::new(|_| Truth::Holds),
    };
    let mut other = observation.clone();
    other.id = ObservationId(9);
    let families = [
        subscription(vec![
            Declaration::Reverse(relation(1, TableId::Countries)),
            Declaration::Observe(observation.clone()),
        ]),
        subscription(vec![
            Declaration::Reverse(relation(1, TableId::States)),
            Declaration::Reverse(relation(9, TableId::Countries)),
            Declaration::Observe(Observation {
                predicate: Arc::new(|_| Truth::Fails),
                ..observation
            }),
            Declaration::Observe(other),
        ]),
    ];
    let mut storage = union_storage();
    let union = Subscription::union(&families, &mut storage).unwrap();
    assert_eq!(union.relations.len(), 2);
    assert_eq!(union.relation_bindings[0].shared, SharedRelationId(0));
    assert_eq!(union.relation_bindings[1].shared, SharedRelationId(1));
    assert_eq!(union.relation_bindings[2].shared, SharedRelationId(0));
    assert_eq!(union.observations.len(), 2);
    assert_eq!(
        union.observation_bindings[0],
        ObservationBinding {
            family: FamilyId(0),
            local: ObservationId(1),
            shared: SharedObservationId(0),
            wake: ObserveWake::EachHour,
        }
    );
    assert_eq!(
        union.observation_bindings[1],
        ObservationBinding {
            family: FamilyId(1),
            local: ObservationId(1),
            shared: SharedObservationId(1),
            wake: ObserveWake::EachHour,
        }
    );
    assert_eq!(union.observation_bindings[2].shared, SharedObservationId(0));
    assert_eq!(
        Subscription::new(Arc::from([
            Declaration::Reverse(relation(1, TableId::Countries)),
            Declaration::Reverse(relation(1, TableId::States))
        ]))
        .err(),
        Some(SubscriptionError::ConflictingRelation {
            relation: RelationId(1)
        })
    );
}

#[test]
fn union_reserves_before_allocation_and_names_exhausted_storage() {
    assert!(
        UnionStorage::new(
            UnionCapacity {
                fields: usize::MAX,
                ..Default::default()
            },
            usize::MAX
        )
        .is_err()
    );
    assert!(
        UnionStorage::new(
            UnionCapacity {
                fields: 1,
                ..Default::default()
            },
            0
        )
        .is_err()
    );
    let mut storage = UnionStorage::new(
        UnionCapacity {
            tables: 1,
            ..Default::default()
        },
        100,
    )
    .unwrap();
    let family = subscription(vec![Declaration::Previous {
        table: TableId::Countries,
        fields: Arc::from([0]),
    }]);
    assert_eq!(
        Subscription::union(&[family], &mut storage).err(),
        Some(SubscriptionError::Capacity {
            owner: "fields",
            needed: 1
        })
    );
}

#[test]
fn observation_metadata_uses_only_change_index_payloads() {
    let observation = Observation {
        product: ProductId("fixture"),
        id: ObservationId(0),
        wake: ObserveWake::EachHour,
        tables: Arc::from([TableId::Countries]),
        predicate: Arc::new(|metadata| {
            if metadata.tables.iter().any(|table| {
                table.written_fields.iter().any(|&word| word != 0) && table.sample_hour == Some(3)
            }) {
                Truth::Holds
            } else {
                Truth::Fails
            }
        }),
    };
    let tables = [TableMetadata {
        table: TableId::Countries,
        written_fields: &[3],
        inserted: Count::new(&[2]).unwrap(),
        deleted: Count::new(&[0]).unwrap(),
        status: IndexedStatus::Complete,
        sample_hour: Some(3),
        acquired: None,
        supplied: None,
    }];
    assert_eq!(
        (observation.predicate)(&Metadata {
            hour: 4,
            tables: &tables,
            unavailable: None,
        }),
        Truth::Holds
    );
}

#[derive(Default)]
struct OwningTimers {
    timers: Vec<(Hour, Vec<u8>, Vec<u8>)>,
    index: std::collections::BTreeMap<Vec<u8>, usize>,
    len: usize,
}
impl TimerStore for OwningTimers {
    fn len(&self) -> usize {
        self.len
    }
    fn reserve_len(&mut self, len: usize) -> io::Result<()> {
        self.timers
            .try_reserve(len.saturating_sub(self.timers.len()))
            .map_err(|_| io::ErrorKind::OutOfMemory.into())
    }
    fn find(&mut self, id: Key<'_>) -> io::Result<Option<usize>> {
        Ok(self.index.get(id.varint()).copied())
    }
    fn set_hour(&mut self, index: usize, hour: Hour) -> io::Result<()> {
        self.timers[index].0 = hour;
        Ok(())
    }
    fn read(&mut self, index: usize) -> io::Result<Timer<'_>> {
        let (hour, bytes, owner) = &self.timers[index];
        Ok(Timer {
            hour: *hour,
            id: key(bytes),
            key: key(owner),
        })
    }
    fn compare(&mut self, left: usize, right: usize) -> io::Result<std::cmp::Ordering> {
        let a = &self.timers[left];
        let b = &self.timers[right];
        Ok((a.0, key(&a.1), key(&a.2)).cmp(&(b.0, key(&b.1), key(&b.2))))
    }
    fn swap(&mut self, left: usize, right: usize) -> io::Result<()> {
        self.timers.swap(left, right);
        *self.index.get_mut(self.timers[left].1.as_slice()).unwrap() = left;
        *self.index.get_mut(self.timers[right].1.as_slice()).unwrap() = right;
        Ok(())
    }
    fn truncate(&mut self, len: usize) -> io::Result<()> {
        for (_, id, _) in &self.timers[len..self.len] {
            self.index.remove(id.as_slice());
        }
        self.len = len;
        Ok(())
    }
    fn pop_last(&mut self) -> io::Result<Timer<'_>> {
        self.len -= 1;
        self.index.remove(self.timers[self.len].1.as_slice());
        self.read(self.len)
    }
}
impl TimerSink<'_> for OwningTimers {
    fn push(&mut self, value: Timer<'_>) -> io::Result<()> {
        let value = (
            value.hour,
            value.id.varint().to_vec(),
            value.key.varint().to_vec(),
        );
        self.index.insert(value.1.clone(), self.len);
        if self.len < self.timers.len() {
            self.timers[self.len] = value;
        } else {
            self.timers.push(value);
        }
        self.len += 1;
        Ok(())
    }
}

struct FixtureContext<'a> {
    rows: TestRows,
    reached: [bool; 2],
    watch: bool,
    unavailable: Option<Missing<'static>>,
    timers: TimerQueue<OwningTimers>,
    closed_windows: Vec<(DeclarationId, Vec<u8>, Hour)>,
    observation_stream: Option<&'a dyn ObservationStream>,
}
impl Default for FixtureContext<'_> {
    fn default() -> Self {
        Self {
            rows: TestRows::default(),
            reached: [false; 2],
            watch: true,
            unavailable: None,
            timers: TimerQueue::new(OwningTimers::default()).unwrap(),
            closed_windows: Vec::new(),
            observation_stream: None,
        }
    }
}
fn slot(id: Key<'_>) -> Option<usize> {
    match id.varint() {
        [1] => Some(0),
        [2] => Some(1),
        _ => None,
    }
}
impl CallContext for FixtureContext<'_> {
    fn observation(&self, declaration: DeclarationId) -> io::Result<&dyn ObservationStream> {
        if declaration != DeclarationId(0) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.observation_stream
            .ok_or_else(|| io::ErrorKind::Unsupported.into())
    }
    fn close_window(&mut self, rule: DeclarationId, key: Key<'_>) -> io::Result<()> {
        if self.closed_at(rule, key)?.is_none() {
            self.closed_windows
                .push((rule, key.varint().to_vec(), self.hour()));
        }
        Ok(())
    }
    fn hour(&self) -> Hour {
        self.rows.hour
    }
    fn exact(&self, _: TableId, id: Key<'_>, field: FieldId) -> io::Result<CellValue<'_>> {
        if let Some(absent) = self.unavailable {
            return Ok(CellValue::Missing(absent));
        }
        match slot(id) {
            Some(slot) => self.rows.value(self.hour(), slot, field),
            None => Ok(CellValue::Missing(missing(Interval::hour(10)))),
        }
    }
    fn previous(&self, _: TableId, _: Key<'_>, _: FieldId) -> io::Result<CellValue<'_>> {
        Ok(self.unavailable.map_or(known(9), CellValue::Missing))
    }
    fn historical(
        &self,
        _: IndexId,
        _: Key<'_>,
        _: ProductFieldId,
        _: Hour,
    ) -> io::Result<ProductCell<'_>> {
        Ok(self.unavailable.map_or(
            ProductCell::Known(ProductValue::Text("earlier name")),
            ProductCell::Missing,
        ))
    }
    fn prefix(
        &self,
        _: DeclarationId,
        _: &[Key<'_>],
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        if let Some(absent) = self.unavailable {
            return Ok(Lookup::Missing(absent));
        }
        self.rows.visit(visitor)?;
        Ok(Lookup::Known(()))
    }
    fn secondary(
        &self,
        declaration: DeclarationId,
        id: Key<'_>,
        visitor: &mut dyn FnMut(Row<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        self.prefix(declaration, &[id], visitor)
    }
    fn members(
        &self,
        _: RelationId,
        _: Key<'_>,
        visitor: &mut dyn FnMut(Key<'_>) -> io::Result<()>,
    ) -> io::Result<Lookup<'_, ()>> {
        if let Some(absent) = self.unavailable {
            return Ok(Lookup::Missing(absent));
        }
        visitor(key(&[1]))?;
        visitor(key(&[2]))?;
        Ok(Lookup::Known(()))
    }
    fn snapshot(&self, _: TableId) -> io::Result<Lookup<'_, &dyn Rows>> {
        Ok(match self.unavailable {
            Some(absent) => Lookup::Missing(absent),
            None => Lookup::Known(&self.rows),
        })
    }
    fn take_watch(&mut self, _: DeclarationId) -> io::Result<bool> {
        Ok(std::mem::take(&mut self.watch))
    }
    fn schedule(&mut self, timer: Timer<'_>) -> io::Result<()> {
        self.timers.schedule(timer)
    }
    fn cancel(&mut self, id: Key<'_>) -> io::Result<bool> {
        self.timers.cancel(id)
    }
    fn insert_filter(&mut self, _: DeclarationId, id: Key<'_>) -> io::Result<FilterInsertion<'_>> {
        let Some(slot) = slot(id) else {
            return Ok(FilterInsertion::Missing(missing(Interval::hour(10))));
        };
        if std::mem::replace(&mut self.reached[slot], true) {
            return Ok(FilterInsertion::Unchanged);
        }
        Ok(match self.unavailable {
            Some(absent) => FilterInsertion::Missing(absent),
            None => FilterInsertion::Appeared(self.rows.row(slot)),
        })
    }
    fn remove_filter(&mut self, _: DeclarationId, id: Key<'_>) -> io::Result<bool> {
        Ok(slot(id).is_some_and(|slot| std::mem::take(&mut self.reached[slot])))
    }
    fn dictionary(&self, id: u32) -> io::Result<Lookup<'_, &str>> {
        Ok(if id == 7 {
            Lookup::Known("synthetic string")
        } else {
            Lookup::Missing(missing(Interval::hour(10)))
        })
    }
    fn dictionary_additions(&self) -> &[DictionaryString<'_>] {
        &[DictionaryString {
            id: 7,
            value: "synthetic string",
        }]
    }
}

struct NullPublication;
impl Publication for NullPublication {
    fn publish(&mut self, _: PublicationRecord<'_>) -> io::Result<()> {
        Ok(())
    }
    fn coverage(&mut self, _: Coverage<'_>) -> io::Result<()> {
        Ok(())
    }
    fn provisional(&mut self, _: ProvisionalWindow<'_>) -> io::Result<()> {
        Ok(())
    }
}
struct WrittenFamily {
    subscription: Subscription,
    calls: usize,
}
impl Family for WrittenFamily {
    fn subscription(&self) -> Subscription {
        self.subscription.clone()
    }
    fn begin(
        &mut self,
        _: &Start<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        Ok(())
    }
    fn hour(
        &mut self,
        _: &At<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        self.calls += 1;
        Ok(())
    }
    fn end(
        &mut self,
        _: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        Ok(Boundary::ClassA)
    }
}
fn at<'a>(hour: Hour, events: &'a [Event<'a>]) -> At<'a> {
    At {
        hour,
        since: QuietView::default(),
        reads: &[],
        observations: &[],
        starts: false,
        opens: false,
        events,
    }
}

#[test]
fn call_services_hydrate_exact_rows_read_in_place_clear_watches_and_resolve_strings() {
    let mut context = FixtureContext::default();
    let table = TableId::Countries;
    let declaration = DeclarationId(0);
    let FilterInsertion::Appeared(row) = context.insert_filter(declaration, key(&[1])).unwrap()
    else {
        panic!("missing hydration");
    };
    assert_eq!(row.fields.value(0).unwrap(), known(11));
    assert_eq!(
        row.fields.value(1).unwrap(),
        CellValue::Known(Value::Bytes(7))
    );
    assert_eq!(context.exact(table, key(&[1]), 0).unwrap(), known(11));
    assert!(matches!(
        context.insert_filter(declaration, key(&[1])).unwrap(),
        FilterInsertion::Unchanged
    ));
    assert_eq!(context.previous(table, key(&[1]), 0).unwrap(), known(9));
    assert_eq!(
        context
            .historical(IndexId(1), key(&[1]), ProductFieldId(1), 2)
            .unwrap(),
        ProductCell::Known(ProductValue::Text("earlier name"))
    );
    assert!(context.take_watch(declaration).unwrap());
    assert!(!context.take_watch(declaration).unwrap());
    assert_eq!(
        context.dictionary(7).unwrap(),
        Lookup::Known("synthetic string")
    );
    assert_eq!(context.dictionary_additions()[0].id, 7);
    let mut sum = 0;
    let mut visit = |row: Row<'_>| {
        if let CellValue::Known(Value::I64(value)) = row.fields.value(0)? {
            sum += value;
        }
        Ok(())
    };
    assert!(matches!(
        context
            .prefix(declaration, &[key(&[1])], &mut visit)
            .unwrap(),
        Lookup::Known(())
    ));
    assert!(matches!(
        context
            .secondary(declaration, key(&[1]), &mut visit)
            .unwrap(),
        Lookup::Known(())
    ));
    let Lookup::Known(rows) = context.snapshot(table).unwrap() else {
        panic!("missing snapshot");
    };
    rows.visit(&mut visit).unwrap();
    assert_eq!(sum, 99);
    let mut members = 0;
    context
        .members(RelationId(0), key(&[1]), &mut |_| {
            members += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(members, 2);
    assert!(context.remove_filter(declaration, key(&[1])).unwrap());
}

#[test]
fn every_lookup_shape_preserves_incomplete_reason_and_hours() {
    let absent = Missing {
        hours: interval(5, 10),
        kind: MissingKind::Refused(REASON),
    };
    let mut context = FixtureContext {
        unavailable: Some(absent),
        ..Default::default()
    };
    let table = TableId::Countries;
    assert_eq!(
        context.exact(table, key(&[1]), 0).unwrap(),
        CellValue::Missing(absent)
    );
    assert_eq!(
        context.previous(table, key(&[1]), 0).unwrap(),
        CellValue::Missing(absent)
    );
    assert_eq!(
        context
            .historical(IndexId(0), key(&[1]), ProductFieldId(0), 5)
            .unwrap(),
        ProductCell::Missing(absent)
    );
    let mut forbidden = |_: Row<'_>| -> io::Result<()> { panic!("partial lookup exposed rows") };
    assert_eq!(
        context
            .prefix(DeclarationId(0), &[], &mut forbidden)
            .unwrap(),
        Lookup::Missing(absent)
    );
    assert_eq!(
        context
            .secondary(DeclarationId(0), key(&[1]), &mut forbidden)
            .unwrap(),
        Lookup::Missing(absent)
    );
    assert!(matches!(context.snapshot(table).unwrap(), Lookup::Missing(value) if value == absent));
    assert!(
        matches!(context.insert_filter(DeclarationId(0), key(&[1])).unwrap(), FilterInsertion::Missing(value) if value == absent)
    );
}

#[test]
fn scheduled_timer_calls_a_family_even_when_no_stream_event_occurs() {
    let mut context = FixtureContext::default();
    let mut family = WrittenFamily {
        subscription: subscription(vec![Declaration::Timer]),
        calls: 0,
    };
    let retained = family.subscription();
    context.schedule(timer(20, &[1])).unwrap();
    assert!(!context.timers.is_due(19).unwrap());
    assert!(context.timers.is_due(20).unwrap());
    let due = [context.timers.pop_due(20).unwrap().unwrap()];
    let events = [Event::Tick {
        hour: 20,
        starts_day: false,
        due: &due,
    }];
    family
        .hour(
            &at(20, &events),
            &mut FixtureContext::default(),
            &mut NullPublication,
        )
        .unwrap();
    assert_eq!(family.calls, 1);
    assert!(matches!(retained.declarations(), [Declaration::Timer]));
}

#[test]
fn end_preserves_quiet_read_observation_spans_endings_and_delivered_watermark() {
    let mut quiet = QuietSpans::default();
    quiet
        .append(Coverage {
            hours: interval(3, 8),
            state: CoverageState::Accepted,
        })
        .unwrap();
    let read_runs = [Interval::hour(4), Interval::hour(7)];
    let observed_runs = [interval(5, 8)];
    let reads = [ReadSpans {
        table: TableId::Countries,
        rule: ReadRule::Supplied,
        spans: SpanView::new(&read_runs).unwrap(),
    }];
    let observations = [ObservationSpans::new(
        ObservationId(0),
        interval(1, 10),
        SpanView::new(&observed_runs).unwrap(),
        &[],
    )
    .unwrap()];
    let at = At {
        hour: 9,
        since: quiet.view(),
        reads: &reads,
        observations: &observations,
        starts: false,
        opens: true,
        events: &[],
    };
    for ending in [
        Ending::Whole {
            hours: interval(3, 9),
        },
        Ending::Refused {
            hours: interval(9, 10),
            reason: REASON,
        },
        Ending::Lost {
            hours: interval(9, 10),
            reason: REASON,
        },
    ] {
        let end = End {
            at,
            ending,
            delivered_watermark: Some(8),
        };
        assert_eq!(end.at.since.hours(), 6);
        assert_eq!(end.at.reads[0].spans.hours(), 2);
        assert_eq!(end.at.observations[0].holds_hours(), 4);
        assert_eq!(end.ending, ending);
        assert_eq!(end.delivered_watermark, Some(8));
    }
}

#[test]
fn reread_appeared_rows_borrow_cells_instead_of_copying_a_table() {
    let bytes = [4, 5, 6];
    struct Bytes<'a>(&'a [u8]);
    impl RowValues for Bytes<'_> {
        fn value(&self, _: Hour, _: usize, _: FieldId) -> io::Result<CellValue<'_>> {
            Ok(CellValue::Known(Value::Opaque(self.0)))
        }
    }
    let cells = Bytes(&bytes);
    let event = Event::Appeared {
        table: TableId::Provinces,
        key: key(&[1]),
        slot: 0,
        fields: RowView::new(10, 0, &[0], &cells),
        source: Appearance::Reread,
    };
    let Event::Appeared { fields, .. } = event else {
        unreachable!()
    };
    assert!(
        matches!(fields.value(0).unwrap(), CellValue::Known(Value::Opaque(value)) if std::ptr::eq(value.as_ptr(), bytes.as_ptr()))
    );
}

#[test]
fn joins_hold_multiple_answers_and_lookup_inside_snapshot_prefix_and_secondary_visits() {
    let mut fixture = FixtureContext::default();
    let context: &mut dyn CallContext = &mut fixture;
    let table = TableId::Countries;
    let first = context.exact(table, key(&[1]), 0).unwrap();
    let second = context.exact(table, key(&[2]), 0).unwrap();
    let history = context
        .historical(IndexId(0), key(&[1]), ProductFieldId(0), 9)
        .unwrap();
    let Lookup::Known(rows) = context.snapshot(table).unwrap() else {
        panic!("missing snapshot")
    };
    let mut joined = 0;
    rows.visit(&mut |row| {
        assert_eq!(row.fields.hour(), context.hour());
        assert_eq!(row.fields.value(0)?, context.exact(table, row.key, 0)?);
        joined += 1;
        Ok(())
    })
    .unwrap();
    for secondary in [false, true] {
        let mut visitor = |row: Row<'_>| {
            let value = context.exact(table, row.key, 0)?;
            context.members(RelationId(0), row.key, &mut |member| {
                assert!(matches!(
                    context.exact(table, member, 0)?,
                    CellValue::Known(_)
                ));
                Ok(())
            })?;
            assert_eq!(value, row.fields.value(0)?);
            joined += 1;
            Ok(())
        };
        if secondary {
            context
                .secondary(DeclarationId(0), key(&[1]), &mut visitor)
                .unwrap();
        } else {
            context
                .prefix(DeclarationId(0), &[key(&[1])], &mut visitor)
                .unwrap();
        }
    }
    assert_eq!((first, second), (known(11), known(22)));
    assert_eq!(
        history,
        ProductCell::Known(ProductValue::Text("earlier name"))
    );
    assert_eq!(joined, 6);
    context.schedule(timer(11, &[1])).unwrap();
    assert!(context.cancel(key(&[1])).unwrap());
    assert!(context.take_watch(DeclarationId(0)).unwrap());
}

#[test]
fn row_handles_request_the_call_hour_when_the_decoder_has_advanced() {
    struct Versions {
        before: [u8; 3],
        ahead: [u8; 3],
    }
    impl RowValues for Versions {
        fn value(&self, hour: Hour, _: usize, _: FieldId) -> io::Result<CellValue<'_>> {
            match hour {
                10 => Ok(CellValue::Known(Value::Opaque(&self.before))),
                12 => Ok(CellValue::Known(Value::Opaque(&self.ahead))),
                _ => Err(io::ErrorKind::InvalidInput.into()),
            }
        }
    }
    let versions = Versions {
        before: [1, 2, 3],
        ahead: [4, 5, 6],
    };
    let row = RowView::new(10, 0, &[0], &versions);
    let ahead = RowView::new(12, 0, &[0], &versions);
    assert_eq!(
        ahead.value(0).unwrap(),
        CellValue::Known(Value::Opaque(&versions.ahead))
    );
    assert_eq!(
        row.value(0).unwrap(),
        CellValue::Known(Value::Opaque(&versions.before))
    );
}

#[test]
fn timers_restore_from_begin_inputs_and_order_due_timers_across_a_boundary() {
    let mut storage = [timer(0, &[0]); 6];
    let mut index = vec![usize::MAX; storage.len() * 2];
    let mut buckets = vec![0; storage.len()];
    let mut queue =
        TimerQueue::new(SliceTimers::new(&mut storage, &mut index, &mut buckets).unwrap()).unwrap();
    queue
        .rebuild(&[timer(30, &[1]), timer(10, &[2]), timer(20, &[3])])
        .unwrap();
    assert_eq!(queue.pop_due(9).unwrap(), None);
    assert_eq!(queue.pop_due(10).unwrap(), Some(timer(10, &[2])));
    assert_eq!(queue.pop_due(19).unwrap(), None);
    queue.rebuild(&[timer(30, &[1]), timer(20, &[3])]).unwrap();
    queue.schedule(timer(20, &[0])).unwrap();
    assert_eq!(queue.pop_due(20).unwrap(), Some(timer(20, &[0])));
    assert_eq!(queue.pop_due(20).unwrap(), Some(timer(20, &[3])));
    assert_eq!(queue.pop_due(29).unwrap(), None);
    assert_eq!(queue.pop_due(30).unwrap(), Some(timer(30, &[1])));
    assert!(queue.is_empty());
}

#[test]
fn timer_replacement_and_cancellation_keep_only_live_ids_in_the_indexed_heap() {
    let mut storage = [timer(0, &[0]); 4];
    let mut index = [usize::MAX; 8];
    let mut buckets = vec![0; storage.len()];
    let mut queue =
        TimerQueue::new(SliceTimers::new(&mut storage, &mut index, &mut buckets).unwrap()).unwrap();
    queue
        .rebuild(&[
            timer(10, &[1]),
            timer(20, &[2]),
            timer(30, &[3]),
            timer(40, &[4]),
        ])
        .unwrap();
    queue.schedule(timer(50, &[1])).unwrap();
    queue.schedule(timer(5, &[3])).unwrap();
    assert_eq!(queue.len(), 4);
    assert!(queue.cancel(key(&[2])).unwrap());
    assert!(!queue.cancel(key(&[2])).unwrap());
    assert_eq!(queue.pop_due(5).unwrap(), Some(timer(5, &[3])));
    assert_eq!(queue.pop_due(39).unwrap(), None);
    queue.schedule(timer(6, &[4])).unwrap();
    assert_eq!(queue.pop_due(6).unwrap(), Some(timer(6, &[4])));
    assert_eq!(queue.pop_due(50).unwrap(), Some(timer(50, &[1])));
    assert!(queue.is_empty());
    queue.rebuild(&[timer(7, &[1]), timer(8, &[1])]).unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.pop_due(8).unwrap(), Some(timer(8, &[1])));
    queue.schedule(timer(9, &[1])).unwrap();
    assert!(queue.cancel(key(&[1])).unwrap());
    assert!(queue.is_empty());
}

#[test]
fn timer_capacity_refusal_preserves_pending_timers_and_paged_stores_own_keys() {
    let first = timer(10, &[0]);
    let later = timer(20, &[1]);
    let mut storage = [first];
    let mut index = vec![usize::MAX; storage.len() * 2];
    let mut buckets = vec![0; storage.len()];
    let mut queue =
        TimerQueue::new(SliceTimers::new(&mut storage, &mut index, &mut buckets).unwrap()).unwrap();
    queue.schedule(first).unwrap();
    assert_eq!(
        queue.schedule(later).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(
        queue.rebuild(&[first, later]).unwrap_err().kind(),
        io::ErrorKind::OutOfMemory
    );
    assert_eq!(queue.pop_due(10).unwrap(), Some(first));
    let mut queue = TimerQueue::new(OwningTimers::default()).unwrap();
    {
        let first = vec![0x80, 1];
        let second = vec![0xff, 1];
        queue.schedule(timer(20, &second)).unwrap();
        queue.schedule(timer(10, &first)).unwrap();
    }
    assert_eq!(queue.pop_due(10).unwrap(), Some(timer(10, &[0x80, 1])));
    assert_eq!(queue.pop_due(20).unwrap(), Some(timer(20, &[0xff, 1])));
}

#[test]
fn arbitrary_width_keys_and_counts_are_canonical_and_numerically_ordered() {
    let mut arbitrary = vec![0x80; 128];
    arbitrary.push(1);
    assert!(key(&arbitrary) > key(&[0xff, 0xff, 1]));
    assert!(Count::new(&arbitrary).unwrap() > Count::new(&[0xff, 0xff, 1]).unwrap());
    assert!(key(&[0xff, 1]) < key(&[0x80, 2]));
    for invalid in [&[][..], &[0x80], &[0x80, 0], &[1, 1]] {
        assert_eq!(Key::new(invalid), Err(InvalidKey));
        assert_eq!(Count::new(invalid), Err(InvalidKey));
    }
}

struct TestSummary(u64);
impl TestSummary {
    fn read_value(summary: &dyn BoundarySummary) -> io::Result<u64> {
        match summary.resolve(Inherited {
            key: key(&[1]),
            field: ProductFieldId(3),
        })? {
            ProductCell::Known(ProductValue::Number(Number::Unsigned(value))) => Ok(value),
            _ => Err(io::ErrorKind::InvalidData.into()),
        }
    }
}
impl BoundarySummary for TestSummary {
    fn value(&self, id: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>> {
        if id != key(&[1]) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        match field.0 {
            3 => Ok(ProductCell::Known(ProductValue::Number(Number::Unsigned(
                self.0,
            )))),
            4 => Ok(ProductCell::Missing(missing(interval(1, 9)))),
            _ => Err(io::ErrorKind::InvalidInput.into()),
        }
    }
    fn bytes(&self) -> u64 {
        size_of::<Self>() as u64
    }
}
fn window_keys() -> WindowKeys {
    WindowKeys::Keys(Arc::from([Arc::<[u8]>::from([1])]))
}
fn window_subscription() -> Subscription {
    subscription(vec![
        Declaration::Lookup {
            table: TableId::Countries,
            fields: Arc::from([0]),
            shape: LookupShape::Exact,
        },
        Declaration::Timer,
        Declaration::Provisional(ProvisionalRule {
            bound: WindowBound::PrefixHours(3),
            inputs: Arc::from([
                WindowInput {
                    declaration: DeclarationId(0),
                    fields: Arc::from([WindowField::Table(0)]),
                    keys: window_keys(),
                },
                WindowInput {
                    declaration: DeclarationId(1),
                    fields: Arc::from([]),
                    keys: window_keys(),
                },
            ]),
            outputs: Arc::from([
                WindowOutput {
                    keys: window_keys(),
                    fields: Arc::from([ProductFieldId(3)]),
                    target: DependencyTarget::Publication,
                },
                WindowOutput {
                    keys: window_keys(),
                    fields: Arc::from([ProductFieldId(3)]),
                    target: DependencyTarget::OutgoingSummary,
                },
            ]),
        }),
    ])
}
struct Dependencies;
impl WindowDependencies for Dependencies {
    fn hours(
        &self,
        key: Key<'_>,
        field: ProductFieldId,
        _: DependencyTarget,
    ) -> io::Result<Option<Interval>> {
        Ok((key.varint() == [1] && field == ProductFieldId(3)).then_some(interval(10, 12)))
    }
    fn visit(
        &self,
        visitor: &mut dyn FnMut(WindowDependency<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        for target in [
            DependencyTarget::Publication,
            DependencyTarget::OutgoingSummary,
        ] {
            visitor(WindowDependency {
                hours: interval(10, 12),
                key: key(&[1]),
                field: ProductFieldId(3),
                target,
            })?;
        }
        Ok(())
    }
}
#[derive(Default)]
struct CheckedPublication {
    records: usize,
    windows: usize,
    corrected: Option<Interval>,
    last: Option<u64>,
}
impl Publication for CheckedPublication {
    fn publish(&mut self, record: PublicationRecord<'_>) -> io::Result<()> {
        self.records += 1;
        if let PublicationChange::Correction { valid_hours, .. } = record.change {
            self.corrected = Some(valid_hours);
        }
        if let PublicationChange::Record(PublicationValue::Value(ProductCell::Known(
            ProductValue::Number(Number::Unsigned(value)),
        ))) = record.change
        {
            self.last = Some(value);
        }
        Ok(())
    }
    fn coverage(&mut self, _: Coverage<'_>) -> io::Result<()> {
        Ok(())
    }
    fn provisional(&mut self, window: ProvisionalWindow<'_>) -> io::Result<()> {
        window.validate(interval(10, 19), &window_subscription(), &NoClosures)?;
        self.windows += 1;
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct WindowTestSummary {
    value: u64,
    later: u64,
}
impl BoundarySummary for WindowTestSummary {
    fn value(&self, key: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>> {
        if key.varint() != [1] {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let value = match field.0 {
            3 => self.value,
            5 => self.later,
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        };
        Ok(ProductCell::Known(ProductValue::Number(Number::Unsigned(
            value,
        ))))
    }
    fn bytes(&self) -> u64 {
        size_of::<Self>() as u64
    }
}
fn later(summary: &dyn BoundarySummary) -> u64 {
    match summary.value(key(&[1]), ProductFieldId(5)).unwrap() {
        ProductCell::Known(ProductValue::Number(Number::Unsigned(value))) => value,
        _ => panic!("missing later state"),
    }
}
struct WindowFamily {
    value: Option<u64>,
    delta: u64,
    summary: WindowTestSummary,
    later: u64,
}
impl Family for WindowFamily {
    fn subscription(&self) -> Subscription {
        window_subscription()
    }
    fn begin(
        &mut self,
        at: &Start<'_>,
        context: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        assert_eq!(context.hour(), at.hours.first());
        assert_eq!(context.exact(TableId::Countries, key(&[1]), 0)?, known(1));
        self.value = match &at.boundary {
            StartingBoundary::ClassB(SummaryInput::Resolved(summary))
            | StartingBoundary::Replay { summary, .. } => Some(TestSummary::read_value(*summary)?),
            StartingBoundary::ClassB(SummaryInput::Provisional(_)) => None,
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        };
        self.delta = 0;
        self.later = 0;
        context.schedule(timer(at.hours.first(), &[1]))
    }
    fn hour(
        &mut self,
        at: &At<'_>,
        context: &mut dyn CallContext,
        out: &mut dyn Publication,
    ) -> io::Result<()> {
        assert_eq!(context.hour(), at.hour);
        assert!(
            matches!(at.events, [Event::Tick { due, .. }] if due.len() == 1 && due[0] == timer(at.hour, &[1]))
        );
        let CellValue::Known(Value::I64(input)) =
            context.exact(TableId::Countries, key(&[1]), 0)?
        else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        let input = u64::try_from(input).map_err(|_| io::ErrorKind::InvalidData)?;
        if at.hour <= 12 {
            self.delta += input;
        } else {
            self.later += input;
        }
        if at.hour < 19 {
            context.schedule(timer(at.hour + 1, &[1]))?;
        }
        if self.value.is_none() {
            if at.hour == 11 {
                out.provisional(ProvisionalWindow {
                    key: key(&[1]),
                    rule: DeclarationId(2),
                    hours: interval(10, 12),
                    dependencies: &Dependencies,
                })?;
            }
            return Ok(());
        }
        let value = if at.hour <= 12 {
            self.value.unwrap() + self.delta
        } else {
            self.later
        };
        out.publish(PublicationRecord {
            hour: at.hour,
            key: key(&[1]),
            field: ProductFieldId(if at.hour <= 12 { 3 } else { 5 }),
            change: PublicationChange::Record(PublicationValue::Value(ProductCell::Known(
                ProductValue::Number(Number::Unsigned(value)),
            ))),
            provenance: Provenance::default(),
            breaks: false,
        })
    }
    fn end(
        &mut self,
        _: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        let (value, kind) = match self.value {
            Some(incoming) => (incoming + self.delta, SummaryKind::Resolved),
            None => (self.delta, SummaryKind::Transfer),
        };
        self.summary = WindowTestSummary {
            value,
            later: self.later,
        };
        Ok(Boundary::ClassB {
            summary: &self.summary,
            kind,
        })
    }
    fn resolve_boundary(
        &mut self,
        incoming: &dyn BoundarySummary,
        transfer: &dyn BoundarySummary,
    ) -> io::Result<Boundary<'_>> {
        self.summary = WindowTestSummary {
            value: TestSummary::read_value(incoming)? + TestSummary::read_value(transfer)?,
            later: later(transfer),
        };
        Ok(Boundary::ClassB {
            summary: &self.summary,
            kind: SummaryKind::Resolved,
        })
    }
}
struct KeptInput {
    hour: Hour,
    value: i64,
}
struct FixtureReplay {
    inputs: Vec<KeptInput>,
}
fn deliver_window_hour(
    family: &mut dyn Family,
    context: &mut FixtureContext,
    out: &mut dyn Publication,
) -> io::Result<()> {
    let hour = context.hour();
    assert_eq!(context.timers.pop_due(hour)?, Some(timer(hour, &[1])));
    let due = [timer(hour, &[1])];
    let events = [Event::Tick {
        hour,
        starts_day: false,
        due: &due,
    }];
    family.hour(&at(hour, &events), context, out)
}
fn whole_end(hours: Interval) -> End<'static> {
    End {
        at: at(hours.last(), &[]),
        ending: Ending::Whole { hours },
        delivered_watermark: Some(hours.last()),
    }
}
impl WindowReplay for FixtureReplay {
    fn replay<'a>(
        &mut self,
        family: &'a mut dyn Family,
        at: &Reconciliation<'_>,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<Boundary<'a>> {
        let subscription = family.subscription();
        let Some(Declaration::Provisional(rule)) =
            subscription.declarations().get(at.scope.rule().0)
        else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        assert_eq!(at.scope.keys()[0].key, key(&[1]));
        assert_eq!(
            at.scope.hours(),
            rule.bound.extent(at.scope.segment(), None)?.hours
        );
        let mut context = FixtureContext::default();
        context.rows.hour = at.scope.hours().first();
        context.rows.cells[0] = known(
            self.inputs
                .first()
                .ok_or(io::ErrorKind::UnexpectedEof)?
                .value,
        );
        at.scope
            .begin(family, at.incoming, false, &mut context, out)?;
        for input in &self.inputs {
            assert!(rule.keeps(
                at.scope.segment(),
                input.hour,
                DeclarationId(0),
                key(&[1]),
                Some(WindowField::Table(0)),
                None
            ));
            context.rows.hour = input.hour;
            context.rows.cells[0] = known(input.value);
            assert_eq!(
                context.timers.pop_due(input.hour)?,
                Some(timer(input.hour, &[1]))
            );
            let due = [timer(input.hour, &[1])];
            assert!(at.scope.timer_due(input.hour, due[0]));
            at.scope.hour(
                family,
                &crate::tests::at(
                    input.hour,
                    &[Event::Tick {
                        hour: input.hour,
                        starts_day: false,
                        due: &due,
                    }],
                ),
                &mut context,
                out,
            )?;
        }
        assert_eq!(self.inputs.len() as u64, at.scope.hours().hours());
        assert_eq!(context.timers.peek()?, None);
        let boundary = at
            .scope
            .end(family, &whole_end(at.scope.hours()), &mut context, out)?;
        let Boundary::ClassB {
            summary,
            kind: SummaryKind::Resolved,
        } = &boundary
        else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        for field in [3, 5] {
            if summary.value(key(&[1]), ProductFieldId(field))?
                != at
                    .expected_at_close
                    .value(key(&[1]), ProductFieldId(field))?
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(boundary)
    }
}

#[test]
fn class_b_keeps_declared_inputs_before_a_late_window_and_finishes_its_transfer_pass() {
    let segment = interval(10, 19);
    let incoming = TestSummary(7);
    let expected = WindowTestSummary {
        value: 13,
        later: 0,
    };
    let mut family = WindowFamily {
        value: None,
        delta: 0,
        summary: WindowTestSummary { value: 0, later: 0 },
        later: 0,
    };
    let subscription = family.subscription();
    let Declaration::Provisional(rule) = &subscription.declarations()[2] else {
        unreachable!()
    };
    let mut replay = FixtureReplay { inputs: Vec::new() };
    let mut context = FixtureContext::default();
    context.rows.cells[0] = known(1);
    let mut out = CheckedPublication::default();
    family
        .begin(
            &Start {
                hours: segment,
                opens: false,
                boundary: StartingBoundary::ClassB(SummaryInput::Provisional(None)),
            },
            &mut context,
            &mut out,
        )
        .unwrap();
    for hour in 10..=19 {
        context.rows.hour = hour;
        context.rows.cells[0] = known(i64::from(hour - 9));
        if rule.keeps(
            segment,
            hour,
            DeclarationId(0),
            key(&[1]),
            Some(WindowField::Table(0)),
            None,
        ) {
            let CellValue::Known(Value::I64(value)) =
                context.exact(TableId::Countries, key(&[1]), 0).unwrap()
            else {
                unreachable!()
            };
            replay.inputs.push(KeptInput { hour, value });
        }
        assert!(!rule.keeps(
            segment,
            hour,
            DeclarationId(0),
            key(&[2]),
            Some(WindowField::Table(0)),
            None
        ));
        if context.timers.is_due(hour).unwrap() {
            deliver_window_hour(&mut family, &mut context, &mut out).unwrap();
        }
        if hour == 10 {
            assert_eq!(out.windows, 0);
            assert_eq!(replay.inputs.len(), 1);
        }
    }
    assert_eq!(
        replay
            .inputs
            .iter()
            .map(|input| input.value)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(out.windows, 1);
    assert_eq!(out.records, 0);
    let Boundary::ClassB {
        summary: transfer,
        kind: SummaryKind::Transfer,
    } = family
        .end(&whole_end(segment), &mut context, &mut out)
        .unwrap()
    else {
        panic!("missing transfer")
    };
    assert_eq!(TestSummary::read_value(transfer).unwrap(), 6);
    assert_eq!(later(transfer), 49);
    let transfer = WindowTestSummary {
        value: TestSummary::read_value(transfer).unwrap(),
        later: later(transfer),
    };
    let Boundary::ClassB {
        summary,
        kind: SummaryKind::Resolved,
    } = family.resolve_boundary(&incoming, &transfer).unwrap()
    else {
        panic!("missing resolved summary")
    };
    assert_eq!(TestSummary::read_value(summary).unwrap(), 13);
    let outgoing = WindowTestSummary {
        value: TestSummary::read_value(summary).unwrap(),
        later: later(summary),
    };
    assert_eq!(outgoing.later, 49);
    let keys = [ReplayKey {
        key: key(&[1]),
        extent: WindowExtent {
            hours: interval(10, 12),
            end: WindowEnd::Prefix,
        },
    }];
    let scope = ReplayScope::new(
        segment,
        DeclarationId(2),
        &keys,
        &Dependencies,
        &subscription,
        &NoClosures,
    )
    .unwrap();
    let reconciliation = Reconciliation {
        scope,
        incoming: &incoming,
        expected_at_close: &expected,
    };
    let Boundary::ClassB {
        summary,
        kind: SummaryKind::Resolved,
    } = replay
        .replay(
            &mut family,
            &reconciliation,
            &mut ReplayPublication::new(scope, &mut out),
        )
        .unwrap()
    else {
        panic!("missing reconciled summary")
    };
    assert_eq!(TestSummary::read_value(summary).unwrap(), 13);
    assert_eq!(later(summary), 0);
    let merged = ReconciledSummary {
        dependencies: &Dependencies,
        replayed: summary,
        transfer: &outgoing,
    };
    assert_eq!(TestSummary::read_value(&merged).unwrap(), 13);
    assert_eq!(later(&merged), 49);
    assert_eq!(out.last, Some(13));
    assert_eq!(out.records, 3);
    assert!(
        replay
            .replay(
                &mut family,
                &Reconciliation {
                    expected_at_close: &outgoing,
                    ..reconciliation
                },
                &mut ReplayPublication::new(scope, &mut out)
            )
            .is_err()
    );
    let incorrect = WindowTestSummary {
        value: 14,
        later: 0,
    };
    assert!(
        replay
            .replay(
                &mut family,
                &Reconciliation {
                    expected_at_close: &incorrect,
                    ..reconciliation
                },
                &mut ReplayPublication::new(scope, &mut out)
            )
            .is_err()
    );
}

#[test]
fn provisional_window_checks_its_subscription_rule_and_all_outgoing_dependencies() {
    let subscription = window_subscription();
    let window = ProvisionalWindow {
        key: key(&[1]),
        rule: DeclarationId(2),
        hours: interval(10, 12),
        dependencies: &Dependencies,
    };
    window
        .validate(interval(10, 19), &subscription, &NoClosures)
        .unwrap();
    assert!(
        window
            .validate(interval(11, 19), &subscription, &NoClosures)
            .is_err()
    );
    let short = ProvisionalWindow {
        hours: interval(10, 11),
        ..window
    };
    assert!(
        short
            .validate(interval(10, 19), &subscription, &NoClosures)
            .is_err()
    );
    let outside = ProvisionalWindow {
        hours: interval(10, 13),
        ..window
    };
    assert!(
        outside
            .validate(interval(10, 19), &subscription, &NoClosures)
            .is_err()
    );
    let wrong_rule = ProvisionalWindow {
        rule: DeclarationId(0),
        ..window
    };
    assert!(
        wrong_rule
            .validate(interval(10, 19), &subscription, &NoClosures)
            .is_err()
    );
    let mut declarations = subscription.declarations().to_vec();
    let Declaration::Provisional(rule) = &mut declarations[2] else {
        unreachable!()
    };
    rule.outputs = Arc::from([WindowOutput {
        keys: window_keys(),
        fields: Arc::from([ProductFieldId(4)]),
        target: DependencyTarget::Publication,
    }]);
    let mismatched = Subscription::new(Arc::from(declarations)).unwrap();
    assert!(
        window
            .validate(interval(10, 19), &mismatched, &NoClosures)
            .is_err()
    );
}

#[test]
fn inherited_references_resolve_known_and_missing_fields_from_the_true_summary() {
    let summary = TestSummary(42);
    assert_eq!(
        summary
            .resolve(Inherited {
                key: key(&[1]),
                field: ProductFieldId(3)
            })
            .unwrap(),
        ProductCell::Known(ProductValue::Number(Number::Unsigned(42)))
    );
    assert_eq!(
        summary
            .resolve(Inherited {
                key: key(&[1]),
                field: ProductFieldId(4)
            })
            .unwrap(),
        ProductCell::Missing(missing(interval(1, 9)))
    );
    assert!(
        summary
            .resolve(Inherited {
                key: key(&[2]),
                field: ProductFieldId(3)
            })
            .is_err()
    );
}

#[test]
fn ordered_lane_keeps_its_state_in_place_across_segments_without_materializing_a_boundary() {
    struct OrderedFamily {
        state: Box<[u64; 4]>,
    }
    impl Family for OrderedFamily {
        fn subscription(&self) -> Subscription {
            subscription(vec![Declaration::Ordered])
        }
        fn begin(
            &mut self,
            at: &Start<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<()> {
            assert!(matches!(at.boundary, StartingBoundary::ClassC));
            Ok(())
        }
        fn hour(
            &mut self,
            _: &At<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<()> {
            self.state[0] += 1;
            Ok(())
        }
        fn end(
            &mut self,
            _: &End<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<Boundary<'_>> {
            Ok(Boundary::ClassC)
        }
    }
    let mut family = OrderedFamily {
        state: Box::new([7, 8, 9, 10]),
    };
    let address = family.state.as_ptr();
    let mut context = FixtureContext::default();
    for hours in [interval(10, 19), interval(20, 29)] {
        context.rows.hour = hours.first();
        family
            .begin(
                &Start {
                    hours,
                    opens: false,
                    boundary: StartingBoundary::ClassC,
                },
                &mut context,
                &mut NullPublication,
            )
            .unwrap();
        family
            .hour(&at(hours.first(), &[]), &mut context, &mut NullPublication)
            .unwrap();
        assert!(matches!(
            family
                .end(&whole_end(hours), &mut context, &mut NullPublication)
                .unwrap(),
            Boundary::ClassC
        ));
        assert_eq!(family.state.as_ptr(), address);
    }
    assert_eq!(*family.state, [9, 8, 9, 10]);
}

#[test]
fn window_rules_reject_invalid_inputs_unbounded_prefixes_and_ordered_mixing() {
    let subscription = window_subscription();
    for failure in 0..7 {
        let mut declarations = subscription.declarations().to_vec();
        let Declaration::Provisional(rule) = &mut declarations[2] else {
            unreachable!()
        };
        match failure {
            0 => rule.bound = WindowBound::PrefixHours(0),
            1 => {
                rule.inputs = Arc::from([WindowInput {
                    declaration: DeclarationId(2),
                    keys: WindowKeys::All,
                    fields: Arc::from([]),
                }])
            }
            2 => {
                rule.inputs = Arc::from([WindowInput {
                    declaration: DeclarationId(99),
                    keys: WindowKeys::All,
                    fields: Arc::from([]),
                }])
            }
            3 => rule.bound = WindowBound::PrefixHours(u64::MAX),
            4 => {
                rule.inputs = Arc::from([WindowInput {
                    declaration: DeclarationId(0),
                    keys: WindowKeys::All,
                    fields: Arc::from([WindowField::Table(1)]),
                }])
            }
            5 => {
                rule.inputs = Arc::from([WindowInput {
                    declaration: DeclarationId(1),
                    keys: WindowKeys::All,
                    fields: Arc::from([WindowField::Table(0)]),
                }])
            }
            _ => declarations.push(Declaration::Ordered),
        }
        assert!(Subscription::new(Arc::from(declarations)).is_err());
    }
    assert_eq!(
        WindowBound::PrefixHours(3)
            .extent(interval(Hour::MAX - 1, Hour::MAX), None)
            .unwrap(),
        WindowExtent {
            hours: interval(Hour::MAX - 1, Hour::MAX),
            end: WindowEnd::Segment
        }
    );
}

#[test]
fn net_memberships_keep_missing_and_recovered_causes_at_step_eight() {
    let evidence = MembershipEvidence {
        kind: MembershipMissing::Lost,
        hours: interval(8, 10),
        reason: "synthetic damage",
    };
    let left = Event::Left {
        cause: MembershipCause::Missing(evidence),
        relation: RelationId(0),
        value: key(&[1]),
        member: key(&[2]),
    };
    let factual = Event::Left {
        cause: MembershipCause::Changed,
        relation: RelationId(0),
        value: key(&[1]),
        member: key(&[2]),
    };
    assert_eq!(left.order(), factual.order());
    let joined = Event::Joined {
        cause: MembershipCause::Recovered(evidence),
        relation: RelationId(0),
        value: key(&[1]),
        member: key(&[2]),
    };
    assert!(left.order() < joined.order());
    assert!(
        Event::Unknown {
            table: TableId::Countries,
            key: key(&[1]),
            field: 0,
            slot: 0,
            missing: &missing(Interval::hour(10)),
        }
        .order()
            < left.order()
    );
    assert!(
        joined.order()
            < Event::Tick {
                hour: 10,
                starts_day: false,
                due: &[]
            }
            .order()
    );
    assert!(
        matches!(left, Event::Left { cause: MembershipCause::Missing(actual), .. } if actual == evidence)
    );
    assert!(
        matches!(joined, Event::Joined { cause: MembershipCause::Recovered(actual), .. } if actual == evidence)
    );
}

#[test]
fn union_capacity_counts_raw_insertions_even_when_declarations_will_deduplicate() {
    let family = subscription(vec![
        Declaration::Lookup {
            table: TableId::Countries,
            fields: Arc::from([0]),
            shape: LookupShape::Exact,
        },
        Declaration::Lookup {
            table: TableId::Countries,
            fields: Arc::from([0]),
            shape: LookupShape::Exact,
        },
    ]);
    let mut storage = UnionStorage::new(
        UnionCapacity {
            tables: 2,
            fields: 1,
            ..Default::default()
        },
        1024,
    )
    .unwrap();
    assert!(matches!(
        Subscription::union(std::slice::from_ref(&family), &mut storage),
        Err(SubscriptionError::Capacity {
            owner: "fields",
            needed: 2
        })
    ));
    let mut storage = UnionStorage::new(
        UnionCapacity {
            tables: 2,
            fields: 2,
            ..Default::default()
        },
        1024,
    )
    .unwrap();
    let families = [family];
    let union = Subscription::union(&families, &mut storage).unwrap();
    assert_eq!(union.tables, &[TableId::Countries]);
    assert_eq!(union.fields.len(), 1);
}

#[test]
fn publication_carries_product_types_provenance_breaks_references_rates_and_backdated_corrections()
{
    let inherited = Inherited {
        key: key(&[1]),
        field: ProductFieldId(7),
    };
    let provenance = Provenance {
        supplying_read: Some(BoundaryValue::Known(20)),
        holder: Some(BoundaryValue::Known(key(&[2]))),
        identity_first: Some(BoundaryValue::Inherited(inherited)),
    };
    let absent = missing(interval(1, 2));
    let list = [
        ProductCell::Known(ProductValue::Text("nested")),
        ProductCell::Missing(absent),
    ];
    let values = [
        ProductValue::Number(Number::Fixed {
            units: -17,
            scale: 1000,
        }),
        ProductValue::Bytes(&[0, 1]),
        ProductValue::Text("product text"),
        ProductValue::Bool(false),
        ProductValue::Key(key(&[0x80, 1])),
        ProductValue::List(&list),
    ];
    let mut out = CheckedPublication::default();
    for value in values {
        let record = PublicationRecord {
            hour: 20,
            key: key(&[1]),
            field: ProductFieldId(9999),
            change: PublicationChange::Record(PublicationValue::Value(ProductCell::Known(value))),
            provenance,
            breaks: true,
        };
        assert!(record.breaks);
        assert_eq!(
            record.provenance.supplying_read,
            Some(BoundaryValue::Known(20))
        );
        assert_eq!(
            record.provenance.holder,
            Some(BoundaryValue::Known(key(&[2])))
        );
        assert_eq!(
            record.provenance.identity_first,
            Some(BoundaryValue::Inherited(inherited))
        );
        out.publish(record).unwrap();
    }
    for value in [
        PublicationValue::Inherited(inherited),
        PublicationValue::Rate(Rate::Known(Number::Signed(2))),
        PublicationValue::Rate(Rate::Missing(absent)),
        PublicationValue::Rate(Rate::Inherited(inherited)),
        PublicationValue::Value(ProductCell::Missing(absent)),
        PublicationValue::End,
    ] {
        let record = PublicationRecord {
            hour: 20,
            key: key(&[1]),
            field: ProductFieldId(0),
            change: PublicationChange::Correction {
                valid_hours: interval(1, 19),
                replacement: value,
            },
            provenance,
            breaks: false,
        };
        assert_eq!(record.hour, 20);
        out.publish(record).unwrap();
    }
    assert_eq!(out.corrected, Some(interval(1, 19)));
    assert_eq!(out.records, 12);
}

struct NoClosures;
impl WindowClosures for NoClosures {
    fn closed_at(&self, _: DeclarationId, _: Key<'_>) -> io::Result<Option<Hour>> {
        Ok(None)
    }
}
impl WindowClosures for FixtureContext<'_> {
    fn closed_at(&self, rule: DeclarationId, key: Key<'_>) -> io::Result<Option<Hour>> {
        Ok(self
            .closed_windows
            .iter()
            .find(|(r, k, _)| *r == rule && k.as_slice() == key.varint())
            .map(|(_, _, hour)| *hour))
    }
}

struct SpanFamily {
    subscription: Subscription,
    calls: usize,
    deliveries: Vec<(Interval, Vec<Interval>)>,
}
impl SpanFamily {
    fn capture(&mut self, at: &At<'_>) {
        assert!(
            !at.events
                .iter()
                .any(|event| matches!(event, Event::Observed { .. }))
        );
        for observation in at.observations {
            self.deliveries
                .push((observation.hours(), observation.holds().collect()));
        }
    }
}
impl Family for SpanFamily {
    fn subscription(&self) -> Subscription {
        self.subscription.clone()
    }
    fn begin(
        &mut self,
        _: &Start<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        Ok(())
    }
    fn hour(
        &mut self,
        at: &At<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        self.calls += 1;
        self.capture(at);
        Ok(())
    }
    fn end(
        &mut self,
        at: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        self.capture(&at.at);
        Ok(Boundary::ClassA)
    }
}

#[test]
fn never_wake_truth_is_shared_queryable_before_a_call_and_delivered_as_runs_at_call_and_end() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let evaluations = Arc::new(AtomicUsize::new(0));
    let counter = evaluations.clone();
    let observation = Observation {
        product: ProductId("fixture"),
        id: ObservationId(3),
        wake: ObserveWake::Never,
        tables: Arc::from([TableId::Countries]),
        predicate: Arc::new(move |metadata| {
            counter.fetch_add(1, Ordering::Relaxed);
            if matches!(metadata.hour, 10 | 11 | 50) {
                Truth::Fails
            } else {
                Truth::Holds
            }
        }),
    };
    let families = [
        subscription(vec![Declaration::Observe(observation.clone())]),
        subscription(vec![Declaration::Observe(Observation {
            product: ProductId("fixture"),
            id: ObservationId(9),
            wake: ObserveWake::EachHour,
            ..observation
        })]),
        subscription(vec![Declaration::ObservationLookup {
            source: ObservationSource {
                product: ProductId("fixture"),
                observation: ObservationId(3),
            },
        }]),
    ];
    let mut storage = union_storage();
    let union = Subscription::union(&families, &mut storage).unwrap();
    assert_eq!(union.observations.len(), 1);
    assert_eq!(union.observation_bindings[0].wake, ObserveWake::Never);
    assert_eq!(union.observation_bindings[1].wake, ObserveWake::EachHour);
    assert_eq!(
        union.observation_lookups,
        &[ObservationLookupBinding {
            family: FamilyId(2),
            local: DeclarationId(0),
            shared: SharedObservationId(0)
        }]
    );
    let mut backing = [Interval::hour(0); 4];
    let mut stream = ObservationRuns::new(1, &mut backing, &mut []);
    let mut owner = SpanFamily {
        subscription: families[0].clone(),
        calls: 0,
        deliveries: Vec::new(),
    };
    let mut waking_calls = 0;
    for hour in 1..=100 {
        let holds = union.observations[0].evaluate(&Metadata {
            hour,
            tables: &[],
            unavailable: None,
        });
        stream.append(Interval::hour(hour), holds).unwrap();
        assert!(union.observation_bindings[0].observed(holds).is_none());
        if union.observation_bindings[1].observed(holds).is_some() {
            waking_calls += 1;
        }
        if hour == 40 {
            assert_eq!(owner.calls, 0);
            let consumer = FixtureContext {
                observation_stream: Some(&stream),
                ..Default::default()
            };
            let answer = consumer
                .observation(union.observation_lookups[0].local)
                .unwrap();
            assert_eq!(answer.metadata_watermark(), Some(40));
            assert_eq!(answer.truth_at(17).unwrap(), Some(Truth::Holds));
            assert_eq!(answer.truth_at(10).unwrap(), Some(Truth::Fails));
            assert_eq!(answer.truth_at(41).unwrap(), None);
            assert_eq!(answer.truth_at(0).unwrap(), None);
            assert_eq!(owner.calls, 0);
        }
        if hour == 70 {
            let mut runs = Vec::new();
            stream
                .visit(interval(1, 70), &mut |run| {
                    assert_eq!(run.truth, Truth::Holds);
                    runs.push(run.hours);
                    Ok(())
                })
                .unwrap();
            let observations = [ObservationSpans::new(
                ObservationId(3),
                interval(1, 70),
                SpanView::new(&runs).unwrap(),
                &[],
            )
            .unwrap()];
            let events = [Event::Tick {
                hour,
                starts_day: false,
                due: &[],
            }];
            owner
                .hour(
                    &At {
                        observations: &observations,
                        ..at(hour, &events)
                    },
                    &mut FixtureContext::default(),
                    &mut NullPublication,
                )
                .unwrap();
        }
    }
    let mut runs = Vec::new();
    stream
        .visit(interval(71, 100), &mut |run| {
            assert_eq!(run.truth, Truth::Holds);
            runs.push(run.hours);
            Ok(())
        })
        .unwrap();
    let observations = [ObservationSpans::new(
        ObservationId(3),
        interval(71, 100),
        SpanView::new(&runs).unwrap(),
        &[],
    )
    .unwrap()];
    let end = End {
        at: At {
            observations: &observations,
            ..at(100, &[])
        },
        ..whole_end(interval(1, 100))
    };
    owner
        .end(&end, &mut FixtureContext::default(), &mut NullPublication)
        .unwrap();
    assert_eq!(owner.calls, 1);
    assert_eq!(waking_calls, 97);
    assert_eq!(evaluations.load(Ordering::Relaxed), 100);
    assert_eq!(
        owner.deliveries,
        [
            (
                interval(1, 70),
                vec![interval(1, 9), interval(12, 49), interval(51, 70)]
            ),
            (interval(71, 100), vec![interval(71, 100)]),
        ]
    );
    let mut runs = Vec::new();
    stream
        .visit(interval(1, 100), &mut |run| {
            assert_eq!(run.truth, Truth::Holds);
            runs.push(run.hours);
            Ok(())
        })
        .unwrap();
    let observations = [ObservationSpans::new(
        ObservationId(3),
        interval(1, 100),
        SpanView::new(&runs).unwrap(),
        &[],
    )
    .unwrap()];
    let mut never_called = SpanFamily {
        subscription: families[0].clone(),
        calls: 0,
        deliveries: Vec::new(),
    };
    never_called
        .end(
            &End {
                at: At {
                    observations: &observations,
                    ..at(100, &[])
                },
                ..whole_end(interval(1, 100))
            },
            &mut FixtureContext::default(),
            &mut NullPublication,
        )
        .unwrap();
    assert_eq!(never_called.calls, 0);
    assert_eq!(
        never_called.deliveries,
        [(
            interval(1, 100),
            vec![interval(1, 9), interval(12, 49), interval(51, 100)]
        )]
    );
}

#[test]
fn observation_stream_refuses_gaps_and_preserves_its_watermark_on_capacity_failure() {
    let mut backing = [Interval::hour(0)];
    let mut stream = ObservationRuns::new(1, &mut backing, &mut []);
    assert_eq!(stream.metadata_watermark(), None);
    stream.append(interval(1, 2), Truth::Holds).unwrap();
    stream.append(Interval::hour(3), Truth::Fails).unwrap();
    assert!(stream.append(Interval::hour(5), Truth::Holds).is_err());
    assert!(stream.append(Interval::hour(4), Truth::Holds).is_err());
    assert_eq!(stream.metadata_watermark(), Some(3));
    assert_eq!(stream.truth_at(3).unwrap(), Some(Truth::Fails));
    assert_eq!(stream.truth_at(4).unwrap(), None);
    assert!(stream.visit(interval(1, 4), &mut |_| Ok(())).is_err());
    assert!(stream.append(Interval::hour(2), Truth::Fails).is_err());
}

#[test]
fn union_rejects_unknown_or_waking_observation_sources() {
    for wake in [ObserveWake::EachHour, ObserveWake::Never] {
        let source = ObservationSource {
            product: ProductId("fixture"),
            observation: ObservationId(if wake == ObserveWake::Never { 99 } else { 0 }),
        };
        let families = [
            subscription(vec![Declaration::Observe(Observation {
                product: ProductId("fixture"),
                id: ObservationId(0),
                wake,
                tables: Arc::from([]),
                predicate: Arc::new(|_| Truth::Holds),
            })]),
            subscription(vec![Declaration::ObservationLookup { source }]),
        ];
        assert!(
            matches!(Subscription::union(&families, &mut union_storage()), Err(SubscriptionError::InvalidObservationSource { source: actual }) if actual == source)
        );
    }
}

struct ClosingFamily {
    subscription: Subscription,
}
impl Family for ClosingFamily {
    fn subscription(&self) -> Subscription {
        self.subscription.clone()
    }
    fn begin(
        &mut self,
        _: &Start<'_>,
        context: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        context.schedule(timer(11, &[1]))?;
        context.schedule(timer(14, &[2]))
    }
    fn hour(
        &mut self,
        at: &At<'_>,
        context: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        for event in at.events {
            if let Event::Tick { due, .. } = event {
                for timer in *due {
                    context.close_window(DeclarationId(2), timer.id)?;
                }
            }
        }
        Ok(())
    }
    fn end(
        &mut self,
        _: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        Ok(Boundary::ClassA)
    }
}

#[test]
fn per_key_windows_keep_only_declared_fields_until_each_keys_close_or_segment_end() {
    let mut declarations = window_subscription().declarations().to_vec();
    let Declaration::Provisional(rule) = &mut declarations[2] else {
        unreachable!()
    };
    rule.bound = WindowBound::UntilClosed;
    rule.inputs = Arc::from([WindowInput {
        declaration: DeclarationId(0),
        keys: WindowKeys::All,
        fields: Arc::from([WindowField::Table(0)]),
    }]);
    rule.outputs = Arc::from([WindowOutput {
        keys: WindowKeys::All,
        fields: Arc::from([ProductFieldId(3)]),
        target: DependencyTarget::Publication,
    }]);
    let subscription = Subscription::new(Arc::from(declarations)).unwrap();
    let families = [subscription.clone()];
    let mut union_storage = union_storage();
    let union = Subscription::union(&families, &mut union_storage).unwrap();
    assert_eq!(union.windows.len(), 1);
    assert_eq!(union.windows[0].family, FamilyId(0));
    assert_eq!(union.windows[0].local, DeclarationId(2));
    assert_eq!(union.windows[0].rule.bound, WindowBound::UntilClosed);
    let Declaration::Provisional(rule) = &subscription.declarations()[2] else {
        unreachable!()
    };
    let mut family = ClosingFamily {
        subscription: subscription.clone(),
    };
    let mut context = FixtureContext::default();
    let segment = interval(10, 19);
    family
        .begin(
            &Start {
                hours: segment,
                opens: false,
                boundary: StartingBoundary::ClassB(SummaryInput::Provisional(None)),
            },
            &mut context,
            &mut NullPublication,
        )
        .unwrap();
    let mut kept = [Vec::new(), Vec::new(), Vec::new()];
    for hour in 10..=19 {
        context.rows.hour = hour;
        for (slot, bytes) in [[1], [2], [3]].iter().enumerate() {
            let key = key(bytes);
            let close = context.closed_at(DeclarationId(2), key).unwrap();
            if rule.keeps(
                segment,
                hour,
                DeclarationId(0),
                key,
                Some(WindowField::Table(0)),
                close,
            ) {
                kept[slot].push(hour);
            }
            assert!(!rule.keeps(
                segment,
                hour,
                DeclarationId(0),
                key,
                Some(WindowField::Table(1)),
                close
            ));
            assert!(!rule.keeps(segment, hour, DeclarationId(1), key, None, close));
        }
        if matches!(hour, 11 | 14) {
            let due = [timer(hour, if hour == 11 { &[1] } else { &[2] })];
            assert_eq!(context.timers.pop_due(hour).unwrap(), Some(due[0]));
            let events = [Event::Tick {
                hour,
                starts_day: false,
                due: &due,
            }];
            family
                .hour(&at(hour, &events), &mut context, &mut NullPublication)
                .unwrap();
        }
    }
    assert_eq!(kept[0], [10, 11]);
    assert_eq!(kept[1], [10, 11, 12, 13, 14]);
    assert_eq!(kept[2], (10..=19).collect::<Vec<_>>());
    context.close_window(DeclarationId(2), key(&[1])).unwrap();
    assert_eq!(
        context.closed_at(DeclarationId(2), key(&[1])).unwrap(),
        Some(11)
    );
    assert_eq!(
        rule.bound.extent(segment, Some(11)).unwrap(),
        WindowExtent {
            hours: interval(10, 11),
            end: WindowEnd::Closed
        }
    );
    assert_eq!(
        rule.bound.extent(segment, None).unwrap(),
        WindowExtent {
            hours: segment,
            end: WindowEnd::Segment
        }
    );
    let dependencies = OneDependency;
    let window = ProvisionalWindow {
        key: key(&[1]),
        rule: DeclarationId(2),
        hours: interval(10, 11),
        dependencies: &dependencies,
    };
    window.validate(segment, &subscription, &context).unwrap();
    assert!(
        ProvisionalWindow {
            hours: interval(10, 12),
            ..window
        }
        .validate(segment, &subscription, &context)
        .is_err()
    );
}

struct OneDependency;
impl WindowDependencies for OneDependency {
    fn hours(
        &self,
        key: Key<'_>,
        field: ProductFieldId,
        target: DependencyTarget,
    ) -> io::Result<Option<Interval>> {
        Ok((key.varint() == [1]
            && field == ProductFieldId(3)
            && target == DependencyTarget::Publication)
            .then_some(interval(10, 11)))
    }
    fn visit(
        &self,
        visitor: &mut dyn FnMut(WindowDependency<'_>) -> io::Result<()>,
    ) -> io::Result<()> {
        visitor(WindowDependency {
            hours: interval(10, 11),
            key: key(&[1]),
            field: ProductFieldId(3),
            target: DependencyTarget::Publication,
        })
    }
}

#[test]
fn segment_retention_is_explicit_including_short_prefixes_and_never_closed_keys() {
    let segment = interval(10, 19);
    for bound in [
        WindowBound::Segment,
        WindowBound::UntilClosed,
        WindowBound::PrefixHours(100),
    ] {
        assert_eq!(
            bound.extent(segment, None).unwrap(),
            WindowExtent {
                hours: segment,
                end: WindowEnd::Segment
            }
        );
    }
    assert_eq!(
        WindowBound::UntilClosed
            .extent(segment, Some(19))
            .unwrap()
            .end,
        WindowEnd::Segment
    );
    assert_eq!(
        WindowBound::PrefixHours(3).extent(segment, None).unwrap(),
        WindowExtent {
            hours: interval(10, 12),
            end: WindowEnd::Prefix
        }
    );
    assert!(
        WindowBound::PrefixHours(u64::MAX)
            .extent(segment, None)
            .is_err()
    );
    assert!(WindowBound::UntilClosed.extent(segment, Some(9)).is_err());
    assert!(WindowBound::UntilClosed.extent(segment, Some(20)).is_err());
}

#[test]
fn slice_timer_bucket_backlinks_survive_collisions_swaps_and_cluster_deletion() {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };
    let bytes: Vec<[u8; 1]> = (0..128).map(|id| [id]).collect();
    let bucket = |bytes: &[u8]| {
        let mut hash = DefaultHasher::new();
        key(bytes).hash(&mut hash);
        hash.finish() % 8
    };
    let colliding: Vec<_> = bytes
        .iter()
        .filter(|bytes| bucket(*bytes) == bucket(&[0]))
        .take(4)
        .collect();
    assert_eq!(colliding.len(), 4);
    let mut storage = [timer(0, &[0]); 4];
    let mut index = [usize::MAX; 8];
    let mut buckets = [0; 4];
    let mut store = SliceTimers::new(&mut storage, &mut index, &mut buckets).unwrap();
    for (slot, bytes) in colliding.iter().enumerate() {
        store.push(timer(slot as Hour, *bytes)).unwrap();
    }
    store.swap(0, 3).unwrap();
    store.swap(1, 2).unwrap();
    store.pop_last().unwrap();
    for slot in 0..store.len() {
        let id = store.read(slot).unwrap().id.varint().to_vec();
        assert_eq!(store.find(key(&id)).unwrap(), Some(slot));
    }
    assert_eq!(store.find(key(colliding[0])).unwrap(), None);
    store.push(timer(99, colliding[0])).unwrap();
    assert_eq!(store.find(key(colliding[0])).unwrap(), Some(3));
}

#[test]
fn never_wake_unknown_gap_advances_watermark_and_is_delivered_separately_from_false() {
    struct Capture {
        calls: usize,
        deliveries: Vec<(Interval, Vec<Interval>, Vec<Interval>, u64)>,
    }
    impl Capture {
        fn capture(&mut self, at: &At<'_>) {
            assert!(
                !at.events
                    .iter()
                    .any(|event| matches!(event, Event::Observed { .. }))
            );
            let delivery = at.observations[0];
            let holds: Vec<_> = delivery.holds().collect();
            let unknown: Vec<_> = delivery
                .unknown()
                .map(|run| {
                    assert_eq!(run.reason, ObservationUnknown::Gap(REASON));
                    run.hours
                })
                .collect();
            let false_hours = delivery.hours().hours()
                - holds.iter().map(|run| run.hours()).sum::<u64>()
                - unknown.iter().map(|run| run.hours()).sum::<u64>();
            self.deliveries
                .push((delivery.hours(), holds, unknown, false_hours));
        }
    }
    impl Family for Capture {
        fn subscription(&self) -> Subscription {
            subscription(vec![])
        }
        fn begin(
            &mut self,
            _: &Start<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<()> {
            Ok(())
        }
        fn hour(
            &mut self,
            at: &At<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<()> {
            self.calls += 1;
            self.capture(at);
            Ok(())
        }
        fn end(
            &mut self,
            end: &End<'_>,
            _: &mut dyn CallContext,
            _: &mut dyn Publication,
        ) -> io::Result<Boundary<'_>> {
            self.capture(&end.at);
            Ok(Boundary::ClassA)
        }
    }
    let families = [subscription(vec![Declaration::Observe(Observation {
        product: ProductId("identities"),
        id: ObservationId(0),
        wake: ObserveWake::Never,
        tables: Arc::from([]),
        predicate: Arc::new(|metadata| {
            if matches!(metadata.hour, 2 | 8) {
                Truth::Fails
            } else {
                Truth::Holds
            }
        }),
    })])];
    let mut storage = union_storage();
    let union = Subscription::union(&families, &mut storage).unwrap();
    let mut holds = [Interval::hour(0); 2];
    let mut unknown = [UnknownRun {
        hours: Interval::hour(0),
        reason: ObservationUnknown::NotRecorded,
    }];
    let mut stream = ObservationRuns::new(1, &mut holds, &mut unknown);
    let mut family = Capture {
        calls: 0,
        deliveries: Vec::new(),
    };
    for hour in 1..=9 {
        let truth = union.observations[0].evaluate(&Metadata {
            hour,
            tables: &[],
            unavailable: (3..=7)
                .contains(&hour)
                .then_some(ObservationUnknown::Gap(REASON)),
        });
        assert!(union.observation_bindings[0].observed(truth).is_none());
        stream.append(Interval::hour(hour), truth).unwrap();
        if hour == 5 {
            let observations = [stream.delivery(ObservationId(0), interval(1, 5)).unwrap()];
            family
                .hour(
                    &At {
                        observations: &observations,
                        ..at(5, &[])
                    },
                    &mut FixtureContext::default(),
                    &mut NullPublication,
                )
                .unwrap();
        }
    }
    let consumer = FixtureContext {
        observation_stream: Some(&stream),
        ..Default::default()
    };
    let answer = consumer.observation(DeclarationId(0)).unwrap();
    assert_eq!(answer.metadata_watermark(), Some(9));
    assert_eq!(
        answer.truth_at(4).unwrap(),
        Some(Truth::Unknown(ObservationUnknown::Gap(REASON)))
    );
    assert_eq!(answer.truth_at(8).unwrap(), Some(Truth::Fails));
    assert_eq!(answer.truth_at(9).unwrap(), Some(Truth::Holds));
    assert_eq!(answer.truth_at(10).unwrap(), None);
    let mut visits = Vec::new();
    answer
        .visit(interval(2, 9), &mut |run| {
            visits.push((
                run.hours,
                match run.truth {
                    Truth::Holds => "holds",
                    Truth::Unknown(ObservationUnknown::Gap(reason)) => {
                        assert_eq!(reason, REASON);
                        "gap"
                    }
                    _ => panic!("false is a complement, never an unknown run"),
                },
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        visits,
        [(interval(3, 7), "gap"), (Interval::hour(9), "holds")]
    );
    let observations = [stream.delivery(ObservationId(0), interval(6, 9)).unwrap()];
    family
        .end(
            &End {
                at: At {
                    observations: &observations,
                    ..at(9, &[])
                },
                ..whole_end(interval(1, 9))
            },
            &mut FixtureContext::default(),
            &mut NullPublication,
        )
        .unwrap();
    assert_eq!(family.calls, 1);
    assert_eq!(
        family.deliveries,
        [
            (
                interval(1, 5),
                vec![Interval::hour(1)],
                vec![interval(3, 5)],
                1
            ),
            (
                interval(6, 9),
                vec![Interval::hour(9)],
                vec![interval(6, 7)],
                1
            ),
        ]
    );
}

#[test]
fn unknown_predicate_provenance_coalesces_and_capacity_failure_keeps_the_watermark() {
    let predicate: Arc<ObservationPredicate> =
        Arc::new(|metadata| match metadata.tables[0].status {
            IndexedStatus::Refused { reason, .. } => {
                Truth::Unknown(ObservationUnknown::Refused(reason))
            }
            _ => Truth::Holds,
        });
    let detail = String::from("synthetic refusal");
    let reason = Reason {
        name: "refusal",
        detail: &detail,
    };
    let tables = [TableMetadata {
        table: TableId::Countries,
        written_fields: &[],
        inserted: Count::new(&[0]).unwrap(),
        deleted: Count::new(&[0]).unwrap(),
        status: IndexedStatus::Refused {
            hours: interval(1, 2),
            reason,
        },
        sample_hour: None,
        acquired: None,
        supplied: None,
    }];
    let truth = predicate(&Metadata {
        hour: 1,
        tables: &tables,
        unavailable: None,
    });
    assert_eq!(truth, Truth::Unknown(ObservationUnknown::Refused(reason)));
    let mut holds = [];
    let mut unknown = [UnknownRun {
        hours: Interval::hour(0),
        reason: ObservationUnknown::NotRecorded,
    }];
    let mut stream = ObservationRuns::new(1, &mut holds, &mut unknown);
    stream.append(Interval::hour(1), truth).unwrap();
    stream.append(Interval::hour(2), truth).unwrap();
    assert!(matches!(
        stream.append(
            Interval::hour(3),
            Truth::Unknown(ObservationUnknown::Damage(REASON))
        ),
        Err(SpanError::Capacity { needed: 2 })
    ));
    assert_eq!(stream.metadata_watermark(), Some(2));
    assert_eq!(stream.truth_at(2).unwrap(), Some(truth));
    assert_eq!(stream.truth_at(3).unwrap(), None);
    assert_eq!(
        stream
            .delivery(ObservationId(0), interval(1, 2))
            .unwrap()
            .unknown()
            .count(),
        1
    );
}

#[test]
fn observation_delivery_clips_borrowed_edge_runs_in_at_and_end() {
    let holds = [interval(1, 100)];
    let unknown = [UnknownRun {
        hours: interval(101, 200),
        reason: ObservationUnknown::Damage(REASON),
    }];
    let delivery = ObservationSpans::new(
        ObservationId(0),
        interval(70, 150),
        SpanView::new(&holds).unwrap(),
        &unknown,
    )
    .unwrap();
    let observations = [delivery];
    let call = At {
        observations: &observations,
        ..at(150, &[])
    };
    let end = End {
        at: call,
        ..whole_end(interval(1, 150))
    };
    for call in [call, end.at] {
        let delivery = call.observations[0];
        assert_eq!(delivery.holds_hours(), 31);
        assert_eq!(delivery.unknown_hours(), 50);
        assert_eq!(delivery.false_hours(), 0);
        assert_eq!(delivery.holds().collect::<Vec<_>>(), [interval(70, 100)]);
        assert_eq!(
            delivery.unknown().collect::<Vec<_>>(),
            [UnknownRun {
                hours: interval(101, 150),
                reason: ObservationUnknown::Damage(REASON)
            }]
        );
        assert_eq!(delivery.holds().map(Interval::hours).sum::<u64>(), 31);
    }
    assert_eq!(holds[0], interval(1, 100));
    assert_eq!(unknown[0].hours, interval(101, 200));
}

#[test]
fn observation_sources_are_stable_across_group_order_and_available_to_foreign_groups() {
    let source = ObservationSource {
        product: ProductId("identities"),
        observation: ObservationId(7),
    };
    let observation = Observation {
        product: source.product,
        id: source.observation,
        wake: ObserveWake::Never,
        tables: Arc::from([TableId::Countries]),
        predicate: Arc::new(|_| Truth::Holds),
    };
    let producer = subscription(vec![Declaration::Observe(observation.clone())]);
    let consumer = subscription(vec![Declaration::ObservationLookup { source }]);
    for families in [
        [producer.clone(), consumer.clone()],
        [consumer.clone(), producer],
    ] {
        let mut storage = union_storage();
        let union = Subscription::union(&families, &mut storage).unwrap();
        assert_eq!(union.observation_lookups.len(), 1);
        assert_eq!(
            union.observation_lookups[0].shared,
            union.observation_bindings[0].shared
        );
    }
    let mut storage = union_storage();
    let union = Subscription::union_with_observations(
        std::slice::from_ref(&consumer),
        std::slice::from_ref(&observation),
        &mut storage,
    )
    .unwrap();
    assert!(union.observation_bindings.is_empty());
    assert_eq!(union.observations.len(), 1);
    assert_eq!(union.tables, [TableId::Countries]);
    assert_eq!(union.observation_lookups[0].shared, SharedObservationId(0));
    assert_eq!(
        union.observations[0].evaluate(&Metadata {
            hour: 9,
            tables: &[],
            unavailable: None
        }),
        Truth::Holds
    );
    let conflicting = Observation {
        predicate: Arc::new(|_| Truth::Fails),
        ..observation.clone()
    };
    assert!(
        matches!(Subscription::union_with_observations(&[consumer], &[observation, conflicting], &mut union_storage()),
        Err(SubscriptionError::ConflictingObservationSource { source: actual }) if actual == source)
    );
}

#[derive(Default)]
struct ShardSummary(std::collections::BTreeMap<Vec<u8>, [u64; 3]>);
impl BoundarySummary for ShardSummary {
    fn value(&self, key: Key<'_>, field: ProductFieldId) -> io::Result<ProductCell<'_>> {
        let values = self
            .0
            .get(key.varint())
            .ok_or(io::ErrorKind::InvalidInput)?;
        let slot = match field.0 {
            3 => 0,
            5 => 1,
            6 => 2,
            _ => return Err(io::ErrorKind::InvalidInput.into()),
        };
        Ok(ProductCell::Known(ProductValue::Number(Number::Unsigned(
            values[slot],
        ))))
    }
    fn bytes(&self) -> u64 {
        self.0.len() as u64 * size_of::<[u64; 3]>() as u64
    }
}
fn shard_value(summary: &dyn BoundarySummary, key: Key<'_>, field: usize) -> io::Result<u64> {
    match summary.value(key, ProductFieldId(field))? {
        ProductCell::Known(ProductValue::Number(Number::Unsigned(value))) => Ok(value),
        _ => Err(io::ErrorKind::InvalidData.into()),
    }
}
struct ShardFamily {
    subscription: Subscription,
    summary: ShardSummary,
    restores: Vec<Vec<u8>>,
    delivered: Vec<(Hour, Vec<u8>)>,
}
impl Family for ShardFamily {
    fn subscription(&self) -> Subscription {
        self.subscription.clone()
    }
    fn begin(
        &mut self,
        at: &Start<'_>,
        context: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<()> {
        let StartingBoundary::Replay { summary, scope } = at.boundary else {
            return Err(io::ErrorKind::InvalidInput.into());
        };
        assert!(summary.value(key(&[3]), ProductFieldId(3)).is_err());
        self.summary.0.clear();
        for item in scope.keys() {
            let value = shard_value(summary, item.key, 3)?;
            let due = Hour::try_from(shard_value(summary, item.key, 6)?)
                .map_err(|_| io::ErrorKind::InvalidData)?;
            self.restores.push(item.key.varint().to_vec());
            self.summary
                .0
                .insert(item.key.varint().to_vec(), [value, 0, due as u64]);
            let id = if item.key.varint() == [1] {
                key(&[41])
            } else {
                key(&[42])
            };
            context.schedule(Timer {
                hour: due,
                id,
                key: item.key,
            })?;
        }
        Ok(())
    }
    fn hour(
        &mut self,
        at: &At<'_>,
        context: &mut dyn CallContext,
        out: &mut dyn Publication,
    ) -> io::Result<()> {
        for event in at.events {
            if let Event::Tick { due, .. } = event {
                for timer in *due {
                    let CellValue::Known(Value::I64(value)) =
                        context.exact(TableId::Countries, timer.key, 0)?
                    else {
                        return Err(io::ErrorKind::InvalidData.into());
                    };
                    self.delivered.push((at.hour, timer.key.varint().to_vec()));
                    let state = self
                        .summary
                        .0
                        .get_mut(timer.key.varint())
                        .ok_or(io::ErrorKind::InvalidInput)?;
                    state[0] += value as u64;
                    out.publish(PublicationRecord {
                        hour: at.hour,
                        key: timer.key,
                        field: ProductFieldId(3),
                        change: PublicationChange::Record(PublicationValue::Value(
                            ProductCell::Known(ProductValue::Number(Number::Unsigned(state[0]))),
                        )),
                        provenance: Provenance::default(),
                        breaks: false,
                    })?;
                    context.close_window(DeclarationId(2), timer.key)?;
                }
            }
        }
        Ok(())
    }
    fn end(
        &mut self,
        _: &End<'_>,
        _: &mut dyn CallContext,
        _: &mut dyn Publication,
    ) -> io::Result<Boundary<'_>> {
        Ok(Boundary::ClassB {
            summary: &self.summary,
            kind: SummaryKind::Resolved,
        })
    }
}
struct ShardReplay {
    restored_timers: usize,
    delivered: Vec<(Hour, Vec<u8>)>,
}
impl WindowReplay for ShardReplay {
    fn replay<'a>(
        &mut self,
        family: &'a mut dyn Family,
        at: &Reconciliation<'_>,
        out: &mut ReplayPublication<'_>,
    ) -> io::Result<Boundary<'a>> {
        let mut context = FixtureContext::default();
        context.rows.cells[0] = known(2);
        context.rows.cells[2] = known(5);
        context.rows.hour = at.scope.hours().first();
        at.scope
            .begin(family, at.incoming, false, &mut context, out)?;
        self.restored_timers = context.timers.len();
        for hour in at.scope.hours().first()..=at.scope.hours().last() {
            context.rows.hour = hour;
            assert!(!at.scope.keeps(
                hour,
                DeclarationId(0),
                key(&[3]),
                Some(WindowField::Table(0))
            ));
            assert!(!at.scope.keeps(
                hour,
                DeclarationId(0),
                key(&[1]),
                Some(WindowField::Table(1))
            ));
            while let Some((due_hour, id, owner)) = context.timers.pop_due(hour)?.map(|timer| {
                (
                    timer.hour,
                    timer.id.varint().to_vec(),
                    timer.key.varint().to_vec(),
                )
            }) {
                let timer = Timer {
                    hour: due_hour,
                    id: key(&id),
                    key: key(&owner),
                };
                assert!(at.scope.timer_due(hour, timer));
                assert!(at.scope.keeps(
                    hour,
                    DeclarationId(0),
                    timer.key,
                    Some(WindowField::Table(0))
                ));
                self.delivered.push((hour, owner.clone()));
                let due = [timer];
                at.scope.hour(
                    family,
                    &crate::tests::at(
                        hour,
                        &[Event::Tick {
                            hour,
                            starts_day: false,
                            due: &due,
                        }],
                    ),
                    &mut context,
                    out,
                )?;
            }
        }
        assert!(context.timers.is_empty());
        let boundary = at
            .scope
            .end(family, &whole_end(at.scope.hours()), &mut context, out)?;
        let Boundary::ClassB {
            summary,
            kind: SummaryKind::Resolved,
        } = &boundary
        else {
            return Err(io::ErrorKind::InvalidData.into());
        };
        for item in at.scope.keys() {
            if summary.value(item.key, ProductFieldId(3))?
                != at.expected_at_close.value(item.key, ProductFieldId(3))?
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
        }
        Ok(boundary)
    }
}

#[test]
fn replay_batches_two_different_closes_and_merges_the_third_keys_transfer_without_restoring_it() {
    let mut declarations = window_subscription().declarations().to_vec();
    let Declaration::Provisional(rule) = &mut declarations[2] else {
        unreachable!()
    };
    rule.bound = WindowBound::UntilClosed;
    for input in Arc::make_mut(&mut rule.inputs) {
        input.keys = WindowKeys::All;
    }
    for output in Arc::make_mut(&mut rule.outputs) {
        output.keys = WindowKeys::All;
    }
    let subscription = Subscription::new(Arc::from(declarations)).unwrap();
    let segment = interval(10, 19);
    let keys = [
        ReplayKey {
            key: key(&[1]),
            extent: WindowExtent {
                hours: interval(10, 11),
                end: WindowEnd::Closed,
            },
        },
        ReplayKey {
            key: key(&[2]),
            extent: WindowExtent {
                hours: interval(10, 14),
                end: WindowEnd::Closed,
            },
        },
    ];
    let mut closures = FixtureContext::default();
    closures.rows.hour = 11;
    closures.close_window(DeclarationId(2), key(&[1])).unwrap();
    closures.rows.hour = 14;
    closures.close_window(DeclarationId(2), key(&[2])).unwrap();
    let rows = [
        WindowDependency {
            hours: interval(10, 11),
            key: key(&[1]),
            field: ProductFieldId(3),
            target: DependencyTarget::Publication,
        },
        WindowDependency {
            hours: interval(10, 11),
            key: key(&[1]),
            field: ProductFieldId(3),
            target: DependencyTarget::OutgoingSummary,
        },
        WindowDependency {
            hours: interval(10, 14),
            key: key(&[2]),
            field: ProductFieldId(3),
            target: DependencyTarget::Publication,
        },
        WindowDependency {
            hours: interval(10, 14),
            key: key(&[2]),
            field: ProductFieldId(3),
            target: DependencyTarget::OutgoingSummary,
        },
    ];
    let dependencies = SliceWindowDependencies::new(&rows).unwrap();
    let scope = ReplayScope::new(
        segment,
        DeclarationId(2),
        &keys,
        &dependencies,
        &subscription,
        &closures,
    )
    .unwrap();
    assert_eq!(scope.hours(), interval(10, 14));
    let incoming = ShardSummary(
        [
            (vec![1], [10, 0, 11]),
            (vec![2], [20, 0, 14]),
            (vec![3], [30, 0, 12]),
        ]
        .into(),
    );
    let expected = ShardSummary([(vec![1], [12, 0, 11]), (vec![2], [25, 0, 14])].into());
    let transfer = ShardSummary(
        [
            (vec![1], [100, 91, 11]),
            (vec![2], [200, 92, 14]),
            (vec![3], [300, 93, 12]),
        ]
        .into(),
    );
    let mut family = ShardFamily {
        subscription: subscription.clone(),
        summary: ShardSummary::default(),
        restores: Vec::new(),
        delivered: Vec::new(),
    };
    let mut replay = ShardReplay {
        restored_timers: 0,
        delivered: Vec::new(),
    };
    let mut out = CheckedPublication::default();
    let reconciliation = Reconciliation {
        scope,
        incoming: &incoming,
        expected_at_close: &expected,
    };
    let Boundary::ClassB {
        summary,
        kind: SummaryKind::Resolved,
    } = replay
        .replay(
            &mut family,
            &reconciliation,
            &mut ReplayPublication::new(scope, &mut out),
        )
        .unwrap()
    else {
        panic!("missing replay")
    };
    let merged = ReconciledSummary {
        dependencies: &dependencies,
        replayed: summary,
        transfer: &transfer,
    };
    assert_eq!(shard_value(&merged, key(&[1]), 3).unwrap(), 12);
    assert_eq!(shard_value(&merged, key(&[2]), 3).unwrap(), 25);
    assert_eq!(shard_value(&merged, key(&[3]), 3).unwrap(), 300);
    for (key, later) in [(key(&[1]), 91), (key(&[2]), 92), (key(&[3]), 93)] {
        assert_eq!(shard_value(&merged, key, 5).unwrap(), later);
    }
    assert_eq!(replay.restored_timers, 2);
    assert_eq!(replay.delivered, [(11, vec![1]), (14, vec![2])]);
    assert_eq!(family.restores, [vec![1], vec![2]]);
    assert_eq!(family.delivered, replay.delivered);
    assert_eq!(out.records, 2);
    let record = PublicationRecord {
        hour: 11,
        key: key(&[1]),
        field: ProductFieldId(3),
        change: PublicationChange::Record(PublicationValue::End),
        provenance: Provenance::default(),
        breaks: false,
    };
    let mut guarded = ReplayPublication::new(scope, &mut out);
    assert!(
        guarded
            .publish(PublicationRecord {
                key: key(&[3]),
                ..record
            })
            .is_err()
    );
    assert!(
        guarded
            .publish(PublicationRecord {
                field: ProductFieldId(5),
                ..record
            })
            .is_err()
    );
    assert!(
        guarded
            .publish(PublicationRecord { hour: 12, ..record })
            .is_err()
    );
    assert!(
        guarded
            .coverage(Coverage {
                hours: segment,
                state: CoverageState::Accepted
            })
            .is_err()
    );
    let mut context = FixtureContext::default();
    assert!(
        ReplayContext::new(scope, &mut context)
            .schedule(timer(12, &[3]))
            .is_err()
    );
    assert!(
        ReplayContext::new(scope, &mut context)
            .exact(TableId::Countries, key(&[3]), 0)
            .is_err()
    );
    let due = [timer(12, &[3])];
    context.rows.hour = 12;
    assert!(
        scope
            .hour(
                &mut family,
                &at(
                    12,
                    &[Event::Tick {
                        hour: 12,
                        starts_day: false,
                        due: &due
                    }]
                ),
                &mut context,
                &mut guarded
            )
            .is_err()
    );
    context.rows.hour = 12;
    assert!(
        ReplayContext::new(scope, &mut context)
            .exact(TableId::Countries, key(&[1]), 0)
            .is_err()
    );
    assert_eq!(out.records, 2);
    let mut unsorted = rows;
    unsorted.swap(0, 1);
    assert!(SliceWindowDependencies::new(&unsorted).is_err());
    assert!(
        !dependencies
            .contains(
                key(&[3]),
                ProductFieldId(3),
                DependencyTarget::OutgoingSummary
            )
            .unwrap()
    );
    assert!(
        !dependencies
            .contains(
                key(&[1]),
                ProductFieldId(5),
                DependencyTarget::OutgoingSummary
            )
            .unwrap()
    );
}
