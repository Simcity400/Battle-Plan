use super::*;
use content_reader_localisation::localisation::{argument_format, format_argument};

pub(super) fn changes_nothing(output: &content_reader_military::special_projects::Output) -> bool {
    output.sub_unit_bonus.is_empty()
        && output.equipment_bonus.is_empty()
        && output.enable_subunits.is_empty()
        && output.enable_equipments.is_empty()
        && output.enable_equipment_modules.is_empty()
        && [
            &output.country_effects,
            &output.scientist_effects,
            &output.facility_state_effects,
        ]
        .into_iter()
        .all(|effects| shows_only(effects, false))
}

fn shows_only(items: &[Item], branch: bool) -> bool {
    items.iter().all(|item| {
        item.keyword_is("custom_effect_tooltip")
            || item.keyword_is("effect_tooltip")
            || branch && item.keyword_is("limit")
            || ["if", "else_if", "else"]
                .into_iter()
                .any(|key| item.keyword_is(key))
                && shows_only(children(item), true)
    })
}

fn uses(count: i32) -> String {
    if count == 1 {
        "1 use".into()
    } else {
        format!("{count} uses")
    }
}

impl Renderer<'_, '_, '_> {
    pub(super) fn concise_leaf(&self, item: &Item, scopes: Scopes) -> Option<Result<String, String>> {
        let own = self.concise?;
        let key = item.keyword()?.folded();
        let number = |label: &str, key: &[u8]| self.concise_number(item, scopes, label, key);
        Some(match key.as_slice() {
            b"add_tech_bonus" => self.concise_tech_bonus(item, scopes),
            b"add_project_progress_ratio" => self.concise_project_progress(item, scopes, own),
            b"add_breakthrough_progress" => self.concise_breakthrough(item, scopes, true),
            b"add_breakthrough_points" => self.concise_breakthrough(item, scopes, false),
            b"add_scientist_xp" => self.concise_scientist_xp(item, scopes),
            b"army_experience" => number("Army experience", b"EFFECT_ADD_ARMY_EXPERIENCE"),
            b"navy_experience" => number("Navy experience", b"EFFECT_ADD_NAVY_EXPERIENCE"),
            b"air_experience" => number("Air experience", b"EFFECT_ADD_AIR_EXPERIENCE"),
            b"add_political_power" => number("Political power", b"POLITICS_ADD_POLITICAL_POWER"),
            b"add_stability" => number("Stability", b"EFFECT_ADD_STABILITY"),
            b"add_war_support" => number("War support", b"EFFECT_ADD_WAR_SUPPORT"),
            _ => return None,
        })
    }

    pub(super) fn concise_own_project(&self, scope: Scope) -> bool {
        matches!(scope, Scope::Project { owner, id } if self.concise == Some((owner, id)))
    }

    fn game_value(
        &self,
        key: &[u8],
        name: &str,
        value: Argument<'_>,
        signed: bool,
    ) -> Result<String, String> {
        let key = std::str::from_utf8(key).map_err(|_| "localisation key is not UTF-8")?;
        let template = self
            .context
            .localisation
            .get(key)
            .ok_or_else(|| format!("localisation key {key} is not defined"))?;
        let format = argument_format(template, name)
            .ok_or_else(|| format!("localisation {key} shows no {name}"))?;
        let format = if signed && !format.contains('=') {
            format!("={format}")
        } else {
            format.to_owned()
        };
        format_argument(value, format.as_bytes())
            .map_err(|why| format!("localisation {key} {name}: {why:?}"))
    }

    fn concise_tech_bonus(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let technology::TechBonus {
            list,
            ahead,
            bonus,
            uses: count,
        } = self.technology_bonus_facts(item, scopes)?;
        let key: &[u8] = match (ahead, bonus) {
            (0, 0) => return Ok(String::new()),
            (0, _) => b"EFFECT_ADD_TECH_BONUS_BONUS_ONLY",
            (_, 0) => b"EFFECT_ADD_TECH_BONUS_AHEAD_ONLY",
            _ => b"EFFECT_ADD_TECH_BONUS",
        };
        let mut text = String::new();
        if bonus != 0 {
            let bonus = self.game_value(key, "BONUS", Argument::Fixed5(bonus), true)?;
            text.push_str(&format!("Research bonus {bonus}"));
        }
        if ahead != 0 {
            let ahead = self.game_value(key, "AHEAD", Argument::Fixed5(ahead), false)?;
            text.push_str(&if text.is_empty() {
                format!("Ahead-of-time penalty reduced by {ahead} years")
            } else {
                format!(" and ahead-of-time penalty reduced by {ahead} years")
            });
        }
        if !list.is_empty() {
            text.push_str(&format!(" for {list}"));
        }
        text.push_str(&format!(" ({})\n", uses(count)));
        Ok(text)
    }

    fn concise_project_progress(
        &self,
        item: &Item,
        scopes: Scopes,
        own: (u32, u32),
    ) -> Result<String, String> {
        let (amount, owner, id) = self.project_progress_amount(item, scopes)?;
        let value = self.game_value(
            b"EFFECT_ADD_SPECIAL_PROJECT_PROGRESS",
            "PROGRESS",
            Argument::Fixed5(amount),
            true,
        )?;
        if (owner, id) == own {
            Ok(format!("Project progress {value}\n"))
        } else {
            Ok(format!("{} progress {value}\n", self.project_name(owner, id)?))
        }
    }

    fn concise_breakthrough(
        &self,
        item: &Item,
        scopes: Scopes,
        progress: bool,
    ) -> Result<String, String> {
        let Some(gains) = self.breakthrough_gains(item, scopes, progress)? else {
            return Ok(String::new());
        };
        let (key, label): (&[u8], _) = if progress {
            (b"ADD_BREAKTHROUGH_PROGRESS_EFFECT_SPECIALIZATION", "breakthrough")
        } else {
            (b"ADD_BREAKTHROUGH_POINTS_EFFECT_SPECIALIZATION", "breakthrough points")
        };
        let mut text = String::new();
        for (name, amount) in gains {
            let amount = self.game_value(key, "AMOUNT", amount, true)?;
            text.push_str(&format!("{name} {label} {amount}\n"));
        }
        Ok(text)
    }

    fn concise_scientist_xp(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some((character, specialization, value)) =
            self.char_scientist_gain(item, scopes, false)?
        else {
            return Ok(String::new());
        };
        let value = self.game_value(
            b"SCIENTIST_ADD_XP_SPECIALIZATION_EFFECT",
            "XP",
            Argument::Fixed5(value),
            true,
        )?;
        Ok(format!(
            "{}: {specialization} experience {value}\n",
            self.effect_character_name(&character)?
        ))
    }

    fn concise_number(
        &self,
        item: &Item,
        scopes: Scopes,
        label: &str,
        key: &[u8],
    ) -> Result<String, String> {
        let value = if scope_selector(item).is_some() {
            0
        } else {
            self.scoped_value(item.value.scalar().unwrap_or(b"0"), scopes)?
        };
        let value = self.game_value(key, "VAL", Argument::Fixed5(value), true)?;
        Ok(format!("{label} {value}\n"))
    }
}
