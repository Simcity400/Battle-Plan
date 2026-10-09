//! Missing native overview operands, hourly in the Slow worker.
//! Native order is explicit in keys; all growth/conversion is off-thread.
use crate::{sampling::qpc, spooling::table, table_capture::CaptureRows};
use engine::{
    Memory, cwar_goal,
    cwar_relation::{self, overview as native},
    read,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[("war overview", 0, &[0, 1, 2])];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    owned: usize,
    goals: usize,
    countries: usize,
    relations: usize,
}
impl Capacities {
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.countries, crate::country_scale::rows(slots, 1));
    }
    pub(crate) fn raise_for_demand(&mut self, demand: native::Demand) {
        crate::country_scale::raise(&mut self.relations, demand.relations);
        crate::country_scale::raise(&mut self.owned, crate::country_scale::rows(demand.owned, 1));
        crate::country_scale::raise(&mut self.goals, crate::country_scale::rows(demand.goals, 1));
    }
}
pub struct Captured {
    owned: CaptureRows<native::OwnedRow>,
    goals: CaptureRows<cwar_goal::relation::Row>,
    countries: CaptureRows<native::CountryRow>,
    discovery: native::DiscoveryCache,
    prefix: Option<[u8; cwar_relation::NAME_PREFIX_LEN]>,
    prefix_outcome: Outcome,
    prefix_view: u32,
    literal: Option<(usize, [u8; cwar_relation::NAME_PREFIX_LEN])>,
}
impl Captured {
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            owned: CaptureRows::new(c.owned),
            goals: CaptureRows::new(c.goals),
            countries: CaptureRows::new(c.countries),
            discovery: native::DiscoveryCache::new(c.countries, c.relations),
            prefix: None,
            prefix_outcome: Outcome::Rows { len: 0 },
            prefix_view: 0,
            literal: None,
        }
    }
    pub fn reset(&mut self) {
        self.owned.reset();
        self.goals.reset();
        self.countries.reset();
        self.prefix = None;
        self.prefix_outcome = Outcome::Rows { len: 0 };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.owned.allocated_bytes()
            + self.goals.allocated_bytes()
            + self.countries.allocated_bytes()
            + self.discovery.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        self.owned.grow(&mut c.owned);
        self.goals.grow(&mut c.goals);
        self.countries.grow(&mut c.countries);
        self.discovery.grow(c.countries, c.relations);
    }
    pub(crate) fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.discovery.invalidate();
        self.owned
            .fail(super::failed_capture(TABLES[0], hour, force_full, reason));
        self.goals
            .fail(super::failed_capture(TABLES[1], hour, force_full, reason));
        self.countries
            .fail(super::failed_capture(TABLES[2], hour, force_full, reason));
        self.prefix = None;
        self.prefix_outcome = super::failed_capture(TABLES[3], hour, force_full, reason);
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        _hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        let _reads = crate::read_accounting::table(TABLES[0]);
        let start = qpc();
        if force_full {
            self.discovery.invalidate();
        }
        let outcomes = native::walk_cached(
            memory,
            base,
            state,
            &mut self.owned.rows,
            &mut self.goals.rows,
            &mut self.countries.rows,
            &mut self.discovery,
        );
        [
            self.owned.outcome,
            self.goals.outcome,
            self.countries.outcome,
        ] = outcomes;
        ticks[0] = qpc().saturating_sub(start).max(0) as u64;
        let _reads = crate::read_accounting::table(TABLES[3]);
        let start = qpc();
        self.prefix_outcome = (|| {
            self.prefix_view = self.discovery.viewpoint()?;
            let prefix = match self
                .literal
                .filter(|(image, _)| *image == base && !force_full)
            {
                Some((_, prefix)) => prefix,
                None => {
                    let bytes = base
                        .checked_add(cwar_relation::NAME_PREFIX_RVA)
                        .and_then(|p| read::<[u8; 9]>(memory, p))
                        .ok_or("native war naming prefix unreadable or invalid")?;
                    if bytes[8] != 0 || bytes[..8].contains(&0) {
                        return Err("native war naming prefix unreadable or invalid");
                    }
                    let prefix = bytes[..8].try_into().unwrap();
                    self.literal = Some((base, prefix));
                    prefix
                }
            };
            self.prefix = Some(prefix);
            Ok(Outcome::Rows { len: 1 })
        })()
        .unwrap_or_else(|reason| Outcome::Unavailable { reason });
        ticks[3] = qpc().saturating_sub(start).max(0) as u64;
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.owned.outcome,
            self.goals.outcome,
            self.countries.outcome,
            self.prefix_outcome,
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::WarOwnedRelations, _>(self.owned.outcome, &self.owned.rows, |r| {
                rows::WarOwnedRelationRow {
                    owner: r.owner,
                    ordinal: r.ordinal,
                    first_country: r.first_country,
                    second_country: r.second_country,
                    retired: r.retired,
                    first_goal_count: r.first_goal_count,
                    second_goal_count: r.second_goal_count,
                }
            }),
            table::<tables::WarRelationGoals, _>(self.goals.outcome, &self.goals.rows, |r| {
                rows::WarRelationGoalRow {
                    owner: r.owner,
                    relation_ordinal: r.relation_ordinal,
                    side: u32::from(r.side),
                    ordinal: r.ordinal,
                    handle: r.handle,
                    resolved: r.resolved,
                    type_token: r.type_token,
                    actor: r.actor,
                    recipient: r.recipient,
                    state_count: r.state_count,
                    first_state: r.first_state,
                    country_count: r.country_count,
                    first_country: r.first_country,
                }
            }),
            table::<tables::WarOverviewCountries, _>(
                self.countries.outcome,
                &self.countries.rows,
                |r| rows::WarOverviewCountryRow {
                    country: r.country,
                    major: r.major,
                    divisions: r.divisions,
                    factories: r.factories,
                    view_subject: r.view_subject,
                    score_available: r.score_available,
                },
            ),
            table::<tables::WarNameRules, _>(
                self.prefix_outcome,
                self.prefix.as_slice(),
                |prefix| rows::WarNameRuleRow {
                    index: 0,
                    prefix: engine::allocation::owned_vec(engine::allocation::copy_vec(prefix)),
                    viewpoint: self.prefix_view,
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_failure_cannot_become_empty_or_restore_the_static_prefix() {
        let mut c = Captured::vacant();
        c.fail_capture(4, true, "stopped worker");
        assert!(
            c.tables().iter().all(
                |t| matches!(t, TableView::Unavailable { reason } if reason == "stopped worker")
            )
        );
        c.fail_capture(4, false, "stopped worker");
        assert_eq!(
            c.outcomes()[3],
            Outcome::Unavailable {
                reason: "stopped worker"
            }
        );
    }
    #[test]
    fn capacities_follow_native_demand_and_country_scale_without_a_ceiling() {
        for count in [1_500, 5_000] {
            let mut capacity = Capacities::default();
            capacity.raise_for_countries(count + 1);
            capacity.raise_for_demand(native::Demand {
                owned: count * 3,
                goals: count * 5,
                relations: count * 25,
            });
            let c = Captured::with_capacities(&capacity);
            assert!(
                c.owned.rows.len() >= count * 3
                    && c.goals.rows.len() >= count * 5
                    && c.countries.rows.len() > count
            );
        }
    }
}
