use super::relations::Relations;
use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{
    identities::CountryKey,
    landscape::{CountryFacts, ProvinceFacts, ProvinceField, Territory, TerritoryProvince},
    retained::Retained,
    value_index::ValueIndex,
};
use replay_data::rows::TableId;
use replay_frame::frame::ViewIdentity;

fn patch<K: Ord + Clone, V: Clone + PartialEq>(
    index: &ValueIndex<K, V>,
    edits: &mut Vec<(K, Option<V>)>,
) -> io::Result<ValueIndex<K, V>> {
    edits.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    edits.dedup_by(|a, b| {
        if a.0 != b.0 {
            return false;
        }
        debug_assert!(a.1 == b.1);
        true
    });
    index.patched(edits)
}

const CONTINUITY: &str = "observation only; country lifetime continuity is unknown";
type Country = Retained<(u32, u32, Vec<u8>)>;
#[derive(Default)]
pub struct TerritoryFold {
    current: Option<State>,
    bank: Option<std::sync::Weak<MapData>>,
}
struct State {
    branch: u64,
    source: ViewIdentity,
    provinces: RowFold<(u32, u32, u32, i32)>,
    links: RowFold<u32>,
    countries: RowFold<Country>,
    appearance: RowFold<[u8; 3]>,
    by_slot: ValueIndex<u32, u32>,
    by_canonical: ValueIndex<(u32, u32), ()>,
    by_link: ValueIndex<(u32, u32), ()>,
    by_controller: ValueIndex<(u32, u32), ()>,
    color_source_of: ValueIndex<u32, u32>,
    missing_by_slot: ValueIndex<u32, Retained<String>>,
    missing_tags: ValueIndex<Retained<String>, usize>,
    by_color_source: ValueIndex<(u32, u32), ()>,
    references: ValueIndex<u32, usize>,
    unresolved: ValueIndex<u32, ()>,
    relations: Relations,
    output: Arc<Territory>,
}
impl TerritoryFold {
    pub fn read_shared(
        &mut self,
        map: &Arc<MapData>,
        frame: &DecodedFrame,
    ) -> io::Result<Arc<Territory>> {
        self.read_input_shared(map, replay_frame::frame::HourInput::new(frame))
    }

    pub fn read_input_shared(
        &mut self,
        map: &Arc<MapData>,
        input: replay_frame::frame::HourInput<'_>,
    ) -> io::Result<Arc<Territory>> {
        let bank = Arc::downgrade(map);
        if self.bank.as_ref().is_none_or(|old| !old.ptr_eq(&bank)) {
            self.current = None;
        }
        self.bank = Some(bank);
        self.read_inner(map, input)
    }
    pub fn read(&mut self, map: &MapData, frame: &DecodedFrame) -> io::Result<Arc<Territory>> {
        self.current = None;
        self.bank = None;
        self.read_inner(map, replay_frame::frame::HourInput::new(frame))
    }
    fn read_inner(
        &mut self,
        map: &MapData,
        input: replay_frame::frame::HourInput<'_>,
    ) -> io::Result<Arc<Territory>> {
        let frame = input.frame();
        let country_rows = core::<tables::Countries>(frame)?;
        let link_rows = core::<tables::CountryLinks>(frame)?;
        let province_rows = core::<tables::Provinces>(frame)?;
        let before = self.current.as_ref().filter(|old| {
            old.branch == frame.branch && old.source.is(&frame.tables[TableId::Campaign.index()])
        });
        let _cost = replay_frame::cpu::scope(
            if before.is_some() {
                "territory delta fold"
            } else {
                "territory cold fold"
            },
            Some("territory"),
        );
        let countries = RowFold::read_input(
            input,
            TableId::Countries,
            country_rows,
            fields(TableId::Countries, &["canonical_slot", "tag"]),
            before.map(|old| &old.countries),
            |row| Ok(Retained::new((row.id, row.canonical_slot, row.tag.clone()))),
        )?;
        let links = RowFold::read_input(
            input,
            TableId::CountryLinks,
            link_rows,
            fields(TableId::CountryLinks, &["canonical_slot"]),
            before.map(|old| &old.links),
            |row| Ok(row.canonical_slot),
        )?;
        let provinces = RowFold::read_input(
            input,
            TableId::Provinces,
            province_rows,
            fields(
                TableId::Provinces,
                &["state", "owner", "controller", "victory_points"],
            ),
            before.map(|old| &old.provinces),
            |row| Ok((row.state, row.owner, row.controller, row.victory_points)),
        )?;
        let relations = Relations::read(map, frame, &links, before.map(|old| &old.relations))?;
        let relation_changes = relations.affected(before.map(|old| &old.relations))?;
        let appearance_rows = rows::<tables::CountryAppearance>(frame)?;
        let appearance = RowFold::read_input(
            input,
            TableId::CountryAppearance,
            appearance_rows.rows(),
            fields(TableId::CountryAppearance, &["red", "green", "blue"]),
            before.map(|old| &old.appearance),
            |row| Ok(row.map_color()),
        )?;
        let mut by_link = before.map_or_else(ValueIndex::default, |old| old.by_link.clone());
        let mut by_controller =
            before.map_or_else(ValueIndex::default, |old| old.by_controller.clone());
        let mut color_source_of =
            before.map_or_else(ValueIndex::default, |old| old.color_source_of.clone());
        let mut missing_by_slot =
            before.map_or_else(ValueIndex::default, |old| old.missing_by_slot.clone());
        let mut missing_tags =
            before.map_or_else(ValueIndex::default, |old| old.missing_tags.clone());
        let mut by_color_source =
            before.map_or_else(ValueIndex::default, |old| old.by_color_source.clone());
        let mut references = before.map_or_else(ValueIndex::default, |old| old.references.clone());
        let mut unresolved = before.map_or_else(ValueIndex::default, |old| old.unresolved.clone());
        let mut province_facts =
            before.map_or_else(ValueIndex::default, |old| old.output.provinces.0.clone());
        let mut country_facts = before.map_or_else(ValueIndex::default, |old| {
            old.output.countries.values.clone()
        });
        let mut link_facts =
            before.map_or_else(ValueIndex::default, |old| old.output.links.clone());
        let mut by_slot = before.map_or_else(ValueIndex::default, |old| old.by_slot.clone());
        let mut by_canonical =
            before.map_or_else(ValueIndex::default, |old| old.by_canonical.clone());
        let mut changed_slots = ValueIndex::<u32, ()>::default();
        for key in countries.edits.iter() {
            if let Some(old) = before.and_then(|old| old.countries.values.get(key)) {
                by_canonical = by_canonical.removed(&(old.1, old.0))?;
                changed_slots = changed_slots.inserted(old.1, ())?;
            }
            if let Some(row) = countries.values.get(key) {
                by_canonical = by_canonical.inserted((row.1, row.0), ())?;
                changed_slots = changed_slots.inserted(row.1, ())?;
            }
        }
        for (&slot, _) in changed_slots.iter() {
            let native = by_canonical
                .last_at_or_before(&(slot, u32::MAX))
                .filter(|(key, _)| key.0 == slot)
                .map(|(key, _)| key.1);
            if let Some(native) = native {
                let country = Retained::new(CountryKey {
                    branch: frame.branch,
                    native,
                    canonical_slot: slot,
                    observed_hour: frame.hour,
                    continuity: CONTINUITY.into(),
                });
                country_facts = country_facts.inserted(slot, country)?;
                by_slot = by_slot.inserted(slot, native)?;
            } else {
                country_facts = country_facts.removed(&slot)?;
                by_slot = by_slot.removed(&slot)?;
            }
        }
        for key in links.edits.iter() {
            link_facts = match links.values.get(key) {
                Some(slot) => link_facts.inserted(*key as u32, *slot)?,
                None => link_facts.removed(&(*key as u32))?,
            };
        }
        let mut dirty_edits = Vec::with_capacity(provinces.edits.len());
        dirty_edits.extend(provinces.edits.iter().map(|id| (*id as u32, Some(()))));
        let mut dirty = ValueIndex::<u32, ()>::default().patched(&dirty_edits)?;
        for key in links.edits.iter() {
            for (&(_, id), _) in by_link
                .iter_from(&(*key as u32, 0))
                .take_while(|(pair, _)| pair.0 == *key as u32)
            {
                dirty = dirty.inserted(id, ())?;
            }
        }
        for key in countries.edits.iter() {
            for slot in before
                .and_then(|old| old.countries.values.get(key))
                .map(|row| row.1)
                .into_iter()
                .chain(countries.values.get(key).map(|row| row.1))
            {
                for (&(_, id), _) in by_controller
                    .iter_from(&(slot, 0))
                    .take_while(|(pair, _)| pair.0 == slot)
                {
                    dirty = dirty.inserted(id, ())?;
                }
            }
        }
        let count = map
            .definitions
            .last_key_value()
            .map_or(0, |(&id, _)| id)
            .max(province_rows.last().map_or(0, |row| row.id)) as usize
            + 1;
        let planes = before.map(|old| {
            [
                &old.output.paint.country_of,
                &old.output.paint.owner_of,
                &old.output.paint.state_of,
            ]
        });
        let changed = dirty.len();
        let double = changed
            .checked_mul(2)
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let quadruple = changed
            .checked_mul(4)
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut plane_edits = Vec::<[(usize, u32); 3]>::with_capacity(changed);
        let mut province_edits = Vec::with_capacity(changed);
        let mut link_edits = Vec::with_capacity(quadruple);
        let mut controller_edits = Vec::with_capacity(double);
        let mut reference_edits = Vec::with_capacity(double);
        let mut dirty_colors = ValueIndex::<u32, ()>::default();
        let mut first_error: Option<(u32, io::Error)> = None;
        for (&id, _) in dirty.iter() {
            let old_row = before.and_then(|old| old.provinces.values.get(&(id as u128)));
            let next_row = provinces.values.get(&(id as u128));
            let old_links = old_row.map(|row| [row.1, row.2]);
            let next_links = next_row.map(|row| [row.1, row.2]);
            for &link in old_links.as_ref().into_iter().flatten() {
                if next_links.is_none_or(|next| !next.contains(&link)) {
                    link_edits.push(((link, id), None));
                }
            }
            let previous_slot = province_facts
                .get(&id)
                .and_then(|fact| fact.controller_slot.filter(|_| fact.controller_raw != 0))
                .filter(|slot| by_controller.get(&(*slot, id)).is_some());
            let next_slot = next_row
                .filter(|row| {
                    row.2 != 0
                        && map
                            .definitions
                            .get(&id)
                            .is_some_and(|definition| definition.kind != ProvinceKind::Sea)
                })
                .and_then(|row| links.values.get(&(row.2 as u128)))
                .copied();
            let same_controller = previous_slot.is_some() && previous_slot == next_slot;
            if let Some(slot) = previous_slot.filter(|_| !same_controller) {
                controller_edits.push(((slot, id), None));
                reference_edits.push((slot, false));
                dirty_colors = dirty_colors.inserted(slot, ())?;
            }
            unresolved = unresolved.removed(&id)?;
            let Some(&(state, owner, controller, victory_points)) =
                provinces.values.get(&(id as u128))
            else {
                province_edits.push((id, None));
                if (id as usize) < count {
                    plane_edits.push([(id as usize, 0); 3]);
                }
                continue;
            };
            let edit = (|| -> io::Result<()> {
                let definition = map.definitions.get(&id).ok_or_else(|| {
                    invalid(format!(
                        "recorded province {id} is absent from map/definition.csv"
                    ))
                })?;
                if id as usize >= count {
                    return Err(invalid("province ids exceed address space"));
                }
                for link in [owner, controller] {
                    if old_links.is_none_or(|old| !old.contains(&link)) {
                        link_edits.push(((link, id), Some(())));
                    }
                }
                let resolve = |raw| links.values.get(&(raw as u128)).copied();
                let owner_slot = resolve(owner);
                let controller_slot = resolve(controller);
                province_edits.push((
                    id,
                    Some(TerritoryProvince {
                        province: id,
                        state,
                        owner_raw: owner,
                        controller_raw: controller,
                        owner_slot,
                        controller_slot,
                        victory_points,
                    }),
                ));
                let mut paint = [0, 0, 0];
                if definition.kind != ProvinceKind::Sea
                    && !(definition.kind == ProvinceKind::Lake && controller == 0)
                {
                    if owner != 0 {
                        paint[1] = owner_slot.ok_or_else(|| invalid(format!("political fill references missing country link {owner} for owner")))?.checked_add(1).ok_or_else(|| invalid("owner country slot exceeds the border key range"))?;
                    }
                    paint[2] = state;
                    if controller == 0 {
                        unresolved = unresolved.inserted(id, ())?;
                    } else {
                        let slot = controller_slot.ok_or_else(|| invalid(format!("political fill references missing country link {controller} for controller")))?;
                        if by_slot.get(&slot).is_none() {
                            return Err(invalid(format!(
                                "political fill country link {controller} resolves to missing country slot {slot}"
                            )));
                        }
                        paint[0] = match slot.checked_add(1) {
                            Some(key) => key,
                            None => {
                                if let Some(source) = relations.source(slot)?
                                    && by_slot.get(&source).is_none()
                                {
                                    return Err(invalid(format!(
                                        "political fill country slot {source} has no Countries row"
                                    )));
                                }
                                return Err(invalid(
                                    "country slot exceeds the gradient border region key range",
                                ));
                            }
                        };
                        if !same_controller {
                            controller_edits.push(((slot, id), Some(())));
                            reference_edits.push((slot, true));
                            dirty_colors = dirty_colors.inserted(slot, ())?;
                        }
                    }
                }
                plane_edits.push(paint.map(|value| (id as usize, value)));
                Ok(())
            })();
            if let Err(error) = edit {
                if error.kind() != io::ErrorKind::InvalidData {
                    return Err(error);
                }
                if first_error.as_ref().is_none_or(|(old, _)| id < *old) {
                    first_error = Some((id, error));
                }
            }
        }
        province_facts = province_facts.patched(&province_edits)?;
        by_link = patch(&by_link, &mut link_edits)?;
        by_controller = patch(&by_controller, &mut controller_edits)?;
        reference_edits.sort_unstable_by_key(|edit| edit.0);
        let mut count_edits = Vec::with_capacity(reference_edits.len());
        let mut at = 0;
        while at < reference_edits.len() {
            let slot = reference_edits[at].0;
            let mut added = 0usize;
            let mut removed = 0usize;
            while at < reference_edits.len() && reference_edits[at].0 == slot {
                if reference_edits[at].1 {
                    added += 1;
                } else {
                    removed += 1;
                }
                at += 1;
            }
            let value = references
                .get(&slot)
                .copied()
                .unwrap_or(0)
                .checked_sub(removed)
                .and_then(|n| n.checked_add(added))
                .ok_or_else(|| invalid("political controller reference count is invalid"))?;
            count_edits.push((slot, (value != 0).then_some(value)));
        }
        references = references.patched(&count_edits)?;
        for slot in changed_slots.keys() {
            for native in before
                .and_then(|old| old.by_slot.get(slot))
                .into_iter()
                .chain(by_slot.get(slot))
            {
                for (&(_, controller), _) in by_color_source
                    .iter_from(&(*native, 0))
                    .take_while(|(key, _)| key.0 == *native)
                {
                    dirty_colors = dirty_colors.inserted(controller, ())?;
                }
            }
        }
        for &slot in relation_changes.keys() {
            if references.get(&slot).is_some() {
                dirty_colors = dirty_colors.inserted(slot, ())?;
            }
        }
        if relations.recolors_all(before.map(|old| &old.relations)) {
            for &slot in references.keys() {
                dirty_colors = dirty_colors.inserted(slot, ())?;
            }
        }
        for key in appearance.edits.iter() {
            for (&(_, slot), _) in by_color_source
                .iter_from(&(*key as u32, 0))
                .take_while(|(pair, _)| pair.0 == *key as u32)
            {
                dirty_colors = dirty_colors.inserted(slot, ())?;
            }
        }
        let mut country_colors = before.map_or_else(ValueIndex::default, |old| {
            old.output.paint.country_colors.clone()
        });
        for (&slot, _) in dirty_colors.iter() {
            country_colors = country_colors.removed(&(slot + 1))?;
            if let Some(old_source) = color_source_of.get(&slot).copied() {
                by_color_source = by_color_source.removed(&(old_source, slot))?;
                color_source_of = color_source_of.removed(&slot)?;
            }
            if let Some(tag) = missing_by_slot.get(&slot).cloned() {
                let count = missing_tags.get(&tag).copied().unwrap_or(0);
                missing_tags = if count <= 1 {
                    missing_tags.removed(&tag)?
                } else {
                    missing_tags.inserted(tag.clone(), count - 1)?
                };
                missing_by_slot = missing_by_slot.removed(&slot)?;
            }
            if references.get(&slot).is_none() {
                continue;
            }
            let color = (|| -> io::Result<[u8; 3]> {
                let Some(source) = relations.source(slot)? else {
                    return Ok(UNAVAILABLE_COLOR);
                };
                let native = by_slot.get(&source).copied().ok_or_else(|| {
                    invalid(format!(
                        "political fill country slot {source} has no Countries row"
                    ))
                })?;
                by_color_source = by_color_source.inserted((native, slot), ())?;
                color_source_of = color_source_of.inserted(slot, native)?;
                let color = match appearance.values.get(&(native as u128)) {
                    Some(color) => *color,
                    None => {
                        let country_native = by_slot
                            .get(&slot)
                            .copied()
                            .expect("validated controller slot");
                        let country = countries
                            .values
                            .get(&(country_native as u128))
                            .expect("validated country row");
                        let tag = Retained::new(String::from_utf8_lossy(&country.2).into_owned());
                        let count = missing_tags.get(&tag).copied().unwrap_or(0);
                        missing_tags = missing_tags.inserted(tag.clone(), count + 1)?;
                        missing_by_slot = missing_by_slot.inserted(slot, tag)?;
                        UNAVAILABLE_COLOR
                    }
                };
                Ok(color)
            })();
            match color {
                Ok(color) => {
                    country_colors = country_colors.inserted(slot + 1, color)?;
                }
                Err(error) => {
                    if error.kind() != io::ErrorKind::InvalidData {
                        return Err(error);
                    }
                    let id = by_controller
                        .iter_from(&(slot, 0))
                        .next()
                        .filter(|(key, _)| key.0 == slot)
                        .map_or(u32::MAX, |(key, _)| key.1);
                    if first_error.as_ref().is_none_or(|(old, _)| id < *old) {
                        first_error = Some((id, error));
                    }
                }
            }
        }
        if let Some((_, error)) = first_error {
            return Err(error);
        }
        if before.is_none() {
            for &id in map.definitions.keys() {
                if id != 0 && provinces.values.get(&(id as u128)).is_none() {
                    return Err(invalid(format!(
                        "hour {} has no recorded row for map province {id}",
                        frame.hour
                    )));
                }
            }
        } else {
            for id in provinces.edits.iter().map(|id| *id as u32) {
                if id != 0
                    && map.definitions.contains_key(&id)
                    && provinces.values.get(&(id as u128)).is_none()
                {
                    return Err(invalid(format!(
                        "hour {} has no recorded row for map province {id}",
                        frame.hour
                    )));
                }
            }
        }

        let mut warnings = (*relations.warnings).clone();
        if !unresolved.is_empty() {
            warnings.push(format!("{} land provinces have a zero controller reference and take no political fill; province ids {}",unresolved.len(),examples(unresolved.keys().map(u32::to_string))));
        }
        if !missing_tags.is_empty() {
            let gap = match &appearance_rows {
                Available::Complete(_) => "CountryAppearance has no row for them".to_owned(),
                Available::Truncated { missing, .. } => {
                    format!("CountryAppearance is truncated ({missing} missing rows)")
                }
                Available::Unavailable { reason } => {
                    format!("CountryAppearance is unavailable: {reason}")
                }
                Available::NotRecorded { mode } => {
                    format!("CountryAppearance is not recorded in a {mode} recording")
                }
            };
            warnings.push(format!(
                "{} countries have no recorded colour this hour ({gap}) and are filled grey: {}",
                missing_tags.len(),
                examples(missing_tags.keys().map(|tag| tag.to_string()))
            ));
        }
        let plane = |at: usize| -> io::Result<ProvinceField> {
            let base = match planes {
                Some(planes) => planes[at].resized(count)?,
                None => ProvinceField::filled(count)?,
            };
            base.patched_sorted(plane_edits.iter().map(|edits| edits[at]))
        };
        let paint = Arc::new(Regions {
            country_of: plane(0)?,
            owner_of: plane(1)?,
            state_of: plane(2)?,
            country_colors,
            warnings,
            strategic_attributes: BTreeMap::new(),
            mode: None,
        });
        let output = Arc::new(Territory {
            paint,
            countries: CountryFacts {
                values: country_facts,
                observed_hour: frame.hour,
            },
            links: link_facts,
            provinces: ProvinceFacts(province_facts),
        });
        self.current = Some(State {
            branch: frame.branch,
            source: frame.tables[TableId::Campaign.index()].identity(),
            provinces,
            links,
            countries,
            appearance,
            by_slot,
            by_canonical,
            by_link,
            by_controller,
            color_source_of,
            missing_by_slot,
            missing_tags,
            by_color_source,
            references,
            unresolved,
            relations,
            output: output.clone(),
        });
        Ok(output)
    }
}
