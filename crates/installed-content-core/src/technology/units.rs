//! The registries that select a technology template's dynamic field readers.
use std::{collections::BTreeSet, io};

use crate::{
    ResolvedLayers, equipment,
    fields::invalid,
    modifiers,
    parser::{
        self, Item,
        native::{self, Field, Shape},
    },
    tokens::Token,
};

#[derive(Default)]
pub(super) struct Registry {
    pub units: BTreeSet<Vec<u8>>,
    pub categories: BTreeSet<Vec<u8>>,
    pub terrains: BTreeSet<Vec<u8>>,
    pub equipment: BTreeSet<Vec<u8>>,
}

#[derive(Clone, Copy)]
pub(super) enum Node {
    File,
    Units,
    Unit,
    Parts,
    Part,
    Stats,
    Adjuster,
    Pool,
    Battalion,
    Modules,
    ModuleAmounts,
    Modifier(modifiers::Node),
}

/// Sub-unit stat readers shared by technology units and unit-bonus
/// effects, including fixed equipment stats.
pub(crate) enum StatKey {
    Scalar,
    Transport,
    Adjuster,
    Pool,
    Battalion,
    Unknown,
}

pub(crate) fn stat_key(key: Option<&[u8]>, terrains: &BTreeSet<Vec<u8>>) -> StatKey {
    let Some(key) = key else {
        return StatKey::Unknown;
    };
    if key == b"transport" {
        return StatKey::Transport;
    }
    if crate::equipment::stat_index(key).is_some()
        || matches!(
            key,
            b"combat_width"
                | b"sprite"
                | b"priority"
                | b"ai_priority"
                | b"own_equipment_fuel_consumption_mult"
        )
    {
        return StatKey::Scalar;
    }
    if matches!(key, b"need" | b"need_equipment") {
        return StatKey::Pool;
    }
    if key == b"battalion_mult" {
        return StatKey::Battalion;
    }
    if matches!(key, b"night" | b"amphibious" | b"river" | b"fort" | b"snow")
        || terrains.contains(key)
    {
        return StatKey::Adjuster;
    }
    StatKey::Unknown
}

/// Equipment token lookup uses the equipment name index, including
/// registered type aliases.
pub(crate) fn equipment_pool_roster(database: &equipment::Database) -> BTreeSet<Vec<u8>> {
    database
        .registered_names()
        .filter_map(|name| match Token::c_str(name) {
            Token::Name(name) => Some(name),
            Token::Rejected => None,
        })
        .collect()
}

/// A found equipment type reads fixed5; a miss reports the token
/// and skips a current opening value.
pub(crate) fn equipment_pool_shape(
    field: &Field<'_>,
    roster: Option<(&BTreeSet<Vec<u8>>, Option<&str>)>,
) -> Shape<()> {
    let Some(key) = field.keyword() else {
        return Shape::Skip;
    };
    if !field.skips_block() {
        return Shape::Skip;
    }
    let Some((names, gap)) = roster else {
        return Shape::Unsettled("equipment-stat reader: equipment names were not supplied".into());
    };
    if names.contains(&key) {
        Shape::One
    } else if let Some(why) = gap {
        Shape::Unsettled(format!(
            "equipment-stat reader: whether `{}` names equipment is unknown: {why}",
            String::from_utf8_lossy(&key)
        ))
    } else {
        Shape::Skip
    }
}

/// The unprojected scalar readers all consume one token. A block in the
/// fallback needs the stat/module registry to distinguish One from error
/// recovery; refuse that form, naming the field.
fn scalar(field: &Field<'_>, refused: &mut Vec<String>) -> Shape<Node> {
    if field.skips_block() {
        refused.push(format!(
            "unit field {}: unsupported block-valued scalar/stat",
            String::from_utf8_lossy(&field.key.text)
        ));
    }
    Shape::One
}

impl Registry {
    // Sub-unit stats, unit adjusters and equipment pools look up keyword
    // tokens, not arbitrary text: quoted names do not select these
    // dynamic readers.
    pub fn shape(
        &self,
        node: Node,
        field: &Field<'_>,
        undecided: &mut Vec<modifiers::Undecided>,
        refused: &mut Vec<String>,
    ) -> Shape<Node> {
        use Node::*;
        let key = field.keyword();
        match (node, key.as_deref()) {
            (File, Some(b"sub_units")) => Shape::Object(Units),
            (File, Some(b"critical_parts")) => Shape::Object(Parts),
            // sub_unit_categories uses its own lexer loop (value slot unchanged);
            // sub_unit_modifiers reads a string list.
            (File, Some(b"sub_unit_categories")) => Shape::RawList,
            (File, Some(b"sub_unit_modifiers")) => Shape::List,
            (File, _) => Shape::Skip,
            (Units, _) => Shape::Object(Unit),
            (Parts, _) => Shape::Object(Part),
            (Unit, Some(b"type")) if field.opens() => Shape::RawList,
            (Unit, Some(b"type")) => Shape::Current,
            // Categories, essential and allowed_battalion_groups use their own
            // lexer loops (value slot unchanged); enable_ability and critical_parts
            // read string lists, and required_dlc resolves each token into its list.
            (Unit, Some(b"categories" | b"essential" | b"allowed_battalion_groups")) => {
                Shape::RawList
            }
            (Unit, Some(b"required_dlc" | b"enable_ability" | b"critical_parts")) => Shape::List,
            (Unit, Some(b"need_equipment_modules")) => Shape::Object(Modules),
            (Unit, Some(b"deployed_leader_modifiers")) | (Part, Some(b"modifier")) => {
                Shape::Object(Modifier(modifiers::Node::Modifier))
            }
            (Part, Some(b"stat_penalties")) => {
                Shape::Object(Modifier(modifiers::Node::EquipmentBonus))
            }
            (
                Part,
                Some(
                    b"icon"
                    | b"frame"
                    | b"chance"
                    | b"str_damage"
                    | b"org_damage"
                    | b"str_damage_multiplier"
                    | b"org_damage_multiplier"
                    | b"damage_instance_per_added_module"
                    | b"base_damage_instance"
                    | b"max_damage_instance",
                ),
            ) => Shape::One,
            (Part, _) => Shape::Skip,
            (Modules, Some(b"any")) => Shape::Object(ModuleAmounts),
            (Modules | ModuleAmounts, _) => scalar(field, refused),
            (Unit | Stats, key) => match stat_key(key, &self.terrains) {
                StatKey::Scalar => Shape::One,
                StatKey::Transport => {
                    if field.skips_block() {
                        refused.push(
                            "unit transport: unsupported block-valued equipment lookup".into(),
                        );
                    }
                    Shape::Current
                }
                StatKey::Adjuster => Shape::Object(Adjuster),
                StatKey::Pool => Shape::Object(Pool),
                StatKey::Battalion => Shape::Object(Battalion),
                StatKey::Unknown if matches!(node, Stats) => Shape::Skip,
                StatKey::Unknown => scalar(field, refused),
            },
            (Adjuster, Some(b"attack" | b"defence" | b"movement")) => Shape::One,
            (Adjuster, _) => Shape::Skip,
            (Pool, _) => equipment_pool_shape(field, Some((&self.equipment, None))).map(|()| Pool),
            // Support category multipliers read category/bools, then numeric
            // stats. Unknown stats report without block skip.
            (Battalion, _) => scalar(field, refused),
            (Modifier(node), _) => {
                modifiers::shape(node, field, modifiers::UNKNOWN, undecided).map(Modifier)
            }
        }
    }

    fn apply(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut undecided = Vec::new();
        let mut refused = Vec::new();
        // The sub-unit database loops to EOF; definitions use the sub-unit
        // and critical-part readers. Each file constructs the lexer in mode 0.
        let read = native::read(
            bytes,
            native::Mode::KeepMark,
            native::Top::File(Node::File),
            None,
            &mut |node, field| self.shape(node, field, &mut undecided, &mut refused),
        );
        if let Some(cause) = read.unsettled {
            return Err(invalid(format!("technology unit registry: {cause}")));
        }
        if let Some(cause) = undecided.first() {
            return Err(invalid(format!("technology unit registry: {cause}")));
        }
        if let Some(cause) = refused.first() {
            return Err(invalid(cause));
        }
        for root in &read.items {
            if root.keyword_is("sub_units") {
                for unit in root.value.items().unwrap_or_default() {
                    if let Some(key) = &unit.key {
                        self.units
                            .insert(parser::c_string(key).to_ascii_lowercase());
                    }
                    // The type list is do-while; an empty list eats the close as an
                    // entry. Do not accept the List replay.
                    for field in unit.value.items().unwrap_or_default() {
                        if field.keyword_is("type")
                            && field.value.items().is_some_and(<[Item]>::is_empty)
                        {
                            return Err(invalid(format!(
                                "subunit {}: unsupported empty type list",
                                String::from_utf8_lossy(unit.key.as_deref().unwrap_or_default())
                            )));
                        }
                    }
                }
            } else if root.keyword_is("sub_unit_categories") {
                for item in root.value.items().unwrap_or_default() {
                    self.categories.insert(
                        item.value
                            .read_string()
                            .ok_or_else(|| invalid("subunit category is not scalar"))?
                            .to_ascii_lowercase(),
                    );
                }
            }
        }
        Ok(())
    }

    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        // Equipment-pool keys look up every registered equipment type name.
        let equipment = crate::equipment::Database::load_complete(layers)?;
        let mut registry = Self {
            equipment: equipment_pool_roster(&equipment),
            terrains: crate::terrain::read_definitions(layers)?
                .categories
                .into_iter()
                .map(|entry| entry.name.to_ascii_lowercase())
                .collect(),
            ..Self::default()
        };
        // Read critical_parts before the unit directory's own files.
        for folder in [
            "common/unit_tags",
            "common/units/critical_parts",
            "common/units",
        ] {
            for path in layers.directory_files(folder, ".txt")? {
                registry
                    .apply(&layers.read(&path)?)
                    .map_err(|e| invalid(format!("{path}: {e}")))?;
            }
        }
        Ok(registry)
    }
}
