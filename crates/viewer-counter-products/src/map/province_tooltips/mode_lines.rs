//! The province hover of a map mode other than the default and states modes: the mode's own
//! lines alone, as the game shows them, without the location block, units or selection.
use super::{Context, ModeHover, line};
use installed_content::localisation::Argument as Arg;
use replay::rows::{Recorded, tables};
use viewer_foundation::map::native_text::FLAG;
use viewer_foundation::tooltip_text::Tooltip;

/// The highlight colour code pair around a name, as the game writes it.
fn highlighted(text: &str) -> String {
    format!("\u{11}H{text}\u{11}!")
}

impl Context<'_> {
    /// The state category line the states mode adds after the controller, naming the state's
    /// live category (States `category`).
    pub(super) fn category(&self) -> Result<String, String> {
        let token = match self.state_row()?.category {
            Recorded::Value(Some(token)) => token,
            Recorded::Value(None) => return Err("the state's category read was refused".to_owned()),
            Recorded::NotRecorded => {
                return Err("the state's live category is not recorded".to_owned());
            }
        };
        let names = viewer_foundation::table_access::TokenNames::new(self.frame)
            .map_err(|e| e.to_string())?;
        let category = names.name(token).map_err(|e| e.to_string())?;
        let name = self.loc(category, &[])?;
        self.loc(
            "PROVINCE_STATE_CATERGORY_TOOLTIP",
            &[("CATEGORY", Arg::Text(&name))],
        )
    }

    fn state_row(&self) -> Result<&replay::rows::StateRow, String> {
        let id = self.province.state;
        self.frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|state| state.id == id)
            .ok_or_else(|| "state row absent".to_owned())
    }

    /// The viewing country's raw reference.
    fn viewer_raw(&self) -> Result<u32, String> {
        let ours = self.vision.ours();
        self.links
            .iter()
            .find_map(|(&raw, &slot)| (slot == ours).then_some(raw))
            .ok_or_else(|| format!("viewing country {ours} has no CountryLinks row"))
    }

    pub(super) fn mode_lines(&self, mode: &ModeHover, out: &mut Tooltip) {
        // The infrastructure line shows for every province, a stateless one at level zero.
        if let ModeHover::Infrastructure(level) = mode {
            let text = level.clone().and_then(|level| {
                self.loc(
                    "MAPMODE_TOOLTIP_INFRASTRUCTURE",
                    &[("INF", Arg::Int32(level))],
                )
            });
            return line(out, "infrastructure", 0, text);
        }
        // Every other mode's own lines describe the province's state.
        if self.province.state == 0 {
            if let ModeHover::Unbuilt(why) = mode {
                out.refuse("map mode hover", why.clone());
            }
            return;
        }
        match mode {
            ModeHover::Location { .. } | ModeHover::Nothing | ModeHover::Infrastructure(_) => {}
            ModeHover::Manpower => line(out, "population", 0, self.population()),
            ModeHover::Diplomacy {
                very_good,
                very_bad,
            } => line(out, "diplomacy", 0, self.diplomacy(*very_good, *very_bad)),
            ModeHover::Ideology => line(out, "ideology", 0, self.ideology()),
            ModeHover::Operatives => super::operatives::append(self, out),
            ModeHover::Occupation => self.occupation(out),
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

    /// The state, its owner, and the owner's relation to the viewing country, the first that
    /// holds of: the viewing country itself, war, a shared faction, a very good opinion and a
    /// very bad opinion.
    fn diplomacy(&self, very_good: i32, very_bad: i32) -> Result<String, String> {
        let state = self.state_row()?;
        let owner_slot = self.slot(state.owner)?;
        let operands = self
            .countries
            .operands(owner_slot)
            .ok_or("country naming operands absent")?
            .as_ref()
            .map_err(Clone::clone)?;
        let owner = self.countries.name(owner_slot)?;
        let mut text = format!(
            "{} ( {owner} ) - ( {FLAG}{} )\n\n",
            self.state_name()?,
            operands.tag
        );
        let viewer_raw = self.viewer_raw()?;
        let viewer = self.countries.name(self.vision.ours())?;
        let args = [("COUNTRY", Arg::Text(&owner)), ("FROM", Arg::Text(&viewer))];
        let relation = if owner_slot == self.vision.ours() {
            highlighted(&owner)
        } else if viewer_foundation::map::directed_wars::Owner::new(self.frame, viewer_raw)
            .map_err(|why| why.to_string())?
            .at_war(state.owner)
            .map_err(|why| why.to_string())?
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

    /// Whether the viewing country is in a faction the owner is a member of.
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

    /// The name of the owner's ruling ideology.
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

    /// The resistance and compliance modes' hover, the same in both: the state, its resistance
    /// and compliance with each line the recorded hour can give, or why resistance cannot grow
    /// there. A line whose operands are not recorded is refused by its key.
    fn occupation(&self, out: &mut Tooltip) {
        let row = self
            .frame
            .complete::<tables::StateResistance>()
            .map_err(|e| e.to_string())
            .map(|rows| {
                rows.iter()
                    .find(|row| row.state == self.province.state)
                    .filter(|row| row.active != 0)
                    .cloned()
            });
        match row {
            Ok(Some(row)) => {
                let mut refused = Vec::new();
                let text = self.occupation_lines(&row, &mut refused);
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

    /// The occupied state's lines in the game's order, blank lines before the target, rate and
    /// effects sections; `refused` collects each line left out, by key.
    fn occupation_lines(
        &self,
        row: &replay::rows::StateResistanceRow,
        refused: &mut Vec<(&'static str, String)>,
    ) -> Result<String, String> {
        let defines = self
            .content
            .province_hover
            .occupation
            .as_ref()
            .map_err(Clone::clone)?;
        let mut refuse = |key: &'static str, why: &str| refused.push((key, why.to_owned()));
        // A line whose own operands are missing is refused alone; the others still show.
        let value = |key: &str, value: i64| self.loc(key, &[("VALUE", Arg::Fixed5(value))]);
        let mut lines = vec![highlighted(&self.state_name()?), String::new()];
        lines.push(self.loc(
            "RESISTANCE_STRENGTH",
            &[("TOT", Arg::Fixed5(percent(row.level)))],
        )?);
        // The target: its value and each term the hour can give.
        lines.push(String::new());
        refuse("RESISTANCE_TARGET_VALUE", TARGET_UNRECORDED);
        lines.push(value(
            "MODIFIER_BASE_VALUE_PERCENTAGE",
            percent(defines.target_base),
        )?);
        refuse(
            "RESISTANCE_TARGET_MODIFIER_HAS_CLAIM",
            "the state's claims are not recorded",
        );
        refuse(
            "RESISTANCE_TARGET_MODIFIER_POP_LOW",
            "the state's current population is not recorded",
        );
        refuse(
            "RESISTANCE_TARGET_MODIFIER_IS_AT_PEACE",
            "the controller's peace test is not derived",
        );
        refuse(
            "RESISTANCE_TARGET_MODIFIER_PER_STABILITY_LOSS",
            "the controller's effective stability is not derived",
        );
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
        // The rate section follows the target, which is not recorded.
        refuse(
            "RESISTANCE_GROWTH_VALUE, RESISTANCE_DECAY_VALUE, RESISTANCE_DISABLED_INFO",
            TARGET_UNRECORDED,
        );
        refuse(
            "RESISTANCE_EFFECTS, COMPLIANCE_EFFECTS",
            "the effect blocks' modifier lines are not built",
        );
        // Compliance.
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
        let base = format!(
            "\u{11}G{}\u{11}!",
            installed_content::fixed5::fixed5_to_string(defines.compliance_growth_base, 3)
        );
        lines.push(self.loc("MODIFIER_BASE_VALUE_STR", &[("VALUE", Arg::Text(&base))])?);
        refuse(
            "COMPLIANCE_GROWTH_HAS_CLAIM",
            "the state's claims are not recorded",
        );
        refuse(
            "COMPLIANCE_GROWTH_IS_AT_PEACE",
            "the controller's peace test is not derived",
        );
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

    /// Whether the occupied country (a raw reference) is capitulated as the resistance target
    /// reads it: it owns a state and controls none.
    fn capitulated(&self, occupied: u32) -> Result<bool, String> {
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

    /// The victory points of the hovered state's provinces.
    fn state_victory_points(&self) -> Result<i64, String> {
        Ok(self
            .frame
            .complete::<tables::Provinces>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|province| province.state == self.province.state)
            .map(|province| i64::from(province.victory_points))
            .sum())
    }

    /// A scripted mode's tooltip text, expanded in the game's scopes: the viewing country as
    /// ROOT, THIS and PREV, and the hovered province's state as FROM, whose own ROOT is itself and
    /// whose FROM is the viewing country. A scope function the shared expansion does not support
    /// is refused by name.
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
            .expand_in(&text, &scopes)
            .map_err(|why| format!("{key}: {why}"))
    }
}

/// One in fixed5.
const FIXED5: i64 = 100_000;

/// Why the resistance target's own lines are refused.
const TARGET_UNRECORDED: &str =
    "the resistance target is not recorded (its modifier, claim, population and spread terms)";

/// A fixed5 percentage-point value as the fraction the percent formats print.
fn percent(points: i64) -> i64 {
    points / 100
}

/// Fixed5 multiplication, truncated toward zero.
fn mul(a: i64, b: i64) -> i64 {
    ((i128::from(a) * i128::from(b)) / i128::from(FIXED5)) as i64
}

/// A fixed5 value truncated to whole units.
fn whole_points(value: i64) -> i64 {
    value / FIXED5
}

/// The `NCountry` resistance and compliance defines the occupation hover reads, each fixed5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OccupationDefines {
    target_base: i64,
    occupied_capitulated: i64,
    per_compliance: i64,
    /// `RESISTANCE_TARGET_MODIFIER_STATE_VP`: (total victory points, target modifier) pairs.
    state_vp: Vec<(i64, i64)>,
    compliance_growth_base: i64,
    decay_at_max_compliance: i64,
}

impl OccupationDefines {
    pub(super) fn load(defines: &installed_content::defines::Defines) -> std::io::Result<Self> {
        let value = |name: &str| defines.fixed5("NCountry", name);
        let pairs = defines.fixed5_array("NCountry", "RESISTANCE_TARGET_MODIFIER_STATE_VP")?;
        if pairs.len() % 2 != 0 {
            return Err(std::io::Error::other(
                "NCountry.RESISTANCE_TARGET_MODIFIER_STATE_VP holds an odd number of values",
            ));
        }
        Ok(Self {
            target_base: value("RESISTANCE_TARGET_BASE")?,
            occupied_capitulated: value("RESISTANCE_TARGET_MODIFIER_OCCUPIED_CAPITULATED")?,
            per_compliance: value("RESISTANCE_TARGET_MODIFIER_PER_COMPLIANCE")?,
            state_vp: pairs
                .chunks_exact(2)
                .map(|pair| (pair[0], pair[1]))
                .collect(),
            compliance_growth_base: value("COMPLIANCE_GROWTH_BASE")?,
            decay_at_max_compliance: value("COMPLIANCE_DECAY_AT_MAX_COMPLIANCE")?,
        })
    }

    /// The target modifier of the last pair whose victory point total the state reaches.
    fn state_vp(&self, victory_points: i64) -> i64 {
        self.state_vp
            .iter()
            .filter(|(total, _)| victory_points * FIXED5 >= *total)
            .last()
            .map_or(0, |(_, modifier)| *modifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occupation_arithmetic_truncates_as_the_game_does() {
        // 12.34567% resistance prints as 12.34%: the level over a hundred, still fixed5.
        assert_eq!(percent(1_234_567), 12_345);
        // 40% compliance at -0.5 per point is -20 whole points; just under it, -19.
        assert_eq!(whole_points(mul(4_000_000, -50_000)), -20);
        assert_eq!(whole_points(mul(3_999_999, -50_000)), -19);
        let defines = OccupationDefines {
            target_base: 3_500_000,
            occupied_capitulated: 1_000_000,
            per_compliance: -50_000,
            state_vp: vec![(500_000, -100_000), (2_000_000, -500_000)],
            compliance_growth_base: 7_500,
            decay_at_max_compliance: -8_300,
        };
        assert_eq!(defines.state_vp(0), 0);
        assert_eq!(defines.state_vp(5), -100_000);
        assert_eq!(defines.state_vp(25), -500_000);
    }
}
