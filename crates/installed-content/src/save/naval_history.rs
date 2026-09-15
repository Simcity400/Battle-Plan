//! Retained naval state. Native save order is retained; anonymous sunk ships
//! never acquire an invented identity from their name or equipment variant.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, GameDate, Item, block, boolean, bytes, field, integer, invalid, null_permitted_handle,
    object_id, required_block, required_bytes, scalar, typed_handle,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// 1F2A20 omits the entire container precisely when its native count is zero.
    pub sunk_convoys: Vec<SunkConvoy>,
    /// 1F2A20 emits history.sunk_ship in increasing native vector position.
    pub sunk_ships: Vec<SunkShip>,
    pub results: BTreeMap<u64, NavalResult>,
    /// CGameState's region keys are runtime roster slots, not authored IDs.
    /// An absent region root remains distinct from a complete empty root.
    pub supremacy_regions: Option<BTreeMap<u32, SupremacyRegion>>,
    pub railways: Option<BTreeMap<u32, Railway>>,
    pub theatre_countries: Option<BTreeMap<Vec<u8>, TheatreCountry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Railway {
    /// E80C40 serializes the native neighbour-indexed array, not sorted edges.
    pub levels: Vec<i32>,
    /// E80C40 omits every signed-nonpositive cooldown, not only raw zero.
    pub cooldown: Option<i32>,
}

/// Province-definition neighbor order, derived independently from game content.
/// The saved railway list uses this order, not the sorted live railway pairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayAdjacency {
    neighbors: Vec<Vec<u32>>,
}

impl RailwayAdjacency {
    pub fn read(layers: &crate::ResolvedLayers) -> io::Result<Self> {
        let definitions = layers.read("map/definition.csv")?;
        let bitmap = layers.read("map/provinces.bmp")?;
        let authored = match layers.read("map/adjacencies.csv") {
            Ok(bytes) => Some(bytes),
            // A53290 leaves the raster adjacency intact when this file is absent.
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && !layers
                        .paths()
                        .any(|path| path.eq_ignore_ascii_case("map/adjacencies.csv")) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        Self::parse(&definitions, &bitmap, authored.as_deref())
    }

    pub fn parse(definitions: &[u8], bitmap: &[u8], authored: Option<&[u8]>) -> io::Result<Self> {
        let parsed = crate::map::parse_definitions(definitions)?;
        // 1533EE0 reads 24-bit pixels directly after its 14+40-byte headers and
        // does not read rows for a negative height. The general viewer decoder
        // accepts additional BMP layouts whose native interpretation differs.
        if bitmap.get(10..14) != Some(54_u32.to_le_bytes().as_slice())
            || bitmap.get(14..18) != Some(40_u32.to_le_bytes().as_slice())
            || bitmap
                .get(22..26)
                .is_none_or(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()) <= 0)
        {
            return Err(invalid(
                "rail adjacency requires native 40-byte-header, bottom-up BMP storage with pixels at byte 54",
            ));
        }
        // The engine's template roster uses record positions; its RGB lookup
        // narrows authored IDs to u16. Require their exact common representation
        // here instead of silently accepting reordered or wrapping definitions.
        let text = std::str::from_utf8(definitions)
            .map_err(|_| invalid("rail adjacency definitions are not UTF-8"))?;
        let mut count = 0_usize;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let id = line
                .split(';')
                .next()
                .unwrap()
                .trim()
                .parse::<u16>()
                .map_err(|_| invalid("rail adjacency definition ID exceeds native u16"))?;
            if usize::from(id) != count {
                return Err(invalid(
                    "rail adjacency requires definition IDs to equal their native record positions, starting at zero",
                ));
            }
            count += 1;
        }
        if count != parsed.len() {
            return Err(invalid("rail adjacency definition record census differs"));
        }
        let image = crate::map::decode_provinces_bmp(bitmap, &parsed)?;
        if image.province_ids.contains(&0) {
            return Err(invalid(
                "rail adjacency cannot use sentinel RGB pixels: the native color loader assigns a new province to unmapped colors",
            ));
        }
        Self::derive(&image, count, authored.unwrap_or_default())
    }

    /// None is an unknown province; an empty slice is a defined empty vector.
    pub fn neighbors(&self, province: u32) -> Option<&[u32]> {
        (province != 0)
            .then(|| self.neighbors.get(province as usize).map(Vec::as_slice))
            .flatten()
    }

    fn derive(
        image: &crate::map::ProvinceImage,
        count: usize,
        authored: &[u8],
    ) -> io::Result<Self> {
        let width = image.width as usize;
        let height = image.height as usize;
        if width == 0
            || height == 0
            || width.checked_mul(height) != Some(image.province_ids.len())
            || image.province_ids.iter().any(|&id| id as usize >= count)
        {
            return Err(invalid(
                "rail adjacency raster dimensions or province identities are invalid",
            ));
        }
        let mut result = Self {
            neighbors: vec![Vec::new(); count],
        };
        // A4EDE0 scans the native bitmap in file order. Its four-neighbor order
        // is right, next row, left, previous row, with horizontal seam wrapping.
        // 13F32B0 appends a new endpoint and never sorts or moves existing ones.
        let at = |x: usize, y: usize| image.province_ids[(height - 1 - y) * width + x];
        for y in 0..height {
            for x in 0..width {
                let source = at(x, y);
                if source == 0 {
                    continue;
                }
                let right_x = if x + 1 == width { 0 } else { x + 1 };
                let left_x = if x == 0 { width - 1 } else { x - 1 };
                let right = at(right_x, y);
                let next = if y + 1 < height { at(x, y + 1) } else { 0 };
                let left = at(left_x, y);
                let previous = if y > 0 { at(x, y - 1) } else { 0 };
                for target in [right, next, left, previous] {
                    result.bitmap_pair(source, target);
                }
                let diagonal = if y + 1 < height {
                    at(right_x, y + 1)
                } else {
                    0
                };
                let four = [source, right, next, diagonal];
                if four.iter().all(|&id| id != 0)
                    && (0..4).all(|a| (a + 1..4).all(|b| four[a] != four[b]))
                {
                    // A4EDE0 calls A541E0 for an invalid four-province crossing;
                    // that helper adds this diagonal using the same append order.
                    result.bitmap_pair(source, diagonal);
                }
            }
        }
        let mut cursor = AdjacencyCsv {
            bytes: authored,
            position: 0,
        };
        cursor.skip_line();
        while let Some(source) = cursor.field() {
            let source = adjacency_integer(source)?;
            if source < 0 {
                break;
            }
            if source == 0 {
                continue;
            }
            let target = adjacency_integer(
                cursor
                    .field()
                    .ok_or_else(|| invalid("authored adjacency lacks a target"))?,
            )?;
            if target <= 0 || source as usize >= count || target as usize >= count {
                return Err(invalid("authored adjacency references an unknown province"));
            }
            for _ in 0..7 {
                cursor
                    .field()
                    .ok_or_else(|| invalid("authored adjacency lacks its native fields"))?;
            }
            // A53290 includes all crossing types and rule names. They change
            // edge properties, not endpoint membership. New entries append
            // source first, then target; updates retain position. Self-edges
            // pass through 13F32B0 and are retained exactly once.
            result.append(source as u32, target as u32);
            result.append(target as u32, source as u32);
            cursor.skip_line();
        }
        Ok(result)
    }

    fn append(&mut self, source: u32, target: u32) {
        let entries = &mut self.neighbors[source as usize];
        if !entries.contains(&target) {
            entries.push(target);
        }
    }

    fn bitmap_pair(&mut self, source: u32, target: u32) {
        if target != 0 && source != target && !self.neighbors[source as usize].contains(&target) {
            self.append(target, source);
            self.append(source, target);
        }
    }
}

struct AdjacencyCsv<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> AdjacencyCsv<'a> {
    fn field(&mut self) -> Option<&'a [u8]> {
        let start = self.position;
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == 0xff {
                return None;
            }
            if byte == b';' || byte == b'\n' {
                return Some(&self.bytes[start..self.position - 1]);
            }
        }
        None
    }

    fn skip_line(&mut self) {
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position += 1;
            if byte == b'\n' || byte == 0xff {
                break;
            }
        }
    }
}

fn adjacency_integer(field: &[u8]) -> io::Result<i32> {
    let field = std::str::from_utf8(field)
        .map_err(|_| invalid("authored adjacency integer is not UTF-8"))?
        .trim();
    if field.is_empty() {
        return Ok(0);
    }
    field
        .parse()
        .map_err(|_| invalid("authored adjacency requires an exact signed integer"))
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
    /// Ordered stationed-unit references; EED430 emits one `unit` per position.
    pub units: Vec<u64>,
    /// One representative province per controller area, in area order.
    pub area_representatives: Vec<u32>,
    /// Serialized COrdersGroup/CArmyGroup definitions in this theatre, in the
    /// order the writer emits them. Their theatre membership is the nesting.
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

/// One serialized COrdersGroup or CArmyGroup definition inside a land theatre.
///
/// member-vector order, then `leader_unit` only when the HQ provider is
/// non-null, then `leader` only when the leader pointer is non-null, and always
/// `field_marshal_group` and `motorization_level`. An army group additionally
/// lists its ordered child `orders_group={ id type }` references.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrdersGroup {
    pub id: u64,
    pub name: Vec<u8>,
    /// Written unconditionally from the raw +0x39 discriminator byte.
    pub is_army_group: bool,
    /// Written unconditionally from the raw +0x38 byte.
    pub motorization_level: i32,
    /// Omission proves the native +0x88 leader pointer is null.
    pub leader: Option<u64>,
    /// Omission proves the native +0x68 HQ-provider pointer is null.
    pub leader_unit: Option<u64>,
    /// Ordered member unit handles; both native id spaces occur.
    pub members: Vec<u64>,
    /// Ordered child orders-group references of an army group.
    pub children: Vec<u64>,
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
        // 1B3870 writes the integer array only for a positive native count.
        let levels = field(fields, b"rail_way")?
            .map(block)
            .transpose()?
            .unwrap_or_default()
            .iter()
            .map(|entry| {
                if entry.key.is_some() {
                    return Err(invalid("railway levels must be a native integer list"));
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

/// Both definition kinds share one serializer, so they parse identically; the
/// saved `field_marshal_group` boolean, not the block key, decides the kind.
fn orders_groups(theatre: &[Item]) -> io::Result<Vec<OrdersGroup>> {
    theatre
        .iter()
        .filter(|entry| {
            matches!(
                entry.key.as_deref(),
                Some(b"orders_group" | b"field_marshal_group")
            )
        })
        .map(|entry| {
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
                name: required_bytes(fields, b"name")?,
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
            })
        })
        .collect()
}

/// The saved controller-area representatives of one theatre, in stored order.
/// CTheatre serializer EED430 emits a single unkeyed id array; an absent field
/// is an empty list and a keyed entry inside it is refused rather than skipped.
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

fn theatre_countries(items: &[Item]) -> io::Result<Option<BTreeMap<Vec<u8>, TheatreCountry>>> {
    let Some(countries) = field(items, b"countries")? else {
        return Ok(None);
    };
    let mut result = BTreeMap::new();
    for country in block(countries)? {
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
                    // EED430 emits one representative province per controller
                    // area, through area+0x28, not each area's membership, and
                    // it writes them as one unkeyed `area={ id id ... }` array
                    // rather than one `area=` field per area.
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
    /// Missing dominance is retained, including a possible inactive roster slot.
    /// FF11E0 omits values exactly when its native values map is empty.
    pub values: Option<Vec<SupremacyValue>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupremacyValue {
    /// FEC450 emits pairs of country tag and CDominanceValues. Equal tags are
    /// retained because distinct raw reference aliases can serialize equally.
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
    /// C1F090 omits both names in one writer mode; omission proves no bytes.
    pub ship_name: Option<Vec<u8>>,
    pub attribution_name: Option<Vec<u8>>,
    pub victim_country: Vec<u8>,
    pub attacker_country: Vec<u8>,
    pub sink_date: GameDate,
    /// Serialized class token spellings, independent of definition name bytes.
    pub class: Vec<u8>,
    pub attribution_class: Vec<u8>,
    pub model_level: i32,
    /// Omitted for null or unresolved references, not necessarily raw zero.
    pub victim_equipment_variant: Option<u64>,
    pub damage_source_reference: Option<u64>,
    /// C1F090 emits location exactly when the province pointer is nonnull.
    pub province: Option<u32>,
    /// The unconditional battle reference may have both words zero.
    pub battle: u64,
    pub cause_record: bool,
    /// C1F090 emits assist=yes only when the native flag is set.
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
    /// CD7010 emits its +0x20 vector as repeated ship blocks in native order.
    /// These handles can identify ships or convoys.
    pub members: Vec<ResultMember>,
    pub details: super::combat_details::ResultSideDetails,
    /// CD7010 copies +0x38 country references to tags without sorting.
    pub countries: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultMember {
    pub id: u64,
    pub details: super::combat_details::NavalMemberDetails,
    /// 1962D80 serializes snapshot+90 only if nonempty and writer mode permits.
    /// Its exact byte length is the captured entry+B0 presence_value.
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
        // CD7010 and CD73D0 emit these lists iff their count is nonzero.
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

fn sunk_ship(items: &[Item]) -> io::Result<SunkShip> {
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
        // C1F090 token 0x3369 is literally air_wing, regardless of caller type.
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
    let mut pack = Pack {
        supremacy_regions: supremacy_regions(items)?,
        railways: railways(items)?,
        theatre_countries: theatre_countries(items)?,
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
mod tests {
    use super::*;

    fn parse_text(bytes: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(bytes)?.items)
    }

    /// One theatre with an army group, its child army, the army's ordered
    /// members and the theatre group that lists both.
    const ORDER_OF_BATTLE: &[u8] = br#"countries={GER={theatres={theatre={
        id={ id=2 type=67 }
        orders_group={
            id={ id=57 type=53 }
            name="Army 1"
            member={ unit={ id=12456 type=4713 } }
            member={ unit={ id=111 type=51 } }
            leader_unit={ id=12456 type=4713 }
            leader={ id=279 type=4713 }
            field_marshal_group=no
            motorization_level=0
        }
        field_marshal_group={
            orders_group={ id=57 type=53 }
            id={ id=431 type=53 }
            name="Army Group 1"
            field_marshal_group=yes
            motorization_level=-1
        }
        theater_group={
            id={ id=45 type=68 }
            priority=1
            name="British Theater 1"
            orders_group={ id=57 type=53 }
            orders_group={ id=431 type=53 }
        }
    }}}}"#;

    #[test]
    fn orders_group_definitions_keep_native_order_kind_and_conditional_pointers() {
        let pack = parse_text(ORDER_OF_BATTLE).unwrap();
        let theatre = &pack.theatre_countries.as_ref().unwrap()[b"GER".as_slice()].land[0];
        assert_eq!(theatre.definitions.len(), 2);
        let army = &theatre.definitions[0];
        assert_eq!(army.id, (57 << 32) | 53);
        assert_eq!(army.name, b"Army 1");
        assert!(!army.is_army_group);
        assert_eq!(army.motorization_level, 0);
        assert_eq!(army.leader, Some((279 << 32) | 4713));
        assert_eq!(army.leader_unit, Some((12456 << 32) | 4713));
        // Both native unit id spaces occur and their order is preserved.
        assert_eq!(army.members, [(12456 << 32) | 4713, (111 << 32) | 51]);
        assert!(army.children.is_empty());
        let group = &theatre.definitions[1];
        assert_eq!(group.id, (431 << 32) | 53);
        assert!(group.is_army_group);
        assert_eq!(group.motorization_level, -1);
        // An army group has no leader, no HQ provider and no direct units.
        assert_eq!((group.leader, group.leader_unit), (None, None));
        assert!(group.members.is_empty());
        assert_eq!(group.children, [(57 << 32) | 53]);
        // The theatre-group listing still parses as references, not definitions.
        assert_eq!(
            theatre.groups[0].members,
            [(57 << 32) | 53, (431 << 32) | 53]
        );
    }

    #[test]
    fn an_orders_group_without_its_unconditional_fields_is_refused() {
        for removed in [
            "name=\"Army 1\"",
            "field_marshal_group=no",
            "motorization_level=0",
        ] {
            let text = String::from_utf8(ORDER_OF_BATTLE.to_vec())
                .unwrap()
                .replace(removed, "");
            assert!(parse_text(text.as_bytes()).is_err(), "{removed}");
        }
        // A member block without its unit reference is malformed, not empty.
        let text = String::from_utf8(ORDER_OF_BATTLE.to_vec())
            .unwrap()
            .replace("member={ unit={ id=111 type=51 } }", "member={ }");
        assert!(parse_text(text.as_bytes()).is_err());
    }

    fn adjacency_content() -> (Vec<u8>, Vec<u8>) {
        let definitions =
            b"0;0;0;0;land\n1;1;0;0;land\n2;2;0;0;sea\n3;3;0;0;lake\n4;4;0;0;land\n5;5;0;0;land\n"
                .to_vec();
        // File rows are [1,2] then [3,4], each padded to eight bytes.
        let pixels = [0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 3, 0, 0, 4, 0, 0];
        let mut bitmap = vec![0; 54];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&70_u32.to_le_bytes());
        bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&2_i32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bitmap.extend_from_slice(&pixels);
        (definitions, bitmap)
    }

    #[test]
    fn railway_adjacency_keeps_native_raster_diagonal_and_authored_order() {
        let (definitions, bitmap) = adjacency_content();
        let authored = b"From;To;Type;Through;x;y;x;y;rule;comment\n1;5;sea;-1;-1;-1;-1;-1;CANAL;new\n2;2;impassable;-1;-1;-1;-1;-1;;self\n2;1;river;-1;-1;-1;-1;-1;;existing\n-1;-1;;-1;-1;-1;-1;-1;-1\n1;999;sea;-1;-1;-1;-1;-1;;ignored\n";
        let adjacency = RailwayAdjacency::parse(&definitions, &bitmap, Some(authored)).unwrap();
        assert_eq!(adjacency.neighbors(1).unwrap(), [2, 3, 4, 5]);
        assert_eq!(adjacency.neighbors(2).unwrap(), [1, 4, 3, 2]);
        assert_eq!(adjacency.neighbors(3).unwrap(), [1, 2, 4]);
        assert_eq!(adjacency.neighbors(4).unwrap(), [1, 2, 3]);
        assert_eq!(adjacency.neighbors(5).unwrap(), [1]);
        assert!(adjacency.neighbors(0).is_none());
        assert!(adjacency.neighbors(6).is_none());
    }

    #[test]
    fn railway_adjacency_refuses_content_whose_native_identity_or_storage_differs() {
        let (definitions, bitmap) = adjacency_content();
        assert!(
            RailwayAdjacency::parse(&definitions, &bitmap, None)
                .unwrap()
                .neighbors(5)
                .unwrap()
                .is_empty()
        );
        let reordered =
            b"0;0;0;0;land\n2;2;0;0;sea\n1;1;0;0;land\n3;3;0;0;lake\n4;4;0;0;land\n5;5;0;0;land\n";
        assert!(RailwayAdjacency::parse(reordered, &bitmap, None).is_err());
        for (offset, value) in [(10, 70_i32), (14, 56), (22, -2)] {
            let mut altered = bitmap.clone();
            altered[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(RailwayAdjacency::parse(&definitions, &altered, None).is_err());
        }
        let mut sentinel = bitmap.clone();
        sentinel[54..57].fill(0);
        assert!(RailwayAdjacency::parse(&definitions, &sentinel, None).is_err());
        assert!(
            RailwayAdjacency::parse(
                &definitions,
                &bitmap,
                Some(b"header\n1;6;sea;-1;-1;-1;-1;-1;;bad\n")
            )
            .is_err()
        );
    }

    #[test]
    fn stationed_units_and_area_representatives_keep_save_order() {
        // EED430 writes the representatives as one unkeyed array, the form the
        // 2026-09-11 live saves use; an empty array is a theatre without areas.
        let pack = parse_text(
            br#"countries={GER={theatres={
                theatre={id={id=5 type=67} volunteers_theatre=SOV
                    unit={id=9 type=4919} unit={id=9 type=4919}
                    area={ 21 22 }}
                theatre={id={id=6 type=67} area={ }}}}}"#,
        )
        .unwrap();
        let land = &pack.theatre_countries.unwrap()[b"GER".as_slice()].land;
        assert_eq!(land[0].units, [(9_u64 << 32) | 4919; 2]);
        assert_eq!(land[0].area_representatives, [21, 22]);
        assert_eq!(
            land[0].volunteers_country.as_deref(),
            Some(b"SOV".as_slice())
        );
        // A theatre outside the volunteer scope still parses with empty lists.
        assert!(land[1].units.is_empty() && land[1].area_representatives.is_empty());
    }

    #[test]
    fn theatre_area_arrays_append_in_order_and_refuse_keyed_or_scalar_forms() {
        let pack = parse_text(
            br#"countries={GER={theatres={theatre={id={id=5 type=67}
                area={ 7 } area={ 8 9 }}}}}"#,
        )
        .unwrap();
        let land = &pack.theatre_countries.unwrap()[b"GER".as_slice()].land;
        assert_eq!(land[0].area_representatives, [7, 8, 9]);
        let keyed =
            parse_text(br#"countries={GER={theatres={theatre={id={id=5 type=67} area={ x=7 }}}}}"#)
                .unwrap_err()
                .to_string();
        assert!(
            keyed.contains("unkeyed representative-province list"),
            "{keyed}"
        );
        let scalar =
            parse_text(br#"countries={GER={theatres={theatre={id={id=5 type=67} area=7}}}}"#)
                .unwrap_err()
                .to_string();
        assert!(
            scalar.contains("expected an ordinary save block"),
            "{scalar}"
        );
    }

    #[test]
    fn rail_info_and_theatre_trees_preserve_scopes_order_and_signed_fields() {
        let pack = parse_text(
            br#"rail_way={rail_way={1={rail_way={0 -1 2147483647} cooldown=1} 2={}}}
            countries={GER={
                theatres={theatre={id={id=4294967295 type=67}
                    theater_group={id={id=0 type=68} priority=-2147483648
                        orders_group={id=4294967295 type=53} orders_group={id=0 type=53}}}}
                air_theatre={id={id=4294967295 type=77} country=GER
                    air_group={id={id=0 type=78} air_theatre={id=4294967295 type=77}
                        name="" air_wing={{id=4294967295 type=69} {id=0 type=69}}}}
            }}"#,
        )
        .unwrap();
        let rails = pack.railways.unwrap();
        assert_eq!(rails[&1].levels, [0, -1, i32::MAX]);
        assert_eq!(rails[&1].cooldown, Some(1));
        assert!(rails[&2].levels.is_empty());
        assert_eq!(rails[&2].cooldown, None);
        let countries = pack.theatre_countries.unwrap();
        let country = &countries[b"GER".as_slice()];
        assert_eq!(country.land[0].volunteers_country, None);
        assert_eq!(country.land[0].groups[0].priority, i32::MIN);
        assert_eq!(country.land[0].groups[0].name, None);
        assert_eq!(
            country.land[0].groups[0].members,
            [(u64::from(u32::MAX) << 32) | 53, 53]
        );
        assert_eq!(country.air[0].name, None);
        assert_eq!(country.air[0].groups[0].name, b"");
        assert_eq!(
            country.air[0].groups[0].wings,
            [(u64::from(u32::MAX) << 32) | 69, 69]
        );
        assert!(parse_text(b"rail_way={rail_way={1={cooldown=0}}}").is_err());
        assert!(parse_text(b"rail_way={rail_way={1={} 1={}}}").is_err());
    }

    #[test]
    fn supremacy_pairs_keep_fixed5_extrema_and_distinguish_missing_scopes() {
        let pack = parse_text(br#"region={
            1={dominance={countries={GER} values={
                { GER {current=90071992547.40993 base_target=-92233720368547.75808 target=92233720368547.75807 individual_ratio=-0.00001 previous=0 decline_from=1.00001} }
                { GER {current=0 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
            }}}
            4={dominance={countries={}}}
            8={}
        }"#).unwrap();
        let regions = pack.supremacy_regions.unwrap();
        let rows = regions[&1].values.as_ref().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].current.raw, 9_007_199_254_740_993);
        assert_eq!(rows[0].base_target.raw, i64::MIN);
        assert_eq!(rows[0].target.raw, i64::MAX);
        assert_eq!(rows[0].individual_ratio.raw, -1);
        assert_eq!(regions[&4].values, Some(Vec::new()));
        assert_eq!(regions[&8].values, None);
        assert_eq!(
            parse_text(b"region={}").unwrap().supremacy_regions,
            Some(BTreeMap::new())
        );
        assert_eq!(parse_text(b"").unwrap().supremacy_regions, None);
        assert!(parse_text(b"region={1={dominance={values={{GER {current=0}}}}}}").is_err());
        assert!(parse_text(b"region={1={} 1={}}").is_err());
    }

    #[test]
    fn convoy_history_preserves_signed_fields_country_aliases_and_order() {
        let pack = parse_text(
            br#"sunk_convoys_history={
            sunk_convoy={month=-2147483648 convoys=-1 killer_country=--- owner=GER}
            sunk_convoy={month=2147483647 convoys=2147483647 killer_country=SPR owner=D01}
        }"#,
        )
        .unwrap();
        assert_eq!(pack.sunk_convoys[0].month, i32::MIN);
        assert_eq!(pack.sunk_convoys[0].convoys, -1);
        assert_eq!(pack.sunk_convoys[1].owner_country, b"D01");
        assert!(
            parse_text(b"sunk_convoys_history={sunk_convoy={month=1 convoys=0 owner=GER}}")
                .is_err()
        );
    }

    #[test]
    fn anonymous_history_preserves_duplicates_optional_references_and_null_battle() {
        let entry = br#"sunk_ship={name="" country=GER killer_country=--- level=-1
            definition=destroyer killer_definition=none date=1939.1.1.1
            equipment_variant={id=4294967295 type=70}
            air_wing={id=0 type=54} battle={id=0 type=0} convoy=no}"#;
        let mut text = b"history={".to_vec();
        text.extend_from_slice(entry);
        text.extend_from_slice(entry);
        text.extend_from_slice(b"}");
        let pack = parse_text(&text).unwrap();
        assert_eq!(pack.sunk_ships.len(), 2);
        assert_eq!(pack.sunk_ships[0], pack.sunk_ships[1]);
        let row = &pack.sunk_ships[0];
        assert_eq!(row.ship_name, Some(Vec::new()));
        assert_eq!(row.attribution_name, None);
        assert_eq!(
            row.victim_equipment_variant,
            Some((u64::from(u32::MAX) << 32) | 70)
        );
        assert_eq!(row.damage_source_reference, Some(54));
        assert_eq!(row.province, None);
        assert_eq!(row.battle, 0);
        assert!(!row.assisted_kill);
        assert_eq!(parse_text(b"").unwrap(), Pack::default());
    }

    #[test]
    fn result_handles_and_country_membership_keep_native_words_and_list_order() {
        let text = br#"naval_combat_result={id={id=4294967295 type=61}
            location=-1 date=1939.1.1.1 to_discard_date=1939.1.2.1
            importance=-2147483648 port_strike=yes naval_strike=no
            attacker={ship={unique_id={id=0 type=69} cached_info={sunk_by="cause"}} countries={GER GER ---}}
            defender={} shown_to_countries={SOV GER}}
        "#;
        let pack = parse_text(text).unwrap();
        let row = pack.results.values().next().unwrap();
        assert_eq!(row.id, (u64::from(u32::MAX) << 32) | 61);
        assert_eq!(row.province, u32::MAX);
        assert_eq!(row.category, i32::MIN as u32);
        assert_eq!(row.sides[0].members[0].id, 69);
        assert_eq!(
            row.sides[0].members[0].sunk_by.as_deref(),
            Some(b"cause".as_slice())
        );
        assert_eq!(
            row.sides[0].countries,
            [b"GER".to_vec(), b"GER".to_vec(), b"---".to_vec()]
        );
        assert!(row.sides[1].members.is_empty());
        let duplicated = [text.as_slice(), text.as_slice()].concat();
        assert!(parse_text(&duplicated).is_err());
    }
}
