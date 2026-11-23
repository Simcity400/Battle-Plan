
use super::{AcclimatizationTerm, ContextRow, ModifierTerm, SupplyTerm, TransportTerm};
use engine_native_memory::{Memory, walk::Buffer};
use crate::combat_sources::Sources;
use engine_sites::Rva;

const ONE: i64 = 100_000;
const EXCEPTIONAL: i64 = 0xffff_ffff;
pub const SLOWEST_SPEED_RVA: Rva = Rva(0x033e_1ac8);
pub const SHARE_MAXIMUM: i64 = 2 * ONE;

type Speed = Result<i64, &'static str>;

const TRAP: &str = "the native fixed5 division traps on a reinforcement speed operand";

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

fn exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}

fn ratio(numerator: i64, denominator: i64, zero: i64) -> Speed {
    if denominator == 0 {
        return Ok(zero);
    }
    if exceptional(numerator) {
        return Ok(EXCEPTIONAL);
    }
    numerator
        .wrapping_mul(ONE)
        .checked_div(denominator)
        .ok_or(TRAP)
}

#[derive(Clone, Copy)]
pub struct Terms<'a> {
    pub context: &'a ContextRow,
    pub modifiers: &'a [ModifierTerm],
    pub supply: &'a [SupplyTerm],
    pub acclimatization: &'a [AcclimatizationTerm],
    pub transport: &'a [TransportTerm],
}

impl Terms<'_> {
    fn scope(&self, scope: u8) -> impl Iterator<Item = &ModifierTerm> {
        self.modifiers
            .iter()
            .filter(move |term| term.scope == scope)
    }

    fn term(&self, scope: u8, key: u32) -> Result<&ModifierTerm, &'static str> {
        let mut terms = self.scope(scope).filter(|term| term.key == key);
        let term = terms
            .next()
            .ok_or("a reinforcement modifier scope lacks a movement-speed key")?;
        match terms.next() {
            None => Ok(term),
            Some(_) => Err("a reinforcement modifier scope repeats a key"),
        }
    }

    fn effective(&self, term: &ModifierTerm, raw: i64) -> i64 {
        let ids = &self.context.ids;
        if term.present == 0
            || (term.feature_selector != 0 && (ids[6] == 0 || term.feature_selector & ids[5] == 0))
        {
            0
        } else if term.metadata_flags & 0x10 != 0 {
            raw.clamp(0, ONE)
        } else {
            raw
        }
    }

    fn value(&self, scope: u8, key: u32) -> Speed {
        let term = self.term(scope, key)?;
        Ok(self.effective(term, term.raw))
    }

    fn optional(&self, scope: u8, key: u32) -> Speed {
        if self.scope(scope).next().is_none() {
            Ok(0)
        } else {
            self.value(scope, key)
        }
    }

    fn exact_keys(&self, scope: u8, keys: &[u32], optional: bool) -> Result<(), &'static str> {
        let count = self.scope(scope).count();
        if (optional && count == 0)
            || (count == keys.len() && keys.iter().all(|&key| self.term(scope, key).is_ok()))
        {
            Ok(())
        } else {
            Err("a reinforcement modifier scope holds keys outside the movement-speed calculation")
        }
    }
}

fn check(value: impl Into<i64>, most: i64, reason: &'static str) -> Result<(), &'static str> {
    if value.into() > most {
        return Err(reason);
    }
    Ok(())
}

pub fn movement_factor(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    if context.ids[0] == 0 {
        return Err("the reinforcement operands name no location");
    }
    check(
        context.ids[6],
        1,
        "the eligibility callback is not one the port evaluates",
    )?;
    check(
        context.flags[11],
        2,
        "the supply mode is outside the native branches",
    )?;
    if terms
        .modifiers
        .iter()
        .any(|term| term.scope > 9 || term.key == 0 || term.present > 1)
    {
        return Err("a reinforcement modifier term is outside the captured forms");
    }
    if context.flags[11] == 2 {
        return transport(terms);
    }
    for (index, most) in [(4, 1), (5, 1), (6, 2), (7, 1), (8, 1), (9, 1)] {
        check(
            context.flags[index],
            most,
            "a reinforcement branch flag is outside the native branches",
        )?;
    }
    terms.exact_keys(0, &[0x2b, 0x1e9, 0x5e, 0x17f, 0x286], false)?;
    terms.exact_keys(1, &[0x2b, 0x5e, 0x17f], true)?;
    terms.exact_keys(2, &[0x2b, 0x1e9, 0x1e7, 0x1e8], context.ids[2] == 0)?;
    if context.ids[2] == 0 && terms.scope(2).next().is_some() {
        return Err("state modifier terms are recorded without a location state");
    }
    terms.exact_keys(3, &[0x2b, 0x1e9], true)?;
    terms.exact_keys(4, &[0x2b, 0x1e9], true)?;
    if terms.scope(5).count() > 1 {
        return Err("more than one priority subunit speed term is recorded");
    }
    if terms.modifiers.iter().any(|term| term.scope > 6) {
        return Err("a land reinforcement records a naval-transfer modifier scope");
    }
    let attacking = context.flags[5] != 0;
    let mut value = ONE
        .wrapping_add(terms.value(0, 0x2b)?)
        .wrapping_add(terms.optional(1, 0x2b)?);
    if attacking {
        value = value.wrapping_add(terms.value(0, 0x1e9)?);
    }
    if context.ids[2] != 0 {
        value = value.wrapping_add(terms.value(2, 0x2b)?);
        if attacking {
            value = value.wrapping_add(terms.value(2, 0x1e9)?);
        }
        let controller = match context.flags[6] {
            1 => Some(0x1e7),
            2 => Some(0x1e8),
            _ => None,
        };
        if let Some(key) = controller {
            value = value.wrapping_add(terms.value(2, key)?);
        }
    }
    value = value.wrapping_add(weather(terms)?);
    for term in terms.scope(5).chain(terms.scope(6)) {
        value = value.wrapping_add(terms.effective(term, term.raw));
    }
    value = value.wrapping_add(air(terms)?);
    value = value.wrapping_add(supply(terms)?);
    value = value.wrapping_add(infrastructure(terms)?);
    if context.flags[0] != 0 {
        value = value
            .wrapping_add(context.scalar_values[11])
            .wrapping_add(terms.value(0, 0x286)?);
    } else if context.flags[1] != 0 {
        value = value.wrapping_add(context.scalar_values[12]);
    }
    Ok(value)
}

fn weather(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    let mut factor = ONE;
    if context.flags[4] != 0 {
        let mut winner = None;
        let mut maximum = 0;
        for term in terms.acclimatization {
            if term.active > 1 {
                return Err("an acclimatization term's activity is outside 0 and 1");
            }
            if term.weight > maximum {
                maximum = term.weight;
                winner = Some(term);
            }
        }
        if winner.is_some_and(|term| term.active != 0) {
            let fraction = ratio(maximum, context.scalar_values[0], EXCEPTIONAL)?;
            let adjustment = mul(fraction, context.scalar_values[1]).clamp(0, ONE);
            factor = ONE.wrapping_sub(adjustment);
        }
    }
    let mut value = 0_i64;
    for scope in [3, 4] {
        value = value.wrapping_add(mul(factor, terms.optional(scope, 0x2b)?));
        if context.flags[5] != 0 {
            value = value.wrapping_add(mul(factor, terms.optional(scope, 0x1e9)?));
        }
    }
    Ok(value)
}

fn air(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    let s = &context.scalar_values;
    if context.flags[7] == 0 || s[14] <= 0 {
        return Ok(0);
    }
    let sum = s[13].wrapping_add(s[14]);
    let share = if sum > 0 {
        ratio(s[14], sum, EXCEPTIONAL)?
    } else {
        0
    };
    let penalty = mul(mul(s[15], share), s[17]);
    let reduction = engine_native_parts::csub_unit_definition::movement_air_defense_value(s[16], s[18], s[19])?;
    Ok(penalty.wrapping_add(reduction).min(0))
}

fn supply(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    if context.flags[2] != 0 {
        return if context.flags[11] == 0 {
            Ok(0)
        } else {
            Err("a supplied reserve records an out-of-supply mode")
        };
    }
    if context.flags[11] != 1 {
        return Err("an out-of-supply reserve records no ordinary supply mode");
    }
    let s = &context.scalar_values;
    if s[5..8].iter().any(|value| i32::try_from(*value).is_err()) {
        return Err("a supply day or define operand is outside its native i32 field");
    }
    let supplied = if s[2] <= 0 {
        0
    } else {
        let grace = ratio(s[3], s[6].wrapping_mul(ONE), EXCEPTIONAL)?;
        s[4].max(grace).clamp(0, ONE)
    };
    let days = s[5].wrapping_mul(ONE);
    let denominator = s[7].wrapping_mul(ONE);
    let days_ratio = if denominator == 0 || days <= i64::MIN + 1 {
        EXCEPTIONAL
    } else {
        days.wrapping_mul(ONE)
            .checked_div(denominator)
            .ok_or(TRAP)?
    };
    let mut severity = mul(ONE.wrapping_sub(supplied), days_ratio);
    if severity != 0 {
        let mut total = 0_i32;
        let mut special = 0_i32;
        for term in terms.supply {
            total = total.wrapping_add(term.count);
            if term.special != 0 {
                special = special.wrapping_add(term.count);
            }
        }
        let denominator = i64::from(total).wrapping_mul(ONE);
        let share = if denominator == 0 {
            0
        } else {
            i64::from(special)
                .wrapping_mul(ONE)
                .wrapping_mul(ONE)
                .checked_div(denominator)
                .ok_or(TRAP)?
        };
        let mut factor = ONE.wrapping_add(terms.value(0, 0x5e)?);
        if share > 0 {
            factor = factor.wrapping_add(mul(share, terms.value(0, 0x17f)?));
        }
        factor = factor.wrapping_add(terms.optional(1, 0x5e)?);
        if share > 0 {
            factor = factor.wrapping_add(mul(share, terms.optional(1, 0x17f)?));
        }
        severity = mul(factor.max(0), severity);
    }
    let mut value = mul(s[8], severity);
    if context.flags[8] != 0 {
        value = value.wrapping_add(s[9]);
    }
    Ok(value)
}

fn infrastructure(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    if context.flags[9] == 0 {
        return if context.ids[3] == 0 {
            Ok(0)
        } else {
            Err("an infrastructure state is recorded without its branch flag")
        };
    }
    if context.ids[3] == 0 {
        return Err("the infrastructure branch is recorded without its state");
    }
    let s = &context.scalar_values;
    let maximum = if s[20] > 0 {
        s[20]
    } else if s[21] > 0 {
        s[21]
    } else {
        s[22]
    };
    let maximum = i32::try_from(maximum)
        .map_err(|_| "the infrastructure maximum is outside its native i32 field")?;
    let level = if context.flags[10] == 0 {
        0
    } else {
        i32::from(
            i16::try_from(s[24])
                .map_err(|_| "the infrastructure level is outside its native i16 field")?,
        )
    };
    Ok((i64::from(maximum.wrapping_sub(level))
        .wrapping_mul(s[23])
        .wrapping_mul(ONE)
        / ONE)
        .max(-95_000))
}

fn transport(terms: &Terms<'_>) -> Speed {
    let context = terms.context;
    terms.exact_keys(7, &[0x29], true)?;
    terms.exact_keys(8, &[0x29], true)?;
    terms.exact_keys(9, &[0x1a7], true)?;
    if terms.modifiers.iter().any(|term| term.scope < 7) {
        return Err("a naval-transfer reinforcement records a land modifier scope");
    }
    let value = ONE.wrapping_add(terms.optional(7, 0x29)?);
    let source = terms.term(8, 0x29).ok();
    let reduction_term = terms.term(9, 0x1a7).ok();
    if source.is_some() != reduction_term.is_some() {
        return Err("a naval-transfer reinforcement records one of its two static scopes");
    }
    if source.is_none()
        && (!terms.transport.is_empty() || context.scalar_values[25..=29].iter().any(|&v| v != 0))
    {
        return Err("naval-transfer operands are recorded without their static scopes");
    }
    if context.ids[5] & (1 << 4) == 0 {
        return if source.is_none() {
            Ok(value)
        } else {
            Err("naval-transfer static scopes are recorded without the feature bit")
        };
    }
    let Some(source) = source else {
        return Ok(value);
    };
    let s = &context.scalar_values;
    let category_mask = u32::try_from(s[29])
        .map_err(|_| "the transport category mask is outside its native u32 field")?;
    let mut total = 0_i64;
    let mut weighted = 0_i64;
    for term in terms.transport {
        total = total.wrapping_add(term.regional_value);
        weighted = weighted.wrapping_add(mul(
            term.regional_value,
            term.own_intel.wrapping_sub(term.enemy_intel),
        ));
    }
    let average = ratio(weighted, total, 0)?;
    let density = if total < 0 {
        0
    } else if total > s[25] {
        s[25]
    } else {
        total
    };
    let mut amount = ratio(density, s[25], EXCEPTIONAL)?;
    if amount != 0 {
        let mut reduction = terms.value(9, 0x1a7)?;
        let difference = average.wrapping_sub(s[26]);
        if difference > 0 {
            let range = s[27].wrapping_sub(s[26]);
            let capped = difference.min(range);
            let scaled = mul(capped, s[28]);
            reduction = reduction.wrapping_add(ratio(scaled, range, 0)?);
        }
        if reduction != 0 {
            amount = mul(ONE.wrapping_sub(reduction), amount).clamp(0, ONE);
        }
    }
    if amount > 0 && category_mask != 0 && source.present != 0 {
        Ok(value.wrapping_add(terms.effective(source, mul(amount, source.raw))))
    } else {
        Ok(value)
    }
}

pub fn speed_share(base_speed: i64, factor: i64, slowest: i64) -> Speed {
    let speed = mul(base_speed, factor);
    if slowest == 0 || exceptional(speed) {
        return Ok(SHARE_MAXIMUM);
    }
    let share = speed
        .wrapping_mul(ONE)
        .checked_div(slowest)
        .ok_or(TRAP)?
        .wrapping_sub(ONE);
    Ok(share.clamp(0, SHARE_MAXIMUM))
}

#[derive(Default)]
pub struct Scratch {
    modifiers: Vec<ModifierTerm>,
    supply: Vec<SupplyTerm>,
    acclimatization: Vec<AcclimatizationTerm>,
    transport: Vec<TransportTerm>,
}

impl Scratch {
    pub fn allocated_bytes(&self) -> usize {
        self.modifiers.capacity() * size_of::<ModifierTerm>()
            + self.supply.capacity() * size_of::<SupplyTerm>()
            + self.acclimatization.capacity() * size_of::<AcclimatizationTerm>()
            + self.transport.capacity() * size_of::<TransportTerm>()
    }
}

#[allow(clippy::too_many_arguments)]
pub fn capture_share(
    memory: &impl Memory,
    base: usize,
    game: usize,
    sources: &mut Sources,
    army: usize,
    handle: u64,
    base_speed: i64,
    scratch: &mut Scratch,
) -> Speed {
    observe(
        memory, base, game, sources, army, handle, base_speed, scratch, [0; 4],
    )?
    .compute(scratch)
}

#[derive(Clone, Copy)]
struct Observation {
    context: ContextRow,
    base_speed: i64,
    slowest: i64,
    starts: [usize; 4],
}

#[allow(clippy::too_many_arguments)]
fn observe(
    memory: &impl Memory,
    base: usize,
    game: usize,
    sources: &mut Sources,
    army: usize,
    handle: u64,
    base_speed: i64,
    scratch: &mut Scratch,
    starts: [usize; 4],
) -> Result<Observation, &'static str> {
    let context = super::capture_context_buffer(
        memory,
        base,
        game,
        sources,
        army,
        handle,
        Buffer::GrowingFrom(&mut scratch.modifiers, starts[0]),
        Buffer::GrowingFrom(&mut scratch.supply, starts[1]),
        Buffer::GrowingFrom(&mut scratch.acclimatization, starts[2]),
        &mut scratch.transport,
        starts[3],
    )?;
    let slowest = sources.field::<i64>(
        memory,
        engine_sites::at(base, SLOWEST_SPEED_RVA)
            .ok_or("the SLOWEST_SPEED define is unreadable")?,
        0,
        "the SLOWEST_SPEED define is unreadable",
    )?;
    Ok(Observation {
        context,
        base_speed,
        slowest,
        starts,
    })
}

impl Observation {
    fn counts(&self) -> [usize; 4] {
        [
            self.context.modifier_count,
            self.context.supply_term_count,
            self.context.acclimatization_count,
            self.context.transport_term_count,
        ]
    }

    fn compute(&self, scratch: &Scratch) -> Speed {
        let context = &self.context;
        let span = |index: usize| {
            self.starts[index]
                .checked_add(self.counts()[index])
                .map(|end| self.starts[index]..end)
        };
        let terms = Terms {
            context,
            modifiers: scratch
                .modifiers
                .get(span(0).ok_or("reinforcement modifier prefix exceeds its buffer")?)
                .ok_or("reinforcement modifier prefix exceeds its buffer")?,
            supply: scratch
                .supply
                .get(span(1).ok_or("reinforcement supply prefix exceeds its buffer")?)
                .ok_or("reinforcement supply prefix exceeds its buffer")?,
            acclimatization: scratch
                .acclimatization
                .get(span(2).ok_or("reinforcement acclimatization prefix exceeds its buffer")?)
                .ok_or("reinforcement acclimatization prefix exceeds its buffer")?,
            transport: scratch
                .transport
                .get(span(3).ok_or("reinforcement transport prefix exceeds its buffer")?)
                .ok_or("reinforcement transport prefix exceeds its buffer")?,
        };
        speed_share(self.base_speed, movement_factor(&terms)?, self.slowest)
    }
}

#[derive(Default)]
pub struct Observations {
    scratch: Scratch,
    rows: Vec<Observation>,
    used: [usize; 4],
}

impl Observations {
    pub fn reset(&mut self) {
        self.rows.clear();
        self.used = [0; 4];
    }

    pub fn allocated_bytes(&self) -> usize {
        self.scratch.allocated_bytes() + self.rows.capacity() * size_of::<Observation>()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game: usize,
        sources: &mut Sources,
        army: usize,
        handle: u64,
        base_speed: i64,
    ) -> Result<usize, &'static str> {
        let row = observe(
            memory,
            base,
            game,
            sources,
            army,
            handle,
            base_speed,
            &mut self.scratch,
            self.used,
        )?;
        let mut used = self.used;
        for (end, count) in used.iter_mut().zip(row.counts()) {
            *end = end
                .checked_add(count)
                .ok_or("reinforcement observation length overflow")?;
        }
        engine_allocation::reserve(&mut self.rows, 1)
            .map_err(|_| "reinforcement observation allocation failed")?;
        let index = self.rows.len();
        self.rows.push(row);
        self.used = used;
        Ok(index)
    }

    pub fn compute(&self, index: usize) -> Speed {
        self.rows
            .get(index)
            .ok_or("reinforcement observation index outside its rows")?
            .compute(&self.scratch)
    }
}

#[cfg(test)]
mod tests;
