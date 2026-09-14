//! Coordinated extended acquisition. Public arrays use global TableId indices;
//! individual packs keep compact local arrays. Allocation is off-thread.

#[cfg(test)]
use crate::sampling::CONSISTENCY_TARGET;
use crate::{
    map_capture::{MapSources, SourceChecks},
    sampling::qpc,
    table_capture::{CaptureRows, comparison, comparison_rows, grow_capacity, grow_vec},
};
use engine::{
    Memory, cair_wing, carmy, cequipment_variant, cequipment_variant_pool::DivisionSource,
    corders_group, cship, walk::Outcome,
};
use replay::{
    frame::{BaselineToken, TABLE_COUNT, TableInput},
    rows::{ScriptVariablesRow, TableId},
};
use std::mem::size_of;

const MISSING: Outcome = Outcome::Unavailable {
    reason: "acquisition source not supplied",
};
const VARIANT_TABLES: [TableId; 4] = [
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
];
const DIVISION_SOURCE_TABLES: [TableId; 8] = [
    TableId::DivisionEquipment,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::DivisionEta,
];
pub type ReadGroup = (&'static str, TableId, Vec<TableId>);

// Native conversion reuse stays within its pack's existing timing scope.
macro_rules! spool_inputs {
    (lexical, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr) => {
        $captured.table_inputs($lexical)
    };
    (characters, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr) => {
        $captured.table_inputs($characters)
    };
    (intel, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr) => {
        $captured.table_inputs($baseline, $scratch)
    };
    (governance, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr) => {
        $captured.table_inputs($research)
    };
    ($pack:ident, $captured:expr, $baseline:expr, $scratch:expr, $research:expr, $characters:expr, $lexical:expr) => {
        $captured.tables().map(TableInput::Sampled)
    };
}

macro_rules! packs {
    ($($pack:ident),+ $(,)?) => {
        $(pub mod $pack;)+
        pub const PACK_NAMES: &[&str] = &[$(stringify!($pack)),+];
        pub const N: usize = 0 $(+ $pack::N)+;
        pub const TABLES: [TableId; N] = {
            let mut tables = [TableId::LexicalTokens; N];
            let mut next = 0;
            $(let mut index = 0;
            while index < $pack::N {
                tables[next] = $pack::TABLES[index]; next += 1; index += 1;
            })+
            assert!(next == N);
            tables
        };
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub struct Capacities {
            $(pub $pack: $pack::Capacities,)+
            pub division_sources: usize,
            pub variant_registry: usize,
        }
        impl Default for Capacities {
            fn default() -> Self {
                Self { $($pack: Default::default(),)+ division_sources: 8192, variant_registry: 8192 }
            }
        }
        pub struct Captured {
            $($pack: $pack::Captured,)+
            divisions: Divisions,
            eta_sources: division_eta::OwnedSources,
            eta_report: Option<division_eta::AcquisitionReport>,
            registry: Registry,
            air_roots: [Option<Outcome>; 2],
            overhead: [u64; TABLE_COUNT],
        }
        impl Captured {
            pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
                assert_eq!(successful.len(), TABLE_COUNT);
                Self {
                    $($pack: $pack::Captured::with_capacities(&capacity.$pack, &local(&$pack::TABLES, successful)),)+
                    divisions: Divisions::new(capacity.division_sources, division_successful(successful)),
                    eta_sources: division_eta::OwnedSources::new(capacity.division_sources, successful[TableId::DivisionEta.index()] < crate::sampling::CONSISTENCY_TARGET),
                    eta_report: None,
                    registry: Registry::new(capacity.variant_registry, variant_successful(successful)),
                    air_roots: [None; 2], overhead: [0; TABLE_COUNT],
                }
            }
            /// Reset before the shared division and map walks, not between
            /// source visitation and capture().
            pub fn reset(&mut self) {
                $(self.$pack.reset();)+
                self.divisions.reset(); self.registry.reset();
                self.eta_sources.clear(); self.eta_report = None;
                self.air_roots = [None; 2]; self.overhead = [0; TABLE_COUNT];
            }
            pub fn allocated_bytes(&self) -> usize {
                0 $(+ self.$pack.allocated_bytes())+ + self.divisions.allocated_bytes() + self.registry.allocated_bytes() + self.eta_sources.allocated_bytes()
            }
            /// Spool-thread metadata for this buffer's native row and scratch
            /// allocations. Excludes core tables, encoded frames and queues.
            pub fn allocated_by_pack(&self) -> Vec<(&'static str, usize)> {
                vec![
                    $((stringify!($pack), self.$pack.allocated_bytes()),)+
                    ("division_sources", self.divisions.allocated_bytes()),
                    ("eta_owned_sources", self.eta_sources.allocated_bytes()),
                    ("variant_registry", self.registry.allocated_bytes()),
                ]
            }
            pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
                assert_eq!(successful.len(), TABLE_COUNT);
                $(self.$pack.grow(&mut capacity.$pack, &local(&$pack::TABLES, successful));)+
                self.divisions.grow(&mut capacity.division_sources, division_successful(successful));
                self.eta_sources.grow(capacity.division_sources, successful[TableId::DivisionEta.index()] < crate::sampling::CONSISTENCY_TARGET);
                self.registry.grow(&mut capacity.variant_registry, variant_successful(successful));
            }
            /// Spool-thread conversion only. Existing core slots survive.
            pub(crate) fn insert_tables(
                &self,
                destination: &mut [TableInput; TABLE_COUNT],
                variables_baseline: Option<(BaselineToken, &[ScriptVariablesRow])>,
                variables_scratch: &mut intel::ScriptVariableComparison,
                research: Option<&mut governance::ResearchConversion<'_>>,
                characters: Option<&mut characters::CharacterConversion<'_>>,
                lexical: Option<&mut lexical::LexicalConversion<'_>>,
            ) -> [i64; PACK_NAMES.len()] {
                let mut costs = [0; PACK_NAMES.len()];
                let mut slots = costs.iter_mut();
                $(let started = qpc();
                for (id, view) in $pack::TABLES.into_iter().zip(spool_inputs!($pack, self.$pack, variables_baseline, variables_scratch, research, characters, lexical)) { destination[id.index()] = view; }
                *slots.next().unwrap() = qpc() - started;)+
                costs
            }
            pub fn outcomes(&self) -> [Outcome; TABLE_COUNT] {
                let mut output = [MISSING; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.outcomes(), &mut output);)+
                output
            }
            pub fn checked_mismatches(&self) -> [usize; TABLE_COUNT] {
                let mut output = [0; TABLE_COUNT];
                $(scatter(&$pack::TABLES, self.$pack.checked_mismatches(), &mut output);)+
                for table in VARIANT_TABLES {
                    output[table.index()] = output[table.index()].saturating_add(self.registry.rows.mismatches);
                }
                output
            }
        }
        /// Off-thread metadata. Sum members, including physically shared work
        /// charged to a pack's anchor, rather than counting that work twice.
        pub fn read_groups() -> Vec<ReadGroup> {
            let mut groups = Vec::new();
            $(for &(name, anchor, members) in $pack::READ_GROUPS {
                // This graph also consumes the shared division census, so
                // report it once in the combined source group below.
                if $pack::TABLES[anchor] == TableId::ArmyUpgradeGroups { continue; }
                groups.push((name, $pack::TABLES[anchor], members.iter().map(|&index| $pack::TABLES[index]).collect()));
            })+
            groups.push(("divisions, equipment, upgrades, movement, counter status and ETA", TableId::Divisions,
                std::iter::once(TableId::Divisions).chain(DIVISION_SOURCE_TABLES).collect()));
            groups.push(("states and hourly state flags", TableId::States,
                vec![TableId::States, TableId::StateFlags]));
            groups
        }
    };
}
packs!(
    air,
    characters,
    combat_progress,
    construction,
    division_eta,
    division_presentation,
    division_templates,
    economy,
    fog,
    governance,
    intel,
    lexical,
    map_names,
    naval_history,
    production,
    projects,
    session,
    state_flags,
    weather
);

impl Captured {
    /// Move only ETA-owned buffers plus copied source identities into the
    /// callback-bounded worker. Empty placeholders allocate nothing.
    pub(crate) fn take_eta_request(
        &mut self,
        base: usize,
        state: usize,
        checked: bool,
    ) -> crate::acquisition::EtaRequest {
        let started = qpc();
        self.eta_sources
            .copy_from(self.divisions.sources(), checked);
        let request = crate::acquisition::EtaRequest {
            base,
            state,
            checked,
            captured: std::mem::replace(&mut self.division_eta, division_eta::Captured::empty()),
            sources: std::mem::replace(
                &mut self.eta_sources,
                division_eta::OwnedSources::new(0, false),
            ),
        };
        self.overhead[TableId::DivisionEta.index()] += elapsed(started);
        request
    }
    /// Rejected jobs restore the normal synchronous path. Accepted jobs always
    /// install a terminal report; failure cannot silently retry or publish a
    /// worker's partial rows as a successful observation.
    pub(crate) fn restore_eta_request(
        &mut self,
        mut request: crate::acquisition::EtaRequest,
        report: Option<Result<division_eta::AcquisitionReport, crate::acquisition::Failure>>,
    ) {
        self.eta_report = report.map(|report| match report {
            Ok(report) => report,
            Err(failure) => {
                request.captured.fail(
                    match failure {
                        crate::acquisition::Failure::Panicked => "ETA acquisition worker panicked",
                        crate::acquisition::Failure::Stopped => {
                            "ETA acquisition worker stopped before reading"
                        }
                    },
                    request.checked,
                );
                division_eta::AcquisitionReport::default()
            }
        });
        request.sources.clear();
        self.division_eta = request.captured;
        self.eta_sources = request.sources;
    }
    fn acquire_eta(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: bool,
    ) -> ([bool; 1], [u64; 1]) {
        if let Some(report) = self.eta_report.take() {
            ([report.matched], [report.ticks])
        } else {
            self.division_eta
                .capture(memory, base, state, &[needed], self.divisions.sources())
        }
    }
    /// Decode-worker handoff for the hourly focus pair (copy/decode split).
    /// Sole cross-thread entry to the governance focus buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn focus_pilot(&mut self) -> &mut governance::Captured {
        &mut self.governance
    }

    /// Decode-worker handoff for the relations family (copy/decode split).
    /// Sole cross-thread entry to the intel relations buffers: the game thread
    /// stages raw-copy state through it on refresh hours and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn relations_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    /// Decode-worker handoff for the characters family (copy/decode split).
    /// Sole cross-thread entry to the characters slot-0 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff. No other extended state
    /// crosses threads here.
    pub(crate) fn characters_pilot(&mut self) -> &mut characters::Captured {
        &mut self.characters
    }

    /// Decode-worker handoff for the projects family (copy/decode split).
    /// Sole cross-thread entry to the projects slot-3 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn projects_pilot(&mut self) -> &mut projects::Captured {
        &mut self.projects
    }

    /// Decode-worker handoff for the lexical family (copy/decode split).
    /// Sole cross-thread entry to the lexical slot-3 buffers: the game
    /// thread stages raw-copy state through it on refresh hours and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn lexical_pilot(&mut self) -> &mut lexical::Captured {
        &mut self.lexical
    }

    /// Decode-worker handoff for the wing-fed air pair (copy/decode split).
    /// Sole cross-thread entry to the air wing-extension buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn air_wing_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the hourly air-statistics table (copy/decode
    /// split). Sole cross-thread entry to the statistics buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff. Shares the air pack buffers with
    /// the wing extensions; each stages its own copies with no cross-family
    /// decoded roots.
    pub(crate) fn air_stats_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the shared hourly bases payload
    /// (copy/decode split). Sole cross-thread entry to the air bases/aces
    /// buffers: the game thread stages raw-copy state through it every hour
    /// and the single FIFO decode worker decodes the staged payload into
    /// these pooled typed rows before the spool ready-queue handoff. The fog
    /// registrations half decodes from the same log (see below).
    pub(crate) fn air_bases_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the shared hourly history payload
    /// (copy/decode split). Sole cross-thread entry to the air history and
    /// damage buffers: the game thread stages raw-copy state through it
    /// every hour and the single FIFO decode worker decodes the staged
    /// payload into these pooled typed rows before the spool ready-queue
    /// handoff.
    pub(crate) fn air_history_pilot(&mut self) -> &mut air::Captured {
        &mut self.air
    }

    /// Decode-worker handoff for the source-fed naval block (copy/decode
    /// split). Sole cross-thread entry to the naval theatre/OOB buffers: the
    /// game thread stages raw-copy state through it every hour and the
    /// single FIFO decode worker decodes the staged payload into these
    /// pooled typed rows before the spool ready-queue handoff.
    pub(crate) fn naval_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly session family (copy/decode
    /// split). Sole cross-thread entry to the session buffers: the game
    /// thread stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn session_pilot(&mut self) -> &mut session::Captured {
        &mut self.session
    }

    /// Decode-worker handoff for the hourly supremacy payload (copy/decode
    /// split). Sole cross-thread entry to the supremacy buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn supremacy_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly sunk-ships payload (copy/decode
    /// split). Sole cross-thread entry to the sunk buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn sunk_pilot(&mut self) -> &mut naval_history::Captured {
        &mut self.naval_history
    }

    /// Decode-worker handoff for the hourly radar payload (copy/decode
    /// split). Sole cross-thread entry to the radar buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn radar_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    /// Decode-worker handoff for the hourly intel payload (copy/decode
    /// split). Sole cross-thread entry to the intel buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO
    /// decode worker decodes the staged payload into these pooled typed rows
    /// before the spool ready-queue handoff.
    pub(crate) fn intel_pilot(&mut self) -> &mut fog::Captured {
        &mut self.fog
    }

    /// Decode-worker handoff for the hourly weather family (copy/decode
    /// split). Sole cross-thread entry to the weather buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn weather_pilot(&mut self) -> &mut weather::Captured {
        &mut self.weather
    }

    /// Decode-worker handoff for the hourly production rest (copy/decode
    /// split). Sole cross-thread entry to the production rest buffers: the
    /// game thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn production_rest_pilot(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    /// Decode-worker handoff for the hourly construction rest (copy/decode
    /// split). Sole cross-thread entry to the construction rest buffers: the
    /// game thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff.
    pub(crate) fn construction_rest_pilot(&mut self) -> &mut construction::Captured {
        &mut self.construction
    }

    /// Decode-worker handoff for the hourly economy rest (copy/decode split).
    /// Sole cross-thread entry to the economy rest buffers: the game thread
    /// stages raw-copy state through it every hour and the single FIFO decode
    /// worker decodes the staged payload into these pooled typed rows before
    /// the spool ready-queue handoff.
    pub(crate) fn economy_rest_pilot(&mut self) -> &mut economy::Captured {
        &mut self.economy
    }

    /// Decode-worker handoff for the hourly capitulation table (copy/decode
    /// split). Sole cross-thread entry to the capitulation buffers: the game
    /// thread stages raw-copy state through it every hour and the single
    /// FIFO decode worker decodes the staged payload into these pooled typed
    /// rows before the spool ready-queue handoff. Shares the intel pack
    /// buffers with the relations/rest families; each stages its own copies
    /// with no cross-family decoded roots.
    pub(crate) fn capitulation_pilot(&mut self) -> &mut intel::Captured {
        &mut self.intel
    }

    /// Game-thread raw copy for the shared hourly bases payload. Runs the
    /// air-hosted copy (country census, manager, bases, registrations,
    /// aces) with disjoint borrows of both packs; the report comes back for
    /// sampling's calibration install, while matched rows and costs install
    /// into the returned report like every other family.
    pub(crate) fn copy_air_bases_refresh(
        &mut self,
        memory: &impl engine::Memory,
        ctx: air::AirBasesCopyContext,
        first: &mut air::AirBasesJob,
        second: Option<&mut air::AirBasesJob>,
    ) -> air::AirBasesCopyReport {
        self.air
            .copy_air_bases_refresh(memory, ctx, &mut self.fog, first, second)
    }

    /// Decode-worker interpretation of a raw shared bases payload, in pack
    /// order: bases and aces rows first, then the fog registrations half,
    /// all from the same shared log. Disjoint borrows keep both packs alive
    /// at once.
    pub(crate) fn decode_air_bases_payload(
        &mut self,
        base: usize,
        job: &air::AirBasesJob,
        memo: &mut air::AirBasesMemo,
        context: air::AirBasesMemoContext,
        decode_ticks: &mut [u64; air::AIR_BASES_N],
    ) -> air::AirBasesMemoReport {
        debug_assert_eq!(base, context.base);
        memo.decode(&mut self.air, &mut self.fog, job, context, decode_ticks)
    }

    /// Game-thread raw-mode gate for the hourly divisions group. Set every
    /// hour from the divisions copy report before `capture` runs the
    /// production acquire below.
    pub(crate) fn set_divisions_raw_mode(&mut self, raw: bool) {
        self.production.divisions_raw_mode = raw;
    }

    /// Decode-worker interpretation of a raw divisions-group payload, in
    /// dependency order: equipment first, then the upgrade graph, both
    /// against the frame's decoded division sources (replayed by the caller
    /// through the division visitor just before). Disjoint borrows keep the
    /// sources and the production buffers alive at once.
    pub(crate) fn decode_divisions_payload(
        &mut self,
        base: usize,
        log: &crate::raw::RawLog<6>,
        decode_ticks: &mut [u64; 6],
    ) {
        let recorded = crate::raw::Recorded::build(log);
        let sources = self.divisions.sources();
        self.production
            .decode_division_detail(base, &sources, &recorded, decode_ticks);
    }

    /// Test-only buffer resize for the divisions-group decode harness:
    /// shrinks the pooled row buffers to the harness lengths, mirroring a
    /// truncated production frame. Capacities are untouched.
    #[cfg(test)]
    pub(crate) fn test_resize_division_buffers(&mut self, lens: [usize; 6]) {
        use engine::{army_upgrade_request_row, carmy_upgrades_requests, cupgrade_delivery};
        self.production.division_equipment.rows =
            vec![engine::cequipment_variant_pool::DivisionRow::default(); lens[1]];
        self.production.groups.rows = vec![carmy_upgrades_requests::Row::default(); lens[2]];
        self.production.requests.rows = vec![army_upgrade_request_row::Row::default(); lens[3]];
        self.production.deliveries.rows = vec![cupgrade_delivery::Row::default(); lens[4]];
        self.production.produced.rows = vec![cupgrade_delivery::ProducedRow::default(); lens[5]];
    }

    /// Test-only read of the production division-detail buffers for the
    /// divisions-group decode harness.
    #[cfg(test)]
    pub(crate) fn test_division_detail(&mut self) -> &mut production::Captured {
        &mut self.production
    }

    pub fn source_checks(needed: &[bool]) -> SourceChecks {
        let air = local(&air::TABLES, needed);
        let naval = local(&naval_history::TABLES, needed);
        SourceChecks {
            land_combats: needed[TableId::LandCombatProgress.index()],
            ships: false,
            air_wings: air[0] || air[2] || naval_history::needs_wing_sources(&naval),
            order_groups: naval_history::needs_order_sources(&naval),
        }
    }
    pub fn needs_division_check(needed: &[bool]) -> bool {
        DIVISION_SOURCE_TABLES
            .iter()
            .any(|table| needed[table.index()])
    }
    pub fn needs_state_check(needed: &[bool]) -> bool {
        needed[TableId::StateFlags.index()]
    }
    /// Callback-scoped source copies are charged to the shared States census.
    pub fn visit_state(&mut self, source: engine::cstate::Source, second: bool) {
        self.state_flags.visit(source, second);
    }
    pub fn finish_states(&mut self, outcome: Outcome, second: bool) {
        self.state_flags.finish(outcome, second);
    }

    /// Source copies remain inside the enclosing Divisions measurement. The
    /// shared read group includes equipment and upgrade tables without per-row
    /// timers or repeated source censuses for individual tables.
    pub fn visit_division(&mut self, pointer: usize, row: &carmy::Row, second: bool) -> u64 {
        self.divisions.visit(pointer, row, second);
        0
    }
    pub(crate) fn visit_division_source(&mut self, source: DivisionSource, second: bool) {
        self.divisions.visit_source(source, second);
    }
    pub(crate) fn reset_division_sources(&mut self) {
        self.divisions.reset();
    }
    #[cfg(test)]
    pub(crate) fn test_division_presentation_tables(&self) -> [replay::frame::TableView; 2] {
        self.division_presentation.tables()
    }
    pub fn finish_divisions(&mut self, outcome: Outcome, second: bool) -> u64 {
        self.divisions.finish(outcome, second);
        0
    }
    pub fn sources<'a, M: Memory>(
        &'a mut self,
        memory: &'a M,
        base: usize,
        needed: &[bool],
        air_raw: bool,
        naval_raw: bool,
    ) -> Sources<'a, M> {
        assert_eq!(needed.len(), TABLE_COUNT);
        let naval = local(&naval_history::TABLES, needed);
        let air_needed = local(&air::TABLES, needed);
        Sources {
            captured: self,
            memory,
            base,
            // Staged payloads, including faulted ones, own these consumers.
            // Static allocation checks above remain conservative for fallback.
            checks: SourceChecks {
                land_combats: needed[TableId::LandCombatProgress.index()],
                ships: false,
                air_wings: (!air_raw && (air_needed[0] || air_needed[2]))
                    || (!naval_raw && naval_history::needs_wing_sources(&naval)),
                order_groups: !naval_raw && naval_history::needs_order_sources(&naval),
            },
            air_needed,
            air_raw,
            naval_raw,
            naval_wing_second: naval_history::needs_wing_sources(&naval),
            naval_order_second: naval_history::needs_order_sources(&naval),
        }
    }
    /// Run after root division acquisition and map.sample. Returned arrays use
    /// global TableId indices; core slots remain zero. The registry census and
    /// variant details run together on their existing cadence.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        current_hour: i32,
        needed: &[bool],
        force_full: bool,
    ) -> ([bool; TABLE_COUNT], [u64; TABLE_COUNT]) {
        assert_eq!(needed.len(), TABLE_COUNT);
        let mut matched = [false; TABLE_COUNT];
        let mut ticks = [0; TABLE_COUNT];
        macro_rules! ordinary {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed));
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        macro_rules! staggered {
            ($($pack:ident),+ $(,)?) => {$(
                let (pass, cost) = self.$pack.capture(memory, base, game_state, &local(&$pack::TABLES, needed), current_hour, force_full);
                scatter(&$pack::TABLES, pass, &mut matched);
                scatter(&$pack::TABLES, cost, &mut ticks);
            )+};
        }
        ordinary!(economy, fog, session, weather, combat_progress, state_flags);
        staggered!(
            characters,
            construction,
            governance,
            intel,
            projects,
            map_names,
            division_templates
        );
        let air_needed = local(&air::TABLES, needed);
        self.air.finish_wings(
            self.air_roots[0].unwrap_or(MISSING),
            self.air_roots[1],
            &air_needed,
        );
        let (pass, cost) = self.air.capture(memory, base, game_state, &air_needed);
        scatter(&air::TABLES, pass, &mut matched);
        scatter(&air::TABLES, cost, &mut ticks);
        let (pass, cost) = self.naval_history.capture(
            memory,
            base,
            game_state,
            current_hour,
            &local(&naval_history::TABLES, needed),
        );
        scatter(&naval_history::TABLES, pass, &mut matched);
        scatter(&naval_history::TABLES, cost, &mut ticks);

        let production_needed = local(&production::TABLES, needed);
        let (pass, cost) = self.production.acquire(
            memory,
            base,
            game_state,
            &production_needed,
            self.divisions.sources(),
            current_hour,
            force_full,
        );
        scatter(&production::TABLES, pass, &mut matched);
        scatter(&production::TABLES, cost, &mut ticks);
        let (pass, cost) = self.division_presentation.capture(
            memory,
            base,
            game_state,
            &local(&division_presentation::TABLES, needed),
            self.divisions.sources(),
        );
        scatter(&division_presentation::TABLES, pass, &mut matched);
        scatter(&division_presentation::TABLES, cost, &mut ticks);
        let (pass, cost) = self.acquire_eta(
            memory,
            base,
            game_state,
            needed[TableId::DivisionEta.index()],
        );
        scatter(&division_eta::TABLES, pass, &mut matched);
        scatter(&division_eta::TABLES, cost, &mut ticks);
        // The saved campaign registry is authoritative. It includes variants
        // absent from units, current production and stockpiles. Preserve every
        // native entry and duplicate for shared off-thread key rejection.
        let variant_walk =
            replay::cadence::should_walk(TableId::VariantDefinitions, current_hour, force_full);
        if variant_walk {
            let begin = qpc();
            self.registry
                .acquire(memory, base, game_state, variant_needed(needed));
            ticks[TableId::VariantDefinitions.index()] += elapsed(begin);
        } else {
            self.registry.mark_skipped();
        }
        let (pass, cost) = self.production.finish_variants(
            memory,
            base,
            &production_needed,
            self.registry.sources(),
            current_hour,
            force_full,
        );
        for (index, table) in production::TABLES.iter().enumerate() {
            matched[table.index()] |= pass[index];
            ticks[table.index()] = ticks[table.index()].saturating_add(cost[index]);
        }
        staggered!(lexical);
        for table in TABLES {
            ticks[table.index()] =
                ticks[table.index()].saturating_add(self.overhead[table.index()]);
            matched[table.index()] &= needed[table.index()]
                && replay::cadence::should_walk(table, current_hour, force_full);
        }
        (matched, ticks)
    }
}

pub struct Sources<'a, M: Memory> {
    captured: &'a mut Captured,
    memory: &'a M,
    base: usize,
    checks: SourceChecks,
    air_needed: [bool; air::N],
    air_raw: bool,
    naval_raw: bool,
    naval_wing_second: bool,
    naval_order_second: bool,
}
impl<M: Memory> Sources<'_, M> {
    fn wing_ticks(&self) -> u64 {
        self.captured.air.wing_ticks().into_iter().sum::<u64>()
            + self.captured.naval_history.source_ticks()[1]
    }
    fn attribute_overhead(&mut self, table: TableId, begin: i64, separately_measured: u64) -> u64 {
        let total = elapsed(begin);
        self.captured.overhead[table.index()] = self.captured.overhead[table.index()]
            .saturating_add(total.saturating_sub(separately_measured));
        total
    }
}
impl<M: Memory> MapSources for Sources<'_, M> {
    fn checks(&self) -> SourceChecks {
        self.checks
    }
    fn begin_land(&mut self, second: bool) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured.combat_progress.begin(second);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn land(&mut self, second: bool, source: engine::cland_combat::Source) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured
            .combat_progress
            .visit(self.memory, self.base, source, second);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn finish_land(&mut self, second: bool, outcome: Outcome) -> u64 {
        if second && !self.checks.land_combats {
            return 0;
        }
        let begin = qpc();
        self.captured.combat_progress.finish(second, outcome);
        self.attribute_overhead(TableId::LandCombatProgress, begin, 0)
    }
    fn begin_ships(&mut self, _: bool) -> u64 {
        0
    }
    fn ship(&mut self, _: bool, _: usize, _: &cship::Row) -> u64 {
        0
    }
    fn finish_ships(&mut self, _: bool, _: Outcome) -> u64 {
        0
    }
    fn begin_air(&mut self, second: bool) -> u64 {
        if self.air_raw {
            return 0;
        }
        let begin = qpc();
        if !second {
            self.captured.air.begin_wings();
        }
        self.attribute_overhead(TableId::WingEquipment, begin, 0)
    }
    fn air(&mut self, second: bool, pointer: usize, _: usize, row: &cair_wing::Row) -> u64 {
        if self.air_raw && (self.naval_raw || (second && !self.naval_wing_second)) {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air.visit_wing(
                self.memory,
                self.base,
                air::WingSource {
                    pointer,
                    row,
                    second,
                },
                &self.air_needed,
            );
        }
        if !self.naval_raw && (!second || self.naval_wing_second) {
            self.captured
                .naval_history
                .visit_wing(self.memory, self.base, pointer, row, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn finish_air(&mut self, second: bool, outcome: Outcome) -> u64 {
        if self.air_raw && (self.naval_raw || (second && !self.naval_wing_second)) {
            return 0;
        }
        let begin = qpc();
        let before = self.wing_ticks();
        if !self.air_raw {
            self.captured.air_roots[usize::from(second)] = Some(outcome);
        }
        if !self.naval_raw && (!second || self.naval_wing_second) {
            self.captured.naval_history.finish_wings(outcome, second);
        }
        let charged = self.wing_ticks().saturating_sub(before);
        self.attribute_overhead(TableId::WingEquipment, begin, charged)
    }
    fn order_groups(
        &mut self,
        second: bool,
        roots: &[corders_group::Root],
        outcome: Outcome,
    ) -> u64 {
        if self.naval_raw || (second && !self.naval_order_second) {
            return 0;
        }
        let begin = qpc();
        let ticks = self.captured.naval_history.source_ticks();
        let before = ticks[0] + ticks[2];
        self.captured.naval_history.set_order_sources(
            self.memory,
            self.base,
            roots,
            outcome,
            second,
        );
        let ticks = self.captured.naval_history.source_ticks();
        let charged = (ticks[0] + ticks[2]).saturating_sub(before);
        self.attribute_overhead(TableId::LandTheatres, begin, charged)
    }
}

fn local<T: Copy, const N: usize>(tables: &[TableId; N], global: &[T]) -> [T; N] {
    tables.map(|table| global[table.index()])
}
fn scatter<T: Copy, const N: usize>(
    tables: &[TableId; N],
    local: [T; N],
    global: &mut [T; TABLE_COUNT],
) {
    for (table, value) in tables.iter().zip(local) {
        global[table.index()] = value;
    }
}
fn variant_successful(successful: &[u16]) -> u16 {
    VARIANT_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn division_successful(successful: &[u16]) -> u16 {
    DIVISION_SOURCE_TABLES
        .iter()
        .map(|table| successful[table.index()])
        .min()
        .unwrap()
}
fn variant_needed(needed: &[bool]) -> bool {
    VARIANT_TABLES.iter().any(|table| needed[table.index()])
}
fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

struct Divisions {
    first: Vec<DivisionSource>,
    second: Vec<DivisionSource>,
    seen: [usize; 2],
    outcomes: [Outcome; 2],
    required: usize,
}
impl Divisions {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            first: vec![Default::default(); capacity],
            second: comparison_rows(capacity, successful),
            seen: [0; 2],
            outcomes: [MISSING; 2],
            required: 0,
        }
    }
    fn reset(&mut self) {
        self.seen = [0; 2];
        self.outcomes = [MISSING; 2];
        self.required = 0;
    }
    fn visit(&mut self, pointer: usize, row: &carmy::Row, second: bool) {
        self.visit_source(
            DivisionSource {
                pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            },
            second,
        );
    }
    fn visit_source(&mut self, source: DivisionSource, second: bool) {
        let pass = usize::from(second);
        let destination = if second {
            &mut self.second
        } else {
            &mut self.first
        };
        if let Some(slot) = destination.get_mut(self.seen[pass]) {
            *slot = source;
        }
        self.seen[pass] = self.seen[pass].saturating_add(1);
        self.required = self.required.max(self.seen[pass]);
    }
    fn finish(&mut self, outcome: Outcome, second: bool) {
        let pass = usize::from(second);
        let capacity = if second {
            self.second.len()
        } else {
            self.first.len()
        };
        self.outcomes[pass] = match outcome {
            Outcome::Rows { len } if len != self.seen[pass] => Outcome::Unavailable {
                reason: "division visitor and source row count disagree",
            },
            Outcome::Rows { len } if len > capacity => truncated_source(capacity, len - capacity),
            Outcome::Truncated { len, unread_rows } => {
                self.required = self.required.max(len.saturating_add(unread_rows as usize));
                let stored = len.min(capacity);
                truncated_source(
                    stored,
                    len.saturating_sub(stored)
                        .saturating_add(unread_rows as usize),
                )
            }
            _ => outcome,
        };
    }
    fn sources(&self) -> production::Sources<'_, DivisionSource> {
        production::Sources {
            first: &self.first,
            first_outcome: self.outcomes[0],
            second: &self.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        grow_capacity(capacity, self.required);
        grow_vec(&mut self.first, *capacity);
        comparison(&mut self.second, *capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        (self.first.capacity() + self.second.capacity()) * size_of::<DivisionSource>()
    }
}
fn truncated_source(len: usize, omitted: usize) -> Outcome {
    match u32::try_from(omitted) {
        Ok(unread_rows) if unread_rows > 0 => Outcome::Truncated { len, unread_rows },
        _ => Outcome::Unavailable {
            reason: "source omitted row count invalid or exceeds u32",
        },
    }
}

struct Registry {
    rows: CaptureRows<cequipment_variant::RegistryRow>,
    outcomes: [Outcome; 2],
}
impl Registry {
    fn new(capacity: usize, successful: u16) -> Self {
        Self {
            rows: CaptureRows::new(capacity, successful),
            outcomes: [MISSING; 2],
        }
    }
    fn reset(&mut self) {
        self.rows.reset();
        self.outcomes = [MISSING; 2];
    }
    fn acquire(&mut self, memory: &impl Memory, base: usize, game_state: usize, needed: bool) {
        self.rows.outcome = bounded_registry(
            cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.rows),
            self.rows.rows.len(),
        );
        self.outcomes[0] = self.rows.outcome;
        if needed {
            self.rows.checked = true;
            let second = bounded_registry(
                cequipment_variant::walk_registry(memory, base, game_state, &mut self.rows.second),
                self.rows.second.len(),
            );
            self.outcomes[1] = second;
            self.rows.compare(second);
            if self.rows.mismatches != 0 {
                self.outcomes = [Outcome::Unavailable {
                    reason: "variant registry changed during double read",
                }; 2];
            }
        }
    }
    // Off-hours carry-forward hint keeps byte-identical spool output.
    fn mark_skipped(&mut self) {
        self.rows.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        self.outcomes = [self.rows.outcome; 2];
    }
    fn sources(&self) -> production::Sources<'_, cequipment_variant::RegistryRow> {
        production::Sources {
            first: &self.rows.rows,
            first_outcome: self.outcomes[0],
            second: &self.rows.second,
            second_outcome: self.outcomes[1],
        }
    }
    fn grow(&mut self, capacity: &mut usize, successful: u16) {
        self.rows.grow(capacity, successful);
    }
    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
}
fn bounded_registry(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "variant registry count exceeds buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "variant registry truncation has no omitted rows",
        }
    } else {
        outcome
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn eta_owned_handoff_restores_storage_and_terminal_worker_results_without_rereading() {
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("completed ETA must not acquire live memory again");
            }
        }
        let capacity = Capacities {
            division_sources: 2,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let bytes = captured.allocated_bytes();
        let row = carmy::Row {
            id: 7,
            ..Default::default()
        };
        captured.divisions.visit(0x1234, &row, false);
        captured.divisions.finish(Outcome::Rows { len: 1 }, false);
        captured.divisions.visit(0x5678, &row, true);
        captured.divisions.finish(Outcome::Rows { len: 1 }, true);
        let mut request = captured.take_eta_request(11, 22, true);
        assert_eq!(
            (request.base, request.state, request.checked),
            (11, 22, true)
        );
        assert_eq!(request.sources.view().first[0].pointer, 0x1234);
        assert_eq!(request.sources.view().second[0].pointer, 0x5678);
        assert!(captured.allocated_bytes() < bytes);
        request.captured.fail("synthetic terminal ETA", true);
        captured.restore_eta_request(
            request,
            Some(Ok(division_eta::AcquisitionReport {
                matched: false,
                ticks: 123,
            })),
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_eq!(
            captured.acquire_eta(&NoRead, 11, 22, true),
            ([false], [123])
        );
        assert_eq!(
            captured.division_eta.outcomes(),
            [Outcome::Unavailable {
                reason: "synthetic terminal ETA"
            }]
        );

        for failure in [
            crate::acquisition::Failure::Panicked,
            crate::acquisition::Failure::Stopped,
        ] {
            let request = captured.take_eta_request(11, 22, true);
            captured.restore_eta_request(request, Some(Err(failure)));
            assert_eq!(captured.acquire_eta(&NoRead, 11, 22, true), ([false], [0]));
            assert!(matches!(
                captured.division_eta.outcomes()[0],
                Outcome::Unavailable { .. }
            ));
            assert_eq!(captured.allocated_bytes(), bytes);
        }
        let request = captured.take_eta_request(11, 22, true);
        captured.restore_eta_request(request, None);
        assert!(captured.eta_report.is_none());
        assert_eq!(captured.allocated_bytes(), bytes);
        // A rejected dispatch leaves the shared source census untouched, for
        // the existing synchronous path and the other division consumers.
        assert_eq!(captured.divisions.first[0].pointer, 0x1234);
        assert_eq!(captured.divisions.second[0].pointer, 0x5678);
        captured.reset();
        assert!(captured.eta_report.is_none());
    }

    #[test]
    fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
        assert_eq!(N, 135);
        let actual: BTreeSet<_> = TABLES.into_iter().collect();
        assert_eq!(actual.len(), N);
        let expected: BTreeSet<_> = TableId::ALL
            .into_iter()
            .filter(|table| table.index() >= 20)
            .collect();
        assert_eq!(actual, expected);
        for (name, anchor, members) in read_groups() {
            assert!(!name.is_empty());
            assert!(members.contains(&anchor));
            if anchor == TableId::Divisions {
                assert_eq!(
                    members,
                    std::iter::once(TableId::Divisions)
                        .chain(DIVISION_SOURCE_TABLES)
                        .collect::<Vec<_>>()
                );
            } else if anchor == TableId::States {
                assert_eq!(members, vec![TableId::States, TableId::StateFlags]);
            } else {
                assert!(members.iter().all(|table| actual.contains(table)));
            }
        }
        let groups = read_groups();
        for table in DIVISION_SOURCE_TABLES {
            assert_eq!(
                groups
                    .iter()
                    .filter(|(_, _, members)| members.contains(&table))
                    .count(),
                1,
                "shared division cost counted more than once for {table:?}"
            );
        }
    }
    #[test]
    fn only_genuine_map_dependencies_retain_sources_and_registry_has_its_own_window() {
        let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
        successful[TableId::VariantStats.index()] -= 1;
        let needed = successful.map(|count| count < CONSISTENCY_TARGET);
        assert!(!Captured::needs_division_check(&needed));
        assert_eq!(Captured::source_checks(&needed), SourceChecks::default());
        assert_eq!(variant_successful(&successful), CONSISTENCY_TARGET - 1);
        let mut registry = Registry::new(2, variant_successful(&successful));
        assert_eq!(registry.rows.second.len(), 2);
        assert_eq!(
            registry.allocated_bytes(),
            4 * size_of::<cequipment_variant::RegistryRow>()
        );
        let mut capacity = 2;
        registry.grow(&mut capacity, CONSISTENCY_TARGET);
        assert!(registry.rows.second.is_empty());
        assert_eq!(
            registry.allocated_bytes(),
            2 * size_of::<cequipment_variant::RegistryRow>()
        );
        successful[TableId::AirTheatreMembers.index()] = 0;
        assert!(
            Captured::source_checks(&successful.map(|count| count < CONSISTENCY_TARGET)).air_wings
        );
        successful[TableId::DivisionEquipment.index()] = 0;
        assert!(Captured::needs_division_check(
            &successful.map(|count| count < CONSISTENCY_TARGET)
        ));
    }
    #[test]
    fn division_source_comparison_waits_for_every_equipment_and_upgrade_table() {
        for pending in DIVISION_SOURCE_TABLES {
            let mut successful = [CONSISTENCY_TARGET; TABLE_COUNT];
            successful[pending.index()] -= 1;
            let needed = successful.map(|count| count < CONSISTENCY_TARGET);
            assert!(Captured::needs_division_check(&needed), "{pending:?}");
            assert_eq!(division_successful(&successful), CONSISTENCY_TARGET - 1);
            let mut source = Divisions::new(2, division_successful(&successful));
            let mut capacity = 2;
            source.grow(&mut capacity, division_successful(&successful));
            assert_eq!(source.second.len(), 2, "{pending:?}");

            successful[pending.index()] += 1;
            assert!(!Captured::needs_division_check(
                &successful.map(|count| count < CONSISTENCY_TARGET)
            ));
            source.grow(&mut capacity, division_successful(&successful));
            assert!(source.second.is_empty(), "{pending:?}");
            assert_eq!(source.first.len(), 2);
        }
    }
    #[test]
    fn division_sources_preserve_roster_owner_and_reject_an_incomplete_parent() {
        let mut sources = Divisions::new(1, 0);
        let row = carmy::Row {
            id: (1_u64 << 48) | 14,
            roster_owner: 17,
            logical_owner: 23,
            template: u64::MAX,
            ..Default::default()
        };
        sources.visit(0x1234, &row, false);
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert_eq!(sources.outcomes[0], Outcome::Rows { len: 1 });
        assert_eq!(
            sources.first[0],
            DivisionSource {
                pointer: 0x1234,
                division: row.id,
                template: u64::MAX,
                country: 17
            }
        );
        sources.visit(0x5678, &row, false);
        sources.finish(
            Outcome::Truncated {
                len: 2,
                unread_rows: 7,
            },
            false,
        );
        assert_eq!(
            sources.outcomes[0],
            Outcome::Truncated {
                len: 1,
                unread_rows: 8
            }
        );
        assert_eq!(sources.required, 9);
        sources.reset();
        sources.finish(Outcome::Rows { len: 1 }, false);
        assert!(matches!(sources.outcomes[0], Outcome::Unavailable { .. }));
    }
    #[test]
    fn registry_false_counts_are_rejected_and_incomplete_censuses_remain_incomplete() {
        assert!(matches!(
            bounded_registry(Outcome::Rows { len: 3 }, 2),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            bounded_registry(
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 0
                },
                2
            ),
            Outcome::Unavailable { .. }
        ));
        let empty = bounded_registry(Outcome::Rows { len: 0 }, 0);
        assert_eq!(empty, Outcome::Rows { len: 0 });
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 9,
        };
        assert_eq!(bounded_registry(truncated, 1), truncated);
    }

    #[test]
    fn focus_pilot_handoff_reaches_the_governance_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.focus_pilot().focus_raw_mode = true;
        assert!(captured.governance.focus_raw_mode);
        captured.focus_pilot().focus_raw_mode = false;
        assert!(!captured.governance.focus_raw_mode);
    }

    #[test]
    fn relations_pilot_handoff_reaches_the_intel_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.relations_pilot().relations_raw_mode = true;
        assert!(captured.intel.relations_raw_mode);
        captured.relations_pilot().relations_raw_mode = false;
        assert!(!captured.intel.relations_raw_mode);
    }

    #[test]
    fn characters_pilot_handoff_reaches_the_characters_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.characters_pilot().raw_mode = true;
        assert!(captured.characters.raw_mode);
        captured.characters_pilot().raw_mode = false;
        assert!(!captured.characters.raw_mode);
    }

    #[test]
    fn projects_pilot_handoff_reaches_the_projects_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.projects_pilot().raw_mode = true;
        assert!(captured.projects.raw_mode);
        captured.projects_pilot().raw_mode = false;
        assert!(!captured.projects.raw_mode);
    }

    #[test]
    fn lexical_pilot_handoff_reaches_the_lexical_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.lexical_pilot().raw_mode = true;
        assert!(captured.lexical.raw_mode);
        captured.lexical_pilot().raw_mode = false;
        assert!(!captured.lexical.raw_mode);
    }

    #[test]
    fn production_rest_pilot_handoff_reaches_the_production_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.production_rest_pilot().rest_raw_mode = true;
        assert!(captured.production.rest_raw_mode);
        captured.production_rest_pilot().rest_raw_mode = false;
        assert!(!captured.production.rest_raw_mode);
    }

    #[test]
    fn construction_rest_pilot_handoff_reaches_the_construction_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.construction_rest_pilot().rest_raw_mode = true;
        assert!(captured.construction.rest_raw_mode);
        captured.construction_rest_pilot().rest_raw_mode = false;
        assert!(!captured.construction.rest_raw_mode);
    }

    #[test]
    fn economy_rest_pilot_handoff_reaches_the_economy_pack() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT]);
        captured.economy_rest_pilot().rest_raw_mode = true;
        assert!(captured.economy.rest_raw_mode);
        captured.economy_rest_pilot().rest_raw_mode = false;
        assert!(!captured.economy.rest_raw_mode);
    }

    #[derive(Default)]
    struct SourceReads {
        reads: std::cell::RefCell<Vec<(usize, usize)>>,
        bytes: Option<Vec<u8>>,
    }
    impl Memory for SourceReads {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, destination.len()));
            self.bytes.as_ref().is_some_and(|bytes| {
                engine::memory::SliceMemory {
                    base: air::wing_test_kit::BASE,
                    bytes,
                }
                .copy(address, destination)
            })
        }
    }

    #[test]
    fn source_adapter_routes_each_staged_consumer_without_replacing_faults() {
        let capacity = Capacities::default();
        let mut actual = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let mut expected = Captured::with_capacities(&capacity, &[0; TABLE_COUNT]);
        let row = cair_wing::Row {
            id: 0x1234_5678_0000_0009,
            ..Default::default()
        };
        // Each dependency alone and no checks cover independent second-pass
        // requests in all four raw-admission combinations, with faults and
        // successful nonempty wing rows from the existing typed fixture.
        for readable in [false, true] {
            for pending in [
                None,
                Some(TableId::WingEquipment),
                Some(TableId::WingCumulative),
                Some(TableId::AirTheatreMembers),
                Some(TableId::LandTheatreMembers),
                Some(TableId::OrdersGroups),
                Some(TableId::OrdersGroupMembers),
                Some(TableId::ArmyGroupMembers),
            ] {
                for air_raw in [false, true] {
                    for naval_raw in [false, true] {
                        actual.reset();
                        expected.reset();
                        let mut needed = [false; TABLE_COUNT];
                        if let Some(table) = pending {
                            needed[table.index()] = true;
                        }
                        let static_checks = Captured::source_checks(&needed);
                        let air_needed = local(&air::TABLES, &needed);
                        let naval_needed = local(&naval_history::TABLES, &needed);
                        let wing_second = naval_history::needs_wing_sources(&naval_needed);
                        let order_second = naval_history::needs_order_sources(&naval_needed);
                        let memory = SourceReads {
                            bytes: readable.then(air::wing_test_kit::fixture),
                            ..Default::default()
                        };
                        let reference_memory = SourceReads {
                            bytes: readable.then(air::wing_test_kit::fixture),
                            ..Default::default()
                        };
                        {
                            let mut sources =
                                actual.sources(&memory, 0, &needed, air_raw, naval_raw);
                            assert_eq!(
                                sources.checks(),
                                SourceChecks {
                                    land_combats: needed[TableId::LandCombatProgress.index()],
                                    ships: false,
                                    air_wings: (!air_raw && (air_needed[0] || air_needed[2]))
                                        || (!naval_raw && wing_second),
                                    order_groups: !naval_raw && order_second,
                                }
                            );
                            for second in [false, true] {
                                sources.begin_air(second);
                                sources.air(second, air::wing_test_kit::WING, 0, &row);
                                sources.finish_air(second, Outcome::Rows { len: 1 });
                                sources.order_groups(
                                    second,
                                    &[],
                                    Outcome::Unavailable {
                                        reason: "owned source fault",
                                    },
                                );

                                // Independent direct consumers, not the adapter's
                                // routing helpers, define the old live fallback.
                                if !air_raw {
                                    if !second {
                                        expected.air.begin_wings();
                                    }
                                    expected.air.visit_wing(
                                        &reference_memory,
                                        0,
                                        air::WingSource {
                                            pointer: air::wing_test_kit::WING,
                                            row: &row,
                                            second,
                                        },
                                        &air_needed,
                                    );
                                    expected.air_roots[usize::from(second)] =
                                        Some(Outcome::Rows { len: 1 });
                                }
                                if !naval_raw && (!second || wing_second) {
                                    expected.naval_history.visit_wing(
                                        &reference_memory,
                                        0,
                                        air::wing_test_kit::WING,
                                        &row,
                                        second,
                                    );
                                    expected
                                        .naval_history
                                        .finish_wings(Outcome::Rows { len: 1 }, second);
                                }
                                if !naval_raw && (!second || order_second) {
                                    expected.naval_history.set_order_sources(
                                        &reference_memory,
                                        0,
                                        &[],
                                        Outcome::Unavailable {
                                            reason: "owned source fault",
                                        },
                                        second,
                                    );
                                }
                            }
                        }
                        assert_eq!(*memory.reads.borrow(), *reference_memory.reads.borrow());
                        assert_eq!(actual.air_roots, expected.air_roots);
                        assert_eq!(actual.air.tables(), expected.air.tables());
                        assert_eq!(
                            actual.naval_history.tables(),
                            expected.naval_history.tables()
                        );
                        assert_eq!(Captured::source_checks(&needed), static_checks);
                        if readable && !air_raw {
                            assert_eq!(actual.air.outcomes()[0], Outcome::Rows { len: 2 });
                            assert_eq!(actual.air.outcomes()[2], Outcome::Rows { len: 1 });
                        }
                        if air_raw && naval_raw {
                            assert!(memory.reads.borrow().is_empty());
                            assert_eq!(actual.air_roots, [None; 2]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dynamic_source_checks_never_release_static_fallback_comparison_storage() {
        let mut captured = Captured::with_capacities(&Capacities::default(), &[0; TABLE_COUNT]);
        let memory = SourceReads::default();
        for index in 0..TABLE_COUNT {
            let mut needed = [false; TABLE_COUNT];
            needed[index] = true;
            let static_checks = Captured::source_checks(&needed);
            assert_eq!(
                captured.sources(&memory, 0, &needed, false, false).checks(),
                static_checks
            );
            assert_eq!(
                captured.sources(&memory, 0, &needed, true, true).checks(),
                SourceChecks {
                    land_combats: needed[TableId::LandCombatProgress.index()],
                    ..SourceChecks::default()
                }
            );
            assert_eq!(Captured::source_checks(&needed), static_checks);
        }
        let needed = [true; TABLE_COUNT];
        for air_raw in [false, true] {
            for naval_raw in [false, true] {
                let checks = captured
                    .sources(&memory, 0, &needed, air_raw, naval_raw)
                    .checks();
                assert_eq!(checks.air_wings, !air_raw || !naval_raw);
                assert_eq!(checks.order_groups, !naval_raw);
            }
        }
    }
}
