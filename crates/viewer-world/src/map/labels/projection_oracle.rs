use super::*;
/// The camera-independent tables the labels read, built once per hour and
/// viewing country and reused by every camera's [`prepare`].
#[derive(Clone, Debug, Default)]
pub struct LabelTables {
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, String>,
    ours: u32,
    allegiances: BTreeMap<u32, Allegiance>,
    capitals: BTreeSet<u32>,
    /// Positive victory points and capitals in the hour's province row order.
    candidates: Vec<usize>,
    names: BTreeMap<u32, Result<Option<Arc<str>>, String>>,
    /// State id to its raw owner link, from `States`.
    state_owners: BTreeMap<u32, u32>,
    /// Why a state the `States` rows lack has no known owner.
    states_gap: String,
    /// Canonical slot to exact original tag or tag bytes or its local refusal.
    /// Slot zero comes from the authenticated ReservedCountryNaming singleton.
    original_tags: BTreeMap<u32, Result<String, String>>,
    /// Why a country the `CountryAppearance` rows lack has no known one.
    appearance_gap: String,
}

/// Why a table's rows lack an entry: a complete table lacks it, a truncated
/// or unavailable one may not have reached it.
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
    /// Reads one hour's links, tags, war sides and capitals for `ours`.
    pub fn read(frame: &DecodedFrame, ours: u32, localisation: &Localisation) -> io::Result<Self> {
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
        let allegiances = allegiance::war_table(frame, &links, ours)?;
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
                    installed_content_country_names::country_names::original_tag(by_slot.get(&slot)?, row)
                        .map(str::to_owned),
                ))
            })
            .collect();
        original_tags.insert(
            0,
            installed_content_country_names::country_names::reserved_zero_original_tag(frame)
                .map(str::to_owned),
        );
        let mut tables = Self {
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
            ours,
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
        Ok(tables)
    }

    /// State-owner tag operands used by victory-point localisation. The original
    /// tag and own tag remain separate recorded fields.
    fn victory_point_tag(&self, state: u32) -> Result<&str, String> {
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

/// Select the greatest victory-point value in the controller's current capital
/// state, breaking ties by lowest province id because recorded data lacks the
/// state's province order. Country/state fallback anchors are unrecorded, so
/// states without positive victory points also use the selected province.
/// Missing `Politics` or `States` rows cannot establish a capital; table
/// limitations report those gaps while ordinary labels remain available.
fn capital_provinces(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> io::Result<BTreeSet<u32>> {
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
    // A capital icon is created only when the state controller has this state
    // as its current capital. `States` records the controller independently.
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
    let mut best: BTreeMap<u32, (i32, u32)> = BTreeMap::new();
    for province in provinces {
        if !wanted.contains(&province.state) {
            continue;
        }
        let candidate = (province.victory_points, province.id);
        best.entry(province.state)
            .and_modify(|current| {
                if candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
                {
                    *current = candidate;
                }
            })
            .or_insert(candidate);
    }
    Ok(best.into_values().map(|(_, province)| province).collect())
}

/// The recorded `ProvinceNames` rows of one hour, by province.
///
/// The table is sparse: a province with no row has no override, which is the
/// whole meaning of absence **from a complete table**. Absence from a truncated
/// or unavailable one means nothing at all, and this function cannot tell the
/// two apart — nor does it have to. Either way the province falls through to the
/// static ladder and keeps the name it had; what changes is whether the hour is
/// entitled to be silent about it, and that is `viewer_foundation::map::tables::limitations`'
/// judgement, not this one's.
///
/// A later row wins over an earlier one for the same province, which is the only
/// reading a table keyed by province admits.
fn province_overrides(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, &[u8]>> {
    Ok(rows::<tables::ProvinceNames>(frame)?
        .rows()
        .iter()
        .map(|row| (row.province, row.name.as_slice()))
        .collect())
}

/// The provinces whose recorded override this port cannot lay out, for the
/// `Field ProvinceNames.name` limitation line.
pub fn unreadable_overrides(frame: &DecodedFrame) -> io::Result<Vec<u32>> {
    let mut out: Vec<u32> = province_overrides(frame)?
        .into_iter()
        .filter(|(_, bytes)| unreadable(bytes))
        .map(|(province, _)| province)
        .collect();
    out.sort_unstable();
    Ok(out)
}

impl LabelTables {
    pub(super) fn closed(
        &self,
        frame: &DecodedFrame,
    ) -> io::Result<map_display_data::landscape::Labels> {
        use map_display_data::landscape::{LabelCandidate, LabelName, Labels};
        let provinces = core::<tables::Provinces>(frame)?;
        Ok(Labels {
            perspective: self.tags.get(&self.ours).cloned(),
            allegiances: self
                .allegiances
                .iter()
                .map(|(&slot, side)| (slot, side.index()))
                .collect(),
            candidates: self
                .candidates
                .iter()
                .map(|&ordinal| {
                    let row = &provinces[ordinal];
                    LabelCandidate {
                        province: row.id,
                        victory_points: row.victory_points,
                        capital: self.capitals.contains(&row.id),
                        controller: self.links.get(&row.controller).copied(),
                        name: match &self.names[&row.id] {
                            Ok(text) => LabelName::Value {
                                text: text.as_ref().map(|text| text.to_string()),
                            },
                            Err(reason) => LabelName::Refused {
                                reason: reason.clone(),
                            },
                        },
                    }
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }
}
