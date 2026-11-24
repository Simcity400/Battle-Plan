use super::{Context, ModeHover, line};
use content_reader_localisation::localisation::Argument as Arg;
use replay_data::{rows::Recorded, rows::tables};
use viewer_native_gui::native_text::FLAG;
use viewer_foundation::tooltip_text::Tooltip;

fn highlighted(text: &str) -> String {
    format!("\u{11}H{text}\u{11}!")
}

impl Context<'_> {
    pub(super) fn category(&self) -> Result<String, String> {
        let token = match self.state_row()?.category {
            Recorded::Value(Some(token)) => token,
            Recorded::Value(None) => return Err("the state's category read was refused".to_owned()),
            Recorded::NotRecorded => {
                return Err("the state's live category is not recorded".to_owned());
            }
        };
        let names = dashboard_text::table_access::TokenNames::new(self.frame)
            .map_err(|e| e.to_string())?;
        let category = names.name(token).map_err(|e| e.to_string())?;
        let name = self.loc(category, &[])?;
        self.loc(
            "PROVINCE_STATE_CATERGORY_TOOLTIP",
            &[("CATEGORY", Arg::Text(&name))],
        )
    }

    pub(super) fn state_row(&self) -> Result<&replay_data::rows::StateRow, String> {
        let id = self.province.state;
        self.frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|state| state.id == id)
            .ok_or_else(|| "state row absent".to_owned())
    }

    fn viewer_raw(&self) -> Result<u32, String> {
        let ours = self.vision.ours();
        self.links
            .iter()
            .find_map(|(&raw, &slot)| (slot == ours).then_some(raw))
            .ok_or_else(|| format!("viewing country {ours} has no CountryLinks row"))
    }

    pub(super) fn mode_lines(&self, mode: &ModeHover, out: &mut Tooltip) {
        if let ModeHover::StrategicAir(selected) = mode {
            return super::strategic_air::append(self, selected, out);
        }
        if let ModeHover::StrategicNavy(sources) = mode {
            return super::strategic_navy::append(self, sources, out);
        }
        if let ModeHover::SeaRegion(sources) = mode {
            return super::strategic_navy::default_region(self, sources, out);
        }
        if let ModeHover::Terrain { .. } = mode {
            return out.refuse("map mode hover", "the terrain mode's text is made from the combat content".into());
        }
        if let ModeHover::Infrastructure(level) = mode {
            let text = level.clone().and_then(|level| {
                self.loc(
                    "MAPMODE_TOOLTIP_INFRASTRUCTURE",
                    &[("INF", Arg::Int32(level))],
                )
            });
            return line(out, "infrastructure", 0, text);
        }
        if self.province.state == 0 {
            if let ModeHover::Unbuilt(why) = mode {
                out.refuse("map mode hover", why.clone());
            }
            return;
        }
        match mode {
            ModeHover::Location { .. }
            | ModeHover::Nothing
            | ModeHover::Infrastructure(_)
            | ModeHover::StrategicAir(_) => {}
            ModeHover::StrategicNavy(_) | ModeHover::SeaRegion(_) | ModeHover::Region | ModeHover::Terrain { .. } => {}
            ModeHover::Manpower => line(out, "population", 0, self.population()),
            ModeHover::Diplomacy {
                very_good,
                very_bad,
            } => line(out, "diplomacy", 0, self.diplomacy(*very_good, *very_bad)),
            ModeHover::Ideology => line(out, "ideology", 0, self.ideology()),
            ModeHover::Operatives => super::operatives::append(self, out),
            ModeHover::Occupation => self.occupation(out, false),
            ModeHover::OccupationMarker => self.occupation(out, true),
            ModeHover::Resources => self.resource_lines(out),
            ModeHover::Scripted(name) => {
                line(
                    out,
                    "scripted mode",
                    0,
                    self.scripted(&format!("{name}_tooltip")),
                );
                match self.scripted(&format!("{name}_tooltip_delayed")) {
                    Ok(text) => out.body = text,
                    Err(why) => out.refuse("scripted mode delayed", why),
                }
            }
            ModeHover::Unbuilt(why) => out.refuse("map mode hover", why.clone()),
        }
    }

    fn population(&self) -> Result<String, String> {
        match self.state_row()?.population {
            Recorded::Value(population) => self.loc(
                "MAPMODE_TOOLTIP_MANPOWER",
                &[("MP", Arg::Int32(population))],
            ),
            Recorded::NotRecorded => Err("the state's population is not recorded".to_owned()),
        }
    }

    fn diplomacy(&self, very_good: i32, very_bad: i32) -> Result<String, String> {
        let state = self.state_row()?;
        let owner_slot = self.slot(state.owner)?;
        let tag = self
            .countries
            .tag(owner_slot, "country naming operands absent")?;
        let owner = self.countries.name(owner_slot)?;
        let mut text = format!("{} ( {owner} ) - ( {FLAG}{} )\n\n", self.state_name()?, tag);
        let viewer_raw = self.viewer_raw()?;
        let viewer = self.countries.name(self.vision.ours())?;
        let args = [("COUNTRY", Arg::Text(&owner)), ("FROM", Arg::Text(&viewer))];
        let relation = if owner_slot == self.vision.ours() {
            highlighted(&owner)
        } else if replay_derive::directed_wars::Owner::new(self.frame, viewer_raw)?
            .at_war(state.owner)?
        {
            self.loc("MAPMODE_ENEMY", &args)?
        } else if self.same_faction(owner_slot)? {
            self.loc("MAPMODE_FACTION", &args)?
        } else {
            let opinion = self
                .frame
                .complete::<tables::RelationOpinions>()
                .map_err(|e| e.to_string())?
                .iter()
                .find(|row| {
                    self.links.get(&row.owner_country) == Some(&owner_slot)
                        && self.links.get(&row.target_country) == Some(&self.vision.ours())
                })
                .map_or(0, |row| row.opinion);
            if opinion > very_good {
                self.loc("MAPMODE_GOOD_OPINION", &args)?
            } else if opinion < very_bad {
                self.loc("MAPMODE_BAD_OPINION", &args)?
            } else {
                String::new()
            }
        };
        text.push_str(&relation);
        Ok(text)
    }

    fn same_faction(&self, owner_slot: u32) -> Result<bool, String> {
        let members = self
            .frame
            .complete::<tables::FactionMembers>()
            .map_err(|e| e.to_string())?;
        let faction_of = |slot: u32| {
            members
                .iter()
                .find(|row| self.links.get(&row.country) == Some(&slot))
                .map(|row| row.faction)
        };
        Ok(faction_of(self.vision.ours())
            .is_some_and(|faction| faction_of(owner_slot) == Some(faction)))
    }

    fn ideology(&self) -> Result<String, String> {
        let state = self.state_row()?;
        if state.owner == 0 {
            return Ok(String::new());
        }
        let owner_slot = self.slot(state.owner)?;
        let ruling = self
            .frame
            .complete::<tables::Politics>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| self.links.get(&row.country) == Some(&owner_slot))
            .and_then(|row| row.ruling_party);
        let Some(ruling) = ruling else {
            return Ok(String::new());
        };
        let party = self
            .frame
            .complete::<tables::PoliticalParties>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.id == ruling && self.links.get(&row.country) == Some(&owner_slot))
            .ok_or("the owner's ruling party has no PoliticalParties row")?;
        let token = String::from_utf8(party.token.clone())
            .map_err(|_| "the ruling party's ideology is not UTF-8".to_owned())?;
        self.loc(&token, &[])
    }

    fn occupation(&self, out: &mut Tooltip, marker: bool) {
        let row = self
            .frame
            .complete::<tables::StateResistance>()
            .map_err(|e| e.to_string())
            .map(|rows| {
                rows.iter()
                    .find(|row| row.state == self.province.state)
                    .filter(|row| marker || row.active != 0)
            });
        match row {
            Ok(Some(row)) => {
                let mut refused = Vec::new();
                let text = if marker {
                    match (&row.hover, &row.modifier_sources) {
                        (Recorded::Value(Some(hover)), Recorded::Value(sources)) => {
                            self.occupation_marker(row, hover, sources, &mut refused)
                        }
                        _ => Err("occupation marker resistance description is not recorded".into()),
                    }
                } else {
                    self.occupation_lines(row, &mut refused)
                };
                line(out, "occupied state", 0, text);
                for (key, why) in refused {
                    out.refuse("state resistance", format!("{key}: {why}"));
                }
            }
            Ok(None) => line(
                out,
                "state resistance",
                0,
                self.loc("RESISTANCE_NOT_OPERATIONAL", &[]),
            ),
            Err(why) => line(out, "state resistance", 0, Err(why)),
        }
    }

    fn occupation_lines(
        &self,
        row: &replay_data::rows::StateResistanceRow,
        refused: &mut Vec<(&'static str, String)>,
    ) -> Result<String, String> {
        if let (Recorded::Value(Some(hover)), Recorded::Value(sources)) =
            (&row.hover, &row.modifier_sources)
        {
            return self.occupation_complete(row, hover, sources, refused);
        }
        let defines = self
            .content
            .province_hover
            .occupation
            .as_ref()
            .map_err(Clone::clone)?;
        let mut refuse = |key: &'static str, why: &str| refused.push((key, why.to_owned()));
        let value = |key: &str, value: i64| self.loc(key, &[("VALUE", Arg::Fixed5(value))]);
        let mut lines = vec![highlighted(&self.state_name()?), String::new()];
        lines.push(self.loc(
            "RESISTANCE_STRENGTH",
            &[("TOT", Arg::Fixed5(percent(row.level)))],
        )?);
        lines.push(String::new());
        refuse("RESISTANCE_TARGET_VALUE", TARGET_UNRECORDED);
        lines.push(value(
            "MODIFIER_BASE_VALUE_PERCENTAGE",
            percent(defines.target_base),
        )?);
        let claim = self.occupation_claim();
        let stability = self.occupation_stability();
        for (key, amount) in [
            (
                "RESISTANCE_TARGET_MODIFIER_HAS_CLAIM",
                claim
                    .clone()
                    .map(|claim| if claim { defines.claim } else { 0 }),
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_POP_LOW",
                self.state_row().and_then(|state| match state.population {
                    Recorded::Value(population) => Ok(defines.population(population)),
                    Recorded::NotRecorded => Err("the state's population is not recorded".into()),
                }),
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_IS_AT_PEACE",
                stability
                    .as_ref()
                    .map(|row| if row.at_war == 0 { defines.peace } else { 0 })
                    .map_err(Clone::clone),
            ),
            (
                "RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS",
                stability.as_ref().map_err(Clone::clone).and_then(|row| {
                    replay_data::country_stability::stability(row)
                        .value
                        .map(|stability| defines.stability_loss(stability))
                        .ok_or_else(|| {
                            "the controller's stability modifier read was refused".into()
                        })
                }),
            ),
        ] {
            match amount {
                Ok(0) => {}
                Ok(amount) => lines.push(value(key, percent(amount))?),
                Err(why) => refuse(key, &why),
            }
        }
        match self.capitulated(row.country) {
            Ok(true) => lines.push(value(
                "RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED",
                percent(defines.occupied_capitulated),
            )?),
            Ok(false) => {}
            Err(why) => refuse("RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED", &why),
        }
        refuse(
            "RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE",
            "the government in exile's host and legitimacy are not recorded",
        );
        match self
            .state_victory_points()
            .map(|points| defines.state_vp(points))
        {
            Ok(0) => {}
            Ok(vp) => lines.push(value("RESISTANCE_TARGET_MODIFIER_STATE_VP", percent(vp))?),
            Err(why) => refuse("RESISTANCE_TARGET_MODIFIER_STATE_VP", &why),
        }
        refuse(
            "resistance target modifiers",
            "the controller's, state's and relation's modifier totals are not recorded",
        );
        refuse(
            "added resistance targets",
            "the state's scripted target additions are not recorded",
        );
        refuse(
            "RESISTANCE_TARGET_FLOW",
            "the resistance spread caches are not recorded",
        );
        refuse(
            "RESISTANCE_TARGET_CAP_FOR_NON_COMPLIANCE",
            TARGET_UNRECORDED,
        );
        let compliance_points = whole_points(mul(row.compliance, defines.per_compliance));
        if compliance_points != 0 {
            lines.push(value(
                "RESISTANCE_TARGET_MODIFIER_PER_COMPLIANCE",
                compliance_points * FIXED5 / 100,
            )?);
        }
        refuse(
            "RESISTANCE_GROWTH_VALUE, RESISTANCE_DECAY_VALUE, RESISTANCE_DISABLED_INFO",
            TARGET_UNRECORDED,
        );
        refuse(
            "RESISTANCE_EFFECTS, COMPLIANCE_EFFECTS",
            "the effect blocks' modifier lines are not built",
        );
        lines.push(String::new());
        lines.push(self.loc(
            "COMPLIANCE_STRENGTH",
            &[("TOT", Arg::Fixed5(percent(row.compliance)))],
        )?);
        lines.push(String::new());
        refuse(
            "COMPLIANCE_GROWTH_VALUE, COMPLIANCE_DECAY_VALUE",
            "the compliance gain modifiers are not recorded",
        );
        let base = super::occupation::base_value(defines.compliance_growth_base, 'G');
        lines.push(self.loc("MODIFIER_BASE_VALUE_STR", &[("VALUE", Arg::Text(&base))])?);
        for (key, amount) in [
            (
                "COMPLIANCE_GROWTH_HAS_CLAIM",
                claim.map(|claim| if claim { defines.compliance_claim } else { 0 }),
            ),
            (
                "COMPLIANCE_GROWTH_IS_AT_PEACE",
                stability
                    .as_ref()
                    .map(|row| {
                        if row.at_war == 0 {
                            defines.compliance_peace
                        } else {
                            0
                        }
                    })
                    .map_err(Clone::clone),
            ),
        ] {
            match amount {
                Ok(0) => {}
                Ok(amount) => lines.push(value(key, percent(amount))?),
                Err(why) => refuse(key, &why),
            }
        }
        refuse(
            "compliance growth modifiers",
            "the controller's, state's and relation's modifier totals are not recorded",
        );
        let decay = mul(percent(row.compliance), defines.decay_at_max_compliance);
        if decay != 0 {
            lines.push(self.loc("COMPLIANCE_DECAY_INFO", &[])?);
            lines.push(self.loc(
                "COMPLIANCE_DECAY_DUE_TO_COMPLIANCE",
                &[("VALUE", Arg::Fixed5(decay))],
            )?);
        } else {
            refuse(
                "COMPLIANCE_DECAY_INFO",
                "whether a later decay term shows is not recorded",
            );
        }
        refuse(
            "COMPLIANCE_DECAY_PER_EXILE_LEGITIMACY",
            "the government in exile's legitimacy is not recorded",
        );
        refuse(
            "compliance gain modifiers, COMPLIANCE_GAIN_IS_DISABLED, COMPLIANCE_GAIN_IS_SCALED",
            "the compliance gain modifier totals are not recorded",
        );
        Ok(lines.join("\n"))
    }

    pub(super) fn occupation_claim(&self) -> Result<bool, String> {
        let controller = self.state_row()?.controller;
        let slot = self.slot(controller)?;
        let state = self
            .frame
            .complete::<tables::StateCores>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| row.state == self.province.state)
            .ok_or("the state's core and claim row is absent")?;
        let Recorded::Value(claims) = &state.claims else {
            return Err("the state's claims are not recorded".into());
        };
        claims.iter().try_fold(false, |found, &claim| {
            if found || claim == controller {
                return Ok(true);
            }
            Ok(claim != 0 && controller != 0 && self.slot(claim)? == slot)
        })
    }

    pub(super) fn occupation_stability(
        &self,
    ) -> Result<&replay_data::rows::CountryStabilityInputRow, String> {
        let controller = self.slot(self.state_row()?.controller)?;
        self.frame
            .complete::<tables::CountryStabilityInputs>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| self.links.get(&row.country) == Some(&controller))
            .ok_or_else(|| "the controller's stability inputs are absent".into())
    }

    pub(super) fn capitulated(&self, occupied: u32) -> Result<bool, String> {
        let slot = self.slot(occupied)?;
        let row = self
            .frame
            .complete::<tables::CountryCounterInputs>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|row| self.links.get(&row.country) == Some(&slot))
            .ok_or("the occupied country has no CountryCounterInputs row")?;
        Ok(row.owned_state_count >= 1 && row.controlled_state_count == 0)
    }

    pub(super) fn state_victory_points(&self) -> Result<i64, String> {
        Ok(self
            .frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|province| province.state == self.province.state)
            .map(|province| i64::from(province.victory_points))
            .sum())
    }

    fn scripted(&self, key: &str) -> Result<String, String> {
        use dashboard_text::scripted_text::{
            EventObject, EventScope, EventScopes, Link, ScriptedText,
        };
        let text = self
            .content
            .localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())?;
        if !text.contains('[') {
            return Ok(text);
        }
        let content = self
            .content
            .scripted
            .as_ref()
            .as_ref()
            .map_err(Clone::clone)?;
        let viewer = self.vision.ours();
        let scopes = EventScopes::new(vec![
            EventScope {
                from: Link::Scope(1),
                ..EventScope::of(EventObject::Country(viewer))
            },
            EventScope {
                from: Link::Scope(0),
                ..EventScope::of(EventObject::State(self.province.state))
            },
        ]);
        ScriptedText::new(self.frame, &self.content.localisation, content, viewer)
            .viewing(viewer)
            .expand_in(&text, &scopes)
            .map_err(|why| format!("{key}: {why}"))
    }
}

pub(super) const FIXED5: i64 = 100_000;

const TARGET_UNRECORDED: &str =
    "the resistance target is not recorded (its modifier, claim, population and spread terms)";

pub(super) fn percent(points: i64) -> i64 {
    if [i64::MIN, i64::MIN + 1, i64::MAX].contains(&points) {
        i64::from(u32::MAX)
    } else {
        points.wrapping_mul(FIXED5) / 10_000_000
    }
}

pub(super) fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / FIXED5
}

pub(super) fn whole_points(value: i64) -> i64 {
    value / FIXED5
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct OccupationDefines {
    pub(super) target_base: i64,
    pub(super) claim: i64,
    pub(super) peace: i64,
    pub(super) per_stability_loss: i64,
    pub(super) population_low: i64,
    pub(super) population_very_low: i64,
    pub(super) population_low_cutoff: i32,
    pub(super) population_very_low_cutoff: i32,
    pub(super) occupied_capitulated: i64,
    pub(super) per_compliance: i64,
    pub(super) state_vp: Vec<(i64, i64)>,
    pub(super) compliance_growth_base: i64,
    pub(super) compliance_claim: i64,
    pub(super) compliance_peace: i64,
    pub(super) decay_at_max_compliance: i64,
    pub(super) max_stability: i64,
    pub(super) max_legitimacy: i64,
    pub(super) exile_min: i64,
    pub(super) exile_max: i64,
    pub(super) target_floor: i64,
    pub(super) reenable: i64,
    pub(super) decay_base: i64,
    pub(super) decay_min: i64,
    pub(super) decay_max: i64,
    pub(super) decay_claim: i64,
    pub(super) decay_factors: Vec<(i64, i64)>,
    pub(super) growth_base: i64,
    pub(super) growth_min: i64,
    pub(super) growth_max: i64,
    pub(super) compliance_min: i64,
    pub(super) compliance_max: i64,
    pub(super) exile_decay: i64,
    pub(super) no_gain_limit: i64,
}

impl OccupationDefines {
    pub(super) fn load(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        let value = |name: &str| defines.fixed5("NResistance", name);
        let pairs = defines.fixed5_array("NResistance", "RESISTANCE_TARGET_MODIFIER_STATE_VP")?;
        if pairs.len() % 2 != 0 {
            return Err(std::io::Error::other(
                "NResistance.RESISTANCE_TARGET_MODIFIER_STATE_VP holds an odd number of values",
            ));
        }
        Ok(Self {
            target_base: value("RESISTANCE_TARGET_BASE")?,
            claim: value("RESISTANCE_TARGET_MODIFIER_HAS_CLAIM")?,
            peace: value("RESISTANCE_TARGET_MODIFIER_IS_AT_PEACE")?,
            per_stability_loss: value("RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS")?,
            population_low: value("RESISTANCE_TARGET_MODIFIER_POP_LOW")?,
            population_very_low: value("RESISTANCE_TARGET_MODIFIER_POP_VERY_LOW")?,
            population_low_cutoff: defines.int32("NResistance", "RESISTANCE_POP_LOW_CUTOFF")?,
            population_very_low_cutoff: defines
                .int32("NResistance", "RESISTANCE_POP_VERY_LOW_CUTOFF")?,
            occupied_capitulated: value("RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED")?,
            per_compliance: value("RESISTANCE_TARGET_MODIFIER_PER_COMPLIANCE")?,
            state_vp: pairs
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .collect(),
            compliance_growth_base: value("COMPLIANCE_GROWTH_BASE")?,
            compliance_claim: value("COMPLIANCE_GROWTH_HAS_CLAIM")?,
            compliance_peace: value("COMPLIANCE_GROWTH_IS_AT_PEACE")?,
            decay_at_max_compliance: value("COMPLIANCE_DECAY_AT_MAX_COMPLIANCE")?,
            max_stability: defines.fixed5("NCountry", "MAX_STABILITY")?,
            max_legitimacy: defines.fixed5("NCountry", "GIE_MAX_LEGITIMACY")?,
            exile_min: value("RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE_MIN")?,
            exile_max: value("RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE_MAX")?,
            target_floor: value("RESISTANCE_TARGET_MIN_CAP_FOR_NON_COMPLIANCE")?,
            reenable: value("RESISTANCE_TARGET_TO_REENABLE_RESISTANCE")?,
            decay_base: value("RESISTANCE_DECAY_BASE")?,
            decay_min: value("RESISTANCE_DECAY_MIN")?,
            decay_max: value("RESISTANCE_DECAY_MAX")?,
            decay_claim: value("RESISTANCE_DECAY_MODIFIER_HAS_CLAIM")?,
            decay_factors: {
                let values =
                    defines.fixed5_array("NResistance", "RESISTANCE_DECAY_MODIFIER_FACTORS")?;
                if values.len() % 2 != 0 {
                    return Err(std::io::Error::other(
                        "NResistance.RESISTANCE_DECAY_MODIFIER_FACTORS holds an odd number of values",
                    ));
                }
                values.chunks_exact(2).map(|v| (v[0], v[1])).collect()
            },
            growth_base: value("RESISTANCE_GROWTH_BASE")?,
            growth_min: value("RESISTANCE_GROWTH_MIN")?,
            growth_max: value("RESISTANCE_GROWTH_MAX")?,
            compliance_min: value("COMPLIANCE_GROWTH_MIN")?,
            compliance_max: value("COMPLIANCE_GROWTH_MAX")?,
            exile_decay: value("COMPLIANCE_DECAY_PER_EXILE_LEGITIMACY")?,
            no_gain_limit: value("NO_COMPLIANCE_GAIN_ENABLE_LIMIT")?,
        })
    }

    pub(super) fn state_vp(&self, victory_points: i64) -> i64 {
        self.state_vp
            .iter()
            .rfind(|(total, _)| victory_points * FIXED5 >= *total)
            .map_or(0, |(_, modifier)| *modifier)
    }

    pub(super) fn population(&self, population: i32) -> i64 {
        if population > self.population_low_cutoff {
            0
        } else if population > self.population_very_low_cutoff {
            self.population_low
        } else {
            self.population_very_low
        }
    }

    pub(super) fn stability_loss(&self, stability: i64) -> i64 {
        let points = (self.max_stability.wrapping_sub(stability))
            .wrapping_mul(self.per_stability_loss)
            .wrapping_mul(100)
            / 10_000_000_000;
        i64::from(points as i32).wrapping_mul(FIXED5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupation_defines_use_the_resistance_section_and_keep_inclusive_population_cutoffs() {
        let defines = content_reader_defines::defines::Defines::new().unwrap();
        let names = [
            "RESISTANCE_TARGET_BASE",
            "RESISTANCE_TARGET_MODIFIER_HAS_CLAIM",
            "RESISTANCE_TARGET_MODIFIER_IS_AT_PEACE",
            "RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS",
            "RESISTANCE_TARGET_MODIFIER_POP_LOW",
            "RESISTANCE_TARGET_MODIFIER_POP_VERY_LOW",
            "RESISTANCE_POP_LOW_CUTOFF",
            "RESISTANCE_POP_VERY_LOW_CUTOFF",
            "RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED",
            "RESISTANCE_TARGET_MODIFIER_PER_COMPLIANCE",
            "COMPLIANCE_GROWTH_BASE",
            "COMPLIANCE_GROWTH_HAS_CLAIM",
            "COMPLIANCE_GROWTH_IS_AT_PEACE",
            "COMPLIANCE_DECAY_AT_MAX_COMPLIANCE",
            "RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE_MIN",
            "RESISTANCE_TARGET_MODIFIER_OCCUPIED_IS_EXILE_MAX",
            "RESISTANCE_TARGET_MIN_CAP_FOR_NON_COMPLIANCE",
            "RESISTANCE_TARGET_TO_REENABLE_RESISTANCE",
            "RESISTANCE_DECAY_BASE",
            "RESISTANCE_DECAY_MIN",
            "RESISTANCE_DECAY_MAX",
            "RESISTANCE_DECAY_MODIFIER_HAS_CLAIM",
            "RESISTANCE_GROWTH_BASE",
            "RESISTANCE_GROWTH_MIN",
            "RESISTANCE_GROWTH_MAX",
            "COMPLIANCE_GROWTH_MIN",
            "COMPLIANCE_GROWTH_MAX",
            "COMPLIANCE_DECAY_PER_EXILE_LEGITIMACY",
            "NO_COMPLIANCE_GAIN_ENABLE_LIMIT",
        ];
        let fields = names
            .iter()
            .map(|name| format!("{name}=0"))
            .collect::<Vec<_>>()
            .join(",");
        defines.load("fixture.lua",format!("NDefines={{NCountry={{MAX_STABILITY=1.25,GIE_MAX_LEGITIMACY=200}},NResistance={{{fields},RESISTANCE_TARGET_MODIFIER_STATE_VP={{1,2}},RESISTANCE_DECAY_MODIFIER_FACTORS={{3,4}}}}}}\nNDefines.NResistance.RESISTANCE_TARGET_BASE=11.25\nNDefines.NResistance.RESISTANCE_POP_LOW_CUTOFF=123\nNDefines.NResistance.RESISTANCE_POP_VERY_LOW_CUTOFF=12\nNDefines.NResistance.RESISTANCE_TARGET_MODIFIER_POP_LOW=-8\nNDefines.NResistance.RESISTANCE_TARGET_MODIFIER_POP_VERY_LOW=-13\nNDefines.NResistance.RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS=0.17").as_bytes()).unwrap();
        let d = OccupationDefines::load(&defines).unwrap();
        assert_eq!(d.target_base, 1_125_000);
        assert_eq!(
            [
                d.population(124),
                d.population(123),
                d.population(13),
                d.population(12)
            ],
            [0, -800_000, -800_000, -1_300_000]
        );
        assert_eq!(d.stability_loss(FIXED5), 400_000);
        assert_eq!(d.stability_loss(125_000), 0);
    }

    #[test]
    fn occupation_arithmetic_truncates_as_the_game_does() {
        assert_eq!(percent(1_234_567), 12_345);
        assert_eq!(percent(i64::MAX), i64::from(u32::MAX));
        assert_eq!(whole_points(mul(4_000_000, -50_000)), -20);
        assert_eq!(whole_points(mul(3_999_999, -50_000)), -19);
        let defines = OccupationDefines {
            target_base: 3_500_000,
            occupied_capitulated: 1_000_000,
            per_compliance: -50_000,
            state_vp: vec![(500_000, -100_000), (2_000_000, -500_000)],
            compliance_growth_base: 7_500,
            decay_at_max_compliance: -8_300,
            ..Default::default()
        };
        assert_eq!(defines.state_vp(0), 0);
        assert_eq!(defines.state_vp(5), -100_000);
        assert_eq!(defines.state_vp(25), -500_000);
    }
}
