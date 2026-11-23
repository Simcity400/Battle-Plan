use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub(super) enum NumericFormat {
    Fixed5,
    Skill,
    Rounded,
    Truncated,
    Size,
    Xp,
    Threat,
    Garrison,
}

impl Renderer<'_, '_, '_> {
    pub(super) fn numeric_value(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
    ) -> Result<String, String> {
        self.numeric_leaf(item, scopes, key, NumericFormat::Fixed5)
    }

    pub(super) fn numeric_leaf(
        &self,
        item: &Item,
        scopes: Scopes,
        key: &[u8],
        format: NumericFormat,
    ) -> Result<String, String> {
        let value = if matches!(format, NumericFormat::Xp | NumericFormat::Threat) {
            i64::from(self.numeric_int_operand(item, scopes)?) * 100_000
        } else if scope_selector(item).is_some() {
            0
        } else {
            self.scoped_value(item.value.scalar().unwrap_or(b"0"), scopes)?
        };
        let key = match format {
            NumericFormat::Threat if value / 100_000 < 1 => b"EFFECT_REMOVE_THREAT".as_slice(),
            NumericFormat::Size if (value / 100_000) as i32 > 1 => {
                b"EFFECT_MIO_ADD_SIZE_PLURAL".as_slice()
            }
            _ => key,
        };
        let (argument, value, expand, newline) = match format {
            NumericFormat::Fixed5 => ("VAL", Argument::Fixed5(value), true, true),
            NumericFormat::Skill => (
                "NUM",
                Argument::Int32(crate::triggers::fixed_round(value) as i32),
                false,
                true,
            ),
            NumericFormat::Rounded => (
                "VAL",
                Argument::Int32(crate::triggers::fixed_round(value) as i32),
                true,
                true,
            ),
            NumericFormat::Truncated | NumericFormat::Size => {
                ("VAL", Argument::Int32((value / 100_000) as i32), true, true)
            }
            NumericFormat::Xp => (
                "VALUE",
                Argument::Int32(((value / 100_000) as i32).max(0)),
                false,
                false,
            ),
            NumericFormat::Threat => (
                "THREAT",
                Argument::Int32(((value / 100_000) as i32).wrapping_abs()),
                true,
                true,
            ),
            NumericFormat::Garrison => {
                let scaled = value.wrapping_mul(100_000) / 100_000;
                let colour = if scaled > 0 {
                    'G'
                } else if scaled < 0 {
                    'R'
                } else {
                    'Y'
                };
                let number = content_reader_parser::fixed5::fixed5_to_string(scaled, 1);
                let value = format!(
                    "\u{11}{colour}{}{number}%\u{11}!",
                    if scaled < 0 { "" } else { "+" }
                );
                return self.char_native_line(
                    key,
                    &[("VALUE", Argument::Text(&value))],
                    scopes,
                    false,
                );
            }
        };
        let mut text = self.char_native_text(key, &[(argument, value)], scopes, expand)?;
        if newline {
            text.push('\n');
        }
        Ok(text)
    }

    fn numeric_int_operand(&self, item: &Item, scopes: Scopes) -> Result<i32, String> {
        if scope_selector(item).is_some() {
            return Ok(0);
        }
        let operand = scalar_operand(item)?;
        match content_reader_parser::fixed5::int_operand(operand, item.quoted()) {
            content_reader_parser::fixed5::IntOperand::Integer(value) => Ok(value),
            content_reader_parser::fixed5::IntOperand::Constant(value) => Ok((value / 100_000) as i32),
            content_reader_parser::fixed5::IntOperand::Scoped => {
                Ok((self.scoped_value(operand, scopes)? / 100_000) as i32)
            }
            _ => Err("integer effect operand was not stored".into()),
        }
    }

    fn numeric_field(&self, item: &Item, name: &str, scopes: Scopes) -> Result<i64, String> {
        field(children(item), name)
            .map(|field| self.scoped_value(scalar_operand(field)?, scopes))
            .unwrap_or(Ok(0))
    }

    fn numeric_marked_country(&self, country: u32) -> Result<String, String> {
        let name = content_reader_localisation::localisation::crt_case_text(
            &self.country_name(country, true)?,
            true,
            true,
        );
        Ok(format!("\u{12}{} {name}", self.tag(country)?))
    }

    pub(super) fn numeric_collaboration(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(features) = self.features(DLC_MASK)? else {
            return Err("simulation features lacks the collaboration feature mask".into());
        };
        if features.dlc_mask & (1 << 5) == 0 {
            return Ok(String::new());
        }
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)
            .map_err(|why| why.to_string())?
            .filter(|target| *target > 0)
        else {
            return Ok(String::new());
        };
        let value = self.numeric_field(item, "value", scopes)?;
        if value == 0 {
            return Ok(String::new());
        }
        let country = self.country_name(target, true)?;
        self.char_native_text(
            if value > 0 {
                b"EFFECT_ADD_COLLABORATION"
            } else {
                b"EFFECT_REMOVE_COLLABORATION"
            },
            &[
                ("TARGET", Argument::Text(&country)),
                ("VALUE", Argument::Fixed5(value.wrapping_abs())),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn numeric_decryption(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)
            .map_err(|why| why.to_string())?
            .filter(|target| *target > 0)
        else {
            return Ok(String::new());
        };
        if matches!(scopes.this, Scope::Country(source) | Scope::CountryLeader { country: source, .. } if source == target)
        {
            return Ok(String::new());
        }
        let country = self.numeric_marked_country(target)?;
        if field(children(item), "ratio").is_some() {
            let value = self.numeric_field(item, "ratio", scopes)?;
            self.char_native_line(
                b"EFFECT_ADD_DECRYPTION_RATIO",
                &[
                    ("COUNTRY", Argument::Text(&country)),
                    ("RATIO", Argument::Fixed5(value)),
                ],
                scopes,
                true,
            )
        } else {
            let value =
                crate::triggers::fixed_round(self.numeric_field(item, "amount", scopes)?) as i32;
            self.char_native_line(
                b"EFFECT_ADD_DECRYPTION",
                &[
                    ("COUNTRY", Argument::Text(&country)),
                    ("AMOUNT", Argument::Int32(value)),
                ],
                scopes,
                true,
            )
        }
    }

    pub(super) fn numeric_intel(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let (Scope::Country(source)
        | Scope::CountryLeader {
            country: source, ..
        }) = scopes.this
        else {
            return Ok(String::new());
        };
        if source == 0
            || !self
                .context
                .world()?
                .exists(source)
                .map_err(|why| why.to_string())?
        {
            return Ok(String::new());
        }
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)
            .map_err(|why| why.to_string())?
            .filter(|target| *target > 0)
        else {
            return Ok(String::new());
        };
        if !self
            .context
            .world()?
            .exists(target)
            .map_err(|why| why.to_string())?
        {
            return Ok(String::new());
        }
        let mut values = Vec::new();
        for (name, key) in [
            (
                "civilian_intel",
                b"MODIFIER_CIVILIAN_INTEL_FACTOR".as_slice(),
            ),
            ("army_intel", b"MODIFIER_ARMY_INTEL_FACTOR".as_slice()),
            ("navy_intel", b"MODIFIER_NAVY_INTEL_FACTOR".as_slice()),
            (
                "airforce_intel",
                b"MODIFIER_AIRFORCE_INTEL_FACTOR".as_slice(),
            ),
        ] {
            let value = self.numeric_field(item, name, scopes)?;
            if value != 0 {
                values.push(format!(
                    "\n{}: {}",
                    self.char_native_text(key, &[], scopes, false)?,
                    content_reader_parser::fixed5::fixed5_percent(value, -1, false)
                ));
            }
        }
        if values.is_empty() {
            return Ok(String::new());
        }
        let intel = values.join(", ");
        self.char_native_text(
            b"EFFECT_ADD_INTEL",
            &[
                ("SOURCE", Argument::Text(&self.country_name(source, true)?)),
                ("TARGET", Argument::Text(&self.country_name(target, true)?)),
                ("INTEL", Argument::Text(&intel)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn numeric_mines(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if !matches!(scopes.this, Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0)
        {
            return Ok(String::new());
        }
        let region =
            crate::triggers::fixed_round(self.numeric_field(item, "region", scopes)?) as i32;
        if region < 1 {
            return Ok(String::new());
        }
        Err(format!(
            "strategic region {region} needs its unrecorded instance name"
        ))
    }

    pub(super) fn numeric_truce(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let hour = self.tables().hour();
        let day = hour.wrapping_sub(hour.wrapping_sub(game_model::calendar::HOUR_EPOCH) % 24);
        let days = crate::triggers::fixed_round(self.numeric_field(item, "days", scopes)?) as i32;
        let (year, month, date) = numeric_date_parts(day.wrapping_add(days.wrapping_mul(24)));
        let month = self.char_native_text(month.as_bytes(), &[], scopes, false)?;
        let date = format!("{date} {month}, {year}");
        self.char_native_text(
            b"EFFECT_SET_TRUCE",
            &[
                ("COUNTRY", Argument::Text(&self.country_name(target, true)?)),
                ("DATE", Argument::Text(&date)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn numeric_import(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let resource = field(fields, "resource")
            .and_then(|item| item.value.read_string())
            .unwrap_or_default();
        let resource = content_reader_parser::parser::text(resource).map_err(|why| why.to_string())?;
        let resources = self
            .context
            .content
            .resources
            .ok_or_else(|| self.content_gap("resources"))?;
        let mut matched = resources
            .iter()
            .filter(|row| row.token.eq_ignore_ascii_case(resource));
        let definition = matched.next().ok_or_else(|| format!("create_import resource {resource} needs the null resource signed icon_frame, which content does not preserve"))?;
        if matched.any(|row| row.icon_frame != definition.icon_frame) {
            return Err(format!(
                "create_import resource {resource} repeated native database record identity is unverified"
            ));
        }
        let icon = definition.icon_frame.filter(|frame| *frame <= i32::MAX as u32).ok_or_else(|| format!("create_import resource {resource} signed/default icon_frame is not preserved by content_reader_key_names::resources"))?;
        let amount = fields
            .iter()
            .filter(|item| item.keyword_is("amount"))
            .filter_map(
                |item| match content_reader_parser::crt::scan_i32(item.value.scalar()?) {
                    content_reader_parser::crt::Scanned::Stored(value) => Some(value),
                    _ => None,
                },
            )
            .next_back()
            .unwrap_or(0);
        let resource = format!("{amount}\u{13}resources_strip|{icon} ");
        let source = self.current_country(scopes)?;
        let target = field(fields, "exporter").ok_or("create_import exporter is absent")?;
        let target = self.country_target(target, scopes)?.unwrap_or(0);
        let source = self.country_name(source, false)?;
        let target = self.country_name(target, false)?;
        let mut args = vec![
            ("RESOURCE", Argument::Text(&resource)),
            ("IMPORTER", Argument::Text(&source)),
            ("EXPORTER", Argument::Text(&target)),
        ];
        let mut date = String::new();
        if let Some(start) = field(fields, "start_date") {
            let value = content_reader_parser::parser::text(start.value.read_string().unwrap_or_default())
                .map_err(|why| why.to_string())?;
            let value = if value.split('.').count() == 3 {
                format!("{value}.1")
            } else {
                value.to_owned()
            };
            let stored = content_reader_save::SavedDate::parse(value.as_bytes()).map_err(|why| {
                format!("create_import date requires the game-date parser: {why}")
            })?;
            if stored != content_reader_save::SavedDate::Invalid {
                let (year, _, day) =
                    numeric_date_parts(stored.engine_hour().map_err(|why| why.to_string())?);
                let content_reader_save::SavedDate::Calendar(calendar) = stored else {
                    unreachable!()
                };
                date = format!("{year}.{}.{day}", calendar.month);
                args.push(("DATE", Argument::Text(&date)));
            }
        }
        self.char_native_text(
            if date.is_empty() {
                b"EFFECT_CREATE_IMPORT"
            } else {
                b"EFFECT_CREATE_IMPORT_HAS_DATA"
            },
            &args,
            scopes,
            true,
        )
    }

    pub(super) fn numeric_license(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let equipment = self.units_equipment_reference_name(children(item), scopes)?;
        let target = if field(children(item), "target").is_none() {
            match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => {
                    Some(country)
                }
                _ => None,
            }
        } else {
            self.country_target(item, scopes)?
        };
        let target = target.unwrap_or(0);
        self.char_native_line(
            b"EFFECT_CREATE_LICENSE_PRODUCTION",
            &[
                ("TARGET", Argument::Text(&self.country_name(target, true)?)),
                ("NAME", Argument::Text(&equipment)),
            ],
            scopes,
            true,
        )
    }

    pub(super) fn numeric_launch_nuke(
        &self,
        _item: &Item,
        _scopes: Scopes,
    ) -> Result<String, String> {
        Ok(String::new())
    }
}

pub(super) fn numeric_date_parts(hour: i32) -> (i32, &'static str, i32) {
    let hours = hour.wrapping_sub(game_model::calendar::HOUR_EPOCH);
    let mut day = hours / 24 % 365;
    let mut month = 0;
    while month < 12 && day >= game_model::calendar::MONTH_LENGTHS[month] {
        day -= game_model::calendar::MONTH_LENGTHS[month];
        month += 1;
    }
    let name = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(month)
    .copied()
    .unwrap_or_default();
    (hours / 8760, name, day + 1)
}
