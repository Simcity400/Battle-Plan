use std::collections::BTreeMap;
use std::sync::OnceLock;

use engine_executable::availability::{Verified, Withheld};
use engine_executable::build::ExecutableIdentity;
use engine_executable::executable_tables::{
    self, Tables,
    auxiliary::{AIR_GRAPH_DISPLAYS, AuxiliaryTables},
    commands::{Kind, Target, Token},
    cursors::{CursorAction, CursorTable},
    folding::FoldingTable,
    numeric::{NumericIdentity, NumericProperty, NumericScope, NumericTables},
    registries::{RegistryTables, TriggerReader},
    synthetic::{self, Parts},
    variables::VariableTables,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Version,
    Keywords,
    Registries,
    Auxiliary,
    Variables,
    Numeric,
    Commands,
    Folding,
    Cursors,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Version => "game version text",
            Self::Keywords => "keyword table",
            Self::Registries => "trigger and effect registries",
            Self::Auxiliary => "equipment, strategy and rule tables",
            Self::Variables => "built-in variable names",
            Self::Numeric => "numeric property table",
            Self::Commands => "player command table",
            Self::Folding => "diacritic folding table",
            Self::Cursors => "map cursor registry",
        }
    }
}

pub struct Executable {
    identity: ExecutableIdentity,
    version: String,
    keywords: BTreeMap<u32, &'static str>,
    modifiers: Vec<&'static str>,
    pinned: bool,
    registries: RegistryTables,
    auxiliary: AuxiliaryTables,
    variables: VariableTables,
    numeric: Vec<NumericProperty>,
    commands: Vec<Kind>,
    objects: Vec<(u32, u32)>,
    folding: Vec<(u32, u32)>,
    cursors: Vec<String>,
    cursor_bindings: BTreeMap<CursorAction, u32>,
    withheld: BTreeMap<Part, String>,
}

const FIRST_SYNTHETIC_ID: u32 = 0x10_0000;

fn leak(text: &str) -> &'static str {
    executable_tables::leak_string(text)
}

impl Executable {
    pub fn empty() -> Self {
        Self {
            identity: ExecutableIdentity::of(crate::DEFAULT_EXECUTABLE),
            version: "synthetic".to_owned(),
            keywords: BTreeMap::new(),
            modifiers: Vec::new(),
            pinned: false,
            registries: RegistryTables::default(),
            auxiliary: AuxiliaryTables {
                equipment_categories: Ok(Vec::new()),
                category_names: Ok(Vec::new()),
                stat_metadata: Ok(Vec::new()),
                ai_strategy_types: Ok(Vec::new()),
                missions: Ok(Vec::new()),
                equipment_missions: Ok(Vec::new()),
                execution_types: Ok(Vec::new()),
                order_types: Ok(Vec::new()),
                selectors: Ok(Vec::new()),
                rules: Ok(Vec::new()),
                air_graph_channels: Ok([0; AIR_GRAPH_DISPLAYS]),
                air_graph_colors: Ok([[0.0; 4]; AIR_GRAPH_DISPLAYS]),
                air_graph_scale: Ok(1.0),
                air_detection_override_prefix: Ok("synthetic_override_"),
                category_families: Ok(executable_tables::auxiliary::CategoryFamilies {
                    land: 0,
                    air: 0,
                    naval: 0,
                    ship_designer: 0,
                    tank_designer: 0,
                    plane_designer: 0,
                }),
            },
            variables: VariableTables {
                global: Vec::new(),
                country: Vec::new(),
                state: Vec::new(),
                unit_leader: Vec::new(),
                industrial_organisation: Vec::new(),
                project: Vec::new(),
            },
            numeric: Vec::new(),
            commands: Vec::new(),
            objects: Vec::new(),
            folding: Vec::new(),
            cursors: Vec::new(),
            cursor_bindings: BTreeMap::new(),
            withheld: BTreeMap::new(),
        }
    }

    pub fn executable(mut self, bytes: &[u8]) -> Self {
        self.identity = ExecutableIdentity::of(bytes);
        self
    }

    pub fn fixed_layout(token_ids: &[u32]) -> Self {
        token_ids
            .iter()
            .fold(Self::empty(), |game, &id| {
                game.keyword(id, &format!("synthetic_token_{id}"))
            })
            .modifiers(
                (0..engine_executable::build::FIXED_MODIFIER_GAP)
                    .map(|index| format!("synthetic_modifier_{index}")),
            )
            .pinned()
    }

    pub fn version(mut self, text: &str) -> Self {
        text.clone_into(&mut self.version);
        self
    }

    pub fn keyword(mut self, id: u32, name: &str) -> Self {
        self.put_keyword(id, name);
        self
    }

    pub fn word(mut self, name: &str) -> Self {
        self.id(name);
        self
    }

    pub fn words<'a>(self, names: impl IntoIterator<Item = &'a str>) -> Self {
        names.into_iter().fold(self, Self::word)
    }

    pub fn token(&self, name: &str) -> Option<u32> {
        self.keywords
            .iter()
            .find(|(_, kept)| kept.eq_ignore_ascii_case(name))
            .map(|(&id, _)| id)
    }

    fn id(&mut self, name: &str) -> u32 {
        if let Some(id) = self.token(name) {
            return id;
        }
        let id = self
            .keywords
            .range(FIRST_SYNTHETIC_ID..)
            .next_back()
            .map_or(FIRST_SYNTHETIC_ID, |(&id, _)| id + 1);
        self.keywords.insert(id, leak(name));
        id
    }

    fn put_keyword(&mut self, id: u32, name: &str) {
        match self.keywords.get(&id) {
            Some(kept) if *kept != name => {
                panic!("synthetic keyword {id:#x} is already {kept}, not {name}")
            }
            Some(_) => {}
            None => {
                if let Some(other) = self.token(name) {
                    panic!("synthetic keyword {name} is already {other:#x}, not {id:#x}");
                }
                self.keywords.insert(id, leak(name));
            }
        }
    }

    pub fn modifiers(mut self, names: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.modifiers
            .extend(names.into_iter().map(|name| leak(name.as_ref())));
        self
    }

    pub fn pinned_modifiers(mut self, names: impl IntoIterator<Item = (u32, &'static str)>) -> Self {
        let count = engine_executable::build::FIXED_MODIFIER_GAP as usize;
        self.modifiers.extend(
            (self.modifiers.len()..count).map(|index| leak(&format!("synthetic_modifier_{index}"))),
        );
        for (index, name) in names {
            self.modifiers[index as usize] = name;
        }
        self.pinned()
    }

    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    pub fn trigger(mut self, name: &str, scopes: u32, reader: TriggerReader, key_reader: u64) -> Self {
        let id = self.id(name);
        self.put_trigger(id, scopes, reader, key_reader);
        self
    }

    pub fn trigger_at(
        mut self,
        id: u32,
        name: &str,
        scopes: u32,
        reader: TriggerReader,
        key_reader: u64,
    ) -> Self {
        self.put_keyword(id, name);
        self.put_trigger(id, scopes, reader, key_reader);
        self
    }

    fn put_trigger(&mut self, id: u32, scopes: u32, reader: TriggerReader, key_reader: u64) {
        let name = self.keywords[&id];
        let registries = &mut self.registries;
        let at = registries.triggers.partition_point(|&(kept, _)| kept < id);
        assert!(
            registries.triggers.get(at).is_none_or(|&(kept, _)| kept != id),
            "synthetic trigger {name} is declared twice"
        );
        registries.triggers.insert(at, (id, name));
        registries.trigger_classes.insert(at, (id, scopes, reader));
        registries.trigger_key_readers.insert(at, (id, key_reader));
    }

    pub fn effect(mut self, name: &str, value_reader: u64, key_reader: u64, class: u64) -> Self {
        let id = self.id(name);
        self.put_effect(id, value_reader, key_reader, class, None);
        self
    }

    pub fn named_effect_class(mut self, name: &'static str, class: u64) -> Self {
        let id = self.id(name);
        self.put_effect(id, 0, 0, class, Some(name));
        self
    }

    pub fn effect_at(
        mut self,
        id: u32,
        name: &str,
        value_reader: u64,
        key_reader: u64,
        class: u64,
    ) -> Self {
        self.put_keyword(id, name);
        self.put_effect(id, value_reader, key_reader, class, None);
        self
    }

    fn put_effect(
        &mut self,
        id: u32,
        value_reader: u64,
        key_reader: u64,
        class: u64,
        class_name: Option<&'static str>,
    ) {
        let name = self.keywords[&id];
        let registries = &mut self.registries;
        let at = registries.effects.partition_point(|&(kept, _)| kept < id);
        assert!(
            registries.effects.get(at).is_none_or(|&(kept, _)| kept != id),
            "synthetic effect {name} is declared twice"
        );
        registries.effects.insert(at, (id, name));
        registries.effect_classes.insert(at, (id, value_reader, key_reader));
        let name = class_name.unwrap_or_else(|| leak(&format!("synthetic_effect_class_{class:x}")));
        registries.effect_identities.insert(at, (id, name, class));
    }

    pub fn auxiliary(mut self, change: impl FnOnce(&mut AuxiliaryTables)) -> Self {
        change(&mut self.auxiliary);
        self
    }

    pub fn variables(mut self, change: impl FnOnce(&mut VariableTables)) -> Self {
        change(&mut self.variables);
        self
    }

    pub fn numeric_scope(mut self, name: &str, scope: NumericScope, ordinal: u32) -> Self {
        self.numeric.push(NumericProperty {
            name: leak(name),
            identity: NumericIdentity::Scope {
                scope,
                ordinal,
                handler_rva: 0x1000 + scope as u32,
            },
        });
        self
    }

    pub fn numeric_callback(mut self, name: &str, vtable_rva: u32) -> Self {
        self.numeric.push(NumericProperty {
            name: leak(name),
            identity: NumericIdentity::Callback {
                vtable_rva,
                value_reader_rva: vtable_rva + 0x10,
                input_reader_rva: vtable_rva + 0x18,
            },
        });
        self
    }

    pub fn command(mut self, vtable: u32, name: &str, token: Token, target: Target) -> Self {
        self.commands.push(Kind {
            vtable,
            name: name.to_owned(),
            token,
            target,
        });
        self
    }

    pub fn command_object(mut self, vtable: u32, mask: u32) -> Self {
        self.objects.push((vtable, mask));
        self
    }

    pub fn folds(mut self, point: char, letter: char) -> Self {
        self.folding.push((point as u32, letter as u32));
        self
    }

    pub fn cursors<S: Into<String>>(mut self, paths: impl IntoIterator<Item = S>) -> Self {
        self.cursors.extend(paths.into_iter().map(Into::into));
        self
    }

    pub fn cursor_binding(mut self, action: CursorAction, slot: u32) -> Self {
        self.cursor_bindings.insert(action, slot);
        self
    }

    pub fn cursor_bindings(
        mut self,
        bindings: impl IntoIterator<Item = (CursorAction, u32)>,
    ) -> Self {
        self.cursor_bindings.extend(bindings);
        self
    }

    pub fn withhold(mut self, part: Part, reason: &str) -> Self {
        self.withheld.insert(part, reason.to_owned());
        self
    }

    pub fn build(self) -> Tables {
        let withheld = |part: Part| -> Option<Withheld> {
            self.withheld
                .get(&part)
                .map(|reason| Withheld::new(part.name(), reason))
        };
        fn present<T>(withheld: Option<Withheld>, value: impl FnOnce() -> T) -> Verified<T> {
            match withheld {
                Some(withheld) => Err(withheld),
                None => Ok(value()),
            }
        }
        let mut commands = self.commands;
        commands.sort_unstable_by_key(|kind| kind.vtable);
        let mut objects = self.objects;
        objects.sort_unstable();
        let folding = FoldingTable::from_pairs(self.folding.iter().copied())
            .expect("synthetic folding pairs are folded points and letters");
        let capacity = self.cursors.len();
        let cursors = CursorTable::from_raw_paths(self.cursors, capacity, self.cursor_bindings);
        Parts {
            identity: self.identity.clone(),
            version: present(withheld(Part::Version), || self.version.clone()),
            keywords: present(withheld(Part::Keywords), || {
                synthetic::keyword_table(
                    self.keywords.iter().map(|(&id, &name)| (id, name)).collect(),
                    self.modifiers.clone(),
                    self.pinned,
                )
            }),
            registries: present(withheld(Part::Registries), || self.registries),
            auxiliary: present(withheld(Part::Auxiliary), || self.auxiliary),
            variables: present(withheld(Part::Variables), || self.variables),
            numeric: present(withheld(Part::Numeric), || NumericTables {
                properties: self.numeric,
            }),
            commands: present(withheld(Part::Commands), || {
                synthetic::command_table(commands, objects)
            }),
            folding: present(withheld(Part::Folding), || folding),
            cursors: present(withheld(Part::Cursors), || cursors),
        }
        .assemble()
    }

    pub fn enter(self) -> Tables {
        let tables = self.build();
        executable_tables::enter(Some(tables));
        tables
    }
}

pub fn tables() -> Tables {
    static EMPTY: OnceLock<Tables> = OnceLock::new();
    *EMPTY.get_or_init(|| Executable::empty().build())
}

pub fn enter() -> Tables {
    let tables = tables();
    executable_tables::enter(Some(tables));
    tables
}

pub fn enter_fixed_modifiers() -> Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    let tables = *TABLES.get_or_init(|| Executable::fixed_layout(&[]).build());
    executable_tables::enter(Some(tables));
    tables
}
