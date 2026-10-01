//! Native saved-target selectors over fresh caller scopes.
use super::{Scope, Scopes, eval::Evaluator};
use crate::scope::{RecordedScopes, valid_saved_target};
use replay::rows::GlobalSavedEventTargetRow as Row;

#[derive(Clone, Copy)]
pub(crate) enum Reader {
    CountrySelector,
    Context,
    Variable,
    Text,
}
impl Reader {
    pub(crate) fn refusal(self, name: &[u8], kind: &str) -> String {
        let (use_, address) = match self {
            Self::Context => ("scope", "14146BB30"),
            Self::Variable => ("variable storage", "140535BE0"),
            Self::Text => ("scripted-text object", "1410CFF00"),
            Self::CountrySelector => unreachable!("country selector has no unsupported target"),
        };
        format!(
            "saved event target {}: {kind} {use_} is not reproduced ({address})",
            String::from_utf8_lossy(name)
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Selected {
    Missing,
    State(i32),
    Country(i32),
    Leader(u64),
    Character(u32),
    Caller,
    Unsupported(&'static str),
}
impl Selected {
    pub(crate) fn scope(
        self,
        recorded: &RecordedScopes,
        caller: Scope,
        name: &[u8],
        reader: Reader,
    ) -> Result<Option<Scope>, String> {
        Ok(match self {
            Self::Missing => None,
            Self::State(id) => Some(Scope::State(id as u32)),
            Self::Country(raw) => recorded.saved_country(raw, name)?.map(Scope::Country),
            Self::Leader(reference) => Some(Scope::Leader(reference)),
            Self::Character(id) => Some(Scope::Character(id)),
            Self::Caller => Some(caller),
            Self::Unsupported(kind) => return Err(reader.refusal(name, kind)),
        })
    }
}
/// 14146BB30 (context), 140535BE0 (variable store), 1410CFF00 (text),
/// 140542000 (country selector).
pub(crate) fn select(r: &Row, reader: Reader) -> Selected {
    if !valid_saved_target(r) {
        return Selected::Missing;
    }
    if matches!(reader, Reader::CountrySelector) {
        return if r.country > 0 {
            Selected::Country(r.country)
        } else {
            Selected::Missing
        };
    }
    let variable = matches!(reader, Reader::Variable);
    if r.state != 0 {
        return if variable && r.state < 0 {
            Selected::Unsupported("negative state with a null native variable store")
        } else {
            Selected::State(r.state)
        };
    }
    if r.country > 0 {
        return Selected::Country(r.country);
    }
    if r.live & Row::CHARACTER != 0 {
        // Variable storage uses the character, not GetUnitLeader (140535BE0).
        return if variable {
            Selected::Character(r.character)
        } else {
            match r.character_leader {
                None => Selected::Character(r.character),
                Some(0) => Selected::Unsupported("unit leader with null identity"),
                Some(reference) => Selected::Leader(reference),
            }
        };
    }
    // Each reader's native priority over the remaining kinds.
    let kinds: &[u16] = match reader {
        Reader::Variable => &[
            Row::INDUSTRIAL_ORGANISATION,
            Row::PURCHASE_CONTRACT,
            Row::RAID_INSTANCE,
            Row::PROJECT,
            Row::FACTION,
        ],
        Reader::Text => &[
            Row::COMBATANT,
            Row::OPERATION,
            Row::ACE,
            Row::INDUSTRIAL_ORGANISATION,
            Row::PURCHASE_CONTRACT,
            Row::PROJECT,
        ],
        _ => &[
            Row::OPERATION,
            Row::REGION,
            Row::COMBATANT,
            Row::ACE,
            Row::UNIT,
            Row::INDUSTRIAL_ORGANISATION,
            Row::PROJECT,
        ],
    };
    if let Some(&mask) = kinds.iter().find(|&&mask| r.live & mask != 0) {
        Selected::Unsupported(Row::kind_name(mask))
    } else if variable {
        Selected::Missing
    } else {
        // No setter: contexts retain caller THIS; text keeps its object.
        Selected::Caller
    }
}
impl Evaluator<'_, '_> {
    pub(super) fn saved_scope(
        &self,
        name: &[u8],
        reader: Reader,
        scopes: Scopes,
    ) -> Result<Option<Scopes>, String> {
        let recorded = self.world().recorded();
        let selected = recorded.saved_selection(name, reader)?;
        Ok(selected
            .scope(recorded, scopes.this, name, reader)?
            .map(|this| {
                let mut entered = scopes.with_this(this);
                if selected == Selected::Caller {
                    entered.bare = scopes.bare;
                }
                entered
            }))
    }
}
