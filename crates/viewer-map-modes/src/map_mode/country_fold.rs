use super::style_fold::Binding;
use super::*;
use crate::political::relations::Relations;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{retained::Retained, value_index::ValueIndex};
use replay_data::rows::TableSchema;
use std::sync::Weak;

type Paint = ValueIndex<u32, Style>;
type Slots = ValueIndex<u32, ()>;
type Bits = [u32; 4];
#[derive(Default)]
pub(super) struct CountryLayers {
    base: Option<Base>,
    country: Paint,
    country_colors: ValueIndex<u32, Bits>,
    factions: Option<Factions>,
    players: Option<Players>,
    ideology: Option<Ideology>,
}
struct Base {
    binding: Binding,
    links: RowFold<u32>,
    countries: RowFold<u32>,
    by_canonical_raw: ValueIndex<(u32, u32), ()>,
    raw: ValueIndex<u32, u32>,
    appearance: RowFold<Bits>,
    relations: Relations,
    slots: Slots,
    source_of: ValueIndex<u32, u32>,
    by_source_target: ValueIndex<(u32, u32), ()>,
    colors: ValueIndex<u32, Bits>,
    unknown: Slots,
    missing: usize,
}
impl Base {
    fn read(
        inputs: &Inputs,
        bank: Weak<MapData>,
        old: Option<&Self>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        let frame = inputs.frame;
        let links = RowFold::read(
            frame,
            TableId::CountryLinks,
            core::<tables::CountryLinks>(frame)?,
            fields(TableId::CountryLinks, &["canonical_slot"]),
            old.map(|old| &old.links),
            |row| Ok(row.canonical_slot),
        )?;
        let countries = RowFold::read(
            frame,
            TableId::Countries,
            core::<tables::Countries>(frame)?,
            fields(TableId::Countries, &["canonical_slot"]),
            old.map(|old| &old.countries),
            |row| Ok(row.canonical_slot),
        )?;
        let (_, slots) = membership::countries(inputs)?;
        let slots = slots.index().clone();
        let relations = Relations::read(inputs.map, frame, &links, old.map(|old| &old.relations))?;
        warnings.extend(relations.warnings.iter().cloned());
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        let appearance = RowFold::read(
            frame,
            TableId::CountryAppearance,
            appearance.rows(),
            fields(
                TableId::CountryAppearance,
                &["red", "green", "blue", "alpha"],
            ),
            old.map(|old| &old.appearance),
            |row| Ok([row.red, row.green, row.blue, row.alpha]),
        )?;
        let mut by_canonical_raw =
            old.map_or_else(ValueIndex::default, |old| old.by_canonical_raw.clone());
        let mut raw = old.map_or_else(ValueIndex::default, |old| old.raw.clone());
        let mut changed_sources = ValueIndex::default();
        for &id in countries.edits.iter() {
            if let Some(&slot) = old.and_then(|old| old.countries.values.get(&id)) {
                by_canonical_raw = by_canonical_raw.removed(&(slot, id as u32))?;
                changed_sources = changed_sources.inserted(slot, ())?;
            }
            if let Some(&slot) = countries.values.get(&id) {
                by_canonical_raw = by_canonical_raw.inserted((slot, id as u32), ())?;
                changed_sources = changed_sources.inserted(slot, ())?;
            }
        }
        for &slot in changed_sources.keys() {
            raw = match by_canonical_raw
                .last_at_or_before(&(slot, u32::MAX))
                .filter(|(key, _)| key.0 == slot)
            {
                Some((key, _)) => raw.inserted(slot, key.1)?,
                None => raw.removed(&slot)?,
            };
        }
        for &id in appearance.edits.iter() {
            for &slot in countries
                .values
                .get(&id)
                .into_iter()
                .chain(old.and_then(|old| old.countries.values.get(&id)))
            {
                changed_sources = changed_sources.inserted(slot, ())?;
            }
        }
        let mut dirty = relations.affected(old.map(|old| &old.relations))?;
        if relations.recolors_all(old.map(|old| &old.relations)) {
            for &slot in slots.keys() {
                dirty = dirty.inserted(slot, ())?;
            }
        }
        dirty = changed_keys(
            &slots,
            &old.map_or_else(Slots::default, |old| old.slots.clone()),
            dirty,
        )?;
        let mut source_of = old.map_or_else(ValueIndex::default, |old| old.source_of.clone());
        let mut by_source_target =
            old.map_or_else(ValueIndex::default, |old| old.by_source_target.clone());
        for &source in changed_sources.keys() {
            for (&(_, target), _) in by_source_target
                .iter_from(&(source, 0))
                .take_while(|(key, _)| key.0 == source)
            {
                dirty = dirty.inserted(target, ())?;
            }
        }
        let mut colors = old.map_or_else(ValueIndex::default, |old| old.colors.clone());
        let mut unknown = old.map_or_else(Slots::default, |old| old.unknown.clone());
        let mut missing = old.map_or(0, |old| old.missing);
        for &target in dirty.keys() {
            let was_unknown = unknown.get(&target).is_some();
            if !was_unknown
                && old.is_some_and(|old| {
                    old.slots.get(&target).is_some() && old.colors.get(&target).is_none()
                })
            {
                missing -= 1;
            }
            unknown = unknown.removed(&target)?;
            if let Some(source) = source_of.get(&target).copied() {
                by_source_target = by_source_target.removed(&(source, target))?;
            }
            if slots.get(&target).is_none() {
                source_of = source_of.removed(&target)?;
                colors = colors.removed(&target)?;
                continue;
            }
            let Some(source) = relations.source(target)? else {
                source_of = source_of.removed(&target)?;
                colors = colors.removed(&target)?;
                unknown = unknown.inserted(target, ())?;
                continue;
            };
            source_of = source_of.inserted(target, source)?;
            by_source_target = by_source_target.inserted((source, target), ())?;
            colors = match raw
                .get(&source)
                .and_then(|raw| appearance.values.get(&(*raw as u128)))
            {
                Some(bits) => colors.inserted(target, *bits)?,
                None => {
                    missing += 1;
                    colors.removed(&target)?
                }
            };
        }
        if missing != 0 {
            warnings.push(format!("{missing} countries have no recorded colour this hour and take no fill in this map mode"));
        }
        Ok(Self {
            binding: Binding::new(inputs, bank),
            links,
            countries,
            by_canonical_raw,
            raw,
            appearance,
            relations,
            slots,
            source_of,
            by_source_target,
            colors,
            unknown,
            missing,
        })
    }
}
fn changed_keys<V: Clone + PartialEq>(
    new: &ValueIndex<u32, V>,
    old: &ValueIndex<u32, V>,
    mut keys: Slots,
) -> io::Result<Slots> {
    let mut error = None;
    new.visit_changes(old, |key, _, _| {
        if error.is_none() {
            match keys.inserted(*key, ()) {
                Ok(next) => keys = next,
                Err(why) => error = Some(why),
            }
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(keys)
}
struct Canonical<V> {
    rows: RowFold<(u32, V)>,
    by_link_row: ValueIndex<(u32, u128), ()>,
    by_slot_row: ValueIndex<(u32, u128), V>,
    by_row_slot: ValueIndex<u128, u32>,
    links: ValueIndex<u128, u32>,
    dirty: Slots,
}
impl<V: Clone + PartialEq> Canonical<V> {
    fn read<T: TableSchema>(
        inputs: &Inputs,
        base: &Base,
        old: Option<&Self>,
        table: &[T::Row],
        mask: u64,
        mut extract: impl FnMut(&T::Row) -> io::Result<(u32, V)>,
        include: impl Fn(&V) -> bool,
    ) -> io::Result<Self> {
        let rows = RowFold::read(
            inputs.frame,
            T::ID,
            table,
            mask,
            old.map(|old| &old.rows),
            |row| extract(row),
        )?;
        let mut by_link_row = old.map_or_else(ValueIndex::default, |old| old.by_link_row.clone());
        let mut by_slot_row = old.map_or_else(ValueIndex::default, |old| old.by_slot_row.clone());
        let mut dirty_rows = ValueIndex::default();
        for &key in rows.edits.iter() {
            dirty_rows = dirty_rows.inserted(key, ())?;
        }
        let mut changed_links = ValueIndex::default();
        let mut error = None;
        base.links.values.visit_changes(
            &old.map_or_else(ValueIndex::default, |old| old.links.clone()),
            |raw, _, _| {
                if error.is_none() {
                    match changed_links.inserted(*raw, ()) {
                        Ok(next) => changed_links = next,
                        Err(why) => error = Some(why),
                    }
                }
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        for &raw in changed_links.keys() {
            for (&(_, key), _) in by_link_row
                .iter_from(&(raw as u32, 0))
                .take_while(|(key, _)| key.0 == raw as u32)
            {
                dirty_rows = dirty_rows.inserted(key, ())?;
            }
        }
        let mut by_row_slot = old.map_or_else(ValueIndex::default, |old| old.by_row_slot.clone());
        let mut dirty = Slots::default();
        for &key in dirty_rows.keys() {
            if let Some((raw, _)) = old.and_then(|old| old.rows.values.get(&key)) {
                by_link_row = by_link_row.removed(&(*raw, key))?;
            }
            if let Some(slot) = by_row_slot.get(&key).copied() {
                by_slot_row = by_slot_row.removed(&(slot, key))?;
                by_row_slot = by_row_slot.removed(&key)?;
                dirty = dirty.inserted(slot, ())?;
            }
            if let Some((raw, value)) = rows.values.get(&key) {
                by_link_row = by_link_row.inserted((*raw, key), ())?;
                if include(value)
                    && let Some(&slot) = base.links.values.get(&(*raw as u128))
                {
                    by_slot_row = by_slot_row.inserted((slot, key), value.clone())?;
                    by_row_slot = by_row_slot.inserted(key, slot)?;
                    dirty = dirty.inserted(slot, ())?;
                }
            }
        }
        Ok(Self {
            rows,
            by_link_row,
            by_slot_row,
            by_row_slot,
            links: base.links.values.clone(),
            dirty,
        })
    }
}

struct Factions {
    members: Canonical<u64>,
    colors: RowFold<Bits>,
    by_faction_slot: ValueIndex<(u64, u32), ()>,
    winner: ValueIndex<u32, u64>,
    slots: Slots,
    styles: Paint,
}
struct Players {
    rows: Canonical<()>,
    colors: ValueIndex<u32, Bits>,
    selected: Option<u32>,
    styles: Paint,
}
type Party = (u32, Retained<Vec<u8>>);
struct Ideology {
    politics: Canonical<Option<u32>>,
    parties: Canonical<Party>,
    by_slot_party_row: ValueIndex<(u32, u32, u128), Retained<Vec<u8>>>,
    unknown_of: ValueIndex<u32, Retained<String>>,
    unknown: ValueIndex<Retained<String>, usize>,
    colors: ValueIndex<u32, Bits>,
    styles: Paint,
}
impl CountryLayers {
    pub(super) fn read(
        &mut self,
        variant: Variant,
        inputs: &Inputs,
        bank: Option<&Weak<MapData>>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<ModeLayer>> {
        let old = self
            .base
            .as_ref()
            .filter(|old| old.binding.accepts(inputs, bank));
        if old.is_none() {
            self.factions = None;
            self.players = None;
            self.ideology = None;
            self.country = Paint::default();
            self.country_colors = ValueIndex::default();
        }
        let base = Base::read(inputs, bank.cloned().unwrap_or_default(), old, warnings)?;
        let paint = match variant {
            Variant::Country => {
                let dirty = changed_keys(&base.colors, &self.country_colors, Slots::default())?;
                for &slot in dirty.keys() {
                    self.country = set_color(
                        &self.country,
                        slot,
                        base.colors.get(&slot).copied(),
                        None,
                        inputs,
                    )?;
                }
                self.country_colors = base.colors.clone();
                self.country.clone()
            }
            Variant::Factions => {
                let members = match complete::<tables::FactionMembers>(inputs.frame, "factions") {
                    Ok(rows) => rows,
                    Err(why) => {
                        warnings.push(why);
                        self.base = Some(base);
                        return Ok(None);
                    }
                };
                let factions = match complete::<tables::Factions>(inputs.frame, "factions") {
                    Ok(rows) => rows,
                    Err(why) => {
                        warnings.push(why);
                        self.base = Some(base);
                        return Ok(None);
                    }
                };
                let old = self.factions.as_ref();
                let members = Canonical::read::<tables::FactionMembers>(
                    inputs,
                    &base,
                    old.map(|old| &old.members),
                    members,
                    TableId::FactionMembers.full_mask(),
                    |row| Ok((row.country, row.faction)),
                    |_| true,
                )?;
                let colors = RowFold::read(
                    inputs.frame,
                    TableId::Factions,
                    factions,
                    fields(
                        TableId::Factions,
                        &["color_red", "color_green", "color_blue", "color_alpha"],
                    ),
                    old.map(|old| &old.colors),
                    |row| {
                        Ok([
                            row.color_red,
                            row.color_green,
                            row.color_blue,
                            row.color_alpha,
                        ])
                    },
                )?;
                let mut by_faction_slot =
                    old.map_or_else(ValueIndex::default, |old| old.by_faction_slot.clone());
                let mut winner = old.map_or_else(ValueIndex::default, |old| old.winner.clone());
                let mut dirty = changed_keys(
                    &base.slots,
                    &old.map_or_else(Slots::default, |old| old.slots.clone()),
                    members.dirty.clone(),
                )?;
                for &id in colors.edits.iter() {
                    for (&(_, slot), _) in by_faction_slot
                        .iter_from(&(id as u64, 0))
                        .take_while(|(key, _)| key.0 == id as u64)
                    {
                        dirty = dirty.inserted(slot, ())?;
                    }
                }
                let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
                for &slot in dirty.keys() {
                    if let Some(id) = winner.get(&slot).copied() {
                        by_faction_slot = by_faction_slot.removed(&(id, slot))?;
                    }
                    let faction = members
                        .by_slot_row
                        .last_at_or_before(&(slot, u128::MAX))
                        .filter(|(key, _)| key.0 == slot)
                        .map(|(_, id)| *id);
                    if let Some(id) = faction {
                        winner = winner.inserted(slot, id)?;
                        by_faction_slot = by_faction_slot.inserted((id, slot), ())?;
                    } else {
                        winner = winner.removed(&slot)?;
                    }
                    styles = if base.slots.get(&slot).is_some() {
                        let color = faction
                            .and_then(|id| colors.values.get(&(id as u128)))
                            .map(|bits| {
                                with_alpha(
                                    bits.map(f32::from_bits),
                                    inputs.content.defines.factions_member_alpha,
                                )
                            })
                            .unwrap_or(inputs.content.defines.factions_not_member);
                        styles.inserted(slot + 1, style(color))?
                    } else {
                        styles.removed(&(slot + 1))?
                    };
                }
                let paint = styles.clone();
                self.factions = Some(Factions {
                    members,
                    colors,
                    by_faction_slot,
                    winner,
                    slots: base.slots.clone(),
                    styles,
                });
                paint
            }
            Variant::Players => {
                let rows = match complete::<tables::Players>(inputs.frame, "players") {
                    Ok(rows) => rows,
                    Err(why) => {
                        warnings.push(why);
                        self.base = Some(base);
                        return Ok(None);
                    }
                };
                let old = self.players.as_ref();
                let rows = Canonical::read::<tables::Players>(
                    inputs,
                    &base,
                    old.map(|old| &old.rows),
                    rows,
                    fields(TableId::Players, &["country"]),
                    |row| Ok((row.country, ())),
                    |_| true,
                )?;
                let mut dirty = changed_keys(
                    &base.colors,
                    &old.map_or_else(ValueIndex::default, |old| old.colors.clone()),
                    rows.dirty.clone(),
                )?;
                if old.is_none_or(|old| old.selected != inputs.selected) {
                    for slot in inputs
                        .selected
                        .into_iter()
                        .chain(old.and_then(|old| old.selected))
                    {
                        dirty = dirty.inserted(slot, ())?;
                    }
                }
                let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
                for &slot in dirty.keys() {
                    let human = rows
                        .by_slot_row
                        .iter_from(&(slot, 0))
                        .next()
                        .is_some_and(|(key, _)| key.0 == slot);
                    styles = set_color(
                        &styles,
                        slot,
                        base.colors.get(&slot).copied(),
                        Some(human),
                        inputs,
                    )?;
                }
                let paint = styles.clone();
                self.players = Some(Players {
                    rows,
                    colors: base.colors.clone(),
                    selected: inputs.selected,
                    styles,
                });
                paint
            }
            Variant::Ideology => {
                let politics = match complete::<tables::Politics>(inputs.frame, "ideology") {
                    Ok(rows) => rows,
                    Err(why) => {
                        warnings.push(why);
                        self.base = Some(base);
                        return Ok(None);
                    }
                };
                let parties = match complete::<tables::PoliticalParties>(inputs.frame, "ideology") {
                    Ok(rows) => rows,
                    Err(why) => {
                        warnings.push(why);
                        self.base = Some(base);
                        return Ok(None);
                    }
                };
                let old = self.ideology.as_ref();
                let politics = Canonical::read::<tables::Politics>(
                    inputs,
                    &base,
                    old.map(|old| &old.politics),
                    politics,
                    fields(TableId::Politics, &["ruling_party"]),
                    |row| Ok((row.country, row.ruling_party)),
                    Option::is_some,
                )?;
                let parties = Canonical::read::<tables::PoliticalParties>(
                    inputs,
                    &base,
                    old.map(|old| &old.parties),
                    parties,
                    fields(TableId::PoliticalParties, &["token"]),
                    |row| Ok((row.country, (row.id, Retained::new(row.token.clone())))),
                    |_| true,
                )?;
                let mut by_slot_party_row =
                    old.map_or_else(ValueIndex::default, |old| old.by_slot_party_row.clone());
                let mut error = None;
                parties.by_slot_row.visit_changes(
                    &old.map_or_else(ValueIndex::default, |old| old.parties.by_slot_row.clone()),
                    |&(slot, ordinal), before, after| {
                        let result = (|| {
                            if let Some((id, _)) = before {
                                by_slot_party_row =
                                    by_slot_party_row.removed(&(slot, *id, ordinal))?;
                            }
                            if let Some((id, token)) = after {
                                by_slot_party_row = by_slot_party_row
                                    .inserted((slot, *id, ordinal), token.clone())?;
                            }
                            Ok::<_, io::Error>(())
                        })();
                        if let Err(why) = result {
                            error = Some(why);
                        }
                    },
                );
                if let Some(error) = error {
                    return Err(error);
                }
                let mut dirty = changed_keys(
                    &base.colors,
                    &old.map_or_else(ValueIndex::default, |old| old.colors.clone()),
                    politics.dirty.clone(),
                )?;
                for &slot in parties.dirty.keys() {
                    dirty = dirty.inserted(slot, ())?;
                }
                let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
                let mut unknown_of =
                    old.map_or_else(ValueIndex::default, |old| old.unknown_of.clone());
                let mut unknown = old.map_or_else(ValueIndex::default, |old| old.unknown.clone());
                for &slot in dirty.keys() {
                    if let Some(token) = unknown_of.get(&slot).cloned() {
                        let count = *unknown.get(&token).expect("unknown ideology count");
                        unknown = if count == 1 {
                            unknown.removed(&token)?
                        } else {
                            unknown.inserted(token, count - 1)?
                        };
                        unknown_of = unknown_of.removed(&slot)?;
                    }
                    let mut color = base.colors.get(&slot).map(|bits| bits.map(f32::from_bits));
                    let ruling = politics
                        .by_slot_row
                        .last_at_or_before(&(slot, u128::MAX))
                        .filter(|(key, _)| key.0 == slot)
                        .and_then(|(_, id)| *id);
                    if color.is_some()
                        && let Some(token) = ruling.and_then(|id| {
                            by_slot_party_row
                                .iter_from(&(slot, id, 0))
                                .next()
                                .filter(|(key, _)| key.0 == slot && key.1 == id)
                                .map(|(_, token)| token)
                        })
                    {
                        let text = String::from_utf8_lossy(token);
                        let group = match inputs.content.ideologies.get(&text) {
                            Ok(Some(group)) => Ok(Some(group)),
                            Ok(None) => inputs.content.ideologies.group_of(&text),
                            Err(gap) => Err(gap),
                        };
                        color = match group {
                            Ok(Some(group)) => group.color.as_ref().ok().map(|color| {
                                with_alpha(*color, inputs.content.defines.ideology_alpha)
                            }),
                            _ => None,
                        };
                        if color.is_none() {
                            let token = Retained::new(text.into_owned());
                            let count = unknown.get(&token).copied().unwrap_or(0);
                            unknown = unknown.inserted(token.clone(), count + 1)?;
                            unknown_of = unknown_of.inserted(slot, token)?;
                        }
                    }
                    styles = match color {
                        Some(color) => styles.inserted(slot + 1, style(color))?,
                        None => styles.removed(&(slot + 1))?,
                    };
                }
                if !unknown.is_empty() {
                    warnings.push(format!("The ideology map mode leaves countries unfilled whose ruling ideology has no readable content colour: {}", unknown.keys().map(|key| key.as_str()).collect::<Vec<_>>().join(", ")));
                }
                let paint = styles.clone();
                self.ideology = Some(Ideology {
                    politics,
                    parties,
                    by_slot_party_row,
                    unknown_of,
                    unknown,
                    colors: base.colors.clone(),
                    styles,
                });
                paint
            }
        };
        self.base = Some(base);
        Ok(Some(ModeLayer {
            region_of: membership::plane(inputs, 0, &paint)?,
            styles: paint,
            thickness: None,
            gradient: None,
            channel: if variant == Variant::Factions {
                inputs.content.defines.factions_channel
            } else {
                inputs.content.defines.country_channel
            },
        }))
    }
}
fn set_color(
    styles: &Paint,
    slot: u32,
    bits: Option<Bits>,
    human: Option<bool>,
    inputs: &Inputs,
) -> io::Result<Paint> {
    let Some(bits) = bits else {
        return styles.removed(&(slot + 1));
    };
    let mut color = bits.map(f32::from_bits);
    if human == Some(false) {
        color = with_alpha(color, inputs.content.defines.players_other_alpha);
    }
    let mut paint = style(color);
    if human.is_some() && Some(slot) == inputs.selected {
        paint.pulse = true;
        paint.fill = inputs.content.defines.selected_fill;
    }
    styles.inserted(slot + 1, paint)
}
