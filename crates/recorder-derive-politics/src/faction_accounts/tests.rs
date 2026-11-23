use super::*;
use recorder_derive_framework::{framework::Writers, testing};
use replay_columnar::{manifest::Status, store::Store, unit::Discard};
use replay_data::rows::RowCodec;

const TEXTS: &[&[u8]] = &[b"AAA", b"BBB", b"CCC"];

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(FactionAccounts::default())]);
    writers.start_run();
    writers
}

struct World {
    countries: Vec<(u32, u32, usize)>,
    links: Vec<(u32, u32)>,
    members: Vec<(u32, u64)>,
    accounts: Vec<(u32, u32)>,
    members_complete: bool,
}

fn world(hour: i32) -> World {
    let countries = |tag_of_one| vec![(1, 1, tag_of_one), (2, 2, 1)];
    let base = |members: Vec<(u32, u64)>| World { countries: countries(0), links: vec![(1, 1), (2, 2)], members, accounts: vec![(10, 1), (20, 2)], members_complete: true };
    match hour {
        0 => base(vec![(1, 7)]),
        1 => base(vec![]),
        2 => World { countries: countries(2), ..base(vec![]) },
        3 => World { countries: countries(2), ..base(vec![(1, 8), (2, 8)]) },
        4 => World { countries: countries(2), ..base(vec![(2, 8)]) },
        5 => World { countries: countries(2), members_complete: false, ..base(vec![(2, 8)]) },
        _ => World { countries: countries(2), ..base(vec![(2, 8)]) },
    }
}

fn sync<R: RowCodec>(store: &mut Store, table: TableId, rows: &[R]) {
    let keep: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (key, _) in testing::rows(store, table) {
        if !keep.contains(&key) {
            testing::remove(store, table, key).unwrap();
        }
    }
    for row in rows {
        testing::put(store, table, row, TEXTS).unwrap();
    }
}

fn run(store: &mut Store, writers: &mut Writers, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    for table in [TableId::Countries, TableId::CountryLinks, TableId::FactionAccounts] {
        store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
    }
    let members = if world.members_complete { Status::Complete } else { Status::Unavailable { reason: 0 } };
    store.status(TableId::FactionMembers.index(), members, true, Some(hour), true).unwrap();
    let countries: Vec<CountryRow> = world.countries.iter().map(|&(id, canonical_slot, tag)| CountryRow { id, canonical_slot, tag: TEXTS[tag].to_vec(), ..Default::default() }).collect();
    sync(store, TableId::Countries, &countries);
    let links: Vec<CountryLinkRow> = world.links.iter().map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).collect();
    sync(store, TableId::CountryLinks, &links);
    if world.members_complete {
        let members: Vec<FactionMemberRow> = world.members.iter().map(|&(country, faction)| FactionMemberRow { country, faction, ordinal: 0 }).collect();
        sync(store, TableId::FactionMembers, &members);
    }
    let accounts: Vec<FactionAccountRow> = world.accounts.iter().map(|&(account, country)| FactionAccountRow { account, country, ..Default::default() }).collect();
    sync(store, TableId::FactionAccounts, &accounts);
    store.settle();
    writers.hour(1, hour, hour == 0, store).unwrap();
    store.end_hour(&mut Discard).unwrap();
}

fn owners(store: &Store) -> Vec<(u32, u64, u32, u32, u8)> {
    let view = store.view(TableId::FactionAccountOwners.index());
    let mut rows: Vec<_> = view
        .rows()
        .map(|slot| {
            let cell = |column: usize| view.number(OWNER[column], slot).unwrap();
            (view.key(slot).unwrap() as u32, cell(0) as u64, cell(1) as u32, cell(2) as u32, cell(3) as u8)
        })
        .collect();
    rows.sort_unstable();
    rows
}

#[test]
fn an_account_keeps_its_last_faction_while_its_holder_holds_and_the_hours_do_not_break() {
    let (mut store, mut writers) = (testing::store(), writers());
    let mut seen = Vec::new();
    let mut statuses = Vec::new();
    for at in 0..=6 {
        run(&mut store, &mut writers, at, &world(at));
        seen.push(owners(&store));
        statuses.push(store.view(TableId::FactionAccountOwners.index()).status() == Status::Complete);
    }
    assert_eq!(
        seen,
        [
            vec![(10, 7, 1, 1, 1)],
            vec![(10, 7, 1, 1, 0)],
            vec![],
            vec![(10, 8, 1, 1, 1), (20, 8, 2, 2, 1)],
            vec![(10, 8, 1, 1, 0), (20, 8, 2, 2, 1)],
            vec![(10, 8, 1, 1, 0), (20, 8, 2, 2, 1)],
            vec![(20, 8, 2, 2, 1)],
        ],
        "a member's faction is kept when it leaves; a new tag ends it; a break keeps only explicit members"
    );
    assert_eq!(statuses, [true, true, true, true, true, false, true], "an input not complete makes the table unavailable, its rows as they were");
}

#[test]
fn an_ambiguous_membership_or_an_unresolved_member_keeps_no_retained_faction() {
    let (mut store, mut writers) = (testing::store(), writers());
    let one = |members: Vec<(u32, u64)>, links: Vec<(u32, u32)>| World { countries: vec![(1, 1, 0), (2, 2, 1)], links, members, accounts: vec![(10, 1), (20, 2)], members_complete: true };
    run(&mut store, &mut writers, 0, &one(vec![(1, 7), (2, 7)], vec![(1, 1), (2, 2), (3, 1)]));
    assert_eq!(owners(&store), [(10, 7, 1, 1, 1), (20, 7, 2, 2, 1)]);
    run(&mut store, &mut writers, 1, &one(vec![(1, 7), (3, 9)], vec![(1, 1), (2, 2), (3, 1)]));
    assert_eq!(owners(&store), [(20, 7, 2, 2, 0)], "slot 1's countries name two factions: its account keeps none");
    run(&mut store, &mut writers, 2, &one(vec![(4, 9)], vec![(1, 1), (2, 2), (3, 1)]));
    assert_eq!(owners(&store), Vec::<(u32, u64, u32, u32, u8)>::new(), "a member no link resolves: no faction is retained");
}

#[test]
fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
    let (mut unbroken, mut writing) = (testing::store(), writers());
    let mut expected = Vec::new();
    for at in 0..=6 {
        run(&mut unbroken, &mut writing, at, &world(at));
        expected.push(testing::rows(&unbroken, TableId::FactionAccountOwners));
    }
    let (mut stopped, mut before) = (testing::store(), writers());
    for at in 0..=1 {
        run(&mut stopped, &mut before, at, &world(at));
    }
    let saved = before.save().unwrap();
    let mut resumed = testing::resumed(&stopped, TEXTS);
    let mut after = writers();
    after.load(&saved, &mut resumed).unwrap();
    after.prime(&resumed, 1, 1);
    for at in 2..=6 {
        run(&mut resumed, &mut after, at, &world(at));
        assert_eq!(testing::rows(&resumed, TableId::FactionAccountOwners), expected[at as usize], "hour {at}");
    }
}
