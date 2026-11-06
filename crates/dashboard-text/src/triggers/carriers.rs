//! Recorded identities for event scopes' project, raid, MIO and unit arms.
use crate::triggers::Failure;

use super::{Evaluator, Scope, World, world::table};
use replay_data::{
    rows::IndustrialOrganisationRow, rows::ProjectRow, rows::RaidRow, rows::TableSchema, rows::mask,
    rows::tables,
};
use std::borrow::Cow;

/// The fields of a project or an organisation a named or decoded carrier reads.
const PROJECT: u64 = mask::<ProjectRow>(&["definition_id"]);
const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["country", "ordinal", "token"]);

impl<'a> World<'a> {
    /// Projects retains the pool owner and local id.
    pub(crate) fn project(&self, owner: u32, id: u32) -> Result<Cow<'a, ProjectRow>, Failure> {
        // Keyed by (owner, local id); its readers read any of its fields.
        table::<tables::Projects>(self.tables(), tables::Projects::ID.full_mask())?
            .get((u128::from(owner) << 32) | u128::from(id))
            .map_err(|why| super::refusal::input_error(tables::Projects::ID, why))?
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Projects, None)
            })
    }

    /// Raid lookup resolves the full reference, not its id alone.
    pub(crate) fn raid(&self, reference: u64) -> Result<Cow<'a, RaidRow>, Failure> {
        // Keyed by (id, reference type), the full reference.
        table::<tables::Raids>(self.tables(), tables::Raids::ID.full_mask())?
            .get(u128::from(reference))
            .map_err(|why| super::refusal::input_error(tables::Raids::ID, why))?
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Raids, None)
            })
    }

    /// Divisions records each army's full unit handle,
    /// including its reference type.
    pub(crate) fn unit(&self, reference: u64) -> Result<replay_frame::frame::Division, Failure> {
        const DIVISION: u64 = mask::<replay_data::rows::DivisionRow>(&[
            "roster_owner",
            "logical_owner",
            "expeditionary_owner",
            "province",
            "template",
        ]);
        // Keyed by the unit handle.
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
    /// Named-carrier keyword prefixes: sp and mio.
    /// The caller checks country presence before either lookup.
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
                // Take the first token match in the country's organisation roster.
                let rows = table::<tables::IndustrialOrganisations>(world.tables(), ORGANISATION)
                    .map_err(|why| why.context(|why| format!("mio:{name}: {why}")))?;
                // The first least ordinal, as a minimum by ordinal finds it.
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
                // Index the country's pool by definition token.
                // Each project's definition_id retains its template token.
                // Database resynchronisation reuses a matching definition
                // before appending; reject a recording that cannot identify it.
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

    /// Decode project, organisation or raid references from
    /// the tagged low-28-bit identifier. Project keys additionally retain owner.
    pub(super) fn decoded_carrier(&self, kind: u32, id: u32) -> Result<Option<Scope>, Failure> {
        let world = self.world();
        match kind {
            engine_projects::nproject_cproject::REFERENCE_TYPE => {
                // Keyed by (owner, local id): the local id is the key's low half.
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
                // Keyed by the organisation's full reference.
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
            engine_native::craid_instance::REFERENCE_TYPE_VALUE => Ok(super::refusal::reference(
                // Keyed by (id, reference type).
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
