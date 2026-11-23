use dashboard_effects::ScriptedEffects;
use super::inputs::Names;
use super::{Arg, Tooltip, counter_content};
use replay_data::{rows::AirCountryRelationRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use viewer_foundation::{allegiance::NativeRelations, perspective::Vision};
use replay_derive::{
    directed_wars::Owner,
    naval_mine_levels::Levels,
    naval_region_control::raw_roster,
};

type Fact<T> = Result<T, String>;
const ONE: i64 = 100_000;

#[derive(Default, Debug, PartialEq, Eq)]
struct Side {
    mines: i64,
    damaged: i32,
    sunk: i32,
}
impl Side {
    fn casualty(&mut self, sunk: u8) {
        if sunk == 0 {
            self.damaged = self.damaged.wrapping_add(1)
        } else {
            self.sunk = self.sunk.wrapping_add(1)
        }
    }
    fn shown(&self) -> bool {
        self.mines > 0 || self.damaged != 0 || self.sunk != 0
    }
}

pub fn build(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    include_neutral: bool,
) -> Tooltip {
    let mut out = Tooltip::default();
    let links = match frame.complete::<tables::CountryLinks>() {
        Ok(rows) => rows
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect(),
        Err(e) => {
            out.refuse("regional mines", e.to_string());
            return out;
        }
    };
    let inputs = Inputs {
        content,
        frame,
        vision,
        region,
        links,
        countries: Names::new(frame, &content.localisation),
    };
    match inputs.summary(include_neutral) {
        Ok((text, penalty)) => {
            out.title = text;
            if let Some(reason) = penalty {
                out.refuse("regional mine penalties", reason);
            }
        }
        Err(reason) => out.refuse("regional mines", reason),
    }
    out
}

pub fn enemy_share(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
) -> Fact<i64> {
    let links: BTreeMap<u32, u32> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let inputs = Inputs {
        content,
        frame,
        vision,
        region,
        links,
        countries: Names::new(frame, &content.localisation),
    };
    let levels = Levels::read(frame, &inputs.links)?;
    let enemies = raw_roster(frame, vision.ours(), AirCountryRelationRow::ENEMY)?;
    inputs.penalty(inputs.raw()?, &enemies, &levels)
}

struct Inputs<'a> {
    content: &'a counter_content::Content,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    region: u32,
    links: BTreeMap<u32, u32>,
    countries: Names<'a>,
}
impl Inputs<'_> {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Fact<String> {
        self.content
            .localisation
            .localise_with(key.as_bytes(), args)
            .map_err(|e| e.to_string())
    }
    fn slot(&self, raw: u32) -> Fact<u32> {
        if raw == 0 {
            Ok(0)
        } else {
            self.links
                .get(&raw)
                .copied()
                .ok_or_else(|| "mine summary country link is unavailable".into())
        }
    }
    fn raw(&self) -> Fact<u32> {
        self.frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.canonical_slot == self.vision.ours())
            .map(|row| row.id)
            .ok_or_else(|| "mine summary viewing country is unavailable".into())
    }
    fn positive_states(&self, raw: u32) -> Fact<bool> {
        Ok(self
            .frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| self.slot(row.country) == self.slot(raw))
            .ok_or("mine summary owned-state count is unavailable")?
            .owned_state_count
            > 0)
    }
    fn summary(&self, include_neutral: bool) -> Fact<(String, Option<String>)> {
        let raw = self.raw()?;
        if !self.positive_states(raw)? {
            return Ok((String::new(), None));
        }
        if !viewer_foundation::naval_mine_feature::enabled(self.frame)? {
            return Ok((String::new(), None));
        }
        let levels = Levels::read(self.frame, &self.links)?;
        let enemies = raw_roster(self.frame, self.vision.ours(), AirCountryRelationRow::ENEMY)?;
        let allies = raw_roster(self.frame, self.vision.ours(), AirCountryRelationRow::ALLY)?;
        let incidents = self
            .frame
            .complete::<tables::NavalAccidentReports>()
            .map_err(|e| e.to_string())?;
        let grouping = NativeRelations::read(self.frame, &self.links).map_err(|e| e.to_string())?;
        let mut ours = Side {
            mines: levels.amount(raw, self.region)?,
            ..Default::default()
        };
        let mut friends = Side::default();
        let mut hostile = Side::default();
        let at_war = |owner, target| Owner::new(self.frame, owner)?.pointer_active(target);
        for report in incidents.iter().filter(|row| {
            row.kind == 1 && self.slot(row.country) == self.slot(raw) && row.region == self.region
        }) {
            hostile.casualty(report.sunk);
        }
        if !enemies.is_empty() {
            for &enemy in &enemies {
                if (enemy as i32) > 0 {
                    hostile.mines = hostile
                        .mines
                        .wrapping_add(levels.amount(enemy, self.region)?);
                }
                for report in incidents.iter().filter(|row| {
                    row.kind == 1
                        && self.slot(row.country) == self.slot(enemy)
                        && row.region == self.region
                }) {
                    if self.slot(report.owner)? == self.vision.ours() {
                        ours.casualty(report.sunk)
                    } else if grouping
                        .co_belligerent(self.vision.ours(), self.slot(report.owner)?)?
                    {
                        friends.casualty(report.sunk)
                    }
                }
            }
        }
        for &ally in &allies {
            if (ally as i32) > 0 {
                friends.mines = friends
                    .mines
                    .wrapping_add(levels.amount(ally, self.region)?);
            }
        }
        for &enemy in &enemies {
            if (enemy as i32) <= 0 {
                continue;
            }
            for report in incidents.iter().filter(|row| {
                row.kind == 1
                    && self.slot(row.country) == self.slot(enemy)
                    && row.region == self.region
            }) {
                if at_war(raw, enemy)? && at_war(report.owner, enemy)? {
                    hostile.casualty(report.sunk);
                }
            }
        }
        let mut neutral = Vec::new();
        if include_neutral {
            let enemy_slots = enemies
                .iter()
                .map(|&raw| self.slot(raw))
                .collect::<Fact<BTreeSet<_>>>()?;
            let ally_slots = allies
                .iter()
                .map(|&raw| self.slot(raw))
                .collect::<Fact<BTreeSet<_>>>()?;
            let mut countries: Vec<_> = self
                .frame
                .complete::<tables::Countries>()
                .map_err(|e| e.to_string())?
                .iter()
                .collect();
            countries.sort_by_key(|row| row.canonical_slot);
            for country in countries {
                if country.id as i32 <= 0 || !self.positive_states(country.id)? {
                    continue;
                }
                let amount = levels.amount(country.id, self.region)?;
                if amount > 0
                    && !enemy_slots.contains(&country.canonical_slot)
                    && !ally_slots.contains(&country.canonical_slot)
                {
                    neutral.push((country.canonical_slot, amount));
                }
            }
        }
        if !ours.shown() && !friends.shown() && !hostile.shown() && neutral.is_empty() {
            return Ok((String::new(), None));
        }
        let mut text = self.loc("NAVAL_MINES_IN_REGION_HEADER", &[])?;
        for (key, side) in [
            ("NAVAL_MINES_IN_REGION_BY_US", ours),
            ("NAVAL_MINES_IN_REGION_BY_FRIENDS", friends),
            ("NAVAL_MINES_IN_REGION_BY_ENEMIES", hostile),
        ] {
            if side.shown() {
                text.push_str(&self.loc(
                    key,
                    &[
                        ("COUNT", Arg::Fixed5(side.mines)),
                        ("COUNT_DAMAGED", Arg::Int32(side.damaged)),
                        ("COUNT_SUNK", Arg::Int32(side.sunk)),
                    ],
                )?);
            }
        }
        for (slot, amount) in neutral {
            let tag = self
                .countries
                .tag(slot, "mine neutral country naming operands are unavailable")?;
            let name = self.countries.name_with(slot, "_DEF")?;
            let name = format!(
                "{}{} {}",
                viewer_native_gui::native_text::FLAG,
                tag,
                content_reader_localisation::localisation::crt_case_text(&name, true, true)
            );
            text.push_str(&self.loc(
                "NAVAL_MINES_IN_REGION_BY_NEUTRAL",
                &[
                    ("COUNTRY", Arg::Text(&name)),
                    ("COUNT", Arg::Fixed5(amount)),
                ],
            )?);
        }
        let penalty = (|| -> Fact<String> {
            if !self.has_static_effect()? {
                return Ok(String::new());
            }
            let scale = self.penalty(raw, &enemies, &levels)?;
            if scale <= 0 {
                return Ok(String::new());
            }
            let body = self.static_effect(scale)?;
            if body.is_empty() {
                return Ok(String::new());
            }
            Ok(format!(
                "{}{}",
                self.loc("NAVAL_MINES_PENALTIES_FROM_ENEMY", &[])?,
                body
            ))
        })();
        match penalty {
            Ok(penalty) => {
                text.push_str(&penalty);
                Ok((text, None))
            }
            Err(reason) => Ok((text, Some(reason))),
        }
    }
    fn penalty(&self, raw: u32, enemies: &[u32], levels: &Levels<'_>) -> Fact<i64> {
        let mut amount = 0_i64;
        let mut weighted_intel = 0_i64;
        for &enemy in enemies {
            if enemy as i32 <= 0 || !self.positive_states(enemy)? {
                continue;
            }
            let row = self
                .frame
                .complete::<tables::Capitulation>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|row| self.slot(row.country) == self.slot(enemy))
                .ok_or("mine penalty enemy capitulation is unavailable")?;
            if row.capitulated != 0 {
                continue;
            }
            let mines = levels.amount(enemy, self.region)?;
            amount = amount.wrapping_add(mines);
            let difference = self
                .intel(raw, enemy)?
                .wrapping_sub(self.intel(enemy, raw)?);
            weighted_intel = weighted_intel.wrapping_add(mines.wrapping_mul(difference) / ONE);
        }
        let d = &self.content.province_hover.navy;
        let maximum = d.mine_max.clone()?;
        let mut factor = density(amount, maximum);
        if factor == 0 {
            return Ok(0);
        }
        let weighted_intel = guarded_fraction(weighted_intel, amount, 0);
        let row = self
            .frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| self.slot(row.country) == self.slot(raw))
            .ok_or("mine penalty country accumulator is unavailable")?;
        let raw_reduction = *row
            .mines_effect_reduction
            .require("CountryCounterInputs.mines_effect_reduction")?
            .as_ref()
            .ok_or("mine reduction accumulator census was refused")?;
        let mut reduction = if let Some(raw_reduction) = raw_reduction {
            let key = engine_executable::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION;
            let definition = self
                .frame
                .complete::<tables::ModifierDefinitions>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|row| row.index == key)
                .ok_or("mine reduction modifier definition is unavailable")?;
            let mask = self
                .frame
                .complete::<tables::SimulationFeatures>()
                .map_err(|e| e.to_string())?
                .first()
                .map(|row| row.dlc_mask);
            replay_derive::naval_mission_derive::effective(definition, mask, Some(raw_reduction))
                .map_err(str::to_owned)?
        } else {
            0
        };
        let lower = d.mine_intel_low.clone()?;
        let excess = weighted_intel.wrapping_sub(lower);
        if excess > 0 {
            let width = d.mine_intel_high.clone()?.wrapping_sub(lower);
            let bonus = excess.min(width).wrapping_mul(d.mine_intel_factor.clone()?) / ONE;
            reduction = reduction.wrapping_add(guarded_fraction(bonus, width, 0));
        }
        if reduction != 0 {
            factor = ONE.wrapping_sub(reduction).wrapping_mul(factor) / ONE;
            factor = factor.clamp(0, ONE);
        }
        Ok(factor)
    }
    fn intel(&self, source: u32, target: u32) -> Fact<i64> {
        if source as i32 <= 0 || target as i32 <= 0 {
            return Ok(0);
        }
        let owner = self.slot(source)?;
        let target = self.slot(target)?;
        let rows = self
            .frame
            .complete::<tables::CountryIntel>()
            .map_err(|e| e.to_string())?;
        let mut count = 0_u32;
        let mut value = 0;
        for row in rows {
            if self.slot(row.owner_country)? == owner {
                count = count.max(row.target_index + 1);
                if row.target_index == target {
                    value = row.navy;
                }
            }
        }
        Ok(if target < count { value } else { 0 })
    }
    fn has_static_effect(&self) -> Fact<bool> {
        let scripted = self
            .content
            .scripted
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)?;
        let effects = scripted.effects()?;
        let view = effects.view(None);
        let static_modifiers = view
            .static_modifiers
            .ok_or("naval mine static modifier content is unavailable")?;
        Ok(static_modifiers
            .get(&scripted.layers.layers, "naval_mines_effect")?
            .0
            .is_some())
    }
    fn static_effect(&self, scale: i64) -> Fact<String> {
        let scripted = self
            .content
            .scripted
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)?;
        let text = dashboard_text::scripted_text::ScriptedText::new(
            self.frame,
            &self.content.localisation,
            scripted,
            self.vision.ours(),
        )
        .viewing(self.vision.ours());
        let effects = scripted.effects()?;
        let context = dashboard_effects::effect_description::Context::new(
            &text,
            &self.content.localisation,
            self.vision.ours(),
            "common/modifiers",
            effects.view(None),
        );
        dashboard_effects::effect_description::scaled_static_modifiers(
            &context,
            "naval_mines_effect",
            "  ",
            scale,
        )
    }
}
fn density(amount: i64, maximum: i64) -> i64 {
    guarded_fraction(
        if amount < 0 {
            0
        } else if maximum < amount {
            maximum
        } else {
            amount
        },
        maximum,
        0xffff_ffff,
    )
}

fn guarded_fraction(value: i64, divisor: i64, empty: i64) -> i64 {
    if divisor == 0 {
        empty
    } else if (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        0xffff_ffff
    } else {
        value.wrapping_mul(ONE).wrapping_div(divisor)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn casualties_keep_occurrences_raw_sunk_truth_and_wrapping_counts() {
        let mut side = Side::default();
        for flag in [0, 0, 2, 255] {
            side.casualty(flag)
        }
        assert_eq!((side.damaged, side.sunk), (2, 2));
        assert!(side.shown());
        side.damaged = i32::MAX;
        side.casualty(0);
        assert_eq!(side.damaged, i32::MIN);
        assert!(
            !Side {
                mines: -1,
                ..Default::default()
            }
            .shown()
        );
    }
    #[test]
    fn mine_fractions_keep_distinct_empty_defaults_and_signed_guards() {
        assert_eq!(guarded_fraction(0, 0, 0), 0);
        assert_eq!(guarded_fraction(0, 0, 0xffff_ffff), 0xffff_ffff);
        assert_eq!(guarded_fraction(-7, 3, 0), -233_333);
        assert_eq!(guarded_fraction(i64::MAX, 3, 0), 0xffff_ffff);
    }
    #[test]
    fn modded_negative_mine_maximum_keeps_the_ordered_clamp() {
        assert_eq!(density(1, -10), 100_000);
        assert_eq!(density(-1, -10), 0);
        assert_eq!(density(0, 0), 0xffff_ffff);
        assert_eq!(density(30, 20), 100_000);
    }
}
