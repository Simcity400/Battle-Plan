mod binary;
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(test)]
mod tests;

use super::input::{HourInput, ReadEvidence, World, observe_design};
use super::private::Facts;
use super::*;
use crate::prepared::pages::Pages;

#[derive(Clone, Default)]
pub struct CountryState {
    result: ScannedLines,
    pending_bytes: usize,
    last: Option<((u32, u32), i32)>,
    open: BTreeMap<u64, Open>,
    carried: bool,
    broken: BTreeMap<u64, Open>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    appearances: BTreeMap<u64, u32>,
    latest: BTreeMap<u64, usize>,
    history: private::Root,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    last_output_due: Option<i32>,
    last_modifier_country: Option<Arc<replay_data::rows::CountryProductionModifiersRow>>,
    last_design_revision: Option<u64>,
}

impl CountryState {
    fn compact(&mut self, facts: &mut dyn private::Facts) -> io::Result<()> {
        self.pending_bytes = 0;
        let active: BTreeSet<_> = self
            .open
            .values()
            .chain(self.broken.values())
            .map(|line| line.index)
            .collect();
        for (&handle, &index) in &self.latest {
            if !active.contains(&index) {
                let mut line = self.result.lines[index].clone();
                line.runs.clear();
                line.output.clear();
                private::put(
                    facts,
                    &mut self.history,
                    &handle.to_string(),
                    &ClosedLine {
                        appearance: self.appearances.get(&handle).copied().unwrap_or_else(|| {
                            line.key.rsplit('#').next().unwrap().parse().unwrap()
                        }),
                        line,
                    },
                )?;
            }
        }
        let mut remap = BTreeMap::new();
        let mut lines = Vec::new();
        let mut modifier_keys = BTreeSet::new();
        for (index, mut line) in std::mem::take(&mut self.result.lines)
            .into_iter()
            .enumerate()
        {
            if active.contains(&index) {
                remap.insert(index, lines.len());
                pages::tail(&mut line.runs);
                pages::tail(&mut line.output);
                modifier_keys.insert(line.key.clone());
                lines.push(line);
            }
        }
        self.result.lines = lines;
        for line in self.open.values_mut().chain(self.broken.values_mut()) {
            line.index = remap[&line.index];
        }
        self.latest.retain(|_, index| {
            if let Some(&new) = remap.get(index) {
                *index = new;
                true
            } else {
                false
            }
        });
        self.appearances
            .retain(|handle, _| self.latest.contains_key(handle));
        self.result
            .modifiers
            .retain(|key, _| modifier_keys.contains(key));
        for runs in self.result.modifiers.values_mut() {
            pages::tail(runs);
        }
        let variants = &self.active_variants;
        let mut organisations: BTreeSet<_> = self
            .result
            .lines
            .iter()
            .flat_map(|line| line.runs.iter().map(|run| run.mio))
            .collect();
        organisations.extend(
            self.result
                .designs
                .iter()
                .filter(|(id, _)| variants.contains(id))
                .filter_map(|(_, reads)| reads.last().and_then(|read| read.author)),
        );
        for (&id, runs) in &self.result.designs {
            if !variants.contains(&id)
                && let Some(last) = runs.last()
            {
                private::put(facts, &mut self.history, &format!("design-{id}"), last)?;
            }
        }
        self.result.designs.retain(|id, _| variants.contains(id));
        for (&id, runs) in &self.result.icons {
            if !variants.contains(&id)
                && let Some(last) = runs.last()
            {
                private::put(facts, &mut self.history, &format!("icon-{id}"), last)?;
            }
        }
        self.result.icons.retain(|id, _| variants.contains(id));
        for (&id, runs) in &self.result.organisations {
            if !organisations.contains(&id)
                && let Some(last) = runs.last()
            {
                private::put(
                    facts,
                    &mut self.history,
                    &format!("organisation-{id}"),
                    last,
                )?;
            }
        }
        self.result
            .organisations
            .retain(|id, _| organisations.contains(id));
        for runs in self.result.designs.values_mut() {
            pages::tail(runs);
        }
        for runs in self.result.icons.values_mut() {
            pages::tail(runs);
        }
        for runs in self.result.organisations.values_mut() {
            pages::tail(runs);
        }
        self.result.reads = Vec::new();
        pages::tail(&mut self.result.line_hours);
        pages::tail(&mut self.result.output_hours);
        pages::tail(&mut self.result.organisation_hours);
        pages::tail(&mut self.result.unread_output);
        // The next page encodes the kept versions again, and each shared or cached row once.
        let mut rows = BTreeSet::new();
        let kept = self
            .result
            .modifiers
            .values()
            .flatten()
            .map(|run| &run.country);
        for row in kept.chain(&self.last_modifier_country) {
            if rows.insert(Arc::as_ptr(row)) {
                self.pending_bytes += modifier_country_bytes(row);
            }
        }
        self.pending_bytes += self.result.designs.values().map(Vec::len).sum::<usize>()
            * std::mem::size_of::<DesignRead>();
        for icon in self.result.icons.values().flatten() {
            self.pending_bytes += pages::record_bytes(icon);
        }
        for organisation in self.result.organisations.values().flatten() {
            self.pending_bytes += pages::record_bytes(organisation);
        }
        Ok(())
    }

    fn lost(&mut self, first: i32, last: i32) {
        self.result.unreadable_stretches += 1;
        mark_span(&mut self.result.unread_output, first.min(last), last);
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Carry {
    first: i32,
    last: i32,
    modifiers: bool,
    output: bool,
}
impl CountryState {
    fn materialize(&mut self) {
        if let Some(carry) = self.carry.take() {
            carry_lines(
                &mut self.result,
                &self.open,
                (carry.first, carry.last),
                carry.modifiers,
                carry.output,
            );
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ClosedLine {
    appearance: u32,
    line: ScannedLine,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NativeCountry {
    pub slot: u32,
    pub id: u32,
    pub epoch: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Alias {
    tag: String,
    first_hour: i32,
    last_hour: i32,
    country: NativeCountry,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Directory {
    pub branch: u64,
    pub coverage: crate::prepared::coverage::Coverage,
    pub last_block: Option<(i32, i32, u64)>,
    pub pictured: bool,
    pub visual: Option<String>,
    pub executable: bool,
    pub pages: u64,
    index: private::Root,
    variants: pages::PageLog,
    aliases: Vec<Alias>,
}

pub fn directory_section(branch: u64) -> String {
    format!("production-lines-directory-v1-{branch}")
}
pub fn coverage_section(branch: u64) -> String {
    format!("preparation-family-production-lines-{branch}")
}
fn alias_section(branch: u64) -> String {
    format!("production-lines-aliases-v2-{branch}")
}
fn variant_section(branch: u64) -> String {
    format!("production-lines-variants-v2-{branch}")
}
fn tag_key(tag: &str) -> String {
    format!(
        "tag-{}",
        tag.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn native_key(country: &NativeCountry) -> String {
    format!("native-{}-{}-{}", country.slot, country.id, country.epoch)
}

#[derive(Clone)]
struct PagedCountry {
    state: CountryState,
    rows: pages::CountryRows,
    head: pages::PageLog,
    sleep: Option<Dormant>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Dormant {
    hour: i32,
    head: pages::PageLog,
    reads: usize,
    stretches: usize,
}
impl Default for PagedCountry {
    fn default() -> Self {
        Self {
            state: CountryState::new(),
            rows: Default::default(),
            head: pages::PageLog::default(),
            sleep: None,
        }
    }
}

#[derive(Clone)]
enum TagCountry {
    Absent,
    Native(NativeCountry),
    Projected(Box<PagedCountry>),
}

impl TagCountry {
    fn projection(&self) -> Option<&PagedCountry> {
        match self {
            Self::Projected(country) => Some(country),
            _ => None,
        }
    }

    fn projection_mut(&mut self) -> Option<&mut PagedCountry> {
        match self {
            Self::Projected(country) => Some(country),
            _ => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ActiveCountry {
    native: NativeCountry,
}

pub struct Prepared {
    world: World,
    absent: PagedCountry,
    native: BTreeMap<NativeCountry, PagedCountry>,
    aliases: BTreeMap<String, TagCountry>,
    active: BTreeMap<(u32, u32), ActiveCountry>,
    initial_tags: BTreeSet<String>,
    quiet: Option<(i32, i32)>,
    last_input: Option<HourInput>,
    evidence_ready: bool,
    evidence: Vec<ReadEvidence>,
    evidence_bytes: usize,
    evidence_base_bytes: usize,
    evidence_design_bytes: usize,
    evidence_organisation_bytes: usize,
    evidence_organisation_hour: Option<i32>,
    directory: Directory,
    alias_last: BTreeMap<String, usize>,
    processed: Option<BlockIndex>,
    block_failed: bool,
}

impl Prepared {
    pub fn advance_block<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        position: usize,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        crate::metadata::check_cancel(cancelled)?;
        let next = self.next_block(replay)?;
        let expected = replay
            .index
            .iter()
            .enumerate()
            .skip(next)
            .find(|(_, block)| block.branch == self.directory.branch);
        let Some((expected, block)) = expected else {
            return Err(crate::invalid("Production cursor is past the branch tail"));
        };
        if position != expected {
            return Err(crate::invalid(
                "Production cursor skips an unprocessed block",
            ));
        }
        let tables = Self::tables(icons.is_some());
        self.block_failed = true;
        let mut observer_failed = false;
        let result = replay.read_block_recorded_alone(
            block,
            &tables,
            |updates, frame| {
                let changes = self
                    .world
                    .recorded_changes(replay, frame, (*updates).clone());
                if let Err(error) =
                    self.observe_changed_into(writer, replay, frame, icons, cancelled, &changes)
                {
                    observer_failed = true;
                    return Err(error);
                }
                Ok(())
            },
            || crate::metadata::check_cancel(cancelled),
        );
        match result {
            Ok(()) => {}
            Err(error) if observer_failed || error.kind() == io::ErrorKind::Interrupted => {
                return Err(error);
            }
            Err(error) => {
                let first = replay::container::ReplayStream::failed_hour(&error)
                    .unwrap_or(block.first_hour);
                self.drain_quiet(writer)?;
                self.lost(first, block.last_hour);
            }
        }
        self.processed = Some(block.clone());
        self.block_failed = false;
        Ok(())
    }

    pub fn new(
        replay: &Replay,
        branch: u64,
        tags: impl IntoIterator<Item = String>,
        icons: Option<&IconContent>,
    ) -> Self {
        let first_hour = branch_blocks(replay, branch)
            .first()
            .map_or(0, |block| block.first_hour);
        let tags: BTreeSet<_> = tags.into_iter().collect();
        Self {
            alias_last: BTreeMap::new(),
            processed: None,
            block_failed: false,
            world: World::default(),
            absent: PagedCountry::default(),
            native: BTreeMap::new(),
            aliases: tags
                .iter()
                .map(|tag| (tag.clone(), TagCountry::Absent))
                .collect(),
            active: BTreeMap::new(),
            initial_tags: tags.clone(),
            quiet: None,
            last_input: None,
            evidence_ready: false,
            evidence: Vec::new(),
            evidence_bytes: 0,
            evidence_base_bytes: 0,
            evidence_design_bytes: 0,
            evidence_organisation_bytes: 0,
            evidence_organisation_hour: None,
            directory: Directory {
                branch,
                coverage: crate::prepared::coverage::Coverage {
                    schema: crate::prepared::coverage::SCHEMA,
                    first_hour,
                    last_hour: first_hour,
                    complete: false,
                },
                last_block: None,
                pictured: icons.is_some(),
                visual: icons.map(|icons| icons.visual_inputs().to_owned()),
                executable: engine_executable::executable_tables::is_initialized(),
                pages: 0,
                index: private::Root::default(),
                variants: pages::PageLog::default(),
                aliases: Vec::new(),
            },
        }
    }

    pub fn tables(pictured: bool) -> Vec<TableId> {
        let mut tables = vec![
            TableId::Campaign,
            TableId::ProductionLines,
            TableId::ProductionLineOutput,
            TableId::IndustrialOrganisations,
            TableId::IndustrialOrganisationDetails,
            TableId::IndustrialOrganisationPolicies,
            TableId::VariantStats,
            TableId::CountryProductionModifiers,
            TableId::ProductionLineModifiers,
            TableId::ModifierDefinitions,
        ];
        tables.extend(VariantCatalogue::TABLES);
        if pictured {
            tables.extend([
                TableId::PoliticalParties,
                TableId::Politics,
                TableId::Factions,
                TableId::FactionMembers,
                TableId::States,
            ]);
        }
        tables
    }

    #[cfg(test)]
    fn observe<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
    ) -> io::Result<bool> {
        let changes = self.world.changes(replay, frame);
        self.observe_changed(writer, replay, frame, icons, &changes)
    }
    fn observe_changed<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<bool> {
        if self
            .quiet
            .is_some_and(|(_, last)| last.checked_add(1) != Some(frame.hour))
        {
            self.drain_quiet(writer)?;
        }
        if self
            .world
            .quiet(replay, frame, changes, self.evidence_ready)
        {
            if self.evidence_ready {
                let mut evidence = self.world.evidence(replay, frame);
                let input = self.last_input.as_ref().expect("unchanged census operands");
                let previous = self.evidence.last();
                if evidence.lines_read
                    || evidence.output_read
                    || evidence.design_revision
                        != previous.map_or(input.design_revision, |read| read.design_revision)
                    || evidence.organisation_read_hour
                        != previous.map_or(input.organisation_read_hour, |read| {
                            read.organisation_read_hour
                        })
                {
                    let design_changed = evidence.design_revision
                        != previous.map_or(input.design_revision, |read| read.design_revision);
                    evidence.organisation_changed = evidence.lines_read
                        && input.organisations_named
                        && evidence.organisation_read_hour != self.evidence_organisation_hour;
                    let bytes = std::mem::size_of::<ReadEvidence>()
                        + usize::from(evidence.lines_read)
                            * (self.native.len() + self.aliases.len() + 1)
                            * (std::mem::size_of::<i32>()
                                + std::mem::size_of::<crate::products::country_groups::HourSpan>())
                        + usize::from(design_changed) * self.evidence_design_bytes
                        + usize::from(evidence.organisation_changed)
                            * self.evidence_organisation_bytes;
                    if crate::prepared::pages::publication_due(true, false, false, || {
                        (
                            self.evidence_base_bytes
                                .saturating_add(self.evidence_bytes)
                                .saturating_add(bytes),
                            0,
                        )
                    }) {
                        self.flush(writer, &AtomicBool::new(false))?;
                    }
                    self.evidence
                        .try_reserve_exact(1)
                        .map_err(io::Error::other)?;
                    self.evidence_bytes = self.evidence_bytes.saturating_add(bytes);
                    if evidence.lines_read {
                        self.evidence_organisation_hour = evidence.organisation_read_hour;
                    }
                    self.evidence.push(evidence);
                }
            }
            let first = self.quiet.map_or(frame.hour, |span| span.0);
            self.quiet = Some((first, frame.hour));
            return Ok(false);
        }
        if !self.evidence.is_empty() {
            self.drain_quiet(writer)?;
        }
        self.evidence_ready = false;
        let previous = self.last_input.as_ref().map(|input| {
            (
                input.lines_complete,
                input.output_complete,
                input.organisations_complete,
                input.modifiers_complete,
                input.design_revision,
            )
        });
        if self.world.identity_changed(frame, changes) {
            self.drain_quiet(writer)?;
        }
        let mut input = self.world.extract_cached(
            replay,
            frame,
            icons,
            &BTreeSet::new(),
            self.last_input.take(),
            changes,
        )?;
        let signature = (
            input.lines_complete,
            input.output_complete,
            input.organisations_complete,
            input.modifiers_complete,
            input.design_revision,
        );
        if previous == Some(signature)
            && !input.identities_changed
            && !input.lines_read
            && !input.output_read
        {
            let first = self.quiet.map_or(input.hour, |span| span.0);
            self.quiet = Some((first, input.hour));
            self.last_input = Some(input);
            return Ok(false);
        }
        // Drain the earlier frontier before applying this hour's semantic edge.
        if let Some((first, last)) = self.quiet.take() {
            let saved = (
                input.hour,
                input.last_hour,
                input.lines_read,
                input.output_read,
                input.modifiers_read,
                input.identities_changed,
            );
            let availability = (
                input.lines_complete,
                input.output_complete,
                input.organisations_complete,
                input.modifiers_complete,
                input.design_revision,
            );
            input.hour = first;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            input.identities_changed = false;
            if let Some(previous) = previous {
                input.lines_complete = previous.0;
                input.output_complete = previous.1;
                input.organisations_complete = previous.2;
                input.modifiers_complete = previous.3;
                input.design_revision = previous.4;
            }
            self.observe_input(writer, &input)?;
            input.hour = saved.0;
            input.last_hour = saved.1;
            input.lines_read = saved.2;
            input.output_read = saved.3;
            input.modifiers_read = saved.4;
            input.identities_changed = saved.5;
            input.lines_complete = availability.0;
            input.output_complete = availability.1;
            input.organisations_complete = availability.2;
            input.modifiers_complete = availability.3;
            input.design_revision = availability.4;
        }
        self.observe_input(writer, &input)?;
        // These censuses add dates and names, but no per-line output or modifier samples.
        self.evidence_ready = input.lines_read
            && input.lines_complete
            && input.rows_complete
            && input.countries.is_some()
            && input.unlinked_lines.is_empty()
            && input.output.is_empty()
            && input.unlinked_output.is_empty()
            && input.modifier_lines.is_empty()
            && (input.modifiers_read || !input.modifiers_complete)
            && input
                .lines
                .values()
                .flatten()
                .all(|row| LineKind::of(row.kind).is_some());
        if self.evidence_ready {
            self.evidence_base_bytes = self.pending_total();
            self.evidence_design_bytes = self
                .native
                .values()
                .chain(self.aliases.values().filter_map(TagCountry::projection))
                .flat_map(|country| &country.state.active_variants)
                .map(|id| input.design_bytes(*id))
                .sum();
            self.evidence_organisation_bytes = self
                .native
                .keys()
                .map(|key| key.slot)
                .chain(
                    self.aliases
                        .iter()
                        .filter(|(_, alias)| alias.projection().is_some())
                        .filter_map(|(tag, _)| {
                            input
                                .countries
                                .as_ref()?
                                .get(tag)?
                                .found()
                                .map(|(slot, _)| slot)
                        }),
                )
                .map(|slot| {
                    let rows = input
                        .lines
                        .get(&slot)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let named: BTreeSet<_> = rows
                        .iter()
                        .flat_map(|row| {
                            std::iter::once(row.mio)
                                .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                        })
                        .collect();
                    named
                        .iter()
                        .filter_map(|id| input.organisations.get(id))
                        .map(pages::record_bytes)
                        .sum::<usize>()
                })
                .sum();
            self.evidence_organisation_hour = input.organisation_read_hour;
        }
        self.last_input = Some(input);
        Ok(true)
    }

    pub fn observe_into<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = self.world.changes(replay, frame);
        self.observe_changed_into(writer, replay, frame, icons, cancelled, &changes)
    }
    pub fn observe_changed_into<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        frame: &DecodedFrame,
        icons: Option<&IconContent>,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        if !self.observe_changed(writer, replay, frame, icons, changes)? {
            if crate::prepared::pages::publication_due(true, false, false, || {
                (self.world.pending_bytes + self.evidence_bytes, 0)
            }) {
                self.flush(writer, cancelled)?;
            }
            return Ok(());
        }
        let bytes = self.pending_total();
        if crate::prepared::pages::publication_due(true, false, false, || (bytes, 0)) {
            self.flush(writer, cancelled)?;
        }
        Ok(())
    }

    /// The bytes the next page holds: every country's pending records and the kept aliases.
    fn pending_total(&self) -> usize {
        self.world.pending_bytes
            + self.absent.state.pending_bytes
            + self
                .native
                .values()
                .map(|country| country.state.pending_bytes)
                .sum::<usize>()
            + self
                .aliases
                .values()
                .filter_map(TagCountry::projection)
                .map(|country| country.state.pending_bytes)
                .sum::<usize>()
            + self
                .directory
                .aliases
                .iter()
                .map(|alias| std::mem::size_of::<Alias>() + alias.tag.capacity())
                .sum::<usize>()
    }

    fn drain_quiet<W: Pages + ?Sized>(&mut self, writer: &mut W) -> io::Result<()> {
        let Some((first, last)) = self.quiet.take() else {
            return Ok(());
        };
        if let Some(mut input) = self.last_input.take() {
            input.hour = first;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            input.identities_changed = false;
            self.observe_input(writer, &input)?;
            let evidence = std::mem::take(&mut self.evidence);
            self.evidence_base_bytes = self.evidence_base_bytes.saturating_add(self.evidence_bytes);
            self.evidence_bytes = 0;
            if !evidence.is_empty() {
                let mut facts = private::Store::new(writer, self.directory.branch);
                for (key, country) in &mut self.native {
                    let selected = if input.linked.contains(&(key.slot, key.id)) {
                        Country::Found(key.slot, key.id)
                    } else {
                        Country::Unresolved
                    };
                    country
                        .state
                        .observe_evidence(&mut facts, &mut input, selected, &evidence)?;
                }
                for (tag, alias) in &mut self.aliases {
                    if let Some(country) = alias.projection_mut() {
                        let selected = input
                            .countries
                            .as_ref()
                            .and_then(|countries| countries.get(tag))
                            .copied()
                            .unwrap_or(Country::Absent);
                        country
                            .state
                            .observe_evidence(&mut facts, &mut input, selected, &evidence)?;
                    }
                }
                self.absent.state.observe_evidence(
                    &mut facts,
                    &mut input,
                    Country::Absent,
                    &evidence,
                )?;
            }
            input.hour = last;
            input.last_hour = last;
            input.lines_read = false;
            input.output_read = false;
            input.modifiers_read = false;
            self.last_input = Some(input);
        }
        Ok(())
    }

    fn initialize_tags<W: Pages + ?Sized>(&mut self, writer: &mut W) -> io::Result<()> {
        let mut facts = private::Store::new(writer, self.directory.branch);
        while let Some(tag) = self.initial_tags.first() {
            private::put(
                &mut facts,
                &mut self.directory.index,
                &format!("tagname-{tag}"),
                tag,
            )?;
            self.initial_tags.pop_first();
        }
        Ok(())
    }

    pub fn reconcile_tags<'a, W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        tags: impl IntoIterator<Item = &'a str>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.next_block(replay)?;
        let result = (|| {
            crate::metadata::check_cancel(cancelled)?;
            self.directory.coverage.complete &=
                self.directory.last_block == last_block(replay, self.directory.branch);
            let mut changed = false;
            for tag in tags {
                crate::metadata::check_cancel(cancelled)?;
                let name = format!("tagname-{tag}");
                let mut facts = private::Store::new(writer, self.directory.branch);
                if facts.get(self.directory.index, &name)?.is_some() {
                    continue;
                }
                private::put(
                    &mut facts,
                    &mut self.directory.index,
                    &name,
                    &tag.to_owned(),
                )?;
                changed = true;
            }
            crate::metadata::check_cancel(cancelled)?;
            if !self.directory.coverage.complete
                && let Some(block) = self.processed.clone().filter(|block| {
                    Some((block.first_hour, block.last_hour, block.offset))
                        == last_block(replay, self.directory.branch)
                })
            {
                self.publish(writer, replay, &block, cancelled)?;
            } else if changed {
                pages::write_json(
                    writer,
                    &directory_section(self.directory.branch),
                    0,
                    &self.directory,
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.block_failed = true;
        }
        result
    }

    fn observe_input<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        input: &HourInput,
    ) -> io::Result<()> {
        self.initialize_tags(writer)?;
        let mut facts = private::Store::new(writer, self.directory.branch);
        let mut born = BTreeSet::new();
        if input.rows_complete && (input.identities_changed || self.active.is_empty()) {
            let mut active = BTreeMap::new();
            for (&(slot, id), tag) in &input.holders {
                let link_key = format!("country-{slot}-{id}");
                let existing = self
                    .active
                    .get(&(slot, id))
                    .map(|country| country.native.clone());
                let previous = if existing.is_none() {
                    private::get(&mut facts, self.directory.index, &link_key)?
                } else {
                    None
                };
                let key = if let Some(key) = existing.or(previous) {
                    key
                } else {
                    let name = format!("epoch-{slot}-{id}");
                    let epoch = private::get::<u64>(&mut facts, self.directory.index, &name)?
                        .unwrap_or(0)
                        + 1;
                    private::put(&mut facts, &mut self.directory.index, &name, &epoch)?;
                    NativeCountry { slot, id, epoch }
                };
                if !self.native.contains_key(&key) {
                    let saved: Option<CountrySnapshot> =
                        private::get(&mut facts, self.directory.index, &native_key(&key))?;
                    let country = match saved {
                        Some(saved) => PagedCountry::restore(
                            saved,
                            input.localisation.clone(),
                            Arc::clone(&input.language),
                        )?,
                        None => {
                            born.insert(key.clone());
                            self.absent.clone()
                        }
                    };
                    let mut country = country;
                    country.wake(&mut facts, &self.absent)?;
                    self.native.insert(key.clone(), country);
                }
                private::put(&mut facts, &mut self.directory.index, &link_key, &key)?;
                active.insert((slot, id), ActiveCountry { native: key });
                let Some(tag) = tag else {
                    continue;
                };
                if !self.aliases.contains_key(tag) {
                    let saved: Option<TagSnapshot> =
                        private::get(&mut facts, self.directory.index, &tag_key(tag))?;
                    let mut alias = match saved {
                        Some(saved) => TagCountry::restore(
                            saved,
                            input.localisation.clone(),
                            Arc::clone(&input.language),
                        )?,
                        None => TagCountry::Absent,
                    };
                    if let Some(country) = alias.projection_mut() {
                        country.wake(&mut facts, &self.absent)?;
                    }
                    self.aliases.insert(tag.clone(), alias);
                }
                private::put(
                    &mut facts,
                    &mut self.directory.index,
                    &format!("tagname-{tag}"),
                    tag,
                )?;
            }
            self.active = active;
        }
        if let Some(countries) = &input.countries {
            for (tag, country) in countries {
                if input.identities_changed {
                    private::put(
                        &mut facts,
                        &mut self.directory.index,
                        &format!("tagname-{tag}"),
                        tag,
                    )?;
                }
                self.aliases
                    .entry(tag.clone())
                    .or_insert(TagCountry::Absent);
                if let Country::Found(slot, id) = country
                    && let Some(native) = self
                        .active
                        .get(&(*slot, *id))
                        .map(|country| &country.native)
                {
                    let previous = self
                        .alias_last
                        .get(tag)
                        .copied()
                        .and_then(|index| self.directory.aliases.get_mut(index));
                    if let Some(previous) = previous.filter(|previous| {
                        previous.country == *native
                            && previous.last_hour.checked_add(1) == Some(input.hour)
                    }) {
                        previous.last_hour = input.last_hour;
                    } else {
                        self.alias_last
                            .insert(tag.clone(), self.directory.aliases.len());
                        self.directory.aliases.push(Alias {
                            tag: tag.clone(),
                            first_hour: input.hour,
                            last_hour: input.last_hour,
                            country: native.clone(),
                        });
                    }
                }
            }
        }
        for (tag, alias) in &mut self.aliases {
            let selected = input
                .countries
                .as_ref()
                .map(|countries| countries.get(tag).copied().unwrap_or(Country::Absent));
            let native = selected
                .and_then(Country::found)
                .and_then(|identity| self.active.get(&identity).map(|country| &country.native));
            match &*alias {
                TagCountry::Native(key)
                    if native == Some(key)
                        || input.countries.is_none()
                        || (selected == Some(Country::Absent)
                            && self
                                .active
                                .get(&(key.slot, key.id))
                                .is_none_or(|country| &country.native != key)) =>
                {
                    continue;
                }
                TagCountry::Absent if matches!(selected, None | Some(Country::Absent)) => continue,
                TagCountry::Absent if native.is_some_and(|key| born.contains(key)) => {
                    *alias = TagCountry::Native(native.unwrap().clone());
                    continue;
                }
                TagCountry::Projected(_) => continue,
                _ => {}
            }
            // A literal tag diverges at renames/reuse; retain its v5 history without changing native identity.
            let mut country = match &*alias {
                TagCountry::Absent => self.absent.clone(),
                TagCountry::Native(key) => match self.native.get(key) {
                    Some(country) => country.clone(),
                    None => PagedCountry::restore(
                        private::get(&mut facts, self.directory.index, &native_key(key))?
                            .ok_or_else(|| crate::invalid("Missing retired production country"))?,
                        input.localisation.clone(),
                        Arc::clone(&input.language),
                    )?,
                },
                TagCountry::Projected(_) => unreachable!(),
            };
            country.wake(&mut facts, &self.absent)?;
            *alias = TagCountry::Projected(Box::new(country));
        }
        for (key, country) in &mut self.native {
            let selected = input.countries.as_ref().map(|_| {
                if self
                    .active
                    .get(&(key.slot, key.id))
                    .is_some_and(|country| &country.native == key)
                {
                    if input.linked.contains(&(key.slot, key.id)) {
                        Country::Found(key.slot, key.id)
                    } else {
                        Country::Unresolved
                    }
                } else {
                    Country::Absent
                }
            });
            country.state.observe_input(&mut facts, input, selected)?;
        }
        for (tag, alias) in &mut self.aliases {
            let Some(country) = alias.projection_mut() else {
                continue;
            };
            let selected = input
                .countries
                .as_ref()
                .map(|countries| countries.get(tag).copied().unwrap_or(Country::Absent));
            country.state.observe_input(&mut facts, input, selected)?;
        }
        self.absent.state.observe_input(
            &mut facts,
            input,
            input.countries.as_ref().map(|_| Country::Absent),
        )?;
        let retired: Vec<_> = self
            .native
            .keys()
            .filter(|key| {
                self.active
                    .get(&(key.slot, key.id))
                    .is_none_or(|active| &active.native != *key)
            })
            .cloned()
            .collect();
        for key in retired {
            let mut country = self.native.remove(&key).unwrap();
            country.sleep = Some(self.absent.dormant(input.last_hour));
            country.flush(&mut facts, &AtomicBool::new(false))?;
            private::put(
                &mut facts,
                &mut self.directory.index,
                &native_key(&key),
                &country.snapshot(&AtomicBool::new(false))?,
            )?;
            private::put(
                &mut facts,
                &mut self.directory.index,
                &format!("history-{}", native_key(&key)),
                &country.head,
            )?;
        }
        let dormant: Vec<_> = self
            .aliases
            .keys()
            .filter(|tag| {
                input.countries.as_ref().is_some_and(|countries| {
                    matches!(countries.get(*tag), None | Some(Country::Absent))
                })
            })
            .cloned()
            .collect();
        for tag in dormant {
            let mut alias = self.aliases.remove(&tag).unwrap();
            if let Some(country) = alias.projection_mut() {
                country.sleep = Some(self.absent.dormant(input.last_hour));
                country.flush(&mut facts, &AtomicBool::new(false))?;
            }
            let head = match &alias {
                TagCountry::Absent => self.absent.head,
                TagCountry::Native(key) => self
                    .native
                    .get(key)
                    .map(|country| country.head)
                    .or(private::get(
                        &mut facts,
                        self.directory.index,
                        &format!("history-{}", native_key(key)),
                    )?)
                    .unwrap_or_default(),
                TagCountry::Projected(country) => country.head,
            };
            private::put(
                &mut facts,
                &mut self.directory.index,
                &format!("history-{}", tag_key(&tag)),
                &head,
            )?;
            private::put(
                &mut facts,
                &mut self.directory.index,
                &tag_key(&tag),
                &alias.snapshot(&AtomicBool::new(false))?,
            )?;
        }
        Ok(())
    }

    pub fn lost(&mut self, first: i32, last: i32) {
        self.absent.state.lost(first, last);
        for country in self.native.values_mut().chain(
            self.aliases
                .values_mut()
                .filter_map(TagCountry::projection_mut),
        ) {
            country.state.lost(first, last);
        }
    }

    fn flush<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        crate::metadata::check_cancel(cancelled)?;
        self.drain_quiet(writer)?;
        self.initialize_tags(writer)?;
        let ordinal = self.directory.pages;
        let branch = self.directory.branch;
        let variants = VariantPage {
            catalogue: self.world.variants.clone(),
            icons: std::mem::take(&mut self.world.icon_events),
        };
        let mut facts = private::Store::new(writer, branch);
        self.directory
            .variants
            .append(&mut facts, &variant_section(branch), &variants)?;
        drop(variants);
        self.absent.flush(&mut facts, cancelled)?;
        private::put(
            &mut facts,
            &mut self.directory.index,
            "history-absent",
            &self.absent.head,
        )?;
        for (native, country) in &mut self.native {
            country.flush(&mut facts, cancelled)?;
            private::put(
                &mut facts,
                &mut self.directory.index,
                &format!("history-{}", native_key(native)),
                &country.head,
            )?;
        }
        for (tag, alias) in &mut self.aliases {
            let head = match alias {
                TagCountry::Absent => self.absent.head,
                TagCountry::Native(native) => match self.native.get(native) {
                    Some(country) => country.head,
                    None => private::get(
                        &mut facts,
                        self.directory.index,
                        &format!("history-{}", native_key(native)),
                    )?
                    .unwrap_or_default(),
                },
                TagCountry::Projected(country) => {
                    country.flush(&mut facts, cancelled)?;
                    country.head
                }
            };
            private::put(
                &mut facts,
                &mut self.directory.index,
                &format!("history-{}", tag_key(tag)),
                &head,
            )?;
        }
        for versions in self.world.variants.versions.values_mut() {
            pages::tail(versions);
        }
        pages::write(
            facts.pages,
            &alias_section(branch),
            ordinal,
            &self.directory.aliases,
        )?;
        let mut last = BTreeMap::new();
        for alias in std::mem::take(&mut self.directory.aliases) {
            last.insert(alias.tag.clone(), alias);
        }
        self.directory.aliases = last
            .into_values()
            .filter(|alias| self.aliases.contains_key(&alias.tag))
            .collect();
        self.alias_last = self
            .directory
            .aliases
            .iter()
            .enumerate()
            .map(|(index, alias)| (alias.tag.clone(), index))
            .collect();
        self.world.pending_bytes = 0;
        self.evidence_base_bytes = self.pending_total();
        self.directory.pages += 1;
        Ok(())
    }

    pub fn publish<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        replay: &Replay,
        block: &BlockIndex,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.next_block(replay)?;
        if self.processed.as_ref() != Some(block) {
            return Err(crate::invalid(
                "Production publication exceeds processed blocks",
            ));
        }
        self.flush(writer, cancelled)?;
        let branch = self.directory.branch;
        self.directory.last_block = Some((block.first_hour, block.last_hour, block.offset));
        self.directory.coverage.last_hour = block.last_hour;
        self.directory.coverage.complete = self.directory.last_block == last_block(replay, branch);

        pages::write_json(writer, &directory_section(branch), 0, &self.directory)?;
        writer.write_at(
            &coverage_section(branch),
            0,
            &serde_json::to_vec(&self.directory.coverage)?,
        )?;
        self.save(writer, cancelled)
    }
}

fn assemble(
    directory: &Directory,
    selected: &str,
    icons: Option<&IconContent>,
    language: Arc<str>,
    cancelled: &AtomicBool,
    mut read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<ScannedLines> {
    let mut assembled = pages::Assembler::default();
    let mut variants: BTreeMap<u64, Vec<equipment::VariantVersion>> = BTreeMap::new();
    let mut catalogue = VariantCatalogue::default();
    let localisation = icons
        .and_then(|icons| icons.catalogue.as_ref())
        .map(|catalogue| Arc::clone(&catalogue.localisation));
    let mut head: pages::PageLog = private::get(
        &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
        directory.index,
        &format!("history-{selected}"),
    )?
    .unwrap_or_default();
    let mut tag_native = None;
    let mut tag_sleep = None;
    if selected.starts_with("tag-") {
        let tag: Option<TagSnapshot> = private::get(
            &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
            directory.index,
            selected,
        )?;
        if matches!(&tag, Some(TagSnapshot::Absent)) || (tag.is_none() && head.len == 0) {
            head = private::get(
                &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
                directory.index,
                "history-absent",
            )?
            .unwrap_or_default();
        }
        match tag {
            Some(TagSnapshot::Native(key)) => tag_native = Some(key),
            Some(TagSnapshot::Projected(snapshot)) => tag_sleep = snapshot.sleep,
            _ => {}
        }
    }
    let mut reader = ReadPages(&mut read);
    let mut facts = private::Store::new(&mut reader, directory.branch);
    let mut cursor = head.cursor(0)?;
    while let Some(page) = cursor.next(&mut facts, &history_section(directory.branch))? {
        crate::metadata::check_cancel(cancelled)?;
        assembled.push(page, localisation.clone(), Arc::clone(&language))?;
    }
    {
        let _naming = replay_frame::cpu::scope("production naming", Some("production-lines"));
        let mut cursor = directory.variants.cursor(0)?;
        while let Some(next) =
            cursor.next::<VariantPage>(&mut facts, &variant_section(directory.branch))?
        {
            crate::metadata::check_cancel(cancelled)?;
            #[cfg(test)]
            let _workspace = tests::NamingOperands::hold(&next.catalogue, &variants);
            let VariantPage {
                catalogue: mut next,
                icons,
            } = next;
            for (id, versions) in std::mem::take(&mut next.versions) {
                let known = variants.entry(id).or_default();
                for version in versions {
                    match known
                        .last_mut()
                        .filter(|last| last.first_hour == version.first_hour)
                    {
                        Some(last) => *last = version,
                        None => known.push(version),
                    }
                }
            }
            catalogue = next;
            for (id, hour, icon, definition_changed) in icons {
                if let Some(versions) = assembled.result.icons.get_mut(&id)
                    && versions.first().is_some_and(|first| first.0 <= hour)
                {
                    let at = versions.partition_point(|(at, _)| *at <= hour);
                    let pending = at != 0
                        && versions[at - 1]
                            .1
                            .otherwise
                            .as_ref()
                            .err()
                            .is_some_and(|reason| {
                                matches!(
                                    reason.as_str(),
                                    "the variant's definition is not recorded yet"
                                        | "the variant's definition is not recorded at this time"
                                )
                            });
                    if (definition_changed || pending) && (at == 0 || versions[at - 1].1 != icon) {
                        versions.insert(at, (hour, icon));
                    }
                }
            }
        }
        catalogue.versions = variants;
    }

    drop(facts);
    drop(reader);
    for versions in assembled.result.icons.values_mut() {
        versions.dedup_by(|a, b| a.1 == b.1);
    }
    assembled.result.variants = catalogue;
    let sleep = if selected.starts_with("native-") {
        private::get::<CountrySnapshot>(
            &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
            directory.index,
            selected,
        )?
        .and_then(|snapshot| snapshot.sleep)
    } else if selected.starts_with("tag-") {
        match tag_native {
            Some(key) => private::get::<CountrySnapshot>(
                &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
                directory.index,
                &native_key(&key),
            )?
            .and_then(|snapshot| snapshot.sleep),
            None => tag_sleep,
        }
    } else {
        None
    };
    if let Some(sleep) = sleep {
        let absent: CountrySnapshot = private::get(
            &mut private::Store::new(&mut ReadPages(&mut read), directory.branch),
            directory.index,
            "absent",
        )?
        .ok_or_else(|| crate::invalid("Missing production absence continuation"))?;
        add_absence(
            &mut assembled.result,
            &sleep,
            absent.head,
            &absent.page.lines,
            directory.branch,
            &mut read,
        )?;
    }
    Ok(assembled.result)
}

pub fn directory(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<Option<Directory>> {
    crate::prepared::series::read_reply(store, key, &directory_section(branch), 0)
}

pub(super) fn binary_current(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
) -> io::Result<bool> {
    Ok(store
        .read(key, &state_section(branch, "world"), 0)?
        .is_some())
}

pub fn current(
    store: &store::Store,
    key: &store::SourceKey,
    source: &Source,
    context: &LoadContext,
    branch: u64,
) -> io::Result<bool> {
    if !binary_current(store, key, branch)? {
        return Ok(false);
    }

    let Some(directory) = directory(store, key, branch)? else {
        return Ok(false);
    };
    let (_, icons) = scan_content(source, context, &mut Vec::new());
    if directory.coverage.schema != crate::prepared::coverage::SCHEMA
        || !directory.coverage.complete
        || directory.last_block != last_block(source.replay_ref(), branch)
        || directory.executable != engine_executable::executable_tables::is_initialized()
        || !pictures_hold_with(
            icons.as_deref(),
            directory.pictured,
            directory.visual.as_deref(),
        )
    {
        return Ok(false);
    }
    let Some(session) = store.session(key)? else {
        return Ok(false);
    };
    let mut read = |section: &str, range| {
        use std::io::Read;
        session
            .read(section, range)?
            .map(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            })
            .transpose()
    };
    let mut reader = ReadPages(&mut read);
    let mut facts = private::Store::new(&mut reader, branch);
    for tag in source.with_metadata(|metadata| {
        metadata
            .countries
            .iter()
            .map(|country| country.tag.clone())
            .collect::<Vec<_>>()
    }) {
        if facts
            .get(directory.index, &format!("tagname-{tag}"))?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn query(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    tag: &str,
    holder: Option<NativeCountry>,
    icons: Option<&IconContent>,
    cancelled: &AtomicBool,
) -> io::Result<(ScannedLines, crate::prepared::coverage::Coverage)> {
    use std::io::Read;
    crate::metadata::check_cancel(cancelled)?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(result) = fixtures::query(source, branch, tag, holder.clone(), icons, cancelled)? {
        return Ok(result);
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let _scope = store.scope(&key)?;
    if !binary_current(&store, &key, branch)? {
        return Err(crate::invalid(
            "Production lines need preparation for the current private-page schema",
        ));
    }
    let mut directory = directory(&store, &key, branch)?
        .ok_or_else(|| crate::invalid("Production lines are still being prepared"))?;
    if directory.coverage.schema != crate::prepared::coverage::SCHEMA
        || directory.executable != engine_executable::executable_tables::is_initialized()
        || !pictures_hold_with(icons, directory.pictured, directory.visual.as_deref())
    {
        return Err(crate::invalid(
            "Production lines need preparation for the current content",
        ));
    }
    directory.coverage.complete &= directory.last_block == last_block(source.replay_ref(), branch);
    let selected = match holder {
        None => tag_key(tag),
        Some(country) => {
            if private::get::<pages::PageLog>(
                &mut private::Store::new(
                    &mut ReadPages(&mut |section: &str, range| {
                        use std::io::Read;
                        store
                            .read(&key, section, range)?
                            .map(|mut file| {
                                let mut bytes = Vec::new();
                                file.read_to_end(&mut bytes)?;
                                Ok(bytes)
                            })
                            .transpose()
                    }),
                    branch,
                ),
                directory.index,
                &format!("history-{}", native_key(&country)),
            )?
            .is_none()
            {
                return Err(crate::invalid(
                    "Production country identity is not prepared",
                ));
            }
            native_key(&country)
        }
    };
    let result = assemble(
        &directory,
        &selected,
        icons,
        recorded_language(source.replay_ref()),
        cancelled,
        |section, range| {
            store
                .read(&key, section, range)?
                .map(|mut file| {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
                .transpose()
        },
    )?;
    if !directory.coverage.complete {
        crate::prepared::coverage::note_short("production-lines");
    }
    Ok((result, directory.coverage))
}
impl CountryState {
    pub fn new() -> Self {
        Self {
            first_read: true,
            ..Self::default()
        }
    }
    fn observe_evidence(
        &mut self,
        facts: &mut dyn private::Facts,
        input: &mut HourInput,
        country: Country,
        evidence: &[ReadEvidence],
    ) -> io::Result<()> {
        let named: BTreeSet<_> = country
            .found()
            .filter(|_| evidence.iter().any(|read| read.organisation_changed))
            .into_iter()
            .flat_map(|(slot, _)| {
                input
                    .lines
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .flat_map(|row| {
                        std::iter::once(row.mio)
                            .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                    })
            })
            .filter(|id| *id != 0)
            .collect();
        for read in evidence {
            input.hour = read.hour;
            input.lines_read = read.lines_read;
            input.output_read = read.output_read;
            input.modifiers_read = read.modifiers_read;
            input.design_revision = read.design_revision;
            input.design_read_hour = read.design_read_hour;
            input.organisation_read_hour = read.organisation_read_hour;
            input.author_names_due = read.author_names_due;
            if let Some(available) = input.output_due(Some(country)) {
                if !available {
                    self.result.unread_output_reads += 1;
                    let first = self
                        .last_output_due
                        .and_then(|hour| hour.checked_add(1))
                        .unwrap_or(read.hour);
                    mark_span(
                        &mut self.result.unread_output,
                        first.min(read.hour),
                        read.hour,
                    );
                }
                self.last_output_due = Some(read.hour);
            }
            let changed = self.last_design_revision != Some(read.design_revision);
            self.last_design_revision = Some(read.design_revision);
            if !self.carried {
                continue;
            }
            if changed {
                for &variant in &self.active_variants {
                    self.pending_bytes +=
                        record_design(&mut self.result, facts, self.history, input, variant)?;
                }
            }
            if !read.lines_read {
                continue;
            }
            self.pending_bytes += std::mem::size_of::<i32>();
            self.result.reads.push(read.hour);
            self.last_census = Some(read.hour);
            if read.modifiers_read
                && let Some(reason) = &input.modifier_refusal
            {
                *self
                    .result
                    .modifier_layout_refusals
                    .entry(reason.clone())
                    .or_default() += 1;
            }
            if read.organisation_changed {
                for id in &named {
                    if let Some(raw) = input.organisations.get(id) {
                        self.pending_bytes +=
                            load_organisation(&mut self.result, facts, self.history, *id)?;
                        let mut raw = RawOrganisation {
                            first_hour: read.hour,
                            ..raw.clone()
                        };
                        if let Some(state) = &mut raw.state {
                            state.read_hour = read.organisation_read_hour;
                        }
                        let versions = self.result.organisations.entry(*id).or_default();
                        if version(versions, raw, |a, b| {
                            RawOrganisation {
                                first_hour: 0,
                                ..a.clone()
                            } == RawOrganisation {
                                first_hour: 0,
                                ..b.clone()
                            }
                        }) {
                            self.pending_bytes += pages::record_bytes(versions.last().unwrap());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn observe_input(
        &mut self,
        facts: &mut dyn private::Facts,
        input: &HourInput,
        country: Option<Country>,
    ) -> io::Result<()> {
        crate::prepared::prepass::note("production-lines.country_fold");
        let Self {
            mut result,
            mut pending_bytes,
            mut last,
            mut open,
            mut carried,
            mut broken,
            mut last_read_identity,
            mut broken_identity,
            mut first_read,
            mut output_open,
            mut organisations_open,
            mut last_census,
            mut appearances,
            mut latest,
            history,
            mut active_variants,
            mut carry,
            mut last_output_due,
            mut last_modifier_country,
            mut last_design_revision,
        } = std::mem::take(self);
        let observed = (|| -> io::Result<()> {
            let hour = input.hour;
            pending_bytes += 6 * std::mem::size_of::<crate::products::country_groups::HourSpan>()
                + std::mem::size_of::<i32>();
            let through = input.last_hour;
            let identity = country.and_then(Country::found);
            let continues = identity.is_some_and(|identity| {
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour))
            });
            if carry.as_ref().is_some_and(|span| {
                !continues
                    || input.lines_read
                    || !input.lines_complete
                    || span.modifiers != input.modifiers_complete
                    || span.output != (output_open && input.output_complete)
            }) {
                let span = carry.take().unwrap();
                carry_lines(
                    &mut result,
                    &open,
                    (span.first, span.last),
                    span.modifiers,
                    span.output,
                );
            }
            last = identity.map(|identity| (identity, through));
            if let Some(read) = input.output_due(country) {
                if !read {
                    result.unread_output_reads += 1;
                    let first = last_output_due
                        .and_then(|due| due.checked_add(1))
                        .unwrap_or(hour);
                    mark_span(&mut result.unread_output, first.min(hour), hour);
                }
                last_output_due = Some(hour);
            }
            if carried && (!continues || !input.lines_complete) {
                broken = std::mem::take(&mut open);
                broken_identity = last_read_identity;
                carried = false;
                output_open = false;
                organisations_open = false;
            }
            let designs_changed = last_design_revision != Some(input.design_revision);
            last_design_revision = Some(input.design_revision);
            let (Some(identity), true) = (identity, input.lines_complete) else {
                return Ok(());
            };
            let slot = identity.0;
            if carried && designs_changed {
                let demanded = active_variants.iter().copied();
                for variant in demanded {
                    pending_bytes += record_design(&mut result, facts, history, input, variant)?;
                }
            }
            if !input.lines_read {
                if carried {
                    let _phase = replay_frame::cpu::scope_phase("production-lines:carry");
                    mark_span(&mut result.line_hours, hour, through);
                    output_open &= input.output_complete;
                    organisations_open &= input.organisations_complete;
                    if organisations_open {
                        mark_span(&mut result.organisation_hours, hour, through);
                    }
                    let modifiers_carried = input.modifiers_complete;
                    match &mut carry {
                        Some(span) => span.last = through,
                        None => {
                            carry = Some(Carry {
                                first: hour,
                                last: through,
                                modifiers: modifiers_carried,
                                output: output_open,
                            })
                        }
                    }
                    if output_open {
                        mark_span(&mut result.output_hours, hour, through);
                    }
                }
                return Ok(());
            }
            let rows = input
                .lines
                .get(&slot)
                .map(Vec::as_slice)
                .unwrap_or_default();
            result.reads.push(hour);
            mark(&mut result.line_hours, hour);
            let previous_read = last_census;
            last_census = Some(hour);
            if broken_identity != Some(identity) {
                broken.clear();
            }
            last_read_identity = Some(identity);
            let mut still = BTreeMap::new();
            result.unlinked_rows += input.unlinked_lines.len();
            let mut mine: Vec<&ProductionLineRow> = rows
                .iter()
                .chain(input.unlinked_lines.iter().filter(|row| {
                    open.contains_key(&row.handle) || broken.contains_key(&row.handle)
                }))
                .collect();
            mine.sort_by_key(|row| row.handle);
            let modifier_country = input.modifier_countries.get(&identity.1).map(|country| {
                if last_modifier_country.as_deref() != Some(country) {
                    pending_bytes += modifier_country_bytes(country);
                    last_modifier_country = Some(Arc::new(country.clone()));
                }
                Arc::clone(
                    last_modifier_country
                        .as_ref()
                        .expect("recorded modifier country"),
                )
            });
            if input.modifiers_read
                && let Some(reason) = &input.modifier_refusal
            {
                *result
                    .modifier_layout_refusals
                    .entry(reason.clone())
                    .or_default() += 1;
            }
            for row in &mine {
                pending_bytes +=
                    std::mem::size_of::<ScannedLine>() + 40 + std::mem::size_of::<RawRun>();
                let Some(kind) = LineKind::of(row.kind) else {
                    result.unknown_kinds += 1;
                    continue;
                };
                let (index, starts_segment) = if let Some(line) = open.remove(&row.handle) {
                    (line.index, false)
                } else if let Some(line) = broken.remove(&row.handle) {
                    (line.index, true)
                } else {
                    if !appearances.contains_key(&row.handle) {
                        let old: Option<ClosedLine> =
                            private::get(facts, history, &row.handle.to_string())?;
                        appearances.insert(row.handle, old.map_or(0, |old| old.appearance));
                    }
                    let appearance = appearances.entry(row.handle).or_default();
                    *appearance += 1;
                    result.lines.push(ScannedLine {
                        key: format!("{}#{appearance}", row.handle),
                        handle: row.handle,
                        kind: Some(kind),
                        runs: Vec::new(),
                        output: Vec::new(),
                        final_output: None,
                        ended_by: None,
                        before_recording: first_read,
                        between_reads: None,
                        variants: Vec::new(),
                    });
                    (result.lines.len() - 1, true)
                };
                latest.insert(row.handle, index);
                pending_bytes += record_design(&mut result, facts, history, input, row.variant)?;
                let values = RawRun {
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment,
                    variant: row.variant,
                    mio: row.mio,
                    active_factories: row.active_factories,
                    requested_factories: row.requested_factories,
                    max_factories: row.max_factories,
                    cost_raw: row.cost_raw,
                    speed_raw: row.speed_raw,
                    efficiency_raw: row.efficiency_raw,
                    priority: row.priority,
                    amount: row.amount,
                };
                let line = &mut result.lines[index];
                if let Some(country) = modifier_country
                    .as_ref()
                    .filter(|country| country.country == row.country)
                    && let Some(operands) = input.modifier_lines.get(&row.handle)
                {
                    if (
                        operands.country,
                        operands.variant,
                        operands.organisation,
                        operands.kind,
                        operands.active_factories,
                    ) == (
                        row.country,
                        row.variant,
                        row.mio,
                        row.kind,
                        row.active_factories,
                    ) {
                        let organisation_name =
                            input.organisation_names.get(&row.mio).cloned().flatten();
                        pending_bytes += modifier_bytes(operands)
                            + organisation_name.as_ref().map_or(0, String::capacity);
                        result.modifiers.entry(line.key.clone()).or_default().push(
                            breakdown::RawModifierRun {
                                first_hour: hour,
                                last_hour: hour,
                                country: Arc::clone(country),
                                line: operands.clone(),
                                run: values,
                                organisation: organisation_name,
                                localisation: input.localisation.clone(),
                                recorded_language: Arc::clone(&input.language),
                            },
                        );
                    }
                }
                match line.runs.last_mut() {
                    Some(run)
                        if !starts_segment
                            && run.last_hour.checked_add(1) == Some(hour)
                            && same_values(run, &values) =>
                    {
                        run.last_hour = hour
                    }
                    _ => {
                        line.runs.push(values);
                    }
                }
                still.insert(row.handle, Open { index });
            }
            for line in open.values().chain(broken.values()) {
                result.lines[line.index].ended_by = Some(hour);
            }
            active_variants = still
                .values()
                .filter_map(|line| result.lines[line.index].runs.last().map(|run| run.variant))
                .collect();
            broken.clear();
            open = still;
            carried = true;
            first_read = false;
            output_open = false;
            let mut produced_between: BTreeSet<u64> = BTreeSet::new();
            if input.output_complete {
                output_open = true;
                mark(&mut result.output_hours, hour);
                let mut by_line: BTreeMap<u64, Vec<(&ProductionLineOutputRow, bool)>> =
                    BTreeMap::new();
                result.unlinked_rows += input.unlinked_output.len();
                for (row, linked) in input
                    .output
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .map(|row| (row, true))
                    .chain(input.unlinked_output.iter().map(|row| (row, false)))
                {
                    if !linked
                        && !latest.contains_key(&row.line)
                        && private::get::<ClosedLine>(facts, history, &row.line.to_string())?
                            .is_none()
                    {
                        continue;
                    }
                    by_line.entry(row.line).or_default().push((row, linked));
                    produced_between.insert(row.variant);
                }
                for (handle, mut rows) in by_line {
                    rows.sort_by_key(|(row, _)| row.ordinal);
                    if !latest.contains_key(&handle) {
                        if let Some(old) =
                            private::get::<ClosedLine>(facts, history, &handle.to_string())?
                        {
                            appearances.insert(handle, old.appearance);
                            latest.insert(handle, result.lines.len());
                            result.lines.push(old.line);
                        }
                    }
                    let index = match latest.get(&handle) {
                        Some(&index) => index,
                        None if rows.iter().all(|(row, _)| row.ended) => {
                            let whole_life =
                                rows.iter().all(|(row, _)| row.observed_since_creation);
                            let since = rows
                                .iter()
                                .map(|(row, _)| row.since_hour)
                                .min()
                                .unwrap_or(hour);
                            let (first_hour, before) = match previous_read {
                                Some(read) if whole_life => (read + 1, false),
                                _ => (since.min(hour), true),
                            };
                            let appearance = appearances.entry(handle).or_default();
                            *appearance += 1;
                            result.lines.push(ScannedLine {
                                key: format!("{handle}#{appearance}"),
                                handle,
                                kind: None,
                                runs: Vec::new(),
                                output: Vec::new(),
                                final_output: None,
                                ended_by: Some(hour),
                                before_recording: before,
                                between_reads: Some(crate::products::country_groups::HourSpan {
                                    first_hour,
                                    last_hour: hour,
                                }),
                                variants: rows.iter().map(|(row, _)| row.variant).collect(),
                            });
                            let index = result.lines.len() - 1;
                            latest.insert(handle, index);
                            produced_between.extend(rows.iter().map(|(row, _)| row.variant));
                            index
                        }
                        None => {
                            result.unlisted_output += 1;
                            continue;
                        }
                    };
                    let run = output_of(&rows, hour);
                    pending_bytes +=
                        pages::record_bytes(&run) + std::mem::size_of::<ScannedLine>() + 40;
                    let line = &mut result.lines[index];
                    if rows.iter().any(|(row, _)| row.ended) || !open.contains_key(&handle) {
                        line.final_output = Some(run);
                    } else {
                        line.output.push(run);
                    }
                }
            }
            organisations_open = false;
            if input.organisations_named {
                organisations_open = true;
                mark(&mut result.organisation_hours, hour);
                let named: BTreeSet<u64> = mine
                    .iter()
                    .flat_map(|row| {
                        std::iter::once(row.mio)
                            .chain(input.designs.get(&row.variant).and_then(|read| read.author))
                    })
                    .filter(|mio| *mio != 0)
                    .collect();
                for id in named {
                    if let Some(raw) = input.organisations.get(&id) {
                        pending_bytes += load_organisation(&mut result, facts, history, id)?;
                        let mut raw = RawOrganisation {
                            first_hour: hour,
                            ..raw.clone()
                        };
                        if let Some(state) = &mut raw.state {
                            state.read_hour = input.organisation_read_hour;
                        }
                        let versions = result.organisations.entry(id).or_default();
                        if version(versions, raw, |a, b| {
                            RawOrganisation {
                                first_hour: 0,
                                ..a.clone()
                            } == RawOrganisation {
                                first_hour: 0,
                                ..b.clone()
                            }
                        }) {
                            pending_bytes += pages::record_bytes(versions.last().unwrap());
                        }
                    }
                }
            }
            let produced: BTreeSet<u64> = mine
                .iter()
                .map(|row| row.variant)
                .chain(produced_between)
                .collect();
            for id in produced {
                if let Some(icon) = input.icons.get(&id) {
                    if !result.icons.contains_key(&id)
                        && let Some(last) = private::get(facts, history, &format!("icon-{id}"))?
                    {
                        pending_bytes += pages::record_bytes(&last);
                        result.icons.insert(id, vec![last]);
                    }
                    if version(
                        result.icons.entry(id).or_default(),
                        (hour, icon.clone()),
                        |a, b| a.1 == b.1,
                    ) {
                        pending_bytes += pages::record_bytes(icon);
                    }
                }
            }

            Ok(())
        })();
        *self = Self {
            result,
            pending_bytes,
            last,
            open,
            carried,
            broken,
            last_read_identity,
            broken_identity,
            first_read,
            output_open,
            organisations_open,
            last_census,
            appearances,
            latest,
            history,
            active_variants,
            carry,
            last_output_due,
            last_modifier_country,
            last_design_revision,
        };
        observed
    }
}

#[derive(Serialize, Deserialize)]
enum TagSnapshot {
    Absent,
    Native(NativeCountry),
    Projected(Box<CountrySnapshot>),
}

impl TagCountry {
    fn snapshot(&mut self, cancelled: &AtomicBool) -> io::Result<TagSnapshot> {
        Ok(match self {
            Self::Absent => TagSnapshot::Absent,
            Self::Native(country) => TagSnapshot::Native(country.clone()),
            Self::Projected(country) => {
                TagSnapshot::Projected(Box::new(country.snapshot(cancelled)?))
            }
        })
    }

    fn restore(
        snapshot: TagSnapshot,
        localisation: Option<Arc<installed_content_localisation::localisation::Localisation>>,
        language: Arc<str>,
    ) -> io::Result<Self> {
        Ok(match snapshot {
            TagSnapshot::Absent => Self::Absent,
            TagSnapshot::Native(country) => Self::Native(country),
            TagSnapshot::Projected(country) => Self::Projected(Box::new(PagedCountry::restore(
                *country,
                localisation,
                language,
            )?)),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct CountrySnapshot {
    page: pages::Page,
    head: pages::PageLog,
    sleep: Option<Dormant>,
    history: private::Root,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    pending_bytes: usize,
    last_country: Option<(Vec<u8>, u64)>,
    next_country: u64,
    last: Option<((u32, u32), i32)>,
    open: BTreeMap<u64, Open>,
    carried: bool,
    broken: BTreeMap<u64, Open>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    appearances: BTreeMap<u64, u32>,
    latest: BTreeMap<u64, usize>,
    last_output_due: Option<i32>,
    last_design_revision: Option<u64>,
}
impl PagedCountry {
    fn snapshot(&mut self, cancelled: &AtomicBool) -> io::Result<CountrySnapshot> {
        let page = self.rows.page(&self.state.result, cancelled)?;
        let last_country = self
            .state
            .last_modifier_country
            .as_ref()
            .map(|row| {
                Ok::<_, io::Error>((
                    saved::encode(replay_data::rows::TableRows::CountryProductionModifiers(
                        vec![(**row).clone()],
                    ))?,
                    self.rows.id(row),
                ))
            })
            .transpose()?;
        Ok(CountrySnapshot {
            page,
            head: self.head,
            sleep: self.sleep.clone(),
            history: self.state.history,
            active_variants: self.state.active_variants.clone(),
            carry: self.state.carry.clone(),
            pending_bytes: self.state.pending_bytes,
            last_country,
            next_country: self.rows.next,
            last: self.state.last.clone(),
            open: self.state.open.clone(),
            carried: self.state.carried.clone(),
            broken: self.state.broken.clone(),
            last_read_identity: self.state.last_read_identity.clone(),
            broken_identity: self.state.broken_identity.clone(),
            first_read: self.state.first_read.clone(),
            output_open: self.state.output_open.clone(),
            organisations_open: self.state.organisations_open.clone(),
            last_census: self.state.last_census.clone(),
            appearances: self.state.appearances.clone(),
            latest: self.state.latest.clone(),
            last_output_due: self.state.last_output_due.clone(),
            last_design_revision: self.state.last_design_revision.clone(),
        })
    }
    fn restore(
        snapshot: CountrySnapshot,
        localisation: Option<Arc<installed_content_localisation::localisation::Localisation>>,
        language: Arc<str>,
    ) -> io::Result<Self> {
        let mut assembled = pages::Assembler::default();
        let continuation = snapshot.page.continuation;
        assembled.push(snapshot.page, localisation, language)?;
        let mut rows = pages::CountryRows::default();
        rows.next = snapshot.next_country;
        rows.continuation = continuation;
        for (id, row) in &assembled.countries {
            rows.bind(row, *id);
        }
        let last_modifier_country = snapshot
            .last_country
            .map(|(bytes, id)| {
                if let Some(row) = assembled.countries.get(&id) {
                    return Ok(Arc::clone(row));
                }
                let replay_data::rows::TableRows::CountryProductionModifiers(mut decoded) =
                    saved::decode(TableId::CountryProductionModifiers, &bytes)?
                else {
                    return Err(crate::invalid("Invalid production continuation country"));
                };
                let row = Arc::new(decoded.remove(0));
                rows.bind(&row, id);
                Ok(row)
            })
            .transpose()?;
        Ok(Self {
            rows,
            head: snapshot.head,
            sleep: snapshot.sleep,
            state: CountryState {
                result: assembled.result,
                pending_bytes: snapshot.pending_bytes,
                last_modifier_country,
                last: snapshot.last,
                open: snapshot.open,
                carried: snapshot.carried,
                broken: snapshot.broken,
                last_read_identity: snapshot.last_read_identity,
                broken_identity: snapshot.broken_identity,
                first_read: snapshot.first_read,
                output_open: snapshot.output_open,
                organisations_open: snapshot.organisations_open,
                last_census: snapshot.last_census,
                appearances: snapshot.appearances,
                latest: snapshot.latest,
                history: snapshot.history,
                active_variants: snapshot.active_variants,
                carry: snapshot.carry,
                last_output_due: snapshot.last_output_due,
                last_design_revision: snapshot.last_design_revision,
            },
        })
    }
}

fn state_section(branch: u64, key: &str) -> String {
    format!("production-lines-state-v2-{branch}-{key}")
}

#[derive(Serialize, Deserialize)]
struct Continuation {
    world: input::WorldSaved,
    active: Vec<((u32, u32), ActiveCountry)>,
    native: Vec<NativeCountry>,
    aliases: Vec<String>,
}
impl Prepared {
    fn save<W: Pages + ?Sized>(
        &mut self,
        writer: &mut W,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.block_failed {
            return Err(crate::invalid("Production block did not finish"));
        }
        self.drain_quiet(writer)?;
        self.initialize_tags(writer)?;
        let branch = self.directory.branch;
        let mut facts = private::Store::new(writer, branch);
        private::put(
            &mut facts,
            &mut self.directory.index,
            "absent",
            &self.absent.snapshot(cancelled)?,
        )?;
        for (native, country) in &mut self.native {
            private::put(
                &mut facts,
                &mut self.directory.index,
                &native_key(native),
                &country.snapshot(cancelled)?,
            )?;
        }
        for (tag, alias) in &mut self.aliases {
            private::put(
                &mut facts,
                &mut self.directory.index,
                &tag_key(tag),
                &alias.snapshot(cancelled)?,
            )?;
        }
        pages::write(
            facts.pages,
            &state_section(branch, "world"),
            0,
            &Continuation {
                world: self.world.save()?,
                active: self
                    .active
                    .iter()
                    .map(|(id, key)| (*id, key.clone()))
                    .collect(),
                native: self.native.keys().cloned().collect(),
                aliases: self.aliases.keys().cloned().collect(),
            },
        )?;
        pages::write_json(facts.pages, &directory_section(branch), 0, &self.directory)
    }
    pub fn resume(
        store: &store::Store,
        key: &store::SourceKey,
        replay: &Replay,
        branch: u64,
        icons: Option<&IconContent>,
    ) -> io::Result<Option<Self>> {
        if !binary_current(store, key, branch)? {
            return Ok(None);
        }

        use std::io::Read;
        let Some(directory) = directory(store, key, branch)? else {
            return Ok(None);
        };
        if directory.coverage.schema != crate::prepared::coverage::SCHEMA
            || directory.executable != engine_executable::executable_tables::is_initialized()
            || !pictures_hold_with(icons, directory.pictured, directory.visual.as_deref())
            || !replay.index.iter().any(|block| {
                block.branch == branch
                    && Some((block.first_hour, block.last_hour, block.offset))
                        == directory.last_block
            })
        {
            return Ok(None);
        }
        let read = |section: &str, range| {
            store
                .read(key, section, range)?
                .map(|mut file| {
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
                .transpose()
        };
        Self::restore_from(directory, replay, icons, read).map(Some)
    }
    fn restore_from(
        mut directory: Directory,
        replay: &Replay,
        icons: Option<&IconContent>,
        mut read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
    ) -> io::Result<Self> {
        let branch = directory.branch;
        let processed = replay
            .index
            .iter()
            .find(|block| {
                block.branch == branch
                    && Some((block.first_hour, block.last_hour, block.offset))
                        == directory.last_block
            })
            .cloned()
            .ok_or_else(|| crate::invalid("Production cursor differs from recording prefix"))?;
        let continuation: Continuation = pages::read(
            &read(&state_section(branch, "world"), 0)?
                .ok_or_else(|| crate::invalid("Missing production continuation"))?,
        )?;
        let localisation = icons
            .and_then(|icons| icons.catalogue.as_ref())
            .map(|catalogue| Arc::clone(&catalogue.localisation));
        let mut reader = ReadPages(&mut read);
        let mut facts = private::Store::new(&mut reader, branch);
        let mut load = |name: &str| -> io::Result<PagedCountry> {
            let snapshot = private::get(&mut facts, directory.index, name)?
                .ok_or_else(|| crate::invalid("Missing production country continuation"))?;
            PagedCountry::restore(snapshot, localisation.clone(), recorded_language(replay))
        };
        let absent = load("absent")?;
        let native = continuation
            .native
            .into_iter()
            .map(|key| Ok((key.clone(), load(&native_key(&key))?)))
            .collect::<io::Result<_>>()?;
        let mut aliases = BTreeMap::new();
        for tag in continuation.aliases {
            let snapshot = private::get(&mut facts, directory.index, &tag_key(&tag))?
                .ok_or_else(|| crate::invalid("Missing production alias continuation"))?;
            aliases.insert(
                tag,
                TagCountry::restore(snapshot, localisation.clone(), recorded_language(replay))?,
            );
        }
        directory.coverage.complete &= directory.last_block == last_block(replay, branch);
        Ok(Self {
            processed: Some(processed),
            block_failed: false,
            world: World::restore(continuation.world)?,
            absent,
            native,
            aliases,
            active: continuation.active.into_iter().collect(),
            initial_tags: BTreeSet::new(),
            quiet: None,
            last_input: None,
            evidence_ready: false,
            evidence: Vec::new(),
            evidence_bytes: 0,
            evidence_base_bytes: 0,
            evidence_design_bytes: 0,
            evidence_organisation_bytes: 0,
            evidence_organisation_hour: None,
            alias_last: directory
                .aliases
                .iter()
                .enumerate()
                .map(|(index, alias)| (alias.tag.clone(), index))
                .collect(),
            directory,
        })
    }
    pub fn next_block(&self, replay: &Replay) -> io::Result<usize> {
        if self.block_failed {
            return Err(crate::invalid("Production block did not finish"));
        }
        match &self.processed {
            None => Ok(0),
            Some(previous) => replay
                .block_position(previous)
                .filter(|_| previous.branch == self.directory.branch)
                .map(|position| position + 1)
                .ok_or_else(|| crate::invalid("Production cursor differs from recording prefix")),
        }
    }
}

pub(super) fn country_at(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    tag: &str,
    hour: i32,
) -> io::Result<NativeCountry> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(country) = fixtures::country_at(source, branch, tag, hour)? {
        return Ok(country);
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let directory = directory(&store, &key, branch)?
        .ok_or_else(|| crate::invalid("Production country identity is still being prepared"))?;
    read_country_at(&directory, tag, hour, |section, range| {
        use std::io::Read;
        store
            .read(&key, section, range)?
            .map(|mut file| {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            })
            .transpose()
    })?
    .ok_or_else(|| {
        crate::invalid("The original production country identity is unavailable at its choice")
    })
}

fn read_country_at(
    directory: &Directory,
    tag: &str,
    hour: i32,
    mut read: impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Option<NativeCountry>> {
    for range in 0..directory.pages {
        let bytes = read(&alias_section(directory.branch), range)?
            .ok_or_else(|| crate::invalid("Missing production identity page"))?;
        let aliases: Vec<Alias> = pages::read(&bytes)?;
        if let Some(alias) = aliases
            .into_iter()
            .find(|alias| alias.tag == tag && alias.first_hour <= hour && hour <= alias.last_hour)
        {
            return Ok(Some(alias.country));
        }
    }
    Ok(None)
}

// A page encodes each shared country row once, so a run charges only its own operands.
fn modifier_bytes(line: &replay_data::rows::ProductionLineModifiersRow) -> usize {
    2 * (std::mem::size_of::<breakdown::RawModifierRun>()
        + line.factory_efficiencies.capacity() * std::mem::size_of::<i64>())
}

fn modifier_country_bytes(country: &replay_data::rows::CountryProductionModifiersRow) -> usize {
    let trees = country
        .trees
        .iter()
        .map(|tree| {
            std::mem::size_of_val(tree)
                + tree.failure.capacity()
                + tree.nodes.capacity()
                    * std::mem::size_of::<replay_data::rows::ModifierBreakdownNode>()
                + tree
                    .nodes
                    .iter()
                    .map(|node| node.name.capacity())
                    .sum::<usize>()
        })
        .sum::<usize>();
    2 * (std::mem::size_of_val(country) + trees)
}

fn history_section(branch: u64) -> String {
    format!("production-lines-history-v3-{branch}")
}
#[derive(Serialize, Deserialize)]
struct VariantPage {
    catalogue: VariantCatalogue,
    icons: Vec<(u64, i32, IconChoice, bool)>,
}

struct ReadPages<F>(F);
impl<F: FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>> Pages for ReadPages<F> {
    fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Vec<u8>>> {
        (self.0)(section, range)
    }
    fn write_at(&mut self, _: &str, _: u64, _: &[u8]) -> io::Result<()> {
        Err(crate::invalid("Production query cannot write pages"))
    }
    fn write(&mut self, _: &str, _: &[u8]) -> io::Result<u64> {
        Err(crate::invalid("Production query cannot write pages"))
    }
}
impl PagedCountry {
    fn flush<W: Pages + ?Sized>(
        &mut self,
        facts: &mut private::Store<'_, W>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.state.materialize();
        let page = self.rows.page(&self.state.result, cancelled)?;
        let section = history_section(facts.branch);
        self.head.append(facts, &section, &page)?;
        drop(page);
        self.state.compact(facts)?;
        self.rows.continuation = true;
        Ok(())
    }
}

impl PagedCountry {
    fn dormant(&self, hour: i32) -> Dormant {
        Dormant {
            hour,
            head: self.head,
            reads: self.state.result.unread_output_reads,
            stretches: self.state.result.unreadable_stretches,
        }
    }
    fn wake<W: Pages + ?Sized>(
        &mut self,
        facts: &mut private::Store<'_, W>,
        absent: &PagedCountry,
    ) -> io::Result<()> {
        if let Some(sleep) = self.sleep.take() {
            let saved = SavedLines::save(&absent.state.result, &AtomicBool::new(false))?;
            let branch = facts.branch;
            add_absence(
                &mut self.state.result,
                &sleep,
                absent.head,
                &saved,
                branch,
                &mut |section, range| facts.pages.read_at(section, range),
            )?;
            self.state.last_output_due = absent.state.last_output_due;
        }
        Ok(())
    }
}
fn add_absence(
    result: &mut ScannedLines,
    sleep: &Dormant,
    head: pages::PageLog,
    absent: &SavedLines,
    branch: u64,
    read: &mut impl FnMut(&str, u64) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<()> {
    result.unread_output_reads += absent.unread_output_reads.saturating_sub(sleep.reads);
    result.unreadable_stretches += absent.unreadable_stretches.saturating_sub(sleep.stretches);
    let mut append = |spans: &[super::super::country_groups::HourSpan]| {
        for span in spans {
            if span.last_hour > sleep.hour {
                mark_span(
                    &mut result.unread_output,
                    span.first_hour.max(sleep.hour.saturating_add(1)),
                    span.last_hour,
                );
            }
        }
    };
    let mut cursor = head.cursor(sleep.head.len)?;
    let mut reader = ReadPages(&mut *read);
    let mut facts = private::Store::new(&mut reader, branch);
    while let Some(page) = cursor.next::<pages::Page>(&mut facts, &history_section(branch))? {
        append(&page.lines.unread_output);
    }
    append(&absent.unread_output);
    Ok(())
}

fn load_organisation(
    result: &mut ScannedLines,
    facts: &mut dyn private::Facts,
    history: private::Root,
    id: u64,
) -> io::Result<usize> {
    if !result.organisations.contains_key(&id)
        && let Some(last) = private::get(facts, history, &format!("organisation-{id}"))?
    {
        let bytes = pages::record_bytes(&last);
        result.organisations.insert(id, vec![last]);
        return Ok(bytes);
    }
    Ok(0)
}
fn record_design(
    result: &mut ScannedLines,
    facts: &mut dyn private::Facts,
    history: private::Root,
    input: &HourInput,
    variant: u64,
) -> io::Result<usize> {
    let mut added = 0;
    if !result.designs.contains_key(&variant)
        && let Some(last) = private::get(facts, history, &format!("design-{variant}"))?
    {
        result.designs.insert(variant, vec![last]);
        added += std::mem::size_of::<DesignRead>();
    }
    if let Some(author) = input.designs.get(&variant).and_then(|read| read.author) {
        added += load_organisation(result, facts, history, author)?;
    }
    Ok(added + observe_design(result, input, variant))
}
