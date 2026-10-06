use super::*;
use installed_content::parser;
use replay::rows::{VariantDefinitionRow, tables};
use std::collections::BTreeSet;

#[cfg(test)]
mod tests;

fn string_field<'a>(items: &'a [Item], name: &str) -> Result<&'a str, String> {
    parser::text(
        field(items, name)
            .and_then(|item| item.value.read_string())
            .unwrap_or_default(),
    )
    .map_err(|why| why.to_string())
}

fn integer_field(items: &[Item], name: &str, default: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.keyword_is(name))
        .filter_map(|item| match installed_content::crt::scan_i32(item.value.scalar()?) {
            installed_content::crt::Scanned::Stored(value) => Some(value),
            _ => None,
        })
        .next_back()
        .unwrap_or(default) // ReadScriptSignedInt32@1424AB970.
}

struct EquipmentTexts<'a>(&'a Localisation);

impl crate::products::equipment::Texts for EquipmentTexts<'_> {
    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }

    fn text(&self, key: &str) -> Option<String> {
        self.0.localise_with(key.as_bytes(), &[]).ok()
    }
}

pub(super) enum TemplateDescription {
    Delete,
    Lock,
    ClearCap,
    Cap,
    Recruit,
}

impl Renderer<'_, '_, '_> {
    fn units_equipment(&self) -> Result<&installed_content::equipment::Database, String> {
        self.context
            .content
            .equipment
            .ok_or_else(|| self.content_gap("equipment"))
    }

    fn units_equipment_type(
        &self,
        fields: &[Item],
        names: &[&str],
        scopes: Scopes,
    ) -> Result<Option<usize>, String> {
        let mut fields = fields
            .iter()
            .filter(|field| names.iter().any(|name| field.keyword_is(name)))
            .peekable();
        if fields.peek().is_none() {
            return Ok(None);
        }
        let database = self.units_equipment()?;
        let mut cached = None;
        let mut input = b"".as_slice();
        let mut queued = false;
        for field in fields {
            let value = parser::c_string(field.value.scalar().ok_or(
                "equipment-type block operand is unsupported by the direct token reader (14139B910)",
            )?);
            if !installed_content::tokens::Token::c_str(value).is_rejected() {
                cached = database.get(value);
            }
            if cached.is_none() {
                input = value;
                queued |= input.starts_with(b"var:");
            }
        }
        if queued {
            cached = database.get(input); // Pending vf1 14139B510 replaces the earlier cache.
        }
        if cached.is_none() && queued {
            let token = (self.scoped_value(input, scopes)? / 100_000) as u32;
            if token == 0 {
                return Ok(None);
            }
            let literal = self.context.world()?.token(token)?.ok_or_else(|| {
                format!("LexicalTokens lacks equipment variable token {token} (14139B350)")
            })?;
            return Ok(database.get(literal));
        }
        Ok(cached) // ReadFromToken 14139B910; Resolve 14139B350 uses signed division.
    }

    fn units_country_equipment_name(
        &self,
        name: &[u8],
        tag: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = parser::text(parser::c_string(name)).map_err(|why| why.to_string())?;
        let prefixed = format!("{tag}_{name}");
        self.localised(
            if self.context.localisation.get(&prefixed).is_some() {
                prefixed.as_bytes()
            } else {
                name.as_bytes()
            },
            &[],
            scopes,
            false,
        ) // LocalizeForCountry 140BBA780.
    }

    /// CEquipmentReference::GetName (141411150), shared by production/licences.
    pub(super) fn units_equipment_reference_name(
        &self,
        fields: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut version = "";
        let mut kind = b"".as_slice();
        let mut creator = b"---".as_slice(); // Country-database slot0, 14070E670/1427CDA44.
        // 141411210 mutates the same reference across repeated equipment bodies.
        for equipment in fields
            .iter()
            .filter(|field| field.keyword_is("equipment"))
        {
            if let Some(name) = field(children(equipment), "version_name") {
                version = parser::text(name.value.read_string().unwrap_or_default())
                    .map_err(|why| why.to_string())?;
            }
            if let Some(value) = field(children(equipment), "type") {
                kind = value.value.read_string().unwrap_or_default();
            }
            if let Some(value) = field(children(equipment), "creator") {
                creator = value.value.read_string().unwrap_or_default();
            }
        }
        let name = if version.is_empty() {
            let database = self.units_equipment()?;
            let name = database
                .named(kind)
                .map_or(b"".as_slice(), |kind| kind.name.as_slice());
            let creator = parser::text(parser::c_string(creator)).map_err(|why| why.to_string())?;
            let defined = creator == "---"
                || self.context.text.recorded().country(creator).is_some()
                || self
                    .context
                    .content
                    .key_names
                    .ok_or_else(|| self.content_gap("effect key names"))?
                    .countries
                    .contains(creator.as_bytes());
            let creator = if defined {
                creator
            } else {
                "---" // 140BA6310/1401DA3F0: exact raw tag spelling, no ROOT/FROM selector.
            };
            self.units_country_equipment_name(name, creator, scopes)?
        } else {
            version.to_owned()
        };
        Ok(name)
    }

    pub(super) fn units_equipment_production(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = self.units_equipment_reference_name(children(item), scopes)?;
        self.localised(
            b"EFFECT_ADD_EQUIPMENT_PRODUCTION",
            &[("EQUIPMENT", Argument::Text(&name))],
            scopes,
            true,
        ) // 14048EA80 -> 141411150: verbatim version name, no LF.
    }

    pub(super) fn units_destroy_ships(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let Some(index) = self.units_equipment_type(fields, &["type"], scopes)? else {
            return Ok(String::new());
        };
        let count = if field(fields, "count")
            .is_some_and(|field| field.value_keyword().as_deref() == Some("all"))
        {
            -1
        } else {
            field(fields, "count")
                .and_then(|field| field.value.scalar())
                .map(installed_content::fixed5::decimal_i32)
                .unwrap_or(0)
        };
        let count_text = if count < 0 {
            self.localised(b"EFFECT_DESTROY_SHIPS_ALL", &[], scopes, true)?
        } else {
            count.to_string()
        };
        let name = self.units_current_name(&self.units_equipment()?.equipment[index].name, scopes)?;
        self.line(
            if count == 1 {
                b"EFFECT_DESTROY_SHIP"
            } else {
                b"EFFECT_DESTROY_SHIPS"
            },
            &[
                ("COUNT", Argument::Text(&count_text)),
                ("TYPE", Argument::Text(&name)),
            ],
            scopes,
            true,
        ) // 14037F150; count is ParseTokenInt32 1424B0080, never scoped.
    }

    pub(super) fn units_send_equipment(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let Some(index) = self.units_equipment_type(fields, &["type", "equipment"], scopes)? else {
            return Ok(String::new());
        };
        let country = self.country_name(self.country_target(item, scopes)?.unwrap_or(0), false)?;
        let name = self.units_current_name(&self.units_equipment()?.equipment[index].name, scopes)?;
        let amount = match field(fields, "amount") {
            Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
            None => 0,
        };
        self.line(
            if parser::read_bool_field(fields, "new_prioritised", false) {
                b"EFFECT_SEND_EQUIPMENT_NEW"
            } else {
                b"EFFECT_SEND_EQUIPMENT_OLD"
            },
            &[
                (
                    "AMOUNT",
                    Argument::Int32(crate::triggers::fixed_round(amount) as i32),
                ),
                ("EQUIPMENT_NAME", Argument::Text(&name)),
                ("COUNTRY", Argument::Text(&country)),
            ],
            scopes,
            true,
        ) // 14049CAA0 ignores old_prioritised; aliases share one operand.
    }

    fn units_variant_name(&self, variant: &VariantDefinitionRow) -> Result<String, String> {
        let frame = self.context.text.frame();
        let empty = crate::products::equipment::Tags::new(&[], &[], None);
        let raw = if variant.display_name.is_empty() {
            let tags = crate::products::equipment::Tags::new(
                frame
                    .complete::<tables::Countries>()
                    .map_err(|why| why.to_string())?,
                frame
                    .complete::<tables::CountryLinks>()
                    .map_err(|why| why.to_string())?,
                if (variant.origin_link as i32) >= 1 {
                    None
                } else {
                    Some(
                        frame
                            .complete::<tables::CountryAppearance>()
                            .map_err(|why| why.to_string())?,
                    )
                },
            );
            let raw = crate::products::equipment::raw_variant(
                variant,
                frame.hour,
                &tags,
                frame
                    .complete::<tables::LexicalTokens>()
                    .map_err(|why| why.to_string())?,
            );
            if raw.name_tag.is_none() {
                return Err("variant origin/creator naming tag is unavailable (140BCB030)".into());
            }
            raw
        } else {
            crate::products::equipment::raw_variant(variant, frame.hour, &empty, &[])
        };
        crate::products::equipment::variant_names(
            &raw,
            Some(&EquipmentTexts(self.context.localisation)),
        )
        .0
        .ok_or_else(|| {
            "shared equipment variant-name formatter cannot represent the name (140BCB030)".into()
        })
    }

    fn units_available_variants(
        &self,
        country: u32,
    ) -> Result<Vec<Option<&VariantDefinitionRow>>, String> {
        let world = self.context.world()?;
        if world.raw_country(country).is_none() {
            return Err(format!(
                "Countries lacks equipment roster country slot {country} (140E535D0/140E53740)"
            ));
        }
        let frame = self.context.text.frame();
        let mut roster = frame
            .complete::<tables::AvailableEquipmentVariants>()
            .map_err(|why| why.to_string())?
            .iter()
            .filter(|row| row.roster_kind == 0 && world.slot(row.country) == Some(country))
            .collect::<Vec<_>>();
        roster.sort_by_key(|row| row.ordinal);
        if roster.is_empty() {
            return Ok(Vec::new());
        }
        let mut definitions = BTreeMap::new();
        for variant in frame
            .complete::<tables::VariantDefinitions>()
            .map_err(|why| why.to_string())?
        {
            definitions.entry(variant.id).or_insert(variant);
        }
        Ok(roster
            .into_iter()
            .map(|row| definitions.get(&row.variant).copied())
            .collect()) // 140E535D0/140E53740: primary roster in native order.
    }

    pub(super) fn units_stockpile(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let amount = match field(fields, "amount") {
            Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
            None => 0,
        };
        let amount = crate::triggers::fixed_round(amount) as i32;
        let producer = match field(fields, "producer") {
            Some(value) => self.evaluator()?.scoped_country(scalar_operand(value)?, scopes)?,
            None => None,
        };
        let Some(index) = self.units_equipment_type(fields, &["type"], scopes)? else {
            return Ok(String::new());
        };
        let database = self.units_equipment()?;
        let kind = &database.equipment[index];
        let name = if kind.is_archetype {
            self.units_current_name(&kind.name, scopes)?
        } else {
            let country = match producer {
                Some(country) => country,
                None => match scopes.this {
                    Scope::Country(country) if (country as i32) > 0 => country,
                    _ => {
                        return Err("native direct country0 production roster is unavailable for concrete equipment without a positive producer (14049B6D0 -> 140BA5140, CCountry+0xF68); a state scope does not select its owner".into());
                    }
                },
            };
            let world = self.context.world()?;
            let mut selected = None;
            for variant in self.units_available_variants(country)? {
                let variant = variant.ok_or(
                    "AvailableEquipmentVariants references an absent VariantDefinitions row",
                )?;
                let token = world
                    .token(variant.equipment_token)?
                    .ok_or("LexicalTokens lacks the available variant equipment token")?;
                if database.get(token) == Some(index) {
                    selected = Some(variant);
                    break;
                }
            }
            let Some(variant) = selected else {
                return Ok(String::new());
            };
            let mut ship = false;
            let mut chain = BTreeSet::new();
            let mut at = index;
            loop {
                if !chain.insert(at) {
                    return Err("equipment ship-classifier archetype cycle (140C88440)".into());
                }
                let kind = &database.equipment[at];
                ship |= database.category(kind) & 0x80004003C1 != 0;
                match kind.archetype {
                    Some(archetype) if archetype != at => at = archetype,
                    _ => break,
                }
            }
            if ship && variant.classifier_flags & 1 == 0 {
                return Ok(String::new());
            }
            self.units_variant_name(variant)?
        };
        let key: &[u8] = match (amount > 0, producer.is_some()) {
            (true, false) => b"EFFECT_ADD_EQUIPMENT_TO_STOCKPILE",
            (false, false) => b"EFFECT_REMOVE_EQUIPMENT_FROM_STOCKPILE",
            (true, true) => b"EFFECT_ADD_EQUIPMENT_TO_STOCKPILE_FOREIGN_PRODUCER",
            (false, true) => b"EFFECT_REMOVE_EQUIPMENT_FROM_STOCKPILE_FOREIGN_PRODUCER",
        };
        let mut args = vec![
            ("EQUIPMENT", Argument::Text(&name)),
            ("AMOUNT", Argument::Int32(amount)),
        ];
        let adjective = producer
            .map(|country| self.context.text.country_adjective(country))
            .transpose()?;
        if let Some(adjective) = &adjective {
            args.push(("PRODUCER", Argument::Text(adjective)));
        }
        self.line(key, &args, scopes, true) // 14049B6D0 / 140E53740 / 140C88440.
    }

    fn units_short_equipment_name(
        &self,
        name: &[u8],
        country: u32,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = parser::text(name).map_err(|why| why.to_string())?;
        let short = format!("{name}_short");
        let country_short = format!("{}_{short}", self.tag(country)?);
        if self.context.localisation.get(&country_short).is_some() {
            self.localised(country_short.as_bytes(), &[], scopes, false)
        } else if self.context.localisation.get(&short).is_some() {
            self.localised(short.as_bytes(), &[], scopes, false)
        } else {
            self.units_country_equipment_name(name.as_bytes(), self.tag(country)?, scopes)
        } // LocalizeShortNameForCountry 140BBAA50.
    }

    fn units_equipment_role(
        &self,
        category: u64,
        archetype: &[u8],
        scopes: Scopes,
    ) -> Result<String, String> {
        let archetype = parser::text(archetype).map_err(|why| why.to_string())?;
        let ship = match category & 0x80004003C1 {
            0x40 => Some("ship_designer_capital_ship"),
            0x80 => Some("ship_designer_submarine"),
            0x100 => Some("ship_designer_screen"),
            0x400000 | 0x400040 => Some("ship_designer_carrier"),
            _ => None,
        };
        let key = if let Some(ship) = ship {
            let specific = format!("{ship}_{archetype}");
            if self.context.localisation.get(&specific).is_some() {
                specific
            } else {
                ship.to_owned()
            }
        } else {
            match category {
                4 => format!("tank_designer_{archetype}"),
                0x400 => "plane_designer_fighter".into(),
                0x800 => "plane_designer_interceptor".into(),
                0x1000 => "plane_designer_tactical_bomber".into(),
                0x2000 => "plane_designer_strategic_bomber".into(),
                0x4000 => "plane_designer_cas".into(),
                0x8000 => "plane_designer_naval_bomber".into(),
                0x20000 => "plane_designer_suicide".into(),
                0x40000 => "plane_designer_scout_plane".into(),
                0x200000 => "plane_designer_maritime_patrol_plane".into(),
                0x2000004 => "tank_designer_flame".into(),
                0x4000004 => "tank_designer_amphibious".into(),
                0x8000004 => "tank_designer_anti_air".into(),
                0x10000004 => "tank_designer_artillery".into(),
                0x20000004 => "tank_designer_anti_tank".into(),
                0x40000004 => "tank_designer_rocket".into(),
                0x100000000 => "plane_designer_heavy_fighter".into(),
                _ => "unknown".into(),
            }
        };
        if self.context.localisation.get(&key).is_none() {
            return Err(format!("native equipment role key {key} is not defined by the recorded content stack (140F74D40/140F74710/140F753E0)"));
        }
        self.localised(key.as_bytes(), &[], scopes, false)
    }

    pub(super) fn units_create_variant(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let Some(index) = self.units_equipment_type(fields, &["type"], scopes)? else {
            return Ok(String::new());
        };
        let country = match scopes.this {
            Scope::Country(country) if (country as i32) > 0 => country,
            Scope::State(NULL_STATE) => return Ok(String::new()),
            Scope::State(state) => {
                let row = self.valid_state(state)?.ok_or_else(|| {
                    "native invalid nonzero state owner lookup returns address 0xC8; resolving a positive equipment type would dereference it (14052B6A0/14049BEA0)".to_string()
                })?;
                if (row.owner as i32) < 1 {
                    return Ok(String::new());
                }
                self.context
                    .world()?
                    .slot(row.owner)
                    .ok_or("CountryLinks lacks equipment-variant state owner")?
            }
            _ => return Ok(String::new()),
        };
        let database = self.units_equipment()?;
        let kind = &database.equipment[index];
        let family = &database.equipment[kind.family];
        let no_tech = if parser::read_bool_field(fields, "allow_without_tech", false) {
            false
        } else {
            let world = self.context.world()?;
            for variant in self.units_available_variants(country)? {
                let variant = variant.ok_or(
                    "AvailableEquipmentVariants references an absent VariantDefinitions row",
                )?;
                if (variant.origin_link as i32) >= 1
                    || world.slot(variant.creator_link) != Some(country)
                {
                    continue;
                }
                let token = world
                    .token(variant.equipment_token)?
                    .ok_or("LexicalTokens lacks the available variant equipment token")?;
                if database.get(token) == Some(kind.family) {
                    return Err("candidate-bearing no-tech probe needs unrecorded CEquipmentVariant.version u8 +0x41C (140E535D0/14049BEA0); allow_without_tech=yes or no matching family/creator/origin candidate is decidable without it".into());
                }
            }
            true
        };
        let modular = if kind.module_slots.is_empty() {
            false
        } else {
            let dlc = self
                .context
                .text
                .frame()
                .complete::<tables::SimulationFeatures>()
                .map_err(|why| why.to_string())?
                .first()
                .ok_or("SimulationFeatures lacks the DLC mask (140F72890)")?
                .dlc_mask;
            let mask = if dlc & (1 << 4) != 0 { 0x80004001C0 } else { 0 }
                | if dlc & (1 << 7) != 0 { 4 } else { 0 }
                | if dlc & (1 << 8) != 0 { 0x1D0026FC00 } else { 0 };
            database.category(kind) & mask != 0
        }; // 140C88540 / 140F72890.
        let type_name = self.units_short_equipment_name(
            if modular { &family.name } else { &kind.name },
            country,
            scopes,
        )?;
        let name = string_field(fields, "name")?;
        let key: &[u8] = match (modular, no_tech, name.is_empty()) {
            (false, false, false) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY",
            (false, false, true) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_NAME",
            (false, true, false) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH",
            (false, true, true) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_LEGACY_NO_TECH_NO_NAME",
            (true, false, false) => b"EFFECT_CREATE_EQUIPMENT_VARIANT",
            (true, false, true) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_NO_NAME",
            (true, true, false) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_NO_TECH",
            (true, true, true) => b"EFFECT_CREATE_EQUIPMENT_VARIANT_NO_TECH_NO_NAME",
        };
        let role = if modular {
            let mut category = database.category(kind);
            if category & 0x80004003C1 != 0 {
                let mut modules = BTreeMap::new();
                for body in fields.iter().filter(|field| field.keyword_is("modules")) {
                    for module in children(body) {
                        let slot = installed_content::tokens::Token::c_str(
                            module.key.as_deref().unwrap_or_default(),
                        );
                        if module.value_keyword().as_deref() == Some("empty") {
                            modules.insert(slot, 0);
                        } else if let Some(name) = module.value.scalar()
                            && let Some(mask) = database.module_category(name)
                        {
                            modules.insert(slot, mask);
                        } // 14191CCF0: unknown module keeps the preceding slot value.
                    }
                }
                for mask in modules.values() {
                    category |= *mask;
                }
                category = installed_content::equipment::normalize_category(category);
            } // Own vector only; do not merge/remove native default module categories.
            let archetype = family.archetype.map_or(b"".as_slice(), |index| {
                database.equipment[index].name.as_slice()
            });
            Some(self.units_equipment_role(category, archetype, scopes)?)
        } else {
            None
        };
        let mut args = vec![("TYPE", Argument::Text(&type_name))];
        if !name.is_empty() {
            args.push(("NAME", Argument::Text(name)));
        }
        if let Some(role) = &role {
            args.push(("ROLE", Argument::Text(role)));
        }
        self.localised(key, &args, scopes, true) // 14049BEA0: no added LF.
    }

    pub(super) fn units_subsidy(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let country = match scopes.this {
            Scope::Country(country) => country,
            _ => 0,
        };
        let country = self.country_name(country, false)?;
        let cic = match field(fields, "cic") {
            Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
            None => 0,
        };
        let kind = self.units_equipment_type(fields, &["equipment_type"], scopes)?;
        let kind = match kind {
            Some(index) => {
                self.units_current_name(&self.units_equipment()?.equipment[index].name, scopes)?
            }
            None => self.units_current_name(b"", scopes)?,
        };
        let mut sellers = Vec::new();
        for body in fields.iter().filter(|field| field.keyword_is("seller_tags")) {
            for value in children(body) {
                let Some(country) = self
                    .evaluator()?
                    .scoped_country(scalar_operand(value)?, scopes)?
                else {
                    continue;
                };
                sellers.push(self.country_name(country, false)?);
            }
        }
        let sellers = if sellers.is_empty() {
            if field(fields, "seller_trigger").is_some_and(|field| {
                field
                    .value
                    .read_string()
                    .is_some_and(|name| !name.is_empty())
            }) {
                return Err("native seller_trigger fallback needs trigger-template vf12 description with parameter6=2 (140306DC0); the shared trigger-description helper is unavailable".into());
            }
            String::new()
        } else {
            sellers.join(&self.localised(b"LIST_OR", &[], scopes, false)?)
        };
        self.line(
            b"EFFECT_MARKET_ADD_EQUIPMENT_SUBSIDY",
            &[
                ("COUNTRY", Argument::Text(&country)),
                ("CIC", Argument::Fixed5(cic)),
                ("TYPE", Argument::Text(&kind)),
                ("SELLERS", Argument::Text(&sellers)),
            ],
            scopes,
            true,
        ) // 140305540 / 140306C50: positive sellers retain order and duplicates.
    }

    pub(super) fn units_unit_bonus(&self, item: &Item) -> Result<String, String> {
        if children(item).iter().all(|entry| entry.keyword_is("name")) {
            return Ok(String::new()); // Empty arrays in 140635420, called by 1403794A0.
        }
        Err("native nonempty unit-bonus formatter needs CSettings+0x2AC template_counter (140635420/1401FABF0) and registered sub-unit/category arrays; the caller exposes neither cached counter preference nor category description helper".into())
    }

    fn units_current_name(&self, name: &[u8], scopes: Scopes) -> Result<String, String> {
        self.units_country_equipment_name(name, self.tag(self.context.perspective)?, scopes)
        // LocalizeForCurrentCountry 140BBA630 -> 140BBA780.
    }

    pub(super) fn units_manpower(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let amount =
            crate::triggers::fixed_round(self.scoped_value(scalar_operand(item)?, scopes)?) as i32;
        let (key, name) = match scopes.this {
            Scope::State(state) if (state as i32) > 0 => (
                b"EFFECT_MANPOWER_ADD_STATE".as_slice(),
                self.state_name(state)?,
            ),
            Scope::Country(country) => (
                b"EFFECT_MANPOWER_ADD_COUNTRY".as_slice(),
                self.country_name(country, false)?,
            ),
            Scope::State(_) => (
                b"EFFECT_MANPOWER_ADD_COUNTRY".as_slice(),
                self.country_name(0, false)?,
            ),
            _ => return Err("native CEventScope state/country fields unavailable (140384C00)".into()),
        };
        self.line(
            key,
            &[("NAME", Argument::Text(&name)), ("VAL", Argument::Int32(amount))],
            scopes,
            true,
        ) // 140384C00, round 1424D8760.
    }

    pub(super) fn units_template(
        &self,
        item: &Item,
        scopes: Scopes,
        description: TemplateDescription,
    ) -> Result<String, String> {
        let fields = children(item);
        let name = string_field(fields, "division_template")?;
        let key = match description {
            TemplateDescription::Delete => {
                if parser::read_bool_field(fields, "disband", false) {
                    b"EFFECT_DISBAND_UNITS".as_slice()
                } else {
                    b"EFFECT_DELETE_UNITS".as_slice()
                }
            } // 14037EEF0.
            TemplateDescription::Lock => {
                if parser::read_bool_field(fields, "is_locked", false) {
                    b"EFFECT_DISABLE_TEMPLATE_EDITTING".as_slice()
                } else {
                    b"EFFECT_ENABLE_TEMPLATE_EDITTING".as_slice()
                }
            } // 140390D10: native spelling has EDITTING.
            TemplateDescription::ClearCap => b"EFFECT_CLEAR_TEMPLATE_DIVISION_CAP".as_slice(), // 14037B560.
            TemplateDescription::Cap => {
                let cap = match field(fields, "division_cap") {
                    Some(cap) => self.scoped_value(scalar_operand(cap)?, scopes)?,
                    None => 0,
                };
                return self.line(
                    b"EFFECT_SET_TEMPLATE_DIVISION_CAP",
                    &[
                        ("NAME", Argument::Text(name)),
                        ("CAP", Argument::Int32((cap / 100_000) as i32)),
                    ],
                    scopes,
                    true,
                ); // 140393B40 truncates before narrowing, not fixed_round.
            }
            TemplateDescription::Recruit => {
                let allowed = self.localised(
                    if parser::read_bool_field(fields, "force_allow_recruiting", false) {
                        b"YES"
                    } else {
                        b"NO"
                    },
                    &[],
                    scopes,
                    false,
                )?;
                return self.line(
                    b"EFFECT_SET_TEMPLATE_FORCE_ALLOW_RECRUITING",
                    &[
                        ("NAME", Argument::Text(name)),
                        ("ALLOW", Argument::Text(&allowed)),
                    ],
                    scopes,
                    true,
                ); // 140393DE0.
            }
        };
        self.line(key, &[("NAME", Argument::Text(name))], scopes, true)
    }

    pub(super) fn units_lock_all(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let locked = match item.value.scalar() {
            Some(value) => parser::read_bool(value).unwrap_or(false),
            None => parser::read_bool_field(children(item), "is_locked", false),
        };
        self.line(
            if locked {
                b"EFFECT_DISABLE_ALL_TEMPLATE_EDITING"
            } else {
                b"EFFECT_ENABLE_ALL_TEMPLATE_EDITING"
            },
            &[],
            scopes,
            true,
        ) // 140384680; bool reader 141385770 / 1403B34B0.
    }

    pub(super) fn units_transfer(
        &self,
        item: &Item,
        scopes: Scopes,
        navy: bool,
    ) -> Result<String, String> {
        let name = self.country_name(self.country_target(item, scopes)?.unwrap_or(0), true)?;
        let args = [(
            if navy { "COUNTRY" } else { "RECIPIENT" },
            Argument::Text(&name),
        )];
        if navy {
            self.line(b"EFFECT_TRANSFER_NAVY", &args, scopes, true) // 1403983B0.
        } else {
            self.localised(b"EFFECT_TRANSFER_UNITS", &args, scopes, true) // 140398970: no LF.
        }
    }

    pub(super) fn units_equipment_fraction(
        &self,
        item: &Item,
        scopes: Scopes,
        send: bool,
    ) -> Result<String, String> {
        if send {
            let value = match field(children(item), "fraction") {
                Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
                None => 0,
            }
            .clamp(0, 100_000);
            let name = self.country_name(self.country_target(item, scopes)?.unwrap_or(0), false)?;
            self.line(
                b"EFFECT_SEND_EQUIPMENT_FRACTION",
                &[
                    ("FRACTION", Argument::Fixed5(value)),
                    ("COUNTRY", Argument::Text(&name)),
                ],
                scopes,
                true,
            ) // 14049CF20.
        } else {
            let value = self
                .scoped_value(scalar_operand(item)?, scopes)?
                .wrapping_sub(100_000);
            self.line(
                b"EFFECT_EQUIPMENT_FRACTION",
                &[("VAL", Argument::Fixed5(value))],
                scopes,
                true,
            )
        } // 14049D1D0.
    }

    pub(super) fn units_design_bonus(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let name = self.localised(string_field(fields, "name")?.as_bytes(), &[], scopes, false)?;
        let mut text = self.localised(
            b"EFFECT_ADD_DESIGN_TEMPLATE_BONUS",
            &[
                (
                    "DISCOUNT",
                    Argument::Fixed5(fixed_field(fields, "cost_factor", 0)),
                ),
                ("USES", Argument::Int32(integer_field(fields, "uses", 0))),
                ("NAME", Argument::Text(&name)),
            ],
            scopes,
            false,
        )?;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text) // 14049B380: no scoped expansion.
    }

    pub(super) fn units_raid_damage(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let ratio = parser::read_bool_field(fields, "ratio", false);
        let value = |name| -> Result<Option<i64>, String> {
            field(fields, name)
                .map(|value| self.scoped_value(scalar_operand(value)?, scopes))
                .transpose()
        };
        let (key, args) = if let Some(amount) = value("plane_loss")? {
            (
                if ratio {
                    b"EFFECT_RAID_DAMAGE_PLANE_RATIO".as_slice()
                } else {
                    b"EFFECT_RAID_DAMAGE_PLANE".as_slice()
                },
                vec![("VAL", Argument::Fixed5(amount))],
            )
        } else if let Some(amount) = value("damage")? {
            (
                b"EFFECT_RAID_DAMAGE_UNIT_STRENGTH".as_slice(),
                vec![("VAL", Argument::Fixed5(amount))],
            )
        } else {
            match (value("str_damage")?, value("org_damage")?) {
                (Some(strength), Some(organization)) => (
                    if ratio {
                        b"EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG_RATIO".as_slice()
                    } else {
                        b"EFFECT_RAID_DAMAGE_UNIT_STRENGTH_ORG".as_slice()
                    },
                    vec![
                        ("STR_VAL", Argument::Fixed5(strength)),
                        ("ORG_VAL", Argument::Fixed5(organization)),
                    ],
                ),
                (Some(amount), None) => (
                    if ratio {
                        b"EFFECT_RAID_DAMAGE_UNIT_STRENGTH_RATIO".as_slice()
                    } else {
                        b"EFFECT_RAID_DAMAGE_UNIT_STRENGTH".as_slice()
                    },
                    vec![("VAL", Argument::Fixed5(amount))],
                ),
                (None, Some(amount)) => (
                    if ratio {
                        b"EFFECT_RAID_DAMAGE_UNIT_ORG_RATIO".as_slice()
                    } else {
                        b"EFFECT_RAID_DAMAGE_UNIT_ORG".as_slice()
                    },
                    vec![("VAL", Argument::Fixed5(amount))],
                ),
                (None, None) => return Ok(String::new()),
            }
        };
        self.localised(key, &args, scopes, true) // 1404A31F0: priority plane/damage/str/org, no LF.
    }

    pub(super) fn units_add_to_template(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let country = self.country_name(self.current_country(scopes)?, false)?;
        self.line(
            b"EFFECT_ADD_UNITS_TO_DIVISION_TEMPLATE",
            &[
                ("COUNTRY", Argument::Text(&country)),
                (
                    "TEMPLATE_NAME",
                    Argument::Text(string_field(children(item), "template_name")?),
                ),
            ],
            scopes,
            true,
        ) // 1403794F0; reader 1403AF6B0.
    }

    pub(super) fn units_create_ship(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let country = match field(children(item), "creator") {
            Some(creator) => self
                .evaluator()?
                .scoped_country(scalar_operand(creator)?, scopes)?,
            None => match scopes.this {
                Scope::Country(country) if (country as i32) > 0 => Some(country),
                _ => None,
            }, // Native direct CEventScope.country +8, not state owner (14037D200).
        };
        let Some(country) = country else {
            return Ok(String::new());
        };
        let country = self.country_name(country, true)?;
        self.localised(
            b"EFFECT_CREATE_SHIP",
            &[
                (
                    "VARIANT",
                    Argument::Text(string_field(children(item), "equipment_variant")?),
                ),
                ("COUNTRY", Argument::Text(&country)),
            ],
            scopes,
            true,
        ) // 14037D200: no LF; creator reader 1403B1A30.
    }

    pub(super) fn units_create(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if item.value.scalar().is_some() {
            return Err("native ReadScript rejects the scalar create_unit body and retains constructor defaults; that diagnostic/default form is not described (140534280/14033CDC0)".into());
        }
        let cursor = scopes
            .cursor
            .ok_or("create_unit needs its native caller scope (1405285F0)")?;
        let at = self.graph.borrow().scopes.len();
        {
            let mut graph = self.graph.borrow_mut();
            let mut copied = *graph
                .scopes
                .get(cursor)
                .ok_or("create_unit caller scope is missing")?;
            for link in [&mut copied.root, &mut copied.from, &mut copied.prev] {
                if *link == Link::Itself {
                    *link = Link::Scope(cursor);
                }
            }
            graph.scopes.push(copied); // copy_construct 1405285F0/copy_members 1405296C0.
            if let Some(targets) = graph.tooltip_targets.get(&cursor).cloned() {
                graph.tooltip_targets.insert(at, targets);
            }
            if let Some(targets) = graph.saved_targets.get(&cursor).cloned() {
                graph.saved_targets.insert(at, targets);
            }
        }
        let result = (|| {
            let copied = crate::triggers::event_scopes::project(&self.graph.borrow(), at)?;
            self.units_create_in(item, copied, scopes)
        })();
        let mut graph = self.graph.borrow_mut();
        graph.scopes.truncate(at);
        graph.tooltip_targets.retain(|cursor, _| *cursor < at);
        graph.saved_targets.retain(|cursor, _| *cursor < at);
        result
    }

    fn units_create_in(
        &self,
        item: &Item,
        scopes: Scopes,
        expansion: Scopes,
    ) -> Result<String, String> {
        let count = match field(children(item), "count") {
            Some(count) => self.scoped_value(scalar_operand(count)?, scopes)?,
            None => 100_000,
        };
        let count = crate::triggers::fixed_round(count) as i32;
        if count < 1 {
            return Ok(String::new());
        }
        let owner = || -> Result<u32, String> {
            match field(children(item), "owner") {
                Some(owner) => Ok(self
                    .evaluator()?
                    .scoped_country(scalar_operand(owner)?, scopes)?
                    .unwrap_or(0)),
                None => Ok(0), // Constructor 14033CDC0 leaves vf15 selector fallback zero.
            }
        };
        let location = self.units_create_location(item, scopes, owner()?)?;
        let country = owner()?; // 14037D490 reevaluates after the copied seed advances.
        let owner = self.context.text.country_adjective(country).map_err(|why| {
            if country == 0 {
                format!("native owner selector is reserved CCountry[0]; its adjective/name input is unavailable (14033CDC0/1405336C0/140BA5560): {why}")
            } else {
                why
            }
        })?;
        let state = location.map(|state| self.state_name(state)).transpose()?;
        let mut args = vec![("OWNER", Argument::Text(&owner))];
        if count > 1 {
            args.push(("COUNT", Argument::Int32(count)));
        }
        if let Some(state) = &state {
            args.push(("STATE", Argument::Text(state)));
        }
        self.localised(
            match (count > 1, state.is_some()) {
                (false, false) => b"EFFECT_CREATE_UNIT".as_slice(),
                (true, false) => b"EFFECT_CREATE_UNIT_WITH_COUNT".as_slice(),
                (false, true) => b"EFFECT_CREATE_UNIT_LOCATION".as_slice(),
                (true, true) => b"EFFECT_CREATE_UNIT_LOCATION_WITH_COUNT".as_slice(),
            },
            &args,
            expansion,
            true,
        ) // 14037D490: rounded count, no LF.
    }

    fn units_create_location(
        &self,
        item: &Item,
        scopes: Scopes,
        owner: u32,
    ) -> Result<Option<u32>, String> {
        let Scope::State(state) = scopes.this else {
            return Ok(None);
        };
        if self.valid_state(state)?.is_none() {
            return Ok(None);
        } // 14036A810.
        let evaluator = self.evaluator()?;
        let world = evaluator.world();
        let frame = world.frame();
        frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?;
        frame
            .complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?;
        let fields = children(item);
        let priority = integer_field(fields, "prioritize_location", 0) as u32;
        let enemy = parser::read_bool_field(fields, "allow_spawning_on_enemy_provs", false);
        let score = field(fields, "country_score");
        let provinces = if score.is_some() {
            world.state_order(state)?.to_vec()
        } else {
            world
                .provinces
                .values()
                .filter(|province| province.state == state)
                .map(|province| province.id)
                .collect::<Vec<_>>()
        };
        if provinces.is_empty() {
            return Err(format!("state {state} has no native province candidates; 14036A810 divides by the state's province count"));
        }
        let cursor = scopes
            .cursor
            .ok_or("create_unit location needs its copied scope cursor (14036A810)")?;
        let copied = self.graph.borrow().scopes[cursor];
        let start = copied.random_draw()? as usize % provinces.len();
        self.graph.borrow_mut().scopes[cursor].random_seed = copied.random_seed.wrapping_add(1);
        // 14036A810 changes only the copied +0x0C seed, preserving caller +0x10.
        let impassable = parser::parse(b"impassable=yes")
            .map_err(|why| why.to_string())?
            .items;
        let mut blocked = BTreeMap::new();
        let mut controller_scores = BTreeMap::new();
        let country = |raw| {
            world.slot(raw).ok_or_else(|| {
                format!("CountryLinks lacks province controller {raw} (1406F3A90)")
            })
        };
        let war = |controller| -> Result<bool, String> {
            for row in frame
                .complete::<tables::WarRelations>()
                .map_err(|why| why.to_string())?
            {
                let pair = (country(row.first_country)?, country(row.second_country)?);
                if pair == (controller, owner) || pair == (owner, controller) {
                    return Ok(true);
                }
            }
            Ok(false) // Installed relation slots, no date/cancel filter (1406F3A00).
        };
        let visibility = |controller| -> Result<bool, String> {
            if controller == owner {
                return Ok(true);
            }
            if war(controller)? {
                return Ok(false);
            }
            let access = frame
                .complete::<tables::RelationLinks>()
                .map_err(|why| why.to_string())?
                .iter()
                .any(|row| {
                    world.slot(row.country) == Some(controller)
                        && row.target_index == owner
                        && row.links & replay::rows::RelationLinkRow::MILITARY_ACCESS != 0
                });
            if access {
                return Ok(true);
            }
            if let Some(faction) = world.faction(controller)?
                && faction != 0
                && world.faction(owner)? == Some(faction)
            {
                return Ok(true);
            }
            for row in frame
                .complete::<tables::Subjects>()
                .map_err(|why| why.to_string())?
            {
                let pair = (country(row.overlord)?, country(row.subject)?);
                if pair == (controller, owner) || pair == (owner, controller) {
                    return Ok(true);
                }
            }
            Ok(false) // Direct installed subjects, no date/cancel filter (1406F3B00).
        };
        let mut admitted = |province: &replay::rows::ProvinceRow, ignore_occupants| -> Result<bool, String> {
            if !world.state_provinces.contains_key(&province.state) {
                return Err(format!("province {} state {} is not defined by the recorded content stack (140E6A1A0)", province.id, province.state));
            }
            let impassable_state = match blocked.get(&province.state) {
                Some(&blocked) => blocked,
                None => {
                    let value = evaluator
                        .evaluate(&impassable, Scopes::of(Scope::State(province.state)))
                        .decided()?;
                    blocked.insert(province.state, value);
                    value
                }
            };
            if impassable_state {
                return Ok(false);
            }
            let controller = country(province.controller)?;
            if visibility(controller)? {
                return Ok(true);
            }
            if !enemy || !war(controller)? {
                return Ok(false);
            }
            if ignore_occupants {
                return Ok(true);
            }
            let occupants = frame
                .complete::<tables::ProvinceUnitOrder>()
                .map_err(|why| why.to_string())?
                .iter()
                .find(|row| row.province == province.id)
                .ok_or_else(|| {
                    format!("ProvinceUnitOrder lacks province {} (140339920)", province.id)
                })?;
            Ok(occupants.mixed.len() == occupants.naval.len())
            // 140E65210/140E6B110: mixed = land + naval + railway units.
        };
        for ignore_occupants in [score.is_some(), true] {
            if let Some(province) = world.provinces.get(&priority)
                && admitted(province, ignore_occupants)?
            {
                return Ok(Some(province.state));
            }
            let mut best = -0x53E2_D620_FB40_i64; // 14036A2F0's initial best score.
            let mut found = false;
            for index in 0..provinces.len() {
                let id = provinces[(start + index) % provinces.len()];
                let province = world.provinces.get(&id).ok_or_else(|| {
                    format!("Provinces lacks state {state} candidate {id} (14036A2F0)")
                })?;
                if !admitted(province, ignore_occupants)? {
                    continue;
                }
                let controller = country(province.controller)?;
                let value = if let Some(score) = score {
                    match controller_scores.get(&controller) {
                        Some(&value) if value != -0x53E2_D620_FB40_i64 => value,
                        _ => {
                            let cursor = scopes.cursor.ok_or(
                                "country_score needs the caller's event graph cursor (14036A2F0)",
                            )?;
                            let at = self.graph.borrow().scopes.len();
                            self.graph.borrow_mut().scopes.push(crate::scripted_text::EventScope {
                                prev: Link::Scope(cursor),
                                ..crate::scripted_text::EventScope::of(EventObject::Country(controller))
                            }); // 140528800/14052DD00: fresh ROOT/FROM, PREV caller.
                            let result = (|| {
                                let projected = crate::triggers::event_scopes::project(
                                    &self.graph.borrow(),
                                    at,
                                )?;
                                evaluator.mean_time_to_happen(
                                    "create_unit.country_score",
                                    children(score),
                                    projected,
                                )
                            })();
                            self.graph.borrow_mut().scopes.truncate(at);
                            let value = result?;
                            controller_scores.insert(controller, value);
                            value
                        }
                    }
                } else {
                    let land = frame
                        .complete::<tables::Divisions>()
                        .map_err(|why| why.to_string())?
                        .iter()
                        .filter(|unit| unit.province == province.id)
                        .count() as i32;
                    let base = province.victory_points.wrapping_add(land.wrapping_mul(-20)) as i64
                        * 100_000;
                    base + if controller == owner {
                        200_000_000
                    } else if visibility(controller)? {
                        100_000_000
                    } else {
                        0
                    }
                };
                if score.is_some() && value < 1 {
                    continue;
                }
                if value > best {
                    best = value;
                    found = true;
                }
            }
            // Every native score query runs. Only STATE reaches 14037D490;
            // ties in the no-score form cannot change the description.
            if found {
                return Ok(Some(state));
            }
            if !enemy || ignore_occupants {
                break;
            }
        }
        Ok(None) // 14036A810/14036A2F0: no location still describes the unit.
    }

    pub(super) fn units_unlock(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let name = parser::text(parser::c_string(scalar_operand(item)?)).map_err(|why| why.to_string())?;
        let units = self
            .context
            .content
            .key_names
            .and_then(|names| names.sub_units.as_ref())
            .ok_or_else(|| self.content_gap("effect key names"))?;
        let index = match units.get(name) {
            installed_content::unit_leader::SubUnit::Index(index) => index,
            installed_content::unit_leader::SubUnit::Unknown => {
                return Err(format!("sub-unit {name} is not defined by the recorded content stack (140AB5DA0)"));
            }
            installed_content::unit_leader::SubUnit::Uncertain(why) => {
                return Err(format!("sub-unit {name}: {why}"));
            }
        };
        let name = units
            .name(index)
            .ok_or("registered sub-unit has no spelling")??;
        let name = self.units_current_name(name.as_bytes(), scopes)?;
        self.localised(
            b"UNLOCK_SUBUNIT_EFFECT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            false,
        )
        // 14051E3C0 -> 1402E30F0: no expansion or LF.
    }

    pub(super) fn delete_template(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let value = item
            .value
            .scalar()
            .or_else(|| field(fields, "division_template").and_then(|item| item.value.read_string()))
            .unwrap_or_default();
        let label = std::str::from_utf8(installed_content::parser::c_string(value))
            .map_err(|_| "template name is not UTF-8")?;
        let disband = installed_content::parser::read_bool_field(fields, "disband", false);
        self.line(
            if disband {
                b"EFFECT_DISBAND_UNIT_TEMPLATE_AND_UNITS"
            } else {
                b"EFFECT_DELETE_UNIT_TEMPLATE_AND_UNITS"
            },
            &[("NAME", Argument::Text(label))],
            scopes,
            true,
        ) // 14037EC90
    }
}

// 1413D1220 / 1413D0370: native keys indexed by installed_content::equipment::STATS.
const EQUIPMENT_STAT_KEYS: [(&[u8], bool); 78] = [
    (b"STAT_ARMY_DEFAULT_MORALE", true), // 142938668.
    (b"STAT_ARMY_DEFENSE", true), // 142938688.
    (b"STAT_ARMY_BREAKTHROUGH", true), // 1429386A0.
    (b"STAT_ARMY_HARDNESS", true), // 1429386B8.
    (b"STAT_ARMY_SOFT_ATTACK", true), // 1429386D0.
    (b"STAT_ARMY_HARD_ATTACK", true), // 1429386E8.
    (b"STAT_ARMY_RECON", true), // 142938700.
    (b"STAT_ARMY_ENTRENCHMENT", true), // 142938710.
    (b"STAT_ARMY_INITIATIVE", true), // 142938728.
    (b"STAT_ARMY_CASUALTY_TRICKLEBACK", true), // 142938740.
    (b"STAT_ARMY_SUPPLY_CONSUMPTION_FACTOR", true), // 142938618.
    (b"STAT_ARMY_SUPPLY_CONSUMPTION", false), // 142938760.
    (b"STAT_ARMY_SUPRESSION", true), // 142938780.
    (b"STAT_ARMY_SUPRESSION_FACTOR", true), // 1429385F8.
    (b"STAT_ARMY_EXPERIENCE_LOSS_FACTOR", true), // 142938798.
    (b"STAT_ARMY_EQUIPMENT_CAPTURE_FACTOR", true), // 1429387C0.
    (b"STAT_ARMY_FUEL_CAPACITY", true), // 1429387E8.
    (b"STAT_ARMY_RECOVERY", true), // 142938800.
    (b"STAT_ARMY_ADDITIONAL_COLLATERAL_DAMAGE", true), // 142938818.
    (b"STAT_NAVY_SURFACE_DETECTION", true), // 142938840.
    (b"STAT_NAVY_SUB_DETECTION", true), // 142938860.
    (b"STAT_NAVY_SURFACE_VISIBILITY", false), // 142938878.
    (b"STAT_NAVY_SUB_VISIBILITY", false), // 142938898.
    (b"STAT_NAVY_CARRIER_SUB_DETECTION", false), // 1429388B8.
    (b"STAT_NAVY_CARRIER_SURFACE_DETECTION", false), // 1429388D8.
    (b"STAT_NAVY_LG_ATTACK", true), // 142938900.
    (b"STAT_NAVY_LG_ARMOR_PIERCING", true), // 142938918.
    (b"STAT_NAVY_HG_ATTACK", true), // 142938938.
    (b"STAT_NAVY_HG_ARMOR_PIERCING", true), // 142938950.
    (b"STAT_NAVY_TORPEDO_ATTACK", true), // 142938970.
    (b"STAT_NAVY_SUB_ATTACK", true), // 142938990.
    (b"STAT_NAVY_ANTI_AIR_ATTACK", true), // 1429389A8.
    (b"STAT_NAVY_AMPHIBIOUS_DEFENSE", true), // 1429389C8.
    (b"STAT_NAVY_MAXIMUM_SPEED", true), // 1429389E8.
    (b"STAT_NAVY_RANGE", true), // 142938A00.
    (b"STAT_NAVY_MINES_PLANTING", true), // 142938A10.
    (b"STAT_NAVY_MINES_SWEEPING", true), // 142938A30.
    (b"STAT_NAVY_LIGHT_GUN_HIT_CHANCE", true), // 142938A50.
    (b"STAT_NAVY_HEAVY_GUN_HIT_CHANCE", true), // 142938A70.
    (b"STAT_NAVY_TORPEDO_HIT_CHANCE", true), // 142938A90.
    (b"STAT_NAVY_TORPEDO_DAMAGE_REDUCTION", true), // 142938AB0.
    (b"STAT_NAVY_TORPEDO_ENEMY_CRIT_CHANCE", false), // 142938AD8.
    (b"STAT_NAVY_WEATHER_PENALTY", false), // 142938B00.
    (b"STAT_NAVY_RAIDING_COORDINATION", true), // 142938B20.
    (b"STAT_NAVY_PATROL_COORDINATION", true), // 142938B40.
    (b"STAT_NAVY_SEARCH_AND_DESTROY_COORDINATION", true), // 142938B60.
    (b"STAT_AIR_RANGE", true), // 142938B90.
    (b"STAT_AIR_DEFENCE", true), // 142938BA0.
    (b"STAT_AIR_ATTACK", true), // 142938BB8.
    (b"STAT_AIR_AGILITY", true), // 142938BC8.
    (b"STAT_AIR_BOMBING", true), // 142938BE0.
    (b"STAT_AIR_SUPERIORITY", true), // 142938BF8.
    (b"STAT_AIR_NAVAL_STRIKE_ATTACK", true), // 142938C10.
    (b"STAT_AIR_NAVAL_STRIKE_TARGETTING", true), // 142938C30.
    (b"STAT_AIR_GROUND_ATTACK", true), // 142938C58.
    (b"STAT_AIR_VISIBILITY_FACTOR", true), // 142938C70.
    (b"STAT_RAILWAY_GUN_ATTACK", true), // 142938C90.
    (b"STAT_RAILWAY_GUN_ATTACK_RANGE", true), // 142938CA8.
    (b"STAT_RAILWAY_GUN_ANNEX_RATIO", false), // 142938CC8.
    (b"STAT_RAILWAY_GUN_HOURS_TO_REDISTRIBUTE", false), // 142938CE8.
    (b"STAT_COMMON_MAX_ORG", true), // 142938D10.
    (b"STAT_COMMON_MAX_STRENGTH", true), // 142938D28.
    (b"STAT_COMMON_MAXIMUM_SPEED", true), // 142938D48.
    (b"STAT_COMMON_ARMOR", true), // 142938D68.
    (b"STAT_COMMON_PIERCING", true), // 142938D80.
    (b"STAT_COMMON_RELIABILITY", true), // 142938D98.
    (b"STAT_COMMON_RELIABILITY_FACTOR", true), // 142938DB0.
    (b"STAT_COMMON_WEIGHT", false), // 142938DD0.
    (b"STAT_COMMON_THRUST", true), // 142938DE8.
    (b"STAT_COMMON_FUEL_CONSUMPTION", false), // 142938E00.
    (b"STAT_COMMON_FUEL_CONSUMPTION_FACTOR", false), // 142938640.
    (b"STAT_STRATEGIC_ATTACK", true), // 142938E20.
    (b"STAT_CARRIER_SIZE", true), // 142938E38.
    (b"STAT_SUB_CARRIER_SIZE", true), // 142938E50.
    (b"STAT_COMMON_ACCLIMATIZATION_HOT_GAIN", true), // 142938E68.
    (b"STAT_COMMON_ACCLIMATIZATION_COLD_GAIN", true), // 142938E90.
    (b"STAT_COMMON_NIGHT_PENALTY", false), // 142938EB8.
    (b"STAT_COMMON_BUILD_COST_IC", false), // 142938ED8.
];

impl Renderer<'_, '_, '_> {
    pub(super) fn units_equipment_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut entries = Vec::new();
        // 14049E0E0 appends every bonus body; name, project and prefix are not
        // consumed by 14049B640's description path.
        for bonus in children(item)
            .iter()
            .filter(|child| child.keyword_is("bonus"))
        {
            for entry in children(bonus) {
                let name = entry.key.as_deref().unwrap_or_default();
                let name = self.localised(parser::c_string(name), &[], scopes, false)?;
                let mut description = format!("\u{11}Y{name}\u{11}!: \n");
                let mut stats: Vec<(usize, i64)> = Vec::new();
                for stat in children(entry) {
                    if stat.keyword_is("instant") {
                        continue;
                    }
                    let Some(index) = stat
                        .keyword()
                        .and_then(|key| installed_content::equipment::stat_index(&key.folded()))
                    else {
                        continue; // Native unexpected-token skip; no stored stat.
                    };
                    let value = stat
                        .value
                        .scalar()
                        .and_then(installed_content::fixed5::script_fixed5)
                        .unwrap_or(0);
                    match stats.iter_mut().find(|(stored, _)| *stored == index) {
                        Some((_, previous)) => *previous = previous.wrapping_add(value),
                        None => stats.push((index, value)),
                    } // ReadAndCombineStatToken 140625D00; zero remains stored.
                }
                let mut lines = Vec::new();
                for (index, value) in stats {
                    let (key, positive_good) = EQUIPMENT_STAT_KEYS[index];
                    let label = self.localised(key, &[], scopes, false)?;
                    let difference = format!(
                        "{}_MOD_DIFF",
                        parser::text(key).map_err(|why| why.to_string())?
                    );
                    let difference_key: &[u8] =
                        if self.context.localisation.get(&difference).is_some() {
                            difference.as_bytes()
                        } else if positive_good {
                            b"STAT_MODIFIER_DIFF"
                        } else {
                            b"STAT_MODIFIER_DIFF_NEG"
                        }; // Positive-good bytes 14309B650; branch 1413D0370.
                    let value = self.localised(
                        difference_key,
                        &[("VALUE", Argument::Fixed5(value))],
                        scopes,
                        false,
                    )?;
                    lines.push(format!("    {label}{value}"));
                }
                description.push_str(&lines.join("\n")); // 140711C80 / 1407104A0.
                entries.push(description);
            }
        }
        Ok(entries.join("\n")) // 140711B20 / 140710720; no added terminal LF.
    }
}
