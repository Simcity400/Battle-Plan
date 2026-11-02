use crate::memory::{
    LedgerTruth, LedgerUnknown, MemoryRoster, MemorySpellings, MemoryTriggers, MemoryVariants,
    OwnedVersion,
};
use crate::*;
use core_events::{ObservationRun, ObservationUnknown};

fn span(first: Hour, last: Hour) -> Interval {
    Interval::new(first, last).unwrap()
}

fn gap() -> Absence {
    Absence::unknown("gap", "recording gap")
}

fn missing<T>(lookup: Lookup<'_, T>) -> (Interval, MissingKind<'_>) {
    match lookup {
        Lookup::Missing(missing) => (missing.hours, missing.kind),
        Lookup::Known(_) => panic!("expected missing"),
    }
}

#[test]
fn history_dates_versions_and_never_answers_missing_as_a_value() {
    let mut history = History::<u32, i64>::default();
    assert!(history.set(10, 1, Ok(0)).unwrap());
    assert!(
        !history.set(12, 1, Ok(0)).unwrap(),
        "an equal value is no change"
    );
    assert!(history.set(15, 1, Err(gap())).unwrap());
    assert!(history.set(20, 1, Ok(5)).unwrap());
    assert!(history.set(19, 1, Ok(6)).is_err(), "hours never go back");
    // Not recorded before the first version, from the asked hour to it.
    assert_eq!(
        missing(history.at(&1, 7, 30)),
        (span(7, 9), MissingKind::NotRecorded)
    );
    assert_eq!(
        history.at(&2, 7, 30),
        Lookup::Missing(Missing {
            hours: Interval::hour(7),
            kind: MissingKind::NotRecorded,
        })
    );
    // A known zero is a value, dated at its version, not at the unchanged later set.
    assert_eq!(
        history.at(&1, 14, 30),
        Lookup::Known(Dated {
            since: 10,
            value: &0
        })
    );
    let (hours, kind) = missing(history.at(&1, 17, 30));
    assert_eq!(hours, span(15, 19));
    assert!(matches!(kind, MissingKind::Unknown(reason) if reason.name == "gap"));
    assert_eq!(
        history.at(&1, 30, 30),
        Lookup::Known(Dated {
            since: 20,
            value: &5
        })
    );
    let mut seen = Vec::new();
    history
        .changes(span(11, 20), &mut |hour, key| {
            seen.push((hour, *key));
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, [(15, 1), (20, 1)]);
}

fn roster() -> MemoryRoster {
    let mut roster = MemoryRoster::default();
    roster.set_link(100, Link(11), Ok(Some(Slot(7)))).unwrap();
    roster.set_link(100, Link(12), Ok(Some(Slot(7)))).unwrap();
    roster.set_link(100, Link(21), Ok(Some(Slot(8)))).unwrap();
    for (country, slot) in [(3, 8), (5, 7), (9, 7)] {
        roster
            .set_country(100, CountryId(country), Ok(Some(Slot(slot))))
            .unwrap();
    }
    roster
        .set_holders(
            100,
            Slot(7),
            Ok(vec![
                (CountryId(5), b"GER".to_vec()),
                (CountryId(9), b"GER".to_vec()),
            ]),
        )
        .unwrap();
    roster
        .set_holders(100, Slot(8), Ok(vec![(CountryId(3), b"GER".to_vec())]))
        .unwrap();
    roster.set_holders(100, Slot(9), Ok(Vec::new())).unwrap();
    roster.set_name_tag(100, Slot(7), Ok(None)).unwrap();
    roster
        .set_name_tag(100, Slot(8), Ok(Some(vec![0xff, b'X'])))
        .unwrap();
    roster
        .set_original_tag(100, CountryId(5), Ok(Ok(b"GER".to_vec())))
        .unwrap();
    roster
        .set_original_tag(
            100,
            CountryId(3),
            Ok(Err(("unresolved".into(), "reference 44".into()))),
        )
        .unwrap();
    roster
        .set_reserved_zero(100, Ok(Ok(b"REB".to_vec())))
        .unwrap();
    roster.set_epoch(100, Ok(1)).unwrap();
    roster.set_complete(100).unwrap();
    roster
}

#[test]
fn roster_keeps_every_row_so_each_family_reads_its_own_mode() {
    let mut roster = roster();
    roster.advance(110).unwrap();
    assert_eq!(
        roster.slot_of(Link(12), 105).unwrap(),
        Lookup::Known(Dated {
            since: 100,
            value: Some(Slot(7))
        })
    );
    // Visited tags borrow the owner, so they outlive the callback without a copy.
    let mut held: Vec<(CountryId, Tag<'_>)> = Vec::new();
    let count = roster
        .holders(Slot(7), 105, &mut |country, tag| {
            held.push((country, tag));
            Ok(())
        })
        .unwrap();
    let rows: Vec<(CountryId, String)> = held
        .iter()
        .map(|(country, tag)| (*country, tag.text().unwrap().to_owned()))
        .collect();
    assert_eq!(
        count,
        Lookup::Known(Dated {
            since: 100,
            value: 2
        })
    );
    // Table order: the last row's tag wins for one family; a unique holder is absent for another.
    assert_eq!(
        rows,
        [(CountryId(5), "GER".into()), (CountryId(9), "GER".into())]
    );
    // A slot known to be empty is known, with no holder: not unknown.
    let none = roster
        .holders(Slot(9), 105, &mut |_, _| panic!("no holder"))
        .unwrap();
    assert_eq!(
        none,
        Lookup::Known(Dated {
            since: 100,
            value: 0
        })
    );
    // A tag that isn't UTF-8 is kept as recorded.
    let original = roster.name_tag(Slot(8), 105).unwrap();
    let Lookup::Known(Dated {
        value: Some(tag), ..
    }) = original
    else {
        panic!("dynamic slot");
    };
    assert_eq!((tag.0, tag.text()), (&[0xff, b'X'][..], None));
    // An original tag that doesn't resolve is refused with its reason, not missing.
    let original = |country| match roster.original_tag(country, 105).unwrap() {
        Lookup::Known(Dated { value, .. }) => value
            .map(|tag| tag.text().unwrap().to_owned())
            .map_err(|reason| reason.detail.to_owned()),
        Lookup::Missing(_) => panic!("known"),
    };
    assert_eq!(original(CountryId(5)), Ok("GER".into()));
    assert_eq!(original(CountryId(3)), Err("reference 44".into()));
    assert!(matches!(
        roster.original_tag(CountryId(9), 105).unwrap(),
        Lookup::Missing(Missing {
            kind: MissingKind::NotRecorded,
            ..
        })
    ));
    let zero = roster.reserved_zero_original_tag(105).unwrap();
    assert!(matches!(
        zero,
        Lookup::Known(Dated {
            value: Ok(Tag(b"REB")),
            ..
        })
    ));
    let mut tagged = Vec::new();
    let count = roster
        .tagged(Tag(b"GER"), 105, &mut |slot| {
            tagged.push(slot);
            Ok(())
        })
        .unwrap();
    // By first holder's id: slot 8's country 3 before slot 7's country 5.
    assert_eq!((count, tagged), (Lookup::Known(2), vec![Slot(8), Slot(7)]));
    let mut links = Vec::new();
    roster
        .links(Slot(7), 105, &mut |link| {
            links.push(link);
            Ok(())
        })
        .unwrap();
    assert_eq!(links, [Link(11), Link(12)]);
    assert!(roster.epoch(111).is_err(), "past the watermark");
}

#[test]
fn roster_forget_makes_every_answer_unknown_and_reports_changes_in_hour_order() {
    let mut roster = roster();
    roster
        .set_holders(104, Slot(8), Ok(vec![(CountryId(4), b"ITA".to_vec())]))
        .unwrap();
    roster.forget(106, gap()).unwrap();
    roster.set_link(108, Link(11), Ok(Some(Slot(8)))).unwrap();
    assert!(matches!(
        roster.slot_of(Link(12), 107).unwrap(),
        Lookup::Missing(_)
    ));
    // One unknown slot makes a tag's slots unknown, never a partial list.
    let tagged = roster.tagged(Tag(b"GER"), 107, &mut |_| Ok(())).unwrap();
    assert!(matches!(tagged, Lookup::Missing(_)));
    let links = roster.links(Slot(8), 108, &mut |_| Ok(())).unwrap();
    assert!(
        matches!(links, Lookup::Missing(_)),
        "links 12 and 21 are still unknown"
    );
    assert_eq!(
        roster.slot_of(Link(11), 108).unwrap(),
        Lookup::Known(Dated {
            since: 108,
            value: Some(Slot(8))
        })
    );
    let mut changes = Vec::new();
    roster
        .changes(span(101, 108), &mut |change| {
            changes.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(changes[0], (104, RosterKey::Holders(Slot(8))));
    assert!(
        changes[1..changes.len() - 1]
            .iter()
            .all(|(hour, _)| *hour == 106)
    );
    assert!(changes.contains(&(106, RosterKey::Epoch)));
    assert_eq!(changes.last(), Some(&(108, RosterKey::Link(Link(11)))));
}

#[test]
fn reverse_roster_lookups_are_missing_until_a_census_and_read_that_hours_members() {
    let mut empty = MemoryRoster::default();
    empty.advance(10).unwrap();
    // Nothing was read: an empty roster is unrecorded, never known to be empty.
    assert!(matches!(
        empty.tagged(Tag(b"GER"), 10, &mut |_| Ok(())).unwrap(),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        empty.links(Slot(1), 10, &mut |_| Ok(())).unwrap(),
        Lookup::Missing(_)
    ));
    assert!(matches!(
        empty.countries(10, &mut |_, _| Ok(())).unwrap(),
        Lookup::Missing(_)
    ));
    empty.set_complete(11).unwrap();
    assert_eq!(
        empty.links(Slot(1), 11, &mut |_| Ok(())).unwrap(),
        Lookup::Known(0),
        "a complete census with no link is a known absence"
    );
    let none = Lookup::Known(Dated {
        since: 11,
        value: None,
    });
    assert_eq!(empty.slot_of(Link(1), 11).unwrap(), none);
    assert_eq!(empty.country(CountryId(1), 11).unwrap(), none);
    let mut completed = Vec::new();
    empty
        .changes(span(11, 11), &mut |change| {
            completed.push(change.key);
            Ok(())
        })
        .unwrap();
    assert_eq!(completed, [RosterKey::Census], "completion changes reverse answers");

    let mut roster = roster();
    // Countries row 10 keeps its own slot 7 while its link aliases slot 8.
    roster.set_country(102, CountryId(10), Ok(Some(Slot(7)))).unwrap();
    roster.set_link(102, Link(10), Ok(Some(Slot(8)))).unwrap();
    roster.set_country(104, CountryId(9), Ok(None)).unwrap();
    roster.advance(110).unwrap();
    assert_eq!(
        roster.country(CountryId(10), 103).unwrap(),
        Lookup::Known(Dated {
            since: 102,
            value: Some(Slot(7))
        })
    );
    assert_eq!(
        roster.slot_of(Link(10), 103).unwrap(),
        Lookup::Known(Dated {
            since: 102,
            value: Some(Slot(8))
        })
    );
    let live = |hour| {
        let mut rows = Vec::new();
        roster
            .countries(hour, &mut |country, slot| {
                rows.push((country.0, slot.0));
                Ok(())
            })
            .unwrap();
        rows
    };
    // Each hour's own members: row 10 doesn't exist yet at 101, and row 9 is gone at 104.
    assert_eq!(live(101), [(3, 8), (5, 7), (9, 7)]);
    assert_eq!(
        roster.country(CountryId(10), 101).unwrap(),
        Lookup::Known(Dated {
            since: 100,
            value: None
        }),
        "row 10 doesn't exist yet at 101"
    );
    assert_eq!(live(104), [(3, 8), (5, 7), (10, 7)]);
    let mut links = Vec::new();
    roster
        .links(Slot(8), 101, &mut |link| {
            links.push(link);
            Ok(())
        })
        .unwrap();
    assert_eq!(links, [Link(21)], "link 10 names slot 8 only from 102");
    let mut changes = Vec::new();
    roster
        .changes(span(101, 110), &mut |change| {
            changes.push(change.key);
            Ok(())
        })
        .unwrap();
    assert!(changes.contains(&RosterKey::Country(CountryId(10))));
    assert!(changes.contains(&RosterKey::Country(CountryId(9))));
}

#[test]
fn spellings_share_a_content_identity_exactly_when_literals_are_equal() {
    let mut spellings = MemorySpellings::default();
    spellings
        .set_spelling(10, Token(1), Ok(b"infantry".to_vec()))
        .unwrap();
    spellings
        .set_spelling(10, Token(2), Ok(b"infantry".to_vec()))
        .unwrap();
    spellings
        .set_spelling(10, Token(3), Ok(b"artillery".to_vec()))
        .unwrap();
    spellings
        .set_spelling(14, Token(3), Err(Absence::NotRecorded))
        .unwrap();
    let content = |token, hour| match spellings.spelling(token, hour).unwrap() {
        Lookup::Known(spelling) => spelling.value.content,
        Lookup::Missing(_) => panic!("known"),
    };
    assert_eq!(content(Token(1), 12), content(Token(2), 12));
    assert_ne!(content(Token(1), 12), content(Token(3), 12));
    assert_eq!(
        missing(spellings.spelling(Token(3), 14).unwrap()),
        (span(14, 14), MissingKind::NotRecorded)
    );
    let mut changed = Vec::new();
    spellings
        .changes(span(11, 14), &mut |change| {
            changed.push(change.key);
            Ok(())
        })
        .unwrap();
    assert_eq!(changed, [Token(3)]);
}

fn version(operands: u64, name: &str) -> OwnedVersion {
    OwnedVersion {
        operands: OperandsId(operands),
        token: Some("infantry_equipment".into()),
        name: Ok(Some(name.into())),
        design: None,
    }
}

#[test]
fn variant_versions_are_missing_while_forgotten_and_keep_their_dates_after() {
    let mut variants = MemoryVariants::default();
    let first = variants
        .set_version(22, Handle(900), Ok(version(1, "Rifle")))
        .unwrap();
    variants.read(22, NamingRead::Census).unwrap();
    let second = variants
        .set_version(25, Handle(901), Ok(version(2, "Gun")))
        .unwrap();
    variants.read(25, NamingRead::Ledger).unwrap();
    variants.forget(30, gap()).unwrap();
    variants.read(34, NamingRead::Ledger).unwrap();
    variants
        .set_version(34, Handle(901), Ok(version(3, "Renamed gun")))
        .unwrap();
    variants.advance(40).unwrap();
    assert_eq!((first, second), (VariantId(0), VariantId(1)));
    assert_eq!(variants.id(Handle(901)), Some(second));
    assert_eq!(variants.handle(second), Some(Handle(901)));
    let since = |id, hour| match variants.version(id, hour).unwrap() {
        Lookup::Known(version) => (
            version.since,
            version.value.name.unwrap().unwrap().to_owned(),
        ),
        Lookup::Missing(_) => panic!("known"),
    };
    assert_eq!(since(first, 29), (22, "Rifle".into()));
    let (hours, kind) = missing(variants.version(first, 31).unwrap());
    assert_eq!(hours, span(30, 33));
    assert!(matches!(kind, MissingKind::Unknown(_)));
    // Unchanged across the forget: still dated at its own read.
    assert_eq!(since(first, 35), (22, "Rifle".into()));
    assert_eq!(since(second, 35), (34, "Renamed gun".into()));
    assert_eq!(variants.known_since(29).unwrap(), Lookup::Known(22));
    assert_eq!(variants.known_since(35).unwrap(), Lookup::Known(34));
    assert_eq!(variants.census(35).unwrap(), Some(22));
    assert_eq!(variants.census(21).unwrap(), None);
    let mut reads = Vec::new();
    variants
        .reads(span(23, 40), &mut |hour, read| {
            reads.push((hour, read));
            Ok(())
        })
        .unwrap();
    assert_eq!(reads, [(25, NamingRead::Ledger), (34, NamingRead::Ledger)]);
    let mut changes = Vec::new();
    variants
        .changes(span(23, 40), &mut |change| {
            changes.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        changes,
        [
            (25, VariantKey::Version(second)),
            (25, VariantKey::Read(NamingRead::Ledger)),
            (30, VariantKey::Forgotten),
            (34, VariantKey::Version(second)),
            (34, VariantKey::Read(NamingRead::Ledger)),
        ]
    );
}

#[test]
fn a_later_loss_ends_the_earlier_missing_span() {
    let mut variants = MemoryVariants::default();
    let id = variants
        .set_version(22, Handle(900), Ok(version(1, "Rifle")))
        .unwrap();
    variants.read(22, NamingRead::Census).unwrap();
    variants.forget(30, gap()).unwrap();
    variants
        .forget(
            32,
            Absence::Lost {
                name: "damage".into(),
                detail: "section lost".into(),
            },
        )
        .unwrap();
    variants.read(34, NamingRead::Ledger).unwrap();
    variants.advance(40).unwrap();
    let (hours, kind) = missing(variants.version(id, 31).unwrap());
    assert_eq!(hours, span(30, 31));
    assert!(matches!(kind, MissingKind::Unknown(_)));
    let (hours, kind) = missing(variants.version(id, 32).unwrap());
    assert_eq!(hours, span(32, 33));
    assert!(matches!(kind, MissingKind::Lost(_)));
}

#[test]
fn variants_never_known_before_their_first_read() {
    let mut variants = MemoryVariants::default();
    let id = variants
        .set_version(5, Handle(1), Ok(version(1, "Rifle")))
        .unwrap();
    assert_eq!(
        missing(variants.known_since(5).unwrap()).1,
        MissingKind::NotRecorded
    );
    assert!(matches!(
        variants.version(id, 5).unwrap(),
        Lookup::Missing(_)
    ));
}

#[test]
fn the_ledger_witness_is_reported_as_runs_with_unrecorded_hours_unknown() {
    let mut variants = MemoryVariants::default();
    variants.set_ledger(10, LedgerTruth::Fails).unwrap();
    variants.set_ledger(12, LedgerTruth::Holds).unwrap();
    variants
        .set_ledger(
            15,
            LedgerTruth::Unknown(LedgerUnknown::Gap, "gap".into(), "lost hours".into()),
        )
        .unwrap();
    variants.advance(20).unwrap();
    let mut runs = Vec::new();
    variants
        .ledger(span(8, 20), &mut |run: ObservationRun<'_>| {
            let truth = match run.truth {
                Truth::Holds => "holds",
                Truth::Fails => "fails",
                Truth::Unknown(ObservationUnknown::NotRecorded) => "not recorded",
                Truth::Unknown(ObservationUnknown::Gap(reason)) if reason.name == "gap" => "gap",
                Truth::Unknown(_) => "other",
            };
            runs.push((run.hours, truth));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        runs,
        [
            (span(8, 9), "not recorded"),
            (span(10, 11), "fails"),
            (span(12, 14), "holds"),
            (span(15, 20), "gap"),
        ]
    );
}

#[test]
fn queries_are_invalidated_only_through_their_own_footprints() {
    use crate::{QueryRead, ReadScope, TextRead};
    use core_events::{Key, TableId};
    let owner = TextRead {
        table: TableId::States,
        field: 1,
    };
    let tension = TextRead {
        table: TableId::WorldTension,
        field: 2,
    };
    let (ger, ita) = (Key::new(&[7]).unwrap(), Key::new(&[9]).unwrap());
    let mut triggers = MemoryTriggers::default();
    triggers.set_text(TextId(1), Ok(vec![(owner, true), (tension, false)]));
    triggers.set_text(TextId(3), Err(("Uncompiled".into(), "unknown scope".into())));

    let mut reads = Vec::new();
    let count = triggers
        .reads(TextId(1), &mut |read, retained| {
            reads.push((read, retained));
            Ok(())
        })
        .unwrap();
    assert_eq!(count, Lookup::Known(2));
    assert_eq!(reads, [(owner, true), (tension, false)]);

    // Registration ids come from the context, so two families can't collide.
    let leaders = triggers.register(TextId(1)).unwrap();
    let feed = triggers.register(TextId(1)).unwrap();
    let broken = triggers.register(TextId(3)).unwrap();
    assert_ne!(leaders, feed);
    triggers.advance(10).unwrap();
    let keyed = [QueryRead {
        read: owner,
        scope: ReadScope::Key(ger),
    }];
    let whole = [QueryRead {
        read: tension,
        scope: ReadScope::Field,
    }];
    triggers.footprint(leaders, 10, &keyed, &[18, 12]).unwrap();
    triggers.footprint(feed, 10, &whole, &[]).unwrap();
    triggers.change(11, owner, ita).unwrap();
    triggers.change(13, owner, ger).unwrap();
    triggers.change(14, tension, ita).unwrap();
    // Re-evaluated at 15, the leaders query reads only the tension field and has no deadline.
    triggers.advance(15).unwrap();
    triggers.footprint(leaders, 15, &whole, &[]).unwrap();
    triggers.change(16, owner, ger).unwrap();
    triggers.advance(20).unwrap();

    let mut undeclared = Vec::new();
    assert_eq!(
        triggers
            .undeclared(leaders, &mut |read| {
                undeclared.push(read);
                Ok(())
            })
            .unwrap(),
        Lookup::Known(1)
    );
    assert_eq!(undeclared, [tension]);
    assert_eq!(triggers.deadline(leaders, 10).unwrap(), Lookup::Known(Some(12)));
    assert_eq!(triggers.deadline(leaders, 15).unwrap(), Lookup::Known(None));
    assert!(matches!(
        triggers.deadline(broken, 10).unwrap(),
        Lookup::Missing(Missing {
            hours,
            kind: MissingKind::Refused(_),
        }) if hours == span(10, 10)
    ));

    let mut changed = Vec::new();
    triggers
        .changes(span(11, 20), &mut |change| {
            changed.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        changed,
        [(12, leaders), (13, leaders), (14, feed)],
        "ITA's owner, tension before re-evaluation and GER after it don't wake leaders"
    );
    triggers.unregister(leaders).unwrap();
    assert!(triggers.unregister(leaders).is_err());
    assert!(triggers.footprint(leaders, 20, &whole, &[]).is_err());
}

#[test]
fn ownership_tells_an_absent_place_from_an_unrecorded_one() {
    use crate::memory::MemoryOwnership;
    let mut ownership = MemoryOwnership::default();
    let facts = ProvinceFacts {
        state: 0,
        controller: 4,
    };
    ownership
        .set_province(10, ProvinceId(1), Ok(Some(facts)))
        .unwrap();
    ownership.set_province(10, ProvinceId(2), Ok(None)).unwrap();
    ownership
        .set_state(
            12,
            StateId(7),
            Ok(Some(StateFacts {
                owner: 3,
                controller: 4,
            })),
        )
        .unwrap();
    ownership
        .set_province(14, ProvinceId(1), Err(gap()))
        .unwrap();
    ownership.advance(20).unwrap();
    // State 0 is recorded so and kept; a known absence is not unknown.
    assert_eq!(
        ownership.province(ProvinceId(1), 11).unwrap(),
        Lookup::Known(Dated {
            since: 10,
            value: Some(facts)
        })
    );
    assert_eq!(
        ownership.province(ProvinceId(2), 11).unwrap(),
        Lookup::Known(Dated {
            since: 10,
            value: None
        })
    );
    assert_eq!(
        missing(ownership.province(ProvinceId(1), 15).unwrap()).0,
        span(14, 20)
    );
    assert_eq!(
        missing(ownership.state(StateId(8), 15).unwrap()).1,
        MissingKind::NotRecorded
    );
    let mut changes = Vec::new();
    ownership
        .changes(span(11, 20), &mut |change| {
            changes.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        changes,
        [
            (12, PlaceKey::State(StateId(7))),
            (14, PlaceKey::Province(ProvinceId(1)))
        ]
    );
}

#[test]
fn state_names_keep_each_usage_and_its_error_mode() {
    use crate::memory::{MemoryStateNames, OwnedNameError};
    let mut names = MemoryStateNames::default();
    names
        .set_state_name(
            10,
            StateId(7),
            StateNameUsage::Place,
            Ok(Ok("Berlin".into())),
        )
        .unwrap();
    names
        .set_state_name(
            10,
            StateId(7),
            StateNameUsage::Raid,
            Ok(Ok("Berlin".into())),
        )
        .unwrap();
    names
        .set_state_name(
            10,
            StateId(8),
            StateNameUsage::Raid,
            Ok(Err(OwnedNameError::TemplateRefused(
                "STATE_8".into(),
                ("refused".into(), "no localisation".into()),
            ))),
        )
        .unwrap();
    names
        .set_state_name(
            13,
            StateId(7),
            StateNameUsage::Place,
            Ok(Ok("Brandenburg".into())),
        )
        .unwrap();
    names
        .set_state_name(
            13,
            StateId(7),
            StateNameUsage::Raid,
            Ok(Ok("Brandenburg".into())),
        )
        .unwrap();
    names.advance(15).unwrap();
    assert_eq!(
        names
            .state_name(StateId(7), StateNameUsage::Raid, 14)
            .unwrap(),
        Lookup::Known(Dated {
            since: 13,
            value: Ok("Brandenburg")
        })
    );
    let refused = names
        .state_name(StateId(8), StateNameUsage::Raid, 14)
        .unwrap();
    assert!(matches!(
        refused,
        Lookup::Known(Dated {
            value: Err(NameError::TemplateRefused { key: "STATE_8", .. }),
            ..
        })
    ));
    // Another usage of a state is its own answer.
    assert!(matches!(
        names
            .state_name(StateId(8), StateNameUsage::Place, 14)
            .unwrap(),
        Lookup::Missing(_)
    ));
    let mut changes = Vec::new();
    names
        .changes(span(11, 15), &mut |change| {
            changes.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(changes, [(13, StateId(7))], "once per state and hour");
}

#[test]
fn character_naming_answers_earlier_hours_within_its_retained_history() {
    use crate::memory::MemoryCharacters;
    let naming = |token| CharacterNaming {
        name_token_id: 1,
        token: Token(token),
        template_present: false,
        template_tag: 0,
        template_file: 0,
        template_line: 0,
        template_generated: false,
    };
    let mut characters = MemoryCharacters::default();
    characters
        .set_character(10, CharacterId(5), Ok(Some(naming(40))))
        .unwrap();
    characters
        .set_character(20, CharacterId(5), Ok(Some(naming(41))))
        .unwrap();
    characters
        .set_character(20, CharacterId(6), Ok(None))
        .unwrap();
    // Token 41's spelling changes character 5's name; token 40 no longer names anyone.
    characters.respell(25, Token(41)).unwrap();
    characters.respell(26, Token(40)).unwrap();
    characters.advance(30).unwrap();
    assert_eq!(
        characters.character(CharacterId(5), 12).unwrap(),
        Lookup::Known(Dated {
            since: 10,
            value: Some(naming(40))
        })
    );
    assert_eq!(
        characters.character(CharacterId(6), 25).unwrap(),
        Lookup::Known(Dated {
            since: 20,
            value: None
        })
    );
    characters.retain_from(15);
    assert_eq!(characters.retained(), Some(15));
    assert!(
        characters.character(CharacterId(5), 12).is_err(),
        "before the retained history"
    );
    let mut changed = Vec::new();
    characters
        .changes(span(11, 30), &mut |change| {
            changed.push((change.hour, change.key));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        changed,
        [(20, CharacterId(5)), (20, CharacterId(6)), (25, CharacterId(5))]
    );
}

#[test]
fn country_naming_keeps_each_field_result_and_its_failures_without_the_hour() {
    use crate::memory::{MemoryCountryNaming, OwnedLook};
    let mut naming = MemoryCountryNaming::default();
    let refusal = |detail: &str| ("unlinked".to_owned(), detail.to_owned());
    // The original tag fails while the civil war resolves: each result stands alone.
    let look = OwnedLook {
        name_tag: b"GER".to_vec(),
        cosmetic_tag: Vec::new(),
        original_or_own_tag: Err(refusal("reference 44")),
        civil_war: Ok(true),
    };
    naming
        .set_naming(10, CountryId(1), Ok((Slot(7), Some(look))))
        .unwrap();
    naming
        .set_naming(10, CountryId(2), Ok((Slot(8), None)))
        .unwrap();
    naming
        .set_naming(
            10,
            CountryId(3),
            Err(Absence::unknown("incomplete", "CountryAppearance")),
        )
        .unwrap();
    naming.advance(20).unwrap();
    let facts = |country| match naming.naming(country, 15).unwrap() {
        Lookup::Known(facts) => facts,
        Lookup::Missing(_) => panic!("known"),
    };
    let first = facts(CountryId(1));
    let read = first.value.look.unwrap();
    assert_eq!(first.value.slot, Slot(7));
    assert_eq!((read.name_tag, read.cosmetic_tag), (Tag(b"GER"), Tag(b"")));
    assert!(matches!(read.original_or_own_tag, Err(reason) if reason.detail == "reference 44"));
    assert_eq!(read.civil_war, Ok(true));
    // No appearance row is a known absence; unrecorded inputs are missing, not either of those.
    assert_eq!(facts(CountryId(2)).value.look, None);
    assert!(matches!(
        naming.naming(CountryId(3), 15).unwrap(),
        Lookup::Missing(_)
    ));
    // A lasting failure is one version, not a change each hour.
    assert_eq!(facts(CountryId(1)).since, 10);
    let mut changes = 0;
    naming
        .changes(span(11, 20), &mut |_| {
            changes += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(changes, 0);
}
