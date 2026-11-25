use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::Decimal;
use replay::campaign::Replay;
use replay_data::{
    BranchId, Hour, display::Step, identity::SampleBoundary, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

use super::{complete_table, invalid};

fn contained_refusal(reason: &str) -> bool {
    reason.contains("spool validation failed: ") || reason.contains("hook family ")
}

pub fn group_tables(name: &str) -> Vec<TableId> {
    let last = name.rsplit(' ').next().unwrap_or(name);
    match last {
        "IntelSourcePools" => vec![TableId::IntelSources, TableId::IntelContributions],
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

pub fn daily_read(metrics: &replay_frame::frame::Metrics, table: TableId) -> bool {
    metrics.table_ns[table.index()] != 0
}

pub fn last_read_hours(
    replay: &Replay,
    branch: BranchId,
    hour: Hour,
) -> io::Result<BTreeMap<TableId, Hour>> {
    let daily = TableId::ALL
        .into_iter()
        .filter(|&table| {
            matches!(
                replay_data::cadence::cadence(table),
                replay_data::cadence::Cadence::Daily(_)
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
        replay.read_block_alone(
            index,
            &daily,
            |_, frame| {
                if frame.hour > hour {
                    return Ok(());
                }
                for &table in &daily {
                    if daily_read(&frame.metrics, table) {
                        let entry = last.entry(table).or_insert(frame.hour);
                        *entry = (*entry).max(frame.hour);
                    }
                }
                Ok(())
            },
            || Ok(()),
        )?;
    }
    Ok(last)
}

pub fn retired_table(table: TableId) -> bool {
    replay_data::format::FormatVersion::CURRENT
        .table_refusal(table)
        .is_some()
}

pub fn recorded_table_counts(frame: &DecodedFrame) -> BTreeMap<TableId, Result<usize, String>> {
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

pub fn is_load_hour(branch: &replay_frame::branches::BranchStart, hour: Hour) -> bool {
    branch.reason == replay_frame::branches::BranchReason::LoadSignal
        && branch.loaded_hour == Some(hour)
        && branch.first_hour == hour
}

#[derive(Debug, Default)]
pub struct Report {
    pub country_emissions: Option<Vec<content_reader_save::SavedCountryEmission>>,
    pub canonical_country_tags: Option<BTreeSet<Vec<u8>>>,
    pub content_reads: Option<content_reader_layers::content::UncoveredReads>,
    pub observed_refresh: Vec<replay_data::cadence::Cadence>,
    pub intel_walked: bool,
    pub forced_full: bool,
    pub hour: Hour,
    pub load_hour: bool,
    pub read_hours: Option<BTreeMap<TableId, Hour>>,
    pub carried: BTreeMap<String, Option<Hour>>,
    pub countries: usize,
    pub provinces: usize,
    pub states: usize,
    pub divisions: usize,
    pub war_relations: usize,
    pub factions: usize,
    pub faction_members: usize,
    pub players: usize,
    pub land_combats: usize,
    pub fleets: usize,
    pub task_forces: usize,
    pub ships: usize,
    pub naval_combats: usize,
    pub naval_combat_participants: usize,
    pub air_wings: usize,
    pub army_orders: usize,
    pub scalar_checks: usize,
    pub differences: Vec<String>,
    pub difference_owners: Vec<String>,
    pub limitations: BTreeMap<String, Vec<String>>,
    pub excluded_states: BTreeSet<u32>,
    pub membership_skips: BTreeMap<String, usize>,
    pub compared_tables: BTreeMap<String, usize>,
    pub recorded_tables: BTreeMap<TableId, Result<usize, String>>,
    pub not_recorded: BTreeSet<TableId>,
    pub deferred_tables: BTreeMap<TableId, String>,
    pub failed_comparisons: BTreeMap<String, String>,
    pub difference_groups: BTreeMap<String, usize>,
    pub active_comparison: Option<String>,
    pub boundary_witness: Vec<String>,
    pub boundary_verdict: Option<String>,
    pub sample_boundary: Option<SampleBoundary>,
    pub display_compared: BTreeMap<String, String>,
}

impl Report {
    pub fn record_country_emissions(&mut self, save: &content_reader_save::Save) {
        self.country_emissions.clone_from(&save.country_emissions);
        self.canonical_country_tags = save.country_emissions.as_ref().map(|emissions| {
            emissions
                .iter()
                .filter(|emission| {
                    matches!(
                        emission.identity.actual,
                        content_reader_save::ActualCountry::Canonical(_)
                    )
                })
                .map(|emission| emission.identity.tag.clone())
                .collect()
        });
    }

    pub fn canonical_countries<'a, T>(
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

    pub fn refuse_uncaptured_reserved_operands(&mut self) {
        let tags: BTreeSet<_> = self
            .country_emissions
            .iter()
            .flatten()
            .filter(|emission| emission.identity.actual == content_reader_save::ActualCountry::Reserved)
            .map(|emission| emission.identity.tag.clone())
            .collect();
        for tag in tags {
            self.failed_comparisons.insert(
                format!("reserved country {} uncaptured save operands", String::from_utf8_lossy(&tag)),
                "the admitted reserved country has no canonical country capture; original_tag, positional intel and certified land theatres are compared separately, but its other saved country operands remain unverified".into(),
            );
        }
    }

    pub fn content_warning(&self) -> Option<String> {
        self.content_reads
            .as_ref()
            .and_then(|reads| reads.warning())
    }

    pub fn set_load_hour(&mut self, branch: &replay_frame::branches::BranchStart) {
        self.load_hour = is_load_hour(branch, self.hour);
    }

    pub fn attempt_table(
        &mut self,
        table: TableId,
        compare: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> bool {
        self.attempt(&format!("{table:?}"), compare)
    }

    pub fn attempt(
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
                self.incomplete(name, error.to_string());
                false
            }
        }
    }

    pub fn incomplete(&mut self, group: impl Into<String>, reason: impl Into<String>) {
        let reason = reason.into();
        match self.failed_comparisons.entry(group.into()) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.push_str("; ");
                existing.push_str(&reason);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(reason);
            }
        }
    }

    pub fn carried_group(&self, name: &str) -> Option<Option<Hour>> {
        let read_hours = self.read_hours.as_ref()?;
        if self.full_refresh() {
            return None;
        }
        let tables = group_tables(name);
        let carried = !tables.is_empty()
            && tables.iter().all(|&table| {
                matches!(
                    replay_data::cadence::cadence(table),
                    replay_data::cadence::Cadence::Daily(_)
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

    pub fn read_at_hour(&self, table: TableId) -> bool {
        if table == TableId::CountryIntel {
            return self.full_refresh() || self.intel_walked;
        }
        self.full_refresh() || self.refresh_is_current(table, self.hour)
    }

    pub fn carried_owners(&mut self, table: TableId, oldest: Option<Hour>) {
        if self.read_hours.is_some() {
            self.carried
                .insert(format!("{table:?} (owners not read at this hour)"), oldest);
        }
    }

    pub fn unavailable_tables(&self) -> usize {
        self.recorded_tables
            .iter()
            .filter(|(table, rows)| {
                rows.is_err()
                    && !retired_table(**table)
                    && !self.not_recorded.contains(*table)
                    && (!self.deferred_tables.contains_key(*table)
                        || rows
                            .as_ref()
                            .err()
                            .is_some_and(|reason| contained_refusal(reason)))
            })
            .count()
    }

    pub fn outcome(&self) -> io::Result<()> {
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

    pub fn record_tables(&mut self, frame: &DecodedFrame) {
        self.hour = frame.hour;
        self.recorded_tables = recorded_table_counts(frame);
        self.not_recorded = TableId::ALL
            .into_iter()
            .filter(|table| !frame.recorded(*table))
            .collect();
        self.observed_refresh.clear();
        self.forced_full = TableId::ALL.into_iter().any(|table| {
            table != TableId::CountryIntel
                && frame.metrics.table_ns[table.index()] != 0
                && matches!(
                    replay_data::cadence::cadence(table),
                    replay_data::cadence::Cadence::Daily(_)
                )
                && !replay_data::cadence::is_refresh(table, frame.hour)
        });
        self.intel_walked = frame.metrics.table_ns[TableId::CountryIntel.index()] != 0;
        let mut field_gaps = BTreeMap::<String, usize>::new();
        for table in TableId::ALL {
            if let replay_frame::frame::TableView::Complete(rows)
            | replay_frame::frame::TableView::Truncated { rows, .. } =
                &*frame.tables[table.index()]
            {
                let (fields, nested) = rows.field_gaps_by_level();
                for (field, count) in fields {
                    *field_gaps.entry(format!("{table:?}.{field}")).or_default() += count;
                }
                for (field, count) in nested {
                    *field_gaps.entry(field.to_owned()).or_default() += count;
                }
            }
            if table != TableId::CountryIntel
                && frame.metrics.table_ns[table.index()] != 0
                && matches!(
                    replay_data::cadence::cadence(table),
                    replay_data::cadence::Cadence::Daily(_) | replay_data::cadence::Cadence::Static
                )
            {
                self.observed_refresh
                    .push(replay_data::cadence::cadence(table));
            }
        }
        for (name, count) in field_gaps {
            self.limitation(
                &format!("{name} not recorded"),
                format!("{name}: {count} rows not recorded"),
            );
        }
        for table in [
            TableId::LexicalTokens,
            TableId::ScriptVariables,
            TableId::ScriptVariableNames,
            TableId::ScriptVariableValues,
        ] {
            if let TableView::Unavailable { reason } = &*frame.tables[table.index()] {
                self.limitation(
                    "explicitly unavailable source remains unverified and makes the overall comparison incomplete; the recorded reason identifies the missing proof or failed read",
                    format!("{table:?}: {reason}"),
                );
            }
        }
    }

    pub fn refresh_is_current(&self, table: TableId, hour: Hour) -> bool {
        let cadence = replay_data::cadence::cadence(table);
        matches!(cadence, replay_data::cadence::Cadence::Static)
            || cadence.is_refresh(hour)
            || self.observed_refresh.contains(&cadence)
    }

    pub fn full_refresh(&self) -> bool {
        self.forced_full
    }

    pub fn packet_fresh(
        &mut self,
        label: &str,
        sample_hour: replay_data::rows::PacketHour,
        hour: i32,
    ) -> bool {
        self.scalar_checks += 1;
        if sample_hour.fresh_at(hour) {
            return true;
        }
        self.difference(format!("{label}.sample_hour"), sample_hour, hour);
        false
    }

    pub fn compared(&mut self, table: &str, rows: usize) {
        *self.compared_tables.entry(table.to_owned()).or_default() += rows;
    }
    pub fn difference(
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

    pub fn difference_lines(&self) -> Vec<String> {
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

    pub fn limitation_lines(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once("Limitations:".to_owned()).chain(self.limitations.iter().flat_map(
            |(reason, subjects)| {
                std::iter::once(format!("  {reason}: {} entries", subjects.len()))
                    .chain(subjects.iter().map(|subject| format!("    {subject}")))
            },
        ))
    }

    pub fn recorded<'a, T>(
        &mut self,
        field: &'a replay_data::rows::Recorded<T>,
        name: &str,
        subject: impl Into<String>,
    ) -> Option<&'a T> {
        match field.require(name) {
            Ok(value) => Some(value),
            Err(reason) => {
                if self.recorded_tables.is_empty() {
                    self.limitation(&reason, subject);
                }
                None
            }
        }
    }

    pub fn limitation(&mut self, reason: &str, subject: impl Into<String>) {
        self.limitations
            .entry(reason.into())
            .or_default()
            .push(subject.into());
    }

    pub fn membership_skip(
        &mut self,
        group: &str,
        skipped: usize,
        membership: &content_reader_save::PartialMapMembership,
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
        membership: &content_reader_save::PartialMapMembership,
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

    pub fn membership_content(&mut self, membership: &content_reader_save::PartialMapMembership) {
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

    pub fn displayed(&mut self, column: &str, step: Step, saved: i64) -> i64 {
        if !self.display_compared.contains_key(column) {
            self.display_compared
                .insert(column.to_owned(), step.describe());
        }
        step.round(saved)
    }

    pub fn scalar_displayed(
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

    pub fn scalar_displayed_by(
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

    pub fn display_precision_lines(&self) -> Vec<String> {
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

    pub fn scalar(
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

    pub fn coverage_line(&self) -> String {
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

    pub fn format_line(format: replay_data::format::FormatVersion) -> String {
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

    pub fn print(&self, format: replay_data::format::FormatVersion) {
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
                        Some(deferred) if !contained_refusal(reason) => {
                            println!("  {table:?}: publication deferred ({deferred})")
                        }
                        _ => println!("  {table:?}: {reason}"),
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
                println!("Boundary witness (engine allocation counters, {boundary} session, at the compared frame taken as settled by a refresh):")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_incomplete_reason_for_one_group_survives() {
        let group = "EventFeedHistory delayed_events";
        let mut report = Report::default();
        report.incomplete(group, "first reason");
        assert!(!report.attempt(group, |_| Err(io::Error::other("second reason"))));
        report.incomplete(group, "third reason");
        assert_eq!(
            report.failed_comparisons[group],
            "first reason; second reason; third reason"
        );
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(report.outcome().is_err());
    }
}
