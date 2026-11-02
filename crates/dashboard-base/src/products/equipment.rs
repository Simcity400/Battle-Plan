//! Equipment facts every equipment product shares: units per week as the game's
//! production view writes them, each variant's naming history (named by
//! [`crate::variant_names`]), and an equipment type's domain (land, air or naval)
//! by the engine's own classifiers. Products about equipment (stockpile,
//! production lines, production) read them here, never a copy.

pub mod columnar;
pub mod designs;
pub mod production;

use crate::{
    Source,
    catalogue::Catalogue,
    metadata::slot_read,
    prepared::prepass::{Held, HeldBytes},
};
use replay_data::{
    rows::LexicalTokenRow, rows::TableId, rows::VariantDefinitionRow, rows::VariantModuleRow,
    rows::VariantUpgradeRow, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::ViewIdentity};
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

/// Decimal places of recorded equipment amounts (`engine_native::equipment_entry`) and
/// of production line cost and speed: fixed5.
pub const EQUIPMENT_DECIMALS: u8 = 5;
const _: () = assert!(engine_native::equipment_entry::FIXED_POINT_SCALE == 100_000);

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
/// effective category flags (`installed_content_core::equipment::Database::category`):
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
        if category & engine_native::cequipment_type::PLANE_MASK != 0 {
            Self::Air
        } else if category & engine_native::cequipment_type::SHIP_MASK != 0 {
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

/// The lexical tokens' spellings a variant read held, with the identity of the view they came from.
type LexicalSpellings = (ViewIdentity, BTreeMap<u32, Option<Vec<u8>>>);

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
    // The view identity and optional spellings form one invalidation witness.
    lexical: Option<LexicalSpellings>,
    definitions: Option<SharedView>,
    modules: Option<SharedView>,
    upgrades: Option<SharedView>,
    /// Each definition's inputs at the read before.
    inputs: HashMap<u64, VariantInput>,
    /// Recorded invalidations since the last supplying read, including hours
    /// on which the catalogue only carried names. None requires a full read.
    pending: [Option<BTreeSet<u128>>; 4],
    token_variants: BTreeMap<u32, BTreeSet<u64>>,
    appearance_dirty: bool,
    country_tags: BTreeMap<u32, replay_data::rows::CountryRow>,
    /// Each slot's appearance row (country, original tag); `None` where the table was incomplete.
    original_tags: Option<BTreeMap<u32, replay_data::rows::CountryAppearanceRow>>,
    tag_links: BTreeMap<u32, BTreeSet<u64>>,
    tag_slots: BTreeMap<u32, BTreeSet<u64>>,
    original_slots: BTreeMap<u32, BTreeSet<u64>>,
}

/// A definition's inputs at a read: its row, and its modules and upgrades in
/// recorded order (None where either table was not complete).
#[derive(Clone, Debug)]
struct VariantInput {
    definition: VariantDefinitionRow,
    design: Option<(Vec<VariantModuleRow>, Vec<VariantUpgradeRow>)>,
}

impl VariantInput {
    fn naming_link(&self) -> u32 {
        if (self.definition.origin_link as i32) >= 1 {
            self.definition.origin_link
        } else {
            self.definition.creator_link
        }
    }
    fn tokens(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.definition.equipment_token)
            .chain(
                self.design
                    .iter()
                    .flat_map(|(modules, _)| modules.iter())
                    .flat_map(|row| [row.slot, row.definition_token]),
            )
            .chain(
                self.design
                    .iter()
                    .flat_map(|(_, upgrades)| upgrades.iter())
                    .map(|row| row.definition_token as u32),
            )
    }
}

impl Held for VariantInput {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.definition).heap(&self.design);
    }
}

impl Held for VariantReads {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.keys)
            .heap(&self.links)
            .heap(&self.appearance)
            .heap(&self.definitions)
            .heap(&self.modules)
            .heap(&self.upgrades)
            .heap(&self.inputs)
            .heap(&self.pending)
            .heap(&self.token_variants)
            .heap(&self.country_tags)
            .heap(&self.original_tags)
            .heap(&self.tag_links)
            .heap(&self.tag_slots)
            .heap(&self.original_slots);
        if let Some((_, spellings)) = &self.lexical {
            bytes.heap(spellings);
        }
    }
}

impl Held for RawVariant {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.token)
            .heap(&self.display_name)
            .heap(&self.suffix)
            .heap(&self.name_tag)
            .heap(&self.type_tag)
            .heap(&self.design)
            .heap(&self.refusals);
    }
}

impl Held for VariantVersion {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.heap(&self.raw);
    }
}

/// Its versions and the operands its changed reads keep, each shared view once.
impl Held for VariantCatalogue {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.versions)
            .heap(&self.known)
            .heap(&self.reads);
    }
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

/// A repeated refused operand has no rows to compare. Complete tables retain
/// their identity proof; their recorded deltas select any changed designs.
fn same_design_view(held: Option<&SharedView>, now: &SharedView) -> bool {
    held.is_some_and(|held| {
        SharedView::ptr_eq(held, now)
            || matches!(
                (&**held, &**now),
                (
                    replay_frame::frame::TableView::Unavailable { .. },
                    replay_frame::frame::TableView::Unavailable { .. }
                )
            ) && held == now
    })
}

fn complete_rows<T: replay_data::rows::TableSchema>(view: &SharedView) -> Option<&[T::Row]> {
    match &**view {
        replay_frame::frame::TableView::Complete(rows) => rows.rows::<T>(),
        _ => None,
    }
}

/// Each appearance row's country and original tag by slot, as [`Tags::new`] reads them.
fn original_rows(
    rows: &[replay_data::rows::CountryAppearanceRow],
    links: &[replay_data::rows::CountryLinkRow],
) -> BTreeMap<u32, replay_data::rows::CountryAppearanceRow> {
    rows.iter()
        .filter_map(|row| {
            Some((
                crate::metadata::link_slot(links, row.country)?,
                replay_data::rows::CountryAppearanceRow {
                    country: row.country,
                    name_tag: row.name_tag.clone(),
                    ..Default::default()
                },
            ))
        })
        .collect()
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

/// Remove one definition's membership, dropping an entry with no users left.
fn remove_dependency(dependencies: &mut BTreeMap<u32, BTreeSet<u64>>, key: u32, id: u64) -> bool {
    let empty = dependencies.get_mut(&key).is_some_and(|variants| {
        variants.remove(&id);
        variants.is_empty()
    });
    if empty {
        dependencies.remove(&key);
    }
    empty
}

impl VariantReads {
    fn remove_dependencies(&mut self, input: &VariantInput) {
        let id = input.definition.id;
        for token in input.tokens() {
            if remove_dependency(&mut self.token_variants, token, id)
                && let Some((_, spellings)) = &mut self.lexical
            {
                spellings.remove(&token);
            }
        }
        let raw = input.naming_link();
        remove_dependency(&mut self.tag_links, raw, id);
        if let Some(slot) = self
            .links
            .as_ref()
            .and_then(complete_rows::<tables::CountryLinks>)
            .and_then(|links| crate::metadata::link_slot(links, raw))
        {
            remove_dependency(&mut self.tag_slots, slot, id);
            remove_dependency(&mut self.original_slots, slot, id);
        }
    }

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
        for (pending, table) in self.pending.iter_mut().zip([
            TableId::VariantDefinitions,
            TableId::VariantModules,
            TableId::VariantUpgrades,
            TableId::LexicalTokens,
        ]) {
            if !continuous {
                *pending = None;
            } else if let Some(held) = pending {
                if let Some((changed, deleted)) = changes.rows(table) {
                    held.extend(
                        changed
                            .iter()
                            .map(|row| row.key)
                            .chain(deleted.iter().copied()),
                    );
                } else {
                    *pending = None;
                }
            }
        }
        if !continuous || changes.rows_changed[TableId::CountryAppearance.index()] {
            static FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
                crate::prepared::frame_changes::field_mask(
                    TableId::CountryAppearance,
                    &["name_tag"],
                )
            });
            self.appearance_dirty |= !continuous
                || changes
                    .rows(TableId::CountryAppearance)
                    .is_none_or(|(changed, deleted)| {
                        !deleted.is_empty() || changed.iter().any(|row| row.mask & *FIELDS != 0)
                    });
        }
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
        let cpu = ::replay_frame::cpu::scope_phase("variants:context");
        let hour = frame.hour;
        let tracked = self.country_at == Some((frame.branch, hour));
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
                || self.pending[3]
                    .as_ref()
                    .filter(|_| tracked)
                    .map_or_else(
                        || held.keys().copied().collect::<Vec<_>>(),
                        |pending| pending.iter().map(|key| *key as u32).collect(),
                    )
                    .into_iter()
                    .all(|token| {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.row_visits += 1;
                        }
                        held.get(&token)
                            .is_none_or(|literal| literal.as_ref() == spelling(token))
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
            && ((tracked && !self.appearance_dirty && self.appearance.is_some())
                || same_view(self.appearance.as_ref(), view(TableId::CountryAppearance))
                || self.appearance.as_ref().is_some_and(|held| {
                    let before = complete_rows::<tables::CountryAppearance>(held);
                    let now = frame.complete::<tables::CountryAppearance>().ok();
                    before.zip(now).is_some_and(|(before, now)| {
                        before
                            .iter()
                            .map(|row| (row.country, &row.name_tag))
                            .eq(now.iter().map(|row| (row.country, &row.name_tag)))
                    })
                }));
        let designs = same_design_view(self.modules.as_ref(), view(TableId::VariantModules))
            && same_design_view(self.upgrades.as_ref(), view(TableId::VariantUpgrades));
        if context
            && same_lexical
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
            self.pending = std::array::from_fn(|_| Some(BTreeSet::new()));
            self.appearance = Some(view(TableId::CountryAppearance).clone());
            self.appearance_dirty = false;
            return Vec::new();
        }
        let appearance = frame.complete::<tables::CountryAppearance>().ok();
        // Exact deltas select definitions by their own key, design owner and
        // used spelling. Invalidations survive every skipped supplying read.
        let mut context_ids = BTreeSet::new();
        if !context && !all && self.definitions.is_some() {
            let next_tags: BTreeMap<_, _> = countries
                .iter()
                .map(|row| {
                    (
                        row.canonical_slot,
                        replay_data::rows::CountryRow {
                            canonical_slot: row.canonical_slot,
                            tag: row.tag.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let next_original = appearance.map(|rows| original_rows(rows, links));
            let changed_slots: BTreeSet<_> = self
                .country_tags
                .keys()
                .chain(next_tags.keys())
                .copied()
                .filter(|slot| self.country_tags.get(slot) != next_tags.get(slot))
                .collect();
            if changed_slots.contains(&0) {
                context_ids.extend(self.inputs.keys().copied());
            } else {
                for slot in changed_slots {
                    context_ids.extend(self.tag_slots.get(&slot).into_iter().flatten().copied());
                }
            }
            match (&self.original_tags, &next_original) {
                (Some(before), Some(now)) => {
                    for slot in before.keys().chain(now.keys()) {
                        if before.get(slot).map(|row| &row.name_tag)
                            != now.get(slot).map(|row| &row.name_tag)
                        {
                            context_ids.extend(
                                self.original_slots.get(slot).into_iter().flatten().copied(),
                            );
                        }
                    }
                }
                (None, None) => {}
                // Whether creators' original tags are known at all changed.
                _ => context_ids.extend(self.original_slots.values().flatten().copied()),
            }
            if let Some(before) = self
                .links
                .as_ref()
                .and_then(complete_rows::<tables::CountryLinks>)
            {
                for (raw, variants) in &self.tag_links {
                    if crate::metadata::link_slot(before, *raw)
                        != crate::metadata::link_slot(links, *raw)
                    {
                        context_ids.extend(variants);
                    }
                }
            }
            self.country_tags = next_tags;
            self.original_tags = next_original;
        }
        let selected = (!all && self.definitions.is_some())
            .then(|| {
                let mut ids: BTreeSet<u64> = if let Some(pending) =
                    self.pending[0].as_ref().filter(|_| tracked)
                {
                    pending.iter().map(|key| *key as u64).collect()
                } else {
                    let held =
                        complete_rows::<tables::VariantDefinitions>(self.definitions.as_ref()?)?;
                    crate::prepared::frame_changes::differing_keys(held, definitions)
                        .into_iter()
                        .map(|key| key as u64)
                        .collect()
                };
                for index in 1..3 {
                    let (held, table) = if index == 1 {
                        (self.modules.as_ref(), TableId::VariantModules)
                    } else {
                        (self.upgrades.as_ref(), TableId::VariantUpgrades)
                    };
                    if same_design_view(held, view(table)) {
                        continue;
                    }
                    let pending = self.pending[index]
                        .as_ref()
                        .filter(|_| tracked)
                        .cloned()
                        .or_else(|| {
                            if index == 1 {
                                let held = complete_rows::<tables::VariantModules>(
                                    self.modules.as_ref()?,
                                )?;
                                let now = frame.complete::<tables::VariantModules>().ok()?;
                                Some(crate::prepared::frame_changes::differing_keys(held, now))
                            } else {
                                let held = complete_rows::<tables::VariantUpgrades>(
                                    self.upgrades.as_ref()?,
                                )?;
                                let now = frame.complete::<tables::VariantUpgrades>().ok()?;
                                Some(crate::prepared::frame_changes::differing_keys(held, now))
                            }
                        })?;
                    ids.extend(pending.iter().map(|key| (*key >> 32) as u64));
                }
                if !same_lexical {
                    for (&token, literal) in &self.lexical.as_ref()?.1 {
                        if literal.as_ref() != spelling(token)
                            && let Some(variants) = self.token_variants.get(&token)
                        {
                            ids.extend(variants);
                        }
                    }
                }
                ids.extend(context_ids);
                Some(ids)
            })
            .flatten();
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("variants:definition_designs");
        let mut built = Vec::new();
        let selected_rows: Vec<_> = if let Some(ids) = &selected {
            ids.iter()
                .filter_map(|id| {
                    let found = definitions.binary_search_by_key(id, |row| row.id).ok();
                    if found.is_none()
                        && let Some(input) = self.inputs.remove(id)
                    {
                        self.remove_dependencies(&input);
                    }
                    found.map(|at| &definitions[at])
                })
                .collect()
        } else {
            self.inputs
                .retain(|id, _| definitions.binary_search_by_key(id, |row| row.id).is_ok());
            definitions.iter().collect()
        };
        let mut tags = if selected.is_some() {
            let slots: BTreeSet<_> = std::iter::once(0)
                .chain(selected_rows.iter().filter_map(|row| {
                    let raw = if (row.origin_link as i32) >= 1 {
                        row.origin_link
                    } else {
                        row.creator_link
                    };
                    crate::metadata::link_slot(links, raw)
                }))
                .collect();
            let countries: Vec<_> = slots
                .iter()
                .filter_map(|slot| self.country_tags.get(slot).cloned())
                .collect();
            let appearances: Option<Vec<_>> = self.original_tags.as_ref().map(|rows| {
                slots
                    .iter()
                    .filter_map(|slot| rows.get(slot).cloned())
                    .collect()
            });
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits +=
                    countries.len() + appearances.as_ref().map_or(0, Vec::len);
            }
            Some(Tags::new(&countries, links, appearances.as_deref()))
        } else {
            None
        };
        for definition in selected_rows {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += 1;
            }
            let id = definition.id;
            let held = self.inputs.remove(&id);
            let _variant_cpu = ::replay_frame::cpu::scope_phase("variants:variant_operands");
            if let Some(held) = &held {
                self.remove_dependencies(held);
            }
            if context
                && same_lexical
                && designs
                && held
                    .as_ref()
                    .is_some_and(|held| held.definition == *definition)
            {
                self.inputs
                    .insert(id, held.expect("unchanged variant inputs"));
                continue;
            }
            // With both design tables unchanged, a held design is this read's.
            let rows: Option<(Vec<&VariantModuleRow>, Vec<&VariantUpgradeRow>)> =
                match (&held, designs) {
                    (Some(held), true) => held.design.as_ref().map(|(modules, upgrades)| {
                        (modules.iter().collect(), upgrades.iter().collect())
                    }),
                    _ => frame
                        .complete::<tables::VariantModules>()
                        .ok()
                        .zip(frame.complete::<tables::VariantUpgrades>().ok())
                        .map(|(modules, upgrades)| {
                            (
                                modules[modules.partition_point(|row| row.variant < id)
                                    ..modules.partition_point(|row| row.variant <= id)]
                                    .iter()
                                    .collect(),
                                upgrades[upgrades.partition_point(|row| row.variant < id)
                                    ..upgrades.partition_point(|row| row.variant <= id)]
                                    .iter()
                                    .collect(),
                            )
                        }),
                };
            let design = rows
                .as_ref()
                .map(|(modules, upgrades)| (modules.as_slice(), upgrades.as_slice()));
            let unchanged = context
                && same_lexical
                && held.as_ref().is_some_and(|held| {
                    held.definition == *definition && same_design(held.design.as_ref(), design)
                });
            if unchanged && let Some(held) = held {
                self.inputs.insert(id, held);
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
            self.inputs.insert(
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
        if !context {
            self.keys = Some(crate::metadata::CountryKeys::read(countries));
            self.country_tags = countries
                .iter()
                .map(|row| {
                    (
                        row.canonical_slot,
                        replay_data::rows::CountryRow {
                            canonical_slot: row.canonical_slot,
                            tag: row.tag.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            self.original_tags = appearance.map(|rows| original_rows(rows, links));
        }
        self.countries = Some(view(TableId::Countries).identity());
        self.country_at = Some((frame.branch, frame.hour));
        self.country_dirty = false;
        self.links = Some(view(TableId::CountryLinks).clone());
        self.appearance = Some(view(TableId::CountryAppearance).clone());
        self.appearance_dirty = false;
        let cpu = ::replay_frame::cpu::scope_phase("variants:spelling_dependencies");
        if selected.is_none() {
            self.token_variants.clear();
            self.tag_links.clear();
            self.tag_slots.clear();
            self.original_slots.clear();
            self.lexical = Some((view(TableId::LexicalTokens).identity(), BTreeMap::new()));
        }
        let (identity, spellings) = self.lexical.as_mut().expect("variant spelling read");
        *identity = view(TableId::LexicalTokens).identity();
        let ids: Vec<_> = selected.map_or_else(
            || self.inputs.keys().copied().collect(),
            |ids| ids.into_iter().collect(),
        );
        for id in ids {
            if let Some(input) = self.inputs.get(&id) {
                let raw = input.naming_link();
                self.tag_links.entry(raw).or_default().insert(id);
                if let Some(slot) = crate::metadata::link_slot(links, raw) {
                    self.tag_slots.entry(slot).or_default().insert(id);
                    if (input.definition.origin_link as i32) < 1 {
                        self.original_slots.entry(slot).or_default().insert(id);
                    }
                }
                for token in input.tokens() {
                    self.token_variants.entry(token).or_default().insert(id);
                    spellings.insert(token, spelling(token).cloned());
                }
            }
        }
        drop(cpu);
        self.definitions = Some(view(TableId::VariantDefinitions).clone());
        self.modules = Some(view(TableId::VariantModules).clone());
        self.upgrades = Some(view(TableId::VariantUpgrades).clone());
        self.pending = std::array::from_fn(|_| Some(BTreeSet::new()));
        built
    }
}

/// What a design is named from at one supplying read of its definitions: [`raw_variant`]'s
/// definitions, tokens and [`Tags::new`]'s countries.
pub const DESIGN_TABLES: [TableId; 5] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
];

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
    /// The refusals of versions [`Self::keep_current`] dropped.
    #[serde(default, skip_serializing_if = "crate::catalogue::Refusals::is_empty")]
    refused: crate::catalogue::Refusals,
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
        if self.reads.country_dirty {
            self.reads.pending = std::array::from_fn(|_| None);
            self.reads.appearance_dirty = true;
        }
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
    /// [`Self::seeded`] whose reads last observed `hour`, as a continuation restores it: a
    /// gap after `hour` forgets what it knows, as it would have.
    pub fn resumed(versions: BTreeMap<u64, Vec<VariantVersion>>, hour: Option<i32>) -> Self {
        Self {
            last_hour: hour,
            ..Self::seeded(versions)
        }
    }
    /// The last hour it observed.
    pub fn last_hour(&self) -> Option<i32> {
        self.last_hour
    }
    /// The frames' tables it reads: select them with the product's own.
    pub const TABLES: [TableId; 8] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::VariantChanges,
    ];

    /// One decoded frame, in hour order.
    pub fn observe(&mut self, format: replay_data::format::FormatVersion, frame: &DecodedFrame) {
        let _ = self.observe_versions(format, frame, None);
    }

    pub fn observe_changed(
        &mut self,
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> Vec<(u64, VariantVersion)> {
        self.observe_versions(format, frame, Some(changes))
    }

    fn observe_versions(
        &mut self,
        format: replay_data::format::FormatVersion,
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
        if !design_changed(frame)
            && !changes.map_or_else(
                || {
                    slot_read(
                        format,
                        &frame.metrics,
                        TableId::VariantDefinitions,
                        frame.hour,
                    )
                },
                |changes| changes.slot_read[TableId::VariantDefinitions.index()],
            )
        {
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
            let cpu = ::replay_frame::cpu::scope_phase("variants:changed_operands");
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
    pub fn hour_of(
        format: replay_data::format::FormatVersion,
        frame: &DecodedFrame,
    ) -> VariantHour {
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
        if !design_changed(frame) && !read {
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
        format: replay_data::format::FormatVersion,
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
        let _ = self.observe_hour_changed(hour, read, build);
    }

    /// [`Self::observe_hour`], returning the versions it recorded, forgotten names
    /// included, as [`Self::observe_changed`] returns them.
    pub fn observe_hour_changed(
        &mut self,
        hour: i32,
        read: VariantHour,
        build: impl FnOnce(bool) -> Vec<(u64, RawVariant)>,
    ) -> Vec<(u64, VariantVersion)> {
        let mut changed = Vec::new();
        if let Some(last) = self.last_hour
            && last.checked_add(1) != Some(hour)
        {
            changed.extend(self.forget_versions(last.saturating_add(1)));
        }
        self.last_hour = Some(hour);
        match read {
            VariantHour::Carried { complete: true } => {}
            VariantHour::Carried { complete: false } | VariantHour::Incomplete => {
                changed.extend(self.forget_versions(hour));
            }
            VariantHour::Read => {
                for (id, raw) in build(!self.settled) {
                    self.apply(id, raw, hour, &mut changed);
                }
                self.settled = true;
            }
        }
        changed
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

    /// Drops each variant's versions before its current one, keeping their
    /// refusals for [`Self::warnings`]: for products that name a variant only
    /// as it is when they read it.
    pub fn keep_current(&mut self) {
        for (id, versions) in &mut self.versions {
            let earlier = versions.len().saturating_sub(1);
            for version in versions.drain(..earlier) {
                for (_, why) in version.raw.iter().flat_map(|raw| &raw.refusals) {
                    self.refused.note(why.as_str(), id.to_string());
                }
            }
        }
    }

    /// Invalid strings refuse only names/design identity, never numeric rows.
    pub fn warnings(&self) -> Vec<String> {
        let mut refusals = self.refused.clone();
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
    types: &installed_content_core::equipment::Database,
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
pub fn equipment_warnings(types: &installed_content_core::equipment::Database) -> Vec<String> {
    types.misreads.clone()
}

/// The authored equipment types as read, or why they could not be.
type Units = Result<Arc<installed_content_core::equipment::Database>, String>;

/// The recorded content stack's equipment types for a catalogue, read on first use.
pub fn units(_source: &Source, catalogue: &Arc<Catalogue>) -> Units {
    units_of(catalogue)
}

/// [`units`] for a consumer that holds only the catalogue.
pub fn units_of(catalogue: &Catalogue) -> Units {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content_core::equipment::Database>>()
        .get_or_init(|| {
            installed_content_core::equipment::Database::load_complete(&catalogue.content.layers)
        })
        .map_err(|error| error.to_string())
}

/// How many times the catalogue's content stack read its equipment types.
#[cfg(any(test, feature = "test-support"))]
pub fn unit_loads(catalogue: &Catalogue) -> usize {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content_core::equipment::Database>>()
        .loads()
}

#[cfg(test)]
mod tests;

/// A writer-observed state update is current even between native census slots.
pub fn design_changed(frame: &DecodedFrame) -> bool {
    frame
        .complete::<tables::VariantChanges>()
        .is_ok_and(|rows| !rows.is_empty())
}

impl product_codec::Product for VariantCatalogue {
    const MIN_BYTES: usize = 17;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        product_codec::Product::encode(&self.versions, out)?;
        product_codec::Product::encode(&self.last_hour, out)?;
        product_codec::Product::encode(&self.known, out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        Ok(Self {
            versions: product_codec::Product::decode(input)?,
            last_hour: product_codec::Product::decode(input)?,
            known: product_codec::Product::decode(input)?,
            ..Self::default()
        })
    }
    fn schema(s: &mut product_codec::Schema) {
        s.field("variant-catalogue/persisted/v1");
        <BTreeMap<u64, Vec<VariantVersion>> as product_codec::Product>::schema(s);
    }
}
impl product_codec::Product for VariantVersion {
    const MIN_BYTES: usize = 5;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        self.first_hour.encode(out)?;
        self.raw.is_some().encode(out)?;
        if let Some(raw) = &self.raw {
            raw.id.encode(out)?;
            raw.first_hour.encode(out)?;
            raw.token.encode(out)?;
            raw.display_name.encode(out)?;
            raw.suffix.encode(out)?;
            raw.append.encode(out)?;
            raw.name_tag.encode(out)?;
            raw.type_tag.encode(out)?;
            raw.design.encode(out)?;
            raw.refusals.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        let first_hour = i32::decode(input)?;
        let raw = if bool::decode(input)? {
            let raw = RawVariant {
                id: u64::decode(input)?,
                first_hour: i32::decode(input)?,
                token: String::decode(input)?,
                display_name: String::decode(input)?,
                suffix: String::decode(input)?,
                append: bool::decode(input)?,
                name_tag: Option::<String>::decode(input)?,
                type_tag: Option::<String>::decode(input)?,
                design: Option::<String>::decode(input)?,
                refusals: Vec::<(String, String)>::decode(input)?,
            };
            if raw.refusals.iter().any(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "token" | "display name" | "suffix" | "name tag" | "type tag" | "design"
                )
            }) {
                return Err(product_codec::invalid("Unknown equipment variant operand"));
            }
            Some(raw)
        } else {
            None
        };
        Ok(Self { first_hour, raw })
    }
    fn schema(s: &mut product_codec::Schema) {
        s.field("variant-version/raw-persisted/v1");
    }
}
