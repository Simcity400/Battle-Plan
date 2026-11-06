use super::facts::{Captures, CensusRow, Cost, Dating, Hour, Power};
use super::*;
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{format::FormatVersion, scope::CaptureMode};
use serde_json::json;

const NONE: PreparedContent = Ok(None);

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [4; 32],
        content: [5; 32],
        branch: [6; 32],
        schema: emit::SCHEMA,
    }
}

/// Countries `(raw id, tag)`, each its own canonical slot and link.
fn census(countries: &[(u32, &str)]) -> Arc<[CensusRow]> {
    countries
        .iter()
        .map(|&(id, tag)| CensusRow {
            id,
            slot: id,
            tag: Arc::from(tag.as_bytes()),
        })
        .collect()
}

fn links(countries: &[(u32, &str)]) -> Arc<[(u32, u32)]> {
    countries.iter().map(|&(id, _)| (id, id)).collect()
}

fn capture(country: u32, since: i32, through: i32) -> PoliticsCaptureCoverageRow {
    PoliticsCaptureCoverageRow {
        country,
        instance: 1,
        epoch: 1,
        since_hour: since,
        through_hour: through,
        streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
    }
}

const ALL: u16 = 0xff;

/// The first hour of a walk: the census, links and every capture handed whole.
fn opening(at: i32, countries: &[(u32, &str)]) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        whole: true,
        complete: ALL,
        census: Some(census(countries)),
        links: Some(links(countries)),
        captures: Captures {
            changed: countries
                .iter()
                .map(|&(id, _)| capture(id, at, at))
                .collect(),
            ..Captures::default()
        },
        ..Hour::default()
    }
}

/// An hour that only extends every capture.
fn steady(at: i32, countries: &[(u32, &str)]) -> Hour {
    Hour {
        branch: 1,
        hour: at,
        complete: ALL,
        captures: Captures {
            extended: countries.iter().map(|&(id, _)| id).collect(),
            ..Captures::default()
        },
        ..Hour::default()
    }
}

fn decision(country: u32, sequence: u64, hour: i32, charge: i64) -> Power {
    Power {
        row: CountryPowerChangeRow {
            country,
            instance: 1,
            sequence,
            epoch: 1,
            hour,
            currency: PowerCurrency::Political,
            operation: PowerOperation::Subtract,
            balance_before_raw: 10_000_000,
            balance_after_raw: 10_000_000 - charge,
            requested_delta_raw: Some(-charge),
            cause: PowerCause::Decision,
            site: PowerSite::AddPoliticalPower,
            identity: PowerIdentity {
                item_key: b"synthetic_decision".to_vec(),
                ..Default::default()
            },
        },
        dating: Dating {
            dated: None,
            category: Some(Arc::from("decisions")),
        },
        resolved: None,
        shape: None,
    }
}

fn daily_gain(country: u32, sequence: u64, day: i32, gain: i64) -> Power {
    Power {
        row: CountryPowerChangeRow {
            country,
            instance: 1,
            sequence,
            epoch: 1,
            hour: day,
            currency: PowerCurrency::Political,
            operation: PowerOperation::Add,
            balance_before_raw: 0,
            balance_after_raw: gain,
            requested_delta_raw: Some(gain),
            cause: PowerCause::DailyGain,
            site: PowerSite::AddPoliticalPowerNoDeficit,
            identity: PowerIdentity::default(),
        },
        dating: Dating {
            dated: None,
            category: None,
        },
        resolved: None,
        shape: None,
    }
}

fn daily_row(country: u32, day: i32, sequence: u64, cost: i64) -> CountryPowerDailyRow {
    CountryPowerDailyRow {
        country,
        instance: 1,
        epoch: 1,
        day_hour: day,
        pp_gross_gain_raw: Some(200_000),
        pp_cost_raw: Some(cost),
        pp_net_gain_raw: Some(200_000 - cost),
        pp_realized_delta_raw: Some(200_000 - cost),
        pp_sequence: Some(sequence),
        cp_gain_raw: Some(50_000),
        cp_realized_delta_raw: Some(50_000),
        cp_sequence: Some(sequence + 1),
    }
}

fn upkeep_cost(country: u32, day: i32, sequence: u64, cost: i64) -> Cost {
    Cost {
        row: CountryPowerDailyCostRow {
            country,
            instance: 1,
            ordinal: 0,
            epoch: 1,
            day_hour: day,
            pp_sequence: sequence,
            component: PowerDailyCostKind::Relation,
            cost_raw: cost,
            cause: PowerCause::DailyGain,
            site: PowerSite::AddPoliticalPowerNoDeficit,
            identity: PowerIdentity::default(),
        },
        dating: Dating {
            dated: None,
            category: Some(Arc::from("dailyCosts")),
        },
        item: None,
        shape: None,
    }
}

/// A publication built hour by hour, cut after each of `cuts`' hours; at each of `resumes` the
/// emit is saved and loaded again and the writer resumed from the root.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<Hour>, cuts: &[i32], resumes: &[i32]) -> Built {
    build_with(&NONE, hours, cuts, resumes)
}

/// [`build`] under `content`.
fn build_with(
    content: &PreparedContent,
    hours: Vec<Hour>,
    cuts: &[i32],
    resumes: &[i32],
) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = emit::Emit::new(content);
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resumes.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
            emit = emit::Emit::load(content, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn ask(built: &Built, params: serde_json::Value) -> io::Result<serde_json::Value> {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity())?;
    let directory = Directory::read(&mut snapshot, emit::SCHEMA)?;
    query::query(&params, &mut snapshot, &directory, &AtomicBool::new(false))
}

fn reply(built: &Built, params: serde_json::Value) -> PoliticsLedgerReply {
    serde_json::from_value(ask(built, params).unwrap()).unwrap()
}

/// GER is captured from 100. A decision is read at 101, the day's gain, its daily row and
/// one recorded cost at the day 120, and a second decision at 125.
fn campaign() -> Vec<Hour> {
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(100, &countries)];
    for at in 101..=130 {
        let mut hour = steady(at, &countries);
        match at {
            101 => hour.power.push(decision(1, 1, 101, 1_500_000)),
            120 => {
                hour.power.push(daily_gain(1, 2, 120, 150_000));
                hour.daily.push(daily_row(1, 120, 2, 50_000));
                hour.costs.push(upkeep_cost(1, 120, 2, 50_000));
            }
            121 => hour.deleted[0].push((1_u128 << 64) | 2),
            125 => hour.power.push(decision(1, 3, 125, 2_000_000)),
            _ => {}
        }
        hours.push(hour);
    }
    hours
}

/// Each recorded row comes back once, as its read saw it: decisions as spends and balance
/// lines, the gain as a balance line only, the cost as a recurring day; governments ask none.
#[test]
fn payments_come_back_as_their_reads_recorded_them() {
    let built = build(campaign(), &[110, 130], &[]);
    let raw = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "balance", "hour": 130 }),
    );
    assert_eq!(
        raw.balance_changes
            .iter()
            .map(|row| (row.hour, row.read_hour, row.currency.as_str()))
            .collect::<Vec<_>>(),
        vec![(101, 101, "Political"), (120, 120, "Political"), (125, 125, "Political")]
    );
    assert_eq!(
        raw.spending
            .iter()
            .map(|spend| (spend.hour, spend.charge_raw.as_deref()))
            .collect::<Vec<_>>(),
        vec![(101, Some("1500000")), (125, Some("2000000"))]
    );
    assert!(
        raw.spending
            .iter()
            .all(|spend| spend.category == "decisions")
    );
    // One item for both decisions: the first decision's id names it.
    assert_eq!(raw.spend_items.len(), 2);
    assert_eq!(raw.daily.len(), 1);
    assert_eq!(raw.daily[0].pp_read_hour, Some(120));
    assert_eq!(raw.recurring.len(), 1);
    assert_eq!(raw.recurring[0].spend.charge_raw.as_deref(), Some("50000"));
    // Without content each payment says why its item is not named, once per payment.
    assert!(
        raw.warnings
            .iter()
            .any(|warning| warning == "recorded content has not been resolved (count: 3)"),
        "{:?}",
        raw.warnings
    );

    let early = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "spending", "hour": 110 }),
    );
    assert_eq!(early.spending.len(), 1);
    assert!(early.recurring.is_empty());
    assert!(early.balance_changes.is_empty() && early.daily.is_empty());

    let governments = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "governments", "hour": 130 }),
    );
    assert!(governments.governments.is_empty());
    assert!(governments.spending.is_empty());

    // A tag the branch never named answers empty, not refused.
    let absent = reply(
        &built,
        json!({ "branchId": "1", "tag": "FRA", "view": "balance", "hour": 130 }),
    );
    assert!(absent.balance_changes.is_empty() && absent.spending.is_empty());
}

/// Content that does not load is the branch's warning once, across a resume, and the ledger
/// keeps every payment its reads recorded.
#[test]
fn a_content_refusal_warns_once_and_keeps_the_payments() {
    let reason = "NDefines.NCharacter.OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU is missing".to_owned();
    let refused: PreparedContent = Err(reason.clone());
    let built = build_with(&refused, campaign(), &[110, 130], &[110]);
    let raw = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "balance", "hour": 130 }),
    );
    assert_eq!(raw.balance_changes.len(), 3);
    assert_eq!(
        raw.warnings
            .iter()
            .filter(|warning| warning.contains(&reason))
            .collect::<Vec<_>>(),
        [&format!("{reason} (count: 1)")]
    );
}

/// The emit saved at a cut and loaded again over the publication goes on as if it had not
/// stopped, whatever the next segment's first hour hands whole.
#[test]
fn accounting_goes_on_across_a_cut_and_resume() {
    let straight = build(campaign(), &[110, 130], &[]);
    let mut hours = campaign();
    // The next segment's first hour hands every capture and table whole.
    let countries = [(1, "GER"), (2, "ITA")];
    let at = hours.iter().position(|hour| hour.hour == 111).unwrap();
    hours[at] = Hour {
        captures: Captures {
            changed: countries
                .iter()
                .map(|&(id, _)| capture(id, 100, 111))
                .collect(),
            ..Captures::default()
        },
        census: Some(census(&countries)),
        links: Some(links(&countries)),
        whole: true,
        ..steady(111, &countries)
    };
    let resumed = build(hours, &[110, 130], &[110]);
    for view in ["spending", "balance", "governments"] {
        for hour in [105, 120, 130] {
            let params = json!({ "branchId": "1", "tag": "GER", "view": view, "hour": hour });
            assert_eq!(
                ask(&straight, params.clone()).unwrap(),
                ask(&resumed, params).unwrap(),
                "{view} at {hour}"
            );
        }
    }
}

/// A reply past the response budget is refused whole: nothing is truncated.
#[test]
fn a_reply_past_the_budget_is_refused_not_truncated() {
    let countries = [(1, "GER")];
    let mut hours = vec![opening(0, &countries)];
    let mut sequence = 0;
    for at in 1..=1000 {
        let mut hour = steady(at, &countries);
        for _ in 0..100 {
            sequence += 1;
            hour.power.push(decision(1, sequence, at, 1_000));
        }
        if at > 1 {
            let start = sequence - 199;
            hour.deleted[0] = (start..start + 100)
                .map(|key| (1_u128 << 64) | u128::from(key))
                .collect();
        }
        hours.push(hour);
    }
    let built = build(hours, &[1000], &[]);
    let error = ask(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "balance", "hour": 1000 }),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("complete-response budget"),
        "{error}"
    );
    // A window inside the budget is answered.
    let window = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "spending", "from": 1, "hour": 3 }),
    );
    assert_eq!(window.spending.len(), 300);
}

/// 1,500 countries over 120 hours, each extending its capture every hour and a few paying: the
/// emit writes each country's coverage once, and every country's payments read back.
#[test]
fn fifteen_hundred_countries_append_only_what_changes() {
    let tags: Vec<String> = (0..1500).map(|index| format!("C{index:03X}")).collect();
    let countries: Vec<(u32, &str)> = tags
        .iter()
        .enumerate()
        .map(|(index, tag)| (index as u32 + 1, tag.as_str()))
        .collect();
    let mut hours = vec![opening(0, &countries)];
    let mut sequence = 0;
    for at in 1..120 {
        let mut hour = steady(at, &countries);
        for change in 0..5 {
            let country = (at as u32 * 37 + change * 11) % 1500 + 1;
            sequence += 1;
            hour.power.push(decision(country, sequence, at, 1_000));
        }
        hours.push(hour);
    }
    let built = build(hours, &[23, 47, 71, 95, 119], &[71]);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, emit::SCHEMA).unwrap();
    // One actor per country and the branch's.
    assert_eq!(directory.actors.len(), 1501);
    for (id, tag) in countries.iter().step_by(137) {
        let reply = reply(
            &built,
            json!({ "branchId": "1", "tag": tag, "view": "spending", "hour": 119 }),
        );
        let paid = (1..120)
            .flat_map(|at: i32| {
                (0..5).map(move |change| (at, (at as u32 * 37 + change * 11) % 1500 + 1))
            })
            .filter(|&(_, country)| country == *id)
            .count();
        assert_eq!(reply.spending.len(), paid, "{tag}");
        assert_eq!(reply.spending_coverage.len(), 1, "{tag}");
        assert!(reply.spending_coverage[0].political, "{tag}");
    }
}

/// Hours 106 to 109 were never prepared: they read as unknown, not as zero, and nothing is
/// counted or totalled over them.
#[test]
fn a_hole_reads_unknown_never_zero() {
    let countries = [(1, "GER")];
    let mut hours = vec![opening(100, &countries)];
    for at in 101..=105 {
        hours.push(steady(at, &countries));
    }
    let mut after = opening(110, &countries);
    after.captures.changed = vec![capture(1, 110, 110)];
    hours.push(after);
    for at in 111..=115 {
        hours.push(steady(at, &countries));
    }
    let built = build(hours, &[105, 115], &[]);
    let raw = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "balance", "hour": 115 }),
    );
    let gap = raw
        .spending_coverage
        .iter()
        .find(|row| row.since_hour == Some(106))
        .expect("the hole has its own interval");
    assert_eq!(gap.through_hour, Some(109));
    assert!(!gap.political && !gap.command);
    assert_eq!(gap.reason.as_deref(), Some("politics coverage unavailable"));
    assert!(raw.daily.is_empty());
    assert!(raw.gains.is_empty());
}

/// A tag the census lists only later reads absent from the census's first listing, not unknown:
/// none of its spending is recorded because it spends none.
#[test]
fn a_tag_listed_later_reads_absent_before_its_listing() {
    let (first, both) = ([(1, "GER")], [(1, "GER"), (2, "ITA")]);
    let mut hours = vec![opening(100, &first)];
    for at in 101..=109 {
        hours.push(steady(at, &first));
    }
    hours.push(opening(110, &both));
    for at in 111..=120 {
        hours.push(steady(at, &both));
    }
    let built = build(hours, &[105, 120], &[105]);
    let raw = reply(
        &built,
        json!({ "branchId": "1", "tag": "ITA", "view": "spending", "hour": 120 }),
    );
    let before = &raw.spending_coverage[0];
    assert_eq!(
        (before.since_hour, before.through_hour),
        (Some(100), Some(109))
    );
    assert!(before.political && before.command);
    assert_eq!(before.reason, None);
    assert!(
        raw.spending_coverage
            .iter()
            .any(|row| row.since_hour == Some(110)),
        "{:?}",
        raw.spending_coverage
    );
}

/// The projector on a synthetic walk: the first hour hands every row whole; later hours only
/// the rows they wrote and the captures they only extended, as one list kept while it stays.
#[test]
fn the_projector_hands_written_rows_and_extended_captures() {
    let tables = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::PoliticsCaptureCoverage,
        TableId::CountryPowerChanges,
        TableId::GovernmentChanges,
        TableId::CountryPowerDaily,
        TableId::CountryPowerDailyCosts,
        TableId::CountryCommandPower,
    ];
    fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: &[R]) {
        walk.table(
            table,
            Input::Snapshot {
                rows,
                partial: None,
                read: true,
                sample: Sample::FromReadBit,
            },
        )
        .unwrap();
    }
    let countries = [("GER", 1_u32), ("ITA", 2)].map(|(tag, id)| CountryRow {
        id,
        canonical_slot: id,
        tag: tag.as_bytes().to_vec(),
        ..Default::default()
    });
    let links = [1_u32, 2].map(|id| CountryLinkRow {
        id,
        canonical_slot: id,
    });
    let mut walk = Synthetic::new(&tables, 1, 100).unwrap();
    let mut held: Option<dashboard_world::World> = None;
    let mut projector = facts::Projector::new(FormatVersion::CURRENT, &NONE).unwrap();
    let mut out: Vec<Hour> = Vec::new();
    let needs = dashboard_world::Needs {
        countries: true,
        ..Default::default()
    };
    for ordinal in 0..4_u32 {
        let at = 100 + ordinal as i32;
        write(&mut walk, TableId::Countries, &countries[..]);
        write(&mut walk, TableId::CountryLinks, &links[..]);
        let captures: Vec<_> = [1_u32, 2].map(|id| capture(id, 100, at)).to_vec();
        write(&mut walk, TableId::PoliticsCaptureCoverage, &captures[..]);
        let power: Vec<CountryPowerChangeRow> = if ordinal == 2 {
            vec![daily_gain(1, 7, at, 100_000).row]
        } else {
            Vec::new()
        };
        write(&mut walk, TableId::CountryPowerChanges, &power[..]);
        write::<GovernmentChangeRow>(&mut walk, TableId::GovernmentChanges, &[]);
        write::<CountryPowerDailyRow>(&mut walk, TableId::CountryPowerDaily, &[]);
        write::<CountryPowerDailyCostRow>(&mut walk, TableId::CountryPowerDailyCosts, &[]);
        let command = [CountryCommandPowerRow {
            canonical_slot: 1,
            allocated_command_power_raw: if ordinal == 3 { 5_000 } else { 0 },
            ..Default::default()
        }];
        write(&mut walk, TableId::CountryCommandPower, &command[..]);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut held,
                starts,
                &needs,
                (FormatVersion::CURRENT, CaptureMode::Full),
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
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    assert_eq!(out.len(), 4);
    assert!(out[0].whole);
    assert_eq!(out[0].census.as_ref().map(|census| census.len()), Some(2));
    assert_eq!(out[0].captures.changed.len(), 2);
    assert!(
        out[1..]
            .iter()
            .all(|hour| !hour.whole && hour.census.is_none())
    );
    // Later hours only extend both captures, one list while it stays the same.
    assert!(out[1].captures.changed.is_empty());
    assert_eq!(&*out[1].captures.extended, &[1, 2]);
    assert!(Arc::ptr_eq(
        &out[1].captures.extended,
        &out[2].captures.extended
    ));
    // The gain is handed at its hour, and deleted at the next.
    assert_eq!(out[2].power.len(), 1);
    assert_eq!(out[3].deleted[0], vec![(1_u128 << 64) | 7]);
    // A gain resolves no item: no frame is read for it.
    assert!(out[2].power[0].dating.dated.is_none());
    assert_eq!(out[3].command, vec![(1, Some(5_000))]);
}

/// Day `day`'s first hour.
fn day(day: i32) -> i32 {
    day * 24
}

/// GER (1) and ITA (2) captured from day 100 to day 170, hour by hour. Each GER day reads its
/// daily row and one recurring cost at its first hour, a decision at its sixth hour on every
/// third day, and an ability's upkeep at its eighth. Day 140's cost has no application; day
/// 145 also reads a credit; day 162 reads day 150's daily row again with a second cost, late.
fn long_campaign() -> Vec<Hour> {
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(day(100), &countries)];
    let mut sequence = 1_000;
    for at in day(100) + 1..day(170) {
        let mut hour = steady(at, &countries);
        let (today, offset) = (at.div_euclid(24), at.rem_euclid(24));
        if offset == 0 {
            sequence += 2;
            hour.daily.push(daily_row(1, at, sequence, 50_000));
            let mut cost = upkeep_cost(1, at, sequence, 40_000 + i64::from(today));
            if today == 140 {
                cost.row.pp_sequence = 7;
            }
            hour.costs.push(cost);
            if today == 145 {
                let mut credit = upkeep_cost(1, at, sequence, -5_000);
                credit.row.ordinal = 1;
                hour.costs.push(credit);
            }
            if today == 162 {
                hour.daily.push(daily_row(1, day(150), 4_242, 50_000));
                let mut late = upkeep_cost(1, day(150), 4_242, 9_000);
                late.row.ordinal = 2;
                hour.costs.push(late);
            }
        }
        if offset == 5 && today % 3 == 0 {
            sequence += 1;
            hour.power
                .push(decision(1, sequence, at, 1_000_000 + i64::from(today)));
        }
        if offset == 7 {
            sequence += 1;
            hour.power.push(ability_upkeep(1, sequence, at, 2_000));
        }
        hours.push(hour);
    }
    hours
}

fn ability_upkeep(country: u32, sequence: u64, hour: i32, charge: i64) -> Power {
    let mut power = decision(country, sequence, hour, charge);
    power.row.currency = PowerCurrency::Command;
    power.row.cause = PowerCause::AbilityUpkeep;
    power.row.identity.item_key = b"synthetic_ability".to_vec();
    power.dating.category = Some(Arc::from("upkeep"));
    power
}

/// `params`' totals, read with rollups when `rolled`, and the rollups read.
fn totals_reply(
    built: &Built,
    params: serde_json::Value,
    rolled: bool,
) -> (serde_json::Value, usize) {
    query::ROLLUPS.with(|cell| cell.set((rolled, 0)));
    let reply = ask(built, params).unwrap();
    let read = query::ROLLUPS.with(|cell| cell.replace((true, 0)).1);
    (reply, read)
}

/// Totals read from a settled period's rollup are the totals of the rows it counts, whatever
/// late rows, credits, missing applications, upkeep and decisions the period holds, across a
/// cut and a resume.
#[test]
fn totals_from_rollups_are_the_totals_of_their_rows() {
    let cuts: Vec<i32> = (110..170).step_by(10).map(|at| day(at) - 1).collect();
    let mut cuts = cuts;
    cuts.push(day(170) - 1);
    let built = build(long_campaign(), &cuts, &[day(150) - 1]);
    let mut used = 0;
    for hour in [day(150), day(165), day(170) - 1] {
        for within in [
            None,
            Some(json!([{ "firstHour": day(101) + 3, "lastHour": day(169) }])),
        ] {
            let mut params = json!({
                "branchId": "1",
                "tag": "GER",
                "view": "spending",
                "totals": true,
                "hour": hour,
            });
            if let Some(within) = within {
                params["within"] = within;
            }
            let (rolled, read) = totals_reply(&built, params.clone(), true);
            let (rows, none) = totals_reply(&built, params, false);
            assert_eq!(none, 0);
            assert_eq!(rolled, rows, "totals through {hour}");
            used += read;
        }
    }
    // Days 128 to 159 settle into one rollup, which the later hours read.
    assert!(used >= 4, "{used} rollups read");
    let reply: PoliticsLedgerReply = serde_json::from_value(
        totals_reply(
            &built,
            json!({
                "branchId": "1", "tag": "GER", "view": "spending", "totals": true,
                "hour": day(170) - 1,
            }),
            true,
        )
        .0,
    )
    .unwrap();
    // The late cost is its own day; the credit and the unmatched cost are warnings.
    assert!(reply.recurring.iter().any(|row| row.days == 1
        && row.spend.hour == day(150)
        && row.spend.charge_raw.as_deref() == Some("9000")));
    assert!(reply.warnings.iter().any(|warning| {
        warning.starts_with("Daily political power cost: matched PP application is unavailable")
    }));
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Negative daily political power cost component"))
    );
    // Every ability's upkeep is one Command item: its settled days one subtotal, its open
    // last day its own.
    let upkeep: Vec<_> = reply
        .recurring
        .iter()
        .filter(|row| row.spend.currency == "Command")
        .collect();
    assert_eq!(upkeep.len(), 2);
    assert!(
        upkeep
            .iter()
            .all(|row| row.spend.item_id == upkeep[0].spend.item_id)
    );
    let settled = upkeep
        .iter()
        .find(|row| row.days == 69)
        .expect("the settled days");
    assert_eq!(settled.known_raw, (2_000 * 69).to_string());
}

/// The game charges a day's daily costs once, at the day's application: a day with other
/// payments at later hours counts its cost once in its parts, however many hours change.
#[test]
fn a_days_cost_counts_once_in_its_parts() {
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(day(100), &countries)];
    for at in day(100) + 1..day(102) + 12 {
        let mut hour = steady(at, &countries);
        if at == day(101) {
            hour.daily.push(daily_row(1, at, 11, 50_000));
            hour.costs.push(upkeep_cost(1, at, 11, 50_000));
        }
        if at > day(101) && at < day(101) + 12 {
            hour.power.push(decision(1, 100 + at as u64, at, 1_000));
        }
        if at == day(102) {
            hour.daily.push(daily_row(1, at, 13, 50_000));
        }
        hours.push(hour);
    }
    let last = day(102) + 11;
    let built = build(hours, &[day(101) + 6, last], &[]);
    let reply = reply(
        &built,
        json!({
            "branchId": "1",
            "tag": "GER",
            "view": "balance",
            "hour": last,
            "summary": {
                "from": day(101),
                "to": day(101) + 23,
                "width": 24,
                "currency": "Political",
                "mode": "spending",
            },
        }),
    );
    let costs = reply
        .part_summaries
        .iter()
        .find(|part| part.id == "dailyCosts")
        .expect("the day's cost category");
    let most = costs
        .summary
        .buckets
        .iter()
        .flatten()
        .map(|bucket| bucket.max_value)
        .fold(f64::MIN, f64::max);
    assert_eq!(most, 0.5, "{:?}", costs.summary.buckets);
}

/// A part's chart joins its pieces exactly: a bin holding hours before the category's first
/// payment (known zero) and after it shows both, and bound countries sharing an hour are
/// refused rather than shown as one series.
#[test]
fn part_summaries_join_their_pieces_exactly() {
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(100, &countries)];
    for at in 101..=140 {
        let mut hour = steady(at, &countries);
        if at == 121 {
            hour.power.push(decision(1, 1, at, 1_500_000));
            hour.power.push(decision(2, 2, at, 1_500_000));
        }
        hours.push(hour);
    }
    let built = build(hours, &[140], &[]);
    let summary = |bindings: serde_json::Value| {
        ask(
            &built,
            json!({
                "branchId": "1",
                "tag": "GER",
                "view": "balance",
                "hour": 140,
                "bindings": bindings,
                "summary": { "from": 100, "to": 140, "width": 5, "currency": "Political", "mode": "spending" },
            }),
        )
    };
    let reply: PoliticsLedgerReply = serde_json::from_value(
        summary(json!({ "GER": [{ "firstHour": 100, "lastHour": 140 }] })).unwrap(),
    )
    .unwrap();
    let decisions = reply
        .part_summaries
        .iter()
        .find(|part| part.id == "decisions")
        .expect("the decisions part");
    let straddling = decisions
        .summary
        .buckets
        .iter()
        .flatten()
        .find(|bucket| bucket.first_hour < 121 && bucket.last_hour >= 121)
        .expect("a bin holding the first payment's hour");
    assert_eq!(straddling.min_value, 0.0);
    assert!(straddling.max_value > 0.0);
    let error = summary(json!({
        "GER": [{ "firstHour": 100, "lastHour": 125 }],
        "ITA": [{ "firstHour": 120, "lastHour": 140 }],
    }))
    .unwrap_err();
    assert!(
        error.to_string().contains("hold some hours together"),
        "{error}"
    );
}

fn government(country: u32, sequence: u64, hour: i32) -> facts::Government {
    facts::Government {
        row: GovernmentChangeRow {
            country,
            instance: 1,
            sequence,
            epoch: 1,
            hour,
            ..Default::default()
        },
        tokens: (None, None),
    }
}

/// Government marks and pages hold the governments read by the asked hour, in read order.
#[test]
fn government_marks_and_pages_show_what_was_read_by_the_hour() {
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(100, &countries)];
    for at in 101..=130 {
        let mut hour = steady(at, &countries);
        if matches!(at, 105 | 110 | 120) {
            hour.governments.push(government(1, at as u64, at));
        }
        hours.push(hour);
    }
    let built = build(hours, &[112, 130], &[]);
    let marks = |to: i32| -> u64 {
        let reply = reply(
            &built,
            json!({
                "branchId": "1",
                "tag": "GER",
                "view": "governments",
                "hour": to,
                "marks": { "from": 100, "to": 130, "width": 4 },
            }),
        );
        reply
            .government_marks
            .expect("marks")
            .clusters
            .iter()
            .map(|cluster| cluster.count.parse::<u64>().unwrap())
            .sum()
    };
    assert_eq!(marks(130), 3);
    assert_eq!(marks(112), 2);
    let page = |cursor: &str| {
        ask(
            &built,
            json!({
                "branchId": "1",
                "tag": "GER",
                "view": "governments",
                "hour": 130,
                "page": { "from": 100, "to": 130, "cursor": cursor, "pageSize": 2 },
            }),
        )
        .unwrap()
    };
    let first = page("0");
    assert_eq!(first["rows"].as_array().unwrap().len(), 2);
    let next = first["cursor"].as_str().expect("a next page").to_owned();
    let second = page(&next);
    assert_eq!(second["rows"].as_array().unwrap().len(), 1);
    assert!(second["cursor"].is_null());
}

/// An item keeps the id of its first payment in every window, even one not holding it.
#[test]
fn an_item_is_named_by_its_first_payment_in_every_window() {
    let built = build(campaign(), &[110, 130], &[]);
    for (from, hour) in [(100, 110), (120, 130)] {
        let reply = reply(
            &built,
            json!({ "branchId": "1", "tag": "GER", "view": "spending", "from": from, "hour": hour }),
        );
        let decision = reply
            .spending
            .iter()
            .find(|spend| spend.category == "decisions")
            .expect("a decision in the window");
        assert_eq!(decision.item_id, rows::mutation_id(1, 1, 1));
        assert!(
            reply
                .spend_items
                .iter()
                .any(|item| item.id == decision.item_id)
        );
    }
}

/// Synthetic layered content, removed when dropped.
struct Stack(std::path::PathBuf);

impl Stack {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ledger_naming_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("base")).unwrap();
        Self(root)
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.0.join("base").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn content(&self) -> Arc<PoliticsContent> {
        self.write(
            "common/defines/00_fixture.lua",
            b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 30 }, NCharacter = { OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU = { \"mod_officers\" } } }",
        );
        self.write(
            "common/terrain/00_fixture.txt",
            b"categories = { fixture_terrain = { } }",
        );
        self.write(
            "common/buildings/00_fixture.txt",
            b"buildings = { fixture_building = { } }",
        );
        self.write(
            "common/abilities/fixture.txt",
            b"ability = { mod_ability = { name = mod_ability_title icon = GFX_mod_ability } }",
        );
        self.write(
            "localisation/english/fixture_l_english.yml",
            "\u{feff}l_english:\n mod_ability_title:0 \"Mod ability\"\n".as_bytes(),
        );
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(&self.0.join("base"), &[])]).unwrap();
        let catalogue = dashboard_text::catalogue::Catalogue::of_layers(layers, "english").unwrap();
        Arc::new(PoliticsContent::load(Arc::new(catalogue)).unwrap())
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An ability's payments are named through the recorded mod stack's localisation, and every
/// payment of it is one item.
#[test]
fn an_item_is_named_through_the_recorded_mod_stack() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for its runtime tables");
    let stack = Stack::new();
    let content = stack.content();
    let countries = [(1, "GER"), (2, "ITA")];
    let mut hours = vec![opening(100, &countries)];
    for at in 101..=120 {
        let mut hour = steady(at, &countries);
        if at == 105 || at == 115 {
            let mut power = decision(1, at as u64, at, 3_000);
            power.row.currency = PowerCurrency::Command;
            power.row.cause = PowerCause::AbilityActivation;
            power.row.identity.item_key = b"mod_ability".to_vec();
            power.dating.category = Some(Arc::from("abilities"));
            hour.power.push(power);
        }
        hours.push(hour);
    }
    // The emit names each item once, at its first payment, through the content it prepares with.
    let built = build_with(&Ok(Some(content)), hours, &[120], &[]);
    let reply = reply(
        &built,
        json!({ "branchId": "1", "tag": "GER", "view": "spending", "hour": 120 }),
    );
    assert_eq!(reply.spending.len(), 2);
    assert_eq!(reply.spend_items.len(), 1);
    assert_eq!(reply.spend_items[0].name.as_deref(), Some("Mod ability"));
    assert_eq!(reply.spend_items[0].id, rows::mutation_id(1, 1, 105));
}
