//! Hourly class-13 simulation operands; map presentation remains viewer-owned.
//!
//! One walk of every country's railway-gun roster. Guns move, fight and lose
//! strength at any hour, so the table is hourly. Rows, route cells, stored
//! names and tooltip scratch grow from original live headers in the worker,
//! never from a fixed content bound. Optional tooltip failures remain local.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::allocation::{collect_vec, copy_vec, default_vec, owned_vec};
use engine::{
    Memory,
    cunit::FirstCombatMemo,
    railway_gun_counter_inputs,
    walk::{Buffer, Outcome},
};
use replay::{
    frame::TableView,
    rows::{
        FreshRailwayCounterEta, RailwayGunCounterInputRow, RailwayGunEtaOperands, Recorded,
        TableId, tables,
    },
};

static NAME_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ETA_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WITHDRAWAL_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FRESH_REFUSALS: [std::sync::atomic::AtomicU64; 4] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 4];
static TRANSPORT_REFUSALS: [std::sync::atomic::AtomicU64; 3] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 3];

pub(crate) fn movement_arrow_data() -> impl serde::Serialize {
    crate::status::status!({
        "withdrawal_refused": WITHDRAWAL_REFUSALS.load(std::sync::atomic::Ordering::Relaxed),
        "transport_refusals": crate::status::map([
            "selected transport railway current-hour packet unavailable",
            "selected transport railway stat weight unreadable",
            "selected transport transfer source unreadable",
        ].into_iter().zip(TRANSPORT_REFUSALS.iter()).map(|(reason, count)|
            (reason, count.load(std::sync::atomic::Ordering::Relaxed))).filter(|(_, count)| *count != 0)),
        "scope": "cumulative field-local railway-gun movement-arrow scalar refusals; established counter inputs remain available",
    })
}

pub(crate) fn eta_data() -> impl serde::Serialize {
    let refused = ETA_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusal": "railway gun first-edge ETA acquisition failed",
        "fresh_refusals": crate::status::map([
            "railway gun fresh ETA callback hour or observation arena unavailable",
            "railway gun fresh maximum-speed acquisition failed",
            "railway gun fresh land supply-factor acquisition failed",
            "railway gun fresh genuine orders-group acquisition failed",
        ].into_iter().zip(FRESH_REFUSALS.iter()).map(|(reason, count)|
            (reason, count.load(std::sync::atomic::Ordering::Relaxed))
        ).filter(|(_, count)| *count != 0)),
    })
}

pub(crate) fn name_data() -> impl serde::Serialize {
    let refused = NAME_REFUSALS.load(std::sync::atomic::Ordering::Relaxed);
    crate::status::status!({
        "refused": refused,
        "refusals": crate::status::map(std::iter::once(("railway gun name guarded read or arena refusal", refused)).filter(|(_, count)| *count != 0)),
        "scope": "cumulative stored railway-gun name acquisition refusals; other counter operands remain available",
    })
}

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::RailwayGunCounterInputs];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: usize,
    paths: usize,
    names: usize,
    named_bonuses: usize,
    supply_origins: usize,
    combat_references: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: crate::country_scale::railway_gun_name_demand().rows,
            paths: crate::country_scale::railway_gun_name_demand().paths,
            names: crate::country_scale::railway_gun_name_demand().bytes,
            named_bonuses: crate::country_scale::railway_gun_name_demand().named_bonuses,
            supply_origins: crate::country_scale::railway_gun_name_demand().supply_origins,
            combat_references: crate::country_scale::railway_gun_name_demand().combat_references,
        }
    }
}

impl Capacities {
    pub(crate) fn raise_for_names(&mut self, demand: railway_gun_counter_inputs::NameDemand) {
        grow_capacity(&mut self.rows, demand.rows);
        grow_capacity(&mut self.paths, demand.paths);
        grow_capacity(&mut self.names, demand.bytes);
        grow_capacity(&mut self.named_bonuses, demand.named_bonuses);
        grow_capacity(&mut self.supply_origins, demand.supply_origins);
        grow_capacity(&mut self.combat_references, demand.combat_references);
    }
}

pub struct Captured {
    rows: CaptureRows<railway_gun_counter_inputs::Row>,
    combat_references: Arena<engine::cunit::MapCombatReference>,
    sample_hour: Option<i32>,
    paths: Arena<u32>,
    names: Arena<u8>,
    tooltip_guns: Vec<(usize, usize)>,
    fresh_observations: Vec<railway_gun_counter_inputs::FreshEtaObservations>,
    named_bonuses: Vec<usize>,
    required_named_bonuses: usize,
    fixed_layout: Option<engine::fixed_layout::Report>,
    supply_origins: Arena<engine::cunit_supply_inputs::Origin>,
    /// First-combat answers for this capture's guns. The gun job runs apart from the
    /// division movement packet, so it keeps its own memo, sized off-thread.
    combats: FirstCombatMemo,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: 0,
            paths: 0,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(capacity.rows),
            combat_references: Arena::new(capacity.combat_references),
            sample_hour: None,
            paths: Arena::new(capacity.paths),
            names: Arena::new(capacity.names),
            tooltip_guns: owned_vec(default_vec(capacity.rows)),
            fresh_observations: owned_vec(default_vec(capacity.rows)),
            named_bonuses: owned_vec(default_vec(capacity.named_bonuses)),
            required_named_bonuses: 0,
            fixed_layout: None,
            supply_origins: Arena::new(capacity.supply_origins),
            combats: FirstCombatMemo::try_new(capacity.rows)
                .map_err(crate::allocation::io)
                .unwrap_or_default(),
        }
    }

    pub fn reset(&mut self) {
        self.combat_references.reset();
        self.sample_hour = None;
        self.rows.reset();
        self.paths.reset();
        self.names.reset();
        self.supply_origins.reset();
        self.combats.clear();
        self.required_named_bonuses = 0;
        self.fresh_observations.fill(Default::default());
    }

    pub fn allocated_bytes(&self) -> usize {
        self.combat_references.allocated_bytes()
            + self.rows.allocated_bytes()
            + self.paths.allocated_bytes()
            + self.names.allocated_bytes()
            + self.combats.allocated_bytes()
            + self.tooltip_guns.capacity() * std::mem::size_of::<(usize, usize)>()
            + self.fresh_observations.capacity()
                * std::mem::size_of::<railway_gun_counter_inputs::FreshEtaObservations>()
            + self.named_bonuses.capacity() * std::mem::size_of::<usize>()
            + self.supply_origins.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.raise_for_names(crate::country_scale::railway_gun_name_demand());
        self.rows.grow(&mut capacity.rows);
        self.combat_references.grow(&mut capacity.combat_references);
        grow_capacity(&mut capacity.paths, self.paths.required);
        self.paths.grow(&mut capacity.paths);
        self.names.grow(&mut capacity.names);
        grow_vec(&mut self.tooltip_guns, capacity.rows);
        grow_vec(&mut self.fresh_observations, capacity.rows);
        grow_capacity(&mut capacity.named_bonuses, self.required_named_bonuses);
        grow_vec(&mut self.named_bonuses, capacity.named_bonuses);
        self.supply_origins.grow(&mut capacity.supply_origins);
        let _ = self
            .combats
            .reserve(capacity.rows)
            .map_err(crate::allocation::io);
    }

    #[cfg(test)]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, game_state: usize) -> [u64; N] {
        self.capture_at(memory, base, game_state, None)
    }

    pub fn capture_with_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
    ) -> [u64; N] {
        self.capture_at(memory, base, game_state, Some(hour))
    }

    fn capture_at(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: Option<i32>,
    ) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let begin = qpc();
        // Answers are valid for one capture only; clearing keeps the reserved room.
        self.combats.clear();
        let mut names = railway_gun_counter_inputs::NameArena {
            bytes: Buffer::Growing(&mut self.names.first),
            used: 0,
            required: 0,
            failures: 0,
        };
        self.sample_hour = hour;
        let mut references = railway_gun_counter_inputs::CombatReferencesArena {
            entries: Buffer::Growing(&mut self.combat_references.first),
            used: 0,
            required: 0,
        };
        let mut tooltip = railway_gun_counter_inputs::TooltipScratch {
            guns: Buffer::Growing(&mut self.tooltip_guns),
            fresh_observations: Buffer::Growing(&mut self.fresh_observations),
            sample_hour: hour,
            named: engine::railway_gun_speed::NamedBonuses {
                pointers: Buffer::Growing(&mut self.named_bonuses),
                required: 0,
            },
            layout: &mut self.fixed_layout,
            supply_origins: Buffer::Growing(&mut self.supply_origins.first),
            supply_origins_len: 0,
            required_supply_origins: 0,
        };
        let walk = railway_gun_counter_inputs::walk_with_tooltip_and_combat_references_growing(
            memory,
            base,
            game_state,
            &mut self.rows.rows,
            &mut self.paths.first,
            &mut self.combats,
            &mut names,
            &mut tooltip,
            &mut references,
        );
        self.combat_references.len = references.used;
        self.combat_references.required = references.required;
        for row in &self.rows.rows[..walk.outcome.len()] {
            if let Some(reason) = row.combat_references_failure {
                super::unit_combat_references::observe_failure(reason);
            }
        }
        super::unit_combat_references::demand(
            true,
            self.combat_references.required,
            self.combat_references.first.len(),
        );
        self.required_named_bonuses = tooltip.named.required;
        self.supply_origins.len = tooltip.supply_origins_len;
        self.supply_origins.required = tooltip.required_supply_origins;
        self.rows.outcome = walk.outcome;
        if hour.is_some() {
            let mut refused = [0u64; 3];
            for row in &self.rows.rows[..walk.outcome.len()] {
                let Some(packet) = &row.transport else {
                    refused[0] += 1;
                    continue;
                };
                refused[1] += u64::from(packet.weight.is_err());
                if let Ok(engine::selected_transport_units::Transfer::Present {
                    status: Ok(0),
                    source,
                    ..
                }) = packet.transfer
                {
                    refused[2] += u64::from(source.is_err());
                }
            }
            for (count, value) in TRANSPORT_REFUSALS.iter().zip(refused) {
                count.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
            }
        }
        WITHDRAWAL_REFUSALS.fetch_add(
            self.rows.rows[..walk.outcome.len()]
                .iter()
                .filter(|row| row.withdrawal.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        ETA_REFUSALS.fetch_add(
            self.rows.rows[..walk.outcome.len()]
                .iter()
                .filter(|row| row.ordinary_len != 0 && row.eta.is_none())
                .count() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut fresh_refusals = [0_u64; 4];
        for row in &self.rows.rows[..walk.outcome.len()] {
            fresh_refusals[0] += u64::from(row.fresh_failure.is_some());
            for (count, failure) in fresh_refusals[1..].iter_mut().zip(row.fresh_failures) {
                *count += u64::from(failure.is_some());
            }
        }
        for (count, refused) in FRESH_REFUSALS.iter().zip(fresh_refusals) {
            count.fetch_add(refused, std::sync::atomic::Ordering::Relaxed);
        }
        self.paths.len = walk.paths_len;
        self.paths.required = walk.required_paths;
        self.names.len = names.used;
        self.names.required = names.required;
        NAME_REFUSALS.fetch_add(names.failures as u64, std::sync::atomic::Ordering::Relaxed);
        [qpc().saturating_sub(begin).max(0) as u64]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }

    pub(crate) fn movement_sources(
        &self,
    ) -> super::production::Sources<'_, railway_gun_counter_inputs::Row> {
        super::production::Sources {
            rows: &self.rows.rows,
            outcome: self.rows.outcome,
        }
    }

    fn supply_origins(
        &self,
        row: &railway_gun_counter_inputs::Row,
    ) -> Option<&[engine::cunit_supply_inputs::Origin]> {
        let len = row.supply?.origins_len.ok()?;
        let end = row.supply_origins_start.checked_add(len)?;
        self.supply_origins
            .first
            .get(..self.supply_origins.len)?
            .get(row.supply_origins_start..end)
    }

    pub(super) fn observe_supply(&self, work: &mut super::supply_inputs::Work) {
        for row in &self.rows.rows[..self.rows.outcome.len()] {
            work.observe(row.supply, row.supply_failure);
        }
    }

    pub fn tables(&self) -> [TableView; N] {
        let Some(paths) = self.paths.first.get(..self.paths.len) else {
            return [unavailable("railway-gun path arena length invalid")];
        };
        let rows = &self.rows.rows[..self.rows.outcome.len().min(self.rows.rows.len())];
        if rows.iter().any(|row| {
            [
                row.ordinary_start.checked_add(row.ordinary_len),
                row.alternate_start.checked_add(row.alternate_len),
            ]
            .iter()
            .any(|end| end.is_none_or(|end| end > paths.len()))
        }) {
            return [unavailable(
                "railway-gun route lies outside the captured arena",
            )];
        }
        // Each gun sits in exactly one country roster; a repeated handle is a
        // torn census. Checked here, off the game thread, in n log n.
        let mut handles: Vec<u64> = match collect_vec(rows.iter().map(|row| row.gun)) {
            Ok(handles) => handles,
            Err(_) => return [unavailable("collector allocation failed")],
        };
        handles.sort_unstable();
        if handles.windows(2).any(|pair| pair[0] == pair[1]) {
            return [unavailable("railway gun appears in more than one roster")];
        }
        let route = |start: usize, len: usize| owned_vec(copy_vec(&paths[start..start + len]));
        [table::<tables::RailwayGunCounterInputs, _>(
            self.rows.outcome,
            &self.rows.rows,
            |r| RailwayGunCounterInputRow {
                transport: Recorded::Value(
                    r.transport
                        .as_ref()
                        .map(super::selected_transport_inputs::project),
                ),
                combat_references: Recorded::Value(super::unit_combat_references::project(
                    self.sample_hour,
                    r.combat_references.and_then(|(start, len)| {
                        self.combat_references
                            .first
                            .get(..self.combat_references.len)?
                            .get(start..start.checked_add(len)?)
                    }),
                    r.combat_count,
                )),
                gun: r.gun,
                roster_owner: r.roster_owner,
                expeditionary_owner: r.expeditionary_owner,
                logical_owner: r.logical_owner,
                province: r.province,
                naval_transfer: u8::from(r.naval_transfer),
                transfer_action_status: u8::from(r.transfer_action_status),
                exiled: u8::from(r.exiled),
                retreat: u8::from(r.retreat),
                strategic_redeployment: u8::from(r.strategic_redeployment),
                reference_entry_count: r.reference_entry_count,
                assignee_group: r.assignee_group,
                subunit_token: r.subunit_token,
                current_strength: r.current_strength,
                maximum_strength: r.maximum_strength,
                received_supply_ratio: r.received_supply_ratio,
                railway_building_level: r.railway_building_level,
                target: r.target,
                combat_role: r.combat_role,
                combat_province: r.combat_province,
                combat_count: r.combat_count,
                attacking: u8::from(r.attacking),
                ordinary_path: route(r.ordinary_start, r.ordinary_len),
                alternate_path: route(r.alternate_start, r.alternate_len),
                name: Recorded::Value(r.name.and_then(|(start, len)| {
                    let bytes = self.names.first.get(..self.names.len)?;
                    let text = bytes.get(start..start.checked_add(len)?)?;
                    Some(owned_vec(copy_vec(text)))
                })),
                eta: Recorded::Value(r.eta.map(|eta| RailwayGunEtaOperands {
                    numerator: eta.numerator,
                    current_speed: eta.current_speed,
                })),
                supply: Recorded::Value(super::supply_inputs::project(
                    r.supply,
                    self.supply_origins(r),
                )),
                withdrawal: Recorded::Value(r.withdrawal),
                fresh: Recorded::Value(r.fresh.map(|inputs| FreshRailwayCounterEta {
                    sample_hour: inputs.sample_hour,
                    maximum_speed: inputs.maximum_speed,
                    land_supply_factor: inputs.land_supply_factor,
                    orders_group: inputs.orders_group,
                })),
            },
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;
    use replay::rows::TableRows;

    fn gun(handle: u64) -> railway_gun_counter_inputs::Row {
        railway_gun_counter_inputs::Row {
            gun: handle,
            roster_owner: 88,
            expeditionary_owner: 9,
            logical_owner: 10,
            province: 12,
            retreat: true,
            reference_entry_count: 2,
            assignee_group: (3_u64 << 32) | 70,
            subunit_token: 42,
            current_strength: -123_456,
            maximum_strength: 234_567,
            received_supply_ratio: -500,
            railway_building_level: Some(-3),
            target: 13,
            combat_role: 2,
            combat_province: 13,
            combat_count: 3,
            attacking: true,
            ordinary_start: 0,
            ordinary_len: 2,
            alternate_start: 2,
            alternate_len: 1,
            ..Default::default()
        }
    }

    #[test]
    fn projects_native_operands_and_refuses_bad_arenas_and_repeated_guns() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 2,
            paths: 3,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        capture.capture(&memory, 0, 0);
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.reset();
        capture.rows.outcome = Outcome::Rows { len: 0 };
        assert!(matches!(
            capture.tables()[0],
            TableView::Complete(TableRows::RailwayGunCounterInputs(ref rows)) if rows.is_empty()
        ));
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        let views = capture.tables();
        let TableView::Complete(TableRows::RailwayGunCounterInputs(rows)) = &views[0] else {
            panic!("complete railway-gun table");
        };
        let row = &rows[0];
        assert_eq!(
            (row.ordinary_path.as_slice(), row.alternate_path.as_slice()),
            (&[13, 14][..], &[15][..])
        );
        assert_eq!(row.railway_building_level, Some(-3));
        assert_eq!(
            (row.retreat, row.reference_entry_count, row.exiled),
            (1, 2, 0)
        );
        assert_eq!(row.received_supply_ratio, -500);

        capture.rows.rows[0].alternate_len = 2;
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.rows[0].alternate_len = 1;
        capture.rows.rows[1] = gun((7_u64 << 32) | 4713);
        capture.rows.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
        capture.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        assert!(matches!(capture.tables()[0], TableView::Truncated { .. }));
    }

    #[test]
    fn truncated_route_demand_grows_the_arena() {
        let mut capacity = Capacities {
            rows: 1,
            paths: 2,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.outcome = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        capture.paths.required = 9;
        capture.names.required = 37;
        capture.required_named_bonuses = 73;
        capture.supply_origins.required = 41;
        capture.grow(&mut capacity);
        assert!(capacity.paths >= 9 && capture.paths.first.len() >= 9);
        assert!(capacity.rows >= 1);
        assert!(capacity.names >= 37 && capture.names.first.len() >= 37);
        assert!(capacity.named_bonuses >= 73 && capture.named_bonuses.len() >= 73);
        assert!(capture.tooltip_guns.len() >= capacity.rows);
        assert!(capacity.supply_origins >= 41 && capture.supply_origins.first.len() >= 41);
    }

    #[test]
    fn stored_names_project_exact_bytes_and_bad_spans_refuse_only_the_name() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(&[0xff, 0, b'G']);
        capture.names.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        for (span, expected) in [
            (Some((0, 3)), Some(vec![0xff, 0, b'G'])),
            (Some((3, 0)), Some(Vec::new())),
            (Some((2, 2)), None),
            (Some((usize::MAX, 1)), None),
            (None, None),
        ] {
            let mut row = gun((7_u64 << 32) | 4713);
            row.name = span;
            row.eta = Some(railway_gun_counter_inputs::EtaOperands {
                numerator: -12_345,
                current_speed: 54_321,
            });
            capture.rows.rows[0] = row;
            let [TableView::Complete(TableRows::RailwayGunCounterInputs(rows))] = capture.tables()
            else {
                panic!("name span cannot withhold old operands");
            };
            assert_eq!(rows[0].name, Recorded::Value(expected));
            assert_eq!(
                rows[0].eta,
                Recorded::Value(Some(RailwayGunEtaOperands {
                    numerator: -12_345,
                    current_speed: 54_321,
                }))
            );
            assert_eq!(rows[0].current_strength, -123_456);
            assert_eq!(rows[0].ordinary_path, [13, 14]);
        }
    }

    #[test]
    fn missing_eta_keeps_stored_name_and_counter_operands() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(b"gun");
        capture.names.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        capture.rows.rows[0].name = Some((0, 3));
        let [TableView::Complete(TableRows::RailwayGunCounterInputs(rows))] = capture.tables()
        else {
            panic!("ETA refusal cannot withhold names or counter operands");
        };
        assert_eq!(rows[0].eta, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
        assert_eq!(rows[0].current_strength, -123_456);
    }

    #[test]
    fn fresh_eta_projects_independently_of_old_eta_and_refused_domains() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 0,
        });
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = gun((7_u64 << 32) | 4713);
        for fresh in [
            Some(railway_gun_counter_inputs::FreshEta {
                sample_hour: -42,
                maximum_speed: Some(i64::MIN),
                land_supply_factor: Some(i64::MAX),
                orders_group: Some(0),
            }),
            Some(railway_gun_counter_inputs::FreshEta {
                sample_hour: 81,
                maximum_speed: None,
                land_supply_factor: Some(0),
                orders_group: None,
            }),
            None,
        ] {
            capture.rows.rows[0].fresh = fresh;
            let [TableView::Complete(TableRows::RailwayGunCounterInputs(rows))] = capture.tables()
            else {
                panic!("fresh refusal cannot withhold counter prefix");
            };
            assert_eq!(
                rows[0].fresh,
                Recorded::Value(fresh.map(|input| FreshRailwayCounterEta {
                    sample_hour: input.sample_hour,
                    maximum_speed: input.maximum_speed,
                    land_supply_factor: input.land_supply_factor,
                    orders_group: input.orders_group,
                }))
            );
            assert_eq!(rows[0].eta, Recorded::Value(None));
            assert_eq!(rows[0].ordinary_path, [13, 14]);
            assert_eq!(rows[0].current_strength, -123_456);
        }
        assert_eq!(capture.fresh_observations.len(), capture.rows.rows.len());
    }

    #[test]
    fn supply_origin_refusals_preserve_exact_scalars_and_other_tooltip_suffixes() {
        use engine::cunit_supply_inputs::{Failure, Inputs, Origin};
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 3,
            names: 3,
            named_bonuses: 0,
            supply_origins: 2,
            combat_references: 0,
        });
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.paths.first.copy_from_slice(&[13, 14, 15]);
        capture.paths.len = 3;
        capture.names.first.copy_from_slice(b"gun");
        capture.names.len = 3;
        capture.supply_origins.first.copy_from_slice(&[
            Origin {
                province: 13,
                requested: 200_000,
                received: 50_000,
            },
            Origin {
                province: 13,
                requested: -123,
                received: 0,
            },
        ]);
        capture.supply_origins.len = 2;
        for origins_len in [
            Ok(2),
            Err(Failure::Unreadable),
            Err(Failure::OriginsCapacity { required: 3 }),
        ] {
            let mut row = gun((7_u64 << 32) | 4713);
            row.name = Some((0, 3));
            row.eta = Some(railway_gun_counter_inputs::EtaOperands {
                numerator: 3,
                current_speed: 4,
            });
            row.supply = Some(Inputs {
                asked: -12,
                received: 34,
                disruption: -56,
                origins_len,
                ..Default::default()
            });
            row.supply_failure = origins_len.err();
            capture.rows.rows[0] = row;
            let [TableView::Complete(TableRows::RailwayGunCounterInputs(rows))] = capture.tables()
            else {
                panic!("supply origins cannot withhold the counter or scalar footer");
            };
            let Recorded::Value(Some(supply)) = &rows[0].supply else {
                panic!("scalar supply values");
            };
            assert_eq!(
                (supply.asked, supply.received, supply.disruption),
                (-12, 34, -56)
            );
            assert_eq!(supply.origins.as_ref().map(Vec::len), origins_len.ok());
            if let Some(origins) = &supply.origins {
                assert_eq!(
                    (
                        origins[1].province,
                        origins[1].requested,
                        origins[1].received
                    ),
                    (13, -123, 0)
                );
            }
            assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
            assert!(matches!(rows[0].eta, Recorded::Value(Some(_))));
            assert_eq!(rows[0].current_strength, -123_456);
        }
        capture.rows.rows[0].supply = None;
        let [TableView::Complete(TableRows::RailwayGunCounterInputs(rows))] = capture.tables()
        else {
            panic!("counter prefix");
        };
        assert_eq!(rows[0].supply, Recorded::Value(None));
        assert_eq!(rows[0].name, Recorded::Value(Some(b"gun".to_vec())));
    }
}

#[cfg(test)]
mod combat_reference_projection_tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn railway_owned_reference_projection_and_refusal_preserve_counter_prefix() {
        let mut capture = Captured::with_capacities(&Capacities {
            rows: 1,
            paths: 0,
            names: 0,
            named_bonuses: 0,
            supply_origins: 0,
            combat_references: 2,
        });
        capture.rows.outcome = Outcome::Rows { len: 1 };
        capture.rows.rows[0] = railway_gun_counter_inputs::Row {
            gun: u64::MAX,
            current_strength: i64::MIN,
            combat_count: 2,
            combat_references: Some((0, 2)),
            ..Default::default()
        };
        capture.combat_references.first[..2].fill(engine::cunit::MapCombatReference {
            combat: 0x8000_0001_0000_1b5a,
            kind: 2,
        });
        capture.combat_references.len = 2;
        capture.sample_hour = Some(31);
        let [TableView::Complete(TableRows::RailwayGunCounterInputs(before))] = capture.tables()
        else {
            panic!("counter prefix unavailable");
        };
        let Recorded::Value(Some(references)) = &before[0].combat_references else {
            panic!("complete owned suffix unavailable");
        };
        assert_eq!((references.sample_hour, references.entries.len()), (31, 2));
        assert_eq!(references.entries[0], references.entries[1]);
        capture.combat_references.len = 1;
        let [TableView::Complete(TableRows::RailwayGunCounterInputs(refused))] = capture.tables()
        else {
            panic!("reference refusal erased the established prefix");
        };
        assert_eq!(refused[0].combat_references, Recorded::Value(None));
        let mut expected = before[0].clone();
        expected.combat_references = Recorded::Value(None);
        assert_eq!(refused[0], expected);
        capture.reset();
        assert_eq!(
            (capture.sample_hour, capture.combat_references.len),
            (None, 0)
        );
    }
}
