
use super::*;
use replay_data::{
    rows::ActiveIdeaRow, rows::CompletedFocusRow, rows::CountryCharacterRow, rows::CountryLinkRow,
    rows::PartyLeaderRow, rows::PoliticalPartyRow, rows::TableId, rows::TableRows,
};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows));
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(World<'_>) -> T) -> T {
    let dlcs = BTreeSet::new();
    let triggers = TriggerNames::default();
    let aliases = Arc::default();
    run(World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    ))
}

struct Draw(u64);

impl Draw {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

const COUNTRIES: std::ops::RangeInclusive<u32> = 101..=106;
const TOKENS: [&[u8]; 3] = [b"a", b"b", b"c"];

fn scenario(draw: &mut Draw) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Default::default(),
    };
    let mut links = Vec::new();
    for id in COUNTRIES {
        if draw.below(5) != 0 {
            links.push(CountryLinkRow {
                id,
                canonical_slot: 1 + draw.below(3) as u32,
            });
        }
    }
    set::<tables::CountryLinks>(&mut frame, links);
    let mut focuses = Vec::new();
    let mut parties = Vec::new();
    let mut leaders = Vec::new();
    let mut characters = Vec::new();
    let mut ideas = Vec::new();
    for country in COUNTRIES {
        for ordinal in 0..draw.below(3) as u32 {
            let token = TOKENS[draw.below(3) as usize].to_vec();
            focuses.push(CompletedFocusRow {
                country,
                ordinal,
                token: token.clone(),
            });
            ideas.push(ActiveIdeaRow {
                country,
                ordinal,
                token,
            });
            characters.push(CountryCharacterRow {
                country,
                ordinal,
                character: (draw.below(4) != 0).then(|| country * 10 + ordinal),
            });
        }
        for id in 1..=3 {
            if draw.below(2) == 0 {
                parties.push(PoliticalPartyRow {
                    country,
                    id,
                    is_ruling: u8::from(draw.below(3) == 0),
                    ..Default::default()
                });
            }
        }
        for party in 1..=3 {
            for ordinal in 0..draw.below(3) as u32 {
                let character = (draw.below(4) != 0).then(|| country * 100 + ordinal);
                leaders.push(PartyLeaderRow {
                    country,
                    party,
                    ordinal,
                    character,
                    role: character.map(|_| 1),
                    leader_id: ordinal as i32,
                });
            }
        }
    }
    set::<tables::CompletedFocuses>(&mut frame, focuses);
    set::<tables::PoliticalParties>(&mut frame, parties);
    set::<tables::PartyLeaders>(&mut frame, leaders);
    set::<tables::CountryCharacters>(&mut frame, characters);
    set::<tables::ActiveIdeas>(&mut frame, ideas);
    frame
}

#[test]
fn rows_out_of_country_order_are_scanned_whole() {
    let mut frame = scenario(&mut Draw(3));
    let rows = |country, token: &[u8]| CompletedFocusRow {
        country,
        ordinal: 0,
        token: token.to_vec(),
    };
    set::<tables::CountryLinks>(
        &mut frame,
        vec![CountryLinkRow {
            id: 102,
            canonical_slot: 1,
        }],
    );
    set::<tables::CompletedFocuses>(&mut frame, vec![rows(105, b"x"), rows(102, b"a")]);
    with_world(&frame, |world| {
        assert_eq!(world.has_focus_unrecorded(1, b"a"), Ok(true));
        assert!(world.has_focus_unrecorded(1, b"b").is_err());
        assert!(matches!(
            world.focus_rows.get().map(|rows| rows.value()),
            Some(Ok(None))
        ));
    });
}

#[test]
fn leader_roles_find_templates_at_the_recorded_first_token_line() {
    let path = "common/characters/mod.txt";
    let roles = LeaderRoles::new(
        content_reader_key_names::characters::parse_leader_templates(
            &path.into(),
            br#"characters = {
    next = {
        corps_commander = {}
    }
    blank = {

        navy_leader = {}
    }
    instanced = {
        instance = {
            allowed = {
                always = yes
            }
            field_marshal = {}
        }
    }
}"#,
        )
        .unwrap(),
    );
    for (line, index) in [(3, 0), (7, 1), (13, 2)] {
        assert_eq!(
            roles.by_place.get(&(path.as_bytes().to_vec(), line)),
            Some(&vec![index])
        );
    }
    for open in [2, 5, 9] {
        assert!(
            !roles
                .by_place
                .contains_key(&(path.as_bytes().to_vec(), open))
        );
    }
}
