use super::*;
use crate::products::equipment_losses::columnar::{self, TABLES};
use dashboard_base::prepared::series::tests::{Fixture, assert_payloads_equal};
use dashboard_core::test_fixtures::{CAMPAIGN, named_frame, records, scale_frame};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, 3);
    let named = named_frame(hour, Some(b"recorded design"));
    for table in [
        TableId::LexicalTokens,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::VariantChanges,
    ] {
        frame.tables[table.index()] = named.tables[table.index()].clone();
    }
    frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(Vec::new())).into();
    frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 17_u64 << 32 | u64::from(VARIANT_TYPE),
            equipment_token: 1,
            creator_link: 100_000,
            display_name: b"recorded design".to_vec(),
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![
            EquipmentDestructionRow {
                country: 100_000,
                cause: EquipmentDestructionCause::LandCombat,
                variant: 17_u64 << 32 | u64::from(VARIANT_TYPE),
                amount_raw: 123_456,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::CombatLogEquipment.index()] =
        TableView::Complete(TableRows::CombatLogEquipment(vec![CombatLogEquipmentRow {
            country: 100_000,
            group: 1,
            kind: 0,
            month: 8,
            variant: 17,
            amount_raw: 345_678,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::CombatLogState.index()] =
        TableView::Complete(TableRows::CombatLogState(vec![CombatLogStateRow {
            viewing_country_raw: 0,
            prune_observed: 1,
        }]))
        .into();
    frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: engine_dlc::dlc_feature_state::EQUIPMENT_LOSSES_MASK,
            ..Default::default()
        }]))
        .into();
    frame.tables[TableId::Players.index()] =
        TableView::Complete(TableRows::Players(Vec::new())).into();
    frame.tables[TableId::DivisionCensus.index()] = TableView::Complete(TableRows::DivisionCensus(
        (0..3)
            .map(|n| DivisionCensusRow {
                country: 100_000 + n,
                grouped: 1,
                ungrouped: 0,
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::CombatLogEquipment as u32,
            owner: 100_000,
            sample_hour: hour,
        }]))
        .into();
    frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
            entries: vec![SunkShipEntry {
                ship_name: b"retained ship".to_vec(),
                victim_country: 100_000,
                sink_hour: 7,
                victim_equipment_variant: 17_u64 << 32 | u64::from(VARIANT_TYPE),
                ..Default::default()
            }],
        }]))
        .into();
    frame.tables[TableId::SunkConvoys.index()] =
        TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
            owner_country: 100_000,
            killer_country: 100_001,
            month: 8,
            convoys: 5,
            ordinal: 0,
        }]))
        .into();
    frame.tables[TableId::LendLeaseEquipment.index()] =
        TableView::Complete(TableRows::LendLeaseEquipment(vec![LendLeaseEquipmentRow {
            giver: 100_000,
            receiver: 100_001,
            exchange: 91,
            kind: 1,
            variant: 17,
            amount_raw: 567_890,
        }]))
        .into();
    for &table in TABLES {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0 && carried != Some(T::ID);
    let sample = if read {
        Sample::FromReadBit
    } else {
        Sample::Carried(frame.hour - 1)
    };
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    macro_rules! tables { ($($table:ident),* $(,)?) => { $(write::<tables::$table>(walk, frame, carried);)* }; }
    tables!(
        Campaign,
        Countries,
        CountryLinks,
        CombatLogEquipment,
        CombatLogState,
        DivisionCensus,
        EquipmentDestruction,
        LendLeaseEquipment,
        Players,
        SimulationFeatures,
        SliceReads,
        SunkConvoys,
        SunkShips,
        CountryAppearance,
        LexicalTokens,
        VariantDefinitions,
        VariantModules,
        VariantUpgrades,
        VariantChanges
    );
}

fn flush(prepared: &mut Prepared, fixture: &Fixture) {
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}

/// Frames walked as the recording's segments are, each projected by the segment's operator and
/// kept in a branch's state: a gap starts a run, and [`Self::restart`] a segment.
pub(super) struct Driver<'r> {
    replay: &'r Replay,
    projector: columnar::Projector,
    walk: Synthetic,
    world: Option<dashboard_world::World>,
    last: Option<i32>,
    ordinal: u32,
    run: u32,
    restart: bool,
}

impl<'r> Driver<'r> {
    pub(super) fn new(replay: &'r Replay, first: i32) -> io::Result<Self> {
        let mut projector = columnar::Projector::new(replay)?;
        projector.start(Start::Run { continues: false }, 1, first)?;
        Ok(Self {
            replay,
            projector,
            walk: Synthetic::new(TABLES, 1, first)?,
            world: None,
            last: None,
            ordinal: 0,
            run: 0,
            restart: false,
        })
    }

    /// The next frame starts a segment, as at a restart or a resumed branch.
    pub(super) fn restart(&mut self) {
        self.restart = true;
    }

    /// Projects `frame` with `carried` taken from the hour before, and keeps it in `prepared`.
    pub(super) fn observe_carried(
        &mut self,
        prepared: &mut Prepared,
        frame: &DecodedFrame,
        carried: Option<TableId>,
    ) -> io::Result<()> {
        let run = self
            .last
            .is_some_and(|last| last.checked_add(1) != Some(frame.hour));
        let opens = self.last.is_none() || run || self.restart;
        if self.last.is_some() && opens {
            self.projector = columnar::Projector::new(self.replay)?;
            let start = if run {
                self.run += 1;
                Start::Run { continues: true }
            } else {
                Start::Restart(self.walk.state())
            };
            assert!(self.projector.starts_at(start));
            self.projector.start(start, 1, frame.hour)?;
            if run {
                self.walk = Synthetic::new(TABLES, 1, frame.hour)?;
            }
        }
        self.restart = false;
        write_hour(&mut self.walk, frame, carried);
        let mut hours: Vec<columnar::Hour> = Vec::new();
        let (projector, world_held, run, ordinal) = (
            &mut self.projector,
            &mut self.world,
            self.run,
            self.ordinal,
        );
        self.walk.end(|state, manifest, frame, starts| -> io::Result<()> {
            let world = dashboard_world::World::at(
                &mut *world_held,
                starts,
                &columnar::needs(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )?;
            projector.hour(
                &segments::Hour {
                    branch: 1,
                    hour: frame.hour,
                    run,
                    ordinal,
                    starts,
                    opens,
                    state,
                    manifest,
                    world,
                },
                &mut hours,
            )
        })??;
        self.last = Some(frame.hour);
        self.ordinal += 1;
        for hour in hours {
            prepared.observe_projected(hour)?;
        }
        Ok(())
    }

    pub(super) fn observe(
        &mut self,
        prepared: &mut Prepared,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        self.observe_carried(prepared, frame, None)
    }
}

/// A run's state and published sections.
type Output = (Prepared, Fixture);

/// The walk's state and sections, publishing at every third hour, with a segment starting at
/// the frame `restart`.
fn projected(
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<(i32, TableId)>,
) -> Output {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
    let fixture = Fixture::new();
    let mut prepared = Prepared::new(&replay, 1, inputs[0].hour);
    let mut driver = Driver::new(&replay, inputs[0].hour).unwrap();
    for (ordinal, input) in inputs.iter().enumerate() {
        if restart == Some(ordinal) {
            driver.restart();
        }
        driver
            .observe_carried(
                &mut prepared,
                input,
                carried
                    .filter(|(at, _)| *at == input.hour)
                    .map(|(_, table)| table),
            )
            .unwrap();
        if input.hour % 3 == 0 {
            flush(&mut prepared, &fixture);
        }
    }
    flush(&mut prepared, &fixture);
    (prepared, fixture)
}

/// The walk in its runs alone, then with a segment starting at `restart`: both publish the same.
fn parity(
    label: &str,
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<(i32, TableId)>,
) -> [Output; 2] {
    let whole = projected(inputs, None, carried);
    let restarted = projected(inputs, restart, carried);
    let expected = whole.1.payloads();
    for prefix in [
        "equipment-loss-1-catalogue",
        "equipment-loss-values-1",
        "equipment-loss-country-1-",
        "equipment-loss-variant-1-",
    ] {
        assert!(
            expected.keys().any(|(name, _)| name.starts_with(prefix)),
            "{label}: {prefix}"
        );
    }
    assert_payloads_equal(&restarted.1.payloads(), &expected, label);
    assert_eq!(
        serde_json::to_value(&restarted.0.absent).unwrap(),
        serde_json::to_value(&whole.0.absent).unwrap(),
        "{label}: absent"
    );
    [whole, restarted]
}

/// A leaf's published total points, and its canonical runs and events.
fn published(
    (prepared, fixture): &Output,
    identity: Identity,
    leaf: LeafKey,
) -> (Vec<LossPoint>, u64, u64) {
    let country = &prepared.countries[&identity];
    let local = country.leaves[&leaf];
    let actor = country.names[local as usize];
    let flag = AtomicBool::new(false);
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        "equipment-loss-values-1".into(),
        &flag,
    )
    .unwrap();
    let points: Vec<LossPoint> = reader
        .events(&[actor], i32::MIN, i32::MAX, 0, 1024)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| serde_json::from_slice(&row.row).unwrap())
        .collect();
    let state = country.values.state(local).unwrap();
    (points, state.runs, state.events)
}

fn every_restart(inputs: &[DecodedFrame]) -> impl Iterator<Item = Option<usize>> {
    std::iter::once(None).chain((1..inputs.len()).map(Some))
}

#[test]
fn local_drains_and_lifetime_maxima_match_at_every_restart() {
    let mut inputs: Vec<_> = (10..20).map(frame).collect();
    if let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
        &mut *inputs[2].tables[TableId::CombatLogEquipment.index()]
    {
        rows[0].amount_raw = -12;
    }
    inputs[3].tables[TableId::CombatLogEquipment.index()] =
        TableView::Complete(TableRows::CombatLogEquipment(Vec::new())).into();
    inputs[4].tables[TableId::LendLeaseEquipment.index()] =
        TableView::Complete(TableRows::LendLeaseEquipment(Vec::new())).into();
    if let TableView::Complete(TableRows::SunkConvoys(rows)) =
        &mut *inputs[5].tables[TableId::SunkConvoys.index()]
    {
        rows[0].convoys = 2;
    }
    if let TableView::Complete(TableRows::SunkShips(rows)) =
        &mut *inputs[6].tables[TableId::SunkShips.index()]
    {
        let entry = rows[0].entries[0].clone();
        rows[0].entries.push(entry);
    }
    if let TableView::Complete(TableRows::DivisionCensus(rows)) =
        &mut *inputs[7].tables[TableId::DivisionCensus.index()]
    {
        rows[0].ungrouped = 4;
    }
    for restart in every_restart(&inputs) {
        parity("accounting", &inputs, restart, None);
    }
}

#[test]
fn partial_and_unavailable_inputs_match_across_cold_segments() {
    for &table in TABLES.iter().filter(|&&table| table != TableId::Campaign) {
        for partial in [false, true] {
            let mut inputs: Vec<_> = (10..15).map(frame).collect();
            let TableView::Complete(rows) = &*inputs[1].tables[table.index()] else {
                continue;
            };
            inputs[1].tables[table.index()] = if partial {
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "named synthetic refusal".into(),
                }
            }
            .into();
            for restart in [None, Some(1), Some(2), Some(3)] {
                parity("status", &inputs, restart, None);
            }
        }
    }
}

#[test]
fn gaps_identity_replacements_and_refused_campaigns_keep_unknown_subtotals() {
    let mut inputs = vec![
        frame(10),
        frame(11),
        frame(14),
        frame(15),
        frame(16),
        frame(17),
    ];
    inputs[1].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "identity missing".into(),
    }
    .into();
    inputs[3].tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"different".to_vec(),
        }]))
        .into();
    for input in &mut inputs[4..] {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].id = 200_000;
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 200_000,
                canonical_slot: 0,
            });
        }
    }
    for restart in [None, Some(1), Some(2), Some(3), Some(4)] {
        parity("identity and gap", &inputs, restart, None);
    }
}

#[test]
fn changed_rows_wait_for_their_read_and_future_combat_dates() {
    let mut inputs: Vec<_> = (10..15).map(frame).collect();
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *inputs[1].tables[TableId::SliceReads.index()]
    {
        rows[0].sample_hour = 12;
    }
    inputs[2].tables[TableId::SliceReads.index()] =
        inputs[1].tables[TableId::SliceReads.index()].clone();
    if let TableView::Complete(TableRows::LendLeaseEquipment(rows)) =
        &mut *inputs[1].tables[TableId::LendLeaseEquipment.index()]
    {
        rows[0].amount_raw = 900_000;
    }
    for restart in [None, Some(1), Some(2), Some(3)] {
        parity(
            "supplying dates",
            &inputs,
            restart,
            Some((11, TableId::LendLeaseEquipment)),
        );
    }
}

#[test]
fn campaign_refusal_cannot_reuse_a_skipped_drain_owner_join() {
    let first = frame(10);
    let mut refused = frame(11);
    refused.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: b"different".to_vec(),
        }]))
        .into();
    if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
        &mut *refused.tables[TableId::EquipmentDestruction.index()]
    {
        rows[0].country = 100_001;
        rows[0].amount_raw = 777;
    }
    let mut next = refused.clone();
    next.hour = 12;
    next.tables[TableId::Campaign.index()] = first.tables[TableId::Campaign.index()].clone();
    let leaf = (
        LossSource::Destruction(EquipmentDestructionCause::LandCombat),
        Some(17),
    );
    for restart in [None, Some(1), Some(2)] {
        let inputs = [first.clone(), refused.clone(), next.clone()];
        let [(prepared, _), _] = parity("skipped owner join", &inputs, restart, None);
        assert_eq!(prepared.countries[&(0, 100_000)].totals[&leaf], 123_456);
        assert_eq!(prepared.countries[&(1, 100_001)].totals[&leaf], 777);
    }
}

#[test]
fn aliases_keep_keyed_last_values_and_repeated_read_events() {
    let mut inputs: Vec<_> = (10..15).map(frame).collect();
    for input in &mut inputs {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 100_003,
                canonical_slot: 0,
            });
        }
        if let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
            &mut *input.tables[TableId::CombatLogEquipment.index()]
        {
            let mut alias = rows[0].clone();
            alias.country = 100_003;
            alias.amount_raw = 111;
            rows.push(alias);
        }
        if let TableView::Complete(TableRows::SliceReads(rows)) =
            &mut *input.tables[TableId::SliceReads.index()]
        {
            rows.push(SliceReadRow {
                table: TableId::CombatLogEquipment as u32,
                owner: 100_003,
                sample_hour: input.hour,
            });
        }
        if let TableView::Complete(TableRows::LendLeaseEquipment(rows)) =
            &mut *input.tables[TableId::LendLeaseEquipment.index()]
        {
            let mut alias = rows[0].clone();
            alias.giver = 100_003;
            alias.amount_raw = 111;
            rows.push(alias);
            rows.sort_unstable_by_key(RowCodec::key);
        }
    }
    if let TableView::Complete(TableRows::LendLeaseEquipment(rows)) =
        &mut *inputs[2].tables[TableId::LendLeaseEquipment.index()]
    {
        rows.push(LendLeaseEquipmentRow {
            giver: 100_001,
            receiver: 100_000,
            exchange: 92,
            kind: 1,
            variant: 17,
            amount_raw: 100,
        });
        rows.sort_unstable_by_key(RowCodec::key);
    }
    for restart in [None, Some(1), Some(2), Some(3)] {
        parity("aliases", &inputs, restart, None);
    }
}

#[test]
fn giver_aliases_sharing_a_key_publish_the_same_points_wherever_segments_start() {
    // Per supplying read from hour 10: the primary's and the alias's amount of one key.
    let amounts = [
        (100, 40),
        (100, 40),
        (130, 40),
        (130, 55),
        (130, 55),
        (130, 55),
    ];
    let mut inputs: Vec<_> = (10..16).map(frame).collect();
    for (input, (primary, alias)) in inputs.iter_mut().zip(amounts) {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 100_003,
                canonical_slot: 0,
            });
        }
        if let TableView::Complete(TableRows::LendLeaseEquipment(rows)) =
            &mut *input.tables[TableId::LendLeaseEquipment.index()]
        {
            rows[0].amount_raw = primary;
            let mut row = rows[0].clone();
            row.giver = 100_003;
            row.amount_raw = alias;
            rows.push(row);
            rows.sort_unstable_by_key(RowCodec::key);
        }
    }
    let point = |hour: i32, total: &str| LossPoint {
        hour,
        total_raw: total.into(),
    };
    for restart in every_restart(&inputs) {
        let outputs = parity("lend aliases", &inputs, restart, None);
        for output in &outputs {
            assert_eq!(
                published(output, (0, 100_000), (LossSource::LendLeaseSunk, Some(17))),
                (vec![point(10, "40"), point(13, "55")], 2, 2),
                "segment restart at {restart:?}"
            );
        }
    }
}

#[test]
fn startup_and_later_partial_drains_preserve_signed_zero_net_drains() {
    let mut inputs: Vec<_> = (10..15).map(frame).collect();
    inputs[0].tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
        reason: EQUIPMENT_DESTRUCTION_PARTIAL_REASON.into(),
    }
    .into();
    for input in &mut inputs[1..] {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 100_003,
                canonical_slot: 0,
            });
        }
        if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
            &mut *input.tables[TableId::EquipmentDestruction.index()]
        {
            let mut correction = rows[0].clone();
            correction.country = 100_003;
            correction.amount_raw = -correction.amount_raw;
            rows.push(correction);
        }
    }
    inputs[3].tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
        reason: EQUIPMENT_DESTRUCTION_PARTIAL_REASON.into(),
    }
    .into();
    for restart in [None, Some(1), Some(2), Some(3), Some(4)] {
        parity("signed subtotal", &inputs, restart, None);
    }
}

#[test]
fn supplemental_markers_and_historical_coverage_keep_named_uncertainty() {
    let mut inputs: Vec<_> = (10..15).map(frame).collect();
    if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
        &mut *inputs[1].tables[TableId::EquipmentDestruction.index()]
    {
        rows.insert(
            0,
            EquipmentDestructionRow {
                coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
                unavailable_reason: "Named supplemental writer refused".to_owned().into(),
                ..Default::default()
            },
        );
    }
    if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
        &mut *inputs[2].tables[TableId::EquipmentDestruction.index()]
    {
        rows[0].coverage = Recorded::NotRecorded;
        rows[0].unavailable_reason = Recorded::NotRecorded;
    }
    if let TableView::Complete(TableRows::EquipmentDestruction(rows)) =
        &mut *inputs[3].tables[TableId::EquipmentDestruction.index()]
    {
        rows[0].country = 999_999;
    }
    for restart in [None, Some(1), Some(2), Some(3), Some(4)] {
        parity("supplemental", &inputs, restart, None);
    }
}

#[test]
fn unused_variant_change_payload_and_definition_fields_do_not_rebuild_names() {
    let mut inputs: Vec<_> = (10..16).map(frame).collect();
    for (ordinal, input) in inputs.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *input.tables[TableId::VariantDefinitions.index()]
        {
            rows[0].obsolete = (ordinal % 2) as u8;
            rows[0].classifier_flags = ordinal as u64;
            rows[0].explicit_entity = vec![b'x'; 32_000 + ordinal];
        }
        input.tables[TableId::VariantChanges.index()] =
            TableView::Complete(TableRows::VariantChanges(vec![VariantChangeRow {
                failure: vec![b'a' + ordinal as u8; 32_000],
                ..Default::default()
            }]))
            .into();
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
    let prepared = Prepared::new(&replay, 1, 10);
    let mut projector = prepared.projector(&replay).unwrap();
    let mut hours: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(TABLES, 1, 10).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (ordinal, input) in inputs.iter().enumerate() {
        write_hour(&mut walk, input, None);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &crate::products::equipment_losses::columnar::needs(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(
            projector.naming_reads(),
            1,
            "unrelated fields rebuilt naming at {}",
            input.hour
        );
    }
    for restart in [None, Some(1), Some(3)] {
        parity("unused variant payload", &inputs, restart, None);
    }
}

#[test]
fn unchanged_combat_amounts_are_not_read_again_for_a_single_raw_owner() {
    let mut inputs: Vec<_> = (10..16).map(frame).collect();
    for input in &mut inputs {
        if let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
            &mut *input.tables[TableId::CombatLogEquipment.index()]
        {
            rows[0].date = input.hour;
        }
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
    let mut projector = Prepared::new(&replay, 1, 10).projector(&replay).unwrap();
    let mut hours: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(TABLES, 1, 10).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (ordinal, input) in inputs.iter().enumerate() {
        write_hour(&mut walk, input, None);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &crate::products::equipment_losses::columnar::needs(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(
            projector.log_reads(),
            1,
            "date-only changes reread loss amounts"
        );
    }
    for restart in [None, Some(1), Some(3)] {
        parity("date-only combat", &inputs, restart, None);
    }
}

#[test]
fn deleted_combat_slots_can_be_reused_by_a_different_owner_before_its_read() {
    let mut inputs: Vec<_> = (10..16).map(frame).collect();
    for (ordinal, input) in inputs.iter_mut().enumerate() {
        let owner = 100_000 + (ordinal as u32 % 2);
        if let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
            &mut *input.tables[TableId::CombatLogEquipment.index()]
        {
            rows[0].country = owner;
            rows[0].amount_raw = 1_000 + ordinal as i64;
        }
        if let TableView::Complete(TableRows::SliceReads(rows)) =
            &mut *input.tables[TableId::SliceReads.index()]
        {
            rows[0].owner = owner;
            rows[0].sample_hour = if ordinal == 1 {
                input.hour + 2
            } else {
                input.hour
            };
        }
    }
    for restart in [None, Some(1), Some(2), Some(3), Some(4)] {
        parity("slot reuse", &inputs, restart, None);
    }
}

#[test]
fn naming_changes_wait_for_the_next_supplying_read_and_refresh_token_dependencies() {
    let first = dashboard_core::test_fixtures::slot_hour(TableId::VariantDefinitions);
    let mut inputs: Vec<_> = (first..first + 6).map(frame).collect();
    for input in &mut inputs {
        input.tables[TableId::VariantChanges.index()] =
            TableView::Complete(TableRows::VariantChanges(Vec::new())).into();
    }
    for input in &mut inputs[1..] {
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *input.tables[TableId::VariantDefinitions.index()]
        {
            rows[0].display_name = b"changed at a later supplying read".to_vec();
        }
    }
    inputs[1].metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *inputs[3].tables[TableId::LexicalTokens.index()]
    {
        rows[0].literal = b"changed_type".to_vec();
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![inputs[0].clone()])]).unwrap();
    let mut projector = Prepared::new(&replay, 1, first).projector(&replay).unwrap();
    let mut hours: Vec<columnar::Hour> = Vec::new();
    let mut walk = Synthetic::new(TABLES, 1, first).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    for (ordinal, input) in inputs[..2].iter().enumerate() {
        write_hour(&mut walk, input, None);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &crate::products::equipment_losses::columnar::needs(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        assert_eq!(
            projector.naming_reads(),
            1,
            "a carried definition was consumed before its supplying read"
        );
    }
    for restart in [None, Some(1), Some(2), Some(3)] {
        parity("lazy naming", &inputs, restart, None);
    }
}

#[test]
fn colliding_aliases_replay_the_survivor_after_deletes_reused_slots_and_truncation() {
    // Per hour: the primary's read, the alias's read and shared-key amount, a truncated log.
    let hours = [
        (10, 10, None, false),
        (10, 11, Some(111), false),
        (10, 11, None, false),
        (13, 11, None, false),
        (13, 14, Some(222), false),
        (15, 14, Some(222), true),
        (15, 16, Some(222), false),
    ];
    let mut inputs: Vec<_> = (10..17).map(frame).collect();
    for (input, &(primary, alias, amount, truncated)) in inputs.iter_mut().zip(&hours) {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *input.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 100_003,
                canonical_slot: 0,
            });
        }
        if let Some(amount) = amount
            && let TableView::Complete(TableRows::CombatLogEquipment(rows)) =
                &mut *input.tables[TableId::CombatLogEquipment.index()]
        {
            let mut row = rows[0].clone();
            row.country = 100_003;
            row.amount_raw = amount;
            rows.push(row);
        }
        if truncated {
            let TableView::Complete(rows) = &*input.tables[TableId::CombatLogEquipment.index()]
            else {
                unreachable!("a complete fixture log");
            };
            input.tables[TableId::CombatLogEquipment.index()] = TableView::Truncated {
                rows: rows.clone(),
                missing: 1,
            }
            .into();
        }
        if let TableView::Complete(TableRows::SliceReads(rows)) =
            &mut *input.tables[TableId::SliceReads.index()]
        {
            rows[0].sample_hour = primary;
            rows.push(SliceReadRow {
                table: TableId::CombatLogEquipment as u32,
                owner: 100_003,
                sample_hour: alias,
            });
        }
    }
    for restart in every_restart(&inputs) {
        parity("alias collisions", &inputs, restart, None);
    }
}

#[test]
fn player_gates_and_unresolved_players_match_at_every_restart() {
    let player = |id: i32, country: u32| PlayerRow {
        id,
        country,
        ..Default::default()
    };
    let mut inputs: Vec<_> = (10..17).map(frame).collect();
    for (ordinal, input) in inputs.iter_mut().enumerate() {
        if let TableView::Complete(TableRows::CombatLogState(rows)) =
            &mut *input.tables[TableId::CombatLogState.index()]
        {
            rows[0].viewing_country_raw = i32::from(ordinal != 4);
        }
        input.tables[TableId::Players.index()] = match ordinal {
            0 => TableView::Complete(TableRows::Players(vec![player(1, 100_001)])),
            1 | 2 => TableView::Complete(TableRows::Players(vec![
                player(1, 100_001),
                player(2, 555_555),
            ])),
            6 => TableView::Unavailable {
                reason: "named synthetic refusal".into(),
            },
            _ => TableView::Complete(TableRows::Players(vec![player(1, 100_000)])),
        }
        .into();
    }
    for restart in every_restart(&inputs) {
        parity("players", &inputs, restart, None);
    }
}

#[test]
fn unlinked_census_owners_refresh_every_membership_at_every_restart() {
    // The unlinked owner's (grouped, ungrouped) per hour from 10; None: no row.
    let counts = [
        None,
        Some((2, 0)),
        Some((0, 0)),
        Some((0, 3)),
        None,
        Some((0, 0)),
        Some((1, 1)),
    ];
    let mut inputs: Vec<_> = (10..17).map(frame).collect();
    for (input, counts) in inputs.iter_mut().zip(counts) {
        if let Some((grouped, ungrouped)) = counts
            && let TableView::Complete(TableRows::DivisionCensus(rows)) =
                &mut *input.tables[TableId::DivisionCensus.index()]
        {
            rows.push(DivisionCensusRow {
                country: 555_555,
                grouped,
                ungrouped,
            });
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *inputs[6].tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 555_555,
            canonical_slot: 1,
        });
    }
    for restart in every_restart(&inputs) {
        parity("unlinked census", &inputs, restart, None);
    }
}

#[test]
fn module_upgrade_appearance_and_tag_writes_rename_designs_at_every_restart() {
    let variant = 17_u64 << 32 | u64::from(VARIANT_TYPE);
    let mut inputs: Vec<_> = (10..17).map(frame).collect();
    for (ordinal, input) in inputs.iter_mut().enumerate() {
        input.tables[TableId::VariantChanges.index()] =
            TableView::Complete(TableRows::VariantChanges(Vec::new())).into();
        input.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
                variant,
                source_ordinal: 0,
                slot: 1,
                definition_token: if ordinal < 1 { 2 } else { 1 },
                definition_active: 1,
            }]))
            .into();
        input.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                variant,
                source_ordinal: 0,
                definition_token: 2,
                level: if ordinal < 2 { 1 } else { 2 },
            }]))
            .into();
        let name_tag: Option<&[u8]> = match ordinal {
            0..=2 => None,
            3 | 6 => Some(&b""[..]),
            _ => Some(&b"OLD"[..]),
        };
        input.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(
                name_tag
                    .into_iter()
                    .map(|name_tag| CountryAppearanceRow {
                        country: 100_000,
                        name_tag: name_tag.to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ))
            .into();
        if ordinal >= 5
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"NEW".to_vec();
        }
    }
    for restart in every_restart(&inputs) {
        parity("naming operands", &inputs, restart, None);
    }
}
