use collector_observers::market_changes::{self as producer, held};
use collector_projection::{projection::Projection, table_conversion::unavailable};
use engine_native::{Memory, walk::Outcome};
use replay_data::rows::{MarketEventRow, TableId, TableRows};
use replay_frame::frame::{TableInput, TableView};
use std::mem::size_of;
pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::CountryMarket,
    TableId::MarketContractTerms,
    TableId::LendLeaseAgreements,
    TableId::MarketEvents,
];
pub type Capacities = usize;
#[cfg(test)]
#[path = "market/native_admission_tests.rs"]
mod native_admission_tests;
pub struct Captured {
    events: Vec<MarketEventRow>,
    outcomes: [Outcome; N],
    changes: std::sync::Mutex<Option<held::Changes>>,
    refusal: Option<&'static str>,
    event_refusal: Option<&'static str>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            events: engine_allocation::owned_vec(engine_allocation::vec(*capacity)),
            outcomes: [Outcome::Unavailable {
                reason: "market capture not supplied",
            }; N],
            changes: std::sync::Mutex::new(None),
            refusal: Some("market capture not supplied"),
            event_refusal: None,
        }
    }
    pub fn reset(&mut self) {
        self.events.clear();
        self.outcomes = [Outcome::Unavailable {
            reason: "market capture not supplied",
        }; N];
        *self
            .changes
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.refusal = Some("market capture not supplied");
        self.event_refusal = None;
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        *capacity = (*capacity).max(producer::capacity());
    }
    pub fn allocated_bytes(&self) -> usize {
        self.events.capacity() * size_of::<MarketEventRow>()
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.refusal = Some(reason);
        self.event_refusal = Some(reason);
        self.outcomes = [Outcome::Unavailable { reason }; N];
        *self
            .changes
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    /// # Safety
    /// Caller must hold hourly capture admission as the sole consumer of market events while
    /// observer storage stays live and teardown is excluded.
    pub unsafe fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        _state: usize,
        hour: i32,
        full: bool,
    ) -> [u64; N] {
        self.reset();
        let begin = crate::clock::qpc();
        // SAFETY: Caller holds hourly capture admission as the sole market-events consumer.
        self.event_refusal = unsafe { producer::drain(&mut self.events) }.err();
        let full = full || held::full_publication_required();
        if !full && let Some(reason) = self.event_refusal {
            held::refuse(reason);
        }
        let events = if self.event_refusal.is_none() {
            &self.events[..]
        } else {
            &[]
        };
        match held::acquire(memory, base, events, hour, full) {
            Ok(mut changes) if self.event_refusal.is_none() || full => {
                self.events.clear();
                if let Err(reason) = producer::publish(&mut changes.events) {
                    self.event_refusal = Some(reason);
                }
                *self
                    .changes
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(changes);
                self.refusal = None;
            }
            Ok(_) => self.refusal = self.event_refusal,
            Err(reason) => {
                self.refusal = Some(reason);
                let mut missing = Vec::new();
                held::unavailable_amounts(&self.events, &mut missing);
                self.events.extend(missing);
                if let Err(reason) = producer::publish(&mut self.events) {
                    self.event_refusal = Some(reason);
                }
                *self
                    .changes
                    .get_mut()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(held::Changes {
                    owners: [Vec::new(), Vec::new()],
                    agreement_keys: Vec::new(),
                    contract_keys: Vec::new(),
                    countries: Vec::new(),
                    contracts: Vec::new(),
                    agreements: Vec::new(),
                    refusals: [Some(reason); 3],
                    events: std::mem::take(&mut self.events),
                    full,
                    countries_full: full,
                });
            }
        }
        self.outcomes = self.observed_outcomes();
        [
            0,
            0,
            0,
            crate::clock::qpc().saturating_sub(begin).max(0) as u64,
        ]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        self.outcomes
    }
    fn observed_outcomes(&self) -> [Outcome; N] {
        let held = self
            .changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(changes) = held.as_ref() else {
            return [Outcome::Unavailable {
                reason: self.refusal.unwrap_or("market conversion already consumed"),
            }; N];
        };
        let lengths = [
            changes.countries.len(),
            changes.contracts.len(),
            changes.agreements.len(),
        ];
        let mut outcomes = [Outcome::Rows { len: 0 }; N];
        for index in 0..3 {
            outcomes[index] = changes.refusals[index].map_or(
                Outcome::Rows {
                    len: lengths[index],
                },
                |reason| Outcome::Unavailable { reason },
            );
        }
        outcomes[3] = self.event_refusal.map_or(
            Outcome::Rows {
                len: changes.events.len(),
            },
            |reason| Outcome::Unavailable { reason },
        );
        outcomes
    }
    pub fn table_inputs(&self, projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let changes = self
            .changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(changes) = changes else {
            let state = self.refusal.unwrap_or("market conversion already consumed");
            let events = self.event_refusal.unwrap_or("market events not supplied");
            return std::array::from_fn(|index| {
                unavailable(if index == 3 { events } else { state }).into()
            });
        };
        let [countries, _] = changes.owners;
        let first = |rows, owners, refusal| match refusal {
            Some(reason) => unavailable(reason).into(),
            None => match projection.as_deref() {
                Some(projection) => {
                    projection.certified_owners(rows, owners, changes.countries_full)
                }
                None if changes.countries_full => {
                    TableInput::SharedComplete(std::sync::Arc::new(rows))
                }
                None => unavailable("market conversion has no baseline projection").into(),
            },
        };
        let country = first(
            TableRows::CountryMarket(changes.countries),
            countries,
            changes.refusals[0],
        );
        let contracts = match changes.refusals[1] {
            Some(reason) => unavailable(reason).into(),
            None => match projection.as_deref() {
                Some(projection) => projection.certified_keys(
                    TableRows::MarketContractTerms(changes.contracts),
                    changes.contract_keys,
                    changes.full,
                ),
                None if changes.full => TableInput::SharedComplete(std::sync::Arc::new(
                    TableRows::MarketContractTerms(changes.contracts),
                )),
                None => unavailable("market conversion has no baseline projection").into(),
            },
        };
        let agreements = match changes.refusals[2] {
            Some(reason) => unavailable(reason).into(),
            None => match projection.as_deref() {
                Some(projection) => projection.certified_keys(
                    TableRows::LendLeaseAgreements(changes.agreements),
                    changes.agreement_keys,
                    changes.full,
                ),
                None if changes.full => TableInput::SharedComplete(std::sync::Arc::new(
                    TableRows::LendLeaseAgreements(changes.agreements),
                )),
                None => unavailable("market conversion has no baseline projection").into(),
            },
        };
        let events = match self.event_refusal {
            Some(reason) => unavailable(reason),
            None => TableView::Complete(TableRows::MarketEvents(changes.events)),
        };
        [country, contracts, agreements, events.into()]
    }
    pub fn tables(&self) -> [TableView; N] {
        std::array::from_fn(|_| unavailable("market tables require certified owner conversion"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_cold_country_header_stays_named_after_census_completion() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        struct NoRetry;
        impl Memory for NoRetry {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("failed shared bootstrap must not be retried");
            }
        }
        let _session = unsafe { producer::test_support::Session::new() };
        unsafe { producer::boundary(
            Some(engine_native::cgame_state::Clock {
                hour: 100,
                ..Default::default()
            }),
            false,
        ) };
        held::discover_country(
            &Unreadable,
            0,
            0x100,
            engine_native::ccountry::Country {
                pointer: 0x1000,
                id: 5000,
                canonical_slot: 5000,
            },
            0,
        );
        held::finish_seed(true);
        held::recover_contracts(&NoRetry, 0, 0x100);
        let mut pack = Captured::vacant();
        unsafe { pack.capture(&NoRetry, 0, 0x100, 100, false) };
        for outcome in &pack.outcomes()[..3] {
            assert_eq!(
                *outcome,
                Outcome::Unavailable {
                    reason: "market bootstrap manager unreadable"
                }
            );
        }
        for input in pack.table_inputs(None) {
            let TableInput::Sampled(TableView::Unavailable { reason }) = input else {
                panic!("failed native bootstrap must stay a named refusal");
            };
            assert!(!reason.is_empty());
        }
    }

    #[test]
    fn attempted_contract_census_failure_is_never_retried_in_the_same_epoch() {
        struct NoRetry;
        impl Memory for NoRetry {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("an attempted contract census must not read native memory again");
            }
        }
        struct FailedHeader(std::cell::Cell<usize>);
        impl Memory for FailedHeader {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                if at == 0x100 + engine_native::cgame_state::MARKET_MANAGER && output.len() == 8 {
                    output.copy_from_slice(&0x1000_u64.to_le_bytes());
                    true
                } else {
                    assert_eq!(
                        at,
                        0x1000 + engine_production::market_contract_manager::OWNED_CONTRACTS
                    );
                    assert_eq!(output.len(), 16);
                    false
                }
            }
        }
        for daily in [true, false] {
            let _session = unsafe { producer::test_support::Session::new() };
            unsafe { producer::boundary(
                Some(engine_native::cgame_state::Clock {
                    hour: 100,
                    ..Default::default()
                }),
                false,
            ) };
            if daily {
                held::finish_contract_seed(false);
            }
            held::finish_seed(true);
            if !daily {
                let memory = FailedHeader(std::cell::Cell::new(0));
                held::recover_contracts(&memory, 0, 0x100);
                assert_eq!(memory.0.get(), 2);
            }
            held::recover_contracts(&NoRetry, 0, 0x100);
            held::recover_contracts(&NoRetry, 0, 0x100);
            let mut pack = Captured::vacant();
            unsafe { pack.capture(&NoRetry, 0, 0x100, 100, false) };
            assert_eq!(pack.outcomes()[0], Outcome::Rows { len: 0 });
            assert!(matches!(pack.outcomes()[1], Outcome::Unavailable { .. }));
            assert_eq!(pack.outcomes()[2], Outcome::Rows { len: 0 });
        }
    }

    #[test]
    fn cold_attach_and_post_gap_censuses_admit_every_column_despite_event_loss() {
        use collector_projection::projection::ProjectionScratch;
        use replay_data::rows::{CountryMarketRow, LendLeaseAgreementRow, MarketContractTermsRow};
        use replay_frame::frame::{FrameDecoder, FrameEncoder, FrameInput, Metrics};
        struct NoReads;
        impl Memory for NoReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("quiet certified state must not poll native holders");
            }
        }
        for populated in [false, true] {
            let session = unsafe { producer::test_support::Session::new() };
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::for_tables(&TABLES);
            let scratch = ProjectionScratch::default();
            for hour in 100..=104 {
                if hour == 102 {
                    session.lose_interval();
                    held::refuse("event ledger observation invalidated");
                    encoder = FrameEncoder::new();
                    decoder = FrameDecoder::for_tables(&TABLES);
                }
                unsafe { producer::boundary(
                    Some(engine_native::cgame_state::Clock {
                        hour,
                        ..Default::default()
                    }),
                    false,
                ) };
                let reseeded = held::full_publication_required();
                assert_eq!(reseeded, matches!(hour, 100 | 102));
                if reseeded {
                    let count = usize::from(populated);
                    if populated {
                        session.seed([
                            TableRows::CountryMarket(vec![
                                CountryMarketRow {
                                    country: 5000,
                                    subsidy_total_raw: i64::from(hour),
                                    ..Default::default()
                                };
                                count
                            ]),
                            TableRows::MarketContractTerms(vec![
                                MarketContractTermsRow {
                                    country: 5000,
                                    id: 17,
                                    kind: 1,
                                    speed: hour,
                                    ..Default::default()
                                };
                                count
                            ]),
                            TableRows::LendLeaseAgreements(vec![
                                LendLeaseAgreementRow {
                                    country: 5000,
                                    id: 17,
                                    fuel_sent_raw: i64::from(hour),
                                    ..Default::default()
                                };
                                count
                            ]),
                        ]);
                    } else {
                        held::finish_seed(true);
                        struct EmptyContracts;
                        impl Memory for EmptyContracts {
                            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                                if at == 0x100 + engine_native::cgame_state::MARKET_MANAGER
                                    && output.len() == 8
                                {
                                    output.copy_from_slice(&0x1000_u64.to_le_bytes());
                                } else if at == 0x1000
                                    + engine_production::market_contract_manager::OWNED_CONTRACTS
                                    && output.len() == 16
                                {
                                    output.fill(0);
                                } else {
                                    panic!("recovery read outside demanded contract census");
                                }
                                true
                            }
                        }
                        held::recover_contracts(&EmptyContracts, 0, 0x100);
                        held::recover_contracts(&NoReads, 0, 0x100);
                    }
                } else if populated {
                    session.emit(MarketEventRow {
                        country: 5000,
                        agreement: 17,
                        kind: producer::FUEL,
                        occurrence: hour as u64,
                        hour,
                        amount_raw: 7,
                        ..Default::default()
                    });
                }
                let mut pack = Captured::vacant();
                unsafe { pack.capture(&NoReads, 0, 0, hour, false) };
                for outcome in &pack.outcomes()[..3] {
                    assert!(
                        matches!(outcome, Outcome::Rows { len } if !reseeded || *len == usize::from(populated)),
                        "hour {hour}: {outcome:?}"
                    );
                }
                let event_loss = matches!(hour, 100 | 102);
                assert_eq!(
                    matches!(pack.outcomes()[3], Outcome::Unavailable { .. }),
                    event_loss
                );
                let mut projection = Projection::new(&encoder, 1, hour, false, &scratch);
                let inputs = pack.table_inputs(Some(&mut projection));
                let mut frame = FrameInput {
                    branch: 1,
                    hour,
                    tables: std::array::from_fn(|_| unavailable("unrelated table").into()),
                    metrics: Metrics::default(),
                };
                for (table, input) in TABLES.into_iter().zip(inputs) {
                    frame.tables[table.index()] = input;
                }
                let wire = encoder.encode_bytes(frame, false).unwrap();
                let frame = wire.decode().unwrap();
                assert_eq!(frame.keyframe, matches!(hour, 100 | 102));
                replay_capture_spool::spool::encode_record(&replay_capture_spool::spool::Record::Frame(frame.clone())).unwrap();
                let decoded = decoder.apply(&frame).unwrap();
                for table in TABLES {
                    match &*decoded.tables[table.index()] {
                        TableView::Complete(rows) => {
                            assert!(table != TableId::MarketEvents || !event_loss);
                            if table == TableId::MarketEvents {
                                assert_eq!(rows.len(), usize::from(populated), "hour {hour}");
                            }
                            let mut bytes = Vec::new();
                            rows.encode_rows(&mut bytes).unwrap();
                            for row in rows.iter() {
                                assert_eq!(row.not_recorded_fields(), 0, "{table:?}");
                                row.visit_nested_field_gaps(&mut |field| {
                                    panic!("{table:?}.{field} is missing")
                                });
                            }
                        }
                        TableView::Unavailable { reason } => {
                            assert_eq!(table, TableId::MarketEvents);
                            assert!(event_loss && !reason.is_empty());
                        }
                        view => {
                            panic!("first-hour admission is neither recorded nor named: {view:?}")
                        }
                    }
                }
                if frame.keyframe {
                    let sought = FrameDecoder::for_tables(&TABLES).apply(&frame).unwrap();
                    for table in &TABLES[..3] {
                        assert!(matches!(
                            &*sought.tables[table.index()],
                            TableView::Complete(_)
                        ));
                    }
                }
            }
        }
    }

    #[test]
    fn complete_occurrences_and_named_amount_refusals_survive_periodic_and_forced_keyframes() {
        use replay_data::rows::tables;
        use replay_frame::{
            KEYFRAME_INTERVAL,
            frame::{FrameDecoder, FrameEncoder, FrameInput, Metrics},
        };
        let source = [
            MarketEventRow {
                id: 1,
                country: 5000,
                agreement: 17,
                kind: producer::FUEL,
                amount_raw: 123,
                ..Default::default()
            },
            MarketEventRow {
                id: 2,
                country: 5000,
                agreement: 17,
                kind: producer::CONTRACT_PAYMENT,
                amount_raw: 456,
                ..Default::default()
            },
            MarketEventRow {
                id: 3,
                country: 5000,
                agreement: 17,
                kind: producer::REMOVED,
                ..Default::default()
            },
            MarketEventRow {
                id: 4,
                country: 5000,
                agreement: 18,
                kind: producer::DELIVERY_UNAVAILABLE,
                mode: producer::DELIVERED,
                ordinal: 1,
                ..Default::default()
            },
        ];
        let input = |hour, events: TableInput| {
            let mut frame = FrameInput {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| unavailable("fixture table").into()),
            };
            frame.tables[TableId::MarketEvents.index()] = events;
            frame
        };
        for (hour, force) in [(1, true), (KEYFRAME_INTERVAL as i32, false)] {
            let source: Vec<_> = source
                .iter()
                .map(|row| MarketEventRow { hour, ..*row })
                .collect();
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::for_tables(&[TableId::MarketEvents]);
            for previous in if hour == 1 {
                vec![0]
            } else {
                vec![0, hour - 1]
            } {
                let wire = encoder
                    .encode_bytes(
                        input(
                            previous,
                            TableView::Complete(TableRows::MarketEvents(Vec::new())).into(),
                        ),
                        false,
                    )
                    .unwrap();
                decoder.apply(&wire.decode().unwrap()).unwrap();
            }
            let mut pack = Captured::vacant();
            pack.refusal = None;
            *pack.changes.get_mut().unwrap() = Some(held::Changes {
                owners: [Vec::new(), Vec::new()],
                agreement_keys: Vec::new(),
                contract_keys: Vec::new(),
                refusals: [None, None, Some("synthetic required total read failure")],
                events: source.clone(),
                countries: Vec::new(),
                contracts: Vec::new(),
                agreements: Vec::new(),
                full: false,
                countries_full: false,
            });
            pack.outcomes = pack.observed_outcomes();
            assert_eq!(pack.outcomes()[3], Outcome::Rows { len: 4 });
            let events = pack.table_inputs(None).into_iter().nth(3).unwrap();
            let wire = encoder.encode_bytes(input(hour, events), force).unwrap();
            let frame = wire.decode().unwrap();
            assert!(frame.keyframe);
            let decoded = decoder.apply(&frame).unwrap();
            assert_eq!(
                decoded.complete::<tables::MarketEvents>().unwrap(),
                source.as_slice()
            );
            let sought = FrameDecoder::for_tables(&[TableId::MarketEvents])
                .apply(&frame)
                .unwrap();
            assert_eq!(
                sought.complete::<tables::MarketEvents>().unwrap(),
                source.as_slice()
            );
            assert!(
                sought.complete::<tables::MarketEvents>().unwrap()[3]
                    .delivery_unavailable_reason()
                    .unwrap()
                    .contains("unreadable")
            );
            let next = encoder
                .encode_bytes(
                    input(
                        hour + 1,
                        TableView::Complete(TableRows::MarketEvents(Vec::new())).into(),
                    ),
                    false,
                )
                .unwrap()
                .decode()
                .unwrap();
            assert!(
                decoder
                    .apply(&next)
                    .unwrap()
                    .complete::<tables::MarketEvents>()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    #[test]
    fn missing_amounts_are_explicit_rows_in_a_complete_occurrence_ledger() {
        let mut pack = Captured::vacant();
        pack.refusal = None;
        *pack.changes.get_mut().unwrap() = Some(held::Changes {
            owners: [Vec::new(), Vec::new()],
            agreement_keys: Vec::new(),
            contract_keys: Vec::new(),
            refusals: [None, None, Some("synthetic total read refusal")],
            events: vec![
                MarketEventRow {
                    id: 1,
                    country: 5000,
                    agreement: 17,
                    kind: producer::FUEL,
                    amount_raw: 123,
                    ..Default::default()
                },
                MarketEventRow {
                    id: 2,
                    country: 5000,
                    agreement: 18,
                    kind: producer::DELIVERY_UNAVAILABLE,
                    mode: producer::DELIVERED,
                    ordinal: 1,
                    ..Default::default()
                },
            ],
            countries: Vec::new(),
            contracts: Vec::new(),
            agreements: Vec::new(),
            full: true,
            countries_full: true,
        });
        pack.outcomes = pack.observed_outcomes();
        assert_eq!(pack.outcomes()[3], Outcome::Rows { len: 2 });
        let [_, _, _, events] = pack.table_inputs(None);
        let TableInput::Sampled(TableView::Complete(TableRows::MarketEvents(rows))) = events else {
            panic!("holder refusal rows must keep the occurrence ledger complete");
        };
        assert_eq!(rows[0].amount_raw, 123);
        assert_eq!(rows[1].agreement, 18);
        assert!(
            rows[1]
                .delivery_unavailable_reason()
                .unwrap()
                .contains("unreadable")
        );
    }

    #[test]
    fn conversion_moves_occurrences_once_and_isolates_state_refusals() {
        let mut pack = Captured::vacant();
        pack.refusal = None;
        let events = vec![MarketEventRow {
            id: 17,
            kind: producer::CONTRACT_CREATED,
            ..Default::default()
        }];
        let allocation = events.as_ptr();
        *pack.changes.get_mut().unwrap() = Some(held::Changes {
            owners: [Vec::new(), Vec::new()],
            agreement_keys: Vec::new(),
            contract_keys: Vec::new(),
            refusals: [None, Some("synthetic contract refusal"), None],
            events,
            countries: Vec::new(),
            contracts: Vec::new(),
            agreements: Vec::new(),
            full: true,
            countries_full: true,
        });
        pack.outcomes = pack.observed_outcomes();
        let outcomes = pack.outcomes();
        assert!(matches!(pack.outcomes()[1], Outcome::Unavailable { .. }));
        assert_eq!(pack.outcomes()[3], Outcome::Rows { len: 1 });
        let [country, contract, agreements, events] = pack.table_inputs(None);
        assert!(matches!(country, TableInput::SharedComplete(_)));
        assert!(matches!(agreements, TableInput::SharedComplete(_)));
        assert!(matches!(
            contract,
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        let TableInput::Sampled(TableView::Complete(TableRows::MarketEvents(events))) = events
        else {
            panic!("occurrence table refused with contract state");
        };
        assert_eq!(
            events.as_ptr(),
            allocation,
            "conversion cloned owned changes"
        );
        assert!(pack.changes.lock().unwrap().is_none());
        assert_eq!(
            pack.outcomes(),
            outcomes,
            "spool outcomes must survive consuming the owned pack"
        );
    }
}
