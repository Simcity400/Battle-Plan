use std::{collections::BTreeMap, io};

use crate::parser::Item;

use super::{
    Decimal, block, boolean, bytes, combat_details, decimal, field, integer, integer_list, invalid,
    null_permitted_handle, object_id, production, required_block, required_bytes, scalar,
    typed_handle,
};

/// One `country_intel` triple of a saved CUnit, in serialized order.
///
/// CUnit writer BF7010 emits three signed integers per stored viewer entry: the
/// viewer key word, the low 32 bits of the entry's decay accumulator, and the
/// sign-extended level byte. Every entry is written, including nonpositive
/// levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntelEntry {
    pub viewer: i32,
    pub timer_low: i32,
    pub level: i32,
}

/// One serialized `railway_gun` in a country's `units` roster.
///
/// A simulation object, not a map counter or a country-view result. The CUnit
/// writer omits some fields conditionally; options keep that distinction for
/// the recording oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayGun {
    pub id: u64,
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    pub province: Option<u32>,
    pub path: Vec<u32>,
    /// CUnit::Serialize@140BF7010: target (0x2EDA) and alternate route (0x362C).
    pub support_attack: Option<u32>,
    pub full_path: Vec<u32>,
    pub strategic_redeployment: bool,
    pub strength: Decimal,
    /// The gun's own combat reference array (CRailwayGun+0x420, token 0x2916,
    /// `CRailwayGun::Serialize@140E78CF0`) in native order; `CUnit::Serialize`
    /// never writes the CUnit combat vector.
    pub combats: Vec<u64>,
    /// Serialized per-viewer simulation intel triples, in native order.
    pub country_intel: Vec<IntelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Division {
    /// Native reference qword: type in the low u32, id in the high u32.
    pub id: u64,
    /// CCountry's enclosing units roster, independently of logical_country.
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    /// CUnit writer BF7010 omits location only when its pointer is null.
    pub province: Option<u32>,
    /// CArmy writer C80E00 emits the standard template, not fake intel, iff nonnull.
    pub template: Option<u64>,
    /// CArmy C80E00 writes primary+3C8 only when the alternate is nonnull.
    pub alternate_template: Option<u64>,
    /// C80E00 writes primary+630 only when nonzero; text retains truthiness.
    pub separate_counter: bool,
    /// Saved strength is absolute current hit points in native /100000 units.
    pub hit_points: Decimal,
    pub organisation: Decimal,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
    pub presentation: DivisionPresentation,
}

/// Fields independently serialized by CUnit BF7010 and CArmy C80E00.
/// Positive-only fields retain omission, rather than inventing a zero value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionPresentation {
    pub path: Vec<u32>,
    /// CUnit::Serialize@140BF7010: target (0x2EDA) and alternate route (0x362C).
    pub support_attack: Option<u32>,
    pub full_path: Vec<u32>,
    pub previous: Option<u32>,
    pub movement_progress: Option<Decimal>,
    pub dig_in: Option<Decimal>,
    pub dig_in_cap: Option<Decimal>,
    pub strategic_redeployment: bool,
    /// CArmy C80E00 writes its +0x340 CNameGroupMember under token 0x3904
    /// (`division_name`) through serializer 1409BCC70.
    pub division_name: Option<DivisionName>,
    pub acclimatization: Option<Acclimatization>,
}

/// One saved `division_name` block. Serializer 1409BCC70 writes `type`
/// (0xE1) unconditionally and every other key conditionally, so absence is
/// kept distinct from a written value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionName {
    pub kind: i32,
    /// 0x38E3, written only when the +0x80 order word is nonzero.
    pub order: Option<i32>,
    /// 0x38E2 `is_name_ordered`, written as `no` only when +0xA8 is clear.
    pub ordered: Option<bool>,
    /// 0x38E1, the +0x88 string when its length is nonzero (outside checksum mode).
    pub custom: Option<Vec<u8>>,
    /// 0x3936, written only when the +0xA9 flag is set.
    pub programmatic: Option<bool>,
}

/// `NAcclimatization::CData::vf2` (140609C70): each nonzero-weight entry keyed
/// by its category's lexer token in native order, three engine-owned progress
/// fields, then `max_acclimatization` unless it is 100.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Acclimatization {
    /// (category name, weight) for every nonzero weight, in native order.
    pub entries: Vec<(Vec<u8>, Decimal)>,
    /// Omitted when the maximum is exactly 100.
    pub maximum: Option<Decimal>,
}

impl Acclimatization {
    /// The maximum the writer omits.
    pub const OMITTED_MAXIMUM_RAW: i64 = 10_000_000;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionTemplate {
    pub template: u64,
    pub country: Vec<u8>,
    pub name: Vec<u8>,
    pub custom_icon: i32,
    pub custom_icon_nato: bool,
    /// `foreign_template_tag`, verbatim.
    pub foreign_template_tag: Option<Vec<u8>>,
    /// Grid kind (regiments/support/regimental support), x, y -> literal unit name.
    pub composition: BTreeMap<(u8, u32, u32), Vec<u8>>,
    /// Token 0x38DF `division_names_group`: 140B977F0 writes the selected
    /// CNameGroup's +8 key only when the composition+0x1B8 group is set.
    pub names_group: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombat {
    pub id: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    /// Direct side unit references, not the separate front subset.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub details: combat_details::LandDetails,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fleet {
    pub id: u64,
    pub owner: Vec<u8>,
    pub home_base: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskForce {
    pub id: u64,
    pub fleet: u64,
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    pub province: Option<u32>,
    pub mission: i32,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
    /// `CUnit::Serialize@140BF7010` (receiver primary `+0x10`) writes the ordinary route
    /// (primary `+0x200`) as token `0x174` `path`, omitted when empty.
    pub path: Vec<u32>,
    /// The movement-target province id (primary `+0x1E8`, the province's `+0xA4`): token
    /// `0x2EDA`, written only for a non-null target, so `None` is the null pointer. The
    /// executable's static token registrations (the `14014B840` calls) spell `0x2EDA`
    /// `support_attack` and `0x362C` `full_path`; a saved moving division writes
    /// `support_attack=<province>` between `previous` (`0x29A3`) and `last_combat_date`, as the
    /// writer orders them.
    pub target: Option<u32>,
    /// The alternate route (primary `+0x220`, count `+0x22C`): token `0x362C` `full_path`,
    /// written only when its count is nonzero. The writer saves no combat list for a task force: the
    /// primary `+0x1A8` combat vector is not serialized ([`NavalCombat::attackers`] is the
    /// saved membership).
    pub alternate_path: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ship {
    /// Position in the task force serializer's ordered ship list.
    pub roster_ordinal: u32,
    pub id: u64,
    pub task_force: u64,
    pub strength: Decimal,
    pub organisation: Decimal,
    pub experience: Decimal,
    pub sunk_convoys: i32,
    /// C2EF80 writes the embedded primary+40 pool under equipment.
    /// Absence is retained so the oracle can reject a missing required wrapper.
    pub equipment: Option<production::Pool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrder {
    pub owner_group: u64,
    pub instance: u32,
    pub country: Vec<u8>,
    pub order_type: i32,
    /// Saved can_execute is a signed byte spelling; preserve all native bits.
    pub active: u8,
    pub paradrop_target: u32,
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirWing {
    pub id: u64,
    pub owner: Vec<u8>,
    pub country_override: Vec<u8>,
    pub air_base: u64,
    pub planes: i32,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    /// The mission's `active` flag, when written.
    pub mission_active: Option<bool>,
    /// Raw fixed5 `CAirMission+0xC0`, written with a mission region.
    pub region_change_penalty: Option<i64>,
    /// The wing's position among its country's `air_wing_pool` blocks'
    /// `air_wings`, in file order: `CStrategicAir` slot 2 (`140C57230`)
    /// writes the pools in `+0x58` order and `CAirWingPool::Serialize`
    /// (`140F53E70`) each pool's wings in `+0x28` order.
    pub pool_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombat {
    pub id: u64,
    pub province: u32,
    /// Each side's participant array (`CNavalCombatant+0x20`), which
    /// `CNavalCombatant::Serialize@14160FF40` writes in array order as repeated token `0x28A3`
    /// `unit` references. `CCombatant::AddUnit@1413CC2B0` appends a unit here and adds its
    /// logical owner (`CUnit+0x1E0`) to the side's country set through `1413CC0D0`.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub attacker_positioning: Decimal,
    pub defender_positioning: Decimal,
    pub attacker_anti_air: Decimal,
    pub defender_anti_air: Decimal,
    pub details: combat_details::NavalDetails,
    /// Every `naval_transport` reference in writer order: `CNavalCombat::Serialize@1415B36C0`
    /// writes each slot of the +0xF0 transfer vector, zero and stale ones included.
    pub transfers: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombatParticipant {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: Option<u64>,
    pub convoy: Option<u64>,
    pub owner: Vec<u8>,
    pub state: u32,
    pub details: combat_details::NavalMemberDetails,
}

pub(super) fn parse_division_templates(
    items: &[Item],
) -> io::Result<BTreeMap<u64, DivisionTemplate>> {
    let mut result = BTreeMap::new();
    let Some(value) = field(items, b"division_templates")? else {
        return Ok(result);
    };
    for item in block(value)? {
        if item.key.as_deref() != Some(b"division_template") {
            return Err(invalid("unexpected division template registry entry"));
        }
        let fields = block(&item.value)?;
        let template = typed_handle(
            field(fields, b"id")?.ok_or_else(|| invalid("division template lacks id"))?,
        )?;
        let mut composition = BTreeMap::new();
        for (kind, key) in [b"regiments".as_slice(), b"support", b"regimental_support"]
            .into_iter()
            .enumerate()
        {
            if let Some(value) = field(fields, key)? {
                for unit in block(value)? {
                    let name = unit
                        .key
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| invalid("template subunit name absent"))?;
                    let position = block(&unit.value)?;
                    let x: u32 = integer(&required_bytes(position, b"x")?)?;
                    let y: u32 = integer(&required_bytes(position, b"y")?)?;
                    if composition
                        .insert((kind as u8, x, y), name.to_vec())
                        .is_some()
                    {
                        return Err(invalid("duplicate division template grid cell"));
                    }
                }
            }
        }
        let row = DivisionTemplate {
            template,
            country: required_bytes(fields, b"country")?,
            name: bytes(fields, b"name")?.unwrap_or_default(),
            custom_icon: integer(&required_bytes(fields, b"template_counter")?)?,
            custom_icon_nato: boolean(
                field(fields, b"ingame_set_template_counter")?
                    .ok_or_else(|| invalid("template lacks counter override flag"))?,
            )?,
            foreign_template_tag: bytes(fields, b"foreign_template_tag")?,
            composition,
            names_group: bytes(fields, b"division_names_group")?,
        };
        if result.insert(template, row).is_some() {
            return Err(invalid("duplicate division template handle"));
        }
    }
    Ok(result)
}

pub(super) fn parse_railway_gun(roster_owner: &[u8], items: &[Item]) -> io::Result<RailwayGun> {
    // CUnit and its supply-consumer base can both write the same reference.
    // Equal repeated ids are native output; conflicting ids are malformed.
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|id| id != handle) {
            return Err(invalid("railway gun has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    let id = id.ok_or_else(|| invalid("railway gun lacks its typed id"))?;
    let province = bytes(items, b"location")?
        .map(|value| integer::<u32>(&value))
        .transpose()?;
    if province == Some(0) {
        return Err(invalid(
            "railway gun explicitly saves location zero; native null location must be omitted",
        ));
    }
    let combats = match field(items, b"combat")? {
        Some(value) => block(value)?
            .iter()
            .map(|item| {
                if item.key.is_some() {
                    return Err(invalid("railway gun combat list has keyed entry"));
                }
                typed_handle(&item.value)
            })
            .collect::<io::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(RailwayGun {
        id,
        roster_owner: roster_owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province,
        path: integer_list(items, b"path")?,
        support_attack: bytes(items, b"support_attack")?
            .map(|value| integer(&value))
            .transpose()?,
        full_path: integer_list(items, b"full_path")?,
        strategic_redeployment: field(items, b"strategic_redeployment")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        strength: Decimal::parse(&required_bytes(items, b"strength")?)?,
        combats,
        country_intel: country_intel(items)?,
    })
}

pub(super) fn parse_division(roster_owner: &[u8], items: &[Item]) -> io::Result<Division> {
    // CUnit and CSupplyConsumer each serialize the same CReferenceObject.
    // Repeated equal id blocks are native output; conflicting ones are not.
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|id| id != handle) {
            return Err(invalid("division has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    let id = id.ok_or_else(|| invalid("division lacks its typed id"))?;
    let province = bytes(items, b"location")?
        .map(|value| integer::<u32>(&value))
        .transpose()?;
    if province == Some(0) {
        return Err(invalid(
            "division explicitly saves location zero; native null location must be omitted",
        ));
    }
    Ok(Division {
        id,
        roster_owner: roster_owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province,
        template: field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
        alternate_template: field(items, b"fake_intel_division_template_id")?
            .map(typed_handle)
            .transpose()?,
        separate_counter: field(items, b"leader")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        hit_points: Decimal::parse(&required_bytes(items, b"strength")?)?,
        organisation: Decimal::parse(&required_bytes(items, b"organisation")?)?,
        country_intel: country_intel(items)?,
        presentation: DivisionPresentation {
            path: integer_list(items, b"path")?,
            support_attack: bytes(items, b"support_attack")?
                .map(|value| integer(&value))
                .transpose()?,
            full_path: integer_list(items, b"full_path")?,
            previous: bytes(items, b"previous")?
                .map(|b| integer(&b))
                .transpose()?,
            movement_progress: decimal(items, b"movement_progress")?,
            dig_in: decimal(items, b"dig_in")?,
            dig_in_cap: decimal(items, b"dig_in_cap")?,
            strategic_redeployment: field(items, b"strategic_redeployment")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
            division_name: field(items, b"division_name")?
                .map(|value| division_name(block(value)?))
                .transpose()?,
            acclimatization: acclimatization(items)?,
        },
    })
}

fn division_name(items: &[Item]) -> io::Result<DivisionName> {
    let flag = |key: &[u8]| field(items, key)?.map(boolean).transpose();
    Ok(DivisionName {
        kind: integer(&required_bytes(items, b"type")?)?,
        order: bytes(items, b"name_order")?
            .map(|value| integer(&value))
            .transpose()?,
        ordered: flag(b"is_name_ordered")?,
        custom: bytes(items, b"override")?,
        programmatic: flag(b"override_set_programmatically")?,
    })
}

fn acclimatization(items: &[Item]) -> io::Result<Option<Acclimatization>> {
    let Some(value) = field(items, b"acclimatization")? else {
        return Ok(None);
    };
    let mut result = Acclimatization::default();
    for item in block(value)? {
        let key = item
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("acclimatization entry lacks a key"))?;
        match key {
            // The writer's own progress fields (tokens 386E, 386F, 3870).
            b"actively_gaining" | b"actively_gaining_speed" | b"other_loss" => {}
            b"max_acclimatization" => {
                if result
                    .maximum
                    .replace(Decimal::parse(scalar(&item.value)?)?)
                    .is_some()
                {
                    return Err(invalid("duplicate max_acclimatization"));
                }
            }
            category => result
                .entries
                .push((category.to_vec(), Decimal::parse(scalar(&item.value)?)?)),
        }
    }
    Ok(Some(result))
}

/// The writer emits one anonymous flat list of three signed integers per entry.
fn country_intel(items: &[Item]) -> io::Result<Vec<IntelEntry>> {
    let Some(value) = field(items, b"country_intel")? else {
        return Ok(Vec::new());
    };
    let entries = block(value)?;
    if entries.is_empty() || entries.len() % 3 != 0 {
        return Err(invalid(
            "saved country_intel is empty or not a whole number of viewer triples",
        ));
    }
    let mut words = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.key.is_some() {
            return Err(invalid("saved country_intel must be an anonymous list"));
        }
        words.push(integer::<i32>(scalar(&entry.value)?)?);
    }
    Ok(words
        .chunks_exact(3)
        .map(|triple| IntelEntry {
            viewer: triple[0],
            timer_low: triple[1],
            level: triple[2],
        })
        .collect())
}

pub(super) fn parse_land_combat(items: &[Item]) -> io::Result<LandCombat> {
    fn participants(items: &[Item], side: &[u8]) -> io::Result<Vec<u64>> {
        required_block(items, side)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"unit"))
            .map(|item| typed_handle(&item.value))
            .collect()
    }
    Ok(LandCombat {
        id: typed_handle(
            field(items, b"id")?.ok_or_else(|| invalid("land combat lacks typed id"))?,
        )?,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        day: integer(&required_bytes(items, b"day")?)?,
        duration: integer(&required_bytes(items, b"duration")?)?,
        attackers: participants(items, b"attacker")?,
        defenders: participants(items, b"defender")?,
        details: combat_details::land(items)?,
    })
}

pub(super) fn parse_fleet(owner: &[u8], items: &[Item]) -> io::Result<Fleet> {
    Ok(Fleet {
        id: object_id(items)?,
        owner: owner.to_vec(),
        home_base: field(items, b"home_base")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
    })
}

pub(super) fn parse_task_force(owner: &[u8], fleet: u64, items: &[Item]) -> io::Result<TaskForce> {
    Ok(TaskForce {
        id: object_id(items)?,
        fleet,
        roster_owner: owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province: field(items, b"location")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        mission: integer(&required_bytes(
            required_block(items, b"mission")?,
            b"mission",
        )?)?,
        country_intel: country_intel(items)?,
        path: integer_list(items, b"path")?,
        target: field(items, b"support_attack")?
            .map(|value| integer::<i32>(scalar(value)?).map(|id| id as u32))
            .transpose()?,
        alternate_path: integer_list(items, b"full_path")?,
    })
}

pub(super) fn parse_ship(task_force: u64, items: &[Item]) -> io::Result<Ship> {
    Ok(Ship {
        roster_ordinal: 0,
        id: object_id(items)?,
        task_force,
        strength: decimal(items, b"strength")?.ok_or_else(|| invalid("ship lacks strength"))?,
        organisation: decimal(items, b"organisation")?
            .ok_or_else(|| invalid("ship lacks organisation"))?,
        experience: decimal(items, b"experience")?.unwrap_or(Decimal::parse(b"0")?),
        sunk_convoys: field(items, b"sunk_convoys")?
            .map(|value| integer(scalar(value)?))
            .transpose()?
            .unwrap_or(0),
        equipment: field(items, b"equipment")?
            .map(block)
            .transpose()?
            .map(production::parse_pool)
            .transpose()?,
    })
}

fn parse_army_order(country: &[u8], owner_group: u64, items: &[Item]) -> io::Result<ArmyOrder> {
    Ok(ArmyOrder {
        owner_group,
        country: country.to_vec(),
        instance: integer(&required_bytes(items, b"instance_id")?)?,
        order_type: integer(&required_bytes(items, b"type")?)?,
        active: field(items, b"can_execute")?
            .map(|value| integer::<i8>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u8,
        paradrop_target: field(items, b"invasion_source")?
            .map(|value| integer::<i32>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u32,
        path: integer_list(items, b"path")?,
        target_states: integer_list(items, b"states")?,
    })
}

pub(super) fn parse_orders(
    country: &[u8],
    items: &[Item],
    orders: &mut BTreeMap<(u64, u32), ArmyOrder>,
) -> io::Result<()> {
    let Some(theatres) = field(items, b"theatres")? else {
        return Ok(());
    };
    for theatre in block(theatres)?
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"theatre"))
    {
        for group in block(&theatre.value)?.iter().filter(|item| {
            matches!(
                item.key.as_deref(),
                Some(b"orders_group" | b"field_marshal_group")
            )
        }) {
            let group = block(&group.value)?;
            let id = object_id(group)?;
            for item in group.iter().filter(|item| {
                matches!(
                    item.key.as_deref(),
                    Some(b"order_instance" | b"fallback" | b"virtual_fallback")
                )
            }) {
                let order = parse_army_order(country, id, block(&item.value)?)?;
                if orders.insert((id, order.instance), order).is_some() {
                    return Err(invalid("duplicate scoped army order instance in save"));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn parse_air_wings(items: &[Item]) -> io::Result<BTreeMap<u64, AirWing>> {
    let mut wings = BTreeMap::new();
    let Some(air) = field(items, b"strategic_air")? else {
        return Ok(wings);
    };
    for country in block(air)? {
        let owner = country
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("strategic air country lacks tag"))?;
        let mut pool_ordinal = 0_u32;
        for pool in block(&country.value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"air_wing_pool"))
        {
            let pool = block(&pool.value)?;
            for wing in pool
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"air_wings"))
            {
                let fields = block(&wing.value)?;
                let mission = required_block(fields, b"mission")?;
                let wing = AirWing {
                    id: object_id(fields)?,
                    owner: owner.to_vec(),
                    country_override: required_bytes(fields, b"tag")?,
                    air_base: typed_handle(
                        field(pool, b"air_base")?
                            .ok_or_else(|| invalid("populated air pool lacks air_base"))?,
                    )?,
                    planes: integer(&required_bytes(fields, b"count")?)?,
                    mission_mask: integer::<i32>(&required_bytes(mission, b"type")?)? as u32,
                    mission_region: field(mission, b"strategic_region")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    mission_active: field(mission, b"active")?.map(boolean).transpose()?,
                    region_change_penalty: decimal(mission, b"region_change_penalty")?
                        .map(|penalty| penalty.raw),
                    pool_ordinal,
                };
                pool_ordinal = pool_ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("air wing pool ordinal exceeds u32"))?;
                if wings.insert(wing.id, wing).is_some() {
                    return Err(invalid("duplicate full air wing handle in save"));
                }
            }
        }
    }
    Ok(wings)
}

pub(super) fn parse_naval_combat(
    items: &[Item],
    participants: &mut BTreeMap<(u64, u8, u32), NavalCombatParticipant>,
) -> io::Result<NavalCombat> {
    let id = object_id(items)?;
    let attacker = required_block(items, b"attacker")?;
    let defender = required_block(items, b"defender")?;
    for (side, fields) in [(0, attacker), (1, defender)] {
        for group in fields
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"group"))
        {
            for item in block(&group.value)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"member"))
            {
                let fields = block(&item.value)?;
                let row = combat_details::naval_participant(fields, id, side)?;
                if participants
                    .insert((id, side, row.battle_slot), row)
                    .is_some()
                {
                    return Err(invalid("duplicate combat side battle slot in save"));
                }
            }
        }
    }
    let units = |side: &[Item]| {
        side.iter()
            .filter(|item| item.key.as_deref() == Some(b"unit"))
            .map(|item| typed_handle(&item.value))
            .collect::<io::Result<Vec<_>>>()
    };
    Ok(NavalCombat {
        id,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        attackers: units(attacker)?,
        defenders: units(defender)?,
        attacker_positioning: Decimal::parse(&required_bytes(attacker, b"positioning")?)?,
        defender_positioning: Decimal::parse(&required_bytes(defender, b"positioning")?)?,
        attacker_anti_air: Decimal::parse(&required_bytes(attacker, b"anti_air")?)?,
        defender_anti_air: Decimal::parse(&required_bytes(defender, b"anti_air")?)?,
        details: combat_details::naval(items)?,
        transfers: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"naval_transport"))
            .map(|item| null_permitted_handle(&item.value))
            .collect::<io::Result<_>>()?,
    })
}
