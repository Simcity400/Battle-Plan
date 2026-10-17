//! Equipment facts every equipment product shares: units per week as the game's
//! production view writes them, each variant's naming history (named by
//! [`crate::variant_names`]), and an equipment type's domain (land, air or naval)
//! by the engine's own classifiers. Products about equipment (stockpile,
//! production lines, production) read them here, never a copy.

pub mod production;

use crate::{Source, catalogue::Catalogue, metadata::slot_read};
use replay::{
    frame::{DecodedFrame, SharedView, ViewIdentity},
    rows::{
        LexicalTokenRow, TableId, VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use ts_rs::TS;

use crate::variant_names::text;
pub use crate::variant_names::{
    RawVariant, Tags, Texts, VariantOrder, compare_variants, enabled_module_slots, localise_for,
    raw_variant, variant_name, variant_names, variant_order,
};

/// Decimal places of recorded equipment amounts (`engine::equipment_entry`) and
/// of production line cost and speed: fixed5.
pub const EQUIPMENT_DECIMALS: u8 = 5;
const _: () = assert!(engine::equipment_entry::FIXED_POINT_SCALE == 100_000);

/// Units a production line makes per week, fixed5, by the game's weekly formula:
/// its production-rate formatter writes the line's speed (the IC added at each
/// daily production tick) × 7 days × 100000 ÷ its cost per unit, truncated toward
/// zero: `speed × 700000 ÷ cost`.
///
/// The formatter shows that weekly figure only when `speed < cost ≤ 7 × speed`;
/// it picks per day (`speed × 100000 ÷ cost`) when `speed ≥ cost`, and per month
/// or year below a week. Each is the same rate truncated in its own unit, so this
/// weekly figure is exact, and is the unit the widgets sum lines in. With a cost of
/// zero, `speed ≥ cost` always holds and the formatter shows its per-day
/// placeholder whatever the speed, not a rate, so there is no weekly
/// figure to show: None without a positive cost, and None on overflow (where the
/// formatter shows the same placeholder).
pub fn weekly_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 700_000 / i128::from(cost_raw)).ok()
}

/// Units a production line makes per day, fixed5, by the game's per-day formula:
/// the production-rate formatter's daily figure, the line's speed (the IC added at
/// each daily production tick) × 100000 ÷ its cost per unit, truncated toward
/// zero: `speed × 100000 ÷ cost`. It is its own truncation, not the weekly figure
/// divided by seven. None without a positive cost (the formatter's placeholder,
/// not a rate) and None on overflow, as for [`weekly_units_raw`].
pub fn daily_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 100_000 / i128::from(cost_raw)).ok()
}

/// Which arm an equipment type serves, from the engine's own classifiers of its
/// effective category flags (`installed_content::equipment::Database::category`):
/// the plane family is air, the ship family (convoys included) naval, everything
/// else land.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    Land,
    Air,
    Naval,
}

/// Starts the key of an equipment type content does not resolve, before the
/// recorded token's spelling (or the variant's handle without one). Settings can
/// store it (a content-set token), so such a type can be included and chosen like
/// any other; it is kept apart, never merged into another type. A content type key
/// starting with it is warned about ([`marked_keys_warning`]).
pub const UNRESOLVED_MARK: &str = ":";
/// The arm of types content does not resolve, whose arm cannot be told.
pub const UNRESOLVED_ARM: &str = "unresolved";

/// The warning for content type keys that start with [`UNRESOLVED_MARK`], which the
/// widgets would take for unresolved types; none when no key does.
pub fn marked_keys_warning<'a>(keys: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let marked: Vec<&str> = keys
        .into_iter()
        .filter(|key| key.starts_with(UNRESOLVED_MARK))
        .collect();
    (!marked.is_empty()).then(|| {
        format!(
            "Equipment types whose key starts with \"{UNRESOLVED_MARK}\" are not supported: the widgets mark unresolved types that way, so they could be mistaken for one ({}).",
            marked.join(", ")
        )
    })
}

impl Domain {
    pub fn of(category: u64) -> Self {
        if category & engine::cequipment_type::PLANE_MASK != 0 {
            Self::Air
        } else if category & engine::cequipment_type::SHIP_MASK != 0 {
            Self::Naval
        } else {
            Self::Land
        }
    }
}

/// A design's fingerprint: 64-bit FNV-1a, as hex, over its equipment token, own name,
/// suffix and modules (each slot, module and whether it is active, in the recorded
/// order), and upgrades (each definition and level, in recorded order), every text
/// as its lexer spelling. The same design reads the same in every
/// branch; a design renamed reads as another.
pub fn design_fingerprint(
    raw: &RawVariant,
    modules: &[(String, String, bool)],
    upgrades: &[(String, u8)],
) -> Option<String> {
    if ["token", "display name", "suffix", "design"]
        .iter()
        .any(|field| raw.check(field).is_err())
    {
        return None;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for &byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    add(raw.token.as_bytes());
    add(raw.display_name.as_bytes());
    add(raw.suffix.as_bytes());
    add(&[u8::from(raw.append)]);
    for (slot, module, active) in modules {
        add(slot.as_bytes());
        add(module.as_bytes());
        add(&[u8::from(*active)]);
    }
    for (definition, level) in upgrades {
        // A non-UTF-8 tag keeps upgrades apart from module text fields.
        add(&[0xfe]);
        add(definition.as_bytes());
        add(&[*level]);
    }
    Some(format!("{hash:016x}"))
}

/// A token's exact spelling: empty for token 0 (none), refusal when unspelled.
fn known_spelling(token: u32, lexical: &[LexicalTokenRow]) -> Result<String, String> {
    if token == 0 {
        return Ok(String::new());
    }
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("LexicalTokens lacks equipment token {token}"))?;
    text(
        &lexical[index].literal,
        &format!("equipment lexer token {token}"),
    )
}

/// A variant's token spelling through the hour's lexical tokens; empty when unspelled.
pub fn variant_token(definition: &VariantDefinitionRow, lexical: &[LexicalTokenRow]) -> String {
    lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .and_then(|index| text(&lexical[index].literal, "equipment type token").ok())
        .unwrap_or_default()
}

/// One entry of a variant's naming history: its operands from a read of the variant
/// table, or `None` from an hour its names stopped being known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantVersion {
    pub first_hour: i32,
    pub raw: Option<RawVariant>,
}

/// A complete variant read's operands for [`VariantCatalogue::observe_built`],
/// built again only where an input changed since the read before: the
/// countries' keys, links, appearance and the lexical tokens its names and
/// design read, or a definition's own row, modules or upgrades. The dictionary
/// keeps only those spellings and a weak identity, leaving its decoder unshared.
#[derive(Clone, Debug, Default)]
pub(crate) struct VariantReads {
    countries: Option<ViewIdentity>,
    country_at: Option<(u64, i32)>,
    country_dirty: bool,
    #[cfg(any(test, feature = "test-support"))]
    work: crate::prepared::observer_tests::Work,
    keys: Option<crate::metadata::CountryKeys>,
    links: Option<SharedView>,
    appearance: Option<SharedView>,
    lexical: Option<(ViewIdentity, BTreeMap<u32, Option<Vec<u8>>>)>,
    definitions: Option<SharedView>,
    modules: Option<SharedView>,
    upgrades: Option<SharedView>,
    /// Each definition's inputs at the read before.
    inputs: HashMap<u64, VariantInput>,
}

/// A definition's inputs at a read: its row, and its modules and upgrades in
/// recorded order (None where either table was not complete).
#[derive(Clone, Debug)]
struct VariantInput {
    definition: VariantDefinitionRow,
    design: Option<(Vec<VariantModuleRow>, Vec<VariantUpgradeRow>)>,
}

#[cfg(test)]
thread_local! {
    /// Definitions whose operands a scan on this thread built.
    pub(crate) static VARIANT_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Whether `now` holds the rows of the view held: the same allocation, or equal.
fn same_view(held: Option<&SharedView>, now: &SharedView) -> bool {
    held.is_some_and(|held| SharedView::ptr_eq(held, now) || held == now)
}

/// A design's modules and upgrades at a read, in recorded order.
type Design<'a> = (&'a [&'a VariantModuleRow], &'a [&'a VariantUpgradeRow]);

/// Whether a design's inputs are those held.
fn same_design(
    held: Option<&(Vec<VariantModuleRow>, Vec<VariantUpgradeRow>)>,
    now: Option<Design<'_>>,
) -> bool {
    match (held, now) {
        (None, None) => true,
        (Some((modules, upgrades)), Some((now_modules, now_upgrades))) => {
            modules.len() == now_modules.len()
                && upgrades.len() == now_upgrades.len()
                && modules.iter().zip(now_modules).all(|(a, b)| a == *b)
                && upgrades.iter().zip(now_upgrades).all(|(a, b)| a == *b)
        }
        _ => false,
    }
}

impl VariantReads {
    /// Carry the key projection through consecutive country deltas that touch
    /// no identity field. A skipped hour or a snapshot requires comparison.
    fn carry_keys(
        &mut self,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) {
        let continuous = self.country_at.is_some_and(|(branch, hour)| {
            branch == frame.branch && hour.checked_add(1) == Some(frame.hour)
        });
        self.country_at = Some((frame.branch, frame.hour));
        self.country_dirty = self.country_dirty
            || !continuous
            || self.keys.as_ref().is_none_or(|keys| {
                #[cfg(any(test, feature = "test-support"))]
                let before = keys.row_visits();
                let kept = frame.complete::<tables::Countries>().is_ok_and(|rows| {
                    keys.kept_changes(rows.len(), changes.rows(TableId::Countries))
                });
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += keys.row_visits() - before;
                }
                !kept
            });
        if !self.country_dirty {
            self.countries = Some(frame.tables[TableId::Countries.index()].identity());
        }
    }

    pub(crate) fn contains(&self, id: u64) -> bool {
        self.inputs.contains_key(&id)
    }

    /// The operands of `frame`'s complete variant read: every definition's
    /// with `all` (names were forgotten since the read before), else only those
    /// whose inputs changed. A definition's operands are a function of its
    /// inputs and the hour alone ([`read_raw`]), so one not built
    /// holds those it had.
    pub(crate) fn build(&mut self, frame: &DecodedFrame, all: bool) -> Vec<(u64, RawVariant)> {
        let hour = frame.hour;
        let view = |table: TableId| &frame.tables[table.index()];
        // Called only at a complete read of these.
        let (Ok(countries), Ok(links), Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::VariantDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) else {
            return Vec::new();
        };
        let spelling = |token| {
            lexical
                .binary_search_by_key(&token, |row| row.token)
                .ok()
                .map(|index| &lexical[index].literal)
        };
        let same_lexical = self.lexical.as_ref().is_some_and(|(identity, held)| {
            identity.is(view(TableId::LexicalTokens))
                || held.iter().all(|(&token, literal)| {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += 1;
                    }
                    literal.as_ref() == spelling(token)
                })
        });
        let same_countries = !all
            && ((!self.country_dirty
                && self
                    .countries
                    .as_ref()
                    .is_some_and(|held| held.is(view(TableId::Countries))))
                || self.keys.as_ref().is_some_and(|keys| {
                    #[cfg(any(test, feature = "test-support"))]
                    let before = keys.row_visits();
                    let same = keys.same(countries);
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += keys.row_visits() - before;
                    }
                    same
                }));
        let context = !all
            && same_countries
            && same_view(self.links.as_ref(), view(TableId::CountryLinks))
            && same_view(self.appearance.as_ref(), view(TableId::CountryAppearance))
            && same_lexical;
        let designs = same_view(self.modules.as_ref(), view(TableId::VariantModules))
            && same_view(self.upgrades.as_ref(), view(TableId::VariantUpgrades));
        if context
            && designs
            && self
                .definitions
                .as_ref()
                .is_some_and(|held| SharedView::ptr_eq(held, view(TableId::VariantDefinitions)))
        {
            self.countries = Some(view(TableId::Countries).identity());
            self.country_at = Some((frame.branch, frame.hour));
            self.country_dirty = false;
            if let Some((identity, _)) = &mut self.lexical {
                *identity = view(TableId::LexicalTokens).identity();
            }
            return Vec::new();
        }
        let appearance = frame.complete::<tables::CountryAppearance>().ok();
        // This read's modules and upgrades by variant, grouped once when needed.
        type Groups<'a> = Option<(
            VariantRows<'a, VariantModuleRow>,
            VariantRows<'a, VariantUpgradeRow>,
        )>;
        let mut groups: Option<Groups<'_>> = None;
        let mut tags: Option<Tags> = None;
        let mut built = Vec::new();
        let mut inputs = HashMap::with_capacity(definitions.len());
        for definition in definitions {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += 1;
            }
            let id = definition.id;
            let held = self.inputs.remove(&id);
            if context
                && designs
                && held
                    .as_ref()
                    .is_some_and(|held| held.definition == *definition)
            {
                inputs.insert(id, held.expect("unchanged variant inputs"));
                continue;
            }
            // With both design tables unchanged, a held design is this read's.
            let rows: Option<(Vec<&VariantModuleRow>, Vec<&VariantUpgradeRow>)> =
                match (&held, designs) {
                    (Some(held), true) => held.design.as_ref().map(|(modules, upgrades)| {
                        (modules.iter().collect(), upgrades.iter().collect())
                    }),
                    _ => groups
                        .get_or_insert_with(|| {
                            frame
                                .complete::<tables::VariantModules>()
                                .ok()
                                .zip(frame.complete::<tables::VariantUpgrades>().ok())
                                .map(|(modules, upgrades)| {
                                    (modules_by_variant(modules), upgrades_by_variant(upgrades))
                                })
                        })
                        .as_ref()
                        .map(|(modules, upgrades)| {
                            (
                                modules.get(&id).cloned().unwrap_or_default(),
                                upgrades.get(&id).cloned().unwrap_or_default(),
                            )
                        }),
                };
            let design = rows
                .as_ref()
                .map(|(modules, upgrades)| (modules.as_slice(), upgrades.as_slice()));
            let unchanged = context
                && held.as_ref().is_some_and(|held| {
                    held.definition == *definition && same_design(held.design.as_ref(), design)
                });
            if unchanged && let Some(held) = held {
                inputs.insert(id, held);
                continue;
            }
            let tags = tags.get_or_insert_with(|| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits +=
                        countries.len() + links.len() + appearance.map_or(0, <[_]>::len);
                }
                Tags::new(countries, links, appearance)
            });
            #[cfg(test)]
            VARIANT_BUILDS.with(|builds| builds.set(builds.get() + 1));
            built.push((id, read_raw(definition, hour, tags, lexical, design)));
            inputs.insert(
                id,
                VariantInput {
                    definition: definition.clone(),
                    design: design.map(|(modules, upgrades)| {
                        (
                            modules.iter().map(|row| (*row).clone()).collect(),
                            upgrades.iter().map(|row| (*row).clone()).collect(),
                        )
                    }),
                },
            );
        }
        self.inputs = inputs;
        if !context {
            self.keys = Some(crate::metadata::CountryKeys::read(countries));
        }
        self.countries = Some(view(TableId::Countries).identity());
        self.country_at = Some((frame.branch, frame.hour));
        self.country_dirty = false;
        self.links = Some(view(TableId::CountryLinks).clone());
        self.appearance = Some(view(TableId::CountryAppearance).clone());
        let tokens = definitions
            .iter()
            .map(|row| row.equipment_token)
            .chain(
                frame
                    .complete::<tables::VariantModules>()
                    .ok()
                    .into_iter()
                    .flatten()
                    .flat_map(|row| [row.slot, row.definition_token]),
            )
            .chain(
                frame
                    .complete::<tables::VariantUpgrades>()
                    .ok()
                    .into_iter()
                    .flatten()
                    .map(|row| row.definition_token as u32),
            );
        self.lexical = Some((
            view(TableId::LexicalTokens).identity(),
            tokens
                .map(|token| (token, spelling(token).cloned()))
                .collect(),
        ));
        self.definitions = Some(view(TableId::VariantDefinitions).clone());
        self.modules = Some(view(TableId::VariantModules).clone());
        self.upgrades = Some(view(TableId::VariantUpgrades).clone());
        built
    }
}

/// Every variant's naming operands over a branch, with where they are known. The
/// variant table is read in its own daily slot and names hold until the next read,
/// but only while the recording is unbroken: a gap, a due read of the table that is
/// not complete (with the country and token tables its names need), or an hour in
/// between whose variant table is not complete (the recording marked what it carries
/// unavailable) ends what is known, so a variant renamed meanwhile never keeps its old
/// name. Every product that names variants over time builds them here.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VariantCatalogue {
    pub versions: BTreeMap<u64, Vec<VariantVersion>>,
    last_hour: Option<i32>,
    known: BTreeSet<u64>,
    #[serde(skip)]
    reads: VariantReads,
    /// A complete read was applied and nothing was forgotten since, so every
    /// definition not read again holds its operands from that read
    /// ([`Self::observe_built`]). False when restored from a save.
    #[serde(skip)]
    settled: bool,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}

/// What the variant catalogue reads of an hour besides a complete read's rows
/// ([`VariantCatalogue::hour_of`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantHour {
    /// The variant table was not read: names carry while it is complete.
    Carried { complete: bool },
    /// It was read, but it or a table its names need was not complete.
    Incomplete,
    /// A complete read.
    Read,
}

/// One complete read's variant modules or upgrades, each variant's rows in
/// their recorded order.
pub type VariantRows<'a, T> = BTreeMap<u64, Vec<&'a T>>;

/// Each variant's modules, in their recorded order.
pub fn modules_by_variant(rows: &[VariantModuleRow]) -> VariantRows<'_, VariantModuleRow> {
    let mut by_variant: VariantRows<'_, VariantModuleRow> = BTreeMap::new();
    for row in rows {
        by_variant.entry(row.variant).or_default().push(row);
    }
    for rows in by_variant.values_mut() {
        rows.sort_by_key(|row| row.source_ordinal);
    }
    by_variant
}

/// Each variant's upgrades, in their recorded order.
pub fn upgrades_by_variant(rows: &[VariantUpgradeRow]) -> VariantRows<'_, VariantUpgradeRow> {
    let mut by_variant: VariantRows<'_, VariantUpgradeRow> = BTreeMap::new();
    for row in rows {
        by_variant.entry(row.variant).or_default().push(row);
    }
    for rows in by_variant.values_mut() {
        rows.sort_by_key(|row| row.source_ordinal);
    }
    by_variant
}

/// A definition's naming operands at a complete read at `hour`, as the catalogue
/// records them: `design` holds its modules and upgrades in recorded order, or
/// is None where either table is not complete.
pub fn read_raw(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    lexical: &[LexicalTokenRow],
    design: Option<(&[&VariantModuleRow], &[&VariantUpgradeRow])>,
) -> RawVariant {
    let mut raw = raw_variant(definition, hour, tags, lexical);
    raw.design = design.and_then(|(modules, upgrades)| {
        let own: Result<Vec<(String, String, bool)>, String> = modules
            .iter()
            .map(|row| {
                Ok((
                    known_spelling(row.slot, lexical)?,
                    known_spelling(row.definition_token, lexical)?,
                    row.definition_active != 0,
                ))
            })
            .collect();
        let own_upgrades: Result<Vec<(String, u8)>, String> = upgrades
            .iter()
            .map(|row| {
                let definition = known_spelling(row.definition_token as u32, lexical)?;
                if definition.is_empty() {
                    return Err("equipment upgrade definition has no token spelling".into());
                }
                Ok((definition, row.level))
            })
            .collect();
        match own.and_then(|own| own_upgrades.map(|upgrades| (own, upgrades))) {
            Ok((own, upgrades)) => design_fingerprint(&raw, &own, &upgrades),
            Err(why) => {
                raw.refusals.push(("design".into(), why));
                None
            }
        }
    });
    raw
}

impl VariantCatalogue {
    /// Authenticate carried naming without rebuilding any retained versions.
    pub fn advance(&mut self, hour: i32) {
        // A span without country deltas cannot extend their identity proof.
        self.reads.country_dirty |= self.reads.country_at.is_none_or(|(_, at)| at != hour);
        self.last_hour = Some(hour);
    }
    /// Authenticate unchanged naming and carry its country-key proof from this
    /// hour's recorded changes without reading a variant catalogue.
    pub fn advance_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) {
        #[cfg(any(test, feature = "test-support"))]
        let before = self.reads.work.row_visits;
        self.reads.carry_keys(frame, changes);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.row_visits += self.reads.work.row_visits - before;
        }
        self.advance(frame.hour);
    }
    /// A catalogue carrying `versions` as they stand: each variant whose latest
    /// version names it is known, so the next forget ends its names as it would
    /// for the catalogue that read them.
    pub fn seeded(versions: BTreeMap<u64, Vec<VariantVersion>>) -> Self {
        let known = versions
            .iter()
            .filter(|(_, versions)| versions.last().is_some_and(|version| version.raw.is_some()))
            .map(|(&id, _)| id)
            .collect();
        Self {
            versions,
            known,
            ..Self::default()
        }
    }
    /// The frames' tables it reads: select them with the product's own.
    pub const TABLES: [TableId; 7] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
    ];

    /// One decoded frame, in hour order.
    pub fn observe(&mut self, format: replay::format::FormatVersion, frame: &DecodedFrame) {
        let _ = self.observe_versions(format, frame, None);
    }

    pub fn observe_changed(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> Vec<(u64, VariantVersion)> {
        self.observe_versions(format, frame, Some(changes))
    }

    fn observe_versions(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> Vec<(u64, VariantVersion)> {
        let mut changed = Vec::new();
        let hour = frame.hour;
        if let Some(changes) = changes {
            #[cfg(any(test, feature = "test-support"))]
            let before = self.reads.work.row_visits;
            self.reads.carry_keys(frame, changes);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += self.reads.work.row_visits - before;
            }
        }
        // After a gap, what was known ends at the first hour not recorded.
        if let Some(last) = self.last_hour
            && last.checked_add(1) != Some(hour)
        {
            changed.extend(self.forget_versions(last.saturating_add(1)));
        }
        self.last_hour = Some(hour);
        if !changes.map_or_else(
            || {
                slot_read(
                    format,
                    &frame.metrics,
                    TableId::VariantDefinitions,
                    frame.hour,
                )
            },
            |changes| changes.slot_read[TableId::VariantDefinitions.index()],
        ) {
            // Carried only while the table stays complete.
            if frame.complete::<tables::VariantDefinitions>().is_err() {
                changed.extend(self.forget_versions(hour));
            }
            return changed;
        }
        let (Ok(countries), Ok(links), Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::VariantDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) else {
            // A due read that did not land: names are not known from here.
            changed.extend(self.forget_versions(hour));
            return changed;
        };
        if changes.is_some() {
            let cpu = ::replay::cpu::scope_phase("variants:changed_operands");
            #[cfg(any(test, feature = "test-support"))]
            let before = self.reads.work.row_visits;
            let built = self.reads.build(frame, !self.settled);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += self.reads.work.row_visits - before;
            }
            for (id, raw) in built {
                self.apply(id, raw, hour, &mut changed);
            }
            self.settled = true;
            drop(cpu);
            return changed;
        }
        // The whole-table observer is also the independent naming oracle.
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.row_visits += countries.len()
                + links.len()
                + lexical.len()
                + frame
                    .complete::<tables::CountryAppearance>()
                    .map_or(0, |rows| rows.len());
        }
        let tags = Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        // Modules and upgrades share the variant read's slot; a complete table
        // without a variant's rows means it has none, an unavailable one is unknown.
        let modules = frame.complete::<tables::VariantModules>().ok().map(|rows| {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += rows.len();
            }
            modules_by_variant(rows)
        });
        let upgrades = frame
            .complete::<tables::VariantUpgrades>()
            .ok()
            .map(|rows| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += rows.len();
                }
                upgrades_by_variant(rows)
            });
        for definition in definitions {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += 1;
            }
            let design = modules
                .as_ref()
                .zip(upgrades.as_ref())
                .map(|(modules, upgrades)| {
                    (
                        modules.get(&definition.id).map_or(&[][..], Vec::as_slice),
                        upgrades.get(&definition.id).map_or(&[][..], Vec::as_slice),
                    )
                });
            let raw = read_raw(definition, hour, &tags, lexical, design);
            self.apply(definition.id, raw, hour, &mut changed);
        }
        self.settled = true;
        changed
    }

    /// A complete read's operands of one definition: a new version unless its
    /// last one holds the same.
    fn apply(
        &mut self,
        id: u64,
        raw: RawVariant,
        hour: i32,
        changed: &mut Vec<(u64, VariantVersion)>,
    ) {
        let versions = self.versions.entry(id).or_default();
        let same = versions.last().is_some_and(|last| {
            last.raw.as_ref().is_some_and(|last| {
                RawVariant {
                    first_hour: hour,
                    ..last.clone()
                } == raw
            })
        });
        if !same {
            let version = VariantVersion {
                first_hour: hour,
                raw: Some(raw),
            };
            push(versions, version.clone());
            self.known.insert(id);
            changed.push((id, version));
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.emitted += 1;
            }
        }
    }

    /// What [`Self::observe`] reads of an hour's frame besides a complete
    /// read's rows: whether the variant table was read, and which of its tables
    /// were complete.
    pub fn hour_of(format: replay::format::FormatVersion, frame: &DecodedFrame) -> VariantHour {
        Self::hour_read(
            frame,
            slot_read(
                format,
                &frame.metrics,
                TableId::VariantDefinitions,
                frame.hour,
            ),
        )
    }

    pub(crate) fn hour_read(frame: &DecodedFrame, read: bool) -> VariantHour {
        if !read {
            VariantHour::Carried {
                complete: frame.complete::<tables::VariantDefinitions>().is_ok(),
            }
        } else if frame.complete::<tables::Countries>().is_err()
            || frame.complete::<tables::CountryLinks>().is_err()
            || frame.complete::<tables::VariantDefinitions>().is_err()
            || frame.complete::<tables::LexicalTokens>().is_err()
        {
            VariantHour::Incomplete
        } else {
            VariantHour::Read
        }
    }

    /// [`Self::observe`], with a complete read's operands built by `build`
    /// rather than here: it is called only at such a read, with whether names
    /// were forgotten since the catalogue's last complete read (or it never had
    /// one), and returns each definition's operands ([`read_raw`]) in table
    /// order, or, when nothing was forgotten, at least every one that may
    /// differ from what it returned at that read; any other reads as it did
    /// then, so it keeps its version. The versions are those `observe` records.
    pub fn observe_built(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        build: impl FnOnce(bool) -> Vec<(u64, RawVariant)>,
    ) {
        self.observe_hour(frame.hour, Self::hour_of(format, frame), build);
    }

    /// [`Self::observe_built`] of an hour as [`Self::hour_of`] read it.
    pub fn observe_hour(
        &mut self,
        hour: i32,
        read: VariantHour,
        build: impl FnOnce(bool) -> Vec<(u64, RawVariant)>,
    ) {
        if let Some(last) = self.last_hour
            && last.checked_add(1) != Some(hour)
        {
            self.forget_versions(last.saturating_add(1));
        }
        self.last_hour = Some(hour);
        match read {
            VariantHour::Carried { complete: true } => {}
            VariantHour::Carried { complete: false } | VariantHour::Incomplete => {
                self.forget_versions(hour);
            }
            VariantHour::Read => {
                let mut changed = Vec::new();
                for (id, raw) in build(!self.settled) {
                    self.apply(id, raw, hour, &mut changed);
                }
                self.settled = true;
            }
        }
    }

    /// Names are not known from `hour` until the next complete read.
    pub fn forget(&mut self, hour: i32) {
        let _ = self.forget_versions(hour);
    }

    fn forget_versions(&mut self, hour: i32) -> Vec<(u64, VariantVersion)> {
        self.settled = false;
        std::mem::take(&mut self.known)
            .into_iter()
            .map(|id| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.history_visits += 1;
                    self.work.emitted += 1;
                }
                let version = VariantVersion {
                    first_hour: hour,
                    raw: None,
                };
                push(
                    self.versions.get_mut(&id).expect("known variant"),
                    version.clone(),
                );
                (id, version)
            })
            .collect()
    }

    /// A variant's operands as last observed: none where they are not known now.
    pub fn current(&self, id: u64) -> Option<&RawVariant> {
        self.versions.get(&id)?.last()?.raw.as_ref()
    }

    /// A variant's equipment token spelling from its latest known read; empty when no
    /// read named it. The latest nonempty token also classifies earlier numeric history.
    pub fn token(&self, id: u64) -> &str {
        self.versions
            .get(&id)
            .into_iter()
            .flatten()
            .rev()
            .filter_map(|version| version.raw.as_ref())
            .map(|raw| raw.token.as_str())
            .find(|token| !token.is_empty())
            .unwrap_or_default()
    }

    /// Invalid strings refuse only names/design identity, never numeric rows.
    pub fn warnings(&self) -> Vec<String> {
        let mut refusals = crate::catalogue::Refusals::default();
        for (id, versions) in &self.versions {
            for raw in versions.iter().filter_map(|version| version.raw.as_ref()) {
                for (_, why) in &raw.refusals {
                    refusals.note(why.as_str(), id.to_string());
                }
            }
        }
        refusals.diagnostic_lines("equipment variants with unknown text/design operands")
    }
}

/// Appends a version, never two at one hour: a later one at the same hour replaces it.
fn push(versions: &mut Vec<VariantVersion>, version: VariantVersion) {
    match versions.last_mut() {
        Some(last) if last.first_hour >= version.first_hour => *last = version,
        _ => versions.push(version),
    }
}

/// A variant's names as recorded at a read of the variant table; a new entry
/// wherever a read changes them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockVariantName {
    /// The read these names are from; they hold until the next entry.
    pub first_hour: i32,
    /// The variant's display name; null when content does not name it, or when
    /// not known.
    pub name: Option<String>,
    /// False from an hour the variant's names stopped being known (a gap, or a due
    /// read of the variant table that did not land) until the next entry.
    pub known: bool,
    /// The design's fingerprint as read then (`equipment::design_fingerprint`: its
    /// name, modules and upgrade levels); null where not known. A saved variant
    /// choice carries it, so a later branch's other design under the same handle
    /// is never taken for it.
    pub design: Option<String>,
}

impl VariantCatalogue {
    /// A variant's names as each read recorded them, dated by the read, with where
    /// they stopped being known: a name is never carried back before the read that
    /// recorded it, nor over hours its names were not known.
    pub fn names(&self, id: u64, texts: Option<&dyn Texts>) -> Vec<StockVariantName> {
        let mut names: Vec<StockVariantName> = Vec::new();
        for version in self.versions.get(&id).into_iter().flatten() {
            let (name, known, design) = match &version.raw {
                Some(raw) => (variant_names(raw, texts).0, true, raw.design.clone()),
                None => (None, false, None),
            };
            if names.last().is_some_and(|last| {
                last.name == name && last.known == known && last.design == design
            }) {
                continue;
            }
            names.push(StockVariantName {
                first_hour: version.first_hour,
                name,
                known,
                design,
            });
        }
        names
    }
}

/// A variant's type resolved through content: its archetype key, database place and domain.
pub fn archetype_of(
    types: &installed_content::equipment::Database,
    token: &str,
) -> Result<(String, usize, Domain), String> {
    let kind = types
        .named(token)
        .ok_or_else(|| format!("{token} is not authored equipment"))?;
    let ordinal = kind
        .archetype
        .ok_or_else(|| format!("equipment {token} has no known archetype"))?;
    let archetype = &types.equipment[ordinal];
    Ok((
        archetype
            .name_str()
            .map_err(|error| error.to_string())?
            .to_owned(),
        ordinal,
        Domain::of(types.category(archetype)),
    ))
}

/// Native-reader diagnostics, with their file and definition names.
pub fn equipment_warnings(types: &installed_content::equipment::Database) -> Vec<String> {
    types.misreads.clone()
}

/// The authored equipment types as read, or why they could not be.
type Units = Result<Arc<installed_content::equipment::Database>, String>;

/// The recorded content stack's equipment types for a catalogue, read on first use.
pub fn units(_source: &Source, catalogue: &Arc<Catalogue>) -> Units {
    units_of(catalogue)
}

/// [`units`] for a consumer that holds only the catalogue.
pub fn units_of(catalogue: &Catalogue) -> Units {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content::equipment::Database>>()
        .get_or_init(|| installed_content::equipment::Database::load_complete(&catalogue.content.layers))
        .map_err(|error| error.to_string())
}

/// How many times the catalogue's content stack read its equipment types.
#[cfg(any(test, feature = "test-support"))]
pub fn unit_loads(catalogue: &Catalogue) -> usize {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content::equipment::Database>>()
        .loads()
}

#[cfg(test)]
mod tests;
