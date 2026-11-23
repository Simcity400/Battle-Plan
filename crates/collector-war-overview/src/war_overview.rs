use crate::{clock::qpc, table_capture::CaptureRows, table_conversion::table};
use collector_observers::{war_changes, war_changes::Changes as _, wargoal_changes};
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native::cwar_relation::{goals as cwar_goal, self, overview as native};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::{TableInput, TableView};

pub const N: usize = 10;
pub const TABLES: [TableId; N] = [
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::CountryWargoals,
    TableId::WargoalTargets,
    TableId::WargoalJustifications,
    TableId::WarParticipation,
    TableId::WarCapturedProvinces,
    TableId::WarLendLeaseHistories,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    owned: usize,
    goals: usize,
    countries: usize,
    relations: usize,
}

collector_primitives::capacity_max!(Capacities {
    owned,
    goals,
    countries,
    relations
});
impl Capacities {
    pub fn raise_for_countries(&mut self, slots: usize) {
        crate::country_scale::raise(&mut self.countries, crate::country_scale::rows(slots, 1));
    }
    pub fn raise_for_demand(&mut self, demand: native::Demand) {
        crate::country_scale::raise(&mut self.relations, demand.relations);
        crate::country_scale::raise(&mut self.owned, crate::country_scale::rows(demand.owned, 1));
        crate::country_scale::raise(&mut self.goals, crate::country_scale::rows(demand.goals, 1));
    }
}
pub struct Captured {
    inventory: crate::wargoals::Captured,
    owned: CaptureRows<native::OwnedRow>,
    goals: CaptureRows<cwar_goal::relation::Row>,
    countries: CaptureRows<native::CountryRow>,
    discovery: native::DiscoveryCache,
    prefix: Option<[u8; cwar_relation::NAME_PREFIX_LEN]>,
    prefix_outcome: Outcome,
    prefix_view: u32,
    participation: Option<[std::sync::Arc<rows::TableRows>; 3]>,
    participation_outcomes: [Outcome; 3],
    literal: Option<(usize, [u8; cwar_relation::NAME_PREFIX_LEN])>,
    certificates: Option<Certificates>,
}

struct Certificates {
    war: (u64, u64),
    owners: Vec<(u32, (u64, u64))>,
}

impl Certificates {
    fn hold(war: (u64, u64), rows: &[native::OwnedRow]) -> Option<Self> {
        let mut owners = Vec::new();
        engine_allocation::advisory(|| engine_allocation::reserve(&mut owners, rows.len())).ok()?;
        owners.extend(rows.iter().map(|row| (row.owner, (0, 0))));
        owners.sort_unstable_by_key(|owner| owner.0);
        owners.dedup_by_key(|owner| owner.0);
        for (country, sequence) in &mut owners {
            *sequence = wargoal_changes::goal_sequence(*country)?;
        }
        Some(Self { war, owners })
    }

    fn hold_now(&self, war: Option<(u64, u64)>) -> bool {
        war == Some(self.war)
            && self.owners.iter().all(|&(country, sequence)| {
                wargoal_changes::goal_sequence(country) == Some(sequence)
            })
    }
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            inventory: crate::wargoals::Captured::default(),
            owned: CaptureRows::new(c.owned),
            goals: CaptureRows::new(c.goals),
            countries: CaptureRows::new(c.countries),
            discovery: native::DiscoveryCache::new(c.countries, c.relations),
            prefix: None,
            prefix_outcome: Outcome::Rows { len: 0 },
            prefix_view: 0,
            literal: None,
            certificates: None,
            participation: None,
            participation_outcomes: [Outcome::Unavailable {
                reason: "war participation not captured",
            }; 3],
        }
    }
    pub fn reset(&mut self) {
        self.participation = None;
        self.participation_outcomes = [Outcome::Unavailable {
            reason: "war participation not captured",
        }; 3];
        self.owned.reset();
        self.goals.reset();
        self.countries.reset();
        self.prefix = None;
        self.prefix_outcome = Outcome::Rows { len: 0 };
    }
    pub fn allocated_bytes(&self) -> usize {
        self.inventory.allocated_bytes()
            + self.owned.allocated_bytes()
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
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        crate::participation::invalidate();
        self.participation = None;
        self.participation_outcomes = [Outcome::Unavailable { reason }; 3];
        self.discovery.invalidate();
        self.certificates = None;
        self.inventory.fail(reason);
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
        let war = war_changes::Observer.quiet();
        let goals = wargoal_changes::goals_quiet();
        let held = self.certificates.take();
        let outcomes = self
            .inventory
            .visit_owners(memory, base, force_full, |owners| {
                native::walk_cached_with_owners(
                    memory,
                    base,
                    state,
                    &mut self.owned.rows,
                    &mut self.goals.rows,
                    &mut self.countries.rows,
                    &mut self.discovery,
                    owners,
                    &mut || held.as_ref().is_some_and(|held| held.hold_now(war)),
                )
            });
        let complete = outcomes[..2]
            .iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }));
        self.certificates = if !complete {
            None
        } else if self.discovery.carried() {
            held
        } else {
            match (war, self.discovery.held_rows()) {
                (Some(war), Some(rows))
                    if goals.is_some()
                        && war_changes::Observer.quiet() == Some(war)
                        && wargoal_changes::goals_quiet() == goals =>
                {
                    Certificates::hold(war, rows)
                }
                _ => None,
            }
        };
        [
            self.owned.outcome,
            self.goals.outcome,
            self.countries.outcome,
        ] = outcomes;
        if let Outcome::Unavailable { reason } = outcomes[0] {
            self.inventory.fail(reason);
        }
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
                    let bytes = engine_sites::at(base, cwar_relation::NAME_PREFIX_RVA)
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
        let _reads = crate::read_accounting::table(TABLES[7]);
        let start = qpc();
        let captured = match self.owned.outcome {
            Outcome::Rows { len } => crate::participation::capture(
                memory,
                base,
                state,
                &self.owned.rows[..len],
                self.discovery.country_sources(),
                force_full,
            ),
            _ => {
                crate::participation::invalidate();
                Err("war participation owned census incomplete")
            }
        };
        match captured {
            Ok(rows) => {
                self.participation_outcomes = rows
                    .each_ref()
                    .map(|rows| Outcome::Rows { len: rows.len() });
                self.participation = Some(rows);
            }
            Err(reason) => {
                self.participation_outcomes = [Outcome::Unavailable { reason }; 3];
            }
        }
        ticks[7] = qpc().saturating_sub(start).max(0) as u64;
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        let inventory = self.inventory.outcomes();
        [
            self.owned.outcome,
            self.goals.outcome,
            self.countries.outcome,
            self.prefix_outcome,
            inventory[0],
            inventory[1],
            inventory[2],
            self.participation_outcomes[0],
            self.participation_outcomes[1],
            self.participation_outcomes[2],
        ]
    }
    fn original_tables(&self) -> [TableView; 7] {
        let [inventory, targets, justifications] = self.inventory.tables();
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
                    prefix: engine_allocation::owned_vec(engine_allocation::copy_vec(prefix)),
                    viewpoint: self.prefix_view,
                },
            ),
            inventory,
            targets,
            justifications,
        ]
    }
    pub fn table_inputs(&self) -> [TableInput; N] {
        let [a, b, c, d, inventory, targets, justifications] =
            self.original_tables().map(TableInput::Sampled);
        let extra: [TableInput; 3] = std::array::from_fn(|index| {
            match (&self.participation, self.participation_outcomes[index]) {
                (Some(rows), Outcome::Rows { .. }) => {
                    TableInput::SharedComplete(std::sync::Arc::clone(&rows[index]))
                }
                (_, Outcome::Unavailable { reason }) => {
                    TableInput::Sampled(crate::table_conversion::unavailable(reason))
                }
                _ => TableInput::Sampled(crate::table_conversion::unavailable(
                    "war participation incomplete",
                )),
            }
        });
        let [e, f, g] = extra;
        [a, b, c, d, inventory, targets, justifications, e, f, g]
    }
    #[cfg(test)]
    fn tables(&self) -> [TableView; N] {
        self.table_inputs().map(|input| match input {
            TableInput::Sampled(view) => view,
            TableInput::SharedComplete(rows) => TableView::Complete((*rows).clone()),
            _ => unreachable!(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_failure_cannot_become_empty_or_restore_the_static_prefix() {
        let _lock = crate::participation::TEST_LOCK.lock().unwrap();
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
}
