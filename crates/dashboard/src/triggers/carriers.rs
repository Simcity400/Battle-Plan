//! Recorded identities for CEventScope's project, raid, MIO and unit arms.

use super::{Evaluator, Scope, World};
use replay::rows::{DivisionRow, ProjectRow, RaidRow, tables};

impl<'a> World<'a> {
    /// ResolveProject@140529480; Projects retains the pool owner and +0x0C id.
    pub(crate) fn project(&self, owner: u32, id: u32) -> Result<&'a ProjectRow, String> {
        self.frame().complete::<tables::Projects>().map_err(|why| why.to_string())?
            .iter().find(|row| row.owner == owner && row.local_id == id)
            .ok_or_else(|| format!("Projects lacks project {owner}:{id}"))
    }

    /// ResolveRaidInstance@140529460 resolves the full reference, not its id alone.
    pub(crate) fn raid(&self, reference: u64) -> Result<&'a RaidRow, String> {
        self.frame().complete::<tables::Raids>().map_err(|why| why.to_string())?
            .iter().find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == reference)
            .ok_or_else(|| format!("Raids lacks raid {reference:#x}"))
    }

    /// ResolveUnit@1405294C0 subtracts 0x10 from the registry object. Divisions
    /// records that CArmy's full CUnit+0x18 handle, including its reference type.
    pub(crate) fn unit(&self, reference: u64) -> Result<&'a DivisionRow, String> {
        self.frame().complete::<tables::Divisions>().map_err(|why| why.to_string())?
            .iter().find(|row| row.id == reference)
            .ok_or_else(|| format!("Divisions lacks UNIT {reference:#x}; non-army unit identities are not recorded here (1405294C0)"))
    }
}

impl Evaluator<'_, '_> {
    /// 14146BB30's keyword-prefix arms: sp (0x4D38) and mio (0x4D37).
    /// The caller checks country presence before either lookup.
    pub(super) fn named_carrier(
        &self, prefix: &str, name: &str, country: u32,
    ) -> Result<Option<Scope>, String> {
        let world = self.world();
        for table in ["Countries", "CountryLinks"] {
            if let Some(why) = world.table_gap(table) {
                return Err(format!("{prefix}:{name}: {why}"));
            }
        }
        let token = world.token_of(name.as_bytes())?;
        match prefix {
            "mio" => {
                // 140E48790: first token match in country-status +0x130/+0x13C.
                let rows = world.frame().complete::<tables::IndustrialOrganisations>()
                    .map_err(|why| format!("mio:{name}: {why}"))?;
                Ok(rows.iter()
                    .filter(|row| world.slot(row.country) == Some(country) && Some(row.token) == token)
                    .min_by_key(|row| row.ordinal)
                    .map(|row| Scope::IndustrialOrganisation(row.organisation)))
            }
            "sp" => {
                // 1414700A0: definition-token index into the country's pool.
                // 140FCADC0 copies template+8 to project+0x18 (definition_id).
                // ResyncWithDatabase 14146F470 reuses a matching definition
                // before appending; reject a recording that cannot identify it.
                let rows = world.frame().complete::<tables::Projects>()
                    .map_err(|why| format!("sp:{name}: {why}"))?;
                let mut found = rows.iter().filter(|row|
                    world.slot(row.owner) == Some(country) && Some(row.definition_id) == token);
                let row = found.next();
                if found.next().is_some() {
                    return Err(format!("sp:{name}: Projects has ambiguous duplicate definitions for one country (14146F470/1414700A0)"));
                }
                Ok(row.map(|row| Scope::Project { owner: row.owner, id: row.local_id }))
            }
            _ => unreachable!("only the two native named-carrier prefixes"),
        }
    }

    /// DecodeScopedValueToEventScope@140537A10: type 86, 79 or 84 from
    /// the tagged low-28-bit identifier. Project keys additionally retain owner.
    pub(super) fn decoded_carrier(&self, kind: u32, id: u32) -> Result<Option<Scope>, String> {
        let world = self.world();
        match kind {
            engine::nproject_cproject::REFERENCE_TYPE => {
                let mut found = world.frame().complete::<tables::Projects>()
                    .map_err(|why| why.to_string())?.iter().filter(|row| row.local_id == id);
                let row = found.next();
                if found.next().is_some() {
                    return Err(format!("project reference id {id} is ambiguous across recorded country pools (140537A10)"));
                }
                Ok(row.map(|row| Scope::Project { owner: row.owner, id: row.local_id }))
            }
            79 => Ok(world.frame().complete::<tables::IndustrialOrganisations>()
                .map_err(|why| why.to_string())?.iter()
                .find(|row| row.organisation == (u64::from(id) << 32 | u64::from(kind)))
                .map(|row| Scope::IndustrialOrganisation(row.organisation))),
            engine::craid_instance::REFERENCE_TYPE_VALUE => Ok(world.frame().complete::<tables::Raids>()
                .map_err(|why| why.to_string())?.iter()
                .find(|row| row.id == id && row.reference_type == kind)
                .map(|row| Scope::Raid(u64::from(id) << 32 | u64::from(kind)))),
            _ => unreachable!("native project/MIO/raid discriminator"),
        }
    }
}
