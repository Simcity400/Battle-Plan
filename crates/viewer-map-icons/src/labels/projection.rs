use super::*;
#[derive(Clone, Debug, Default)]
pub struct LabelTables {
    pub display: map_display_data::landscape::Labels,
    pub(super) links: BTreeMap<u32, u32>,
    pub(super) tags: BTreeMap<u32, String>,
    pub(super) allegiances: BTreeMap<u32, Allegiance>,
    pub(super) capitals: BTreeSet<u32>,
    pub(super) candidates: Vec<usize>,
    pub(super) names: BTreeMap<u32, Result<Option<Arc<str>>, String>>,
    state_owners: BTreeMap<u32, u32>,
    states_gap: String,
    original_tags: BTreeMap<u32, Result<String, String>>,
    appearance_gap: String,
}

fn gap<T: replay_data::rows::TableSchema>(available: &Available<'_, T>) -> String {
    match available {
        Available::Complete(_) => "the complete table has no row for it".into(),
        Available::Truncated { missing, .. } => {
            format!("the table is truncated ({missing} rows missing)")
        }
        Available::Unavailable { reason } => format!("the table is unavailable: {reason}"),
        Available::NotRecorded { mode } => {
            format!("the table is not recorded in a {mode} recording")
        }
    }
}

impl LabelTables {
    pub fn read(frame: &DecodedFrame, ours: u32, localisation: &Localisation) -> io::Result<Self> {
        Self::read_inner(frame, Some(ours), localisation)
    }
    pub fn facts(
        frame: &DecodedFrame,
        localisation: &Localisation,
    ) -> io::Result<map_display_data::landscape::LabelFacts> {
        let tables = Self::read_inner(frame, None, localisation)?;
        Ok(map_display_data::landscape::LabelFacts {
            tags: tables.tags,
            candidates: tables.display.candidates,
        })
    }
    fn read_inner(
        frame: &DecodedFrame,
        viewing: Option<u32>,
        localisation: &Localisation,
    ) -> io::Result<Self> {
        let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let countries = core::<tables::Countries>(frame)?;
        let by_slot: BTreeMap<_, _> = countries
            .iter()
            .map(|country| (country.canonical_slot, country))
            .collect();
        let tags: BTreeMap<u32, String> = countries
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        let allegiances = viewing
            .map(|ours| allegiance::war_table(frame, &links, ours))
            .transpose()?
            .unwrap_or_default();
        let capitals = capital_provinces(frame, &links)?;
        let candidates = core::<tables::Provinces>(frame)?
            .iter()
            .enumerate()
            .filter(|(_, province)| province.victory_points > 0 || capitals.contains(&province.id))
            .map(|(index, _)| index)
            .collect();
        let states = rows::<tables::States>(frame)?;
        let appearance = rows::<tables::CountryAppearance>(frame)?;
        let mut original_tags: BTreeMap<_, _> = appearance
            .rows()
            .iter()
            .filter_map(|row| {
                let slot = *links.get(&row.country)?;
                Some((
                    slot,
                    content_reader_country_names::country_names::original_tag(by_slot.get(&slot)?, row)
                        .map(str::to_owned),
                ))
            })
            .collect();
        original_tags.insert(
            0,
            content_reader_country_names::country_names::reserved_zero_original_tag(frame)
                .map(str::to_owned),
        );
        let mut tables = Self {
            display: Default::default(),
            names: BTreeMap::new(),
            state_owners: states
                .rows()
                .iter()
                .map(|state| (state.id, state.owner))
                .collect(),
            states_gap: gap(&states),
            original_tags,
            appearance_gap: gap(&appearance),
            links,
            tags,
            allegiances,
            capitals,
            candidates,
        };
        let overrides = province_overrides(frame)?;
        let provinces = core::<tables::Provinces>(frame)?;
        for &index in &tables.candidates {
            let province = &provinces[index];
            let recorded = overrides.get(&province.id).copied();
            let tag = if recorded.is_some_and(overrides_ladder) {
                Ok(None)
            } else {
                tables.victory_point_tag(province.state).map(Some)
            };
            let name = tag.and_then(|tag| {
                label_name(localisation, recorded, tag, province.id)
                    .map(|name| name.map(Arc::from))
                    .map_err(|refusal| refusal.to_string())
            });
            tables.names.insert(province.id, name);
        }
        tables.display = map_display_data::landscape::Labels {
            perspective: viewing.and_then(|ours| tables.tags.get(&ours).cloned()),
            allegiances: tables
                .allegiances
                .iter()
                .map(|(&slot, side)| (slot, side.index()))
                .collect(),
            candidates: tables
                .candidates
                .iter()
                .map(|&index| {
                    let province = &provinces[index];
                    map_display_data::landscape::LabelCandidate {
                        province: province.id,
                        victory_points: province.victory_points,
                        capital: tables.capitals.contains(&province.id),
                        controller: tables.links.get(&province.controller).copied(),
                        name: match &tables.names[&province.id] {
                            Ok(text) => map_display_data::landscape::LabelName::Value {
                                text: text.as_ref().map(|text| text.to_string()),
                            },
                            Err(reason) => map_display_data::landscape::LabelName::Refused {
                                reason: reason.clone(),
                            },
                        },
                    }
                })
                .collect::<Vec<_>>()
                .into(),
        };
        #[cfg(test)]
        if let Some(ours) = viewing {
            let reference = super::projection_oracle::LabelTables::read(frame, ours, localisation)?
                .closed(frame)?;
            assert_eq!(tables.display, reference);
        }
        Ok(tables)
    }

    pub(super) fn victory_point_tag(&self, state: u32) -> Result<&str, String> {
        let slot = if state == 0 {
            0
        } else {
            let owner = *self.state_owners.get(&state).ok_or_else(|| {
                format!(
                    "the owner of state {state} (States.owner) is not recorded: {}",
                    self.states_gap
                )
            })?;
            if owner == 0 {
                0
            } else {
                *self.links.get(&owner).ok_or_else(|| {
                    format!("the owner link {owner} of state {state} resolves to no country")
                })?
            }
        };
        self.original_tags
            .get(&slot)
            .ok_or_else(|| {
                format!(
                    "CountryAppearance.original_tag of slot {slot} is not recorded: {}",
                    self.appearance_gap
                )
            })?
            .as_deref()
            .map_err(Clone::clone)
    }
}

pub(super) fn capital_provinces(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<BTreeSet<u32>> {
    Ok(capital_choices(frame, links)?
        .into_values()
        .filter_map(Result::ok)
        .collect())
}

pub(super) fn withheld_capitals(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Vec<(u32, &'static str)>> {
    Ok(capital_choices(frame, links)?
        .into_iter()
        .filter_map(|(state, choice)| choice.err().map(|why| (state, why)))
        .collect())
}

pub fn note_withheld_capitals(
    out: &mut viewer_foundation::tables::Limitations,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<()> {
    note_withheld(out, withheld_capitals(frame, links)?);
    Ok(())
}

pub const NO_VICTORY_POINTS: &str = "none of its provinces has victory points, and the anchor the game falls \
     back on is not recorded";

pub const TIED_VICTORY_POINTS: &str = "its provinces tie for the most victory points, and the state's province \
     order that settles the tie is not recorded";

pub fn note_withheld(out: &mut viewer_foundation::tables::Limitations, states: impl IntoIterator<Item = (u32, &'static str)>) {
    let mut withheld: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (state, why) in states {
        withheld.entry(why).or_default().push(state.to_string());
    }
    for (why, states) in withheld {
        let (noun, verb) = if states.len() == 1 {
            ("state", "carries")
        } else {
            ("states", "carry")
        };
        out.note_field(
            "Capital stars",
            &format!("capital {noun} {} {verb} no star: {why}", states.join(", ")),
        );
    }
}

fn capital_choices(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<BTreeMap<u32, Result<u32, &'static str>>> {
    let provinces = core::<tables::Provinces>(frame)?;
    let capitals: BTreeMap<u32, u32> = rows::<tables::Politics>(frame)?
        .rows()
        .iter()
        .filter(|politics| politics.capital > 0)
        .filter_map(|politics| {
            links
                .get(&politics.country)
                .map(|&slot| (slot, politics.capital as u32))
        })
        .collect();
    let wanted: BTreeSet<u32> = rows::<tables::States>(frame)?
        .rows()
        .iter()
        .filter(|state| {
            links
                .get(&state.controller)
                .and_then(|slot| capitals.get(slot))
                == Some(&state.id)
        })
        .map(|state| state.id)
        .collect();
    let mut best: BTreeMap<u32, (i32, Vec<u32>)> = wanted
        .into_iter()
        .map(|state| (state, (0, Vec::new())))
        .collect();
    for province in provinces {
        let Some((most, holders)) = best.get_mut(&province.state) else {
            continue;
        };
        if province.victory_points > *most {
            *most = province.victory_points;
            holders.clear();
        }
        if province.victory_points > 0 && province.victory_points == *most {
            holders.push(province.id);
        }
    }
    Ok(best
        .into_iter()
        .map(|(state, (_, holders))| {
            let choice = match holders[..] {
                [province] => Ok(province),
                [] => Err(NO_VICTORY_POINTS),
                _ => Err(TIED_VICTORY_POINTS),
            };
            (state, choice)
        })
        .collect())
}

pub(super) fn province_overrides(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, &[u8]>> {
    Ok(rows::<tables::ProvinceNames>(frame)?
        .rows()
        .iter()
        .map(|row| (row.province, row.name.as_slice()))
        .collect())
}

pub fn unreadable_overrides(frame: &DecodedFrame) -> io::Result<Vec<u32>> {
    let mut out: Vec<u32> = province_overrides(frame)?
        .into_iter()
        .filter(|(_, bytes)| unreadable(bytes))
        .map(|(province, _)| province)
        .collect();
    out.sort_unstable();
    Ok(out)
}
