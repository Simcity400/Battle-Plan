
use std::cell::OnceCell;
use viewer_unit_reads::air_range;

use replay_data::{
    rows::AceRangeInputRow, rows::AirWingRow, rows::ArmyOrderRow, rows::CountryCommandPowerRow,
    rows::OrdersGroupRow, rows::RaidCountryInputRow, rows::TaskForceRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use super::transport_counts::{self, Origin, RegionGeometry};
use viewer_unit_reads::{
    air_registry_sources::AirRegistrySources,
    air_transport_content::AirTransportContent,
};

pub(super) struct Context<'a> {
    frame: &'a DecodedFrame,
    content: Option<AirTransportContent<'a>>,
    registry: OnceCell<Result<AirRegistrySources, String>>,
}

impl<'a> Context<'a> {
    pub(super) fn new(frame: &'a DecodedFrame, content: Option<AirTransportContent<'a>>) -> Self {
        Self {
            frame,
            content,
            registry: OnceCell::new(),
        }
    }

    pub(super) fn count(
        &self,
        order: &ArmyOrderRow,
        group: Option<&OrdersGroupRow>,
    ) -> Result<i32, String> {
        Ok(self.totals(order, group)?.plane_count)
    }

    pub(super) fn totals(
        &self,
        order: &ArmyOrderRow,
        group: Option<&OrdersGroupRow>,
    ) -> Result<transport_counts::AcceptedTotals, String> {
        if (order.paradrop_target as i32) < 1 {
            return Ok(transport_counts::AcceptedTotals::default());
        }
        if self.content.as_ref().is_some_and(|content| {
            !content
                .geometry
                .centroids
                .contains_key(&order.paradrop_target)
        }) {
            return Ok(transport_counts::AcceptedTotals::default());
        }
        let provinces = self.frame.complete::<tables::Provinces>().map_err(error)?;
        let source = provinces
            .binary_search_by_key(&order.paradrop_target, |row| row.id)
            .ok()
            .map(|at| &provinces[at]);
        let Some(source) = source else {
            return Err("the paradrop source province has no Provinces row".into());
        };
        if source.state == 0 {
            return Ok(transport_counts::AcceptedTotals::default());
        }
        let base = self
            .frame
            .complete::<tables::AirBases>()
            .map_err(error)?
            .iter()
            .find(|base| base.kind == 0 && base.state == Some(source.state));
        let Some(base) = base else {
            return Ok(transport_counts::AcceptedTotals::default());
        };
        let group = group.ok_or("the paradrop's owner has no OrdersGroups row")?;
        let slot = if group.country == 0 {
            0
        } else {
            self.slot(group.country)?
        };
        let registrations = self
            .frame
            .complete::<tables::AirBaseRegistrations>()
            .map_err(error)?;
        let registration = registrations
            .binary_search_by_key(&(base.id, slot), |row| (row.base, row.slot))
            .ok()
            .map(|at| &registrations[at]);
        let Some(registration) = registration else {
            return Ok(transport_counts::AcceptedTotals::default());
        };
        if registration.wings.is_empty() {
            return Ok(transport_counts::AcceptedTotals::default());
        }
        let destination = order.path.first().copied().unwrap_or(0);
        let distance = if (destination as i32) > 0 {
            self.content.as_ref().map_or(0, |content| {
                transport_counts::distance(content.geometry, source.id, destination)
            })
        } else {
            0
        };
        let valid_destination = (destination as i32) > 0
            && self
                .content
                .as_ref()
                .is_none_or(|content| content.geometry.centroids.contains_key(&destination));
        let region = self
            .content
            .as_ref()
            .filter(|_| valid_destination)
            .and_then(|content| content.target_region(destination));
        let has_region = valid_destination && (self.content.is_none() || region.is_some());
        let sources = Sources {
            context: self,
            region,
            region_geometry: OnceCell::new(),
        };
        transport_counts::totals(&registration.wings, distance, has_region, &sources)
    }

    fn content(&self) -> Result<&AirTransportContent<'a>, String> {
        self.content
            .as_ref()
            .ok_or_else(|| "the paradrop transport content is unavailable".into())
    }

    fn slot(&self, raw: u32) -> Result<u32, String> {
        let links = self
            .frame
            .complete::<tables::CountryLinks>()
            .map_err(error)?;
        links
            .binary_search_by_key(&raw, |row| row.id)
            .ok()
            .map(|at| links[at].canonical_slot)
            .ok_or_else(|| format!("country {raw} has no CountryLinks row"))
    }

    fn registry(&self) -> Result<&AirRegistrySources, String> {
        self.registry
            .get_or_init(|| {
                AirRegistrySources::read(self.frame)
                    .map_err(error)?
                    .ok_or_else(|| {
                        "the paradrop's AirReferenceWitnesses table is not complete".into()
                    })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String> {
        let wings = self.frame.complete::<tables::AirWings>().map_err(error)?;
        Ok(wings
            .binary_search_by_key(&handle, |row| row.id)
            .ok()
            .map(|at| &wings[at]))
    }

    fn wing_range(&self, wing: &AirWingRow) -> Result<i64, String> {
        self.frame
            .complete::<tables::CountryLinks>()
            .map_err(error)?;
        let defines = self.content()?;
        air_range::wing_range(
            wing,
            air_range::Defines {
                map_scale: defines.map_scale,
                ace_wing_size: defines.ace_wing_size,
                ace_wing_size_max_bonus: defines.ace_wing_size_max_bonus,
            },
            self,
        )
    }

    fn host_force(&self, base: u64, host: u64) -> Result<Option<&TaskForceRow>, String> {
        if host == 0 {
            return Ok(None);
        }
        let registry = self.registry()?;
        if registry.hosts_without_task_force.contains(&base) {
            return Err("the paradrop wing's carrier host has a null task force".into());
        }
        if !registry.host_owners.contains_key(&base) {
            return Ok(None);
        }
        let ships = self.frame.complete::<tables::Ships>().map_err(error)?;
        let ship = ships
            .binary_search_by_key(&host, |row| row.id)
            .ok()
            .map(|at| &ships[at])
            .ok_or("the resolved paradrop carrier host has no Ships row")?;
        if ship.task_force == 0 {
            return Err("the resolved paradrop carrier host has a null task force".into());
        }
        let forces = self.frame.complete::<tables::TaskForces>().map_err(error)?;
        forces
            .binary_search_by_key(&ship.task_force, |row| row.id)
            .ok()
            .map(|at| Some(&forces[at]))
            .ok_or_else(|| "the paradrop carrier's task force has no TaskForces row".into())
    }
}

struct Sources<'c, 'a> {
    context: &'c Context<'a>,
    region: Option<&'c [u32]>,
    region_geometry: OnceCell<RegionGeometry>,
}

impl transport_counts::Sources for Sources<'_, '_> {
    fn wing(&self, handle: u64) -> Result<&AirWingRow, String> {
        self.context
            .wing(handle)?
            .ok_or_else(|| format!("registered paradrop wing {handle:#x} has no AirWings row"))
    }

    fn is_transport_pool(&self, wing: &AirWingRow) -> Result<bool, String> {
        let index = wing
            .pool_subunit
            .require("AirWings.pool_subunit")?
            .ok_or("the paradrop wing's pool sub-unit identity is unavailable")?;
        if index == 0 {
            Ok(false)
        } else {
            self.context.content()?.transport_pool(index)
        }
    }

    fn range(&self, wing: &AirWingRow) -> Result<i64, String> {
        self.context.wing_range(wing)
    }

    fn transfer_resolves(&self, handle: u64) -> Result<bool, String> {
        Ok(self.context.registry()?.transfer_resolves(handle))
    }

    fn region_admits(&self, wing: &AirWingRow) -> Result<bool, String> {
        if wing.air_base == 0 {
            return Ok(false);
        }
        let registry = self.context.registry()?;
        let Some(base) = registry.bases.get(&wing.air_base) else {
            return Ok(false);
        };
        let host = self.context.host_force(base.id, base.host)?;
        if host.is_some_and(|force| force.mission != 0) {
            return Ok(false);
        }
        let raid = wing
            .raid_reference
            .require("AirWings.raid_reference")?
            .ok_or("the paradrop wing's raid reference is unavailable")?;
        if raid.resolves || wing.mission_mask & (1 << 11) != 0 {
            return Ok(false);
        }
        let geometry = self.context.content()?.geometry;
        let origin = if let Some(force) = host {
            Origin::Host(
                geometry
                    .centroids
                    .get(&force.province)
                    .copied()
                    .unwrap_or([0; 2]),
            )
        } else if let Some(state) = base.state {
            let provinces = self
                .context
                .content()?
                .state_provinces
                .get(&state)
                .ok_or_else(|| {
                    format!("paradrop pool-base state {state} is not in the recorded content")
                })?;
            Origin::State(geometry.state_centre(provinces).unwrap_or([0; 2]))
        } else {
            return Ok(false);
        };
        let region = self
            .region
            .ok_or("the paradrop target region was not supplied")?;
        Ok(self
            .region_geometry
            .get_or_init(|| RegionGeometry::new(geometry, region))
            .reaches(geometry, region, origin, self.context.wing_range(wing)?))
    }
}

impl air_range::Sources for Context<'_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        Context::slot(self, raw).ok()
    }

    fn country(&self, slot: u32) -> Result<&RaidCountryInputRow, String> {
        let rows = self
            .frame
            .complete::<tables::RaidCountryInputs>()
            .map_err(error)?;
        rows.binary_search_by_key(&slot, |row| row.canonical_slot)
            .ok()
            .map(|at| &rows[at])
            .ok_or_else(|| format!("country slot {slot} has no RaidCountryInputs row"))
    }

    fn command_power(&self, slot: u32) -> Result<&CountryCommandPowerRow, String> {
        let rows = self
            .frame
            .complete::<tables::CountryCommandPower>()
            .map_err(error)?;
        rows.binary_search_by_key(&slot, |row| row.canonical_slot)
            .ok()
            .map(|at| &rows[at])
            .ok_or_else(|| format!("country slot {slot} has no CountryCommandPower row"))
    }

    fn wing(&self, handle: u64) -> Result<Option<&AirWingRow>, String> {
        Context::wing(self, handle)
    }

    fn ace(&self, handle: u64) -> Result<Option<&AceRangeInputRow>, String> {
        let rows = self
            .frame
            .complete::<tables::AceRangeInputs>()
            .map_err(error)?;
        Ok(rows
            .binary_search_by_key(&handle, |row| row.ace)
            .ok()
            .map(|at| &rows[at]))
    }
}

fn error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
pub(super) mod tests;
