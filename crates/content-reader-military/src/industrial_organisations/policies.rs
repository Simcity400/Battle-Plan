
use content_reader_script::{key_names::KeyNames, modifiers, native::{self, Shape}};
use crate::mio_traits::{Bonus, is_production_stat};
use content_reader_parser::parser::{Item, c_string};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub token: Vec<u8>,
    pub name: Vec<u8>,
    pub icon: Vec<u8>,
    pub visible: Vec<Item>,
    pub available: Vec<Item>,
    pub groups: Vec<(Vec<u8>, Vec<Bonus>)>,
    pub organisation_modifiers: modifiers::Modifiers,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub notes: Vec<String>,
    rows: BTreeMap<Vec<u8>, Policy>,
    refusals: BTreeMap<Vec<u8>, String>,
    unread: Vec<String>,
}

#[derive(Clone, Copy)]
enum Reader {
    File,
    Policy,
    Groups,
    Bonus,
    Modifier(modifiers::Node),
}

impl Catalog {
    pub fn read_file(&mut self, input: &[u8], names: &KeyNames) -> Result<(), String> {
        let mut seen: BTreeSet<_> = self
            .rows
            .keys()
            .chain(self.refusals.keys())
            .cloned()
            .collect();
        let mut current = None;
        let mut duplicate_notes = Vec::new();
        let registry = |key: &[u8]| modifiers::registered(key, Some(names));
        let mut undecided = Vec::new();
        let read = native::read(
            input,
            native::Mode::KeepMark,
            native::Top::Persistent(Reader::File),
            Some(names),
            &mut |reader, field| match reader {
                Reader::File => {
                    let name = field.key.c_str().to_ascii_lowercase();
                    if seen.insert(name.clone()) {
                        current = Some(name);
                        Shape::Object(Reader::Policy)
                    } else {
                        duplicate_notes.push(format!(
                            "MIO policy {}: duplicate definition skipped (count: 1)",
                            String::from_utf8_lossy(&name)
                        ));
                        current = None;
                        Shape::Skip
                    }
                }
                Reader::Policy => match field.keyword().as_deref() {
                    Some(b"name" | b"icon" | b"cost" | b"cooldown") => Shape::One,
                    Some(b"allowed" | b"available" | b"visible") => Shape::Trigger,
                    Some(b"on_add" | b"on_remove") => Shape::Effect,
                    Some(b"ai_will_do") => native::mean_time_to_happen(field),
                    Some(b"equipment_bonus" | b"production_bonus") => {
                        Shape::Object(Reader::Groups)
                    }
                    Some(b"organization_modifier") => {
                        Shape::Object(Reader::Modifier(modifiers::Node::Modifier))
                    }
                    _ => Shape::Skip,
                },
                Reader::Groups => Shape::Object(Reader::Bonus),
                Reader::Bonus => {
                    if field.keyword().is_some_and(|name| {
                        content_reader_script::equipment_stats::stat_index(&name).is_some()
                            || std::str::from_utf8(&name)
                                .ok()
                                .is_some_and(is_production_stat)
                    }) {
                        Shape::One
                    } else {
                        Shape::Skip
                    }
                }
                Reader::Modifier(node) => {
                    modifiers::shape(node, field, &registry, &mut undecided).map(Reader::Modifier)
                }
            },
        );
        self.notes.extend(duplicate_notes);
        self.unread
            .extend(read.unattributed_misreads.iter().map(ToString::to_string));
        let undecided: Vec<_> = undecided.into_iter().map(|why| why.to_string()).collect();
        for (item, notes) in read.items.into_iter().zip(read.item_misreads) {
            let token = c_string(item.key.as_deref().unwrap_or_default()).to_vec();
            let name = token.to_ascii_lowercase();
            if self.rows.contains_key(&name) || self.refusals.contains_key(&name) {
                continue;
            }
            let why: Vec<_> = notes
                .iter()
                .map(ToString::to_string)
                .chain(self.unread.iter().cloned())
                .chain(undecided.iter().cloned())
                .collect();
            if !why.is_empty() {
                let why = why.join("; ");
                self.notes.push(format!(
                    "MIO policy {}: {why} (count: 1)",
                    String::from_utf8_lossy(&token)
                ));
                self.refusals.insert(name, why);
                continue;
            }
            let mut policy = Policy {
                token,
                ..Default::default()
            };
            let mut groups = BTreeMap::new();
            for field in item.value.items().unwrap_or_default() {
                if field.keyword_is("name") {
                    policy.name = field.value.read_string().unwrap_or_default().to_vec();
                } else if field.keyword_is("icon") {
                    policy.icon = field.value.read_string().unwrap_or_default().to_vec();
                } else if field.keyword_is("visible") {
                    policy
                        .visible
                        .extend_from_slice(field.value.items().unwrap_or_default());
                } else if field.keyword_is("available") {
                    policy
                        .available
                        .extend_from_slice(field.value.items().unwrap_or_default());
                } else if field.keyword_is("organization_modifier") {
                    policy
                        .organisation_modifiers
                        .read_all(field.value.items().unwrap_or_default());
                } else if field.keyword_is("equipment_bonus")
                    || field.keyword_is("production_bonus")
                {
                    for group in field.value.items().unwrap_or_default() {
                        let token = c_string(group.key.as_deref().unwrap_or_default()).to_vec();
                        let key = (
                            content_reader_politics::decisions::seed_component(&token) as u32,
                            token.to_ascii_lowercase(),
                        );
                        groups.entry(key).or_insert_with(|| {
                            let mut bonuses = Vec::new();
                            for item in group.value.items().unwrap_or_default() {
                                let Some(name) = item.keyword().map(|name| name.folded()) else {
                                    continue;
                                };
                                let name = String::from_utf8_lossy(&name).into_owned();
                                if content_reader_script::equipment_stats::stat_index(name.as_bytes()).is_none()
                                    && !is_production_stat(&name)
                                {
                                    continue;
                                }
                                if bonuses.iter().any(|bonus: &Bonus| bonus.name == name) {
                                    continue;
                                }
                                bonuses.push(Bonus {
                                    name,
                                    value: modifiers::stored(item),
                                });
                            }
                            (token, bonuses)
                        });
                    }
                }
            }
            policy.groups = groups.into_values().collect();
            self.rows.insert(name, policy);
        }
        if let Some(why) = read.unsettled {
            let why = why.to_string();
            if let Some(name) = current {
                self.rows.remove(&name);
                self.refusals.insert(name, why.clone());
            }
            self.unread.push(why.clone());
            return Err(why);
        }
        Ok(())
    }

    pub fn policy(&self, token: &[u8]) -> Result<&Policy, String> {
        let name = token.to_ascii_lowercase();
        if let Some(why) = self.refusals.get(&name) {
            return Err(format!(
                "MIO policy {}: {why}",
                String::from_utf8_lossy(token)
            ));
        }
        self.rows.get(&name).ok_or_else(|| {
            format!(
                "MIO policy {} is not defined{}",
                String::from_utf8_lossy(token),
                if self.unread.is_empty() {
                    String::new()
                } else {
                    format!(": {}", self.unread.join("; "))
                },
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_projection_keeps_first_groups_stats_and_definition() {
        content_reader_script::test_game::enter();
        let mut catalog = Catalog::default();
        catalog.read_file(
            b"synthetic={name=NAME equipment_bonus={z={soft_attack=1 soft_attack=9} a={defense=2}} production_bonus={z={production_cost_factor=3}} organization_modifier={custom_modifier_tooltip=TIP}} synthetic={name=LATE}",
            &KeyNames::default(),
        ).unwrap();
        let row = catalog.policy(b"SYNTHETIC").unwrap();
        assert_eq!(row.name, b"NAME");
        assert!(row.icon.is_empty() && row.visible.is_empty() && row.available.is_empty());
        assert_eq!(row.groups.len(), 2);
        let group = row.groups.iter().find(|(name, _)| name == b"z").unwrap();
        assert_eq!(
            group.1,
            vec![Bonus {
                name: "soft_attack".into(),
                value: 100_000
            }]
        );
        assert_eq!(row.organisation_modifiers.tooltips(), vec!["TIP"]);
        catalog
            .read_file(b"SYNTHETIC={name=OTHER}", &KeyNames::default())
            .unwrap();
        assert_eq!(catalog.policy(b"synthetic").unwrap().name, b"NAME");
        assert!(catalog.policy(b"missing").is_err());
        catalog
            .read_file(b"bad={name={unsupported}}", &KeyNames::default())
            .unwrap();
        assert!(
            catalog
                .policy(b"bad")
                .unwrap_err()
                .contains("field `name` is a block")
        );
        assert!(
            catalog
                .notes
                .iter()
                .any(|why| why.contains("MIO policy bad") && why.contains("count: 1"))
        );
        let mut catalog = Catalog::default();
        catalog
            .read_file(
                b"gated={icon=GFX_gated visible={always=no} available={always=yes} visible={has_war=yes}}",
                &KeyNames::default(),
            )
            .unwrap();
        let gated = catalog.policy(b"gated").unwrap();
        assert_eq!(gated.icon, b"GFX_gated");
        let keys = |items: &[Item]| {
            items
                .iter()
                .map(|item| c_string(item.key.as_deref().unwrap_or_default()).to_vec())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            keys(&gated.visible),
            [b"always".to_vec(), b"has_war".to_vec()]
        );
        assert_eq!(keys(&gated.available), [b"always".to_vec()]);
    }
}
