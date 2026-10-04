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
mod tests;
