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
use crate::map::{
    allegiance::Allegiance,
    directed_wars::Owner,
    memo::Keyed,
    visibility::{self, Families},
};
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

/// The tables [`Index`] is built from besides its volunteer and hour [`Key`].
pub(crate) const TABLES: &[TableId] = &[
    TableId::CountryLinks,
    TableId::FactionMembers,
    TableId::Countries,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
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
    hosts: Result<Vec<(u32, u32)>, String>,
    /// Directed packet admission is tied to the presented hour.
    hour: i32,
}

impl Key {
    fn read(
        sent: &[SentVolunteerRow],
        divisions: &[DivisionRow],
        links: &BTreeMap<u32, u32>,
        ours: u32,
        hour: i32,
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
            let host = divisions[at].logical_owner;
            let slot = canonical(links, host)
                .ok_or("SentVolunteers logical country missing from CountryLinks")?;
            // Actual zero does not alias a positive reference resolving to zero.
            if seen.insert((host != 0, slot)) {
                hosts.push((host, row.ordinal));
            }
        }
        Ok(Self {
            ours,
            hosts: Ok(hosts),
            hour,
        })
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
    query_problem: Option<String>,
    volunteer_problem: Option<String>,
}

struct Country {
    friend: Result<bool, String>,
    enemy: Result<bool, String>,
    first_host: Option<u32>,
    /// First hostile or unreadable host, in native order. Equality at that
    /// position still wins before its war query.
    first_host_enemy: Option<(u32, Result<(), String>)>,
}

fn either(a: Result<bool, String>, b: Result<bool, String>) -> Result<bool, String> {
    if a == Ok(true) || b == Ok(true) {
        return Ok(true);
    }
    a?;
    b
}

impl Index {
    /// This hour's index: a recent hour's while [`TABLES`] are the same
    /// allocations and the [`Key`] is equal.
    pub(super) fn read(
        memo: &Memo,
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
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
                ours,
                frame.hour,
            )
        })();
        let key = match key {
            Ok(key) => key,
            Err(problem) => Key {
                ours,
                hour: frame.hour,
                hosts: Err(problem),
            },
        };
        memo.get_or_build(frame, key.clone(), |projected| {
            let links: BTreeMap<u32, u32> = projected
                .complete::<tables::CountryLinks>()?
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            Ok::<_, io::Error>(Arc::new(Self::new(projected, &links, &key)))
        })
        .unwrap_or_else(|error| Arc::new(Self::unavailable(error.to_string())))
    }

    fn unavailable(problem: String) -> Self {
        Self {
            problem: Some(problem),
            ..Self::default()
        }
    }

    fn new(frame: &DecodedFrame, links: &BTreeMap<u32, u32>, key: &Key) -> Self {
        let viewer = if key.ours == 0 {
            Ok(0)
        } else {
            links
                .iter()
                .find(|&(_, &slot)| slot == key.ours)
                .map(|(&raw, _)| raw)
                .ok_or_else(|| "viewing country CountryLinks row missing".to_owned())
        }
        .and_then(|raw| Owner::new(frame, raw));
        let families = Families::read(frame, links);
        let volunteer_problem = key.hosts.as_ref().err().cloned();
        let mut query_problem = volunteer_problem.clone();
        let hosts = key.hosts.as_deref().unwrap_or_default();
        let countries = links
            .keys()
            .copied()
            .chain([0])
            .map(|raw| {
                let friend = viewer.as_ref().map_err(Clone::clone).and_then(|viewer| {
                    visibility::counter_shared(
                        frame,
                        links,
                        viewer,
                        raw,
                        families.as_ref().map_err(|reason| *reason),
                    )
                });
                let enemy = viewer
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|viewer| viewer.at_war(raw));
                let first_host = hosts
                    .iter()
                    .find(|&&(host, _)| {
                        raw == host
                            || (raw != 0
                                && host != 0
                                && canonical(links, raw) == canonical(links, host))
                    })
                    .map(|&(_, position)| position);
                let mut first_host_enemy = None;
                if !hosts.is_empty() {
                    let receiver = Owner::new(frame, raw);
                    for &(host, position) in hosts {
                        match receiver
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|receiver| receiver.at_war(host))
                        {
                            Ok(false) => {}
                            Ok(true) => {
                                first_host_enemy = Some((position, Ok(())));
                                break;
                            }
                            Err(reason) => {
                                first_host_enemy = Some((position, Err(reason)));
                                break;
                            }
                        }
                    }
                }
                for result in [&friend, &enemy] {
                    if let Err(reason) = result {
                        query_problem.get_or_insert_with(|| reason.clone());
                    }
                }
                if let Some((_, Err(reason))) = &first_host_enemy {
                    query_problem.get_or_insert_with(|| reason.clone());
                }
                (
                    raw,
                    Country {
                        friend,
                        enemy,
                        first_host,
                        first_host_enemy,
                    },
                )
            })
            .collect();
        Self {
            countries,
            problem: None,
            query_problem,
            volunteer_problem,
        }
    }

    /// Why [`Index::of`] answers `None`.
    pub(super) fn problem(&self) -> &str {
        self.problem
            .as_deref()
            .or(self.query_problem.as_deref())
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
        let a = self.countries.get(&roster)?;
        let b = self.countries.get(&logical)?;
        let mut friend = either(a.friend.clone(), b.friend.clone());
        let mut enemy = either(a.enemy.clone(), b.enemy.clone());
        if friend == Ok(true) {
            return Some(Side::Friend);
        }
        // Unknown volunteers may set friend even for an already-known enemy.
        // They cannot erase a known ordinary friend or affect a mission item.
        if self.volunteer_problem.is_some() {
            return None;
        }
        if let Some(host) = a.first_host
            && b.first_host_enemy
                .as_ref()
                .is_none_or(|(war, _)| host <= *war)
        {
            friend = Ok(true);
        } else if let Some((_, result)) = &b.first_host_enemy {
            if let Err(reason) = result {
                if a.first_host.is_some() {
                    // An earlier unknown war may stop a later friendly host.
                    friend = either(friend, Err(reason.clone()));
                }
                enemy = either(enemy, Err(reason.clone()));
            } else {
                enemy = Ok(true);
            }
        }
        // 1418C5AC1 tests friend before enemy. A proved friend never demands
        // unknown enemy operands; an unknown friend cannot become neutral.
        Some(if friend.ok()? {
            Side::Friend
        } else if enemy.ok()? {
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
        let a = self.countries.get(&roster)?;
        let b = self.countries.get(&logical)?;
        Some(if either(a.enemy.clone(), b.enemy.clone()).ok()? {
            Allegiance::Enemy
        } else if either(a.friend.clone(), b.friend.clone()).ok()? {
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
