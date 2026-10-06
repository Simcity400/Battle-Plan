//! Presentation fields from the same native technology reader as equipment links.
use super::*;
use crate::tokens::Token;

#[derive(Clone, Debug, Default)]
pub struct Definition {
    pub id: Vec<u8>,
    pub parent: Vec<u8>,
    pub small_layout: bool,
    pub equipment: Vec<Item>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub definitions: BTreeMap<Token, Definition>,
    pub categories: BTreeMap<Token, Vec<u8>>,
    /// Full categories/aliases differ from KeyNames' registration-only roster.
    pub equipment: crate::equipment::Database,
    pub notes: Vec<String>,
    pub refusal: Option<String>,
}

impl Catalog {
    pub fn read(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Self> {
        // Reuse KeyNames' sub-unit/category/terrain rosters; do not reread them.
        let sub_units = names.sub_units.as_ref().ok_or_else(|| invalid("technology description sub-unit roster absent"))?;
        let registry = units::Registry {
            units: sub_units.registered().map(|name| name.to_ascii_lowercase()).collect(),
            categories: sub_units.categories().clone(),
            terrains: names.effects.terrains.clone(),
            equipment: names.effects.equipment.clone(),
        };
        let mut catalog = Self {
            equipment: crate::equipment::Database::load_complete(layers)?,
            ..Self::default()
        };
        // 140AC1BA0: direct directory, mode 0. 140ABCFC0/140AC3C70 are
        // shared with the equipment projection, including @variable consumption.
        for path in layers.directory_files("common/technologies", ".txt")? {
            let (read, _, _) = read_templates(&layers.read(&path)?, &registry, names);
            catalog.notes.extend(read.misreads.iter().map(|note| format!("{path}: {note}")));
            if let Some(why) = &read.unsettled {
                catalog.refusal = Some(format!("{path}: technology description unread: {why}"));
            }
            catalog.append(&read.items);
        }
        Ok(catalog)
    }

    fn append(&mut self, roots: &[Item]) {
        for root in roots {
            if root.keyword_is("technology_categories") {
                // 140ABCFC0 -> 140ABB190: anonymous category names; lookup
                // 140AC12B0 returns the first valid object with the token.
                for entry in root.value.items().unwrap_or_default() {
                    if let Some(name) = entry.value.scalar() {
                        self.categories.entry(Token::c_str(name)).or_insert_with(|| parser::c_string(name).to_vec());
                    }
                }
            }
            if !root.keyword_is("technologies") { continue; }
            for item in root.value.items().unwrap_or_default() {
                let id = parser::c_string(item.key.as_deref().unwrap_or_default());
                let identity = Token::c_str(id);
                if self.definitions.contains_key(&identity) { continue; }
                let mut definition = Definition { id: id.to_vec(), ..Definition::default() };
                // +0x3B0 parent, +0x40A force_use_small_tech_layout; constructor
                // 140ABBF20 clears the flag. 140AC3C70 keeps the equipment list.
                for field in item.value.items().unwrap_or_default() {
                    if field.keyword_is("parent") {
                        definition.parent = field.value.read_string().unwrap_or_default().to_vec();
                    } else if field.keyword_is("force_use_small_tech_layout") {
                        definition.small_layout = field.value_keyword().as_deref() == Some("yes");
                    } else if field.keyword_is("enable_equipments") {
                        definition.equipment.extend(field.value.items().unwrap_or_default().iter().cloned());
                    }
                }
                // 140ABCFC0 allocates every repeated definition afresh;
                // 140AB9020/140AC1A80 return the first registered token.
                self.definitions.insert(identity, definition);
            }
        }
    }

    pub fn get(&self, operand: &[u8]) -> Result<Option<&Definition>, String> {
        if let Some(why) = &self.refusal { return Err(why.clone()); }
        Ok(self.definitions.get(&Token::c_str(operand)))
    }

    pub fn category(&self, operand: &[u8]) -> Result<Option<&[u8]>, String> {
        if let Some(why) = &self.refusal { return Err(why.clone()); }
        Ok(self.categories.get(&Token::c_str(operand)).map(Vec::as_slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn technology_description_names_use_native_parent_and_layout_flag() {
        let (read, _, _) = read_templates(b"@title=mod_parent technology_categories={mod_category} technologies={mod_tech={parent=@title force_use_small_tech_layout=yes enable_equipments={mod_equipment}}}", &units::Registry::default(), &KeyNames::default());
        assert!(read.unsettled.is_none(), "{read:?}");
        let mut catalog = Catalog::default();
        catalog.append(&read.items);
        let definition = catalog.get(b"MOD_TECH").unwrap().unwrap();
        // parent uses the current-token reader, not the resolved value slot.
        assert_eq!(definition.parent, b"@title");
        assert!(definition.small_layout);
        assert_eq!(catalog.category(b"MOD_CATEGORY").unwrap(), Some(&b"mod_category"[..]));
        assert_eq!(definition.equipment[0].value.scalar(), Some(&b"mod_equipment"[..]));
    }
}
