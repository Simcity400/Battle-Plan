//! CUnitCounterItem::update (1418C4B50), independent of fog and plate colour.
//!
//! Primary CUnit +1D8 is roster owner; +1E0 is logical country (140BF5290,
//! serializer 140BF7010 token 3410), not the +1DC expeditionary owner.
//! 1406F3A90(a,b,0,0) accepts identity, otherwise 1406F3B00 vetoes active war
//! before accepting the same non-null faction or either direct subject endpoint
//! (140D11030/140D11080). Access, attaches and common enemies do not qualify.
//! 1406F3A00 tests the pair's +2E8 war and its zero retired byte +49, with a
//! positive receiver id. Both owner countries participate; then +310's ordered
//! volunteers_sent loop can set a flag. Friend wins over enemy at 1418C5AC1.
//! Land, railway guns and at-sea task forces share this path, including hidden
//! icons. Port CNavalMissionUnitItem (14197EAC0) never colours its +AE0 type.
//! The same two flags choose the plate's allegiance colour ([`plate`]).
use crate::map::{allegiance::Allegiance, memo::Keyed, visibility::Relations};
use replay::{
    frame::DecodedFrame,
    rows::{DivisionRow, SentVolunteerRow, TableId, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Friend,
    Enemy,
    Neutral,
}

/// The plate's `COUNTER_MODE_ALLEGIANCE_*` in the default colour mode
/// (`CSettings+0x2A8 == 1`, 1418C5244..1418C52E1), set before the coarse and
/// hidden arms: ours when the roster owner is the viewing country (raw or
/// canonical equality at 1418C5261), otherwise the type icon's own friend and
/// enemy flags, sent-volunteer loop included. An unavailable relation leaves
/// our own plates ours and draws the rest as other; the counter report counts
/// them.
pub(super) fn plate(own: bool, side: Option<Side>) -> Allegiance {
    match side {
        _ if own => Allegiance::Ours,
        Some(Side::Friend) => Allegiance::Allied,
        Some(Side::Enemy) => Allegiance::Enemy,
        Some(Side::Neutral) | None => Allegiance::Other,
    }
}

/// `NGraphics.FRIEND_COLOR`, `ENEMY_COLOR` and `NEUTRAL_COLOR`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Colors {
    friend: [f32; 4],
    enemy: [f32; 4],
    neutral: [f32; 4],
}

impl Colors {
    pub(super) fn read(defines: &installed_content::defines::Defines) -> io::Result<Self> {
        let color = |name| -> io::Result<[f32; 4]> {
            let values = defines.numbers("NGraphics", name)?;
            let [r, g, b]: [f64; 3] = values.try_into().map_err(|_: Vec<f64>| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("NDefines.NGraphics.{name} is not three numbers"),
                )
            })?;
            // 1418C5B37 passes alpha 1 to CColor ctor 142236F70; CIcon::SetColor
            // 1422E6AB0 -> CGraphicalObject::SetColor 142299D70 replaces RGBA;
            // buttonstate.shader PixelShaderUp multiplies the sampled texel.
            Ok([r as f32, g as f32, b as f32, 1.0])
        };
        Ok(Self {
            friend: color("FRIEND_COLOR")?,
            enemy: color("ENEMY_COLOR")?,
            neutral: color("NEUTRAL_COLOR")?,
        })
    }

    pub(super) fn of(self, side: Side) -> [f32; 4] {
        match side {
            Side::Friend => self.friend,
            Side::Enemy => self.enemy,
            Side::Neutral => self.neutral,
        }
    }

    /// The shipped `00_defines.lua` values, for tests only.
    #[cfg(test)]
    pub(super) const VANILLA: Self = Self {
        friend: [0.7, 0.9, 0.7, 1.0],
        enemy: [1.0, 0.7, 0.7, 1.0],
        neutral: [1.0, 1.0, 1.0, 1.0],
    };

    /// Three distinct non-vanilla colours, for tests only.
    #[cfg(test)]
    pub(super) const TEST: Self = Self {
        friend: [0.2, 0.8, 0.3, 1.0],
        enemy: [0.9, 0.1, 0.4, 1.0],
        neutral: [0.8, 0.6, 0.4, 1.0],
    };
}

/// The tables [`Index`] is built from besides its [`Key`]. Countries and
/// Divisions change every hour in columns the index does not read; the key
/// carries the little it needs from them, so an unchanged index is reused.
pub(crate) const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
];

/// Recent indexes, keyed on [`TABLES`] and the hour's [`Key`].
pub(crate) type Memo = Keyed<Index, Key>;

/// What the index reads from the hour's Countries, Divisions and SentVolunteers.
#[derive(Clone, PartialEq)]
pub(crate) struct Key {
    ours: u32,
    /// Each volunteer host's first position in the viewing country's list, in
    /// list order.
    hosts: Vec<(u32, u32)>,
    /// Sorted canonical slots whose stored id (CCountry+8) is not
    /// signed-positive, or that have no Countries row.
    nonpositive: Vec<u32>,
}

impl Key {
    fn read(
        sent: &[SentVolunteerRow],
        divisions: &[DivisionRow],
        links: &BTreeMap<u32, u32>,
        ids: &BTreeMap<u32, u32>,
        ours: u32,
    ) -> Result<Self, String> {
        // Rows are in key order (country, ordinal); only the viewing country's
        // own object holds a list.
        let mut lists = links
            .iter()
            .filter(|&(_, &slot)| slot == ours)
            .map(|(&raw, _)| {
                let start = sent.partition_point(|row| row.country < raw);
                let end = sent.partition_point(|row| row.country <= raw);
                &sent[start..end]
            })
            .filter(|list| !list.is_empty());
        let list = lists.next().unwrap_or_default();
        if lists.next().is_some() {
            return Err("SentVolunteers holds two lists for the viewing country".into());
        }
        let mut seen = BTreeSet::new();
        let mut hosts = Vec::new();
        for (position, row) in list.iter().enumerate() {
            if row.ordinal as usize != position {
                return Err("SentVolunteers ordinals are not contiguous".into());
            }
            // 1418C51A8 skips null references and 14220A3D0 unresolved ones. The
            // complete Divisions census holds every live division; its keys are
            // sorted and unique.
            if row.division == 0 {
                continue;
            }
            let Ok(at) = divisions.binary_search_by_key(&row.division, |d| d.id) else {
                continue;
            };
            let host = canonical(links, divisions[at].logical_owner)
                .ok_or("SentVolunteers logical country missing from CountryLinks")?;
            if seen.insert(host) {
                hosts.push((host, row.ordinal));
            }
        }
        let mut nonpositive: Vec<u32> = links
            .values()
            .chain([&ours])
            .copied()
            .filter(|slot| ids.get(slot).is_none_or(|&id| id as i32 <= 0))
            .collect();
        nonpositive.sort_unstable();
        nonpositive.dedup();
        Ok(Self {
            ours,
            hosts,
            nonpositive,
        })
    }

    /// 1406F3A00 requires its receiver's stored country id to be positive.
    fn positive(&self, slot: u32) -> bool {
        self.nonpositive.binary_search(&slot).is_err()
    }
}

/// Each country's relation to the viewing country, by raw reference.
///
/// The first matching sent volunteer decides the loop (1418C51A1..1418C52B4):
/// each host keeps its first position, and each country at war with a host the
/// earliest such position. A lookup never walks divisions or volunteer lists.
#[derive(Default)]
pub(crate) struct Index {
    countries: HashMap<u32, Country>,
    problem: Option<String>,
}

#[derive(Clone, Copy, Default)]
struct Country {
    friend: bool,
    enemy: bool,
    first_host: Option<u32>,
    first_host_enemy: Option<u32>,
}

impl Index {
    /// This hour's index: a recent hour's while [`TABLES`] are the same
    /// allocations and the [`Key`] is equal.
    pub(super) fn read(
        memo: &Memo,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ids: &BTreeMap<u32, u32>,
        ours: u32,
    ) -> Arc<Self> {
        let key = (|| {
            Key::read(
                frame
                    .complete::<tables::SentVolunteers>()
                    .map_err(|e| format!("SentVolunteers: {e}"))?,
                frame
                    .complete::<tables::Divisions>()
                    .map_err(|e| format!("Divisions: {e}"))?,
                links,
                ids,
                ours,
            )
        })();
        let key = match key {
            Ok(key) => key,
            Err(problem) => return Arc::new(Self::unavailable(problem)),
        };
        memo.get_or_build(frame, key.clone(), |projected| {
            let links: BTreeMap<u32, u32> = projected
                .complete::<tables::CountryLinks>()?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            Ok::<_, io::Error>(Arc::new(match Relations::read(projected, &links) {
                Ok(relations) => Self::new(&links, &relations, &key),
                Err(table) => Self::unavailable(format!("relations: {table}")),
            }))
        })
        .unwrap_or_else(|error| Arc::new(Self::unavailable(error.to_string())))
    }

    fn unavailable(problem: String) -> Self {
        Self {
            problem: Some(problem),
            ..Self::default()
        }
    }

    fn new(links: &BTreeMap<u32, u32>, relations: &Relations, key: &Key) -> Self {
        let ours = key.ours;
        let mut countries: BTreeMap<_, _> = links
            .values()
            .map(|&slot| {
                (
                    slot,
                    Country {
                        friend: relations.shared(ours, slot),
                        enemy: key.positive(ours) && relations.at_war(ours, slot),
                        ..Country::default()
                    },
                )
            })
            .collect();
        // Hosts are in list order, so the first position stored is the earliest.
        for &(host, position) in &key.hosts {
            countries.entry(host).or_default().first_host = Some(position);
            for enemy in relations.enemies(host) {
                // The loop's war test is received by the representative's
                // logical country: this enemy.
                if key.positive(enemy) {
                    countries
                        .entry(enemy)
                        .or_default()
                        .first_host_enemy
                        .get_or_insert(position);
                }
            }
        }
        let mut raw: HashMap<_, _> = links
            .iter()
            .map(|(&id, slot)| (id, countries.get(slot).copied().unwrap_or_default()))
            .collect();
        raw.entry(0).or_insert(Country {
            friend: ours == 0,
            ..Default::default()
        });
        Self {
            countries: raw,
            problem: None,
        }
    }

    /// Why [`Index::of`] answers `None`.
    pub(super) fn problem(&self) -> &str {
        self.problem
            .as_deref()
            .unwrap_or("counter owner missing from CountryLinks")
    }

    pub(super) fn of(&self, roster: u32, logical: u32) -> Option<Side> {
        // Signed-nonpositive roster bypasses all predicates at 1418C501C.
        if (roster as i32) <= 0 {
            return Some(Side::Friend);
        }
        if self.problem.is_some() {
            return None;
        }
        let a = *self.countries.get(&roster)?;
        let b = if roster == logical {
            a
        } else {
            *self.countries.get(&logical)?
        };
        let mut friend = a.friend || b.friend;
        let mut enemy = a.enemy || b.enemy;
        // Equality is tested before war on each reference, then either match
        // breaks. Do not OR all hosts: a prior hostile host can stop a later friend.
        if let Some(host) = a.first_host
            && b.first_host_enemy.is_none_or(|war| host <= war)
        {
            friend = true;
        } else if b.first_host_enemy.is_some() {
            enemy = true;
        }
        // 1418C5AC1 tests friend first, then enemy; hidden type uses the same path.
        Some(if friend {
            Side::Friend
        } else if enemy {
            Side::Enemy
        } else {
            Side::Neutral
        })
    }

    /// A `CNavalMissionUnitItem`'s (port item's or combat tile's) plate allegiance, as
    /// `BuildNavalCounterData@14197B4F0` sets its bytes and `CColor::CColor@140B3C210` reads them:
    /// ours (`+0x18`) when the first task force's roster owner (`+0x1D8`) is the viewing country;
    /// otherwise enemy (`+0x1A`) when `1406F3A00` holds between the viewer and the roster owner,
    /// or the logical owner (`+0x1E0`) where it is another country; only then allied (`+0x19`)
    /// by `1406F3A90` over the same two; else other. Enemy is tested first and there is no
    /// sent-volunteer loop, unlike the at-sea counter's type relation ([`Index::of`]). `None`
    /// when the relations or either owner are unavailable ([`Index::problem`]).
    pub(super) fn mission_item(&self, own: bool, roster: u32, logical: u32) -> Option<Allegiance> {
        if own {
            return Some(Allegiance::Ours);
        }
        if self.problem.is_some() {
            return None;
        }
        let a = *self.countries.get(&roster)?;
        let b = if roster == logical {
            a
        } else {
            *self.countries.get(&logical)?
        };
        Some(if a.enemy || b.enemy {
            Allegiance::Enemy
        } else if a.friend || b.friend {
            Allegiance::Allied
        } else {
            Allegiance::Other
        })
    }
}

fn canonical(links: &BTreeMap<u32, u32>, raw: u32) -> Option<u32> {
    if raw == 0 {
        Some(0)
    } else {
        links.get(&raw).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{CountryLinkRow, FactionMemberRow, SubjectRow, TableRows, WarRelationRow},
    };

    fn built(
        links: &BTreeMap<u32, u32>,
        ids: &BTreeMap<u32, u32>,
        relations: &Relations,
        ours: u32,
        sent: &[SentVolunteerRow],
        divisions: &[DivisionRow],
    ) -> Result<Index, String> {
        let key = Key::read(sent, divisions, links, ids, ours)?;
        Ok(Index::new(links, relations, &key))
    }

    fn empty_frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
        }
    }

    fn world(
        wars: &[(u32, u32)],
        factions: &[(u32, u64)],
        subjects: &[(u32, u32)],
    ) -> (BTreeMap<u32, u32>, BTreeMap<u32, u32>, Relations) {
        let links: BTreeMap<_, _> = (0..8)
            .map(|s| (100 + s, s))
            .chain([(0, 0), (501, 1)])
            .collect();
        let mut frame = empty_frame();
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                links
                    .iter()
                    .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ));
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(
                wars.iter()
                    .map(|&(a, b)| WarRelationRow {
                        first_country: 100 + a,
                        second_country: 100 + b,
                        start_hour: 1,
                        end_hour: 2,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(
                factions
                    .iter()
                    .map(|&(c, f)| FactionMemberRow {
                        country: 100 + c,
                        faction: f,
                        ordinal: 0,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(
            subjects
                .iter()
                .map(|&(a, b)| SubjectRow {
                    overlord: 100 + a,
                    subject: 100 + b,
                    ..Default::default()
                })
                .collect(),
        ));
        let relations = Relations::read(&frame, &links).unwrap();
        (
            links,
            (0..8).map(|slot| (slot, 100 + slot)).collect(),
            relations,
        )
    }
    #[test]
    fn country_identity_faction_subject_and_active_war_predicates() {
        use Side::*;
        let (links, ids, relations) = world(
            &[(1, 4), (1, 5)],
            &[(1, 7), (2, 7), (4, 7)],
            &[(1, 3), (1, 5), (3, 6)],
        );
        let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
        for (owner, logical, expected) in [
            (101, 101, Friend),
            (501, 501, Friend),
            (102, 102, Friend),
            (103, 103, Friend),
            (104, 104, Enemy),
            (105, 105, Enemy),
            (106, 106, Neutral),
            (107, 107, Neutral),
            (107, 102, Friend),
            (107, 104, Enemy),
            (102, 104, Friend),
            (104, 102, Friend),
            (0, 104, Friend),
            (u32::MAX, 104, Friend),
        ] {
            assert_eq!(
                index.of(owner, logical),
                Some(expected),
                "{owner} {logical}"
            );
        }
        let reverse = built(&links, &ids, &relations, 3, &[], &[]).unwrap();
        assert_eq!(reverse.of(101, 101), Some(Friend));
        // A common enemy alone (wartime ally) is neither faction nor subject.
        let (links, ids, relations) = world(&[(1, 4), (2, 4)], &[], &[]);
        assert_eq!(
            built(&links, &ids, &relations, 1, &[], &[])
                .unwrap()
                .of(102, 102),
            Some(Neutral)
        );
    }
    fn volunteers(order: &[u64]) -> Vec<SentVolunteerRow> {
        order
            .iter()
            .enumerate()
            .map(|(ordinal, &division)| SentVolunteerRow {
                country: 101,
                ordinal: ordinal as u32,
                division,
            })
            .collect()
    }
    #[test]
    fn first_volunteer_match_and_friend_precedence_survive_aliases_and_stale_refs() {
        use Side::*;
        let (links, ids, relations) = world(&[(2, 3)], &[(1, 1), (4, 1)], &[]);
        let divisions = vec![
            DivisionRow {
                id: 10,
                logical_owner: 102,
                ..Default::default()
            },
            DivisionRow {
                id: 20,
                logical_owner: 103,
                ..Default::default()
            },
        ];
        let build = |order: &[u64]| {
            built(&links, &ids, &relations, 1, &volunteers(order), &divisions).unwrap()
        };
        // Host 2 fights the representative's logical host 3: stop before host 3.
        assert_eq!(build(&[10, 20]).of(103, 103), Some(Enemy));
        assert_eq!(build(&[20, 10]).of(103, 103), Some(Friend));
        assert_eq!(build(&[0, 999, 20, 10, 20]).of(103, 103), Some(Friend));
        assert_eq!(build(&[10, 20]).of(104, 103), Some(Friend));
        assert_eq!(build(&[10]).of(102, 107), Some(Friend));
        assert_eq!(build(&[10]).of(107, 103), Some(Enemy));
        // Membership belongs to the vector owner, not the division roster owner.
        let mut sent = volunteers(&[10]);
        sent[0].country = 102;
        assert_eq!(
            built(&links, &ids, &relations, 1, &sent, &divisions)
                .unwrap()
                .of(102, 102),
            Some(Neutral)
        );
    }
    #[test]
    fn plates_take_the_type_icons_relation_through_sent_volunteers() {
        // The viewer (1) fights no one itself; its volunteers serve host 2, which
        // fights 3 and 4. 5 shares the viewer's faction; 6 is unrelated.
        let (links, ids, relations) = world(&[(2, 3), (2, 4)], &[(1, 9), (5, 9)], &[]);
        let divisions = [DivisionRow {
            id: 10,
            logical_owner: 102,
            ..Default::default()
        }];
        let index = built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions).unwrap();
        let plate_of = |slot: u32| plate(slot == 1, index.of(100 + slot, 100 + slot));
        assert_eq!(plate_of(1), Allegiance::Ours);
        assert_eq!(plate_of(2), Allegiance::Allied);
        assert_eq!(plate_of(3), Allegiance::Enemy);
        assert_eq!(plate_of(4), Allegiance::Enemy);
        assert_eq!(plate_of(5), Allegiance::Allied);
        assert_eq!(plate_of(6), Allegiance::Other);
        // Without the volunteer the owner-only reading leaves 3 and 4 other.
        let alone = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
        assert_eq!(plate(false, alone.of(103, 103)), Allegiance::Other);
        // Ours needs no relation input; an unavailable relation is drawn as other.
        assert_eq!(plate(true, None), Allegiance::Ours);
        assert_eq!(plate(false, None), Allegiance::Other);
    }

    /// Port items and combat tiles (`14197B4F0`) test enemy before friend over the first task
    /// force's roster and logical owners, with no sent-volunteer loop and no bypass for a
    /// signed-nonpositive roster owner; the at-sea counter's relation (`1418C4B50`) differs on
    /// each of these.
    #[test]
    fn mission_items_test_enemy_first_without_the_volunteer_loop() {
        use Allegiance::*;
        // The viewer (1) shares a faction with 2 and fights 4.
        let (links, ids, relations) = world(&[(1, 4)], &[(1, 7), (2, 7)], &[]);
        let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
        for (own, roster, logical, expected) in [
            (true, 104, 104, Ours),
            (false, 102, 102, Allied),
            (false, 104, 104, Enemy),
            (false, 106, 106, Other),
            // A friend and an enemy: the enemy byte wins, where the counter's friend does.
            (false, 102, 104, Enemy),
            (false, 104, 102, Enemy),
            (false, 106, 102, Allied),
            // The null roster reference is no bypass here.
            (false, 0, 104, Enemy),
            (false, 0, 106, Other),
        ] {
            assert_eq!(
                index.mission_item(own, roster, logical),
                Some(expected),
                "{own} {roster} {logical}"
            );
        }
        assert_eq!(index.of(102, 104), Some(Side::Friend));
        assert_eq!(index.of(0, 104), Some(Side::Friend));
        // Volunteers the viewer sends to 2, which fights 3, colour only the counter.
        let (links, ids, relations) = world(&[(2, 3)], &[], &[]);
        let divisions = [DivisionRow {
            id: 10,
            logical_owner: 102,
            ..Default::default()
        }];
        let index = built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions).unwrap();
        assert_eq!(index.of(102, 102), Some(Side::Friend));
        assert_eq!(index.of(103, 103), Some(Side::Enemy));
        assert_eq!(index.mission_item(false, 102, 102), Some(Other));
        assert_eq!(index.mission_item(false, 103, 103), Some(Other));
        // Unavailable relations or an unlinked owner name no side.
        assert_eq!(
            Index::unavailable("x".into()).mission_item(false, 102, 102),
            None
        );
        assert_eq!(
            Index::unavailable("x".into()).mission_item(true, 102, 102),
            Some(Ours)
        );
        assert_eq!(index.mission_item(false, 999, 999), None);
    }

    #[test]
    fn nonpositive_receiver_id_cannot_be_at_war() {
        let (links, mut ids, relations) = world(&[(1, 2), (2, 3)], &[], &[]);
        ids.insert(1, u32::MAX);
        assert_eq!(
            built(&links, &ids, &relations, 1, &[], &[])
                .unwrap()
                .of(102, 102),
            Some(Side::Neutral)
        );
        let divisions = [DivisionRow {
            id: 10,
            logical_owner: 102,
            ..Default::default()
        }];
        ids.insert(3, u32::MAX);
        assert_eq!(
            built(&links, &ids, &relations, 1, &volunteers(&[10]), &divisions)
                .unwrap()
                .of(103, 103),
            Some(Side::Neutral)
        );
    }
    #[test]
    fn attache_and_air_access_do_not_tint_an_unrelated_country() {
        use replay::rows::RelationLinkRow;
        let (links, ids, relations) = world(&[], &[], &[]);
        let mut frame = empty_frame();
        *frame.tables[TableId::RelationLinks.index()] =
            TableView::Complete(TableRows::RelationLinks(vec![RelationLinkRow {
                country: 102,
                target_index: 1,
                links: RelationLinkRow::ATTACHE | RelationLinkRow::AIR_BASE_ACCESS,
            }]));
        // Native type 2934 = military access (140D0A6D0 factory). The
        // counter's flags 0,0 exclude the +2A0 access arm of 1406F3B00.
        *frame.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(vec![
                replay::rows::RelationTupleRow {
                    owner_country: 101,
                    target_country: 102,
                    type_id: 0x2934,
                    first_country: 101,
                    second_country: 102,
                    ..Default::default()
                },
            ]));
        let relations = relations.with_attaches(&frame, &links).unwrap();
        assert!(relations.allied_for_fog(2, 1));
        assert_eq!(
            built(&links, &ids, &relations, 1, &[], &[])
                .unwrap()
                .of(102, 102),
            Some(Side::Neutral)
        );
    }
    #[test]
    fn indexed_first_match_agrees_with_the_ordered_native_loop() {
        let (links, ids, relations) = world(
            &[(1, 4), (2, 3), (3, 5), (2, 6)],
            &[(1, 8), (7, 8)],
            &[(1, 2)],
        );
        let divisions: Vec<_> = (1..8)
            .map(|id| DivisionRow {
                id,
                logical_owner: 100 + id as u32,
                ..Default::default()
            })
            .collect();
        for shift in 0..9 {
            let mut order: Vec<_> = (0..9).collect(); // null and one unresolved handle
            order.rotate_left(shift);
            for reverse in [false, true] {
                if reverse {
                    order.reverse();
                }
                let index =
                    built(&links, &ids, &relations, 1, &volunteers(&order), &divisions).unwrap();
                for roster in 100..108 {
                    for logical in 100..108 {
                        let a = links[&roster];
                        let b = links[&logical];
                        let mut friend = relations.shared(1, a) || relations.shared(1, b);
                        let mut enemy = relations.at_war(1, a) || relations.at_war(1, b);
                        for reference in &order {
                            let Some(division) = divisions.iter().find(|d| d.id == *reference)
                            else {
                                continue;
                            };
                            let host = links[&division.logical_owner];
                            if host == a {
                                friend = true;
                                break;
                            }
                            if relations.at_war(b, host) {
                                enemy = true;
                                break;
                            }
                        }
                        let expected = if friend {
                            Side::Friend
                        } else if enemy {
                            Side::Enemy
                        } else {
                            Side::Neutral
                        };
                        assert_eq!(
                            index.of(roster, logical),
                            Some(expected),
                            "order={order:?} {roster} {logical}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn the_memo_reuses_an_index_until_its_tables_or_key_change() {
        let (links, ids, _) = world(&[], &[], &[]);
        let mut frame = empty_frame();
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                links
                    .iter()
                    .map(|(&id, &canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .collect(),
            ));
        let divisions = |host: u32| {
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: 10,
                logical_owner: host,
                ..Default::default()
            }]))
            .into()
        };
        frame.tables[TableId::Divisions.index()] = divisions(102);
        *frame.tables[TableId::SentVolunteers.index()] =
            TableView::Complete(TableRows::SentVolunteers(volunteers(&[10])));
        let memo = Memo::new(TABLES, 4);
        let first = Index::read(&memo, &frame, &links, &ids, 1);
        assert_eq!(first.of(102, 102), Some(Side::Friend));
        // Divisions is a new allocation every hour; an equal host keeps the index.
        frame.tables[TableId::Divisions.index()] = divisions(102);
        assert!(Arc::ptr_eq(
            &first,
            &Index::read(&memo, &frame, &links, &ids, 1)
        ));
        // A changed host or a changed relation table builds a new one.
        frame.tables[TableId::Divisions.index()] = divisions(103);
        let moved = Index::read(&memo, &frame, &links, &ids, 1);
        assert_eq!(moved.of(103, 103), Some(Side::Friend));
        frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                first_country: 101,
                second_country: 104,
                ..Default::default()
            }]))
            .into();
        let at_war = Index::read(&memo, &frame, &links, &ids, 1);
        assert!(!Arc::ptr_eq(&moved, &at_war));
        assert_eq!(at_war.of(104, 104), Some(Side::Enemy));
        // A missing input withholds rather than reusing an older index.
        frame.tables[TableId::SentVolunteers.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        }
        .into();
        assert!(
            Index::read(&memo, &frame, &links, &ids, 1)
                .problem()
                .contains("SentVolunteers")
        );
    }

    #[test]
    #[ignore = "unit-scale tint cost; run release, one test thread"]
    fn type_tint_unit_scale() {
        use std::{hint::black_box, time::Instant};
        let stress = std::env::var("BATTLE_PLAN_UNIT_SHAPE").or_else(|_| std::env::var("HOI4_UNIT_SHAPE")).as_deref() == Ok("stress");
        let (n, d) = if stress {
            (5_000_u32, 100_000_usize)
        } else {
            (1_500, 50_000)
        };
        let links: BTreeMap<_, _> = (1..=n).map(|s| (s + 10_000, s)).collect();
        let ids: BTreeMap<_, _> = links.iter().map(|(&id, &slot)| (slot, id)).collect();
        let mut frame = empty_frame();
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(
                (1..=n)
                    .flat_map(|a| {
                        (1..=4).map(move |offset| WarRelationRow {
                            first_country: 10_000 + a,
                            second_country: 10_000 + (a + offset) % n + 1,
                            ..Default::default()
                        })
                    })
                    .collect(),
            ));
        let relations = Relations::read(&frame, &links).unwrap();
        let divisions: Vec<_> = (1..=d)
            .map(|i| DivisionRow {
                id: i as u64,
                roster_owner: 10_001,
                logical_owner: 10_000 + (i as u32 % n) + 1,
                ..Default::default()
            })
            .collect();
        // Deliberately all divisions in this viewer's ordered sent list. The
        // ordinary case references only a small part of the division census.
        let sent: Vec<_> = (0..d)
            .map(|i| SentVolunteerRow {
                country: 10_001,
                ordinal: i as u32,
                division: (i + 1) as u64,
            })
            .collect();
        let (mut keys, mut builds, mut lookups) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..25 {
            // Every hour reads the key; only a changed key or table builds.
            let start = Instant::now();
            let key = black_box(Key::read(&sent, &divisions, &links, &ids, 1).unwrap());
            keys.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            let index = black_box(Index::new(&links, &relations, &key));
            builds.push(start.elapsed().as_secs_f64() * 1000.0);
            let start = Instant::now();
            for division in &divisions {
                black_box(index.of(
                    black_box(division.roster_owner),
                    black_box(division.logical_owner),
                ));
            }
            lookups.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(index.of(10_001, 10_001), Some(Side::Friend));
        }
        for times in [&mut keys, &mut builds, &mut lookups] {
            times.sort_by(f64::total_cmp);
        }
        println!(
            "TYPE_TINT countries={n} divisions={d} sent={} hourly_key_median_ms={:.6} build_median_ms={:.6} all_lookup_median_ms={:.6}",
            sent.len(),
            keys[12],
            builds[12],
            lookups[12]
        );
    }

    #[test]
    fn incomplete_inputs_withhold_instead_of_inventing_neutral() {
        let (links, ids, relations) = world(&[], &[], &[]);
        let index = built(&links, &ids, &relations, 1, &[], &[]).unwrap();
        assert_eq!(index.of(101, 777), None);
        let bad = Index {
            problem: Some("SentVolunteers unavailable".into()),
            ..Default::default()
        };
        assert_eq!(bad.of(101, 101), None);
        assert!(bad.problem().contains("SentVolunteers"));
    }
    #[test]
    fn mod_defines_override_all_three_type_colors_and_keep_texture_alpha() {
        let root = std::env::temp_dir().join(format!("hoi4_type_tint_{}", std::process::id()));
        let base = root.join("base");
        let addon = root.join("mod");
        std::fs::create_dir_all(base.join("common/defines")).unwrap();
        std::fs::create_dir_all(addon.join("common/defines")).unwrap();
        std::fs::write(base.join("common/defines/00_base.lua"),b"NDefines={NGraphics={FRIEND_COLOR={0.1,0.2,0.3},ENEMY_COLOR={0.4,0.5,0.6},NEUTRAL_COLOR={0.7,0.8,0.9}}}").unwrap();
        std::fs::write(addon.join("common/defines/zz_tint.lua"),b"NDefines.NGraphics.FRIEND_COLOR={0.2,0.8,0.3};NDefines.NGraphics.ENEMY_COLOR={0.9,0.1,0.4};NDefines.NGraphics.NEUTRAL_COLOR={0.8,0.6,0.4}").unwrap();
        let layers =
            installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let defines = installed_content::defines::read(&layers).unwrap();
        let colors = Colors::read(&defines).unwrap();
        assert_eq!(colors, Colors::TEST);
        for side in [Side::Friend, Side::Enemy, Side::Neutral] {
            assert_eq!(colors.of(side)[3], 1.0);
        }
        defines
            .load("bad", b"NDefines.NGraphics.ENEMY_COLOR={1,2}")
            .unwrap();
        assert!(
            Colors::read(&defines)
                .unwrap_err()
                .to_string()
                .contains("ENEMY_COLOR")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
