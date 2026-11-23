use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::content::diagnostics::Once as OnceLock;
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_parser::{coverage::Coverage, parser::Item, tokens::Token};
use content_reader_script::{key_names::KeyNames, native::{self, Shape}};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Default, Debug, Clone)]
pub struct Upgrade {
    pub name: Option<Vec<u8>>,
    pub bonus_raw: i64,
    pub cost_raw: i64,
}
#[derive(Default, Debug, Clone)]
pub struct Group {
    pub name: Option<Vec<u8>>,
    pub upgrade_type: Option<Vec<u8>>,
    pub upgrades: Vec<Vec<u8>>,
}
#[derive(Default, Debug)]
pub struct Catalog {
    pub upgrades: BTreeMap<Token, Upgrade>,
    pub groups: BTreeMap<Token, Group>,
    pub warnings: Vec<String>,
    coverage: Coverage,
    inline: BTreeSet<Token>,
    ambiguous: BTreeSet<Token>,
}
impl Catalog {
    pub fn upgrade(&self, name: &[u8]) -> Result<&Upgrade, String> {
        let identity = String::from_utf8_lossy(name);
        let token = Token::c_str(name);
        if self.ambiguous.contains(&token) {
            return Err(format!(
                "upgrade {identity} has distinct inline definitions"
            ));
        }
        if let Some(why) = self.coverage.unknown(&identity) {
            return Err(why.to_string());
        }
        self.upgrades
            .get(&token)
            .ok_or_else(|| format!("upgrade {identity} definition missing"))
    }
    pub fn group(&self, name: &[u8]) -> Result<&Group, String> {
        let identity = String::from_utf8_lossy(name);
        if let Some(why) = self.coverage.unknown(&identity) {
            return Err(why.to_string());
        }
        self.groups
            .get(&Token::c_str(name))
            .ok_or_else(|| format!("upgrade group {identity} definition missing"))
    }
}
#[derive(Clone, Copy)]
enum Node {
    File,
    Upgrade,
    Group,
    Uniform,
    Mtth,
}
fn parse(bytes: &[u8], names: &KeyNames, groups: bool) -> native::Read {
    native::read(
        bytes,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        Some(names),
        &mut |node, field| match node {
            Node::File => Shape::Object(if groups { Node::Group } else { Node::Upgrade }),
            Node::Upgrade => match field.keyword().as_deref() {
                Some(b"name" | b"desc" | b"icon" | b"upgrade_cost" | b"bonus") => Shape::One,
                Some(b"ai_will_do") => Shape::Object(Node::Mtth),
                _ => Shape::Skip,
            },
            Node::Group => match field.keyword().as_deref() {
                Some(
                    b"name"
                    | b"desc"
                    | b"icon"
                    | b"upgrade_type"
                    | b"default_upgrade"
                    | b"display_as_percentage",
                ) => Shape::One,
                Some(b"upgrades") => Shape::Statements(Node::Uniform),
                _ => Shape::Skip,
            },
            Node::Uniform if groups && field.opens() => Shape::Object(Node::Upgrade),
            Node::Uniform if groups => Shape::One,
            Node::Uniform => Shape::One,
            Node::Mtth => native::mean_time_to_happen(field),
        },
    )
}
fn upgrade(catalog: &mut Catalog, item: &Item) {
    let Some(fields) = item.value.items() else {
        return;
    };
    let name = item.key.as_deref().unwrap_or_default();
    catalog.coverage.add(&String::from_utf8_lossy(name));
    let target = catalog
        .upgrades
        .entry(Token::c_str(item.key.as_deref().unwrap_or_default()))
        .or_default();
    for field in fields {
        if field.keyword_is("name") {
            target.name = field.value.scalar().map(<[u8]>::to_vec);
        }
        if field.keyword_is("bonus") {
            target.bonus_raw = content_reader_script::modifiers::stored(field);
        }
        if field.keyword_is("upgrade_cost") {
            target.cost_raw = content_reader_script::modifiers::stored(field);
        }
    }
}
pub fn cached(layers: &ResolvedLayers) -> Result<Arc<Catalog>, String> {
    let memo = layers.derived::<OnceLock<Result<Arc<Catalog>, String>>>();
    memo.get_or_init(|| {
        let names = layers.key_names().map_err(|why| why.to_string())?;
        let mut catalog = Catalog::default();
        for (folder, groups) in [
            ("common/factions/member_upgrades/", false),
            ("common/factions/member_upgrades/member_groups/", true),
        ] {
            for path in
                script_files::paths(layers, folder, Some(".txt")).map_err(|why| why.to_string())?
            {
                catalog.coverage.next_file();
                let read = parse(
                    &path.read(layers).map_err(|why| why.to_string())?,
                    &names,
                    groups,
                );
                for item in &read.items {
                    if groups {
                        let Some(fields) = item.value.items() else {
                            continue;
                        };
                        let token = Token::c_str(item.key.as_deref().unwrap_or_default());
                        let mut selected = Vec::new();
                        for field in fields {
                            if field.keyword_is("upgrades") {
                                for value in field.value.items().unwrap_or_default() {
                                    if value.value.items().is_some() {
                                        let token =
                                            Token::c_str(value.key.as_deref().unwrap_or_default());
                                        if catalog.upgrades.contains_key(&token) {
                                            catalog.ambiguous.insert(token.clone());
                                        }
                                        catalog.inline.insert(token);
                                        upgrade(&mut catalog, value);
                                        if let Some(key) = &value.key {
                                            selected.push(key.clone());
                                        }
                                    } else if let Some(name) = value.value.scalar() {
                                        selected.push(name.to_vec());
                                    }
                                }
                            }
                        }
                        catalog.coverage.add(&String::from_utf8_lossy(
                            item.key.as_deref().unwrap_or_default(),
                        ));
                        let group = catalog.groups.entry(token).or_default();
                        group.upgrades.extend(selected);
                        for field in fields {
                            if field.keyword_is("name") {
                                group.name = field.value.scalar().map(<[u8]>::to_vec);
                            }
                            if field.keyword_is("upgrade_type") {
                                group.upgrade_type = field.value.scalar().map(<[u8]>::to_vec);
                            }
                        }
                    } else {
                        let token = Token::c_str(item.key.as_deref().unwrap_or_default());
                        if catalog.inline.contains(&token) {
                            catalog.ambiguous.insert(token);
                        }
                        upgrade(&mut catalog, item);
                    }
                }
                if let Some(why) = read.unsettled {
                    return Err(format!("faction upgrades in {path}: {why}"));
                }
                catalog.warnings.extend(
                    read.misreads
                        .iter()
                        .map(|why| format!("faction upgrades in {path}: {why}")),
                );
            }
        }
        Ok(Arc::new(catalog))
    })
    .clone()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_upgrade_loaders_retain_a_leading_byte_order_mark_in_the_first_identity() {
        content_reader_script::test_game::enter();
        for groups in [false, true] {
            let read = parse(b"\xef\xbb\xbfsynthetic={}", &KeyNames::default(), groups);
            assert_eq!(
                read.items[0].key.as_deref(),
                Some(&b"\xef\xbb\xbfsynthetic"[..])
            );
        }
    }
    #[test]
    fn numeric_upgrade_bonus_merges_and_unknown_fields_do_not_become_modifiers() {
        content_reader_script::test_game::enter();
        let read = parse(
            b"synthetic={bonus=0.25 upgrade_cost=3 unsupported_modifier={foo=9}}",
            &KeyNames::default(),
            false,
        );
        let mut catalog = Catalog::default();
        for item in &read.items {
            upgrade(&mut catalog, item);
        }
        let value = &catalog.upgrades[&Token::c_str(b"synthetic")];
        assert_eq!(value.bonus_raw, 25000);
        assert_eq!(value.cost_raw, 300000);
    }
    #[test]
    fn inline_upgrade_ai_weights_are_not_treated_as_nested_upgrade_definitions() {
        content_reader_script::test_game::enter();
        let read=parse(b"synthetic_group={upgrade_type=faction_member_upgrade_manpower upgrades={synthetic={bonus=0.125 ai_will_do={factor=2 modifier={factor=3 always=yes}}}}}",&KeyNames::default(),true);
        assert!(read.unsettled.is_none());
        let fields = read.items[0].value.items().unwrap();
        let upgrades = fields
            .iter()
            .find(|field| field.keyword_is("upgrades"))
            .unwrap()
            .value
            .items()
            .unwrap();
        let mut catalog = Catalog::default();
        upgrade(&mut catalog, &upgrades[0]);
        assert_eq!(catalog.upgrade(b"synthetic").unwrap().bonus_raw, 12500);
    }
    #[test]
    fn faction_upgrade_authored_names_follow_mod_replace_path_and_inline_definitions() {
        content_reader_script::test_game::enter();
        let stack = content_reader_layers::test_support::Stack::new("faction-names");
        stack.write(
            &stack.base(),
            "common/factions/member_upgrades/base.txt",
            "old={name=base_name bonus=0.1}",
        );
        stack.write(&stack.addon(),"common/factions/member_upgrades/member_groups/mod.txt","mod_group={name=mod_group_name upgrade_type=mod_type upgrades={inline={name=mod_inline_name bonus=0.25}}} missing_name={upgrades={}}");
        let layers = stack.layers(&["common/factions/member_upgrades"]);
        let catalog = cached(&layers).unwrap();
        assert!(catalog.upgrade(b"old").is_err());
        assert_eq!(
            catalog.group(b"mod_group").unwrap().name.as_deref(),
            Some(b"mod_group_name".as_slice())
        );
        assert_eq!(
            catalog.upgrade(b"inline").unwrap().name.as_deref(),
            Some(b"mod_inline_name".as_slice())
        );
        assert!(catalog.group(b"missing_name").unwrap().name.is_none());
    }
}
