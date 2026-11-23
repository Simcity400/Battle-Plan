use super::*;
use map_display_data::buildings as data;

impl PreparedSlot {
    fn wire(
        &self,
        key: &impl Fn(usize) -> io::Result<Arc<data::AssetKey>>,
        types: &[content_reader_key_names::buildings::BuildingType],
    ) -> io::Result<data::Slot> {
        let select = |value: Selection| {
            Ok::<_, io::Error>(data::Selection {
                admitted: value.admitted,
                mesh: key(value.mesh)?,
                definition: key(value.definition)?,
                variant: value.variant,
            })
        };
        Ok(data::Slot {
            id: self.id,
            selected_type: self.source.map(|(index, _)| types[index].name.clone()),
            recorded_country: self.source.map(|(_, country)| country),
            drawn_at_distance: self.drawn_at_distance,
            entity: self
                .entity
                .as_ref()
                .map(|entity| {
                    Ok::<_, io::Error>(data::Entity {
                        healthy: select(entity.healthy)?,
                        ruined: select(entity.ruined)?,
                        destroyed: entity.destroyed,
                        uncertain: entity.uncertain,
                        damaged: entity.damaged,
                        disable_grow_animation: entity.disable_grow_animation,
                        instance: entity.instance.into(),
                    })
                })
                .transpose()?,
        })
    }
}

impl PreparedBuildings {
    pub(super) fn display_with(
        &self,
        bank: [u8; 32],
        perspective: u32,
        fog: bool,
        warnings: Vec<String>,
        key: impl Fn(usize) -> io::Result<Arc<data::AssetKey>>,
        types: &[content_reader_key_names::buildings::BuildingType],
        authored: Arc<Vec<data::AuthoredSlot>>,
        previous: Option<(&Chunked<PreparedSlot>, &data::Buildings)>,
    ) -> io::Result<data::Buildings> {
        let _cost = replay_frame::cpu::scope("buildings.wire", Some("buildings"));
        let previous = previous.filter(|(_, value)| {
            value.bank == bank && value.perspective == perspective && value.fog == fog
        });
        let previous_chunks = previous.filter(|(old, wire)| {
            old.chunks.len() == self.slots.chunks.len()
                && wire.slots.len() == self.slots.chunks.len()
        });
        let mut slots =
            previous_chunks.map_or_else(data::SlotChunks::default, |(_, old)| old.slots.clone());
        let changed = match previous_chunks {
            Some((old, _)) => self.slots.chunks.changed_pages(&old.chunks)?,
            None => (0..self.slots.chunks.len()).try_fold(
                map_display_data::copy_index::CopyIndex::default(),
                |pages, index| pages.inserted(index, ()),
            )?,
        };
        let mut edits = Vec::with_capacity(changed.len());
        for (&index, _) in changed.iter() {
            let chunk = self.slots.chunks.get(index).expect("authored slot chunk");
            let mut values = Vec::with_capacity(chunk.len());
            for slot in chunk.iter() {
                values.push(slot.wire(&key, types)?);
            }
            edits.push((
                index,
                Some(map_display_data::retained::Retained::new(values)),
            ));
        }
        slots = slots.patched(&edits)?;
        Ok(data::Buildings {
            bank,
            branch: self.branch,
            hour: self.hour,
            perspective,
            fog,
            shrink_distance: self.shrink_distance,
            popup_speed: self.popup_speed,
            authored,
            slots,
            groups: match previous {
                Some((_, old)) => Arc::clone(&old.groups),
                None => Arc::new(
                    self.groups
                        .iter()
                        .map(|group| data::Group {
                            province: group.province,
                            bounds: group.bounds,
                            slots: group.slots.clone(),
                        })
                        .collect(),
                ),
            },
            city_damage: Arc::new(self.city_damage.display()),
            warnings,
        })
    }

    #[cfg(test)]
    pub(super) fn install(
        value: &data::Buildings,
        groups: &[ProvinceSlots],
        binding: impl Fn(&data::AssetKey) -> io::Result<usize>,
        previous: Option<(&data::Buildings, &Self)>,
    ) -> io::Result<Self> {
        Self::install_with(value, groups, binding, previous, None)
    }

    fn install_with(
        value: &data::Buildings,
        groups: &[ProvinceSlots],
        binding: impl Fn(&data::AssetKey) -> io::Result<usize>,
        previous: Option<(&data::Buildings, &Self)>,
        shared: Option<Chunked<PreparedSlot>>,
    ) -> io::Result<Self> {
        let select = |value: &data::Selection| {
            Ok::<_, io::Error>(Selection {
                admitted: value.admitted,
                mesh: binding(&value.mesh)?,
                definition: binding(&value.definition)?,
                variant: value.variant,
            })
        };
        let previous = previous.filter(|(old, _)| {
            old.bank == value.bank && old.perspective == value.perspective && old.fog == value.fog
        });
        let previous_chunks = previous.filter(|(old, installed)| {
            old.slots.len() == value.slots.len()
                && installed.slots.chunks.len() == value.slots.len()
        });
        let slots = match (shared, previous_chunks) {
            (Some(slots), _) => slots,
            (_, Some((old, installed))) if old.slots.ptr_eq(&value.slots) => {
                installed.slots.clone()
            }
            _ => {
                let mut directory = previous_chunks.map_or_else(
                    map_display_data::value_index::ValueIndex::default,
                    |(_, installed)| installed.slots.directory(),
                );
                let mut changed = map_display_data::copy_index::CopyIndex::default();
                if let Some((old, _)) = previous_chunks {
                    let mut error = None;
                    value.slots.visit_changes(&old.slots, |index, _, _| {
                        if error.is_none() {
                            match changed.inserted(*index, ()) {
                                Ok(next) => changed = next,
                                Err(failed) => error = Some(failed),
                            }
                        }
                    });
                    if let Some(error) = error {
                        return Err(error);
                    }
                } else {
                    for index in 0..value.slots.len() {
                        changed = changed.inserted(index, ())?;
                    }
                }
                let mut edits = Vec::with_capacity(changed.len());
                for (&index, _) in changed.iter() {
                    let chunk = value
                        .slots
                        .get(index)
                        .expect("contiguous building chunk directory");
                    if chunk.is_empty()
                        || chunk.len() > incremental::CHUNK_SIZE
                        || (index + 1 < value.slots.len() && chunk.len() != incremental::CHUNK_SIZE)
                    {
                        return Err(invalid("building projection has an invalid slot chunk"));
                    }
                    let mut slots = Vec::with_capacity(chunk.len());
                    let start = index
                        .checked_mul(incremental::CHUNK_SIZE)
                        .ok_or_else(|| invalid("building slot count overflow"))?;
                    for (offset, slot) in chunk.iter().enumerate() {
                        if usize::try_from(slot.id).ok() != start.checked_add(offset) {
                            return Err(invalid("building projection slot order is invalid"));
                        }
                        slots.push(PreparedSlot {
                            source: None,
                            id: slot.id,
                            drawn_at_distance: slot.drawn_at_distance,
                            entity: slot
                                .entity
                                .as_ref()
                                .map(|entity| {
                                    Ok::<_, io::Error>(
                                        PreparedEntity {
                                            healthy: select(&entity.healthy)?,
                                            ruined: select(&entity.ruined)?,
                                            destroyed: entity.destroyed,
                                            uncertain: entity.uncertain,
                                            damaged: entity.damaged,
                                            disable_grow_animation: entity.disable_grow_animation,
                                            instance: entity.instance.into(),
                                        }
                                        .retain(),
                                    )
                                })
                                .transpose()?,
                        });
                    }
                    let chunk = map_display_data::retained::Retained::new(
                        Arc::<[PreparedSlot]>::from(slots),
                    );
                    edits.push((index, Some(chunk)));
                }
                directory = directory.patched(&edits)?;
                Chunked::from_directory(directory)?
            }
        };
        let next = slots.len();
        let validated = previous.is_some_and(|(old, installed)| {
            Arc::ptr_eq(&old.authored, &value.authored)
                && Arc::ptr_eq(&old.groups, &value.groups)
                && installed.slots.len() == next
        });
        if !validated {
            if value.authored.len() != next
                || value
                    .authored
                    .iter()
                    .enumerate()
                    .any(|(index, slot)| slot.id as usize != index)
            {
                return Err(invalid(
                    "building projection authored slot order is invalid",
                ));
            }
            if value.groups.len() != groups.len() {
                return Err(invalid("building projection group order is invalid"));
            }
            let mut province = None;
            for (group, authored) in value.groups.iter().zip(groups) {
                if province.is_some_and(|old| old >= group.province)
                    || group.slots.iter().any(|&slot| slot as usize >= next)
                    || group.province != authored.province
                    || group.bounds.map(f32::to_bits) != authored.bounds.map(f32::to_bits)
                    || group.slots != authored.slots
                {
                    return Err(invalid("building projection group order is invalid"));
                }
                province = Some(group.province);
            }
        }
        Ok(Self {
            branch: value.branch,
            hour: value.hour,
            shrink_distance: value.shrink_distance,
            popup_speed: value.popup_speed,
            slots,
            groups: match previous {
                Some((old, installed)) if Arc::ptr_eq(&old.groups, &value.groups) => {
                    Arc::clone(&installed.groups)
                }
                _ => value
                    .groups
                    .iter()
                    .map(|group| ProvinceSlots {
                        province: group.province,
                        bounds: group.bounds,
                        slots: group.slots.clone(),
                    })
                    .collect(),
            },
            city_damage: Arc::new(super::super::sprawl::Damage::from_display(
                &value.city_damage,
            )),
        })
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl MapBuildingAssets {
    fn keys(&self) -> &[Arc<data::AssetKey>] {
        self.display_keys.get_or_init(|| {
            self.library
                .meshes
                .iter()
                .map(|mesh| mesh.asset.display())
                .collect()
        })
    }

    pub fn display_slot(&self, slot: &PreparedSlot) -> io::Result<data::Slot> {
        let keys = self.keys();
        slot.wire(
            &|mesh| keys.get(mesh).cloned().ok_or_else(|| invalid("building mesh is outside its bank")),
            &self.facts.ordered,
        )
    }

    fn display_groups(&self) -> Arc<Vec<data::Group>> {
        Arc::clone(self.display_groups.get_or_init(|| {
            Arc::new(
                self.groups
                    .iter()
                    .map(|group| data::Group { province: group.province, bounds: group.bounds, slots: group.slots.clone() })
                    .collect(),
            )
        }))
    }

    fn authored(&self) -> &Arc<Vec<data::AuthoredSlot>> {
        self.display_sites.get_or_init(|| {
            let ordinary = self
                .slots
                .iter()
                .flat_map(|((location, bucket), positions)| {
                    positions
                        .iter()
                        .enumerate()
                        .map(move |(order, (position, province))| {
                            (*location, bucket.clone(), order, position, *province, false)
                        })
                });
            let floating = self
                .floating
                .iter()
                .enumerate()
                .map(|(order, (position, province))| {
                    (
                        position.state,
                        position.kind.clone(),
                        order,
                        position,
                        Some(*province),
                        true,
                    )
                });
            Arc::new(
                ordinary
                    .chain(floating)
                    .enumerate()
                    .map(
                        |(id, (location, bucket, order, position, province, floating))| {
                            data::AuthoredSlot {
                                id: id as u32,
                                location,
                                bucket,
                                order,
                                province,
                                position: [position.x, position.y, position.z],
                                rotation: position.rotation,
                                floating,
                            }
                        },
                    )
                    .collect(),
            )
        })
    }

    pub fn slot_pages(&self, slots: &[PreparedSlot], changed: &[usize], previous: Option<&SlotPages>) -> io::Result<SlotPages> {
        let size = incremental::CHUNK_SIZE;
        let chunk = |index: usize| map_display_data::retained::Retained::new(Arc::<[PreparedSlot]>::from(&slots[index * size..((index + 1) * size).min(slots.len())]));
        let previous = previous.filter(|old| old.0.len() == slots.len());
        let Some(previous) = previous else {
            let directory = (0..slots.len().div_ceil(size)).try_fold(map_display_data::value_index::ValueIndex::default(), |directory, index| directory.inserted(index, chunk(index)))?;
            return Ok(SlotPages(Chunked::from_directory(directory)?));
        };
        let mut named: Vec<usize> = changed.iter().copied().filter(|&index| index * size < slots.len()).collect();
        named.sort_unstable();
        named.dedup();
        if named.is_empty() {
            return Ok(previous.clone());
        }
        let edits: Vec<_> = named.into_iter().map(|index| (index, Some(chunk(index)))).collect();
        Ok(SlotPages(Chunked::from_directory(previous.0.directory().patched(&edits)?)?))
    }

    pub fn shared_product(&self, pages: &SlotPages, bank: [u8; 32], hour: WireHour) -> io::Result<data::Buildings> {
        if *self.display_binding.get_or_init(|| bank) != bank {
            return Err(invalid("building bank was replaced without rebinding its content"));
        }
        let groups = self.display_groups();
        Ok(data::Buildings {
            bank,
            branch: hour.branch,
            hour: hour.hour,
            perspective: hour.perspective,
            fog: hour.fog,
            shrink_distance: self.defines.shrink_distance,
            popup_speed: self.defines.popup_speed,
            authored: Arc::clone(self.authored()),
            slots: data::SlotChunks::default().with_producer(self.projection_id(), Arc::clone(&pages.0.chunks)),
            groups,
            city_damage: hour.city_damage,
            warnings: hour.warnings,
        })
    }

    pub fn display_binding(&self) -> io::Result<[u8; 32]> {
        self.display_binding
            .get()
            .copied()
            .ok_or_else(|| invalid("building bank is not bound to display content"))
    }

    pub fn display(
        &self,
        prepared: &PreparedBuildings,
        bank: [u8; 32],
        perspective: u32,
        fog: bool,
        warnings: Vec<String>,
        previous: Option<&BuildingsBase>,
    ) -> io::Result<data::Buildings> {
        if *self.display_binding.get_or_init(|| bank) != bank {
            return Err(invalid(
                "building bank was replaced without rebinding its content",
            ));
        }
        let keys = self.keys();
        let authored = self.authored();
        let mut value = prepared.display_with(
            bank,
            perspective,
            fog,
            warnings,
            |slot| {
                keys.get(slot)
                    .cloned()
                    .ok_or_else(|| invalid("building mesh is outside its bank"))
            },
            &self.facts.ordered,
            Arc::clone(authored),
            previous.and_then(|base| base.display.as_deref().map(|value| (&base.slots, value))),
        )?;
        value.slots = value
            .slots
            .with_producer(self.projection_id(), Arc::clone(&prepared.slots.chunks));
        Ok(value)
    }
}

#[derive(Clone, Debug)]
pub struct SlotPages(pub(super) Chunked<PreparedSlot>);

impl SlotPages {
    pub fn held(&self) -> usize {
        self.0.chunks.len() * 48
    }
}

pub struct WireHour {
    pub branch: u64,
    pub hour: i32,
    pub perspective: u32,
    pub fog: bool,
    pub warnings: Vec<String>,
    pub city_damage: Arc<data::CityDamage>,
}

impl BuildingHistory {
    pub fn install_display(
        &mut self,
        value: &data::Buildings,
        assets: &MapBuildingAssets,
    ) -> io::Result<PreparedBuildings> {
        if value.bank != assets.display_binding()? {
            return Err(invalid("building projection content binding mismatch"));
        }
        let bindings = assets
            .display_bindings
            .get_or_init(|| {
                let mut bindings = BTreeMap::new();
                for (slot, mesh) in assets.library.meshes.iter().enumerate() {
                    if bindings.insert(mesh.asset.display(), slot).is_some() {
                        return Err(
                            "building bank contains an ambiguous asset identity".to_string()
                        );
                    }
                }
                Ok(bindings)
            })
            .as_ref()
            .map_err(|reason| invalid(reason))?;
        let shared = value
            .slots
            .producer::<incremental::ChunkDirectory<PreparedSlot>>(assets.projection_id())
            .map(Chunked::shared);
        let installed = PreparedBuildings::install_with(
            value,
            &assets.groups,
            |key| {
                bindings
                    .get(key)
                    .copied()
                    .ok_or_else(|| invalid("building projection asset is absent from its bank"))
            },
            self.display_install
                .as_ref()
                .filter(|(id, _, _)| *id == assets.projection_id())
                .map(|(_, old, installed)| (old, installed)),
            shared,
        )?;
        self.display_install = Some((assets.projection_id(), value.clone(), installed.clone()));
        Ok(installed)
    }
}
