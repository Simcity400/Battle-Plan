use replay_data::rows::tables;
use viewer_counter_kit::CounterInputs;
use super::{AirWingRow, Item, Wing, items};
use content_reader_localisation::localisation::Argument;
use replay_data::{rows::AirBaseRow, rows::Recorded, rows::WingEquipmentRow};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use dashboard_text::table_access::TokenNames;
use viewer_foundation::tooltip_text::Tooltip;

#[derive(Clone, Debug)]
pub struct PreparedItem {
    pub item: Item,
    pub icon_frame: i32,
    pub action: Option<i32>,
    pub country: Option<u32>,
    pub group_color: Option<[f32; 4]>,
    pub markers: [Option<bool>; 4],
    pub capacity: Option<i32>,
    pub hover: Arc<Tooltip>,
    pub marker_hovers: [Arc<Tooltip>; 4],
    pub refused: BTreeMap<String, usize>,
}

pub struct Census<'a> {
    frame: &'a DecodedFrame,
    primary: HashMap<u64, &'a WingEquipmentRow>,
}
impl<'a> Census<'a> {
    pub fn new(frame: &'a DecodedFrame) -> Result<Self, String> {
        let mut primary = HashMap::new();
        for row in frame
            .complete::<tables::WingEquipment>()
            .map_err(|e| e.to_string())?
        {
            let prior: &mut &WingEquipmentRow = primary.entry(row.wing).or_insert(row);
            if prior.amount_raw < row.amount_raw {
                *prior = row;
            }
        }
        Ok(Self { frame, primary })
    }
    pub fn carrier(
        &self,
        inputs: &CounterInputs<'_>,
        ships: Option<&super::super::naval::ShipInputs<'_>>,
        members: &[u64],
        viewer: u32,
    ) -> Result<Vec<PreparedItem>, String> {
        let ships = ships.ok_or("carrier wing grid ship roster is unavailable")?;
        let bases = carrier_bases(ships, members);
        self.bases(inputs, &bases, viewer)
    }
    pub fn base(
        &self,
        inputs: &CounterInputs<'_>,
        base: u64,
        viewer: u32,
    ) -> Result<Vec<PreparedItem>, String> {
        self.bases(inputs, &[base], viewer)
    }
    pub fn bases(
        &self,
        inputs: &CounterInputs<'_>,
        bases: &[u64],
        viewer: u32,
    ) -> Result<Vec<PreparedItem>, String> {
        let registrations = self
            .frame
            .complete::<tables::AirBaseRegistrations>()
            .map_err(|e| e.to_string())?;
        let roster: Vec<_> = bases
            .iter()
            .filter_map(|base| {
                registrations
                    .iter()
                    .find(|row| row.base == *base && row.slot == viewer)
            })
            .flat_map(|registration| registration.wings.iter().copied())
            .collect();
        if roster.is_empty() {
            return Ok(Vec::new());
        }
        let wings = self
            .frame
            .complete::<tables::AirWings>()
            .map_err(|e| e.to_string())?;
        let variants = self
            .frame
            .complete::<tables::VariantDefinitions>()
            .map_err(|e| e.to_string())?;
        let tokens = TokenNames::new(self.frame).map_err(|e| e.to_string())?;
        let equipment = inputs.equipment.map_err(str::to_owned)?;
        let links = self
            .frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let slot = |raw: u32| {
            links
                .binary_search_by_key(&raw, |row| row.id)
                .ok()
                .map(|at| links[at].canonical_slot)
        };
        let selected: std::collections::BTreeSet<_> = inputs.selected_air.iter().copied().collect();
        let mut values = HashMap::new();
        let mut prepared = Vec::new();
        for handle in &roster {
            let Some(wing) = wings
                .binary_search_by_key(handle, |row| row.id)
                .ok()
                .map(|at| &wings[at])
            else {
                continue;
            };
            let primary = self
                .primary
                .get(handle)
                .ok_or_else(|| format!("wing {handle} has no primary equipment entry"))?;
            let variant = variants
                .binary_search_by_key(&primary.variant, |row| row.id)
                .ok()
                .map(|at| &variants[at])
                .ok_or_else(|| format!("wing {handle} primary variant is unavailable"))?;
            let name = tokens
                .name(variant.equipment_token)
                .map_err(|e| e.to_string())?;
            let definition = equipment
                .named(name)
                .ok_or_else(|| format!("wing equipment {name} is not in the recorded content"))?;
            let group = wing
                .attached_orders_group
                .ok_or("air grid attachment identity is unavailable")?;
            let transfer = resolved_transfer(self.frame, wing);
            let eligible =
                wing.eligibility_veto == 0 && wing.eligibility_counter <= wing.eligibility_limit;
            let available = if !eligible {
                Some(false)
            } else {
                transfer.as_ref().ok().map(|&moving| !moving)
            };
            let raid = match wing.assigned_to_raid() {
                Some(value) => Some(value),
                None => match wing.raid_reference {
                    Recorded::Value(Some(value)) => Some(value.resolves),
                    _ => None,
                },
            };
            let no_mission = if wing.mission_mask != 0 || available == Some(false) {
                Some(false)
            } else {
                and(available, raid.map(|value| !value))
            };
            let no_region = if wing.mission_mask == 0
                || wing.mission_mask & 0x800 != 0
                || wing.mission_region.is_some()
                || available == Some(false)
            {
                Some(false)
            } else {
                and(available, raid.map(|value| !value))
            };
            let action = if !eligible || transfer == Ok(true) {
                3
            } else if transfer.is_err() {
                0
            } else if wing.mission_mask != 0
                && ((wing.mission_region.is_some() && wing.mission_active != 0)
                    || wing.mission_mask & 0x800 != 0)
            {
                4
            } else {
                2
            };
            let capacity = capacity(self.frame, inputs, wing);
            values.insert(
                *handle,
                Values {
                    wing,
                    icon_frame: definition
                        .air_map_icon_frame
                        .wrapping_mul(2)
                        .wrapping_sub(i32::from(!inputs.settings.nato)),
                    archetype_frame: definition
                        .archetype
                        .and_then(|at| equipment.equipment.get(at))
                        .map(|value| {
                            value
                                .air_map_icon_frame
                                .wrapping_mul(2)
                                .wrapping_sub(i32::from(!inputs.settings.nato))
                        }),
                    country: slot(if wing.country_override > 0 {
                        wing.country_override as u32
                    } else {
                        wing.owner
                    }),
                    capacity,
                    no_mission,
                    no_region,
                    action: (action != 0).then_some(action),
                },
            );
            prepared.push(Wing {
                row: wing,
                selected: selected.contains(handle),
                archetype: definition.archetype,
                group,
            });
        }
        items(prepared, false)
            .into_iter()
            .map(|item| {
                let members: Vec<_> = item.members.iter().map(|id| &values[id]).collect();
                let representative = members[0];
                let Summary {
                    action,
                    markers,
                    capacity: capacities,
                    mut refused,
                } = summary(item.group, &members);
                let group_color = group_color(self.frame, item.group, &mut refused);
                let hover = hover(inputs, self.frame, &item, representative, capacities);
                let marker_hovers = std::array::from_fn(|index| {
                    let key = match index {
                        0 => "AIR_WING_NO_MISSION",
                        1 if representative.wing.attachment_coverage() == Some(false) => {
                            "AIR_WING_NO_COVERAGE"
                        }
                        1 => "AIR_WING_NO_REGION",
                        2 => "AIR_WING_TRAINING",
                        _ => "AIRWING_MISSION_TRAINING_WILL_STOP_AT_MAX",
                    };
                    let mut tip = Tooltip::default();
                    let text = if index == 1 && representative.wing.attachment_coverage().is_none()
                    {
                        Err("representative's retained coverage byte is unavailable".into())
                    } else {
                        inputs
                            .localisation
                            .localise(key.as_bytes())
                            .map_err(|e| e.to_string())
                    };
                    tip.line(true, 0, key, text);
                    if index == 2 && markers[3] == Some(true) {
                        tip.line(
                            true,
                            2,
                            "training stop",
                            inputs
                                .localisation
                                .localise(b"AIRWING_MISSION_TRAINING_WILL_STOP_AT_MAX")
                                .map_err(|e| e.to_string()),
                        );
                    }
                    Arc::new(tip)
                });
                Ok(PreparedItem {
                    item,
                    action,
                    icon_frame: representative.icon_frame,
                    country: representative.country,
                    group_color,
                    markers,
                    capacity: capacities,
                    hover: Arc::new(hover),
                    marker_hovers,
                    refused,
                })
            })
            .collect()
    }
}

fn group_color(
    frame: &DecodedFrame,
    group: u64,
    refused: &mut BTreeMap<String, usize>,
) -> Option<[f32; 4]> {
    if group == 0 {
        return None;
    }
    let row = viewer_foundation::tables::rows::<tables::OrdersGroupPlans>(frame)
        .ok()
        .and_then(|plans| {
            let rows = plans.rows();
            rows.binary_search_by_key(&group, |row| row.group)
                .ok()
                .map(|at| &rows[at])
                .or_else(|| rows.iter().find(|row| row.group == group))
        });
    match row {
        None => {
            refused.insert(
                "Air grid group marker: current group colour is unavailable".into(),
                1,
            );
            None
        }
        Some(row) => match viewer_unit_reads::counter_inputs::color(
            row.red, row.green, row.blue, row.alpha,
        ) {
            Some(rgba) => Some(rgba),
            None => {
                refused.insert(
                    format!(
                        "Air grid group marker: orders group {group}'s colour \
                         (OrdersGroupPlans) has non-finite components"
                    ),
                    1,
                );
                None
            }
        },
    }
}

fn carrier_bases(ships: &super::super::naval::ShipInputs<'_>, members: &[u64]) -> Vec<u64> {
    members
        .iter()
        .flat_map(|member| ships.hulls(*member))
        .filter_map(|ship| ship.model.counter.as_ref())
        .map(|counter| counter.carrier_air_base)
        .filter(|base| *base != 0)
        .collect()
}

struct Values<'a> {
    wing: &'a AirWingRow,
    icon_frame: i32,
    archetype_frame: Option<i32>,
    country: Option<u32>,
    capacity: Result<i32, String>,
    no_mission: Option<bool>,
    no_region: Option<bool>,
    action: Option<i32>,
}
struct Summary {
    action: Option<i32>,
    markers: [Option<bool>; 4],
    capacity: Option<i32>,
    refused: BTreeMap<String, usize>,
}
fn summary(group: u64, members: &[&Values<'_>]) -> Summary {
    let mut refused = BTreeMap::new();
    let mut no_mission = Some(false);
    let mut no_region = Some(false);
    let mut training = false;
    let mut stop = Some(false);
    let mut covered = Some(false);
    let mut capacities = Some(0_i32);
    let frames: std::collections::BTreeSet<_> =
        members.iter().filter_map(|value| value.action).collect();
    let action = if frames.len() > 1 {
        Some(1)
    } else if members.iter().all(|value| value.action.is_some()) {
        frames.first().copied()
    } else {
        None
    };
    if action.is_none() {
        refused.insert(
            "Air grid action frame: transfer reference resolution unavailable".into(),
            1,
        );
    }
    for (index, value) in members.iter().enumerate() {
        no_mission = or(no_mission, value.no_mission);
        no_region = or(no_region, value.no_region);
        training |= value.wing.mission_mask & 0x800 != 0;
        stop = if index == 0 {
            and(Some(training), value.wing.training_stop())
        } else {
            and(stop, and(Some(training), value.wing.training_stop()))
        };
        covered = or(
            covered,
            if group == 0 {
                Some(false)
            } else {
                value.wing.attachment_coverage()
            },
        );
        capacities = capacities
            .zip(value.capacity.as_ref().ok().copied())
            .map(|(a, b)| a.wrapping_add(b));
        if let Err(reason) = &value.capacity {
            *refused
                .entry(format!("Air grid capacity: {reason}"))
                .or_default() += 1;
        }
    }
    let markers = [
        if group == 0 { no_mission } else { Some(false) },
        if group == 0 {
            and(no_mission.map(|v| !v), no_region)
        } else {
            covered.map(|v| !v)
        },
        and(
            Some(training),
            and(
                no_mission.map(|v| !v),
                and(
                    no_region.map(|v| !v),
                    if group == 0 { Some(true) } else { covered },
                ),
            ),
        ),
        and(Some(training), stop),
    ];
    for (name, value) in [
        "no_mission",
        "no_region",
        "training",
        "training_will_stop_at_max_icon",
    ]
    .into_iter()
    .zip(markers)
    {
        if value.is_none() {
            *refused
                .entry(format!(
                    "Air grid {name}: reached retained flag or raid resolution is unavailable"
                ))
                .or_default() += 1;
        }
    }
    Summary {
        action,
        markers,
        capacity: capacities,
        refused,
    }
}
fn and(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    if a == Some(false) || b == Some(false) {
        Some(false)
    } else {
        a.zip(b).map(|(a, b)| a && b)
    }
}
fn or(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    if a == Some(true) || b == Some(true) {
        Some(true)
    } else {
        a.zip(b).map(|(a, b)| a || b)
    }
}
fn resolved_transfer(frame: &DecodedFrame, wing: &AirWingRow) -> Result<bool, String> {
    if wing.base_override == 0 {
        return Ok(false);
    }
    let witnesses = frame
        .complete::<tables::AirReferenceWitnesses>()
        .map_err(|e| e.to_string())?;
    witnesses
        .binary_search_by_key(&wing.base_override, |row| row.handle)
        .ok()
        .map(|at| witnesses[at].base_kind.is_some())
        .ok_or("transfer reference has no current witness".into())
}
fn capacity(
    frame: &DecodedFrame,
    inputs: &CounterInputs<'_>,
    wing: &AirWingRow,
) -> Result<i32, String> {
    let subtype = match wing.pool_subunit {
        Recorded::Value(Some(index)) => index,
        _ => return Err("pool sub-unit definition is unavailable".into()),
    };
    let units = inputs.content.wing_types.as_ref().map_err(Clone::clone)?;
    let name = units
        .name(subtype)
        .ok_or("pool sub-unit index is absent from content")??;
    let bases = frame
        .complete::<tables::AirBases>()
        .map_err(|e| e.to_string())?;
    let base = |handle| {
        bases
            .binary_search_by_key(&handle, |row| row.id)
            .ok()
            .map(|at| &bases[at])
    };
    if units.category(name.as_bytes())? & (1_u64 << 33) != 0 {
        return base(wing.air_base)
            .map(|base| base.capacity)
            .ok_or("capacity's pool base is unavailable".into());
    }
    let current: &AirBaseRow = if wing.base_override != 0 && resolved_transfer(frame, wing)? {
        base(wing.base_override)
    } else {
        base(wing.air_base)
    }
    .ok_or("current air base is unavailable")?;
    let kind = if current.state.is_some() {
        0
    } else {
        let ships = frame
            .complete::<tables::ShipModels>()
            .map_err(|e| e.to_string())?;
        let host = ships
            .binary_search_by_key(&current.host, |row| row.ship)
            .ok()
            .map(|at| &ships[at])
            .ok_or("carrier host model is unavailable")?;
        carrier_kind(host)?
    };
    Ok(units.air_wing_sizes(name.as_bytes(), inputs.equipment.map_err(str::to_owned)?)?[kind])
}
fn carrier_kind(host: &replay_data::rows::ShipModelRow) -> Result<usize, String> {
    if host.formation_flags & 0x80 != 0 {
        return Ok(3);
    }
    let Recorded::Value(Some(kind @ 1..=2)) = host.air_capacity_kind else {
        return Err("carrier module-capacity subtype is unavailable or unsupported".into());
    };
    Ok(kind as usize)
}
pub fn ratio(planes: i32, capacity: i32) -> i64 {
    if capacity <= 0 {
        0
    } else {
        i64::from(planes).wrapping_mul(10_000_000_000) / (i64::from(capacity) * 100_000)
    }
}
fn hover(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    item: &Item,
    representative: &Values<'_>,
    capacity: Option<i32>,
) -> Tooltip {
    let mut tip = Tooltip::default();
    let loc = inputs.localisation;
    let name = (|| -> Result<String, String> {
        if item.members.len() == 1 {
            let Recorded::Value(Some(name)) = &representative.wing.name else {
                return Err("wing's stored name is unavailable".into());
            };
            let name = name.split(|&byte| byte == 0).next().unwrap_or_default();
            return std::str::from_utf8(name)
                .map(str::to_owned)
                .map_err(|e| e.to_string());
        }
        let Recorded::Value(Some(index)) = representative.wing.pool_subunit else {
            return Err("pool type is unavailable".into());
        };
        let units = inputs.content.wing_types.as_ref().map_err(Clone::clone)?;
        let key = units
            .name(index)
            .ok_or("wing type index is absent from content")??;
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let country = representative
            .country
            .and_then(|slot| countries.iter().find(|row| row.canonical_slot == slot))
            .ok_or("effective country is unavailable")?;
        let tag = std::str::from_utf8(&country.tag).map_err(|e| e.to_string())?;
        let kind = viewer_foundation::tooltip_text::wing_short_name(loc, tag, key)?;
        loc.localise_with(
            b"AIRWING_MAP_STACK_HEADER",
            &[
                ("TYPE", Argument::Text(&kind)),
                ("COUNT", Argument::Int32(item.members.len() as i32)),
            ],
        )
        .map_err(|e| e.to_string())
    })();
    tip.line(
        false,
        0,
        "Air grid header",
        name.and_then(|name| {
            if item.members.len() == 1 {
                loc.localise_with(b"HEADER", &[("KEY", Argument::Text(&name))])
                    .map_err(|e| e.to_string())
            } else {
                Ok(name + "\n")
            }
        }),
    );
    tip.line(
        true,
        0,
        "Air grid type icon",
        representative
            .archetype_frame
            .ok_or("primary archetype icon frame is unavailable".into())
            .and_then(|frame| {
                loc.localise_with(
                    b"AIRWING_STACK_INFO_MAP_ICON",
                    &[("ICONFRAME", Argument::Int32(frame))],
                )
                .map_err(|e| e.to_string())
            }),
    );
    tip.line(
        true,
        0,
        "Air grid strength",
        capacity
            .ok_or("wing capacity is unavailable".into())
            .and_then(|limit| {
                let value = ratio(item.planes, limit);
                let key: &[u8] = if value < 50_000 {
                    b"AIRWING_R_STR"
                } else if value == 50_000 {
                    b"AIRWING_H_STR"
                } else {
                    b"AIRWING_G_STR"
                };
                loc.localise_with(
                    key,
                    &[
                        ("COUNT", Argument::Int32(item.planes)),
                        ("LIMIT", Argument::Int32(limit)),
                        ("VALUE", Argument::Fixed5(value)),
                    ],
                )
                .map_err(|e| e.to_string())
            }),
    );
    tip
}

#[cfg(test)]
mod tests {
    use super::{AirWingRow, BTreeMap, DecodedFrame, Values, and, carrier_bases, carrier_kind, group_color, or, ratio, summary};
    fn values(wing: &AirWingRow) -> Values<'_> {
        Values {
            wing,
            icon_frame: 1,
            archetype_frame: None,
            country: Some(1),
            capacity: Ok(100),
            no_mission: Some(false),
            no_region: Some(false),
            action: Some(4),
        }
    }

    #[test]
    fn production_fold_keeps_mixed_actions_training_stop_and_attached_coverage() {
        let training = AirWingRow {
            mission_mask: 0x800,
            map_flags: Some(0b0111).into(),
            ..Default::default()
        };
        let ordinary = AirWingRow {
            map_flags: Some(0b1101).into(),
            ..Default::default()
        };
        let a = values(&training);
        let mut b = values(&ordinary);
        b.action = Some(3);
        let mixed = summary(7, &[&a, &b]);
        assert_eq!(mixed.action, Some(1));
        assert_eq!(mixed.capacity, Some(200));
        assert_eq!(
            mixed.markers,
            [Some(false), Some(false), Some(true), Some(false)]
        );
        assert!(mixed.refused.is_empty());
        let uncovered = summary(7, &[&a]);
        assert_eq!(
            uncovered.markers,
            [Some(false), Some(true), Some(false), Some(true)]
        );
        let idle = Values {
            no_mission: Some(true),
            no_region: Some(true),
            ..values(&training)
        };
        assert_eq!(
            summary(0, &[&idle]).markers,
            [Some(true), Some(false), Some(false), Some(true)]
        );
        assert_eq!(summary(7, &[&b, &a]).markers[3], Some(false));
    }

    #[test]
    fn production_fold_preserves_wrapping_capacity_and_independent_refusals() {
        let row = AirWingRow::default();
        let a = Values {
            capacity: Ok(i32::MAX),
            ..values(&row)
        };
        let mut b = Values {
            capacity: Ok(1),
            ..values(&row)
        };
        assert_eq!(summary(0, &[&a, &b]).capacity, Some(i32::MIN));
        b.capacity = Err("synthetic capacity refusal".into());
        b.action = None;
        let result = summary(0, &[&a, &b]);
        assert_eq!(result.capacity, None);
        assert_eq!(result.action, None);
        assert_eq!(result.markers, [Some(false); 4]);
        assert!(
            result
                .refused
                .contains_key("Air grid capacity: synthetic capacity refusal")
        );
        assert!(
            result
                .refused
                .contains_key("Air grid action frame: transfer reference resolution unavailable")
        );
    }

    #[test]
    fn carrier_kind_uses_the_retained_predicate_instead_of_positive_equipment_flags() {
        use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
        let mut host = ShipModelRow {
            air_capacity_kind: Some(1).into(),
            counter: Some(ShipCounterInputs {
                positive_equipment_flags: u64::MAX,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(carrier_kind(&host), Ok(1));
        host.air_capacity_kind = Some(2).into();
        host.counter.as_mut().unwrap().positive_equipment_flags = 0;
        assert_eq!(carrier_kind(&host), Ok(2));
        for value in [None, Some(0), Some(3), Some(u32::MAX)] {
            host.air_capacity_kind = value.into();
            assert!(carrier_kind(&host).is_err());
        }
        host.formation_flags = 0x80;
        host.air_capacity_kind = None.into();
        assert_eq!(carrier_kind(&host), Ok(3));
    }
    #[test]
    fn carrier_grid_visits_all_bases_in_member_then_ship_order_without_an_intel_gate() {
        use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
        use viewer_unit_reads::counter_order::Hull;
        let models: Vec<_> = [91, 92, 0, 93]
            .into_iter()
            .map(|base| ShipModelRow {
                counter: Some(ShipCounterInputs {
                    carrier_air_base: base,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let ships = super::super::super::naval::ShipInputs {
            equipment: Err("unreached"),
            tables: super::super::super::naval::ShipTables::from_rosters(
                [
                    (
                        4,
                        vec![Hull {
                            strength: 0,
                            model: &models[3],
                        }],
                    ),
                    (
                        8,
                        models[..3]
                            .iter()
                            .map(|model| Hull { strength: 0, model })
                            .collect(),
                    ),
                ]
                .into(),
            ),
        };
        assert_eq!(carrier_bases(&ships, &[8, 4, 8]), [91, 92, 93, 91, 92]);
        assert!(carrier_bases(&ships, &[]).is_empty());
    }

    #[test]
    fn group_color_refuses_non_finite_components_and_preserves_valid_colors() {
        use replay_data::{rows::OrdersGroupPlanRow, rows::TableId, rows::TableRows};
        use replay_frame::{frame::Metrics, frame::TableView};
        let frame_with = |rows: Vec<OrdersGroupPlanRow>| {
            let mut frame = DecodedFrame {
                branch: 1,
                hour: 1,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| {
                    TableView::Unavailable {
                        reason: "fixture".into(),
                    }
                    .into()
                }),
            };
            *frame.tables[TableId::OrdersGroupPlans.index()] =
                TableView::Complete(TableRows::OrdersGroupPlans(rows));
            frame
        };
        let valid = [0.2f32, 0.4, 0.6, 1.0];
        let frame = frame_with(vec![
            OrdersGroupPlanRow {
                group: 7,
                red: valid[0].to_bits(),
                green: valid[1].to_bits(),
                blue: valid[2].to_bits(),
                alpha: valid[3].to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 8,
                red: f32::NAN.to_bits(),
                green: 0.4f32.to_bits(),
                blue: 0.6f32.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 9,
                red: 0.2f32.to_bits(),
                green: f32::INFINITY.to_bits(),
                blue: 0.6f32.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 10,
                red: 0.2f32.to_bits(),
                green: 0.4f32.to_bits(),
                blue: f32::NEG_INFINITY.to_bits(),
                alpha: 1.0f32.to_bits(),
                ..Default::default()
            },
            OrdersGroupPlanRow {
                group: 11,
                red: 0.2f32.to_bits(),
                green: 0.4f32.to_bits(),
                blue: 0.6f32.to_bits(),
                alpha: f32::NAN.to_bits(),
                ..Default::default()
            },
        ]);
        let mut refused = BTreeMap::new();
        assert_eq!(group_color(&frame, 0, &mut refused), None);
        assert!(refused.is_empty());
        assert_eq!(group_color(&frame, 7, &mut refused), Some(valid));
        assert!(refused.is_empty());
        for group in [8, 9, 10, 11] {
            let mut refused = BTreeMap::new();
            assert_eq!(group_color(&frame, group, &mut refused), None);
            assert!(
                refused
                    .keys()
                    .any(|key| key.contains("non-finite") && key.contains(&group.to_string())),
                "{refused:?}"
            );
        }
        let mut absent = BTreeMap::new();
        assert_eq!(group_color(&frame, 99, &mut absent), None);
        assert!(
            absent
                .contains_key("Air grid group marker: current group colour is unavailable")
        );
    }

    #[test]
    fn group_color_inspects_retained_rows_of_truncated_orders_group_plans() {
        use replay_data::{rows::OrdersGroupPlanRow, rows::TableId, rows::TableRows};
        use replay_frame::{frame::Metrics, frame::TableView};
        let valid = [0.2f32, 0.4, 0.6, 1.0];
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::OrdersGroupPlans.index()] = TableView::Truncated {
            rows: TableRows::OrdersGroupPlans(vec![
                OrdersGroupPlanRow {
                    group: 7,
                    red: valid[0].to_bits(),
                    green: valid[1].to_bits(),
                    blue: valid[2].to_bits(),
                    alpha: valid[3].to_bits(),
                    ..Default::default()
                },
                OrdersGroupPlanRow {
                    group: 8,
                    red: f32::NAN.to_bits(),
                    green: 0.4f32.to_bits(),
                    blue: 0.6f32.to_bits(),
                    alpha: 1.0f32.to_bits(),
                    ..Default::default()
                },
            ]),
            missing: 1,
        };
        let mut refused = BTreeMap::new();
        assert_eq!(group_color(&frame, 7, &mut refused), Some(valid));
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(group_color(&frame, 8, &mut refused), None);
        assert!(
            refused
                .keys()
                .any(|key| key.contains("non-finite") && key.contains('8')),
            "{refused:?}"
        );
        let mut absent = BTreeMap::new();
        assert_eq!(group_color(&frame, 99, &mut absent), None);
        assert!(
            absent.contains_key("Air grid group marker: current group colour is unavailable")
        );
    }

    #[test]
    fn known_dominating_flags_do_not_refuse_unreached_operands() {
        assert_eq!(and(Some(false), None), Some(false));
        assert_eq!(or(Some(true), None), Some(true));
        assert_eq!(and(Some(true), None), None);
        assert_eq!(or(Some(false), None), None);
    }
    #[test]
    fn strength_ratio_keeps_signed_capacity_gates_and_integer_division() {
        assert_eq!(ratio(7, 10), 70_000);
        assert_eq!(ratio(7, 0), 0);
        assert_eq!(ratio(7, -10), 0);
        assert_eq!(ratio(-1, 3), -33_333);
    }
}
