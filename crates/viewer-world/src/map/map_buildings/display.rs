use super::*;
use map_display_data::buildings as data;

impl From<ModelInstance> for data::Instance {
    fn from(value: ModelInstance) -> Self {
        Self {
            position: value.position,
            rotation: value.rotation,
            scale: value.scale,
            animation_seed: value.animation_seed,
        }
    }
}
impl From<data::Instance> for ModelInstance {
    fn from(value: data::Instance) -> Self {
        Self {
            position: value.position,
            rotation: value.rotation,
            scale: value.scale,
            animation_seed: value.animation_seed,
            ..Self::default()
        }
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
        types: &[installed_content_core::buildings::BuildingType],
        authored: Arc<Vec<data::AuthoredSlot>>,
        previous: Option<(&Chunked<PreparedSlot>, &data::Buildings)>,
    ) -> io::Result<data::Buildings> {
        let _cost = replay_frame::cpu::scope("buildings.wire", Some("buildings"));
        let select = |value: Selection| {
            Ok::<_, io::Error>(data::Selection {
                admitted: value.admitted,
                mesh: key(value.mesh)?,
                definition: key(value.definition)?,
                variant: value.variant,
            })
        };
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
        let _edits_credit = map_display_data::credit::display_products().admit_item(
            changed
                .len()
                .checked_mul(size_of::<(
                    usize,
                    Option<map_display_data::retained::Retained<Vec<data::Slot>>>,
                )>())
                .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?,
        )?;
        let mut edits = Vec::with_capacity(changed.len());
        for (&index, _) in changed.iter() {
            let chunk = self.slots.chunks.get(index).expect("authored slot chunk");
            let bytes = chunk
                .iter()
                .fold(
                    chunk.len().checked_mul(2 * size_of::<data::Slot>()),
                    |bytes, slot| {
                        bytes.and_then(|bytes| {
                            bytes.checked_add(
                                slot.source.map_or(0, |(index, _)| types[index].name.len()),
                            )
                        })
                    },
                )
                .and_then(|bytes| {
                    bytes.checked_add(
                        map_display_data::retained::Retained::<Vec<data::Slot>>::overhead(),
                    )
                })
                .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
            let credit = map_display_data::credit::display_products().admit_item(bytes)?;
            let values = chunk
                .iter()
                .map(|slot| {
                    Ok(data::Slot {
                        id: slot.id,
                        selected_type: slot.source.map(|(index, _)| types[index].name.clone()),
                        recorded_country: slot.source.map(|(_, country)| country),
                        drawn_at_distance: slot.drawn_at_distance,
                        entity: slot
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
                })
                .collect::<io::Result<Vec<_>>>()?;
            edits.push((
                index,
                Some(map_display_data::retained::Retained::with_credit(
                    credit,
                    || values,
                )),
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

    pub(super) fn install(
        value: &data::Buildings,
        groups: &[ProvinceSlots],
        binding: impl Fn(&data::AssetKey) -> io::Result<usize>,
        previous: Option<(&data::Buildings, &Self)>,
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
        let slots = match previous_chunks {
            Some((old, installed)) if old.slots.ptr_eq(&value.slots) => installed.slots.clone(),
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
                let _edits_credit = map_display_data::credit::display_products().admit_item(
                    changed
                        .len()
                        .checked_mul(size_of::<(
                            usize,
                            Option<map_display_data::retained::Retained<Arc<[PreparedSlot]>>>,
                        )>())
                        .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?,
                )?;
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
                    let bytes = chunk.len().checked_mul(2 * size_of::<PreparedSlot>())
                        .and_then(|bytes| bytes.checked_add(2 * size_of::<usize>() + map_display_data::retained::Retained::<Arc<[PreparedSlot]>>::overhead()))
                        .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
                    let credit = map_display_data::credit::display_products().admit_item(bytes)?;
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
                                    Ok::<_, io::Error>(PreparedEntity {
                                        healthy: select(&entity.healthy)?,
                                        ruined: select(&entity.ruined)?,
                                        destroyed: entity.destroyed,
                                        uncertain: entity.uncertain,
                                        damaged: entity.damaged,
                                        disable_grow_animation: entity.disable_grow_animation,
                                        instance: entity.instance.into(),
                                    })
                                })
                                .transpose()?,
                        });
                    }
                    edits.push((
                        index,
                        Some(map_display_data::retained::Retained::with_credit(
                            credit,
                            || Arc::<[PreparedSlot]>::from(slots),
                        )),
                    ));
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
                // Main visits each province's slots in authored file order, not slot-id order.
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
        let keys = self.display_keys.get_or_init(|| {
            self.library
                .meshes
                .iter()
                .map(|mesh| mesh.asset.display())
                .collect()
        });
        let authored = self.display_sites.get_or_init(|| {
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
        });
        prepared.display_with(
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
        )
    }
}

impl BuildingHistory {
    pub(crate) fn install_display(
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
        let installed = PreparedBuildings::install(
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
        )?;
        self.display_install = Some((assets.projection_id(), value.clone(), installed.clone()));
        Ok(installed)
    }
}
