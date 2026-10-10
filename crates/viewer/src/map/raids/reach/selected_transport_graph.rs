//! F9EE50's reached spotting/interception caller. Independent original raw
//! domains are joined only after the native caller reaches each operand.
use super::*;
use installed_content::unit_path::CommonMoveWorld;
use replay::rows::{
    Recorded, TransportConvoyVariant, TransportMissionInputs, TransportMissionReceiver,
    TransportReceiverRisk, TransportSpottingRegion,
};
use replay::{transport_damage_graph as damage, transport_spotting_graph as spotting};

pub(super) fn chance(
    moves: &Moves<'_, '_>,
    receiver: &TransportMissionReceiver,
    target_owner: u32,
    convoys: i32,
) -> Fact<i64> {
    let mut input = Inputs {
        moves,
        receiver,
        target_owner,
    };
    let spotting = spotting::query(&mut input).map_err(str::to_owned)?;
    let defines = *moves
        .assets
        .transport_damage
        .as_ref()
        .map_err(Clone::clone)?;
    let damage = damage::query(&mut input, defines, convoys).map_err(str::to_owned)?;
    let weights = moves
        .assets
        .transport_chance
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(
        installed_content::unit_path::selected_transport_risk::mission_chance(
            spotting,
            damage,
            weights.spotting_weight,
            weights.damage_weight,
            weights.base,
        ),
    )
}

struct Inputs<'m, 'w, 'a, 'r> {
    moves: &'m Moves<'w, 'a>,
    receiver: &'r TransportMissionReceiver,
    target_owner: u32,
}
impl Inputs<'_, '_, '_, '_> {
    fn context(&self) -> damage::Fact<replay::transport_spotting_context::View<'_>> {
        replay::transport_spotting_context::View::new(
            self.packet()?.spotting_context.as_ref(),
            self.moves.world.frame().hour,
        )
    }
    fn packet(&self) -> damage::Fact<&TransportMissionInputs> {
        let rows = self
            .moves
            .table::<tables::ReservedCountryNaming>()
            .map_err(|_| "transport original mission singleton refused")?;
        if rows.len() != 1 {
            return Err("transport original mission singleton is incomplete");
        }
        let packet = match &rows[0].transport_mission_inputs {
            Recorded::Value(Some(packet)) => packet,
            _ => return Err("transport original mission packet unavailable"),
        };
        if packet.sample_hour != self.moves.world.frame().hour {
            return Err("transport original mission packet has wrong sample hour");
        }
        Ok(packet)
    }
    fn region_context(&self, region: u32) -> damage::Fact<&TransportSpottingRegion> {
        let context = self
            .packet()?
            .spotting_context
            .as_ref()
            .ok_or("transport original regional spotting context refused")?;
        if context.sample_hour != self.moves.world.frame().hour || !context.regions_complete {
            return Err("transport original regional spotting context is stale or incomplete");
        }
        if context
            .regions
            .windows(2)
            .any(|rows| rows[0].region_id >= rows[1].region_id)
        {
            return Err("transport original regional spotting context domain malformed");
        }
        context
            .regions
            .binary_search_by_key(&region, |row| row.region_id)
            .map(|index| &context.regions[index])
            .map_err(|_| "transport original regional spotting context missing")
    }
    fn risk(&self) -> damage::Fact<&TransportReceiverRisk> {
        self.receiver
            .risk
            .as_ref()
            .ok_or("transport original receiver risk graph refused")
    }
    fn variant(&self) -> damage::Fact<Option<&TransportConvoyVariant>> {
        let slot = self
            .moves
            .slot(self.target_owner)
            .map_err(|_| "transport target convoy raw-country link refused")?;
        let actual = self
            .moves
            .actual_country_raw(slot)
            .map_err(|_| "transport target convoy actual country refused")?;
        let rows = self
            .moves
            .table::<tables::CountryAppearance>()
            .map_err(|_| "transport original target convoy country table refused")?;
        let index = rows
            .binary_search_by_key(&actual, |row| row.country)
            .map_err(|_| "transport original target convoy receiver missing")?;
        let packet = replay::transport_convoys::current(
            &rows[index].transport_convoys,
            self.moves.world.frame().hour,
        )?;
        packet
            .first_variant
            .as_ref()
            .ok_or("transport original first convoy selector refused")
            .map(Option::as_ref)
    }
    fn carrier_fill(&self, receiver: &TransportMissionReceiver) -> damage::Fact<i64> {
        let packet = receiver
            .risk
            .as_ref()
            .ok_or("transport carrier receiver graph refused")?;
        match packet.ship_count {
            Some(0) => Ok(0),
            Some(count) if count > 0 => carrier::fill(
                self.moves,
                receiver,
                self.packet()?
                    .risk_ships
                    .as_ref()
                    .ok_or("transport original SHIPS dictionary refused")?,
            ),
            Some(_) => Err("transport original carrier ship count is signed-negative"),
            None => Err("transport original carrier ship header refused"),
        }
    }
    fn receiver_detection(&self, receiver: &TransportMissionReceiver) -> damage::Fact<i64> {
        let packet = receiver
            .risk
            .as_ref()
            .ok_or("transport original spotting contributor graph refused")?;
        let stats = packet
            .stats
            .as_ref()
            .ok_or("transport original spotting contributor stat selector refused")?;
        let raw = stats
            .detection_138
            .ok_or("transport original contributor stat138 refused")?;
        let carrier = stats
            .carrier_detection_160
            .ok_or("transport original contributor stat160 refused")?;
        let fill = self.carrier_fill(receiver)?;
        let combined = receiver
            .combined_detection
            .ok_or("transport original contributor combined2A refused")?;
        Ok(damage::detection([raw, carrier], fill, combined))
    }
}

impl spotting::Inputs for Inputs<'_, '_, '_, '_> {
    fn first_convoy_visibility(&mut self) -> spotting::Fact<Option<i64>> {
        self.variant()?
            .map(|variant| {
                variant
                    .visibility
                    .ok_or("transport original first convoy visibility refused")
            })
            .transpose()
    }
    fn defines(&mut self) -> spotting::Fact<spotting::Defines> {
        self.moves
            .assets
            .transport_spotting
            .as_ref()
            .copied()
            .map_err(|_| "transport recorded-content strategic spotting defines refused")
    }
    fn navy_detection(&mut self) -> spotting::Fact<Vec<i64>> {
        let owner = self
            .receiver
            .mission_owner
            .ok_or("transport original embedded mission owner refused")?;
        let slot = self
            .moves
            .slot(owner)
            .map_err(|_| "transport mission-owner country link refused")?;
        let region = self
            .risk()?
            .region
            .ok_or("transport original mission actual-region receiver refused")?;
        let view = self
            .moves
            .transport_missions()
            .map_err(|_| "transport original mission singleton refused")?;
        let mut output = Vec::new();
        for &handle in view.ledger(slot, region)? {
            // F97BF0 dereferences these entries; its law has no E8B8E0 null skip.
            if handle == 0 {
                return Err("transport F97BF0 reached null mission or backlink");
            }
            output.push(self.receiver_detection(view.receiver(handle)?)?);
        }
        Ok(output)
    }
    fn air_contribution(&mut self, visibility: i64) -> spotting::Fact<i64> {
        use replay::transport_air_normalization as normalization;
        let owner = self
            .receiver
            .mission_owner
            .ok_or("transport original embedded mission owner refused")?;
        let slot = self
            .moves
            .slot(owner)
            .map_err(|_| "transport original air-owner country link refused")?;
        let region = self
            .risk()?
            .region
            .ok_or("transport original air actual-region receiver refused")?;
        let packet = self
            .packet()?
            .air
            .as_ref()
            .ok_or("transport original air graph unavailable")?;
        let (country, source) =
            spotting::air_region(packet, self.moves.world.frame().hour, slot, region)?;
        if self
            .moves
            .slot(
                country
                    .country
                    .ok_or("transport original air-root owner refused")?,
            )
            .map_err(|_| "transport original air-root owner link refused")?
            != slot
        {
            return Err("transport original air-root owner association differs");
        }
        if source.count == Some(0) {
            return Ok(0);
        }
        let context = self.region_context(region);
        let sampled = context.and_then(|context| {
            context
                .sampled_province
                .ok_or("transport original sampled air province refused")
        });
        let regional_factor = context.and_then(|context| {
            let index = usize::try_from(
                context
                    .runtime_slot
                    .ok_or("transport original air region runtime slot refused")?,
            )
            .map_err(|_| "transport original air region runtime slot negative")?;
            country
                .factors
                .as_deref()
                .ok_or("transport original air regional factor domain refused")?
                .get(index)
                .copied()
                .flatten()
                .ok_or("transport original air regional factor refused")
        });
        let normalized_defines = self
            .moves
            .assets
            .transport_air_normalization
            .as_ref()
            .copied()
            .map_err(|_| "transport recorded-content air normalization defines refused")
            .and_then(|defines| {
                replay::fixed_layout::validate_recording(
                    self.moves.world.frame(),
                    replay::fixed_layout::TRANSPORT_AIR,
                )
                .map(|()| defines)
                .map_err(|_| "transport recorded air modifier layout refused")
            });
        let disruption_defines = self
            .moves
            .assets
            .transport_air_disruption
            .as_ref()
            .copied()
            .map_err(|_| "transport recorded-content air disruption defines refused");
        let mut missions = Vec::new();
        for mission in source
            .missions
            .as_deref()
            .ok_or("transport original air occurrences refused")?
        {
            let mission = mission
                .as_ref()
                .ok_or("transport F96470 reached null mission or backlink");
            let kind = mission.and_then(|mission| {
                mission
                    .current_type
                    .ok_or("transport original air current type refused")
            });
            let aircraft = mission.and_then(|mission| {
                mission
                    .aircraft
                    .ok_or("transport original air aircraft count refused")
            });
            let raw = mission.and_then(|mission| {
                mission
                    .disruption
                    .ok_or("transport original air disruption word refused")
            });
            let normalized = mission.and_then(|mission| {
                mission
                    .normalization
                    .as_ref()
                    .ok_or("transport original air normalization producer refused")
            });
            let combat = kind.and_then(|kind| {
                normalized.and_then(|source| {
                    normalized_defines.map(|defines| normalization::combat(source, kind, defines))
                })
            });
            let attack = combat.and_then(|leaves| leaves[0]);
            let defence = combat.and_then(|leaves| leaves[1]);
            let speed = normalized.and_then(|source| {
                normalized_defines.and_then(|defines| normalization::speed(source, defines))
            });
            let efficiency = aircraft.and_then(|aircraft| {
                kind.and_then(|kind| {
                    spotting::disruption_factor_leaves(
                        raw,
                        aircraft,
                        kind,
                        attack,
                        defence,
                        speed,
                        disruption_defines,
                    )
                })
            });
            let detection = mission.and_then(|mission| {
                Ok([
                    mission
                        .detection_13
                        .ok_or("transport original air detection stat13 refused")?,
                    mission
                        .detection_14
                        .ok_or("transport original air detection stat14 refused")?,
                ])
            });
            missions.push(spotting::AirMission {
                current_type: kind,
                aircraft,
                disruption_factor: efficiency,
                detection,
            });
        }
        Ok(spotting::air_contribution(
            &missions,
            sampled,
            self.moves
                .assets
                .transport_air_defines
                .as_ref()
                .copied()
                .map_err(|_| "transport recorded-content air spotting defines refused"),
            self.moves
                .assets
                .transport_air_factors
                .as_deref()
                .map_err(|_| "transport recorded-content air mission factors refused"),
            regional_factor,
            [visibility, 0],
        )?[1])
    }
    fn radar_counts(&mut self) -> spotting::Fact<(i32, i32)> {
        let owner = self
            .receiver
            .mission_owner
            .ok_or("transport original embedded mission owner refused")?;
        let region = self
            .risk()?
            .region
            .ok_or("transport original radar source region refused")?;
        self.context()?.radar_counts(owner, region, &mut |raw| {
            self.moves
                .slot(raw)
                .map_err(|_| "transport current radar owner country link refused")
        })
    }
    fn navy_intel_difference(&mut self) -> spotting::Fact<i64> {
        let owner = self
            .receiver
            .mission_owner
            .ok_or("transport original embedded mission owner refused")?;
        self.context()?
            .navy_intel_difference(owner, self.target_owner, &mut |raw| {
                self.moves
                    .slot(raw)
                    .map_err(|_| "transport current navy-intel owner country link refused")
            })
    }
    fn weather_detection(&mut self) -> spotting::Fact<Option<i64>> {
        let region = self
            .risk()?
            .region
            .ok_or("transport original weather source region refused")?;
        self.context()?.weather_detection(region)
    }
}

impl damage::Inputs for Inputs<'_, '_, '_, '_> {
    fn detection_multiplier(&mut self) -> damage::Fact<i64> {
        let defines = *self
            .moves
            .assets
            .transport_detection
            .as_ref()
            .map_err(|_| "transport recorded-content detection multiplier defines refused")?;
        let cache = self
            .receiver
            .detection_cache
            .ok_or("transport original mission cache pair refused")?;
        replay::transport_spotting::detection_multiplier(
            cache,
            defines,
            true,
            self.receiver
                .received_supply_ratio
                .map(Some)
                .ok_or("transport original received supply refused"),
        )
    }
    fn combined_detection(&mut self) -> damage::Fact<i64> {
        self.receiver
            .combined_detection
            .ok_or("transport original combined2A refused")
    }
    fn detection_stats(&mut self) -> damage::Fact<[i64; 2]> {
        let stats = self
            .risk()?
            .stats
            .as_ref()
            .ok_or("transport original damage stat selector refused")?;
        Ok([
            stats
                .detection_138
                .ok_or("transport original damage stat138 refused")?,
            stats
                .carrier_detection_160
                .ok_or("transport original damage stat160 refused")?,
        ])
    }
    fn carrier_fill_sum(&mut self) -> damage::Fact<i64> {
        self.carrier_fill(self.receiver)
    }
    fn first_variant(&mut self) -> damage::Fact<Option<damage::Variant>> {
        Ok(self.variant()?.map(|variant| damage::Variant {
            visibility: variant
                .visibility
                .ok_or("transport convoy visibility refused"),
            surface_detection: variant
                .surface_detection
                .ok_or("transport convoy surface detection refused"),
            submarine_detection: variant
                .submarine_detection
                .ok_or("transport convoy submarine detection refused"),
            speed: variant.speed.ok_or("transport convoy speed refused"),
        }))
    }
    fn visibility_stats(&mut self) -> damage::Fact<[i64; 2]> {
        let stats = self
            .risk()?
            .stats
            .as_ref()
            .ok_or("transport original visibility stat selector refused")?;
        Ok([
            stats
                .visibility_148
                .ok_or("transport original visibility stat148 refused")?,
            stats
                .visibility_150
                .ok_or("transport original visibility stat150 refused")?,
        ])
    }
    fn detached_activity(&mut self) -> damage::Fact<i32> {
        self.risk()?
            .detached_activity
            .ok_or("transport original detached activity refused")
    }
    fn visibility_modifier(&mut self) -> damage::Fact<i64> {
        self.risk()?
            .visibility
            .ok_or("transport original combined19F refused")
    }
    fn actual_speed(&mut self) -> damage::Fact<i64> {
        let packet = self.risk()?;
        let adjustment = packet
            .actual_terrain_speed
            .ok_or("transport original class terrain-speed fold refused")?;
        let base = packet
            .speed_base
            .ok_or("transport original speed base refused")?;
        let modifier = packet
            .speed_modifier
            .ok_or("transport original speed modifier refused")?;
        let scale = self
            .moves
            .assets
            .transport_exposure
            .as_ref()
            .map_err(|_| "transport recorded-content naval speed scale refused")?
            .speed_scale;
        Ok((base.wrapping_mul(
            100_000_i64
                .wrapping_add(modifier)
                .wrapping_add(adjustment)
                .max(10_000),
        ) / 100_000)
            .wrapping_mul(scale)
            / 100_000)
    }
    fn coordination(&mut self) -> damage::Fact<i64> {
        self.risk()?
            .coordination
            .ok_or("transport original combined19 refused")
    }
    fn spotting_modifier(&mut self) -> damage::Fact<i64> {
        self.risk()?
            .spotting_chance
            .ok_or("transport original combined55 refused")
    }
    fn targeted_spotting_modifier(&mut self) -> damage::Fact<i64> {
        self.risk()?
            .targeted_spotting_modifier
            .ok_or("transport original targeted spotting1C5 refused")
    }
    fn simulation_night_factor(&mut self) -> damage::Fact<i64> {
        carrier::night(self.moves, self.receiver)
    }
    fn night_spotting_modifier(&mut self) -> damage::Fact<i64> {
        self.risk()?
            .night_spotting_chance
            .ok_or("transport original combined108 refused")
    }
}
