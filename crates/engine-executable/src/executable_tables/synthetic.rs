use crate::availability::{Verified, Withheld};
use crate::build::ExecutableIdentity;

use super::{
    ExecutableTables, Tables, auxiliary::AuxiliaryTables, commands, cursors::CursorTable,
    folding::FoldingTable, keywords::KeywordTables, numeric::NumericTables,
    registries::RegistryTables, variables::VariableTables,
};

pub struct Parts {
    pub identity: ExecutableIdentity,
    pub version: Verified<String>,
    pub keywords: Verified<KeywordTables>,
    pub registries: Verified<RegistryTables>,
    pub auxiliary: Verified<AuxiliaryTables>,
    pub variables: Verified<VariableTables>,
    pub numeric: Verified<NumericTables>,
    pub commands: Verified<commands::CommandTables>,
    pub folding: Verified<FoldingTable>,
    pub cursors: Verified<CursorTable>,
}

impl Parts {
    pub fn empty() -> Self {
        let absent = |part: &'static str| Withheld::new(part, "absent from these synthetic tables");
        Self {
            identity: ExecutableIdentity {
                size: 0,
                sha256: [0; 32],
            },
            version: Err(absent("game version text")),
            keywords: Err(absent(super::KEYWORDS)),
            registries: Err(absent(super::REGISTRIES)),
            auxiliary: Err(absent(super::AUXILIARY)),
            variables: Err(absent(super::VARIABLES)),
            numeric: Err(absent(super::NUMERIC)),
            commands: Err(absent(super::COMMANDS)),
            folding: Err(absent(super::FOLDING)),
            cursors: Err(absent(super::CURSORS)),
        }
    }

    pub fn keywords(mut self, keywords: KeywordTables) -> Self {
        self.keywords = Ok(keywords);
        self
    }

    pub fn cursors(mut self, cursors: CursorTable) -> Self {
        self.cursors = Ok(cursors);
        self
    }

    pub fn assemble(self) -> Tables {
        Box::leak(Box::new(ExecutableTables {
            identity: self.identity,
            version: self.version,
            keywords: self.keywords,
            registries: self.registries,
            auxiliary: self.auxiliary,
            variables: self.variables,
            numeric: self.numeric,
            commands: self.commands,
            folding: self.folding,
            cursors: self.cursors,
        }))
    }
}

pub fn keyword_table(
    mut names: Vec<(u32, &'static str)>,
    modifier_names: Vec<&'static str>,
    pinned: bool,
) -> KeywordTables {
    names.sort_unstable_by_key(|&(id, _)| id);
    if pinned {
        KeywordTables::pinned(names, modifier_names)
    } else {
        KeywordTables::recorded(names, modifier_names)
    }
}

pub fn command_table(kinds: Vec<commands::Kind>, objects: Vec<(u32, u32)>) -> commands::CommandTables {
    commands::CommandTables::recorded(kinds, objects)
}
