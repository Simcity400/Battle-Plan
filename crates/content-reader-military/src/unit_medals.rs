use content_reader_key_names::key_name_products::KeyNamesExt as _;
use content_reader_layers::{ResolvedLayers, script_files};
use content_reader_script::{key_names::KeyNames, modifiers::{self, Modifiers}, native::{self, Shape}, scope::ScopeKind};
use content_reader_parser::parser::{Item, Value, c_string};
use std::io;

#[derive(Clone, Debug, Default)]
pub struct Medal {
    pub name: Vec<u8>,
    pub source: String,
    pub unit: Modifiers,
    pub leader: Modifiers,
    pub ship: Modifiers,
    pub officer_xp: Option<Value>,
    pub on_grant: Vec<Item>,
    pub refusals: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct Medals {
    pub definitions: Vec<Medal>,
    pub refusals: Vec<String>,
}
#[derive(Clone, Copy)]
enum Node {
    Top,
    Entries,
    Medal,
    Modifier(modifiers::Node),
}
impl Medals {
    pub fn load(layers: &ResolvedLayers, registry: modifiers::Registry<'_>) -> io::Result<Self> {
        let names = layers.key_names()?;
        let mut medals = Self::default();
        for path in script_files::paths(layers, "common/unit_medals/", Some(".txt"))? {
            medals.read(&path.text(), &path.read(layers)?, &names, registry);
        }
        Ok(medals)
    }
    fn read(
        &mut self,
        path: &str,
        bytes: &[u8],
        names: &KeyNames,
        registry: modifiers::Registry<'_>,
    ) {
        let mut unresolved = Vec::new();
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(names),
            &mut |node, field| match node {
                Node::Top if field.keyword_is("unit_medals") => Shape::Object(Node::Entries),
                Node::Top => Shape::Skip,
                Node::Entries => Shape::Object(Node::Medal),
                Node::Medal => match field.keyword().as_deref() {
                    Some(b"unit_modifiers" | b"leader_modifiers" | b"ship_modifiers") => {
                        Shape::Object(Node::Modifier(modifiers::Node::Modifier))
                    }
                    Some(b"available") => Shape::Trigger,
                    Some(b"one_time_effect") => Shape::EffectIn(ScopeKind::Unit.bit()),
                    Some(b"officer_xp") => Shape::Expression,
                    Some(b"icon" | b"frame" | b"cost") => Shape::One,
                    _ => Shape::Skip,
                },
                Node::Modifier(node) => {
                    modifiers::shape(node, field, registry, &mut unresolved).map(Node::Modifier)
                }
            },
        );
        if let Some(gap) = read.unsettled {
            self.refusals
                .push(format!("{path} line {}: {}", gap.line, gap.what));
        }
        self.refusals
            .extend(unresolved.into_iter().map(|why| format!("{path}: {why}")));
        for root in read.items {
            if root.keyword().map(|key| key.folded()).as_deref() != Some(b"unit_medals") {
                continue;
            }
            for entry in root.value.items().unwrap_or_default() {
                let Some(name) = &entry.key else {
                    continue;
                };
                let mut medal = Medal {
                    name: c_string(name).to_vec(),
                    source: path.into(),
                    ..Default::default()
                };
                for item in entry.value.items().unwrap_or_default() {
                    match item.keyword().map(|key| key.folded()).as_deref() {
                        Some(b"unit_modifiers") => {
                            medal.unit.read_all(item.value.items().unwrap_or_default())
                        }
                        Some(b"leader_modifiers") => medal
                            .leader
                            .read_all(item.value.items().unwrap_or_default()),
                        Some(b"ship_modifiers") => {
                            medal.ship.read_all(item.value.items().unwrap_or_default())
                        }
                        Some(b"one_time_effect") => medal
                            .on_grant
                            .extend_from_slice(item.value.items().unwrap_or_default()),
                        Some(b"officer_xp") => medal.officer_xp = Some(item.value.clone()),
                        _ => {}
                    }
                }
                for block in [&medal.unit, &medal.leader, &medal.ship] {
                    if let Some(why) = block.unheld.cause() {
                        medal
                            .refusals
                            .push(format!("{}: {why}", String::from_utf8_lossy(&medal.name)));
                    }
                }
                self.definitions.push(medal);
            }
        }
    }
    pub fn get(&self, ordinal: i32, name: &[u8]) -> Result<&Medal, String> {
        if let Some(why) = self.refusals.first() {
            return Err(why.clone());
        }
        self.definitions
            .get(
                ordinal
                    .checked_sub(1)
                    .and_then(|ordinal| usize::try_from(ordinal).ok())
                    .unwrap_or(usize::MAX),
            )
            .filter(|medal| medal.name.eq_ignore_ascii_case(c_string(name)))
            .ok_or_else(|| {
                format!(
                    "medal {} definition {ordinal} is absent from recorded content",
                    String::from_utf8_lossy(name)
                )
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn medal_content_keeps_order_repetitions_variables_and_all_modifier_domains() {
        content_reader_script::test_game::enter();
        let mut medals = Medals::default();
        medals.read("synthetic", br"@a=0.2 unit_medals={ MOD={unit_modifiers={army_morale_factor=@a} leader_modifiers={army_attack_factor=0.1} ship_modifiers={naval_morale_factor=0.3} officer_xp=37 one_time_effect={add_manpower=12}} MOD={unit_modifiers={max_dig_in=4}}}", &KeyNames::default(), &|_| Some(true));
        assert_eq!(medals.definitions.len(), 2);
        assert!(medals.get(0, b"mod").is_err());
        let first = medals.get(1, b"mod").unwrap();
        assert_eq!(first.unit.values[0].value, 20_000);
        assert_eq!(first.leader.values[0].value, 10_000);
        assert_eq!(first.ship.values[0].value, 30_000);
        assert_eq!(first.on_grant.len(), 1);
        assert_eq!(medals.get(2, b"mod").unwrap().unit.values[0].value, 400_000);
    }
}
