//! Observed ledger deltas append by supplying read; only continuation is replaced.
use super::*;
use crate::prepared::series::{self, Accumulator, Reader};

#[derive(Serialize, Deserialize)]
struct Event {
    leaf: LeafKey,
    hour: i32,
    amount_raw: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Events {
    series: Accumulator,
    #[serde(skip)]
    keys: BTreeMap<String, u64>,
    #[serde(skip)]
    leaves: BTreeSet<LeafKey>,
}
impl Events {
    fn observe(&mut self, family: &mut FamilyScan, hour: i32) -> io::Result<()> {
        self.series.through(hour);
        for (leaf, deltas) in std::mem::take(&mut family.leaves) {
            // Tags are nonempty; the empty catalogue entry owns unresolved endpoints.
            let tag = leaf.country.as_ref().map_or("", |country| &country.0);
            let next = u64::try_from(self.keys.len())
                .map_err(|_| crate::invalid("Military air key count overflow"))?;
            let key = *self.keys.entry(tag.to_owned()).or_insert(next);
            self.leaves.insert(leaf.clone());
            for ((read_hour, hour), amount) in deltas {
                self.series.event(
                    key,
                    read_hour,
                    &Event {
                        leaf: leaf.clone(),
                        hour,
                        amount_raw: amount.to_string(),
                    },
                )?;
            }
        }
        Ok(())
    }
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        family: &str,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.series
            .flush(writer, &format!("{family}-{branch}-series"), 0, cancelled)?;
        series::write_catalogue(writer, family, branch, &self.keys, &self.leaves)
    }
    fn gather(
        &self,
        family: &mut FamilyScan,
        store: &store::Store,
        key: &store::SourceKey,
        section: String,
        from: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        family.leaves = self
            .leaves
            .iter()
            .cloned()
            .map(|leaf| (leaf, BTreeMap::new()))
            .collect();
        let mut reader = Reader::new(store, key, section, cancelled)?;
        let keys: Vec<_> = self.keys.values().copied().collect();
        let mut cursor = 0;
        loop {
            check_cancel(cancelled)?;
            let page = reader.events(&keys, from, i32::MAX, cursor, 1024)?;
            for row in page.rows {
                let event: Event = serde_json::from_value(row.row)?;
                let amount = event
                    .amount_raw
                    .parse::<i128>()
                    .map_err(|_| crate::invalid("Invalid prepared military air delta"))?;
                family.add(event.leaf, event.hour, row.hour, amount)?;
            }
            let Some(next) = page.cursor else {
                break;
            };
            cursor = next;
        }
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Prepared {
    scan: Scan,
    air: Events,
    bombing: Events,
}
impl Prepared {
    #[cfg(test)]
    pub(crate) fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let changes = crate::prepared::frame_changes::FrameChangeTracker::new()
            .observe(frame, replay::format::FormatVersion::default());
        self.observe_changed(frame, &changes)
    }
    pub(crate) fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan.air.observe_changed(frame, changes, false)?;
        self.scan.bombing.observe_changed(frame, changes, true)?;
        self.air.observe(&mut self.scan.air, frame.hour)?;
        self.bombing.observe(&mut self.scan.bombing, frame.hour)
    }
    pub(crate) fn buffered_bytes(&self) -> usize {
        self.air.series.bytes() + self.bombing.series.bytes()
    }
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.air.seal_presence();
        self.scan.bombing.seal_presence();
        self.air
            .flush(writer, "military-air-losses", branch, cancelled)?;
        self.bombing
            .flush(writer, "military-bombing", branch, cancelled)?;
        let bytes = crate::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)?;
        writer.write_at(
            &format!("military-air-ledger-{branch}"),
            0,
            bytes.as_slice(),
        )
    }
    pub(crate) fn read(
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<Self> {
        let section = format!("military-air-ledger-{branch}");
        let file = match saved {
            Some(previous) if previous != key => store.read_prefix(previous, key, &section, 0)?,
            _ => store.read(key, &section, 0)?,
        }
        .ok_or_else(|| crate::invalid("Prepared military air continuation is missing"))?;
        serde_json::from_reader(zstd::stream::Decoder::new(file)?).map_err(Into::into)
    }
    pub(crate) fn read_catalogues(
        &mut self,
        store: &store::Store,
        key: &store::SourceKey,
        saved: Option<&store::SourceKey>,
        branch: u64,
    ) -> io::Result<()> {
        for (name, events) in [
            ("military-air-losses", &mut self.air),
            ("military-bombing", &mut self.bombing),
        ] {
            let catalogue: Option<series::Catalogue<BTreeSet<LeafKey>>> = match saved {
                Some(previous) if previous != key => store
                    .read_prefix(previous, key, &format!("{name}-{branch}-catalogue"), 0)?
                    .map(|file| serde_json::from_reader(io::BufReader::new(file)))
                    .transpose()?,
                _ => series::read_catalogue(store, key, name, branch)?,
            };
            (events.keys, events.leaves) = catalogue
                .ok_or_else(|| crate::invalid("Prepared military air catalogue is missing"))?;
        }
        Ok(())
    }
    fn reply(
        &mut self,
        source: &Source,
        store: &store::Store,
        key: &store::SourceKey,
        params: &MilitaryAirParams,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<MilitaryAirReply> {
        self.scan.air.seal_presence();
        self.scan.bombing.seal_presence();
        let branch = params.branch_id.parse::<u64>().expect("validated branch");
        let declaration = source
            .replay_ref()
            .branches
            .iter()
            .find(|row| row.id == branch)
            .ok_or_else(|| crate::invalid("Unknown branch"))?;
        (self.scan.first, self.scan.last) = branch_bounds(source.replay_ref(), declaration);
        let (events, family, name) = match params.family {
            MilitaryAirFamily::AirLosses => (&self.air, &mut self.scan.air, "military-air-losses"),
            MilitaryAirFamily::Bombing => {
                (&self.bombing, &mut self.scan.bombing, "military-bombing")
            }
        };
        events.gather(
            family,
            store,
            key,
            format!("{name}-{branch}-series"),
            params.from_hour.unwrap_or(i32::MIN),
            cancelled,
        )?;
        self.scan.reply(source, params, context, cancelled)
    }
}

pub(crate) fn query(
    source: &Source,
    store: &store::Store,
    key: &store::SourceKey,
    mut params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let availability = params
        .as_object_mut()
        .and_then(|params| params.remove("availability"))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| crate::invalid("Invalid ledger availability request"))
        })
        .transpose()?
        .unwrap_or(false);
    let params: MilitaryAirParams = serde_json::from_value(params)?;
    MilitaryAirLedger::validate(&params).map_err(crate::invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let mut state = Prepared::read(store, key, None, branch)?;
    if availability {
        let family = match params.family {
            MilitaryAirFamily::AirLosses => &state.scan.air,
            MilitaryAirFamily::Bombing => &state.scan.bombing,
        };
        return Ok(serde_json::json!({ "available": family.available }));
    }
    state.read_catalogues(store, key, None, branch)?;
    serde_json::to_value(state.reply(source, store, key, &params, context, cancelled)?)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared::series::tests::Fixture;
    use crate::test_fixtures::{CAMPAIGN, records, scale_frame};
    use replay::{
        frame::TableView,
        rows::{AirLossEventRow, BuildingBombingDamageRow, TableRows},
    };

    fn loss(id: u64, hour: i32, amount: i64) -> AirLossEventRow {
        AirLossEventRow {
            id,
            game_hour: hour,
            country: 9,
            region: 40001,
            native_cause: 1,
            family_base: 3,
            amount_raw: amount,
            ..Default::default()
        }
    }
    fn source(frames: Vec<DecodedFrame>) -> Source {
        Source::from_replay(
            crate::SourceKey {
                source_id: "ledger-prepared".into(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    fn params(family: MilitaryAirFamily, from_hour: Option<i32>, top: bool) -> MilitaryAirParams {
        MilitaryAirParams {
            branch_id: "1".into(),
            family,
            regions: vec![],
            causes: vec![],
            from_hour,
            top: top.then_some(MilitaryAirTop {
                count: 5000,
                metric: MilitaryAirMetric::Total,
            }),
        }
    }
    fn assert_parity(state: &mut Prepared, fixture: &Fixture, source: &Source) {
        let cancelled = AtomicBool::new(false);
        let context = LoadContext { content: None };
        for family in [MilitaryAirFamily::AirLosses, MilitaryAirFamily::Bombing] {
            for from in [None, Some(10), Some(12)] {
                for top in [false, true] {
                    let params = params(family, from, top);
                    let actual = state
                        .reply(
                            source,
                            &fixture.store,
                            &fixture.key,
                            &params,
                            &context,
                            &cancelled,
                        )
                        .unwrap();
                    let expected =
                        MilitaryAirLedger::load(source, &params, &context, &cancelled).unwrap();
                    assert_eq!(
                        serde_json::to_value(actual).unwrap(),
                        serde_json::to_value(expected).unwrap()
                    );
                }
            }
        }
    }
    fn resume(fixture: &Fixture, saved: Option<&store::SourceKey>) -> Prepared {
        let mut state = Prepared::read(&fixture.store, &fixture.key, saved, 1).unwrap();
        state
            .read_catalogues(&fixture.store, &fixture.key, saved, 1)
            .unwrap();
        state
    }

    fn work(state: &Prepared) -> crate::prepared::observer_tests::Work {
        let air = state.scan.air.work;
        let bombing = state.scan.bombing.work;
        crate::prepared::observer_tests::Work {
            row_visits: air.row_visits + bombing.row_visits,
            history_visits: air.history_visits + bombing.history_visits,
            emitted: air.emitted + bombing.emitted,
            ..Default::default()
        }
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "5,000-country observer work and reply parity"
    )]
    fn ledger_shared_driver_equal_reads_defer_country_presence_and_match_the_oracle() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(8, |hours| {
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut first = scale_frame(1, 10, 5000);
            first.tables[TableId::AirLossEvents.index()] =
                TableView::Complete(TableRows::AirLossEvents(vec![])).into();
            first.tables[TableId::BuildingBombingDamage.index()] =
                TableView::Complete(TableRows::BuildingBombingDamage(vec![])).into();
            let replay = crate::test_fixtures::replay(vec![first.clone()]);
            let mut state = Prepared::default();
            state.observe_changed(
                &first,
                &FrameChangeTracker::new().observe(&first, replay.format),
            )?;
            let delta = observer_tests::unchanged_frames(
                work(&state),
                &first,
                hours as i32,
                replay.format,
                |frame, changes| {
                    state.observe_changed(frame, changes)?;
                    Ok(work(&state))
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            assert_eq!(
                state
                    .scan
                    .air
                    .countries
                    .values()
                    .next()
                    .unwrap()
                    .last()
                    .unwrap()
                    .last_hour,
                first.hour
            );
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag)?;
            writer.commit()?;
            let frames = (0..=hours)
                .map(|offset| {
                    let mut frame = first.clone();
                    frame.hour += offset as i32;
                    frame
                })
                .collect();
            assert_parity(&mut resume(&fixture, None), &fixture, &source(frames));
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn ledger_shared_driver_unchanged_work_does_not_visit_retained_alias_history() {
        use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let measured = observer_tests::scaled_work(20, |history| {
            let mut state = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut frame = super::super::tests::frame(0, vec![], vec![]);
            let format = replay::format::FormatVersion::default();
            for hour in 0..history {
                frame.hour = hour as i32;
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = format!("alias-{hour}").into_bytes();
                }
                state.observe_changed(&frame, &tracker.observe(&frame, format))?;
            }
            assert_eq!(state.scan.air.countries.len(), history);
            let delta = observer_tests::unchanged_frames(
                work(&state),
                &frame,
                10,
                format,
                |frame, changes| {
                    state.observe_changed(frame, changes)?;
                    Ok(work(&state))
                },
            )?;
            let mut previous = frame.clone();
            previous.hour += 10;
            tracker.observe(&previous, format);
            frame.hour += 11;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].political_power_raw = Some(999);
            }
            let before = work(&state);
            state.observe_changed(&frame, &tracker.observe(&frame, format))?;
            assert_eq!(
                work(&state).history_visits,
                before.history_visits,
                "balance-only changes do not touch presence"
            );
            frame.hour += 1;
            frame.tables[TableId::AirLossEvents.index()] =
                TableView::Complete(TableRows::AirLossEvents(vec![loss(999999, frame.hour, 1)]))
                    .into();
            let before = work(&state);
            state.observe_changed(&frame, &tracker.observe(&frame, format))?;
            assert_eq!(work(&state).row_visits - before.row_visits, 1);
            assert_eq!(work(&state).history_visits, before.history_visits);
            Ok(delta)
        })
        .unwrap();
        assert_eq!(measured, [observer_tests::Work::default(); 2]);
        observer_tests::assert_flat(measured);
    }

    #[test]
    fn prepared_ledger_two_commits_resume_append_preserve_exact_replies() {
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let damage = |id, hour| BuildingBombingDamageRow {
            id,
            game_hour: hour,
            attacker: None,
            victim: 9,
            state: 0,
            building_type: 9001,
            cause: BuildingBombingDamageCause::AirMission,
            amount_raw: 12_345_678,
            ..Default::default()
        };
        let mut frames = vec![
            super::super::tests::frame(
                10,
                vec![loss(0, 9, i64::MAX), loss(1, 9, i64::MAX)],
                vec![damage(0, 9)],
            ),
            super::super::tests::frame(11, vec![loss(2, 8, -100_000)], vec![damage(1, 10)]),
            super::super::tests::frame(13, vec![loss(3, 12, 100_000)], vec![]),
        ];
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frames[2].tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed holder".to_vec();
        }
        let mut state = Prepared::default();
        let mut writer = fixture.writer();
        state.observe(&frames[0]).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        state = resume(&fixture, None);
        let mut writer = fixture.writer();
        state.observe(&frames[1]).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert_parity(
            &mut resume(&fixture, None),
            &fixture,
            &source(frames[..2].to_vec()),
        );
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = resume(&fixture, Some(&previous));
        state.observe(&frames[2]).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert_parity(&mut resume(&fixture, None), &fixture, &source(frames));
        let available = query(
            &source(vec![super::super::tests::frame(10, vec![], vec![])]),
            &fixture.store,
            &fixture.key,
            serde_json::json!({"branchId":"1", "family":"airLosses", "availability":true}),
            &LoadContext { content: None },
            &flag,
        )
        .unwrap();
        assert_eq!(available, serde_json::json!({"available":true}));
    }

    #[test]
    fn ledger_deferred_presence_survives_two_commits_resume_append_and_a_gap() {
        use crate::prepared::frame_changes::FrameChangeTracker;
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let first = super::super::tests::frame(10, vec![], vec![]);
        let mut frames: Vec<_> = (10..=39)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        frames[29].hour = 41;
        let format = replay::format::FormatVersion::default();
        let mut state = Prepared::default();
        for chunk in [&frames[..10], &frames[10..20]] {
            let mut tracker = FrameChangeTracker::new();
            let mut writer = fixture.writer();
            for frame in chunk {
                state
                    .observe_changed(frame, &tracker.observe(frame, format))
                    .unwrap();
            }
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = resume(&fixture, None);
        }
        assert_parity(&mut state, &fixture, &source(frames[..20].to_vec()));
        let previous = fixture.key.clone();
        let mut writer = fixture.append_writer();
        state = resume(&fixture, Some(&previous));
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[20..] {
            state
                .observe_changed(frame, &tracker.observe(frame, format))
                .unwrap();
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert_parity(&mut resume(&fixture, None), &fixture, &source(frames));
    }

    #[test]
    fn prepared_ledger_5000_countries_and_identity_gaps_match_the_oracle() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut first = scale_frame(1, 10, 5000);
        for table in [TableId::AirLossEvents, TableId::BuildingBombingDamage] {
            *first.tables[table.index()] = match table {
                TableId::AirLossEvents => TableView::Complete(TableRows::AirLossEvents(vec![])),
                _ => TableView::Complete(TableRows::BuildingBombingDamage(vec![])),
            };
        }
        let mut last = first.clone();
        last.hour = 12;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *last.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"new alias".to_vec();
            rows[1].id = 999_999;
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *last.tables[TableId::CountryLinks.index()]
        {
            rows[1].id = 999_999;
            rows.sort_by_key(|row| row.id);
        }
        let mut missing = first.clone();
        missing.hour = 11;
        *missing.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "identity read missing".into(),
        };
        let frames = vec![first, missing, last];
        let mut state = Prepared::default();
        for chunk in [&frames[..2], &frames[2..]] {
            let mut writer = fixture.writer();
            for frame in chunk {
                state.observe(frame).unwrap();
            }
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            state = resume(&fixture, None);
        }
        assert_eq!(state.scan.air.countries.len(), 5002);
        assert_parity(&mut state, &fixture, &source(frames));
    }

    #[test]
    fn prepared_ledger_availability_reads_only_the_continuation_flag() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let frame = super::super::tests::frame(10, vec![], vec![]);
        let source = source(vec![frame.clone()]);
        let mut state = Prepared::default();
        state.observe(&frame).unwrap();
        // Deliberately publish neither series files nor country/leaf catalogues.
        let mut writer = fixture.writer();
        let bytes =
            zstd::stream::encode_all(serde_json::to_vec(&state).unwrap().as_slice(), 3).unwrap();
        writer
            .write_at("military-air-ledger-1", 0, bytes.as_slice())
            .unwrap();
        writer.commit().unwrap();
        for family in ["airLosses", "bombing"] {
            assert_eq!(
                query(
                    &source,
                    &fixture.store,
                    &fixture.key,
                    serde_json::json!({"branchId":"1", "family":family, "availability":true}),
                    &LoadContext { content: None },
                    &flag
                )
                .unwrap(),
                serde_json::json!({"available":true})
            );
        }
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_ledger_continuation_does_not_retain_hourly_event_history() {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut state = Prepared::default();
        let mut sizes = Vec::new();
        for through in [99, 9999] {
            let first = state
                .scan
                .air
                .observed
                .last()
                .map_or(0, |span| span.last_hour + 1);
            for hour in first..=through {
                state
                    .observe(&super::super::tests::frame(
                        hour,
                        vec![loss(hour as u64, hour, 100_000)],
                        vec![],
                    ))
                    .unwrap();
            }
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            sizes.push(serde_json::to_vec(&state).unwrap().len());
            assert!(state.scan.air.leaves.is_empty());
            state = resume(&fixture, None);
        }
        eprintln!("ledger continuation JSON bytes at 100 and 10,000 event hours: {sizes:?}");
        assert!(
            sizes[1] <= sizes[0] + 64,
            "dated events must remain outside continuation"
        );
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            "military-air-losses-1-series".into(),
            &flag,
        )
        .unwrap();
        assert_eq!(reader.event_count_through(0, 9999).unwrap(), 10_000);
    }
}
