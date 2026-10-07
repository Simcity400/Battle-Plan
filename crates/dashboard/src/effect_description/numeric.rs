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

    // Shared 532-byte vf7 template: Evaluate@1405382B0, substitution@142230F20,
    // Expand@1410CF560, LF. Other forms retain their native integer conversion.
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
            0 // ParseScopeSelector@14137DCD0 leaves CScopedVariable +58 at zero.
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
            ), // Skill vf7@1402E9B70..1402EAC20; round@1424D8760.
            NumericFormat::Rounded => (
                "VAL",
                Argument::Int32(crate::triggers::fixed_round(value) as i32),
                true,
                true,
            ), // CAddNukeEffect@140375150.
            NumericFormat::Truncated | NumericFormat::Size => {
                ("VAL", Argument::Int32((value / 100_000) as i32), true, true)
            } // CIC@14048E850, task capacity@1402FD1E0, size@1402FCCF0.
            NumericFormat::Xp => (
                "VALUE",
                Argument::Int32(((value / 100_000) as i32).max(0)),
                false,
                false,
            ), // CGainXpEffect@1402EC4D0 / vf26@1402EF010.
            NumericFormat::Threat => (
                "THREAT",
                Argument::Int32(((value / 100_000) as i32).wrapping_abs()),
                true,
                true,
            ), // CAddThreatEffect@1403785F0, zero takes REMOVE.
            NumericFormat::Garrison => {
                // FormatColoredFixed5Value@142257CE0: scale 1, precision 1,
                // positive-good, '%' suffix and '+' even for zero; no expansion.
                let scaled = value.wrapping_mul(100_000) / 100_000;
                let colour = if scaled > 0 {
                    'G'
                } else if scaled < 0 {
                    'R'
                } else {
                    'Y'
                };
                let number = installed_content::fixed5::fixed5_to_string(scaled, 1);
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
        // CIntEffect::vf6@14137DD70 / vf26@141384870: literal %i or
        // signed fixed5 division, narrowed to i32. Shares the existing parser.
        match installed_content::fixed5::int_operand(operand, item.quoted()) {
            installed_content::fixed5::IntOperand::Integer(value) => Ok(value),
            installed_content::fixed5::IntOperand::Constant(value) => Ok((value / 100_000) as i32),
            installed_content::fixed5::IntOperand::Scoped => {
                Ok((self.scoped_value(operand, scopes)? / 100_000) as i32)
            }
            _ => Err("CIntEffect operand was not stored (14137DD70)".into()),
        }
    }

    fn numeric_field(&self, item: &Item, name: &str, scopes: Scopes) -> Result<i64, String> {
        field(children(item), name)
            .map(|field| self.scoped_value(scalar_operand(field)?, scopes))
            .unwrap_or(Ok(0))
    }

    // CountryTag_FormatMarkedName@140BA59E0 -> capitalized +50@1406E63A0.
    fn numeric_marked_country(&self, country: u32) -> Result<String, String> {
        let name =
            installed_content::localisation::crt_case_text(&self.country_name(country, true)?, true, true);
        Ok(format!("\u{12}{} {name}", self.tag(country)?))
    }

    pub(super) fn numeric_collaboration(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let [features] = self
            .context
            .text
            .frame()
            .complete::<tables::SimulationFeatures>()
            .map_err(|why| why.to_string())?
        else {
            return Err(
                "SimulationFeatures lacks the collaboration feature mask (1401AE640 case 0x1B)"
                    .into(),
            );
        };
        if features.dlc_mask & (1 << 5) == 0 {
            return Ok(String::new());
        }
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)?
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
        ) // 140371C60: feature gate, nonzero value, no LF.
    }

    pub(super) fn numeric_decryption(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)?
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
        } // 140373590: ratio's assigned flag +130 wins over amount; LF.
    }

    pub(super) fn numeric_intel(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let (Scope::Country(source)
        | Scope::CountryLeader {
            country: source, ..
        }) = scopes.this
        else {
            return Ok(String::new());
        };
        if source == 0 || !self.context.world()?.exists(source)? {
            return Ok(String::new());
        }
        let Some(target) = field(children(item), "target") else {
            return Ok(String::new());
        };
        let Some(target) = self
            .evaluator()?
            .scoped_country(scalar_operand(target)?, scopes)?
            .filter(|target| *target > 0)
        else {
            return Ok(String::new());
        };
        if !self.context.world()?.exists(target)? {
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
                    installed_content::fixed5::fixed5_percent(value, -1, false)
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
        ) // 140373D80 ->141409250 /141085460 /141085540; no LF.
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
            "strategic region {region} needs unrecorded CStrategicRegion instance name +0x38/+0x48 (140374410 ->140E0EB10; writers 141B50C80/141B4F0B0)"
        ))
    }

    pub(super) fn numeric_truce(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let hour = self.context.text.frame().hour;
        let day = hour.wrapping_sub(hour.wrapping_sub(engine::cdate::HOUR_EPOCH) % 24);
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
        // 1403941B0: start of current day + rounded days; FormatDayMonthYear@140526FD0.
    }

    pub(super) fn numeric_import(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let resource = field(fields, "resource")
            .and_then(|item| item.value.read_string())
            .unwrap_or_default();
        let resource = installed_content::parser::text(resource).map_err(|why| why.to_string())?;
        let resources = self
            .context
            .content
            .resources
            .ok_or_else(|| self.content_gap("resources"))?;
        let mut matched = resources
            .iter()
            .filter(|row| row.token.eq_ignore_ascii_case(resource));
        let definition = matched.next().ok_or_else(|| format!("create_import resource {resource} needs the native null resource signed icon_frame +0xDC (14037C2F0/140BBB090), not preserved by content"))?;
        if matched.any(|row| row.icon_frame != definition.icon_frame) {
            return Err(format!(
                "create_import resource {resource} repeated native database record identity is unverified (140AB2DF0)"
            ));
        }
        let icon = definition.icon_frame.filter(|frame| *frame <= i32::MAX as u32).ok_or_else(|| format!("create_import resource {resource} signed/default icon_frame +0xDC is not preserved by installed_content::resources (14037C2F0/140BBB090)"))?;
        let amount = fields
            .iter()
            .filter(|item| item.keyword_is("amount"))
            .filter_map(
                |item| match installed_content::crt::scan_i32(item.value.scalar()?) {
                    installed_content::crt::Scanned::Stored(value) => Some(value),
                    _ => None,
                },
            )
            .next_back()
            .unwrap_or(0); // ReadScriptSignedInt32@1424AB970 retains failed writes.
        let resource = format!("{amount}\u{13}resources_strip|{icon} "); // Literal14275B798.
        let source = self.current_country(scopes)?;
        let target =
            field(fields, "exporter").ok_or("create_import exporter is absent (140BA5D10)")?;
        let target = self.country_target(target, scopes)?.unwrap_or(0); // 140BA5960.
        let source = self.country_name(source, false)?;
        let target = self.country_name(target, false)?;
        let mut args = vec![
            ("RESOURCE", Argument::Text(&resource)),
            ("IMPORTER", Argument::Text(&source)),
            ("EXPORTER", Argument::Text(&target)),
        ];
        let mut date = String::new();
        if let Some(start) = field(fields, "start_date") {
            let value = installed_content::parser::text(start.value.read_string().unwrap_or_default())
                .map_err(|why| why.to_string())?;
            let value = if value.split('.').count() == 3 {
                format!("{value}.1")
            } else {
                value.to_owned()
            };
            let stored = installed_content::save::SavedDate::parse(value.as_bytes()).map_err(|why| {
                format!("create_import date requires CGameDate parser (140527450): {why}")
            })?;
            if stored != installed_content::save::SavedDate::Invalid {
                let (year, _, day) =
                    numeric_date_parts(stored.engine_hour().map_err(|why| why.to_string())?);
                let installed_content::save::SavedDate::Calendar(calendar) = stored else {
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
        // 14037C2F0 /1403B1260: factories have no description operand, no LF.
    }

    pub(super) fn numeric_license(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let equipment = self.units_equipment_reference_name(children(item), scopes)?; //141411150.
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
        let target = target.unwrap_or(0); // 14049C740 -> 140BA56A0.
        self.char_native_line(
            b"EFFECT_CREATE_LICENSE_PRODUCTION",
            &[
                ("TARGET", Argument::Text(&self.country_name(target, true)?)),
                ("NAME", Argument::Text(&equipment)),
            ],
            scopes,
            true,
        )
        // 14049C740: priority/cost change execution only, explicit LF.
    }

    pub(super) fn numeric_launch_nuke(
        &self,
        _item: &Item,
        _scopes: Scopes,
    ) -> Result<String, String> {
        // 140383630 constructs and destroys the formatted state text, then
        // unconditionally clears RSI at 140383B03..140383B18 (assembly).
        Ok(String::new())
    }
}

// GetMonth@140177140 / GetDay@140176DB0, FormatDayMonthYear@140526FD0.
pub(super) fn numeric_date_parts(hour: i32) -> (i32, &'static str, i32) {
    let hours = hour.wrapping_sub(engine::cdate::HOUR_EPOCH);
    let mut day = hours / 24 % 365;
    let mut month = 0;
    while month < 12 && day >= engine::cdate::MONTH_LENGTHS[month] {
        day -= engine::cdate::MONTH_LENGTHS[month];
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
