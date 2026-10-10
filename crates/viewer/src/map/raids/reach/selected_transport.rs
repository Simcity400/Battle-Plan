//! Consume only reached hourly transport facts. A missing nested graph
//! refuses the complete requested fragment, never supplying zero risk.
use super::*;
use installed_content::unit_path::selected_transport::{
    self as law, Admission, AdmissionInputs, Transfer, Unit,
};
use installed_content::unit_path::{CommonClass, CommonMoveText, CommonMoveWorld};
use replay::rows::{UnitTransferInputs, UnitTransportInputs};

#[path = "selected_transport_carrier.rs"]
mod carrier;
#[path = "selected_transport_graph.rs"]
mod graph;

impl<'w, 'a> Moves<'w, 'a> {
    pub(super) fn transport_message(
        &self,
        selection: &[crate::map::unit_selection::Unit],
        target: u32,
        content: &impl CommonMoveText,
    ) -> Fact<law::Outputs> {
        let selected: Vec<_> = selection
            .iter()
            .map(|unit| match *unit {
                crate::map::unit_selection::Unit::Division(handle) => Unit::Army(handle),
                crate::map::unit_selection::Unit::RailwayGun(handle) => Unit::RailwayGun(handle),
                crate::map::unit_selection::Unit::TaskForce(_) => Unit::Other,
            })
            .collect();
        law::message(
            &selected,
            target,
            false,
            &mut MessageInputs { moves: self },
            content,
        )
    }
    pub(super) fn transport_unit(
        &self,
        class: CommonClass,
        handle: u64,
    ) -> Fact<&UnitTransportInputs> {
        let (packet, name) = match class {
            CommonClass::Army => (
                &self.movement(handle)?.transport,
                "DivisionMovement.transport",
            ),
            CommonClass::RailwayGun => (
                &self.common_gun(handle)?.transport,
                "RailwayGunCounterInputs.transport",
            ),
        };
        let packet = packet
            .require(name)?
            .as_ref()
            .ok_or_else(|| format!("{name} capture refused"))?;
        if packet.sample_hour != self.world.frame().hour {
            return Err(format!("{name} sample is stale"));
        }
        Ok(packet)
    }

    pub(super) fn transport_admission(
        &self,
        class: CommonClass,
        handle: u64,
        source: u32,
        target: u32,
    ) -> Fact<Admission> {
        let unit = match class {
            CommonClass::Army => Unit::Army(handle),
            CommonClass::RailwayGun => Unit::RailwayGun(handle),
        };
        law::admit(
            unit,
            source,
            &mut Inputs {
                moves: self,
                class,
                handle,
                source,
                target,
            },
        )
        .map_err(str::to_owned)
    }

    pub(super) fn transport_reason(
        &self,
        admission: Admission,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        if let Some(reason) = admission.reason {
            if let law::Reason::NotEnough {
                available,
                required,
            } = reason
            {
                text.push_str(&content.transport_stock(available, required)?);
            } else {
                text.push_str(&content.plain(reason.key())?);
            }
        }
        Ok(admission.allowed)
    }

    fn transport_owners(&self, class: CommonClass, handle: u64) -> Fact<(u32, u32)> {
        match class {
            CommonClass::Army => {
                let row = self
                    .division(handle)?
                    .ok_or("selected transport division owner missing")?;
                Ok((row.roster_owner, row.logical_owner))
            }
            CommonClass::RailwayGun => {
                let row = self.common_gun(handle)?;
                Ok((row.roster_owner, row.logical_owner))
            }
        }
    }

    pub(super) fn transport_stock(&self, class: CommonClass, handle: u64) -> Fact<i32> {
        let (roster, logical) = self.transport_owners(class, handle)?;
        let slot = self.slot(logical)?;
        let actual = self.actual_country_raw(slot)?;
        let rows = self.table::<tables::CountryAppearance>()?;
        let index = rows
            .binary_search_by_key(&actual, |row| row.country)
            .map_err(|_| "selected transport original convoy receiver missing")?;
        let value = replay::transport_convoys::current(
            &rows[index].transport_convoys,
            self.world.frame().hour,
        )
        .map_err(str::to_owned)?;
        // Test raw equality first, including raw zero, then aliases only when both
        // references are nonzero. Other logical owners use their independent signed
        // field, separate from counter selection or reservations.
        if roster == logical || (roster != 0 && logical != 0 && self.slot(roster)? == slot) {
            let priority = value
                .priority
                .ok_or("selected transport native transfer priority refused")?;
            replay::transport_convoys::available(value, priority).map_err(str::to_owned)
        } else {
            value
                .unreserved
                .ok_or_else(|| "selected transport alternate convoy count refused".into())
        }
    }

    fn transport_access(&self, class: CommonClass, handle: u64, target: u32) -> Fact<bool> {
        let (_, logical) = self.transport_owners(class, handle)?;
        self.common_transport_access(logical, self.controller_raw(target)?)
    }

    fn transport_missions(&self) -> Fact<replay::transport_mission_view::MissionView<'a>> {
        let rows = self.table::<tables::ReservedCountryNaming>()?;
        if rows.len() != 1 {
            return Err("selected transport original mission singleton is incomplete".into());
        }
        replay::transport_mission_view::MissionView::new(
            &rows[0].transport_mission_inputs,
            self.world.frame().hour,
        )
        .map_err(str::to_owned)
    }

    fn transport_sea_region(&self, province: u32, source: bool) -> Fact<Option<u32>> {
        let content = self.assets.moves.as_ref().map_err(Clone::clone)?;
        let sea = if content.selected_land(province)? {
            self.port_sea(province)?
        } else {
            province
        };
        // Reject a missing target sea. Keep the original source land province
        // when its port-sea lookup is null.
        let resolved = if sea == 0 && source { province } else { sea };
        if resolved == 0 {
            return Ok(None);
        }
        self.province(resolved)?;
        Ok(self.strategic_region(resolved))
    }

    pub(super) fn transport_sea_route(
        &self,
        class: CommonClass,
        handle: u64,
        target: u32,
    ) -> Fact<bool> {
        let Some(target) = self.transport_sea_region(target, false)? else {
            return Ok(false);
        };
        // Use actual unit location independently of the caller's source mode.
        let source = match class {
            CommonClass::Army => {
                self.division(handle)?
                    .ok_or("transport actual source division missing")?
                    .province
            }
            CommonClass::RailwayGun => self.common_gun(handle)?.province,
        };
        let Some(source) = self.transport_sea_region(source, true)? else {
            return Ok(false);
        };
        let missions = self.transport_missions()?;
        let search = missions.naval_search().map_err(str::to_owned)?;
        let (_, logical) = self.transport_owners(class, handle)?;
        // Select the roster owner for strategic navy, even when the logical owner
        // used by adjacency and risk differs.
        let (roster, _) = self.transport_owners(class, handle)?;
        let navy = self.slot(roster)?;
        let content = self.assets.moves.as_ref().map_err(Clone::clone)?;
        installed_content::unit_path::selected_naval_search::admit(
            search,
            source,
            target,
            true,
            |region| {
                missions
                    .access_modes(navy)
                    .map_err(str::to_owned)?
                    .get(region as usize)
                    .copied()
                    .ok_or_else(|| {
                        "transport original access byte outside native navy domain".into()
                    })
            },
            |mode| {
                Ok(missions
                    .access_modes(navy)
                    .map_err(str::to_owned)?
                    .contains(&mode))
            },
            |name| {
                let name = std::str::from_utf8(name)
                    .map_err(|_| "transport adjacency rule name is not UTF-8")?;
                let rule =
                    installed_content::adjacency_rules::find(content.rules(), name).ok_or_else(|| {
                        format!(
                            "transport native adjacency rule {name:?} missing from recorded content"
                        )
                    })?;
                if let Some(reason) = &rule.unsupported {
                    return Err(format!("transport adjacency rule {name}: {reason}"));
                }
                let mover = Mover {
                    graph: content,
                    world: self,
                };
                Ok(rule.permits(
                    mover.common_rule_verdict(rule, self.slot(logical)?)?,
                    installed_content::adjacency_rules::ARMY,
                ))
            },
        )
    }
}

struct MessageInputs<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
}
fn selected_unit(unit: Unit) -> (CommonClass, u64) {
    match unit {
        Unit::Army(handle) => (CommonClass::Army, handle),
        Unit::RailwayGun(handle) => (CommonClass::RailwayGun, handle),
        Unit::Other => unreachable!("selected movement filters other classes before their inputs"),
    }
}
impl law::MessageInputs for MessageInputs<'_, '_, '_> {
    fn target_base(&mut self, target: u32) -> Fact<bool> {
        self.moves.has_naval_base(target)
    }
    fn source(&mut self, unit: Unit) -> Fact<u32> {
        let (class, handle) = selected_unit(unit);
        match class {
            CommonClass::Army => Ok(self
                .moves
                .division(handle)?
                .ok_or("selected transport division actual location missing")?
                .province),
            CommonClass::RailwayGun => Ok(self.moves.common_gun(handle)?.province),
        }
    }
    fn same_area(&mut self, source: u32, target: u32) -> Fact<bool> {
        self.moves.controller_area_equal(source, target)
    }
    fn source_base(&mut self, source: u32) -> Fact<bool> {
        self.moves.has_naval_base(source)
    }
    fn admission(&mut self, unit: Unit, source: u32, target: u32) -> Fact<Admission> {
        let (class, handle) = selected_unit(unit);
        self.moves
            .transport_admission(class, handle, source, target)
    }
    fn weight(&mut self, unit: Unit) -> Fact<i64> {
        let (class, handle) = selected_unit(unit);
        self.moves
            .transport_unit(class, handle)?
            .weight
            .ok_or_else(|| "selected transport all-selection original weight refused".into())
    }
    fn risk(&mut self, unit: Unit, source: u32, target: u32, convoys: i32) -> Fact<i64> {
        installed_content::unit_path::selected_transport_risk::query(
            unit,
            source,
            target,
            convoys,
            &mut RiskInputs { moves: self.moves },
        )
    }
}

struct RiskInputs<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
}
impl installed_content::unit_path::selected_transport_risk::Inputs for RiskInputs<'_, '_, '_> {
    fn route(&mut self, unit: Unit, source: u32, target: u32) -> Fact<Option<Vec<u32>>> {
        let (class, handle) = selected_unit(unit);
        let (roster, logical) = self.moves.transport_owners(class, handle)?;
        let actual = match class {
            CommonClass::Army => {
                self.moves
                    .division(handle)?
                    .ok_or("transport risk actual army missing")?
                    .province
            }
            CommonClass::RailwayGun => self.moves.common_gun(handle)?.province,
        };
        let orders_group = match class {
            CommonClass::Army => {
                self.moves
                    .division(handle)?
                    .ok_or("transport risk army orders group missing")?
                    .orders_group
            }
            // The railway callback borrows its current unit area only if the
            // border-conflict gate reaches it.
            CommonClass::RailwayGun => 0,
        };
        let unit = Army {
            location: actual,
            owner: self.moves.slot(logical)?,
            navy_country: self.moves.slot(roster)?,
            orders_group,
            ..Default::default()
        };
        let graph = self.moves.assets.moves.as_ref().map_err(Clone::clone)?;
        let class = match class {
            CommonClass::Army => CounterClass::Army,
            CommonClass::RailwayGun => CounterClass::Railway(handle),
        };
        Mover {
            graph,
            world: self.moves,
        }
        .transport_risk_route(&unit, source, target, class)
    }
    fn region(&mut self, province: u32) -> Fact<Option<u32>> {
        self.moves.province(province)?;
        Ok(self.moves.strategic_region(province))
    }
    fn province_present(&mut self, province: u32) -> Fact<bool> {
        Ok(self.moves.world.provinces.contains_key(&province))
    }
    fn enemies(&mut self, unit: Unit) -> Fact<Vec<u32>> {
        let (class, handle) = selected_unit(unit);
        let (_, logical) = self.moves.transport_owners(class, handle)?;
        installed_content::unit_path::CommonMoveWorld::ordered_enemy_raws(
            self.moves,
            self.moves.slot(logical)?,
        )
    }
    fn ledger(&mut self, enemy: u32, region: u32) -> Fact<Vec<u64>> {
        Ok(self
            .moves
            .transport_missions()?
            .ledger(self.moves.slot(enemy)?, region)
            .map_err(str::to_owned)?
            .to_vec())
    }
    fn mission_chance(&mut self, handle: u64, unit: Unit, convoys: i32) -> Fact<i64> {
        let missions = self.moves.transport_missions()?;
        let receiver = missions.receiver(handle).map_err(str::to_owned)?;
        if !replay::transport_mission_view::demanded(receiver).map_err(str::to_owned)? {
            return Ok(0);
        }
        let (class, handle) = selected_unit(unit);
        let (_, target_owner) = self.moves.transport_owners(class, handle)?;
        graph::chance(self.moves, receiver, target_owner, convoys)
    }
    fn distance(&mut self, _: Unit, previous: Option<u32>, current: u32) -> Fact<i64> {
        let previous =
            previous.ok_or("selected transport exposure reached a null previous province")?;
        let content = self
            .moves
            .assets
            .transport_exposure
            .as_ref()
            .map_err(Clone::clone)?;
        self.moves
            .assets
            .moves
            .as_ref()
            .map_err(Clone::clone)?
            .transport_exposure_distance(previous, current, content.embark_distance)
    }
    fn speed(&mut self, unit: Unit, previous_region: Option<u32>) -> Fact<i64> {
        let region =
            previous_region.ok_or("selected transport speed reached a null previous region")?;
        let (class, handle) = selected_unit(unit);
        let (_, logical) = self.moves.transport_owners(class, handle)?;
        if logical as i32 <= 0 {
            return Err(
                "selected transport speed owner is not a supported positive reference".into(),
            );
        }
        let actual = self.moves.actual_country_raw(self.moves.slot(logical)?)?;
        let rows = self.moves.table::<tables::CountryCounterInputs>()?;
        let index = rows
            .binary_search_by_key(&actual, |row| row.country)
            .map_err(|_| "selected transport speed country packet missing")?;
        let packet = rows[index]
            .movement
            .require("CountryCounterInputs.movement")?
            .as_ref()
            .ok_or("selected transport speed owner capture refused")?;
        if packet.sample_hour != self.moves.world.frame().hour {
            return Err("selected transport speed owner sample is stale".into());
        }
        // The original owner demand includes authenticated idle divisions and
        // railway guns. Its domain covers every original strategic region.
        let domain = packet
            .transport
            .as_ref()
            .ok_or("selected transport speed region domain refused")?;
        if domain
            .windows(2)
            .any(|pair| pair[0].region >= pair[1].region)
        {
            return Err("selected transport speed region keys are not unique/increasing".into());
        }
        let index = domain
            .binary_search_by_key(&region, |row| row.region)
            .map_err(|_| "selected transport speed previous region outside captured domain")?;
        let speed = domain[index]
            .speed
            .ok_or("selected transport speed previous region capture refused")?;
        let scale = self
            .moves
            .assets
            .transport_exposure
            .as_ref()
            .map_err(Clone::clone)?
            .speed_scale;
        Ok(speed.wrapping_mul(scale) / 100_000)
    }
}

struct Inputs<'m, 'w, 'a> {
    moves: &'m Moves<'w, 'a>,
    class: CommonClass,
    handle: u64,
    source: u32,
    target: u32,
}

// This interface retains static native refusal names at the pure-law boundary.
// Existing frame/content errors remain refusals, never converted to false.
impl AdmissionInputs for Inputs<'_, '_, '_> {
    fn target_base(&mut self) -> law::Fact<bool> {
        self.moves
            .has_naval_base(self.target)
            .map_err(|_| "selected transport target naval-base census unavailable")
    }
    fn source_land(&mut self) -> law::Fact<bool> {
        self.moves
            .assets
            .moves
            .as_ref()
            .map_err(|_| "selected transport source content graph unavailable")?
            .selected_land(self.source)
            .map_err(|_| "selected transport source province/terrain unavailable")
    }
    fn source_base(&mut self) -> law::Fact<bool> {
        self.moves
            .has_naval_base(self.source)
            .map_err(|_| "selected transport source naval-base census unavailable")
    }
    fn coastal_special_template(&mut self) -> law::Fact<bool> {
        self.moves
            .transport_unit(self.class, self.handle)
            .map_err(|_| "selected transport current-hour unit packet unavailable")?
            .coastal_template
            .ok_or("selected transport coastal-template predicate refused")
    }
    fn transfer(&mut self) -> law::Fact<Option<Transfer>> {
        match self
            .moves
            .transport_unit(self.class, self.handle)
            .map_err(|_| "selected transport current-hour unit packet unavailable")?
            .transfer
            .ok_or("selected transport original transfer selector/identity refused")?
        {
            UnitTransferInputs::Absent => Ok(None),
            UnitTransferInputs::Present { status, source, .. } => Ok(Some(Transfer {
                status: status.ok_or("selected transport transfer status unreadable"),
                source: source.ok_or("selected transport transfer source unreadable or unreached"),
            })),
        }
    }
    fn weight(&mut self) -> law::Fact<i64> {
        self.moves
            .transport_unit(self.class, self.handle)
            .map_err(|_| "selected transport current-hour unit packet unavailable")?
            .weight
            .ok_or("selected transport original stat weight refused")
    }
    fn available_convoys(&mut self) -> law::Fact<i32> {
        self.moves
            .transport_stock(self.class, self.handle)
            .map_err(|_| "selected transport original logical-owner convoy stock refused")
    }
    fn military_access(&mut self) -> law::Fact<bool> {
        self.moves
            .transport_access(self.class, self.handle, self.target)
            .map_err(|_| "selected transport current receiver access refused")
    }
    fn sea_route(&mut self) -> law::Fact<bool> {
        self.moves
            .transport_sea_route(self.class, self.handle, self.target)
            .map_err(|_| "selected transport naval search inputs refused")
    }
}

#[cfg(test)]
#[path = "selected_transport_tests.rs"]
mod tests;
