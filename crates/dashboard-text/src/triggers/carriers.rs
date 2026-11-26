use crate::triggers::Failure;

use super::{Evaluator, Scope, World, world::table};
use replay_data::{
    rows::IndustrialOrganisationRow, rows::ProjectRow, rows::RaidRow, rows::TableSchema, rows::mask,
    rows::tables,
};
use std::borrow::Cow;

const PROJECT: u64 = mask::<ProjectRow>(&["definition_id"]);
const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["country", "ordinal", "token"]);

impl<'a> World<'a> {
    pub fn project(&self, owner: u32, id: u32) -> Result<Cow<'a, ProjectRow>, Failure> {
        if let Some(accepted) = self.accepted_projects(super::ProjectScope::Owner(owner)) {
            return match accepted.rows.get(&(owner, id)) {
                Some(row) => Ok(Cow::Owned(row.clone())),
                None if accepted.owners.contains(&owner) => Err(super::refusal::missing_reference(replay_data::rows::TableId::Projects, None)),
                None => Err(super::refusal::refused("the project's owner has no Projects read supplying this hour".to_owned())),
            };
        }
        table::<tables::Projects>(self.tables(), tables::Projects::ID.full_mask())?
            .get((u128::from(owner) << 32) | u128::from(id))
            .map_err(|why| super::refusal::input_error(tables::Projects::ID, why))?
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Projects, None)
            })
    }

    pub fn raid(&self, reference: u64) -> Result<Cow<'a, RaidRow>, Failure> {
        table::<tables::Raids>(self.tables(), tables::Raids::ID.full_mask())?
            .get(u128::from(reference))
            .map_err(|why| super::refusal::input_error(tables::Raids::ID, why))?
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Raids, None)
            })
    }

    pub fn unit(&self, reference: u64) -> Result<replay_frame::frame::Division, Failure> {
        const DIVISION: u64 = mask::<replay_data::rows::DivisionRow>(&[
            "roster_owner",
            "logical_owner",
            "expeditionary_owner",
            "province",
            "template",
        ]);
        self.tables()
            .complete::<tables::Divisions>(DIVISION)
            .and_then(|rows| rows.get(u128::from(reference)))
            .map_err(|why| super::refusal::input_error(replay_data::rows::TableId::Divisions, why))?
            .map(|row| replay_frame::frame::Division::from(&*row))
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Divisions, None)
            })
    }
}

impl Evaluator<'_, '_> {
    pub(super) fn named_carrier(
        &self,
        prefix: &str,
        name: &str,
        country: u32,
    ) -> Result<Option<Scope>, Failure> {
        let world = self.world();
        for table in ["Countries", "CountryLinks"] {
            if let Some(why) = world.table_gap(table) {
                return Err(why.context(|why| format!("{prefix}:{name}: {why}")));
            }
        }
        let token = world.token_of(name.as_bytes())?;
        match prefix {
            "mio" => {
                let rows = table::<tables::IndustrialOrganisations>(world.tables(), ORGANISATION)
                    .map_err(|why| why.context(|why| format!("mio:{name}: {why}")))?;
                let mut first: Option<(u32, u64)> = None;
                for row in rows.iter() {
                    let row = row.map_err(|why| {
                        super::refusal::input_error(tables::IndustrialOrganisations::ID, why)
                    })?;
                    if world.slot(row.country) == Some(country)
                        && Some(row.token) == token
                        && first.is_none_or(|(ordinal, _)| row.ordinal < ordinal)
                    {
                        first = Some((row.ordinal, row.organisation));
                    }
                }
                Ok(first.map(|(_, organisation)| Scope::IndustrialOrganisation(organisation)))
            }
            "sp" => {
                let rows = table::<tables::Projects>(world.tables(), PROJECT)
                    .map_err(|why| why.context(|why| format!("sp:{name}: {why}")))?;
                let mut found = Vec::new();
                for row in rows.iter() {
                    let row =
                        row.map_err(|why| super::refusal::input_error(tables::Projects::ID, why))?;
                    if world.slot(row.owner) == Some(country) && Some(row.definition_id) == token {
                        found.push(row);
                        if found.len() > 1 {
                            break;
                        }
                    }
                }
                let mut found = found.into_iter();
                let row = found.next();
                if found.next().is_some() {
                    return Err(format!(
                        "sp:{name}: Projects has ambiguous duplicate definitions for one country"
                    )
                    .into());
                }
                Ok(row.map(|row| Scope::Project {
                    owner: row.owner,
                    id: row.local_id,
                }))
            }
            _ => unreachable!("only the two native named-carrier prefixes"),
        }
    }

    pub(super) fn decoded_carrier(&self, kind: u32, id: u32) -> Result<Option<Scope>, Failure> {
        let world = self.world();
        match kind {
            engine_projects::nproject_cproject::REFERENCE_TYPE => {
                let mut found = table::<tables::Projects>(world.tables(), 0)?
                    .keys()
                    .filter(|&key| key as u32 == id)
                    .map(|key| ProjectRow {
                        owner: (key >> 32) as u32,
                        local_id: key as u32,
                        ..Default::default()
                    });
                let row = super::refusal::reference(found.next(), id != 0, || {
                    super::refusal::missing_reference(replay_data::rows::TableId::Projects, None)
                });
                if found.next().is_some() {
                    return Err(format!(
                        "project reference id {id} is ambiguous across recorded country pools"
                    )
                    .into());
                }
                Ok(row.map(|row| Scope::Project {
                    owner: row.owner,
                    id: row.local_id,
                }))
            }
            79 => Ok(super::refusal::reference(
                table::<tables::IndustrialOrganisations>(world.tables(), 0)?
                    .contains(u128::from(u64::from(id) << 32 | u64::from(kind)))
                    .then(|| Scope::IndustrialOrganisation(u64::from(id) << 32 | u64::from(kind))),
                id != 0,
                || {
                    super::refusal::missing_reference(
                        replay_data::rows::TableId::IndustrialOrganisations,
                        None,
                    )
                },
            )),
            engine_native_air::craid_instance::REFERENCE_TYPE_VALUE => Ok(super::refusal::reference(
                table::<tables::Raids>(world.tables(), 0)?
                    .contains(u128::from(u64::from(id) << 32 | u64::from(kind)))
                    .then(|| Scope::Raid(u64::from(id) << 32 | u64::from(kind))),
                id != 0,
                || super::refusal::missing_reference(replay_data::rows::TableId::Raids, None),
            )),
            _ => unreachable!("native project/MIO/raid discriminator"),
        }
    }
}
