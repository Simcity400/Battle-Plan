//! The division stat calculator the game runs for a template's grouped composition and an
//! equipment pool: once for a deployed division (its held equipment and current manpower,
//! before the deployed modifier pass) and once for the division designer's preview (one
//! chosen design per required equipment type, the template's own manpower and full fuel,
//! with nothing after it).
//!
//! Arithmetic is the game's fixed5: a product truncates toward zero after a wrapping signed
//! multiplication ([`mul`]), a quotient is the guarded fixed5 division ([`div`]), and every
//! stage the game takes is taken, an identity factor included. Each group of identical
//! definitions contributes in composition order: its equipment terms (each held design's
//! effective stats times one plus the definition's own coefficient, scaled by the share of the
//! requirement it fills), then its own base words for the stats that have one. Support
//! companies' category grants adjust both. Totals are then finished per stat: summed, averaged
//! over every real cell (or, for hardness, every cell whose type is not support), blended
//! between the largest group and the average (armour and piercing), or scaled by an
//! accumulated factor (suppression, supply, fuel). Speed is selected group by group: each group
//! whose definition affects speed offers its equipment speed per sub-unit, and the slowest offer
//! wins, floored at the slowest speed after every group.
//!
//! Only the stats whose rule is established are computed; every other slot stays `None`.

use installed_content_core::equipment::STAT_COUNT;

/// Fixed5 one.
pub const ONE: i64 = 100_000;

/// The guarded fixed5 quotient's result when it is undefined (a zero denominator).
pub const UNDEFINED: i64 = 4_294_967_295;

/// A fixed5 product: the wrapping signed product, truncated toward zero.
pub fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

/// The guarded fixed5 quotient, truncated toward zero; undefined for a zero denominator.
pub fn div(a: i64, b: i64) -> i64 {
    if b == 0 {
        return UNDEFINED;
    }
    (i128::from(a) * i128::from(ONE) / i128::from(b)) as i64
}

/// A count of cells as fixed5.
pub fn count(n: i32) -> i64 {
    i64::from(n).wrapping_mul(ONE)
}

/// The stat slots the calculator treats specially, by their executable metadata indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slots {
    pub max_strength: usize,
    pub max_organisation: usize,
    pub default_morale: usize,
    pub recon: usize,
    pub entrenchment: usize,
    pub initiative: usize,
    pub casualty_trickleback: usize,
    pub supply_consumption: usize,
    pub supply_consumption_factor: usize,
    pub suppression: usize,
    pub suppression_factor: usize,
    pub experience_loss_factor: usize,
    pub equipment_capture_factor: usize,
    pub reliability: usize,
    pub reliability_factor: usize,
    pub weight: usize,
    pub fuel_capacity: usize,
    pub fuel_consumption: usize,
    pub fuel_consumption_factor: usize,
    pub soft_attack: usize,
    pub hard_attack: usize,
    pub air_attack: usize,
    pub defense: usize,
    pub breakthrough: usize,
    pub armor_value: usize,
    pub ap_attack: usize,
    pub hardness: usize,
    pub maximum_speed: usize,
    pub build_cost_ic: usize,
}

impl Slots {
    /// Every slot by its keyword in the executable's stat metadata, or the first keyword the
    /// metadata lacks.
    pub fn new(index: impl Fn(&str) -> Option<usize>) -> Result<Self, String> {
        let slot = |name: &str| {
            index(name).ok_or_else(|| format!("the executable's stat metadata has no {name}"))
        };
        Ok(Self {
            max_strength: slot("max_strength")?,
            max_organisation: slot("max_organisation")?,
            default_morale: slot("default_morale")?,
            recon: slot("recon")?,
            entrenchment: slot("entrenchment")?,
            initiative: slot("initiative")?,
            casualty_trickleback: slot("casualty_trickleback")?,
            supply_consumption: slot("supply_consumption")?,
            supply_consumption_factor: slot("supply_consumption_factor")?,
            suppression: slot("suppression")?,
            suppression_factor: slot("suppression_factor")?,
            experience_loss_factor: slot("experience_loss_factor")?,
            equipment_capture_factor: slot("equipment_capture_factor")?,
            reliability: slot("reliability")?,
            reliability_factor: slot("reliability_factor")?,
            weight: slot("weight")?,
            fuel_capacity: slot("fuel_capacity")?,
            fuel_consumption: slot("fuel_consumption")?,
            fuel_consumption_factor: slot("fuel_consumption_factor")?,
            soft_attack: slot("soft_attack")?,
            hard_attack: slot("hard_attack")?,
            air_attack: slot("air_attack")?,
            defense: slot("defense")?,
            breakthrough: slot("breakthrough")?,
            armor_value: slot("armor_value")?,
            ap_attack: slot("ap_attack")?,
            hardness: slot("hardness")?,
            maximum_speed: slot("maximum_speed")?,
            build_cost_ic: slot("build_cost_ic")?,
        })
    }

    /// The executable's own metadata.
    pub fn executable() -> Result<Self, String> {
        Self::new(|name| installed_content_core::equipment::stat_index(name.as_bytes()))
    }

    /// Stats with a base word scaled by the group's fill.
    fn scaled_bases(&self) -> [usize; 14] {
        [
            self.max_strength,
            self.recon,
            self.entrenchment,
            self.initiative,
            self.casualty_trickleback,
            self.supply_consumption,
            self.supply_consumption_factor,
            self.suppression,
            self.suppression_factor,
            self.experience_loss_factor,
            self.equipment_capture_factor,
            self.reliability,
            self.reliability_factor,
            self.weight,
        ]
    }

    /// Stats summed over the composition with nothing further.
    fn sums(&self) -> [usize; 15] {
        [
            self.max_strength,
            self.recon,
            self.entrenchment,
            self.initiative,
            self.casualty_trickleback,
            self.experience_loss_factor,
            self.equipment_capture_factor,
            self.reliability_factor,
            self.weight,
            self.soft_attack,
            self.hard_attack,
            self.air_attack,
            self.defense,
            self.breakthrough,
            self.fuel_capacity,
        ]
    }

    /// Stats averaged over every real cell.
    fn means(&self) -> [usize; 5] {
        [
            self.default_morale,
            self.max_organisation,
            self.armor_value,
            self.ap_attack,
            self.reliability,
        ]
    }
}

/// A support company's grant to the definitions of one category: added to their base words,
/// or summed into a factor on their equipment terms and base words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub category: String,
    pub add: bool,
    pub stats: Vec<(usize, i64)>,
}

/// One effective sub-unit definition, as the calculator reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    /// Every stat word: a coefficient on equipment terms, and a base for the stats that have
    /// one.
    pub stats: [i64; STAT_COUNT],
    pub combat_width: i64,
    pub manpower: i32,
    pub training_time: i32,
    /// Its type mask holds support: no equipment hardness, and outside the hardness average.
    pub support_type: bool,
    /// Its group is support: its grants reach the composition.
    pub support_group: bool,
    pub categories: Vec<String>,
    pub grants: Vec<Grant>,
    /// Required equipment, by archetype, in the definition's order; fixed5 amounts.
    pub requirements: Vec<(usize, i64)>,
    /// Its critical equipment archetypes, in authored order: the group's critical-equipment
    /// fill reads them ([`Group::critical`]).
    pub critical: Vec<usize>,
    /// The archetype a transport restriction admits for organisation, strength and speed.
    pub transport: Option<usize>,
    /// The multiplier on its equipment's fuel consumption.
    pub own_fuel_consumption: i64,
    /// Its equipment speed takes part in the division's speed selection.
    pub affects_speed: bool,
}

/// One design in the pool: its effective stats, archetype and amount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub stats: [i64; STAT_COUNT],
    pub archetype: usize,
    /// Fixed5 amount.
    pub amount: i64,
}

/// A group of identical definitions: its count and its critical-equipment fill (one when the
/// definition names no critical equipment).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    pub definition: &'a Definition,
    pub count: i32,
    pub critical: i64,
}

/// The calculator's operands besides the composition: the manpower fill, the fuel fill and
/// the loaded defines it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operands {
    pub manpower_fill: i64,
    pub fuel_fill: i64,
    pub armor_vs_average: i64,
    pub pen_vs_average: i64,
    pub out_of_fuel_equipment: i64,
    pub fuel_capacity_hours: i64,
    pub army_fuel_cost: i64,
    /// The floor of the division's speed after each group.
    pub slowest_speed: i64,
}

/// The calculator's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub stats: [Option<i64>; STAT_COUNT],
    pub combat_width: i64,
    pub manpower: i32,
    pub training_time: i32,
}

/// The equipment factor of a design: reduced while fuel runs short, for a design that uses
/// fuel.
fn fuel_factor(held: &Held, slots: &Slots, operands: &Operands) -> i64 {
    if held.stats[slots.fuel_consumption] > 0 && operands.fuel_fill < ONE {
        operands.out_of_fuel_equipment
            + mul(ONE - operands.out_of_fuel_equipment, operands.fuel_fill)
    } else {
        ONE
    }
}

/// The calculator over `groups` in composition order and `pool`, the held or chosen designs in
/// their stored order.
pub fn calculate(slots: &Slots, groups: &[Group<'_>], pool: &[Held], operands: &Operands) -> Block {
    let mut stats = [None; STAT_COUNT];
    let manpower = groups.iter().fold(0_i32, |sum, group| {
        sum.wrapping_add(group.definition.manpower.wrapping_mul(group.count))
    });
    let training_time = groups
        .iter()
        .map(|group| group.definition.training_time)
        .fold(0, i32::max);
    if groups.is_empty() {
        for slot in slots.sums().into_iter().chain(slots.means()).chain([
            slots.hardness,
            slots.suppression,
            slots.supply_consumption,
            slots.fuel_consumption,
            slots.maximum_speed,
        ]) {
            stats[slot] = Some(0);
        }
        return Block {
            stats,
            combat_width: 0,
            manpower,
            training_time,
        };
    }
    // The template's requirement of each archetype and the pool's holding of it.
    let mut required = std::collections::BTreeMap::<usize, i64>::new();
    for group in groups {
        for &(archetype, amount) in &group.definition.requirements {
            let total = required.entry(archetype).or_default();
            *total = total.wrapping_add(amount.wrapping_mul(i64::from(group.count)));
        }
    }
    let mut held = std::collections::BTreeMap::<usize, i64>::new();
    for design in pool {
        let total = held.entry(design.archetype).or_default();
        *total = total.wrapping_add(design.amount);
    }
    let all = groups
        .iter()
        .fold(0_i32, |sum, group| sum.wrapping_add(group.count));
    let non_support = groups
        .iter()
        .filter(|group| !group.definition.support_type)
        .fold(0_i32, |sum, group| sum.wrapping_add(group.count));
    let mut total = [0_i64; STAT_COUNT];
    let (mut armor_max, mut pen_max) = (0_i64, 0_i64);
    let mut width = 0_i64;
    let mut raw_fuel = 0_i64;
    let mut speed = 0_i64;
    for group in groups {
        let definition = group.definition;
        let mut equipment = [0_i64; STAT_COUNT];
        let requirements = definition.requirements.len() as i64;
        for &(archetype, _) in &definition.requirements {
            let r = required.get(&archetype).copied().unwrap_or(0);
            let h_total = held.get(&archetype).copied().unwrap_or(0);
            for design in pool.iter().filter(|design| design.archetype == archetype) {
                let q = if h_total > 0 && h_total > r {
                    div(r, h_total)
                } else {
                    ONE
                };
                let share = mul(div(mul(design.amount, q), r), group.critical);
                let share = operands.manpower_fill.min(share);
                let w = mul(count(group.count), share);
                let f = fuel_factor(design, slots, operands);
                let transported = definition
                    .transport
                    .is_none_or(|transport| transport == design.archetype);
                for (slot, &value) in design.stats.iter().enumerate() {
                    if value == 0 {
                        continue;
                    }
                    if !transported
                        && [
                            slots.max_organisation,
                            slots.max_strength,
                            slots.maximum_speed,
                        ]
                        .contains(&slot)
                    {
                        continue;
                    }
                    if slot == slots.hardness && definition.support_type {
                        continue;
                    }
                    let coefficient = mul(ONE + definition.stats[slot], w);
                    let term = if slot == slots.reliability {
                        mul(coefficient / requirements, value)
                    } else if slot == slots.fuel_capacity {
                        mul(coefficient, value)
                    } else if slot == slots.fuel_consumption {
                        mul(mul(coefficient, definition.own_fuel_consumption), value)
                    } else {
                        mul(mul(coefficient, f), value)
                    };
                    equipment[slot] = equipment[slot].wrapping_add(term);
                }
            }
        }
        // The largest group's armour and piercing, before support grants.
        armor_max = armor_max.max(div(equipment[slots.armor_value], count(group.count)));
        pen_max = pen_max.max(div(equipment[slots.ap_attack], count(group.count)));
        // Grants from every distinct support group, whatever its count.
        let mut added = [0_i64; STAT_COUNT];
        let mut factor = [0_i64; STAT_COUNT];
        for source in groups
            .iter()
            .filter(|source| source.definition.support_group)
        {
            for grant in &source.definition.grants {
                // Category names are one token under the lexer's ASCII fold.
                if !definition
                    .categories
                    .iter()
                    .any(|category| category.eq_ignore_ascii_case(&grant.category))
                {
                    continue;
                }
                let into = if grant.add { &mut added } else { &mut factor };
                for &(slot, value) in &grant.stats {
                    into[slot] = into[slot].wrapping_add(value);
                }
            }
        }
        for slot in 0..STAT_COUNT {
            total[slot] = total[slot].wrapping_add(mul(equipment[slot], ONE + factor[slot]));
        }
        // The group's speed offer: its equipment speed per sub-unit, after support grants.
        let offer = mul(
            div(equipment[slots.maximum_speed], count(group.count)),
            ONE + factor[slots.maximum_speed],
        );
        if definition.affects_speed {
            speed = if speed == 0 { offer } else { speed.min(offer) };
        }
        speed = speed.max(operands.slowest_speed);
        let base = |slot: usize| {
            mul(
                definition.stats[slot].wrapping_add(added[slot]),
                ONE + factor[slot],
            )
        };
        let k = mul(
            count(group.count),
            group.critical.min(operands.manpower_fill),
        );
        for slot in slots.scaled_bases() {
            total[slot] = total[slot].wrapping_add(mul(k, base(slot)));
        }
        for slot in [slots.max_organisation, slots.default_morale] {
            total[slot] = total[slot].wrapping_add(mul(count(group.count), base(slot)));
        }
        for slot in [slots.fuel_capacity, slots.fuel_consumption_factor] {
            total[slot] = total[slot].wrapping_add(base(slot));
        }
        raw_fuel = raw_fuel
            .wrapping_add(equipment[slots.fuel_consumption])
            .wrapping_add(base(slots.fuel_consumption));
        width = width.wrapping_add(mul(k, definition.combat_width));
        total[slots.reliability] = total[slots.reliability].max(0);
        total[slots.supply_consumption] = total[slots.supply_consumption].max(0);
    }
    for slot in slots.sums() {
        stats[slot] = Some(total[slot]);
    }
    stats[slots.maximum_speed] = Some(speed);
    stats[slots.suppression] = Some(mul(
        total[slots.suppression],
        ONE + total[slots.suppression_factor],
    ));
    stats[slots.supply_consumption] = Some(mul(
        total[slots.supply_consumption],
        ONE + total[slots.supply_consumption_factor],
    ));
    stats[slots.fuel_consumption] = Some(mul(
        raw_fuel,
        (ONE + total[slots.fuel_consumption_factor]).max(0),
    ));
    for slot in slots.means() {
        stats[slot] = Some(div(total[slot], count(all)));
    }
    stats[slots.hardness] = Some(if non_support != 0 {
        div(total[slots.hardness], count(non_support))
    } else {
        total[slots.hardness]
    });
    let blend = |max: i64, mean: Option<i64>, weight: i64| {
        mean.map(|mean| mul(max, weight).wrapping_add(mul(mean, ONE - weight)))
    };
    stats[slots.armor_value] = blend(
        armor_max,
        stats[slots.armor_value],
        operands.armor_vs_average,
    );
    stats[slots.ap_attack] = blend(pen_max, stats[slots.ap_attack], operands.pen_vs_average);
    if raw_fuel > 0 {
        stats[slots.fuel_capacity] = Some(mul(
            mul(raw_fuel, operands.fuel_capacity_hours).wrapping_add(total[slots.fuel_capacity]),
            operands.army_fuel_cost,
        ));
    }
    Block {
        stats,
        combat_width: width,
        manpower,
        training_time,
    }
}
