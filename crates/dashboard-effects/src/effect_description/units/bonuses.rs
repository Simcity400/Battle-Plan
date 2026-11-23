
use super::*;
use content_reader_script::equipment_stats::{STAT_COUNT, stat_metadata};
use content_reader_parser::tokens::Token;
use content_reader_key_names::unit_leader::Adjuster;

fn newline(text: &mut String) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
}

fn stats(items: &[Item]) -> [i64; STAT_COUNT] {
    let mut values = [0; STAT_COUNT];
    for item in items {
        if let Some(index) = item
            .keyword()
            .and_then(|key| content_reader_script::equipment_stats::stat_index(&key.folded()))
            && let Some(value) = content_reader_script::modifiers::read_fixed5(item)
        {
            values[index] = value;
        }
    }
    values
}

fn adjuster(key: &str, items: &[Item]) -> Adjuster {
    let mut result = Adjuster {
        key: key.into(),
        ..Default::default()
    };
    for block in items.iter().filter(|item| item.keyword_is(key)) {
        for item in children(block) {
            let target = if item.keyword_is("movement") {
                &mut result.movement
            } else if item.keyword_is("attack") {
                &mut result.attack
            } else if item.keyword_is("defence") {
                &mut result.defence
            } else {
                continue;
            };
            if let Some(value) = content_reader_script::modifiers::read_fixed5(item) {
                *target = value;
            }
        }
    }
    result
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn units_bonus_description(
        &mut self,
        items: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        if items.iter().all(|item| item.keyword_is("name")) {
            return Ok(String::new());
        }
        let context = self.context;
        let names = match context.content.key_names {
            Some(names) => names,
            None => context
                .text
                .content()
                .effects()?
                .key_names
                .as_ref()
                .ok_or_else(|| self.content_gap("effect key names"))?,
        };
        let units = names
            .sub_units
            .as_ref()
            .ok_or("unit-bonus sub-unit registry unavailable")?;
        let mut entries = Vec::new();
        for item in items.iter().filter(|item| !item.keyword_is("name")) {
            let Some(key) = item.keyword() else {
                continue;
            };
            let token = Token::Name(key.folded());
            let selected = units.token_name(&token).and_then(|unit| {
                if unit.is_empty() {
                    units
                        .category_name(&token)
                        .map(|name| name.map(|name| (true, name)))
                } else {
                    Ok(Some((false, unit)))
                }
            });
            match selected {
                Ok(Some((category, name))) => entries.push((category, name, children(item))),
                Ok(None) => {}
                Err(why) => self.refuse(&format!("unit bonus {}", key.text().escape_ascii()), why),
            }
        }
        let mut text = String::new();
        for category in [false, true] {
            for (_, name, fields) in entries.iter().filter(|entry| entry.0 == category) {
                match self.units_bonus_body(name, fields, category, units, scopes) {
                    Ok(body) => {
                        newline(&mut text);
                        text.push_str(&body);
                    }
                    Err(why) => self.refuse(&format!("unit bonus {name}"), why),
                }
            }
        }
        Ok(text)
    }

    fn units_bonus_stat(&self, index: usize, value: i64, scopes: Scopes) -> Result<String, String> {
        let (_, label, _) = stat_metadata()[index];
        let mut text = self.localised(label.as_bytes(), &[], scopes, false)?;
        let candidate = format!("{label}_MOD_VALUE");
        let key = if self.context.localisation.get(&candidate).is_some() {
            candidate.as_bytes()
        } else {
            b"STAT_MODIFIER_VALUE"
        };
        text.push_str(&self.localised(
            key,
            &[("VALUE", Argument::Fixed5(value))],
            scopes,
            false,
        )?);
        text.push('\n');
        Ok(text)
    }

    fn units_bonus_adjuster(&self, adjuster: &Adjuster, scopes: Scopes) -> Result<String, String> {
        let lines = self.char_adjuster_lines(adjuster, scopes)?;
        if lines.is_empty() {
            return Ok(lines);
        }
        let name = self.localised(adjuster.key.as_bytes(), &[], scopes, false)?;
        Ok(format!(
            "\u{11}H{name}\u{11}!\n{}",
            indent_nonempty_lines(&lines, 3)
        ))
    }

    fn units_bonus_body(
        &mut self,
        name: &str,
        items: &[Item],
        category: bool,
        units: &content_reader_script::sub_units::SubUnits,
        scopes: Scopes,
    ) -> Result<String, String> {
        let icon = if category {
            format!("\u{13}GFX_{name}_icon_small ")
        } else {
            let frame = if self.context.text.nato_counters() {
                2
            } else {
                1
            };
            format!("\u{13}GFX_{name}_icon_small|{frame} ")
        };
        let display_name = self.units_current_name(name.as_bytes(), scopes)?;
        let mut text = self.localised(
            b"SUBUNIT_BONUS_HEADER",
            &[
                ("ICON", Argument::Text(&icon)),
                ("NAME", Argument::Text(&display_name)),
            ],
            scopes,
            false,
        )?;
        newline(&mut text);
        if !category {
            let width = fixed_field(items, "combat_width", 0);
            if width != 0 {
                text.push_str(&self.localised(b"STAT_ARMY_COMBAT_WIDTH", &[], scopes, false)?);
                text.push_str(&self.localised(
                    b"STAT_ARMY_COMBAT_WIDTH_DIFF",
                    &[("VALUE", Argument::Fixed5(width))],
                    scopes,
                    false,
                )?);
                text.push('\n');
            }
        }
        for (index, value) in stats(items)
            .into_iter()
            .enumerate()
            .filter(|(_, value)| *value != 0)
        {
            text.push_str(if category { "  " } else { "   " });
            text.push_str(&self.units_bonus_stat(index, value, scopes)?);
        }
        if category {
            return Ok(text);
        }
        for key in ["night", "amphibious", "river", "fort", "snow"] {
            match self.units_bonus_adjuster(&adjuster(key, items), scopes) {
                Ok(body) => text.push_str(&body),
                Err(why) => self.refuse(&format!("unit bonus {name} {key}"), why),
            }
        }
        match self.units_bonus_terrain(items, scopes) {
            Ok(body) => text.push_str(&body),
            Err(why) => self.refuse(&format!("unit bonus {name} terrain"), why),
        }
        match self.units_bonus_multipliers(items, units, scopes) {
            Ok(body) => text.push_str(&body),
            Err(why) => self.refuse(&format!("unit bonus {name} battalion_mult"), why),
        }
        Ok(text)
    }

    fn units_bonus_terrain(&self, items: &[Item], scopes: Scopes) -> Result<String, String> {
        let mut text = String::new();
        if items.iter().any(|item| {
            item.value.items().is_some()
                && ![
                    "night",
                    "amphibious",
                    "river",
                    "fort",
                    "snow",
                    "need",
                    "need_equipment",
                    "battalion_mult",
                ]
                .iter()
                .any(|key| item.keyword_is(key))
        }) {
            let terrain = self.context.text.content().environment()?.terrain()?;
            let mut seen = BTreeSet::new();
            for terrain in &terrain.categories {
                if !seen.insert(Token::c_str(&terrain.name)) {
                    continue;
                }
                let key = parser::text(&terrain.name).map_err(|why| why.to_string())?;
                text.push_str(&self.units_bonus_adjuster(&adjuster(key, items), scopes)?);
            }
        }
        Ok(text)
    }

    fn units_bonus_multipliers(
        &self,
        items: &[Item],
        units: &content_reader_script::sub_units::SubUnits,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut text = String::new();
        let mut previous = None;
        for block in items
            .iter()
            .filter(|item| item.keyword_is("battalion_mult"))
        {
            let mut category = None;
            let mut percentage = false;
            let mut add = false;
            let mut values = Vec::new();
            for item in children(block) {
                if item.keyword_is("category") {
                    let token = Token::c_str(item.value.scalar().unwrap_or_default());
                    if let Some(found) = units.category_name(&token)? {
                        category = Some(found);
                    }
                } else if item.keyword_is("display_as_percentage") || item.keyword_is("add") {
                    let target = if item.keyword_is("add") {
                        &mut add
                    } else {
                        &mut percentage
                    };
                    if let Some(value) = parser::read_bool(item.value.scalar().unwrap_or_default())
                    {
                        *target = value;
                    }
                } else if let Some(index) = item
                    .keyword()
                    .and_then(|key| content_reader_script::equipment_stats::stat_index(&key.folded()))
                    && let Some(value) = content_reader_script::modifiers::read_fixed5(item)
                    && value != 0
                {
                    values.push((index, value));
                }
            }
            let category = category.ok_or("battalion_mult has no resolved sub-unit category")?;
            if previous != Some(category) {
                let name = self.units_current_name(category.as_bytes(), scopes)?;
                text.push_str(&self.localised(
                    b"SUBUNIT_BONUS_CATEGORY_ADD",
                    &[("CATEGORY", Argument::Text(&name))],
                    scopes,
                    false,
                )?);
                newline(&mut text);
                previous = Some(category);
            }
            for (index, value) in values {
                let (_, label, positive) = stat_metadata()[index];
                let key: &[u8] = match (positive, percentage && !add) {
                    (true, true) => b"SUBUNIT_CATEGORY_ADD_STAT",
                    (true, false) => b"SUBUNIT_CATEGORY_MULT_STAT",
                    (false, true) => b"SUBUNIT_CATEGORY_NEG_ADD_STAT",
                    (false, false) => b"SUBUNIT_CATEGORY_NEG_MULT_STAT",
                };
                let name = self.localised(label.as_bytes(), &[], scopes, false)?;
                let line = self.localised(
                    key,
                    &[
                        ("NAME", Argument::Text(&name)),
                        ("VALUE", Argument::Fixed5(value)),
                    ],
                    scopes,
                    false,
                )?;
                text.push_str(&indent_nonempty_lines(&line, 4));
                newline(&mut text);
            }
        }
        Ok(text)
    }
}
