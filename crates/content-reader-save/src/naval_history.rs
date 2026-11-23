
use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, Item, block, boolean, bytes, field, integer, invalid, null_permitted_handle,
    object_id, required_block, required_bytes, scalar, typed_handle,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub accident_reports: BTreeMap<Vec<u8>, Vec<super::naval_accidents::Report>>,
    pub sunk_convoys: Vec<SunkConvoy>,
    pub sunk_ships: Vec<SunkShip>,
    pub results: BTreeMap<u64, NavalResult>,
    pub supremacy_regions: Option<BTreeMap<u32, SupremacyRegion>>,
    pub railways: Option<BTreeMap<u32, Railway>>,
    pub theatre_countries: Option<BTreeMap<Vec<u8>, TheatreCountry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Railway {
    pub levels: Vec<i32>,
    pub cooldown: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayAdjacency {
    neighbors: Vec<Vec<u32>>,
}

impl RailwayAdjacency {
    pub fn read(layers: &content_reader_layers::ResolvedLayers) -> io::Result<Self> {
        let files = content_reader_key_names::terrain::read_default_map(layers)?;
        let definitions = layers.read(&files.file("definitions")?)?;
        let bitmap = layers.read(&files.file("provinces")?)?;
        let authored = content_reader_map::movement::adjacency::read_authored(layers, &files)?;
        Self::parse(&definitions, &bitmap, authored.as_deref())
    }

    pub fn parse(definitions: &[u8], bitmap: &[u8], authored: Option<&[u8]>) -> io::Result<Self> {
        let mut definitions = content_reader_map::map::read_definitions(definitions)?;
        let image = content_reader_map::map::decode_provinces_bmp(bitmap, &mut definitions)?;
        Self::derive(
            &image,
            definitions.provinces.len(),
            authored.unwrap_or_default(),
        )
    }

    pub fn neighbors(&self, province: u32) -> Option<&[u32]> {
        (province != 0)
            .then(|| self.neighbors.get(province as usize).map(Vec::as_slice))
            .flatten()
    }

    fn derive(
        image: &content_reader_map::map::ProvinceImage,
        count: usize,
        authored: &[u8],
    ) -> io::Result<Self> {
        use content_reader_map::movement::adjacency;
        let width = image.width as usize;
        let ids: Vec<u32> = image
            .province_ids
            .chunks(width.max(1))
            .rev()
            .flatten()
            .copied()
            .collect();
        let mut neighbors = adjacency::raster(width, image.height as usize, &ids, count)?;
        adjacency::append_authored(&mut neighbors, &mut Default::default(), authored)?;
        Ok(Self { neighbors })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TheatreCountry {
    pub land: Vec<LandTheatre>,
    pub air: Vec<AirTheatre>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandTheatre {
    pub id: u64,
    pub volunteers_country: Option<Vec<u8>>,
    pub groups: Vec<LandGroup>,
    pub units: Vec<u64>,
    pub area_representatives: Vec<u32>,
    pub definitions: Vec<OrdersGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandGroup {
    pub id: u64,
    pub priority: i32,
    pub name: Option<Vec<u8>>,
    pub members: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirTheatre {
    pub id: u64,
    pub country: Vec<u8>,
    pub name: Option<Vec<u8>>,
    pub groups: Vec<AirGroup>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirGroup {
    pub id: u64,
    pub theatre: u64,
    pub name: Vec<u8>,
    pub wings: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrdersGroup {
    pub id: u64,
    pub name: Option<Vec<u8>>,
    pub theatre_ordinal: u32,
    pub is_army_group: bool,
    pub motorization_level: i32,
    pub leader: Option<u64>,
    pub leader_unit: Option<u64>,
    pub members: Vec<u64>,
    pub children: Vec<u64>,
    pub withdrawing: bool,
    pub distance: Option<i32>,
    pub invasion_source_distance: Option<i32>,
    pub hq_nearest_front: Option<i32>,
    pub target_template: Option<u64>,
    pub hq_deployed: bool,
    pub hq_queued: bool,
    pub hq_deployment: Option<HqDeployment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HqDeployment {
    pub country: Vec<u8>,
    pub priority: i32,
    pub equipment: super::production::Pool,
    pub required: Vec<(Vec<u8>, Decimal)>,
    pub assembled_manpower: i32,
    pub requested_manpower: i32,
    pub order: i32,
    pub requisitioned: bool,
}

fn hq_deployment(items: &[Item]) -> io::Result<HqDeployment> {
    let required = required_block(items, b"hq_requested_equipment")?
        .iter()
        .map(|entry| {
            let token = entry
                .key
                .as_ref()
                .ok_or_else(|| invalid("headquarters requirement lacks a definition key"))?;
            Ok((token.clone(), Decimal::parse(scalar(&entry.value)?)?))
        })
        .collect::<io::Result<_>>()?;
    Ok(HqDeployment {
        country: required_bytes(items, b"country")?,
        priority: bytes(items, b"priority")?
            .map(|value| integer(&value))
            .transpose()?
            .unwrap_or(1),
        equipment: super::production::parse_pool(required_block(
            items,
            b"hq_assembled_equipment",
        )?)?,
        required,
        assembled_manpower: integer(&required_bytes(items, b"hq_assembled_manpower")?)?,
        requested_manpower: integer(&required_bytes(items, b"hq_requested_manpower")?)?,
        order: integer(&required_bytes(items, b"hq_deploy_order")?)?,
        requisitioned: boolean(
            field(items, b"hq_requisitioned_from_army")?
                .ok_or_else(|| invalid("headquarters requisition flag missing"))?,
        )?,
    })
}

fn railways(items: &[Item]) -> io::Result<Option<BTreeMap<u32, Railway>>> {
    let Some(manager) = field(items, b"rail_way")? else {
        return Ok(None);
    };
    let mut result = BTreeMap::new();
    for entry in required_block(block(manager)?, b"rail_way")? {
        let province = integer(
            entry
                .key
                .as_deref()
                .ok_or_else(|| invalid("saved railway info lacks province key"))?,
        )?;
        let fields = block(&entry.value)?;
        let levels = field(fields, b"rail_way")?
            .map(block)
            .transpose()?
            .unwrap_or_default()
            .iter()
            .map(|entry| {
                if entry.key.is_some() {
                    return Err(invalid("railway levels must be an unkeyed integer list"));
                }
                integer::<i32>(scalar(&entry.value)?)
            })
            .collect::<io::Result<Vec<_>>>()?;
        let cooldown = bytes(fields, b"cooldown")?
            .map(|value| integer::<i32>(&value))
            .transpose()?;
        if cooldown.is_some_and(|value| value <= 0) {
            return Err(invalid("railway writer omits signed-nonpositive cooldown"));
        }
        if result
            .insert(province, Railway { levels, cooldown })
            .is_some()
        {
            return Err(invalid("duplicate saved railway province"));
        }
    }
    Ok(Some(result))
}

fn orders_groups(theatre: &[Item]) -> io::Result<Vec<OrdersGroup>> {
    let mut roster_ordinals = [0_u32; 2];
    theatre
        .iter()
        .filter(|entry| {
            matches!(
                entry.key.as_deref(),
                Some(b"orders_group" | b"field_marshal_group")
            )
        })
        .map(|entry| {
            let roster = usize::from(entry.key.as_deref() == Some(b"field_marshal_group"));
            let theatre_ordinal = roster_ordinals[roster];
            roster_ordinals[roster] = theatre_ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("saved theatre orders-group ordinal exceeds u32"))?;
            let fields = block(&entry.value)?;
            let members = fields
                .iter()
                .filter(|entry| entry.key.as_deref() == Some(b"member"))
                .map(|entry| {
                    let member = block(&entry.value)?;
                    typed_handle(
                        field(member, b"unit")?
                            .ok_or_else(|| invalid("saved orders group member lacks its unit"))?,
                    )
                })
                .collect::<io::Result<Vec<_>>>()?;
            let children = fields
                .iter()
                .filter(|entry| entry.key.as_deref() == Some(b"orders_group"))
                .map(|entry| typed_handle(&entry.value))
                .collect::<io::Result<Vec<_>>>()?;
            Ok(OrdersGroup {
                id: object_id(fields)?,
                name: bytes(fields, b"name")?,
                theatre_ordinal,
                is_army_group: boolean(field(fields, b"field_marshal_group")?.ok_or_else(
                    || invalid("saved orders group lacks its unconditional field_marshal_group"),
                )?)?,
                motorization_level: integer(&required_bytes(fields, b"motorization_level")?)?,
                leader: field(fields, b"leader")?.map(typed_handle).transpose()?,
                leader_unit: field(fields, b"leader_unit")?
                    .map(typed_handle)
                    .transpose()?,
                members,
                children,
                withdrawing: field(fields, b"withdrawing")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                distance: bytes(fields, b"distance")?
                    .map(|value| integer::<i32>(&value))
                    .transpose()?
                    .map(|value| {
                        if value < 0 {
                            Err(invalid("orders group writer omits negative distance"))
                        } else {
                            Ok(value)
                        }
                    })
                    .transpose()?,
                hq_nearest_front: bytes(fields, b"hq_nearest_front_province_id")?
                    .map(|value| integer::<i32>(&value))
                    .transpose()?,
                target_template: field(fields, b"target_template")?
                    .map(typed_handle)
                    .transpose()?,
                hq_deployed: field(fields, b"deployed")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                hq_queued: field(fields, b"deploy_queued")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                hq_deployment: field(fields, b"hq_deploy_distributable")?
                    .map(|value| hq_deployment(block(value)?))
                    .transpose()?,
                invasion_source_distance: bytes(fields, b"hq_distance_to_naval_invasion_source")?
                    .map(|value| integer::<i32>(&value))
                    .transpose()?
                    .map(|value| {
                        if value < 0 {
                            Err(invalid(
                                "orders group writer omits negative invasion-source distance",
                            ))
                        } else {
                            Ok(value)
                        }
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn area_representatives(fields: &[Item]) -> io::Result<Vec<u32>> {
    let mut representatives = Vec::new();
    for entry in fields
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"area"))
    {
        for item in block(&entry.value)? {
            if item.key.is_some() {
                return Err(invalid(
                    "saved theatre area must be an unkeyed representative-province list",
                ));
            }
            representatives.push(integer::<i32>(scalar(&item.value)?)? as u32);
        }
    }
    Ok(representatives)
}

fn theatre_countries(
    countries: super::CountriesView<'_>,
) -> io::Result<Option<BTreeMap<Vec<u8>, TheatreCountry>>> {
    if !countries.is_present() {
        return Ok(None);
    }
    let mut result = BTreeMap::new();
    for country in countries.iter() {
        let tag = country
            .key
            .clone()
            .ok_or_else(|| invalid("saved theatre country lacks tag"))?;
        let fields = block(&country.value)?;
        let mut row = TheatreCountry::default();
        if let Some(theatres) = field(fields, b"theatres")? {
            for entry in block(theatres)?
                .iter()
                .filter(|entry| entry.key.as_deref() == Some(b"theatre"))
            {
                let fields = block(&entry.value)?;
                let groups = fields
                    .iter()
                    .filter(|entry| entry.key.as_deref() == Some(b"theater_group"))
                    .map(|entry| {
                        let fields = block(&entry.value)?;
                        Ok(LandGroup {
                            id: object_id(fields)?,
                            priority: integer(&required_bytes(fields, b"priority")?)?,
                            name: bytes(fields, b"name")?,
                            members: fields
                                .iter()
                                .filter(|entry| entry.key.as_deref() == Some(b"orders_group"))
                                .map(|entry| typed_handle(&entry.value))
                                .collect::<io::Result<_>>()?,
                        })
                    })
                    .collect::<io::Result<_>>()?;
                row.land.push(LandTheatre {
                    id: object_id(fields)?,
                    volunteers_country: bytes(fields, b"volunteers_theatre")?,
                    groups,
                    units: fields
                        .iter()
                        .filter(|entry| entry.key.as_deref() == Some(b"unit"))
                        .map(|entry| typed_handle(&entry.value))
                        .collect::<io::Result<_>>()?,
                    area_representatives: area_representatives(fields)?,
                    definitions: orders_groups(fields)?,
                });
            }
        }
        for entry in fields
            .iter()
            .filter(|entry| entry.key.as_deref() == Some(b"air_theatre"))
        {
            let fields = block(&entry.value)?;
            let groups = fields.iter().filter(|entry| entry.key.as_deref() == Some(b"air_group")).map(|entry| {
                let fields = block(&entry.value)?;
                let wings = field(fields, b"air_wing")?.map(|value| {
                    block(value)?.iter().map(|entry| {
                        if entry.key.is_some() { return Err(invalid("saved air-group wings must be an anonymous reference list")); }
                        typed_handle(&entry.value)
                    }).collect::<io::Result<Vec<_>>>()
                }).transpose()?.unwrap_or_default();
                Ok(AirGroup {
                    id: object_id(fields)?,
                    theatre: typed_handle(field(fields, b"air_theatre")?.ok_or_else(|| invalid("air group lacks its parent theatre reference"))?)?,
                    name: bytes(fields, b"name")?.ok_or_else(|| invalid("air group lacks its unconditional name field"))?,
                    wings,
                })
            }).collect::<io::Result<_>>()?;
            row.air.push(AirTheatre {
                id: object_id(fields)?,
                country: required_bytes(fields, b"country")?,
                name: bytes(fields, b"name")?,
                groups,
            });
        }
        if result.insert(tag, row).is_some() {
            return Err(invalid("duplicate saved theatre country tag"));
        }
    }
    Ok(Some(result))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupremacyRegion {
    pub values: Option<Vec<SupremacyValue>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupremacyValue {
    pub country: Vec<u8>,
    pub current: Decimal,
    pub base_target: Decimal,
    pub target: Decimal,
    pub individual_ratio: Decimal,
    pub previous: Decimal,
    pub decline_from: Decimal,
}

fn supremacy_regions(items: &[Item]) -> io::Result<Option<BTreeMap<u32, SupremacyRegion>>> {
    let Some(value) = field(items, b"region")? else {
        return Ok(None);
    };
    let mut regions = BTreeMap::new();
    for entry in block(value)? {
        let slot = integer::<u32>(
            entry
                .key
                .as_deref()
                .ok_or_else(|| invalid("saved naval supremacy region lacks runtime slot"))?,
        )?;
        let fields = block(&entry.value)?;
        let values = field(fields, b"dominance")?
            .map(|dominance| {
                let Some(value) = field(block(dominance)?, b"values")? else {
                    return Ok(Vec::new());
                };
                block(value)?
                    .iter()
                    .map(|entry| {
                        if entry.key.is_some() {
                            return Err(invalid(
                                "naval supremacy values must contain anonymous country/value pairs",
                            ));
                        }
                        let [country, values] = block(&entry.value)? else {
                            return Err(invalid(
                                "naval supremacy pair requires one country and one values block",
                            ));
                        };
                        if country.key.is_some() || values.key.is_some() {
                            return Err(invalid("naval supremacy pair elements must be anonymous"));
                        }
                        let country = scalar(&country.value)?.to_vec();
                        if country.is_empty() {
                            return Err(invalid("naval supremacy country tag is empty"));
                        }
                        let fields = block(&values.value)?;
                        let scalar = |key: &[u8]| Decimal::parse(&required_bytes(fields, key)?);
                        Ok(SupremacyValue {
                            country,
                            current: scalar(b"current")?,
                            base_target: scalar(b"base_target")?,
                            target: scalar(b"target")?,
                            individual_ratio: scalar(b"individual_ratio")?,
                            previous: scalar(b"previous")?,
                            decline_from: scalar(b"decline_from")?,
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?;
        if regions.insert(slot, SupremacyRegion { values }).is_some() {
            return Err(invalid(
                "duplicate saved naval supremacy runtime region slot",
            ));
        }
    }
    Ok(Some(regions))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkConvoy {
    pub month: i32,
    pub killer_country: Vec<u8>,
    pub owner_country: Vec<u8>,
    pub convoys: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SunkShip {
    pub ship_name: Option<Vec<u8>>,
    pub attribution_name: Option<Vec<u8>>,
    pub victim_country: Vec<u8>,
    pub attacker_country: Vec<u8>,
    pub sink_date: GameDate,
    pub class: Vec<u8>,
    pub attribution_class: Vec<u8>,
    pub model_level: i32,
    pub victim_equipment_variant: Option<u64>,
    pub damage_source_reference: Option<u64>,
    pub province: Option<u32>,
    pub battle: u64,
    pub cause_record: bool,
    pub assisted_kill: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalResult {
    pub id: u64,
    pub province: u32,
    pub result_date: GameDate,
    pub retire_date: GameDate,
    pub category: u32,
    pub port_strike: bool,
    pub naval_strike: bool,
    pub sides: [ResultSide; 2],
    pub admitted: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultSide {
    pub members: Vec<ResultMember>,
    pub details: super::combat_details::ResultSideDetails,
    pub countries: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultMember {
    pub id: u64,
    pub details: super::combat_details::NavalMemberDetails,
    pub sunk_by: Option<Vec<u8>>,
}

fn date(items: &[Item], key: &[u8]) -> io::Result<GameDate> {
    GameDate::parse(&required_bytes(items, key)?)
}

fn required_boolean(items: &[Item], key: &[u8]) -> io::Result<bool> {
    boolean(field(items, key)?.ok_or_else(|| {
        invalid(format!(
            "naval history lacks {}",
            String::from_utf8_lossy(key)
        ))
    })?)
}

fn country_list(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let Some(value) = field(items, key)? else {
        return Ok(Vec::new());
    };
    block(value)?
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("naval result country list contains a keyed entry"));
            }
            let tag = scalar(&entry.value)?;
            if tag.is_empty() {
                return Err(invalid("naval result country tag is empty"));
            }
            Ok(tag.to_vec())
        })
        .collect()
}

pub(crate) fn sunk_ship(items: &[Item]) -> io::Result<SunkShip> {
    let assist = field(items, b"assist")?.map(boolean).transpose()?;
    if assist == Some(false) {
        return Err(invalid("sunk ship writer omits an unset assist flag"));
    }
    Ok(SunkShip {
        ship_name: bytes(items, b"name")?,
        attribution_name: bytes(items, b"killer_name")?,
        victim_country: required_bytes(items, b"country")?,
        attacker_country: required_bytes(items, b"killer_country")?,
        sink_date: date(items, b"date")?,
        class: required_bytes(items, b"definition")?,
        attribution_class: required_bytes(items, b"killer_definition")?,
        model_level: integer(&required_bytes(items, b"level")?)?,
        victim_equipment_variant: field(items, b"equipment_variant")?
            .map(typed_handle)
            .transpose()?,
        damage_source_reference: field(items, b"air_wing")?.map(typed_handle).transpose()?,
        province: field(items, b"location")?
            .map(|value| integer::<i32>(scalar(value)?).map(|value| value as u32))
            .transpose()?,
        battle: null_permitted_handle(
            field(items, b"battle")?.ok_or_else(|| invalid("sunk ship lacks battle"))?,
        )?,
        cause_record: required_boolean(items, b"convoy")?,
        assisted_kill: assist.unwrap_or(false),
    })
}

fn result_side(items: &[Item]) -> io::Result<ResultSide> {
    Ok(ResultSide {
        details: super::combat_details::result_side(items)?,
        members: items
            .iter()
            .filter(|entry| entry.key.as_deref() == Some(b"ship"))
            .map(|entry| {
                let items = block(&entry.value)?;
                Ok(ResultMember {
                    details: super::combat_details::naval_member(items)?,
                    id: typed_handle(
                        field(items, b"unique_id")?
                            .ok_or_else(|| invalid("naval result member lacks unique_id"))?,
                    )?,
                    sunk_by: bytes(required_block(items, b"cached_info")?, b"sunk_by")?,
                })
            })
            .collect::<io::Result<_>>()?,
        countries: country_list(items, b"countries")?,
    })
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack {
        supremacy_regions: supremacy_regions(items)?,
        railways: railways(items)?,
        theatre_countries: theatre_countries(countries)?,
        accident_reports: super::naval_accidents::parse(countries)?,
        ..Default::default()
    };
    if let Some(value) = field(items, b"sunk_convoys_history")? {
        for entry in block(value)? {
            if entry.key.as_deref() != Some(b"sunk_convoy") {
                return Err(invalid("unexpected sunk convoy history entry"));
            }
            let fields = block(&entry.value)?;
            pack.sunk_convoys.push(SunkConvoy {
                month: integer(&required_bytes(fields, b"month")?)?,
                killer_country: required_bytes(fields, b"killer_country")?,
                owner_country: required_bytes(fields, b"owner")?,
                convoys: integer(&required_bytes(fields, b"convoys")?)?,
            });
        }
    }
    if let Some(value) = field(items, b"history")? {
        for entry in block(value)? {
            if entry.key.as_deref() != Some(b"sunk_ship") {
                return Err(invalid("unexpected retained sunk ship history entry"));
            }
            pack.sunk_ships.push(sunk_ship(block(&entry.value)?)?);
        }
    }
    for entry in items
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"naval_combat_result"))
    {
        let fields = block(&entry.value)?;
        let id = object_id(fields)?;
        let result = NavalResult {
            id,
            province: integer::<i32>(&required_bytes(fields, b"location")?)? as u32,
            result_date: date(fields, b"date")?,
            retire_date: date(fields, b"to_discard_date")?,
            category: integer::<i32>(&required_bytes(fields, b"importance")?)? as u32,
            port_strike: required_boolean(fields, b"port_strike")?,
            naval_strike: required_boolean(fields, b"naval_strike")?,
            sides: [
                result_side(required_block(fields, b"attacker")?)?,
                result_side(required_block(fields, b"defender")?)?,
            ],
            admitted: country_list(fields, b"shown_to_countries")?,
        };
        if pack.results.insert(id, result).is_some() {
            return Err(invalid("duplicate retained naval result handle"));
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests;
