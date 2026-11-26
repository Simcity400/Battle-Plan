use super::{Scope, Scopes, eval::Evaluator};
use crate::scope::{RecordedScopes, valid_saved_target};
use crate::triggers::Failure;
use replay_data::rows::GlobalSavedEventTargetRow as Row;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Reader {
    CountrySelector,
    Context,
    Variable,
    Text,
}
impl Reader {
    pub(crate) fn refusal(self, name: &[u8], kind: &str) -> String {
        let use_ = match self {
            Self::Context => "scope",
            Self::Variable => "variable storage",
            Self::Text => "scripted-text object",
            Self::CountrySelector => unreachable!("country selector has no unsupported target"),
        };
        let reference = match kind {
            "unit" => Some("unit"),
            "industrial_organisation" => Some("organisation"),
            "raid_instance" => Some("raid"),
            "project" => Some("project"),
            _ => None,
        };
        if let Some(reference) = reference {
            return format!(
                "saved event target {}: {kind} {use_} needs the full {reference} reference; global saved event targets records liveness only",
                String::from_utf8_lossy(name)
            );
        }
        format!(
            "saved event target {}: {kind} {use_} is not reproduced",
            String::from_utf8_lossy(name)
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
        recorded: &RecordedScopes<'_>,
        caller: Scope,
        name: &[u8],
        reader: Reader,
    ) -> Result<Option<Scope>, Failure> {
        Ok(match self {
            Self::Missing => None,
            Self::State(id) => Some(Scope::State(id as u32)),
            Self::Country(raw) => recorded.saved_country(raw, name)?.map(Scope::Country),
            Self::Leader(reference) => Some(Scope::Leader(reference)),
            Self::Character(id) => Some(Scope::Character(id)),
            Self::Caller => Some(caller),
            Self::Unsupported(kind) => return Err(reader.refusal(name, kind).into()),
        })
    }
}
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
        Selected::Caller
    }
}
impl Evaluator<'_, '_> {
    pub(super) fn saved_scope(
        &self,
        name: &[u8],
        reader: Reader,
        scopes: Scopes,
    ) -> Result<Option<Scopes>, Failure> {
        let recorded = self.world().recorded();
        if let Some(mut this) = self.graph_saved(name, reader, scopes)? {
            if matches!(reader, Reader::CountrySelector)
                && let Scope::CountryLeader { country, .. } = this
            {
                this = Scope::Country(country);
            }
            if matches!(reader, Reader::CountrySelector) && !matches!(this, Scope::Country(_)) {
                return Ok(None);
            }
            if matches!(reader, Reader::Variable)
                && let Scope::Leader(reference) = this
            {
                this = Scope::Character(
                    self.world()
                        .unit_leader(reference)?
                        .ok_or("saved target unit leader is not recorded")?
                        .character_id,
                );
            }
            return self.with_this(scopes, this).map(Some);
        }
        let selected = recorded.saved_selection(name, reader)?;
        selected
            .scope(recorded, scopes.this, name, reader)?
            .map(|this| {
                let mut entered = self.with_this(scopes, this)?;
                if selected == Selected::Caller {
                    entered.bare = scopes.bare;
                }
                Ok(entered)
            })
            .transpose()
    }
}
