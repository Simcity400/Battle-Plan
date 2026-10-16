use super::*;
use installed_content::parser;
use replay::rows::{VariantDefinitionRow, tables};
use std::cmp::Ordering;
use std::collections::BTreeSet;

mod bonuses;

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
        .filter_map(
            |item| match installed_content::crt::scan_i32(item.value.scalar()?) {
                installed_content::crt::Scanned::Stored(value) => Some(value),
                _ => None,
            },
        )
        .next_back()
        .unwrap_or(default) // Read a signed integer, retaining the default after a failed scan.
}

fn variant_version(variant: &VariantDefinitionRow) -> Result<u8, String> {
    variant
        .version
        .require("VariantDefinitions.version")
        .copied()
}

struct EquipmentTexts<'a>(&'a Localisation, &'a str);

impl crate::variant_names::Texts for EquipmentTexts<'_> {
    fn has(&self, key: &str) -> bool {
        self.0.get(key).is_some()
    }

    fn text(&self, key: &str) -> Option<String> {
        self.0.localise_with(key.as_bytes(), &[]).ok()
    }

    fn stored_text(&self, text: &str, what: &str) -> Result<String, String> {
        crate::scripted_text::stored_text(self.0, self.1, text.as_bytes(), what)
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
    pub(super) fn units_change_template(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        // Divisions supplies the Army existence witness.
        let world = self.context.world()?;
        let owner = world.unit(reference)?.logical_owner; // Use the unit's logical owner rather than its roster owner.
        let country = if owner == 0 {
            0 // Only raw zero maps directly to reserved country zero.
        } else {
            world.slot(owner).ok_or_else(|| {
                format!("CountryLinks lacks change-division-template logical owner {owner}")
            })?
        };
        if country == 0 {
            return Err("reserved country-zero division-template roster is not recorded".into());
        }
        let template = string_field(children(item), "division_template")?; // Read the authored template string.
        if !world.has_template(country, template.as_bytes())? {
            return Ok(String::new()); // An absent template precedes the unit-name read.
        }
        let name = self.unit_name(reference)?;
        // The exact comparison already established the display-name bytes.
        self.localised(
            b"CHANGE_DIV_TEMPLATE_EFFECT",
            &[
                ("TEMPLATE", Argument::Text(template)),
                ("DIV", Argument::Text(&name)),
            ],
            scopes,
            false,
        ) // Plain localisation, without LF.
    }

    pub(super) fn units_destroy_unit(&self, scopes: Scopes) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        // Resolve the unit; the effect operand is unused.
        let name = self.unit_name(reference)?;
        self.line(
            b"EFFECT_UNIT_DESTROY_UNIT",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        ) // Expand in the caller scope, then append LF.
    }

    pub(super) fn units_set_organization(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Scope::Unit(reference) = scopes.this else {
            return Ok(String::new());
        };
        if reference == 0 {
            return Ok(String::new());
        }
        self.context.world()?.unit(reference)?; // Resolve the Army before evaluating the operand.
        let value = if scope_selector(item).is_some() {
            0 // Read selectors before assigning the default-zero operand.
        } else {
            self.scoped_value(scalar_operand(item)?, scopes)?
        };
        self.localised(
            b"EFFECT_SET_UNIT_ORGANIZATION",
            &[("VAL", Argument::Fixed5(value))],
            scopes,
            false,
        ) // Raw type-9 value, without clamping, expansion or LF.
    }

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
            let value =
                parser::c_string(field.value.scalar().ok_or(
                    "equipment-type block operand is unsupported by the direct token reader",
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
            cached = database.get(input); // A later pending token replaces the earlier cache.
        }
        if cached.is_none() && queued {
            let token = (self.scoped_value(input, scopes)? / 100_000) as u32;
            if token == 0 {
                return Ok(None);
            }
            let literal =
                self.context.world()?.token(token)?.ok_or_else(|| {
                    format!("LexicalTokens lacks equipment variable token {token}")
                })?;
            return Ok(database.get(literal));
        }
        Ok(cached) // Resolve the token using signed division.
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
        ) // Localise for the selected country.
    }

    /// Equipment-reference names shared by production and licences.
    pub(super) fn units_equipment_reference_name(
        &self,
        fields: &[Item],
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut version = "";
        let mut kind = b"".as_slice();
        let mut creator = b"---".as_slice(); // The reserved creator tag is the country-database default.
        // Repeated equipment bodies mutate the same reference.
        for equipment in fields.iter().filter(|field| field.keyword_is("equipment")) {
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
                "---" // Use exact raw tag spelling, without ROOT/FROM selection.
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
        ) // Use the verbatim version name, without LF.
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
        let name =
            self.units_current_name(&self.units_equipment()?.equipment[index].name, scopes)?;
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
        ) // Parse an integer count without scope evaluation.
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
        let name =
            self.units_current_name(&self.units_equipment()?.equipment[index].name, scopes)?;
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
        ) // Ignore old_prioritised; aliases share one operand.
    }

    fn units_variant_raw(
        &self,
        variant: &VariantDefinitionRow,
    ) -> Result<crate::variant_names::RawVariant, String> {
        let frame = self.context.text.frame();
        let empty = crate::variant_names::Tags::new(&[], &[], None);
        let raw = if variant.display_name.is_empty() {
            let tags = crate::variant_names::Tags::new(
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
            let raw = crate::variant_names::raw_variant(
                variant,
                frame.hour,
                &tags,
                frame
                    .complete::<tables::LexicalTokens>()
                    .map_err(|why| why.to_string())?,
            );
            if raw.name_tag.is_none() {
                raw.check("name tag")?;
                return Err("variant origin/creator naming tag is unavailable".into());
            }
            raw
        } else {
            crate::variant_names::raw_variant(variant, frame.hour, &empty, &[])
        };
        Ok(raw)
    }

    fn units_variant_name(&self, variant: &VariantDefinitionRow) -> Result<String, String> {
        crate::variant_names::variant_name(
            &self.units_variant_raw(variant)?,
            Some(&EquipmentTexts(
                self.context.localisation,
                self.context.text.recorded_language(),
            )),
        )?
        .ok_or_else(|| "shared equipment variant-name formatter cannot represent the name".into())
    }

    fn units_available_variants(
        &self,
        country: u32,
    ) -> Result<Vec<Option<&VariantDefinitionRow>>, String> {
        let world = self.context.world()?;
        if world.raw_country(country).is_none() {
            return Err(format!(
                "Countries lacks equipment roster country slot {country}"
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
            .collect()) // Preserve primary roster order.
    }

    pub(super) fn units_stockpile(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let amount = match field(fields, "amount") {
            Some(value) => self.scoped_value(scalar_operand(value)?, scopes)?,
            None => 0,
        };
        let amount = crate::triggers::fixed_round(amount) as i32;
        let producer = match field(fields, "producer") {
            Some(value) => self
                .evaluator()?
                .scoped_country(scalar_operand(value)?, scopes)?,
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
                    Scope::Country(country) | Scope::CountryLeader { country, .. }
                        if (country as i32) > 0 =>
                    {
                        country
                    }
                    _ => {
                        return Err("direct country-zero production roster is unavailable for concrete equipment without a positive producer; a state scope does not select its owner".into());
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
                    return Err("equipment ship-classifier archetype cycle".into());
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
        self.line(key, &args, scopes, true)
    }

    fn units_short_equipment_name(
        &self,
        name: &[u8],
        country: u32,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.units_short_name_for_tag(name, self.tag(country)?, scopes)
    }

    fn units_short_name_for_tag(
        &self,
        name: &[u8],
        tag: &str,
        scopes: Scopes,
    ) -> Result<String, String> {
        let name = parser::text(name).map_err(|why| why.to_string())?;
        let short = format!("{name}_short");
        let country_short = format!("{tag}_{short}");
        if self.context.localisation.get(&country_short).is_some() {
            self.localised(country_short.as_bytes(), &[], scopes, false)
        } else if self.context.localisation.get(&short).is_some() {
            self.localised(short.as_bytes(), &[], scopes, false)
        } else {
            self.units_country_equipment_name(name.as_bytes(), tag, scopes)
        }
    }

    fn units_enabled_module_slots(&self, index: usize) -> Result<bool, String> {
        crate::variant_names::enabled_module_slots(
            self.units_equipment()?,
            index,
            self.context.text.frame(),
        )
    }

    fn units_variant_order(
        &self,
        variant: &VariantDefinitionRow,
        index: usize,
    ) -> Result<crate::variant_names::VariantOrder, String> {
        crate::variant_names::variant_order(
            self.units_equipment()?,
            index,
            variant,
            self.context.text.frame(),
        )
    }

    fn units_pool_variant_order(
        &self,
        left: &VariantDefinitionRow,
        left_index: usize,
        right: &VariantDefinitionRow,
        right_index: usize,
    ) -> Result<Ordering, String> {
        if left.id == right.id {
            return Ok(Ordering::Equal);
        }
        // Merge equal version/type/creator/origin tuples,
        // even when their definition handles differ.
        let version = variant_version(left)?.cmp(&variant_version(right)?);
        if version != Ordering::Equal {
            return Ok(version);
        }
        let kind = (left_index as i32).cmp(&(right_index as i32));
        if kind != Ordering::Equal {
            return Ok(kind);
        }
        let world = self.context.world()?;
        let slot = |raw| -> Result<i32, String> {
            if raw == 0 {
                Ok(0)
            } else {
                world
                    .slot(raw)
                    .map(|slot| slot as i32)
                    .ok_or_else(|| format!("CountryLinks lacks equipment pool country {raw}"))
            }
        };
        let creator = slot(left.creator_link)?.cmp(&slot(right.creator_link)?);
        if creator != Ordering::Equal {
            return Ok(creator);
        }
        Ok(slot(left.origin_link)?.cmp(&slot(right.origin_link)?))
    }

    pub(super) fn equipment_list_description(
        &self,
        entries: &[Item],
        seller: u32,
        scopes: Scopes,
        separator: &str,
        key: &[u8],
    ) -> Result<String, String> {
        // Only resolved variants evaluate their amounts.
        let mut pool: Vec<(usize, usize, &VariantDefinitionRow, i64)> = Vec::new();
        for entry in entries.iter().filter(|entry| entry.keyword_is("equipment")) {
            let fields = children(entry);
            let index = self.units_equipment_type(fields, &["type"], scopes)?;
            let producer = field(fields, "producer")
                .map(|item| {
                    self.evaluator()?
                        .scoped_country(scalar_operand(item)?, scopes)
                })
                .transpose()?;
            let Some(index) = index else { continue };
            if producer == Some(None) {
                continue; // Invalid explicit producer.
            }
            let country = producer.flatten().unwrap_or(seller);
            let database = self.units_equipment()?;
            let kind = &database.equipment[index];
            let world = self.context.world()?;
            let mut candidates = Vec::new();
            let raw_name = field(fields, "variant_name")
                .map(scalar_operand)
                .transpose()?
                .unwrap_or_default();
            let raw_name = parser::c_string(raw_name);
            let named = !raw_name.is_empty(); // Remember whether the parsed name is nonempty.
            if !kind.is_archetype
                && [
                    b"token:".as_slice(),
                    b"mtth:",
                    b"mio:",
                    b"sp:",
                    b"constant:",
                ]
                .iter()
                .any(|prefix| raw_name.starts_with(prefix))
            {
                return Err(format!(
                    "variant_name {} uses unsupported stored text after a special prefix",
                    String::from_utf8_lossy(raw_name),
                ));
            }
            // Retain spelling after these two prefixes and before variable-name case
            // folding; the name itself is not evaluated.
            let name = raw_name
                .strip_prefix(b"var:")
                .or_else(|| raw_name.strip_prefix(b"temp_var:"))
                .unwrap_or(raw_name);
            let name = if kind.is_archetype {
                "" // Archetypes ignore the variant-name operand.
            } else {
                parser::text(name).map_err(|why| why.to_string())?
            };
            let mut roster = self.units_available_variants(country)?;
            if !kind.is_archetype && !named {
                roster.reverse();
            }
            for variant in roster {
                let variant = variant.ok_or(
                    "AvailableEquipmentVariants references an absent VariantDefinitions row",
                )?;
                let token = world
                    .token(variant.equipment_token)?
                    .ok_or("LexicalTokens lacks the available variant equipment token")?;
                let at = database
                    .get(token)
                    .ok_or("available variant equipment is absent from recorded content")?;
                if kind.is_archetype {
                    if database.equipment[at].archetype == Some(index) {
                        candidates.push((variant, at));
                    }
                } else if at == index {
                    if named {
                        let creator = producer.flatten().unwrap_or(0);
                        let creator_raw = if creator == 0 {
                            0
                        } else {
                            world
                                .raw_country(creator)
                                .ok_or("Countries lacks variant producer")?
                        };
                        if (variant.origin_link as i32) > 0
                            || (variant.creator_link != creator_raw
                                && (variant.creator_link == 0
                                    || creator == 0
                                    || world.slot(variant.creator_link) != Some(creator)))
                        {
                            continue;
                        }
                        let mut base = (*variant).clone();
                        base.append_variant_name = 0;
                        if self.units_variant_name(&base)? != name {
                            continue;
                        }
                        candidates.push((variant, at));
                        break; // Stop at the first exact creator/display-name match.
                    }
                    candidates.push((variant, at));
                    break; // Stop at the first match in reverse roster order.
                }
            }
            let selected = if kind.is_archetype && candidates.len() > 1 {
                let owner = world
                    .raw_country(country)
                    .ok_or("Countries lacks archetype roster owner")?
                    as i32;
                let mut ranked = candidates
                    .iter()
                    .map(|&(variant, at)| {
                        self.units_variant_order(variant, at)
                            .map(|order| (variant, at, order))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                ranked.sort_by(|left, right| {
                    crate::variant_names::compare_variants(owner, country as i32, left.2, right.2)
                }); // Use the shared stable country comparator.
                ranked.first().map(|&(variant, at, _)| (variant, at))
            } else {
                candidates.first().copied()
            };
            let Some((variant, at)) = selected else {
                continue;
            };
            let amount = field(fields, "amount")
                .map(|item| self.scoped_value(scalar_operand(item)?, scopes))
                .transpose()?
                .unwrap_or(0);
            let archetype = database.equipment[at]
                .archetype
                .ok_or("equipment pool has a null archetype")?;
            if let Some(start) = pool.iter().position(|(kind, _, _, _)| *kind == archetype) {
                let mut insert = start;
                let mut equal = false;
                for (offset, &(kind, prior_index, prior, _)) in pool[start..].iter().enumerate() {
                    if kind != archetype {
                        break;
                    }
                    insert = start + offset;
                    match self.units_pool_variant_order(prior, prior_index, variant, at)? {
                        Ordering::Less => insert += 1,
                        Ordering::Equal => {
                            equal = true;
                            break;
                        }
                        Ordering::Greater => break,
                    }
                } // Stop at the first range boundary.
                if equal {
                    pool[insert].3 = pool[insert]
                        .3
                        .wrapping_add(amount)
                        .clamp(-0x53e2d620fb40, 0x53e2d620fb40);
                } else {
                    pool.insert(
                        insert,
                        (
                            archetype,
                            at,
                            variant,
                            amount.clamp(-0x53e2d620fb40, 0x53e2d620fb40),
                        ),
                    );
                }
            } else {
                // New archetypes append their first item.
                pool.push((
                    archetype,
                    at,
                    variant,
                    amount.clamp(-0x53e2d620fb40, 0x53e2d620fb40),
                ));
            }
        }
        let mut lines = Vec::new();
        for (archetype, _, variant, quantity) in pool {
            let amount = quantity.div_euclid(100_000) as i32;
            if amount <= 0 {
                continue; // Floor, narrow to i32 and retain only positive amounts.
            }
            let database = self.units_equipment()?;
            let kind = self.units_short_equipment_name(
                &database.equipment[archetype].name,
                self.context.perspective,
                scopes,
            )?;
            let raw = self.units_variant_raw(variant)?;
            let name = match raw.own_name(Some(&EquipmentTexts(
                self.context.localisation,
                self.context.text.recorded_language(),
            )))? {
                Some(name) => name,
                None => {
                    raw.check("token")?;
                    raw.check("name tag")?;
                    self.units_short_name_for_tag(
                        raw.token.as_bytes(),
                        raw.name_tag
                            .as_deref()
                            .ok_or("variant naming tag is unavailable")?,
                        scopes,
                    )?
                }
            }; // Use the short display name without a suffix.
            lines.push(self.localised(
                key,
                &[
                    ("EQUIPMENT", Argument::Text(&name)),
                    ("TYPE", Argument::Text(&kind)),
                    ("AMOUNT", Argument::Int32(amount)),
                ],
                scopes,
                false,
            )?);
        }
        Ok(lines.join(separator))
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
            return Err(format!(
                "native equipment role key {key} is not defined by the recorded content stack"
            ));
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
            Scope::Country(country) | Scope::CountryLeader { country, .. }
                if (country as i32) > 0 =>
            {
                country
            }
            Scope::State(NULL_STATE) => return Ok(String::new()),
            Scope::State(state) => {
                let row = self.valid_state(state)?.ok_or_else(|| {
                    "an invalid nonzero state owner cannot resolve a positive equipment type"
                        .to_string()
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
            let mut found = false;
            for variant in self.units_available_variants(country)? {
                let variant = variant.ok_or(
                    "AvailableEquipmentVariants references an absent VariantDefinitions row",
                )?;
                let token = world
                    .token(variant.equipment_token)?
                    .ok_or("LexicalTokens lacks the available variant equipment token")?;
                if database.get(token) != Some(kind.family) || variant_version(variant)? != 0 {
                    continue;
                }
                let owner = world
                    .raw_country(country)
                    .ok_or("Countries lacks equipment-variant owner")?;
                let same_creator = variant.creator_link == owner
                    || (variant.creator_link != 0
                        && world
                            .slot(variant.creator_link)
                            .ok_or("CountryLinks lacks equipment-variant creator")?
                            == country);
                if same_creator && (variant.origin_link as i32) < 1 {
                    found = true;
                    break; // Stop at the first family/version-zero/creator/origin match.
                }
            }
            !found
        };
        let modular = self.units_enabled_module_slots(index)?;
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
                        } // An unknown module keeps the preceding slot value.
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
        self.localised(key, &args, scopes, true) // No added LF.
    }

    pub(super) fn units_subsidy(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
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
        for body in fields
            .iter()
            .filter(|field| field.keyword_is("seller_tags"))
        {
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
                return Err("seller_trigger fallback needs a trigger-template description; the shared trigger-description helper is unavailable".into());
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
        ) // Positive sellers retain order and duplicates.
    }

    pub(super) fn units_unit_bonus(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        self.units_bonus_description(children(item), scopes)
    }

    fn units_current_name(&self, name: &[u8], scopes: Scopes) -> Result<String, String> {
        self.units_country_equipment_name(name, self.tag(self.context.perspective)?, scopes)
        // Localise for the current country.
    }

    pub(super) fn units_manpower(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let amount =
            crate::triggers::fixed_round(self.scoped_value(scalar_operand(item)?, scopes)?) as i32;
        let (key, name) = match scopes.this {
            Scope::State(state) if (state as i32) > 0 => (
                b"EFFECT_MANPOWER_ADD_STATE".as_slice(),
                self.state_name(state)?,
            ),
            Scope::Country(country) | Scope::CountryLeader { country, .. } => (
                b"EFFECT_MANPOWER_ADD_COUNTRY".as_slice(),
                self.country_name(country, false)?,
            ),
            Scope::State(_) => (
                b"EFFECT_MANPOWER_ADD_COUNTRY".as_slice(),
                self.country_name(0, false)?,
            ),
            _ => {
                return Err("scope state/country fields unavailable".into());
            }
        };
        self.line(
            key,
            &[
                ("NAME", Argument::Text(&name)),
                ("VAL", Argument::Int32(amount)),
            ],
            scopes,
            true,
        ) // Round the value.
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
            }
            TemplateDescription::Lock => {
                if parser::read_bool_field(fields, "is_locked", false) {
                    b"EFFECT_DISABLE_TEMPLATE_EDITTING".as_slice()
                } else {
                    b"EFFECT_ENABLE_TEMPLATE_EDITTING".as_slice()
                }
            } // The authored key spells EDITTING.
            TemplateDescription::ClearCap => b"EFFECT_CLEAR_TEMPLATE_DIVISION_CAP".as_slice(),
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
                ); // Truncate before narrowing rather than rounding.
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
                );
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
        )
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
            self.line(b"EFFECT_TRANSFER_NAVY", &args, scopes, true)
        } else {
            self.localised(b"EFFECT_TRANSFER_UNITS", &args, scopes, true) // No LF.
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
            )
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
        }
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
        Ok(text) // No scope expansion.
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
        self.localised(key, &args, scopes, true) // Priority is plane/damage/strength/organisation; no LF.
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
        )
    }

    pub(super) fn units_create_ship(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let country = match field(children(item), "creator") {
            Some(creator) => self
                .evaluator()?
                .scoped_country(scalar_operand(creator)?, scopes)?,
            None => match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. }
                    if (country as i32) > 0 =>
                {
                    Some(country)
                }
                _ => None,
            }, // Use the carried scope country directly rather than the state owner.
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
        ) // No LF.
    }

    pub(super) fn units_create(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        if item.value.scalar().is_some() {
            return Err("the scalar create_unit body is rejected and retains constructor defaults; that diagnostic/default form is not described".into());
        }
        let cursor = scopes.cursor.ok_or("create_unit needs its caller scope")?;
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
            graph.scopes.push(copied); // Copy the scope and its members.
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
                None => Ok(0), // The selector fallback defaults to zero.
            }
        };
        let location = self.units_create_location(item, scopes, owner()?)?;
        let country = owner()?; // Reevaluate the owner after the copied seed advances.
        let owner = self.context.text.country_adjective(country).map_err(|why| {
            if country == 0 {
                format!("owner selector is reserved country zero; its adjective/name input is unavailable: {why}")
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
        ) // Rounded count, without LF.
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
        }
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
            return Err(format!(
                "state {state} has no province candidates; location selection divides by the state's province count"
            ));
        }
        let cursor = scopes
            .cursor
            .ok_or("create_unit location needs its copied scope cursor")?;
        let copied = self.graph.borrow().scopes[cursor];
        let start = copied.random_draw()? as usize % provinces.len();
        self.graph.borrow_mut().scopes[cursor].random_seed = copied.random_seed.wrapping_add(1);
        // Change only the copied seed, preserving the caller's random counter.
        let impassable = parser::parse(b"impassable=yes")
            .map_err(|why| why.to_string())?
            .items;
        let mut blocked = BTreeMap::new();
        let mut controller_scores = BTreeMap::new();
        let country = |raw| {
            world
                .slot(raw)
                .ok_or_else(|| format!("CountryLinks lacks province controller {raw}"))
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
            Ok(false) // Installed relation slots, without a date or cancellation filter.
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
            Ok(false) // Direct installed subjects, without a date or cancellation filter.
        };
        let mut admitted =
            |province: &replay::rows::ProvinceRow, ignore_occupants| -> Result<bool, String> {
                if !world.state_provinces.contains_key(&province.state) {
                    return Err(format!(
                        "province {} state {} is not defined by the recorded content stack",
                        province.id, province.state
                    ));
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
                    .ok_or_else(|| format!("ProvinceUnitOrder lacks province {}", province.id))?;
                Ok(occupants.mixed.len() == occupants.naval.len())
                // Mixed unit counts include land, naval and railway units.
            };
        for ignore_occupants in [score.is_some(), true] {
            if let Some(province) = world.provinces.get(&priority)
                && admitted(province, ignore_occupants)?
            {
                return Ok(Some(province.state));
            }
            let mut best = -0x53E2_D620_FB40_i64; // Initial best score.
            let mut found = false;
            for index in 0..provinces.len() {
                let id = provinces[(start + index) % provinces.len()];
                let province = world
                    .provinces
                    .get(&id)
                    .ok_or_else(|| format!("Provinces lacks state {state} candidate {id}"))?;
                if !admitted(province, ignore_occupants)? {
                    continue;
                }
                let controller = country(province.controller)?;
                let value = if let Some(score) = score {
                    match controller_scores.get(&controller) {
                        Some(&value) if value != -0x53E2_D620_FB40_i64 => value,
                        _ => {
                            let cursor = scopes
                                .cursor
                                .ok_or("country_score needs the caller's event graph cursor")?;
                            let at = self.graph.borrow().scopes.len();
                            self.graph
                                .borrow_mut()
                                .scopes
                                .push(crate::scripted_text::EventScope {
                                    prev: Link::Scope(cursor),
                                    ..crate::scripted_text::EventScope::of(EventObject::Country(
                                        controller,
                                    ))
                                }); // Fresh ROOT/FROM with caller PREV.
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
            // Every score query runs. Only STATE reaches the unit description;
            // ties in the no-score form cannot change the description.
            if found {
                return Ok(Some(state));
            }
            if !enemy || ignore_occupants {
                break;
            }
        }
        Ok(None) // A missing location still describes the unit.
    }

    pub(super) fn units_unlock(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let name =
            parser::text(parser::c_string(scalar_operand(item)?)).map_err(|why| why.to_string())?;
        let units = self
            .context
            .content
            .key_names
            .and_then(|names| names.sub_units.as_ref())
            .ok_or_else(|| self.content_gap("effect key names"))?;
        let index = match units.get(name) {
            installed_content::unit_leader::SubUnit::Index(index) => index,
            installed_content::unit_leader::SubUnit::Unknown => {
                return Err(format!(
                    "sub-unit {name} is not defined by the recorded content stack"
                ));
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
        // No expansion or LF.
    }

    pub(super) fn delete_template(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let fields = children(item);
        let value = item
            .value
            .scalar()
            .or_else(|| {
                field(fields, "division_template").and_then(|item| item.value.read_string())
            })
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
        )
    }
}

impl Renderer<'_, '_, '_> {
    pub(super) fn units_equipment_bonus(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let mut entries = Vec::new();
        // Append every bonus body; name, project and prefix
        // are unused by the description path.
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
                    } // Combine stat values; retain zero entries.
                }
                let mut lines = Vec::new();
                for (index, value) in stats {
                    let (_, label_key, positive_good) =
                        installed_content::equipment::stat_metadata()[index];
                    let key = label_key.as_bytes();
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
                        }; // Select the positive-good polarity for the stat.
                    let value = self.localised(
                        difference_key,
                        &[("VALUE", Argument::Fixed5(value))],
                        scopes,
                        false,
                    )?;
                    lines.push(format!("    {label}{value}"));
                }
                description.push_str(&lines.join("\n"));
                entries.push(description);
            }
        }
        Ok(entries.join("\n")) // No added terminal LF.
    }
}
