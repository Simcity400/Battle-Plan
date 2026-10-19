//! Recorded identities for event scopes' project, raid, MIO and unit arms.
use crate::triggers::{Failure, refusal::FrameInputs};

use super::{Evaluator, Scope, World};
use replay_data::{rows::DivisionRow, rows::ProjectRow, rows::RaidRow, rows::tables};

impl<'a> World<'a> {
    /// Projects retains the pool owner and local id.
    pub(crate) fn project(&self, owner: u32, id: u32) -> Result<&'a ProjectRow, Failure> {
        self.frame()
            .trigger_table::<tables::Projects>()?
            .iter()
            .find(|row| row.owner == owner && row.local_id == id)
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Projects, None)
            })
    }

    /// Raid lookup resolves the full reference, not its id alone.
    pub(crate) fn raid(&self, reference: u64) -> Result<&'a RaidRow, Failure> {
        self.frame()
            .trigger_table::<tables::Raids>()?
            .iter()
            .find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == reference)
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(replay_data::rows::TableId::Raids, None)
            })
    }

    /// Divisions records each army's full unit handle,
    /// including its reference type.
    pub(crate) fn unit(&self, reference: u64) -> Result<&'a DivisionRow, Failure> {
        self.frame()
            .trigger_table::<tables::Divisions>()?
            .iter()
            .find(|row| row.id == reference)
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
                let rows = world
                    .frame()
                    .trigger_table::<tables::IndustrialOrganisations>()
                    .map_err(|why| why.context(|why| format!("mio:{name}: {why}")))?;
                Ok(rows
                    .iter()
                    .filter(|row| {
                        world.slot(row.country) == Some(country) && Some(row.token) == token
                    })
                    .min_by_key(|row| row.ordinal)
                    .map(|row| Scope::IndustrialOrganisation(row.organisation)))
            }
            "sp" => {
                // Index the country's pool by definition token.
                // Each project's definition_id retains its template token.
                // Database resynchronisation reuses a matching definition
                // before appending; reject a recording that cannot identify it.
                let rows = world
                    .frame()
                    .trigger_table::<tables::Projects>()
                    .map_err(|why| why.context(|why| format!("sp:{name}: {why}")))?;
                let mut found = rows.iter().filter(|row| {
                    world.slot(row.owner) == Some(country) && Some(row.definition_id) == token
                });
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
                let mut found = world
                    .frame()
                    .trigger_table::<tables::Projects>()?
                    .iter()
                    .filter(|row| row.local_id == id);
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
                world
                    .frame()
                    .trigger_table::<tables::IndustrialOrganisations>()?
                    .iter()
                    .find(|row| row.organisation == (u64::from(id) << 32 | u64::from(kind)))
                    .map(|row| Scope::IndustrialOrganisation(row.organisation)),
                id != 0,
                || {
                    super::refusal::missing_reference(
                        replay_data::rows::TableId::IndustrialOrganisations,
                        None,
                    )
                },
            )),
            engine_native::craid_instance::REFERENCE_TYPE_VALUE => Ok(super::refusal::reference(
                world
                    .frame()
                    .trigger_table::<tables::Raids>()?
                    .iter()
                    .find(|row| row.id == id && row.reference_type == kind)
                    .map(|_| Scope::Raid(u64::from(id) << 32 | u64::from(kind))),
                id != 0,
                || super::refusal::missing_reference(replay_data::rows::TableId::Raids, None),
            )),
            _ => unreachable!("native project/MIO/raid discriminator"),
        }
    }
}
