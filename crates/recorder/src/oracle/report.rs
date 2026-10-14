use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_save::Decimal;
use replay::{
    BranchId, Hour,
    container::Replay,
    display::Step,
    frame::{DecodedFrame, TableView},
    identity::SampleBoundary,
    rows::{TableId, TableRows},
};

use super::{complete_table, invalid};

/// The tables a comparison group compares, when its name says so: a group
/// named after one table, optionally with a family prefix ("economy
/// CountryFuel"), or one of the named multi-table groups. Other groups name
/// no tables and are always compared.
pub(super) fn group_tables(name: &str) -> Vec<TableId> {
    let last = name.rsplit(' ').next().unwrap_or(name);
    match last {
        "Deployment" => vec![
            TableId::DeploymentConveyors,
            TableId::DeploymentLines,
            TableId::DeploymentManpower,
            TableId::DeploymentEquipment,
        ],
        "ArmyUpgrades" => vec![
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ],
        "Variants" => vec![
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantStats,
            TableId::VariantUpgrades,
        ],
        "IndustrialOrganisations" => vec![
            TableId::IndustrialOrganisations,
            TableId::IndustrialOrganisationHistory,
        ],
        _ => TableId::ALL
            .into_iter()
            .filter(|table| format!("{table:?}") == last)
            .collect(),
    }
}

/// Whether `frame` read the daily `table`: at its own slot hour by the
/// cadence, since the slot's walk is always taken, or off it when its own walk
/// or another table of its slot was timed (a forced full hour). A timing alone
/// is no proof either way: a shared read is charged to one anchor, which can
/// be an hourly table (the economy-rest raw-copy split charges the Daily(17)
/// tables' read to ProvinceSupply). CountryIntel's walk can move off its slot
/// alone, so it is read only where its own walk was timed, and it and the
/// other tables of its slot are no evidence for each other.
pub(crate) fn daily_read(hour: Hour, metrics: &replay::frame::Metrics, table: TableId) -> bool {
    let timed = |table: TableId| metrics.table_ns[table.index()] != 0;
    if table == TableId::CountryIntel {
        return timed(table);
    }
    let cadence = replay::cadence::cadence(table);
    cadence.is_refresh(hour)
        || TableId::ALL.into_iter().any(|other| {
            other != TableId::CountryIntel
                && replay::cadence::cadence(other) == cadence
                && timed(other)
        })
}

/// Per daily table, the last hour at or before `hour` on `branch` whose frame
/// read it ([`daily_read`]). Blocks are read newest first, keeping frame
/// metrics and no table payloads, and the scan stops once every daily table
/// has a read hour (a branch starts with a full read).
pub(super) fn last_read_hours(
    replay: &Replay,
    branch: BranchId,
    hour: Hour,
) -> io::Result<BTreeMap<TableId, Hour>> {
    let daily = TableId::ALL
        .into_iter()
        .filter(|&table| {
            matches!(
                replay::cadence::cadence(table),
                replay::cadence::Cadence::Daily(_)
            )
        })
        .collect::<Vec<_>>();
    let mut blocks = replay
        .index
        .iter()
        .filter(|index| index.branch == branch && index.first_hour <= hour)
        .collect::<Vec<_>>();
    blocks.sort_by_key(|index| std::cmp::Reverse(index.first_hour));
    let mut last = BTreeMap::new();
    for index in blocks {
        if daily.iter().all(|table| last.contains_key(table)) {
            break;
        }
        for frame in replay.read_block_selected(index, &[])? {
            if frame.hour > hour {
                break;
            }
            for &table in &daily {
                if daily_read(frame.hour, &frame.metrics, table) {
                    let entry = last.entry(table).or_insert(frame.hour);
                    *entry = (*entry).max(frame.hour);
                }
            }
        }
    }
    Ok(last)
}

/// Tables outside the current recording contract have no acquisition to compare.
pub(super) fn retired_table(table: TableId) -> bool {
    replay::format::FormatVersion::CURRENT
        .table_refusal(table)
        .is_some()
}

pub(super) fn recorded_table_counts(
    frame: &DecodedFrame,
) -> BTreeMap<TableId, Result<usize, String>> {
    TableId::ALL
        .into_iter()
        .map(|table| {
            let count = complete_table(frame, table)
                .map(TableRows::len)
                .map_err(|error| error.to_string());
            (table, count)
        })
        .collect()
}

#[derive(Debug, Default)]
pub(super) struct Report {
    /// Admitted whole-save occurrences, preserving physical order and repeats.
    /// Ordinary country censuses use only their authenticated canonical owners.
    pub(super) country_emissions: Option<Vec<installed_content_save::SavedCountryEmission>>,
    pub(super) canonical_country_tags: Option<BTreeSet<Vec<u8>>>,
    /// Queried at output time, after all content-dependent comparisons.
    pub(super) content_reads: Option<installed_content::content::UncoveredReads>,
    /// Positive game-thread costs identify refresh groups actually walked,
    /// including forced branch/keyframe refreshes outside their usual slot.
    /// A reconstructed snapshot alone is not proof: it may contain cached rows.
    /// CountryIntel's walk is left out: it can move off its slot alone, so it
    /// proves only itself ([`Report::intel_walked`]).
    pub(super) observed_refresh: Vec<replay::cadence::Cadence>,
    /// CountryIntel's own walk was measured at the compared hour.
    pub(super) intel_walked: bool,
    /// A daily table other than CountryIntel was walked off its slot: this
    /// hour was a forced full walk (branch start, load, gap or refresh).
    pub(super) forced_full: bool,
    /// The compared hour.
    pub(super) hour: Hour,
    /// The compared hour is both the loaded and the first sampled hour of an
    /// observed in-game load, never inferred from refresh costs, a session
    /// start or an hour discontinuity.
    pub(super) load_hour: bool,
    /// Per table, the last hour at or before the compared hour whose frame
    /// read it (`last_read_hours`). When set, a daily table not read at the
    /// compared hour is carried, not compared (`attempt`).
    pub(super) read_hours: Option<BTreeMap<TableId, Hour>>,
    /// Comparison groups skipped because their daily tables were carried from
    /// an earlier read, with that read's hour.
    pub(super) carried: BTreeMap<String, Option<Hour>>,
    pub(super) countries: usize,
    pub(super) provinces: usize,
    pub(super) states: usize,
    pub(super) divisions: usize,
    pub(super) war_relations: usize,
    pub(super) factions: usize,
    pub(super) faction_members: usize,
    pub(super) players: usize,
    pub(super) land_combats: usize,
    pub(super) fleets: usize,
    pub(super) task_forces: usize,
    pub(super) ships: usize,
    pub(super) naval_combats: usize,
    pub(super) naval_combat_participants: usize,
    pub(super) air_wings: usize,
    pub(super) army_orders: usize,
    pub(super) scalar_checks: usize,
    pub(super) differences: Vec<String>,
    /// The comparison group each difference belongs to, in lockstep with
    /// `differences`, so every group can print all of its own rows.
    pub(super) difference_owners: Vec<String>,
    pub(super) limitations: BTreeMap<String, Vec<String>>,
    pub(super) excluded_states: BTreeSet<u32>,
    pub(super) membership_skips: BTreeMap<String, usize>,
    pub(super) compared_tables: BTreeMap<String, usize>,
    pub(super) recorded_tables: BTreeMap<TableId, Result<usize, String>>,
    /// A validated pending publication, never an assumed empty table.
    pub(super) deferred_tables: BTreeMap<TableId, String>,
    pub(super) failed_comparisons: BTreeMap<String, String>,
    pub(super) difference_groups: BTreeMap<String, usize>,
    pub(super) active_comparison: Option<String>,
    /// One line per allocation counter: recorded value, saved value and gap.
    pub(super) boundary_witness: Vec<String>,
    /// The explicit verdict on whether the save was taken at the sampled tick.
    pub(super) boundary_verdict: Option<String>,
    /// The compared session's sample boundary, printed with the witness.
    pub(super) sample_boundary: Option<SampleBoundary>,
    /// Each column compared at display precision, with that precision.
    pub(super) display_compared: BTreeMap<String, String>,
}

impl Report {
    pub(super) fn record_country_emissions(&mut self, save: &installed_content_save::Save) {
        self.country_emissions.clone_from(&save.country_emissions);
        self.canonical_country_tags = save.country_emissions.as_ref().map(|emissions| {
            emissions
                .iter()
                .filter(|emission| {
                    matches!(
                        emission.identity.actual,
                        installed_content_save::ActualCountry::Canonical(_)
                    )
                })
                .map(|emission| emission.identity.tag.clone())
                .collect()
        });
    }

    /// Return owned iteration storage so comparisons may mutate the report
    /// while borrowing the save. Without admission, retain the strict full census.
    pub(super) fn canonical_countries<'a, T>(
        &self,
        countries: &'a BTreeMap<Vec<u8>, T>,
    ) -> Vec<(&'a Vec<u8>, &'a T)> {
        countries
            .iter()
            .filter(|(tag, _)| {
                self.canonical_country_tags
                    .as_ref()
                    .is_none_or(|tags| tags.contains(tag.as_slice()))
            })
            .collect()
    }

    pub(super) fn refuse_uncaptured_reserved_operands(&mut self) {
        let tags: BTreeSet<_> = self
            .country_emissions
            .iter()
            .flatten()
            .filter(|emission| emission.identity.actual == installed_content_save::ActualCountry::Reserved)
            .map(|emission| emission.identity.tag.clone())
            .collect();
        for tag in tags {
            self.failed_comparisons.insert(
                format!("reserved country {} uncaptured save operands", String::from_utf8_lossy(&tag)),
                "the admitted reserved country has no canonical country capture; original_tag, positional intel and certified land theatres are compared separately, but its other saved country operands remain unverified".into(),
            );
        }
    }

    pub(super) fn content_warning(&self) -> Option<String> {
        self.content_reads
            .as_ref()
            .and_then(|reads| reads.warning())
    }

    pub(super) fn set_load_hour(&mut self, branch: &replay::branches::BranchStart) {
        self.load_hour = branch.reason == replay::branches::BranchReason::LoadSignal
            && branch.loaded_hour == Some(self.hour)
            && branch.first_hour == self.hour;
    }

    pub(super) fn attempt_table(
        &mut self,
        table: TableId,
        compare: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> bool {
        self.attempt(&format!("{table:?}"), compare)
    }

    pub(super) fn attempt(
        &mut self,
        name: &str,
        compare: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> bool {
        if let Some(last_read) = self.carried_group(name) {
            self.carried.insert(name.into(), last_read);
            return true;
        }
        let previous = self.active_comparison.replace(name.into());
        let result = compare(self);
        self.active_comparison = previous;
        match result {
            Ok(()) => true,
            Err(error) => {
                self.failed_comparisons
                    .insert(name.into(), error.to_string());
                false
            }
        }
    }

    /// At a non-full hour, a group whose tables are all daily and none was
    /// read at this hour compares rows carried from an earlier read against
    /// the save's current values: it is skipped and reported with the last
    /// read hour (`Some` of the latest one among its tables). Hourly, static
    /// and sliced tables, and groups that do not name their tables, are
    /// always compared; sliced comparisons select the owners read at this
    /// hour themselves.
    pub(super) fn carried_group(&self, name: &str) -> Option<Option<Hour>> {
        let read_hours = self.read_hours.as_ref()?;
        if self.full_refresh() {
            return None;
        }
        let tables = group_tables(name);
        let carried = !tables.is_empty()
            && tables.iter().all(|&table| {
                matches!(
                    replay::cadence::cadence(table),
                    replay::cadence::Cadence::Daily(_)
                ) && !self.read_at_hour(table)
            });
        carried.then(|| {
            tables
                .iter()
                .filter_map(|table| read_hours.get(table))
                .max()
                .copied()
        })
    }

    /// Whether a daily table was read at the compared hour. Its slot's walk
    /// is always taken, except CountryIntel's: a held shared buffer moves it
    /// to a later hour, and that retry is recorded as its measured cost
    /// (`sampling::exported_table_ticks`). So CountryIntel is read at an hour
    /// only when its own walk was measured there (or the hour is a full
    /// refresh), whatever the other tables of its slot (IntelNetworks,
    /// LogisticsEquipment) did; they are read on their slot hour or when their
    /// walk was measured.
    pub(super) fn read_at_hour(&self, table: TableId) -> bool {
        if table == TableId::CountryIntel {
            return self.full_refresh() || self.intel_walked;
        }
        self.full_refresh() || self.refresh_is_current(table, self.hour)
    }

    /// A sliced table compared only for the owners read at this hour: the
    /// others are carried, not compared, since their oldest read.
    pub(super) fn carried_owners(&mut self, table: TableId, oldest: Option<Hour>) {
        if self.read_hours.is_some() {
            self.carried
                .insert(format!("{table:?} (owners not read at this hour)"), oldest);
        }
    }

    pub(super) fn unavailable_tables(&self) -> usize {
        self.recorded_tables
            .iter()
            .filter(|(table, rows)| {
                rows.is_err()
                    && !retired_table(**table)
                    && !self.deferred_tables.contains_key(*table)
            })
            .count()
    }

    pub(super) fn outcome(&self) -> io::Result<()> {
        let unavailable = self.unavailable_tables();
        if self.differences.is_empty()
            && unavailable == 0
            && self.failed_comparisons.is_empty()
            && self.excluded_states.is_empty()
        {
            Ok(())
        } else {
            Err(invalid(format!(
                "oracle: {} differences; {} unavailable, truncated or invalid tables; {} failed or skipped comparison groups; {} groups with membership exclusions; comparison {}",
                self.differences.len(),
                unavailable,
                self.failed_comparisons.len(),
                self.membership_skips.len(),
                if unavailable == 0
                    && self.failed_comparisons.is_empty()
                    && self.excluded_states.is_empty()
                {
                    "completed"
                } else {
                    "incomplete"
                },
            )))
        }
    }

    pub(super) fn record_tables(&mut self, frame: &DecodedFrame) {
        self.hour = frame.hour;
        self.recorded_tables = recorded_table_counts(frame);
        self.observed_refresh.clear();
        // Every daily table is walked off its slot only on a forced full hour,
        // except CountryIntel, whose walk moves to the next hour when its
        // shared buffer is held (`daily_intel::Loan::borrow`). The frame has
        // no explicit full-refresh flag, so this is inferred from timings: a
        // full hour whose every off-slot daily walk timed zero (a cancelled
        // job at detach, say) is not recognised: its off-slice rowless
        // owners are then left uncompared, and with read hours supplied its
        // off-slot daily groups are reported carried, rather than compared
        // wrongly.
        self.forced_full = TableId::ALL.into_iter().any(|table| {
            table != TableId::CountryIntel
                && frame.metrics.table_ns[table.index()] != 0
                && matches!(
                    replay::cadence::cadence(table),
                    replay::cadence::Cadence::Daily(_)
                )
                && !replay::cadence::is_refresh(table, frame.hour)
        });
        self.intel_walked = frame.metrics.table_ns[TableId::CountryIntel.index()] != 0;
        let mut field_gaps = BTreeMap::<String, usize>::new();
        for table in TableId::ALL {
            if let replay::frame::TableView::Complete(rows)
            | replay::frame::TableView::Truncated { rows, .. } = &*frame.tables[table.index()]
            {
                for (field, count) in rows.field_gaps() {
                    let name = if field.contains('.') {
                        field.to_owned()
                    } else {
                        format!("{table:?}.{field}")
                    };
                    *field_gaps.entry(name).or_default() += count;
                }
            }
            if table != TableId::CountryIntel
                && frame.metrics.table_ns[table.index()] != 0
                && matches!(
                    replay::cadence::cadence(table),
                    replay::cadence::Cadence::Daily(_) | replay::cadence::Cadence::Static
                )
            {
                self.observed_refresh.push(replay::cadence::cadence(table));
            }
        }
        for (name, count) in field_gaps {
            self.limitation(
                &format!("{name} not recorded"),
                format!("{name}: {count} rows not recorded"),
            );
        }
        for table in [TableId::LexicalTokens, TableId::ScriptVariables] {
            if let TableView::Unavailable { reason } = &*frame.tables[table.index()] {
                self.limitation(
                    "explicitly unavailable source remains unverified and makes the overall comparison incomplete; the recorded reason identifies the missing proof or failed read",
                    format!("{table:?}: {reason}"),
                );
            }
        }
    }

    /// Static tables never go stale; a forced full walk (measured work on any
    /// table of the same cadence family) counts as current.
    pub(super) fn refresh_is_current(&self, table: TableId, hour: Hour) -> bool {
        let cadence = replay::cadence::cadence(table);
        matches!(cadence, replay::cadence::Cadence::Static)
            || cadence.is_refresh(hour)
            || self.observed_refresh.contains(&cadence)
    }

    /// A forced full walk (branch start, load, gap or requested refresh), as
    /// `record_tables` saw it.
    pub(super) fn full_refresh(&self) -> bool {
        self.forced_full
    }

    /// Whether a packet was read at the oracle hour. Current packets were
    /// read with their hourly host row; a historical stamp naming another hour
    /// is a `{label}.sample_hour` difference and the packet is not compared.
    pub(super) fn packet_fresh(
        &mut self,
        label: &str,
        sample_hour: replay::rows::PacketHour,
        hour: i32,
    ) -> bool {
        self.scalar_checks += 1;
        if sample_hour.fresh_at(hour) {
            return true;
        }
        self.difference(format!("{label}.sample_hour"), sample_hour, hour);
        false
    }

    pub(super) fn compared(&mut self, table: &str, rows: usize) {
        *self.compared_tables.entry(table.to_owned()).or_default() += rows;
    }
    pub(super) fn difference(
        &mut self,
        field: impl AsRef<str>,
        replay: impl std::fmt::Debug,
        save: impl std::fmt::Debug,
    ) {
        let group = self
            .active_comparison
            .as_deref()
            .unwrap_or("direct comparisons");
        if let Some(count) = self.difference_groups.get_mut(group) {
            *count += 1;
        } else {
            self.difference_groups.insert(group.into(), 1);
        }
        self.difference_owners.push(group.to_owned());
        self.differences.push(format!(
            "{}: replay {replay:?}, save {save:?}",
            field.as_ref()
        ));
    }

    /// Every difference is printed, grouped by its comparison, with a count
    /// line per group. The former hard hundred-line cap silently dropped whole
    /// groups, including ones whose only difference it hid, which left them
    /// undiagnosable from the report alone. There is no cap now.
    pub(super) fn difference_lines(&self) -> Vec<String> {
        if self.differences.is_empty() {
            return Vec::new();
        }
        let mut grouped = BTreeMap::<&str, Vec<&str>>::new();
        for (group, difference) in self.difference_owners.iter().zip(&self.differences) {
            grouped
                .entry(group.as_str())
                .or_default()
                .push(difference.as_str());
        }
        let mut lines = vec![format!(
            "All {} differences, by group:",
            self.differences.len()
        )];
        for (group, differences) in grouped {
            lines.push(format!("  {group}: {} differences", differences.len()));
            lines.extend(differences.iter().map(|line| format!("    {line}")));
        }
        lines
    }

    pub(super) fn limitation_lines(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once("Limitations:".to_owned()).chain(self.limitations.iter().flat_map(
            |(reason, subjects)| {
                std::iter::once(format!("  {reason}: {} entries", subjects.len()))
                    .chain(subjects.iter().map(|subject| format!("    {subject}")))
            },
        ))
    }

    pub(super) fn recorded<'a, T>(
        &mut self,
        field: &'a replay::rows::Recorded<T>,
        name: &str,
        subject: impl Into<String>,
    ) -> Option<&'a T> {
        match field.require(name) {
            Ok(value) => Some(value),
            Err(reason) => {
                // Whole-frame comparisons already count every absent field in record_tables.
                if self.recorded_tables.is_empty() {
                    self.limitation(&reason, subject);
                }
                None
            }
        }
    }

    pub(super) fn limitation(&mut self, reason: &str, subject: impl Into<String>) {
        self.limitations
            .entry(reason.into())
            .or_default()
            .push(subject.into());
    }

    pub(super) fn membership_skip(
        &mut self,
        group: &str,
        skipped: usize,
        membership: &installed_content_save::PartialMapMembership,
        affected: &BTreeSet<u32>,
    ) {
        self.excluded_states
            .extend(membership.excluded_states.keys());
        if skipped == 0 {
            return;
        }
        self.membership_skips.insert(group.into(), skipped);
        self.limitation(
            &format!("excluded state history leaves membership unknown for {group}; dependent comparisons are skipped"),
            format!(
                "{group}: {skipped} rows with membership-dependent comparisons skipped; {}",
                Self::state_exclusions(membership, affected)
            ),
        );
    }

    fn state_exclusions(
        membership: &installed_content_save::PartialMapMembership,
        affected: &BTreeSet<u32>,
    ) -> String {
        affected
            .iter()
            .map(|id| {
                let excluded = &membership.excluded_states[id];
                format!(
                    "state {id} ({}: {})",
                    excluded.file,
                    excluded.reasons.join("; ")
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub(super) fn membership_content(&mut self, membership: &installed_content_save::PartialMapMembership) {
        self.excluded_states
            .extend(membership.excluded_states.keys());
        if !self.excluded_states.is_empty() {
            self.limitation(
                "content state-history exclusions make membership coverage incomplete",
                format!(
                    "{} excluded states: {}",
                    membership.excluded_states.len(),
                    Self::state_exclusions(membership, &self.excluded_states)
                ),
            );
        }
        for note in &membership.notes {
            self.limitation("state-history reader notes", note.clone());
        }
    }

    /// The saved exact value of `column` as the recording stores it: rounded
    /// to the column's display step when the recording keeps that column at
    /// display precision, which the report then names.
    pub(super) fn displayed(&mut self, column: &str, step: Step, saved: i64) -> i64 {
        if !self.display_compared.contains_key(column) {
            self.display_compared
                .insert(column.to_owned(), step.describe());
        }
        step.round(saved)
    }

    /// [`Self::scalar`] for a column stored at a display step: the save's
    /// exact value is rounded with the same step before comparing.
    pub(super) fn scalar_displayed(
        &mut self,
        tag: &str,
        field: &str,
        captured: Option<i64>,
        saved: Option<&Decimal>,
        column: &str,
        step: Step,
    ) {
        self.scalar_displayed_by(
            tag,
            field,
            captured,
            saved,
            column,
            &step.describe(),
            |raw| step.round(raw),
        );
    }

    /// [`Self::scalar_displayed`] for a column whose precision is no fixed
    /// step: `round` stores the save's exact value as the recording does, and
    /// `precision` names it in the report.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scalar_displayed_by(
        &mut self,
        tag: &str,
        field: &str,
        captured: Option<i64>,
        saved: Option<&Decimal>,
        column: &str,
        precision: &str,
        round: impl FnOnce(i64) -> i64,
    ) {
        match (captured, saved) {
            (Some(captured), Some(saved)) => {
                self.scalar_checks += 1;
                if !self.display_compared.contains_key(column) {
                    self.display_compared
                        .insert(column.to_owned(), precision.to_owned());
                }
                let expected = round(saved.raw);
                if captured != expected {
                    self.difference(format!("{tag}.{field}"), captured, expected);
                }
            }
            _ => self.scalar(tag, field, captured, saved),
        }
    }

    /// The columns compared at display precision and their precision.
    pub(super) fn display_precision_lines(&self) -> Vec<String> {
        if self.display_compared.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![format!(
            "Compared at display precision ({} columns; the save's exact value is rounded with the recorded step):",
            self.display_compared.len()
        )];
        lines.extend(
            self.display_compared
                .iter()
                .map(|(column, precision)| format!("  {column}: {precision}")),
        );
        lines
    }

    pub(super) fn scalar(
        &mut self,
        tag: &str,
        field: &str,
        captured: Option<i64>,
        saved: Option<&Decimal>,
    ) {
        match (captured, saved) {
            (Some(captured), Some(saved)) => {
                self.scalar_checks += 1;
                if captured != saved.raw {
                    self.difference(format!("{tag}.{field}"), captured, saved.raw);
                }
            }
            (None, _) => self.limitation(
                "captured scalar is explicitly unavailable",
                format!("{tag}.{field}"),
            ),
            (_, None) => self.difference(
                format!("{tag}.{field}"),
                captured,
                "missing serialized scalar",
            ),
        }
    }

    /// Carried groups are not failures: at a non-full hour only the tables
    /// read at that hour are compared, and the rest are listed after this line.
    pub(super) fn coverage_line(&self) -> String {
        let mut line = format!(
            "Comparison status for recorded-format coverage: {}; {} table availability issues; {} failed or skipped comparison groups; {} groups with membership exclusions",
            if self.unavailable_tables() == 0
                && self.failed_comparisons.is_empty()
                && self.excluded_states.is_empty()
            {
                "complete"
            } else {
                "incomplete"
            },
            self.unavailable_tables(),
            self.failed_comparisons.len(),
            self.membership_skips.len(),
        );
        if !self.excluded_states.is_empty() {
            line.push_str(&format!(
                "; {} excluded states; {} rows with membership-dependent comparisons skipped",
                self.excluded_states.len(),
                self.membership_skips.values().sum::<usize>()
            ));
        }
        if !self.carried.is_empty() {
            line.push_str(&format!(
                "; {} comparison groups or sliced scopes carried, not compared (not read at hour {})",
                self.carried.len(),
                self.hour
            ));
        }
        line
    }

    pub(super) fn format_line(format: replay::format::FormatVersion) -> String {
        let tables = TableId::ALL
            .into_iter()
            .filter(|&table| !retired_table(table))
            .count();
        format!(
            "Replay format {}: comparison checks {} table types",
            format.version(),
            tables
        )
    }

    pub(super) fn print(&self, format: replay::format::FormatVersion) {
        if let Some(warning) = self.content_warning() {
            println!("Warning: {warning}");
        }
        println!("{}", Self::format_line(format));
        println!(
            "Compared: {} provinces, {} states, {} divisions, {} war relations, {} factions, {} faction members, {} countries, {} exact field comparisons",
            self.provinces,
            self.states,
            self.divisions,
            self.war_relations,
            self.factions,
            self.faction_members,
            self.countries,
            self.scalar_checks
        );
        println!(
            "Map tables: {} uniquely matched players, {} land combats, {} fleets, {} task forces, {} ships",
            self.players, self.land_combats, self.fleets, self.task_forces, self.ships
        );
        println!(
            "Map tables: {} naval combats, {} naval combat participants, {} air wings, {} army orders",
            self.naval_combats, self.naval_combat_participants, self.air_wings, self.army_orders
        );
        println!("Differences: {}", self.differences.len());
        for (group, count) in &self.difference_groups {
            println!("  Differences {group}: {count}");
        }
        println!("{}", self.coverage_line());
        for (group, last_read) in &self.carried {
            match last_read {
                Some(hour) => println!("  {group}: carried, not compared (last read hour {hour})"),
                None => println!("  {group}: carried, not compared (read hour unknown)"),
            }
        }
        for (table, rows) in &self.compared_tables {
            println!("Compared {table}: {rows}");
        }
        for line in self.display_precision_lines() {
            println!("{line}");
        }
        if !self.recorded_tables.is_empty() {
            println!(
                "Recorded table coverage (row counts alone do not establish save comparison):"
            );
            for (table, rows) in &self.recorded_tables {
                match rows {
                    Ok(rows) => println!("  {table:?}: {rows} rows"),
                    Err(reason) if retired_table(*table) => {
                        println!("  {table:?}: retired, not compared ({reason})")
                    }
                    Err(reason) => match self.deferred_tables.get(table) {
                        Some(deferred) => {
                            println!("  {table:?}: publication deferred ({deferred})")
                        }
                        None => println!("  {table:?}: {reason}"),
                    },
                }
            }
        }
        for (group, reason) in &self.failed_comparisons {
            println!("  Comparison {group} incomplete: {reason}");
        }
        for line in self.difference_lines() {
            println!("{line}");
        }
        match self.sample_boundary {
            Some(boundary) => {
                println!("Boundary witness (engine allocation counters at the {boundary} sample):")
            }
            None => println!("Boundary witness (engine allocation counters at the sampled tick):"),
        }
        match &self.boundary_verdict {
            Some(verdict) => println!("  {verdict}"),
            None => println!(
                "  not evaluated; the counter comparison itself failed and is listed above"
            ),
        }
        for line in &self.boundary_witness {
            println!("  {line}");
        }
        for line in self.limitation_lines() {
            println!("{line}");
        }
    }
}
