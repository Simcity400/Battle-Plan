use super::*;
use paradox_text::{Entry, Scalar};

impl AuthoredEquipmentCatalog {
    pub fn resolve_generated(mut self) -> EquipmentResult<Self> {
        self.definitions.clear();
        self.modules.clear();
        self.definition_order.clear();
        self.generation_resolved = true;
        self.generation_dlcs = self.active_dlcs.clone();
        let mut sources = self.sources.clone();
        // 1409ed0b0 parses upgrades, then modules, then the equipment directory.
        sources.sort_by_key(|source| {
            let path = source.source.normalized_game_relative_path.as_str();
            if path.starts_with("common/units/equipment/upgrades/") { 0 }
            else if path.starts_with("common/units/equipment/modules/") { 1 }
            else { 2 }
        });
        for source in sources {
            let mut expanded = source.authored.clone();
            paradox_text::expand_file_scoped_variables(&mut expanded).map_err(|error| {
                EquipmentDefinitionError::VariableExpansion {
                    source: source.source.normalized_game_relative_path.clone(), detail: error.to_string(),
                }
            })?;
            for (raw, entry) in source.authored.fields().iter().zip(expanded.fields()) {
                if entry.key_matches("equipments") || entry.key_matches("equipment_modules") {
                    self.resolve_container(&source.source, raw, entry)?;
                } else if entry.key_matches("duplicate_archetypes") {
                    self.resolve_duplicate_block(&source.source, raw, entry)?;
                }
            }
        }
        Ok(self)
    }

    fn resolve_container(&mut self, source: &UsedContentKey, raw: &Entry, entry: &Entry) -> EquipmentResult<()> {
        let fields = entry.value.as_object().ok_or_else(|| malformed("equipments", "container"))?;
        if let Some(limit) = entry.value.first_value_ci("limit") {
            if !self.content_limit("equipments", limit)? { return Ok(()) }
        }
        let modules = entry.key_matches("equipment_modules");
        for (raw, entry) in raw.value.fields().iter().zip(fields) {
            let token = entry.key.as_deref().ok_or_else(|| malformed("equipments", "token"))?;
            if token.starts_with('@') || entry.key_matches("limit") { continue }
            if entry.value.as_object().is_none() { return Err(malformed(token, "definition")) }
            let mut definition = AuthoredEquipmentDefinition {
                token: token.into(), source: source.clone(), authored: raw.value.clone(),
                expanded: entry.value.clone(), container_limit: None, generation: None, generated_type_mask: None,
            };
            if !modules {
                for field in VERIFIED_STATS {
                    let raw = self.stat_before_clone(&definition, field)?;
                    set_scalar(&mut definition.expanded, field, &format_fixed5(raw));
                }
            }
            if modules {
                self.modules.insert(token.into(), vec![definition]);
            } else {
                if !self.definitions.contains_key(token) { self.definition_order.push(token.into()) }
                // 1409e5170 resets an existing equipment before parsing its replacement.
                self.definitions.insert(token.into(), vec![definition]);
            }
        }
        Ok(())
    }

    fn resolve_duplicate_block(&mut self, source: &UsedContentKey, raw: &Entry, entry: &Entry) -> EquipmentResult<()> {
        let entries = entry.value.as_object().ok_or_else(|| malformed("duplicate_archetypes", "container"))?;
        let mut recipes: Vec<AuthoredEquipmentDefinition> = Vec::new();
        for (raw, entry) in raw.value.fields().iter().zip(entries) {
            let token = entry.key.as_deref().ok_or_else(|| malformed("duplicate_archetypes", "token"))?;
            if token.starts_with('@') { continue }
            let fields = entry.value.as_object().ok_or_else(|| malformed(token, "duplicate_archetypes"))?;
            if let Some(recipe) = recipes.iter_mut().find(|recipe| recipe.token == token) {
                // Repeated recipe names parse into the same object, without a constructor reset.
                object_mut(&mut recipe.expanded).extend_from_slice(fields);
                object_mut(&mut recipe.authored).extend_from_slice(raw.value.fields());
            } else {
                recipes.push(AuthoredEquipmentDefinition {
                    token: token.into(), source: source.clone(), authored: raw.value.clone(),
                    expanded: entry.value.clone(), container_limit: None, generation: None, generated_type_mask: None,
                });
            }
        }
        for recipe in recipes { self.clone_archetype(&recipe)? }
        Ok(())
    }

    fn clone_archetype(&mut self, recipe: &AuthoredEquipmentDefinition) -> EquipmentResult<()> {
        let base = recipe.scalar("archetype")?.ok_or_else(|| EquipmentDefinitionError::MissingField {
            token: recipe.token.clone(), field: "archetype".into(),
        })?;
        let base_definition = self.definition(base)?;
        if base_definition.boolean("is_archetype")? != Some(true) {
            return Err(EquipmentDefinitionError::NotArchetype(base.into()));
        }
        let only_base = recipe.boolean("only_duplicate_archetype")?.unwrap_or(false);
        let clear_slots = match recipe.scalar("module_slots")? {
            None => false, Some("none") => true,
            Some(_) => return Err(malformed(&recipe.token, "module_slots")),
        };
        let mut candidates = Vec::new();
        for token in &self.definition_order {
            let definition = self.definition(token)?;
            if definition.generation.is_some() { continue }
            if only_base && token != base { continue }
            if token != base && self.resolve_archetype(token, &mut Vec::new())?.as_ref().map(|x| x.token.as_str()) != Some(base) {
                continue;
            }
            // Validate cycles before applying the engine's hierarchy comparator.
            self.hierarchy_profile(token)?;
            candidates.push(token.clone());
        }
        // Stable hierarchy ordering: roots/ancestors precede their descendants.
        for index in 1..candidates.len() {
            let mut position = index;
            while position > 0 && self.generation_precedes(&candidates[position], &candidates[position - 1], base)? {
                candidates.swap(position, position - 1);
                position -= 1;
            }
        }
        let mut remapped = BTreeMap::<String, String>::new();
        for original_token in candidates {
            let original = self.definition(&original_token)?.clone();
            let common = original_token.bytes().zip(base.bytes()).take_while(|(a,b)| a == b).count();
            let mut name = recipe.token.as_bytes().to_vec();
            name.extend_from_slice(&original_token.as_bytes()[common..]);
            let token = String::from_utf8(name).map_err(|_| malformed(&recipe.token, "generated_token_utf8"))?;
            if self.definitions.contains_key(&token) {
                return Err(EquipmentDefinitionError::UnsupportedAuthored {
                    token, field: "duplicate_archetypes.existing_definition_mode".into(),
                });
            }
            let old_archetype = self.resolve_archetype(&original_token, &mut Vec::new())?.map(|x| x.token);
            let archetype = old_archetype.as_ref().and_then(|x| remapped.get(x)).cloned().unwrap_or_else(|| token.clone());
            let old_parent = original.scalar("parent")?.unwrap_or(&original_token);
            let parent = remapped.get(old_parent).cloned().unwrap_or_else(|| token.clone());
            let mut generated = original.clone();
            generated.token = token.clone();
            generated.source = recipe.source.clone();
            generated.generation = Some(EquipmentGeneration {
                recipe: recipe.identity(), original: original.identity(), authored_recipe: recipe.authored.clone(),
            });
            for field in ["alias", "type", "variant_name", "derived_variant_name", "archetype", "parent", "is_archetype"] {
                remove_field(&mut generated.expanded, field);
            }
            set_scalar(&mut generated.expanded, "parent", &parent);
            set_scalar(&mut generated.expanded, "archetype", &archetype);
            set_scalar(&mut generated.expanded, "is_archetype", if archetype == token { "yes" } else { "no" });
            let mut mask = 0;
            for value in recipe.expanded.values_ci("type") {
                mask = recipe.category_value(value, "type", true)?;
            }
            generated.generated_type_mask = Some(mask);
            if clear_slots {
                remove_field(&mut generated.expanded, "default_modules");
                set_value(&mut generated.expanded, "module_slots", Value::Object(Vec::new()));
            }
            for field in ["sprite", "picture"] {
                if let Some(value) = recipe.scalar(field)?.filter(|x| !x.is_empty()) {
                    set_scalar(&mut generated.expanded, field, value);
                }
            }
            for field in ["substitute", "air_map_icon_frame", "interface_overview_category_index"] {
                if let Some(value) = recipe.scalar(field)? { set_scalar(&mut generated.expanded, field, value) }
            }
            for value in recipe.expanded.values_ci("forbid_mission_type") {
                object_mut(&mut generated.expanded).push(Entry { key: Some("forbid_mission_type".into()), value: value.clone(), line: 0, column: 0 });
            }
            self.apply_for_each(recipe, &original, &mut generated, archetype == token)?;
            self.definition_order.push(token.clone());
            self.definitions.insert(token.clone(), vec![generated]);
            remapped.insert(original_token, token);
        }
        Ok(())
    }

    fn generation_precedes(&self, left: &str, right: &str, base: &str) -> EquipmentResult<bool> {
        let mut left = Some(left.to_owned());
        let mut right = Some(right.to_owned());
        for _ in 0..64 {
            if left == right { return Ok(false) }
            if left.is_none() { return Ok(true) }
            if right.is_none() { return Ok(false) }
            if left.as_deref() == Some(base) { return Ok(true) }
            if right.as_deref() == Some(base) { return Ok(false) }
            left = self.generation_ancestor(left.as_deref().unwrap())?;
            right = self.generation_ancestor(right.as_deref().unwrap())?;
        }
        Err(EquipmentDefinitionError::RelationshipCycle(vec![left.unwrap_or_default(), right.unwrap_or_default()]))
    }

    fn generation_ancestor(&self, token: &str) -> EquipmentResult<Option<String>> {
        let definition = self.definition(token)?;
        if let Some(parent) = definition.scalar("parent")?.filter(|x| *x != token) { return Ok(Some(parent.into())) }
        Ok(self.resolve_archetype(token, &mut Vec::new())?.filter(|x| x.token != token).map(|x| x.token))
    }

    fn apply_for_each(&self, recipe: &AuthoredEquipmentDefinition, original: &AuthoredEquipmentDefinition,
        generated: &mut AuthoredEquipmentDefinition, is_archetype: bool) -> EquipmentResult<()>
    {
        let mut operations = BTreeMap::<String, (String, i64)>::new();
        let mut name_transform = None;
        for value in recipe.expanded.values_ci("for_each") {
            for entry in value.as_object().ok_or_else(|| malformed(&recipe.token, "for_each"))? {
                let field = entry.key.as_deref().ok_or_else(|| malformed(&recipe.token, "for_each.field"))?;
                if field == "variant_name" {
                    let [transform] = entry.value.fields() else { return Err(malformed(&recipe.token, "for_each.variant_name")) };
                    if !transform.key_matches("find_and_replace") { return Err(malformed(&recipe.token, "for_each.variant_name")) }
                    let [find, replace] = transform.value.fields() else { return Err(malformed(&recipe.token, "find_and_replace")) };
                    if find.key.is_some() || replace.key.is_some() { return Err(malformed(&recipe.token, "find_and_replace")) }
                    name_transform = Some((find.value.as_scalar().ok_or_else(|| malformed(&recipe.token, "find_and_replace"))?,
                        replace.value.as_scalar().ok_or_else(|| malformed(&recipe.token, "find_and_replace"))?));
                    continue;
                }
                if !verified_stat(field) {
                    return Err(EquipmentDefinitionError::UnsupportedAuthored { token: recipe.token.clone(), field: format!("for_each.{field}.lexical_stat") });
                }
                let [operation] = entry.value.fields() else { return Err(malformed(&recipe.token, &format!("for_each.{field}"))) };
                let kind = operation.key.as_deref().filter(|x| matches!(*x, "set" | "multiply" | "add"))
                    .ok_or_else(|| malformed(&recipe.token, &format!("for_each.{field}.operation")))?;
                let operand = operation.value.as_scalar().and_then(parse_fixed5).ok_or_else(|| malformed(&recipe.token, field))?;
                operations.insert(field.into(), (kind.into(), operand));
            }
        }
        for (field, (kind, operand)) in operations {
            let raw = if kind == "set" { operand } else {
                let original_value = self.stat_before_clone(original, &field)?;
                if kind == "add" { original_value.wrapping_add(operand) }
                else { original_value.wrapping_mul(operand) / 100_000 }
            };
            set_scalar(&mut generated.expanded, &field, &format_fixed5(raw));
        }
        if !is_archetype {
            if let Some((find, replace)) = name_transform {
                let name = generated.token.replacen(find, replace, 1);
                set_scalar(&mut generated.expanded, "variant_name", &name);
            }
            let mut explicit_name = None;
            for mappings in recipe.expanded.values_ci("variant_name") {
                for mapping in mappings.as_object().ok_or_else(|| malformed(&recipe.token, "variant_name"))? {
                    let name = mapping.value.as_scalar().ok_or_else(|| malformed(&recipe.token, "variant_name"))?;
                    if mapping.key.is_none() { return Err(malformed(&recipe.token, "variant_name")) }
                    if explicit_name.is_none() && mapping.key.as_deref() == Some(generated.token.as_str()) {
                        explicit_name = Some(name);
                    }
                }
            }
            if let Some(name) = explicit_name { set_scalar(&mut generated.expanded, "variant_name", name) }
        }
        Ok(())
    }

    fn stat_before_clone(&self, original: &AuthoredEquipmentDefinition, field: &str) -> EquipmentResult<i64> {
        // 140c82f90 initializes each admitted stat slot to zero; only explicit
        // archetype parsing copies the stat block (140c89cc0 -> 140c86ec0).
        let mut raw = 0;
        for entry in original.expanded.fields() {
            if entry.key_matches("archetype") {
                let token = entry.value.as_scalar().ok_or_else(|| malformed(&original.token, "archetype"))?;
                if token == original.token { continue }
                let archetype = self.definition(token)?;
                let value = archetype.scalar(field)?.ok_or_else(|| malformed(token, field))?;
                raw = parse_fixed5(value).ok_or_else(|| malformed(token, field))?;
            } else if entry.key_matches(field) {
                raw = entry.value.as_scalar().and_then(parse_fixed5).ok_or_else(|| malformed(&original.token, field))?;
            }
        }
        Ok(raw)
    }
}

const VERIFIED_STATS: &[&str] = &["recon", "air_range", "air_agility", "air_superiority", "air_bombing",
    "anti_air_attack", "railway_gun_attack", "railway_gun_annex_ratio", "submarine_carrier_size", "armor_value"];
fn verified_stat(field: &str) -> bool { VERIFIED_STATS.contains(&field) }

fn malformed(token: &str, field: &str) -> EquipmentDefinitionError {
    EquipmentDefinitionError::MalformedField { token: token.into(), field: field.into() }
}
fn object_mut(value: &mut Value) -> &mut Vec<Entry> {
    match value { Value::Object(entries) => entries, _ => unreachable!("validated authored object") }
}
fn remove_field(value: &mut Value, field: &str) { object_mut(value).retain(|entry| !entry.key_matches(field)); }
fn set_value(value: &mut Value, field: &str, replacement: Value) {
    remove_field(value, field);
    object_mut(value).push(Entry { key: Some(field.into()), value: replacement, line: 0, column: 0 });
}
fn set_scalar(value: &mut Value, field: &str, replacement: &str) {
    set_value(value, field, Value::Scalar(Scalar::Bare(replacement.into())));
}
fn parse_fixed5(text: &str) -> Option<i64> {
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix('-').or_else(|| text.strip_prefix('+')).unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty() || !whole.bytes().chain(fraction.bytes()).all(|x| x.is_ascii_digit()) { return None }
    let whole = whole.parse::<u64>().ok()?;
    let bound = i64::MAX as u64 + u64::from(negative);
    if whole > bound { return None }
    let mut part = 0i64;
    for index in 0..5 { part = part * 10 + fraction.as_bytes().get(index).map_or(0, |x| i64::from(x - b'0')); }
    let raw = (whole as i64).wrapping_mul(100_000).wrapping_add(part);
    Some(if negative { raw.wrapping_neg() } else { raw })
}
fn format_fixed5(raw: i64) -> String {
    let magnitude = raw.unsigned_abs();
    format!("{}{}.{:05}", if raw < 0 { "-" } else { "" }, magnitude / 100_000, magnitude % 100_000)
}
