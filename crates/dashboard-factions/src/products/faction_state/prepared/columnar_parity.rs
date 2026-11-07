use super::super::{
    columnar::{self, Projector},
    harness::{Fixture, Walked},
};
use super::*;
use dashboard_base::{
    SourceKey,
    prepared::{
        coverage::{Coverage, SCHEMA},
        frame_changes::FrameChangeTracker,
        prefix::Prefix,
    },
    test_fixtures as fixtures,
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{format::FormatVersion, rows::*};

const FORMAT: FormatVersion = FormatVersion::CURRENT;

/// The supplying reads a walk does not hold, sought in the row recording's frames.
struct Recorded<'a>(&'a Source);

impl columnar::Earlier for Recorded<'_> {
    fn read(
        &self,
        branch: u64,
        (lo, hi): (i32, i32),
        cat: Option<&catalogue::Catalogue>,
        cancelled: &AtomicBool,
    ) -> io::Result<Option<columnar::Kept>> {
        let mut walks = Supplying::new(self.0, branch);
        let Some(hour) = walks
            .read_hour((branch, hi), cancelled)?
            .filter(|&hour| hour >= lo)
        else {
            return Ok(None);
        };
        let Some(narrow) = walks.frame(hour, false, cancelled)? else {
            return Ok(None);
        };
        let wide = cat.is_some_and(|cat| {
            selectors::read_names_scripted(replay_frame::frame::HourFrame::from(&*narrow), cat)
        });
        let frame = match wide {
            true => walks
                .frame(hour, true, cancelled)?
                .ok_or_else(|| io::Error::other("The read's scripted-text tables are not recorded"))?,
            false => narrow,
        };
        Ok(Some(columnar::Kept { hour, wide, frame }))
    }
}

fn projector<'a>(source: &'a Source, context: &LoadContext) -> Projector<'a> {
    Projector::new(FORMAT, catalogued(source, context), Arc::new(Recorded(source))).unwrap()
}

macro_rules! rows {
    ($frame:expr, $table:ident) => {
        match &mut *$frame.tables[TableId::$table.index()] {
            TableView::Complete(TableRows::$table(rows)) => rows,
            _ => unreachable!("a complete fixture table"),
        }
    };
}

/// Two factions: 9 with countries 101 and 102 (raw 11 and 12), 10 with 103 (raw 13); each
/// account is one country's, and every hour moves an unread country, account and projection
/// field, faction 9's leader's manpower and its goal's progress.
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = fixtures::source(1, hour, Some(0));
    let tables = &mut frame.tables;
    *tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"AAA".to_vec(),
            political_power_raw: Some(i64::from(hour)),
            ..Default::default()
        },
        CountryRow {
            id: 102,
            canonical_slot: 8,
            tag: b"BBB".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 103,
            canonical_slot: 9,
            tag: b"CCC".to_vec(),
            ..Default::default()
        },
    ]));
    *tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (12, 8), (13, 9), (101, 7), (102, 8), (103, 9)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    *tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(
        [9, 10]
            .into_iter()
            .map(|id| FactionRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    *tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(
        [(11, 9, 0), (12, 9, 1), (13, 10, 0)]
            .into_iter()
            .map(|(country, faction, ordinal)| FactionMemberRow {
                country,
                faction,
                ordinal,
            })
            .collect(),
    ));
    *tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(vec![
            FactionAccountRow {
                account: 3,
                country: 101,
                influence_refresh: 1,
                influence_numerator_raw: 1,
                influence_denominator_raw: 4,
                influence_faction: 9,
                upgrade_groups: vec![2],
                upgrade_tokens: vec![3],
                statistic_raw: i64::from(hour),
                ..Default::default()
            },
            FactionAccountRow {
                account: 4,
                country: 102,
                influence_refresh: 1,
                influence_numerator_raw: 1,
                influence_denominator_raw: 2,
                influence_faction: 9,
                statistic_raw: i64::from(hour),
                ..Default::default()
            },
            FactionAccountRow {
                account: 5,
                country: 103,
                influence_refresh: 1,
                influence_numerator_raw: 1,
                influence_denominator_raw: 1,
                influence_faction: 10,
                ..Default::default()
            },
        ]));
    *tables[TableId::FactionRules.index()] = TableView::Complete(TableRows::FactionRules(
        [9, 10]
            .into_iter()
            .map(|faction| FactionRuleRow {
                faction,
                ordinal: 0,
                token: 1,
            })
            .collect(),
    ));
    *tables[TableId::FactionPrograms.index()] =
        TableView::Complete(TableRows::FactionPrograms(Vec::new()));
    *tables[TableId::FactionMinimumUpgrades.index()] =
        TableView::Complete(TableRows::FactionMinimumUpgrades(vec![
            FactionMinimumUpgradeRow {
                faction: 9,
                ordinal: 0,
                group: 2,
                token: 3,
            },
        ]));
    *tables[TableId::FactionDoctrine.index()] =
        TableView::Complete(TableRows::FactionDoctrine(vec![FactionDoctrineRow {
            faction: 9,
            ordinal: 0,
            token: 4,
        }]));
    *tables[TableId::FactionManpower.index()] = TableView::Complete(TableRows::FactionManpower(
        [(9, 0, 11, 1000 + hour), (9, 1, 12, 7), (10, 0, 13, 5)]
            .into_iter()
            .map(|(faction, ordinal, country, value)| FactionManpowerRow {
                faction,
                ordinal,
                country,
                value,
            })
            .collect(),
    ));
    *tables[TableId::FactionProjection.index()] =
        TableView::Complete(TableRows::FactionProjection(
            [9, 10]
                .into_iter()
                .map(|faction| FactionProjectionRow {
                    faction,
                    projection_raw: i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ));
    *tables[TableId::FactionUnlocks.index()] = TableView::Complete(TableRows::FactionUnlocks(
        [9, 10]
            .into_iter()
            .map(|faction| FactionUnlockRow {
                faction,
                ..Default::default()
            })
            .collect(),
    ));
    *tables[TableId::FactionGoalStatus.index()] =
        TableView::Complete(TableRows::FactionGoalStatus(
            [9, 10]
                .into_iter()
                .map(|faction| FactionGoalStatusRow {
                    faction,
                    ..Default::default()
                })
                .collect(),
        ));
    *tables[TableId::FactionGoalEntries.index()] =
        TableView::Complete(TableRows::FactionGoalEntries(vec![FactionGoalEntryRow {
            faction: 9,
            kind: 0,
            ordinal: 0,
            token: Some(5),
            status: 1,
            progress_raw: i64::from(hour) * 10,
            ..Default::default()
        }]));
    *tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        [
            (1, "rule_a"),
            (2, "group_b"),
            (3, "upgrade_c"),
            (4, "doctrine_d"),
            (5, "goal_e"),
            (6, "rule_f"),
        ]
        .into_iter()
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .collect(),
    ));
    *tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow::default()]));
    *tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow::default(),
        ]));
    *tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(Vec::new()));
    *tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(Vec::new()));
    for &table in columnar::TABLES {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
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

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame) {
    macro_rules! tables { ($($table:ident),* $(,)?) => { $(write::<tables::$table>(walk, frame);)* }; }
    tables!(
        Campaign,
        Countries,
        CountryLinks,
        ReservedCountryNaming,
        Factions,
        FactionMembers,
        FactionAccounts,
        FactionProjection,
        FactionUnlocks,
        FactionGoalStatus,
        FactionGoalEntries,
        FactionRules,
        FactionPrograms,
        FactionMinimumUpgrades,
        FactionDoctrine,
        FactionManpower,
        LexicalTokens,
        ProjectPrograms,
        Projects,
        LocalPlayer,
    );
}

/// The recording the row path reads its programme operands from.
fn source(inputs: &[DecodedFrame]) -> Source {
    let frames = inputs
        .iter()
        .map(|input| {
            let mut frame = input.clone();
            frame.branch = 1;
            frame
        })
        .collect();
    Source::from_replay(
        SourceKey {
            source_id: "faction-columnar".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(
            replay::campaign::Replay::from_records(
                fixtures::CAMPAIGN.into(),
                &[fixtures::records(1, frames)],
            )
            .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// The recorded stack's catalogue, as the native backend loads it.
fn catalogued(source: &Source, context: &LoadContext) -> Option<Arc<catalogue::Catalogue>> {
    context
        .content
        .as_ref()
        .and_then(|options| catalogue::catalogue(source, options).ok())
}

/// The emit of a segment's hours, publishing every third; at `reload`, once published, it
/// goes on from what it published.
fn emit(hours: &mut Vec<columnar::Hour>, projected: &mut Walked, fixture: &Fixture, reload: Option<i32>) {
    for hour in std::mem::take(hours) {
        let at = hour.hour;
        projected.observe_projected(hour);
        if at % 3 == 0 {
            fixture.publish(projected);
            if reload == Some(at) {
                fixture.reload();
            }
        }
    }
}

/// The row frames' projection observes `inputs` hour by hour, reading every input anew at
/// `blocks` and at gaps; the columnar projection walks segments restarting at `restarts` and at
/// gaps. Both publish through the emit alike; every lane either publishes must be equal. The
/// columnar publication.
fn parity(label: &str, inputs: &[DecodedFrame], blocks: &[usize], restarts: &[usize]) -> Fixture {
    let source = source(inputs);
    parity_in(
        label,
        &source,
        &LoadContext { content: None },
        inputs,
        (blocks, restarts),
        None,
    )
}

/// [`parity`] over `source` read with `context`, the emit reloaded mid-segment at `reload`.
fn parity_in(
    label: &str,
    source: &Source,
    context: &LoadContext,
    inputs: &[DecodedFrame],
    (blocks, restarts): (&[usize], &[usize]),
    reload: Option<i32>,
) -> Fixture {
    let label = format!("{label}: blocks {blocks:?}, restarts {restarts:?}, reload {reload:?}");
    let branch = inputs[0].branch;
    let flag = AtomicBool::new(false);
    let [row_sections, projected_sections] = [Fixture::new(), Fixture::new()];
    let mut row = Walked::default();
    let mut projected = Walked::default();
    let mut tracker = FrameChangeTracker::new();
    let tables = columnar::tables();
    let mut walk = Synthetic::new(tables, branch, inputs[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut segment = projector(source, context);
    let mut hours: Vec<columnar::Hour> = Vec::new();
    segment
        .start(Start::Run { continues: false }, branch, inputs[0].hour)
        .unwrap();
    for (ordinal, input) in inputs.iter().enumerate() {
        let run = ordinal > 0 && inputs[ordinal - 1].hour.checked_add(1) != Some(input.hour);
        let opens = ordinal == 0 || run || restarts.contains(&ordinal);
        if ordinal > 0 && opens {
            segment.end(&mut hours).unwrap();
            emit(&mut hours, &mut projected, &projected_sections, reload);
            segment = projector(source, context);
            let start = if run {
                Start::Run { continues: true }
            } else {
                Start::Restart(walk.state())
            };
            assert!(segment.starts_at(start), "{label}");
            segment.start(start, branch, input.hour).unwrap();
            if run {
                walk = Synthetic::new(tables, branch, input.hour).unwrap();
            }
        }
        if ordinal > 0 && (run || blocks.contains(&ordinal)) {
            tracker.invalidate();
        }
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &crate::products::faction_state::columnar::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            let changes = tracker.observe(frame, FORMAT);
            row.observe(source, context, frame, &changes, &flag)
                .unwrap();
            if frame.hour % 3 == 0 {
                row_sections.publish(&mut row);
            }
            segment
                .hour(
                    &segments::Hour {
                        branch,
                        hour: frame.hour,
                        run: u32::from(run),
                        ordinal: ordinal as u32,
                        starts,
                        opens,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
    }
    segment.end(&mut hours).unwrap();
    emit(&mut hours, &mut projected, &projected_sections, reload);
    row_sections.publish(&mut row);
    projected_sections.publish(&mut projected);
    let expected = row_sections.dump();
    assert!(!expected.is_empty(), "{label}");
    assert_eq!(projected_sections.dump(), expected, "{label}");
    projected_sections
}

/// No restart, each hour's alone, and one at every hour.
fn placements(inputs: &[DecodedFrame]) -> Vec<Vec<usize>> {
    let mut all = vec![Vec::new(), (1..inputs.len()).collect()];
    all.extend((1..inputs.len()).map(|at| vec![at]));
    all
}

fn reply(
    fixture: &Fixture,
    inputs: &[DecodedFrame],
    params: FactionStateParams,
) -> FactionStateReply {
    let source = source(inputs);
    let prefix = Prefix::new(
        &source,
        inputs[0].branch,
        Coverage {
            schema: SCHEMA,
            first_hour: inputs[0].hour,
            last_hour: inputs[inputs.len() - 1].hour,
            complete: true,
        },
        None,
    );
    fixture
        .reply(
            &source,
            &params,
            &LoadContext { content: None },
            prefix,
            &AtomicBool::new(false),
        )
        .unwrap()
}

fn influence(fixture: &Fixture, inputs: &[DecodedFrame], faction: &str) -> FactionInfluenceReply {
    let params = FactionStateParams::Influence {
        branch_id: inputs[0].branch.to_string(),
        faction: faction.into(),
    };
    match reply(fixture, inputs, params) {
        FactionStateReply::Influence(reply) => reply,
        _ => unreachable!(),
    }
}

fn rules(fixture: &Fixture, inputs: &[DecodedFrame], faction: &str) -> FactionRulesReply {
    let params = FactionStateParams::Rules {
        branch_id: inputs[0].branch.to_string(),
        faction: faction.into(),
    };
    match reply(fixture, inputs, params) {
        FactionStateReply::Rules(reply) => reply,
        _ => unreachable!(),
    }
}

fn manpower(fixture: &Fixture, inputs: &[DecodedFrame], faction: &str) -> FactionManpowerReply {
    let params = FactionStateParams::Manpower {
        branch_id: inputs[0].branch.to_string(),
        faction: faction.into(),
    };
    match reply(fixture, inputs, params) {
        FactionStateReply::Manpower(reply) => reply,
        _ => unreachable!(),
    }
}

fn goals(fixture: &Fixture, inputs: &[DecodedFrame], faction: &str) -> FactionGoalsReply {
    let params = FactionStateParams::Goals {
        branch_id: inputs[0].branch.to_string(),
        faction: faction.into(),
        first_hour: None,
        last_hour: None,
    };
    match reply(fixture, inputs, params) {
        FactionStateReply::Goals(reply) => reply,
        _ => unreachable!(),
    }
}

fn hours(spans: &[HourSpan]) -> Vec<(i32, i32)> {
    spans
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect()
}

/// The held goal list at `hour`.
fn list_at(reply: &FactionGoalsReply, hour: i32) -> &FactionGoalList {
    let run = reply
        .runs
        .iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
        .expect("a goal list");
    &reply.lists[run.list as usize]
}

#[test]
fn members_leaving_and_joining_across_segment_starts_keep_their_associations() {
    let mut inputs: Vec<_> = (100..108).map(frame).collect();
    for input in &mut inputs[4..] {
        rows!(input, FactionMembers).retain(|row| row.country != 12);
        rows!(input, FactionManpower).retain(|row| row.country != 12);
    }
    for input in &mut inputs[6..] {
        rows!(input, FactionMembers).insert(
            1,
            FactionMemberRow {
                country: 12,
                faction: 10,
                ordinal: 1,
            },
        );
        rows!(input, FactionManpower).push(FactionManpowerRow {
            faction: 10,
            ordinal: 1,
            country: 12,
            value: 3,
        });
    }
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("membership", &inputs, &[3], &restarts));
    }
    let sections = last.unwrap();
    let nine = influence(&sections, &inputs, "9");
    let left = nine
        .accounts
        .iter()
        .find(|account| account.key == "4:8:102:BBB")
        .unwrap();
    assert_eq!(hours(&left.member), [(100, 103)]);
    assert_eq!(hours(&left.retained), [(104, 105)]);
    let ten = influence(&sections, &inputs, "10");
    let joined = ten
        .accounts
        .iter()
        .find(|account| account.key == "4:8:102:BBB")
        .unwrap();
    assert_eq!(hours(&joined.member), [(106, 107)]);
    assert!(joined.retained.is_empty());
    let pool = manpower(&sections, &inputs, "9");
    let total_at = |hour: i32| {
        pool.total
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .and_then(|run| run.value)
    };
    assert_eq!(total_at(103), Some(1110));
    assert_eq!(total_at(104), Some(1104));
}

#[test]
fn a_faction_formed_and_one_dissolved_read_alike_at_every_restart() {
    let mut inputs: Vec<_> = (100..106).map(frame).collect();
    for input in &mut inputs[3..] {
        for row in rows!(input, Factions).iter_mut() {
            if row.id == 10 {
                row.id = 11;
            }
        }
        for row in rows!(input, FactionUnlocks).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
        for row in rows!(input, FactionProjection).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
        for row in rows!(input, FactionGoalStatus).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
        for row in rows!(input, FactionRules).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
        for row in rows!(input, FactionManpower).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
        for row in rows!(input, FactionMembers).iter_mut() {
            if row.faction == 10 {
                row.faction = 11;
            }
        }
    }
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("formed and dissolved", &inputs, &[2], &restarts));
    }
    let sections = last.unwrap();
    let dissolved = goals(&sections, &inputs, "10");
    assert!(!list_at(&dissolved, 102).absent);
    assert!(list_at(&dissolved, 103).absent);
    assert!(list_at(&dissolved, 105).absent);
    let formed = goals(&sections, &inputs, "11");
    assert!(
        list_at(&formed, 102).absent,
        "a complete roster proves it absent"
    );
    assert!(!list_at(&formed, 103).absent);
    let ruled = rules(&sections, &inputs, "10");
    let rule = &ruled.rules[0];
    let last = rule.runs.last().unwrap();
    assert_eq!((last.first_hour, last.last_hour), (103, 105));
    assert_eq!(
        last.reason.as_deref(),
        Some("Faction rule owner is unavailable")
    );
    let account = influence(&sections, &inputs, "11")
        .accounts
        .into_iter()
        .find(|account| account.key == "5:9:103:CCC")
        .unwrap();
    assert_eq!(hours(&account.member), [(103, 105)]);
}

#[test]
fn a_new_leader_renumbers_the_pool_across_segment_starts() {
    let mut inputs: Vec<_> = (100..106).map(frame).collect();
    for input in &mut inputs[3..] {
        for row in rows!(input, FactionMembers).iter_mut() {
            row.ordinal = match row.country {
                11 => 1,
                12 => 0,
                _ => row.ordinal,
            };
        }
        for row in rows!(input, FactionManpower).iter_mut() {
            if row.faction == 9 {
                (row.country, row.value) = match row.ordinal {
                    0 => (12, 7),
                    _ => (11, 1000 + input.hour),
                };
            }
        }
    }
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("leader", &inputs, &[3], &restarts));
    }
    let pool = manpower(&last.unwrap(), &inputs, "9");
    let held = |key: &str| {
        let member = pool
            .members
            .iter()
            .find(|member| member.key == key)
            .unwrap_or_else(|| panic!("{key}"));
        (
            member.runs[0].first_hour,
            member.runs[member.runs.len() - 1].last_hour,
        )
    };
    assert_eq!(held("0:7:101:AAA"), (100, 102));
    assert_eq!(held("1:8:102:BBB"), (100, 102));
    assert_eq!(held("0:8:102:BBB"), (103, 105));
    assert_eq!(held("1:7:101:AAA"), (103, 105));
}

#[test]
fn rules_and_goals_changing_read_alike_at_every_restart() {
    let mut inputs: Vec<_> = (100..107).map(frame).collect();
    for input in &mut inputs[2..] {
        rows!(input, FactionRules)[0].token = 6;
    }
    for input in &mut inputs[4..] {
        let entries = rows!(input, FactionGoalEntries);
        entries[0].kind = 1;
        entries[0].status = 2;
    }
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("rules and goals", &inputs, &[4], &restarts));
    }
    let sections = last.unwrap();
    let ruled = rules(&sections, &inputs, "9");
    let rule = &ruled.rules[0];
    let token_at = |hour: i32| {
        rule.runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .and_then(|run| run.token.clone())
    };
    assert_eq!(token_at(101).as_deref(), Some("rule_a"));
    assert_eq!(token_at(102).as_deref(), Some("rule_f"));
    let nine = goals(&sections, &inputs, "9");
    assert_eq!(list_at(&nine, 103).goals.len(), 1);
    let done = list_at(&nine, 104);
    assert!(done.goals.is_empty());
    assert_eq!(done.completed.len(), 1);
    let progress = &nine.progress[0];
    assert_eq!(progress.runs[0].first_hour, 100);
    assert_eq!(progress.runs[progress.runs.len() - 1].last_hour, 103);
    assert_eq!(
        progress.runs.len(),
        4,
        "the progress of each hour it is held"
    );
}

#[test]
fn partial_and_unavailable_tables_read_alike_at_every_restart() {
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Factions,
        TableId::FactionMembers,
        TableId::FactionAccounts,
        TableId::FactionRules,
        TableId::FactionMinimumUpgrades,
        TableId::FactionDoctrine,
        TableId::FactionManpower,
        TableId::FactionGoalStatus,
        TableId::FactionGoalEntries,
        TableId::LexicalTokens,
    ] {
        for partial in [false, true] {
            let mut inputs: Vec<_> = (100..106).map(frame).collect();
            for input in &mut inputs[2..4] {
                let TableView::Complete(rows) = &*input.tables[table.index()] else {
                    unreachable!("a complete fixture table");
                };
                let refused = if partial {
                    TableView::Truncated {
                        rows: rows.clone(),
                        missing: 1,
                    }
                } else {
                    TableView::Unavailable {
                        reason: "synthetic refusal".into(),
                    }
                };
                *input.tables[table.index()] = refused;
            }
            for restarts in [vec![], vec![2], vec![3], (1..inputs.len()).collect()] {
                let sections = parity(
                    &format!("{table:?} partial {partial}"),
                    &inputs,
                    &[3],
                    &restarts,
                );
                if table == TableId::FactionRules && !partial {
                    let ruled = rules(&sections, &inputs, "9");
                    let rule = &ruled.rules[0];
                    let refused: Vec<_> = rule
                        .runs
                        .iter()
                        .filter(|run| {
                            run.reason.as_deref()
                                == Some("FactionRules is unavailable: synthetic refusal")
                        })
                        .map(|run| (run.first_hour, run.last_hour))
                        .collect();
                    assert_eq!(refused, [(102, 103)], "one refusal however segments fall");
                }
            }
        }
    }
}

#[test]
fn a_gap_breaks_every_lane_and_both_sides_resume_alike() {
    let inputs: Vec<_> = (100..104).chain(106..110).map(frame).collect();
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("gap", &inputs, &[2], &restarts));
    }
    let nine = influence(&last.unwrap(), &inputs, "9");
    let known = &nine
        .channels
        .iter()
        .find(|channel| channel.channel == "influence")
        .unwrap()
        .known;
    assert_eq!(hours(known), [(100, 103), (106, 109)]);
    assert_eq!(hours(&nine.unobserved), [(104, 105)]);
}

#[test]
fn another_branch_reads_alike_at_every_restart() {
    let inputs: Vec<_> = (100..104)
        .map(|hour| {
            let mut input = frame(hour);
            input.branch = 2;
            input
        })
        .collect();
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("branch", &inputs, &[2], &restarts));
    }
    let nine = influence(&last.unwrap(), &inputs, "9");
    assert_eq!(nine.accounts.len(), 2);
}

fn programme_reference(id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE)
}

/// Programme 17 read at 100 alone: faction 9 shares it from 103, after the read, and faction
/// 10 from 105.
fn programme_hour(hour: i32) -> DecodedFrame {
    let mut input = frame(hour);
    input.metrics.table_ns[TableId::ProjectPrograms.index()] = u64::from(hour == 100);
    let mut shared = Vec::new();
    for (faction, from) in [(9, 103), (10, 105)] {
        if hour >= from {
            shared.push(FactionProgramRow {
                faction,
                ordinal: 0,
                reference: programme_reference(17),
            });
        }
    }
    *input.tables[TableId::FactionPrograms.index()] =
        TableView::Complete(TableRows::FactionPrograms(shared));
    *input.tables[TableId::ProjectPrograms.index()] =
        TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
            id: 17,
            owner: 11,
            ..Default::default()
        }]));
    input
}

#[test]
fn programmes_read_before_a_restart_or_before_they_were_shared_read_alike() {
    let inputs: Vec<_> = (100..110).map(programme_hour).collect();
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity("programmes", &inputs, &[4], &restarts));
    }
    let sections = last.unwrap();
    for (faction, from) in [("9", 103), ("10", 105)] {
        let ruled = rules(&sections, &inputs, faction);
        let runs = &ruled.programmes[0].runs;
        assert_eq!(runs[0].first_hour, from, "{faction}");
        assert_eq!(runs[runs.len() - 1].last_hour, 109, "{faction}");
        for run in runs {
            assert_eq!(
                run.token.as_deref(),
                Some(programme_reference(17).to_string()).as_deref(),
                "{faction}"
            );
            assert_eq!(run.reason, None, "{faction}: the read at 100 supplies it");
        }
    }
}

const NAMED_GOALS: &str = "mod_scripted = { name = MOD_SCRIPTED category = long_term }\n";
const NAMED_TEXTS: &str = "\u{feff}l_english:\n rule_a:0 \"[ROOT.GetTag] rule\"\n MOD_SCRIPTED:0 \"[ROOT.GetTag] pact goal\"\n";

/// Rule `rule_a` named by the viewer, and goal `mod_scripted` titled by it: 101 (AAA) views
/// until 103, 103 (CCC) from 104, renamed DDD from 106.
fn named_hour(hour: i32) -> DecodedFrame {
    let mut input = frame(hour);
    *input.tables[TableId::LocalPlayer.index()] =
        TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
            country: if hour < 104 { 101 } else { 103 },
            observer_country: 0,
        }]));
    if hour >= 106 {
        rows!(input, Countries)[2].tag = b"DDD".to_vec();
    }
    for row in rows!(input, LexicalTokens).iter_mut() {
        if row.token == 5 {
            row.literal = b"mod_scripted".to_vec();
        }
    }
    input
}

#[test]
fn dynamic_names_and_scripted_goal_texts_read_alike_at_every_restart() {
    let machine = fixtures::visual_mods::Machine::new("faction_columnar_names");
    machine.write("common/factions/goals/mod.txt", NAMED_GOALS.as_bytes());
    machine.write(
        "localisation/english/mod_l_english.yml",
        NAMED_TEXTS.as_bytes(),
    );
    let inputs: Vec<_> = (100..109).map(named_hour).collect();
    let (source, options) = machine.open_frames(inputs.clone());
    let context = LoadContext {
        content: Some(options),
    };
    let mut last = None;
    for restarts in placements(&inputs) {
        last = Some(parity_in(
            "names",
            &source,
            &context,
            &inputs,
            (&[3], &restarts),
            None,
        ));
    }
    let sections = last.unwrap();
    let ruled = rules(&sections, &inputs, "9");
    let named_at = |hour: i32| {
        ruled.rules[0]
            .runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .map(|run| (run.name.clone(), run.reason.clone()))
    };
    assert!(named_at(103).is_some());
    assert_ne!(named_at(103), named_at(104), "the viewer changed");
    assert_ne!(named_at(104), named_at(106), "the viewer was renamed");
    assert_eq!(named_at(104), named_at(105));
    let nine = goals(&sections, &inputs, "9");
    assert_ne!(
        nine.runs[0].list,
        nine.runs[nine.runs.len() - 1].list,
        "the goal's title follows its viewer"
    );
}

/// [`named_hour`] with faction 9's pool and goal progress still: until the viewer changes at
/// 104 and is renamed at 106, every hour moves only fields no lane and no dynamic text reads.
fn quiet_named_hour(hour: i32) -> DecodedFrame {
    let mut input = named_hour(hour);
    for row in rows!(input, FactionManpower).iter_mut() {
        if row.faction == 9 && row.ordinal == 0 {
            row.value = 1000;
        }
    }
    rows!(input, FactionGoalEntries)[0].progress_raw = 0;
    input
}

#[test]
fn held_dynamic_texts_let_an_hour_moving_only_unread_fields_idle_without_a_trigger_frame() {
    let machine = fixtures::visual_mods::Machine::new("faction_columnar_quiet_names");
    machine.write("common/factions/goals/mod.txt", NAMED_GOALS.as_bytes());
    machine.write(
        "localisation/english/mod_l_english.yml",
        NAMED_TEXTS.as_bytes(),
    );
    let inputs: Vec<_> = (100..109).map(quiet_named_hour).collect();
    let (source, options) = machine.open_frames(inputs.clone());
    let context = LoadContext {
        content: Some(options),
    };
    for restarts in [vec![], vec![2], vec![5]] {
        parity_in("quiet names", &source, &context, &inputs, (&[3], &restarts), None);
    }
    let (_, evaluated, built) = projected(&source, &context, &inputs);
    assert_eq!(evaluated, 3, "the start, the viewer and its renaming");
    let at = |hour: i32| built[(hour - 100) as usize];
    assert!(at(100) > 0, "the first hour evaluates the held texts");
    for hour in [101, 102, 103, 105, 107, 108] {
        assert_eq!(
            at(hour),
            at(hour - 1),
            "{hour}: the held texts' reads did not move"
        );
    }
    assert!(at(104) > at(103), "the viewer changed");
    assert!(at(106) > at(105), "the viewer was renamed");
}

#[test]
fn an_emit_reloaded_mid_segment_publishes_as_one_never_reloaded() {
    let mut inputs: Vec<_> = (100..110).map(frame).collect();
    for input in &mut inputs[4..] {
        rows!(input, FactionMembers).retain(|row| row.country != 12);
        rows!(input, FactionManpower).retain(|row| row.country != 12);
    }
    for input in &mut inputs[7..] {
        rows!(input, FactionRules)[0].token = 6;
    }
    let source = source(&inputs);
    for restarts in [vec![], vec![4], vec![2, 7]] {
        for reload in [102, 105, 108] {
            parity_in(
                "reload",
                &source,
                &LoadContext { content: None },
                &inputs,
                (&[3], &restarts),
                Some(reload),
            );
        }
    }
}

const WORLD: usize = 1_500;

/// 1,500 countries, the first half members of faction 9 and the rest of faction 10, each
/// with its account and pooled manpower. Every hour moves an unread country and account
/// field of every one; 102 moves account 7's share alone, and 104 renames country 5.
fn world_hour(hour: i32) -> DecodedFrame {
    let mut input = frame(hour);
    rows!(input, FactionGoalEntries)[0].progress_raw = 0;
    let faction = |index: usize| if index < WORLD / 2 { 9 } else { 10 };
    let ordinal = |index: usize| (index % (WORLD / 2)) as u32;
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..WORLD)
            .map(|index| CountryRow {
                id: 1_000 + index as u32,
                canonical_slot: 1_000 + index as u32,
                tag: if index == 5 && hour >= 104 {
                    b"RENAMED".to_vec()
                } else {
                    format!("C{index:04}").into_bytes()
                },
                political_power_raw: Some(i64::from(hour)),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..WORLD)
            .map(|index| CountryLinkRow {
                id: 1_000 + index as u32,
                canonical_slot: 1_000 + index as u32,
            })
            .collect(),
    ));
    *input.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            (0..WORLD)
                .map(|index| FactionMemberRow {
                    country: 1_000 + index as u32,
                    faction: faction(index),
                    ordinal: ordinal(index),
                })
                .collect(),
        ));
    *input.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(
            (0..WORLD)
                .map(|index| FactionAccountRow {
                    account: index as u32,
                    country: 1_000 + index as u32,
                    influence_refresh: 1,
                    influence_numerator_raw: if index == 7 && hour >= 102 { 2 } else { 1 },
                    influence_denominator_raw: WORLD as i64,
                    influence_faction: faction(index),
                    statistic_raw: i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ));
    *input.tables[TableId::FactionManpower.index()] =
        TableView::Complete(TableRows::FactionManpower(
            (0..WORLD)
                .map(|index| FactionManpowerRow {
                    faction: faction(index),
                    ordinal: ordinal(index),
                    country: 1_000 + index as u32,
                    value: index as i32,
                })
                .collect(),
        ));
    input
}

/// One segment's projection of `inputs` read with `context`, from a run's start; how many hours
/// it evaluated, and how many frames for dynamic texts it had built after each hour.
fn projected(
    source: &Source,
    context: &LoadContext,
    inputs: &[DecodedFrame],
) -> (Vec<columnar::Hour>, usize, Vec<usize>) {
    let branch = inputs[0].branch;
    let mut walk = Synthetic::new(columnar::tables(), branch, inputs[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut segment = projector(source, context);
    segment
        .start(Start::Run { continues: false }, branch, inputs[0].hour)
        .unwrap();
    let mut hours: Vec<columnar::Hour> = Vec::new();
    let mut built = Vec::with_capacity(inputs.len());
    for (ordinal, input) in inputs.iter().enumerate() {
        write_hour(&mut walk, input);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &crate::products::faction_state::columnar::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            segment
                .hour(
                    &segments::Hour {
                        branch,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: ordinal == 0,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        built.push(segment.trigger_frames());
    }
    segment.end(&mut hours).unwrap();
    (hours, segment.evaluated, built)
}

#[test]
fn fifteen_hundred_countries_evaluate_only_the_hours_and_rows_that_changed() {
    let inputs: Vec<_> = (100..106).map(world_hour).collect();
    for restarts in [vec![], vec![2], vec![3, 5]] {
        parity("1,500 countries", &inputs, &[3], &restarts);
    }
    let source = source(&inputs);
    let (hours, evaluated, _) = projected(&source, &LoadContext { content: None }, &inputs);
    assert_eq!(evaluated, 3, "the start, the share and the renaming");
    for hour in &hours {
        let ops = (
            hour.projected.accounts.ops.len(),
            hour.projected.factions.len(),
        );
        match hour.hour {
            101 | 103 | 105 => assert_eq!(ops, (0, 0), "{}: nothing read moved", hour.hour),
            102 => {
                assert_eq!(ops, (1, 0), "102: one account's share");
                assert!(matches!(
                    hour.projected.accounts.ops[0],
                    super::projection::AccountOp::Share { account: 7, .. }
                ));
            }
            _ => {}
        }
    }
    let nine = influence(&parity("1,500 countries", &inputs, &[], &[]), &inputs, "9");
    assert_eq!(
        nine.accounts.len(),
        WORLD / 2 + 1,
        "a renamed country's account is new"
    );
}
