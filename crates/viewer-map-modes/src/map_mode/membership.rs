use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{landscape::ProvinceField, value_index::ValueIndex};
use replay_frame::frame::ViewIdentity;

#[derive(Default)]
pub(super) struct Memberships {
    state: [Option<State>; 2],
    bank: Option<std::sync::Weak<MapData>>,
}
struct State {
    branch: u64,
    source: ViewIdentity,
    bank: Option<std::sync::Weak<MapData>>,
    provinces: RowFold<u32>,
    links: Option<RowFold<u32>>,
    by_raw: ValueIndex<(u32, u32), ()>,
    by_region: ValueIndex<(u32, u32), ()>,
    regions: ValueIndex<u32, usize>,
    slots: Slots,
    plane: ProvinceField,
    changed: ValueIndex<u32, ()>,
    version: u64,
    filtered: Option<Filtered>,
}
struct Filtered {
    version: u64,
    styles: ValueIndex<u32, Style>,
    plane: ProvinceField,
}
impl Memberships {
    pub(super) fn bind(&mut self, map: &Arc<MapData>) {
        self.bank = Some(Arc::downgrade(map));
    }

    fn prepare(&mut self, inputs: &Inputs, kind: usize) -> io::Result<()> {
        let frame = inputs.frame;
        let provinces = core::<tables::Provinces>(frame)?;
        let bank = self
            .bank
            .as_ref()
            .filter(|bank| {
                bank.as_ptr() == inputs.map as *const MapData && bank.strong_count() != 0
            })
            .cloned();
        let previous = self.state[kind].as_ref().filter(|old| {
            bank.as_ref()
                .zip(old.bank.as_ref())
                .is_some_and(|(a, b)| a.ptr_eq(b))
                && old.branch == frame.branch
                && old.source.is(&frame.tables[TableId::Campaign.index()])
        });
        let rows = RowFold::read(
            frame,
            TableId::Provinces,
            provinces,
            fields(
                TableId::Provinces,
                if kind == 0 {
                    &["controller"]
                } else {
                    &["state"]
                },
            ),
            previous.map(|old| &old.provinces),
            |row| Ok(if kind == 0 { row.controller } else { row.state }),
        )?;
        let links = if kind == 0 {
            Some(RowFold::read(
                frame,
                TableId::CountryLinks,
                core::<tables::CountryLinks>(frame)?,
                fields(TableId::CountryLinks, &["canonical_slot"]),
                previous.and_then(|old| old.links.as_ref()),
                |row| Ok(row.canonical_slot),
            )?)
        } else {
            None
        };
        if let Some(old) = previous {
            if rows.values.ptr_eq(&old.provinces.values)
                && match (&links, &old.links) {
                    (Some(new), Some(old)) => new.values.ptr_eq(&old.values),
                    (None, None) => true,
                    _ => false,
                }
            {
                let state = self.state[kind]
                    .as_mut()
                    .expect("same membership continuation");
                state.provinces = rows;
                state.links = links;
                return Ok(());
            }
        }
        let mut by_raw = previous.map_or_else(ValueIndex::default, |old| old.by_raw.clone());
        let mut by_region = previous.map_or_else(ValueIndex::default, |old| old.by_region.clone());
        let mut regions = previous.map_or_else(ValueIndex::default, |old| old.regions.clone());
        let mut slots = previous.map_or_else(Slots::default, |old| old.slots.clone());
        let mut dirty = ValueIndex::default();
        for id in rows.edits.iter() {
            dirty = dirty.inserted(*id as u32, ())?;
        }
        for raw in links
            .as_ref()
            .into_iter()
            .flat_map(|links| links.edits.iter())
        {
            for (&(_, id), _) in by_raw
                .iter_from(&(*raw as u32, 0))
                .take_while(|(key, _)| key.0 == *raw as u32)
            {
                dirty = dirty.inserted(id, ())?;
            }
        }
        let len = region_table_len(
            inputs.map,
            rows.values
                .at(rows.values.len().saturating_sub(1))
                .map(|(id, _)| *id as u32)
                .into_iter(),
        );
        let plane = match previous {
            Some(old) => old.plane.resized(len)?,
            None => ProvinceField::filled(len)?,
        };
        let mut edits = ValueIndex::default();
        let mut changed = ValueIndex::default();
        for (&id, _) in dirty.iter() {
            let raw = rows.values.get(&(id as u128)).copied();
            let old_raw = previous
                .and_then(|old| old.provinces.values.get(&(id as u128)))
                .copied();
            if kind == 0 && raw != old_raw {
                if let Some(raw) = old_raw {
                    by_raw = by_raw.removed(&(raw, id))?;
                }
                if let Some(raw) = raw {
                    by_raw = by_raw.inserted((raw, id), ())?;
                }
            }
            let old_key = previous
                .and_then(|old| old.plane.get(id as usize))
                .copied()
                .unwrap_or(0);
            let key = match raw {
                Some(raw) if raw != 0 && kind == 1 => raw + 1,
                Some(raw) if raw != 0 && inputs.map.definitions.get(&id).is_some_and(|definition| definition.kind != ProvinceKind::Sea) => {
                    links.as_ref().and_then(|links| links.values.get(&(raw as u128))).copied().ok_or_else(|| invalid(format!("political fill references missing country link {raw} for controller")))? + 1
                }
                _ => 0,
            };
            if key == old_key {
                continue;
            }
            if old_key != 0 {
                by_region = by_region.removed(&(old_key, id))?;
                let count = regions
                    .get(&old_key)
                    .copied()
                    .expect("old membership count");
                if count == 1 {
                    regions = regions.removed(&old_key)?;
                    if kind == 0 {
                        slots.0 = slots.0.removed(&(old_key - 1))?;
                    }
                } else {
                    regions = regions.inserted(old_key, count - 1)?;
                }
            }
            if key != 0 {
                by_region = by_region.inserted((key, id), ())?;
                let count = regions.get(&key).copied().unwrap_or(0);
                regions = regions.inserted(key, count + 1)?;
                if kind == 0 && count == 0 {
                    slots.0 = slots.0.inserted(key - 1, ())?;
                }
            }
            changed = changed.inserted(id, ())?;
            if (id as usize) < len {
                edits = edits.inserted(id as usize, key)?;
            }
        }
        let plane = plane.patched_sorted(edits.iter().map(|(&id, &key)| (id, key)))?;
        let filtered = if previous.is_some() {
            self.state[kind].as_mut().unwrap().filtered.take()
        } else {
            None
        };
        let version = self.state[kind]
            .as_ref()
            .map_or(0, |old| old.version.wrapping_add(1));
        self.state[kind] = Some(State {
            bank,
            branch: frame.branch,
            source: frame.tables[TableId::Campaign.index()].identity(),
            provinces: rows,
            links,
            by_raw,
            by_region,
            regions,
            slots,
            plane,
            changed,
            version,
            filtered,
        });
        Ok(())
    }
    pub(super) fn countries(&mut self, inputs: &Inputs) -> io::Result<(ProvinceField, Slots)> {
        self.prepare(inputs, 0)?;
        let state = self.state[0].as_ref().unwrap();
        Ok((state.plane.clone(), state.slots.clone()))
    }
    pub(super) fn filtered(
        &mut self,
        inputs: &Inputs,
        kind: usize,
        styles: &ValueIndex<u32, Style>,
    ) -> io::Result<ProvinceField> {
        self.prepare(inputs, kind)?;
        let state = self.state[kind].as_mut().unwrap();
        let mut dirty = ValueIndex::default();
        let previous_styles = state
            .filtered
            .as_ref()
            .map_or_else(ValueIndex::default, |old| old.styles.clone());
        let mut changed_keys = ValueIndex::default();
        let mut error = None;
        styles.visit_changes(&previous_styles, |key, old, new| {
            if old.is_some() != new.is_some() && error.is_none() {
                match changed_keys.inserted(*key, ()) {
                    Ok(next) => changed_keys = next,
                    Err(why) => error = Some(why),
                }
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        for key in changed_keys.keys() {
            for (&(_, id), _) in state
                .by_region
                .iter_from(&(*key, 0))
                .take_while(|(pair, _)| pair.0 == *key)
            {
                dirty = dirty.inserted(id, ())?;
            }
        }
        let base = match &state.filtered {
            Some(old)
                if (old.version == state.version
                    || old.version.wrapping_add(1) == state.version) =>
            {
                if old.version != state.version {
                    for id in state.changed.keys() {
                        dirty = dirty.inserted(*id, ())?;
                    }
                }
                old.plane.resized(state.plane.len())?
            }
            _ => {
                for id in state.provinces.values.keys() {
                    dirty = dirty.inserted(*id as u32, ())?;
                }
                ProvinceField::filled(state.plane.len())?
            }
        };
        let plane = base.patched_sorted(
            dirty
                .keys()
                .filter(|id| (**id as usize) < state.plane.len())
                .map(|id| {
                    let raw = state.plane.get(*id as usize).copied().unwrap_or(0);
                    (
                        *id as usize,
                        if styles.get(&raw).is_some() { raw } else { 0 },
                    )
                }),
        )?;
        state.filtered = Some(Filtered {
            version: state.version,
            styles: styles.clone(),
            plane: plane.clone(),
        });
        Ok(plane)
    }
}
#[derive(Clone, Default)]
pub(super) struct Slots(ValueIndex<u32, ()>);
impl Slots {
    pub(super) fn index(&self) -> &ValueIndex<u32, ()> {
        &self.0
    }
    #[cfg(test)]
    pub(super) fn shares(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl<'a> IntoIterator for &'a Slots {
    type Item = &'a u32;
    type IntoIter = std::iter::Map<
        map_display_data::value_index::Iter<'a, u32, ()>,
        fn((&'a u32, &'a ())) -> &'a u32,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(slot, _)| slot)
    }
}
pub(super) fn countries(inputs: &Inputs) -> io::Result<(ProvinceField, Slots)> {
    inputs
        .content
        .memberships
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .countries(inputs)
}
pub(super) fn plane(
    inputs: &Inputs,
    kind: usize,
    styles: &ValueIndex<u32, Style>,
) -> io::Result<ProvinceField> {
    inputs
        .content
        .memberships
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .filtered(inputs, kind, styles)
}
