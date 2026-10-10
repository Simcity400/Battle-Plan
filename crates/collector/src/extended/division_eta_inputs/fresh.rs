//! Off-thread projection of this hour's independent counter ETA domains.
use super::*;
use replay::rows::{FreshCounterEta, FreshNormalEta, Recorded};

pub(super) const EMPTY_PROVINCE: cprovince::StaticProvince = cprovince::StaticProvince {
    pointer: 0,
    definition: 0,
    flags: 0,
    terrain: 0,
    terrain_index: 0,
};

/// Observed by the spool before growth; none of this runs in acquisition.
#[derive(Default)]
pub(crate) struct Work {
    rows_captured: u64,
    rows_refused: u64,
    ranges_refused: u64,
    captured: [u64; 3],
    unreached: [u64; 3],
    refused: [u64; 3],
    refusals: Vec<((&'static str, &'static str), u64)>,
    rows_required: usize,
    rows_capacity: usize,
    terrain_required: usize,
    terrain_capacity: usize,
    map_provinces_required: usize,
    map_provinces_capacity: usize,
    terrain_kinds: usize,
}

impl Work {
    fn refuse(
        &mut self,
        domain: &'static str,
        reason: &'static str,
        count: u64,
    ) -> std::io::Result<()> {
        if count == 0 {
            return Ok(());
        }
        let key = (domain, reason);
        let index = match self.refusals.binary_search_by_key(&key, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                engine::allocation::reserve(&mut self.refusals, 1)?;
                self.refusals.insert(index, (key, 0));
                index
            }
        };
        self.refusals[index].1 = self.refusals[index].1.saturating_add(count);
        Ok(())
    }

    pub(crate) fn observe(&mut self, capture: &Captured) -> std::io::Result<()> {
        self.rows_required = 0;
        self.rows_capacity = 0;
        self.terrain_required = 0;
        self.terrain_capacity = 0;
        self.map_provinces_required = 0;
        self.map_provinces_capacity = 0;
        self.terrain_kinds = 0;
        for range in capture.active() {
            let walk = range.fresh.outcome();
            let (rows, terrain) = range.fresh.capacities();
            self.rows_required = engine::allocation::add(self.rows_required, walk.required_rows)?;
            self.rows_capacity = engine::allocation::add(self.rows_capacity, rows)?;
            self.terrain_required =
                engine::allocation::add(self.terrain_required, range.required_fresh_terrain()?)?;
            self.terrain_capacity = engine::allocation::add(self.terrain_capacity, terrain)?;
            self.map_provinces_required = self.map_provinces_required.max(range.required_provinces);
            self.map_provinces_capacity =
                engine::allocation::add(self.map_provinces_capacity, range.land_terrains.len())?;
            self.terrain_kinds = self.terrain_kinds.max(range.land_len.unwrap_or(0));
            let failure = range
                .failure
                .or(range.fresh_failure)
                .or(match walk.outcome {
                    Outcome::Unavailable { reason } => Some(reason),
                    _ => None,
                });
            if let Some(reason) = failure {
                self.ranges_refused = self.ranges_refused.saturating_add(1);
                self.refuse("range", reason, 1)?;
                continue;
            }
            let missing = walk.required_rows.saturating_sub(range.fresh.rows().len()) as u64;
            self.rows_refused = self.rows_refused.saturating_add(missing);
            self.refuse("row", "fresh counter ETA row arena exhausted", missing)?;
            for row in range.fresh.rows() {
                if Some(row.sample_hour) != range.fresh_hour {
                    self.rows_refused = self.rows_refused.saturating_add(1);
                    self.refuse("row", "fresh counter ETA sample hour invalid", 1)?;
                    continue;
                }
                self.rows_captured = self.rows_captured.saturating_add(1);
                let span = row.normal.ok().flatten().is_none_or(|normal| {
                    normal
                        .terrain_start
                        .checked_add(normal.terrain_len)
                        .is_some_and(|end| end <= range.fresh.terrain().len())
                });
                let outcomes = [
                    row.current_speed.map(|value| value.is_some()),
                    row.normal.map(|value| value.is_some()).and_then(|value| {
                        if span {
                            Ok(value)
                        } else {
                            Err("fresh counter ETA terrain span invalid")
                        }
                    }),
                    row.rail.map(|value| value.is_some()),
                ];
                for (index, (domain, outcome)) in ["current_speed", "normal", "rail"]
                    .into_iter()
                    .zip(outcomes)
                    .enumerate()
                {
                    match outcome {
                        Ok(true) => self.captured[index] = self.captured[index].saturating_add(1),
                        Ok(false) => {
                            self.unreached[index] = self.unreached[index].saturating_add(1)
                        }
                        Err(reason) => {
                            self.refused[index] = self.refused[index].saturating_add(1);
                            self.refuse(domain, reason, 1)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        let reasons = |domain| {
            crate::status::map(
                self.refusals
                    .iter()
                    .filter(move |((source, _), _)| *source == domain)
                    .map(|&((_, reason), count)| (reason, count)),
            )
        };
        crate::status::status!({
            "rows_captured": self.rows_captured,
            "rows_refused": self.rows_refused,
            "ranges_refused": self.ranges_refused,
            "row_refusals": reasons("row"),
            "range_refusals": reasons("range"),
            "current_speed": {
                "captured": self.captured[0], "unreached": self.unreached[0], "refused": self.refused[0],
                "refusals": reasons("current_speed"),
            },
            "normal": {
                "captured": self.captured[1], "unreached": self.unreached[1],
                "refused": self.refused[1], "refusals": reasons("normal"),
            },
            "rail": {
                "captured": self.captured[2], "unreached": self.unreached[2],
                "refused": self.refused[2], "refusals": reasons("rail"),
            },
            "rows_required": self.rows_required,
            "rows_capacity": self.rows_capacity,
            "terrain_required": self.terrain_required,
            "terrain_capacity": self.terrain_capacity,
            "map_provinces_required_per_range": self.map_provinces_required,
            "map_province_slots_capacity": self.map_provinces_capacity,
            "terrain_kinds": self.terrain_kinds,
            "scope": "processed current-hour fresh ETA inputs; legacy operands and counters retain their five-hour meaning; range refusals do not invent a missing unit count",
        })
    }
}

/// This suffix is joined to the genuine movement row only for its current hour.
/// An unavailable current-edge speed leaves independently acquired duration
/// domains usable; a normal-domain failure leaves the rail domain usable.
pub(super) fn project(
    row: &carmy::FreshCounterEtaRow,
    terrain: &[(i32, i64)],
    hour: i32,
) -> Recorded<Option<FreshCounterEta>> {
    if row.sample_hour != hour {
        return Recorded::Value(None);
    }
    let normal = row.normal.ok().flatten().and_then(|normal| {
        let end = normal.terrain_start.checked_add(normal.terrain_len)?;
        let terrain = terrain.get(normal.terrain_start..end)?;
        Some(FreshNormalEta {
            speed_modifier: normal.prepared.modifier,
            speed_base: normal.prepared.base_speed,
            speed_org: normal.prepared.organisation,
            speed_fuel: normal.prepared.fuel,
            fort_penalty: normal.fort_penalty,
            snow_penalty: normal.snow_penalty,
            river_adjustment: normal.river_adjustment,
            river_leader: normal.river_leader,
            terrain_speed: owned_vec(engine::allocation::collect_vec(
                terrain
                    .iter()
                    .map(|&(terrain, modifier)| TerrainSpeed { terrain, modifier }),
            )),
        })
    });
    Recorded::Value(Some(FreshCounterEta {
        sample_hour: row.sample_hour,
        current_speed: row.current_speed.ok().flatten(),
        normal,
        rail: row.rail.ok().flatten(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> carmy::FreshCounterEtaRow {
        carmy::FreshCounterEtaRow {
            division: 7,
            sample_hour: 12,
            current_speed: Ok(Some(0)),
            normal: Ok(Some(carmy::FreshNormalEtaRow {
                prepared: carmy::PreparedNormalSpeed {
                    modifier: i64::MIN,
                    base_speed: i64::MAX,
                    organisation: None,
                    fuel: Some(-1),
                    outer_scale: 100_000,
                },
                fort_penalty: -2,
                snow_penalty: -3,
                river_adjustment: -4,
                river_leader: -5,
                terrain_start: 1,
                terrain_len: 2,
            })),
            rail: Ok(Some(carmy::StrategicRedeploymentInputs {
                country_modifier: -6,
                state_modifier: Some(-7),
                friendly_state_modifier: None,
            })),
        }
    }

    fn value(row: &carmy::FreshCounterEtaRow, terrain: &[(i32, i64)]) -> FreshCounterEta {
        let Recorded::Value(Some(value)) = project(row, terrain, 12) else {
            panic!("current-hour payload missing");
        };
        value
    }

    #[test]
    fn independent_domains_keep_signed_values_and_observed_zero() {
        let terrain = [(1, 99), (3, i64::MIN), (5, i64::MAX), (9, 99)];
        let source = row();
        let captured = value(&source, &terrain);
        assert_eq!(captured.current_speed, Some(0));
        let normal = captured.normal.unwrap();
        assert_eq!(normal.speed_modifier, i64::MIN);
        assert_eq!(normal.speed_base, i64::MAX);
        assert_eq!((normal.speed_org, normal.speed_fuel), (None, Some(-1)));
        assert_eq!(
            (
                normal.fort_penalty,
                normal.snow_penalty,
                normal.river_adjustment,
                normal.river_leader,
            ),
            (-2, -3, -4, -5)
        );
        assert_eq!(
            normal.terrain_speed,
            vec![
                TerrainSpeed {
                    terrain: 3,
                    modifier: i64::MIN
                },
                TerrainSpeed {
                    terrain: 5,
                    modifier: i64::MAX
                },
            ]
        );
        let mut source = source;
        source.current_speed = Err("current edge failed");
        let captured = value(&source, &terrain);
        assert_eq!(captured.current_speed, None);
        assert!(captured.normal.is_some());
        assert_eq!(captured.rail, source.rail.unwrap());
        source.normal = Err("normal graph failed");
        let captured = value(&source, &terrain);
        assert_eq!(captured.normal, None);
        assert_eq!(captured.rail, source.rail.unwrap());
        source.normal = row().normal;
        for rail in [Err("rail graph failed"), Ok(None)] {
            source.rail = rail;
            let captured = value(&source, &terrain);
            assert!(captured.normal.is_some());
            assert_eq!(captured.current_speed, None);
            assert_eq!(captured.rail, None);
        }
    }

    #[test]
    fn invalid_normal_span_does_not_erase_other_domains_or_invent_empty_terrain() {
        for (start, len) in [(1, 3), (usize::MAX, 1)] {
            let mut source = row();
            let mut normal = source.normal.unwrap().unwrap();
            normal.terrain_start = start;
            normal.terrain_len = len;
            source.normal = Ok(Some(normal));
            let captured = value(&source, &[(1, 0), (3, 0)]);
            assert_eq!(captured.normal, None);
            assert_eq!(captured.current_speed, Some(0));
            assert_eq!(captured.rail, source.rail.unwrap());
        }
    }

    #[test]
    fn an_earlier_payload_cannot_be_relabelled_as_current() {
        let source = row();
        assert!(matches!(project(&source, &[], 13), Recorded::Value(None)));
        assert!(matches!(project(&source, &[], 11), Recorded::Value(None)));
        assert!(matches!(
            project(&source, &[], 12),
            Recorded::Value(Some(_))
        ));
    }

    #[test]
    fn idle_unreached_current_speed_preserves_normal_and_rail_domains() {
        let mut source = row();
        source.current_speed = Ok(None);
        let terrain = [(1, 99), (3, i64::MIN), (5, i64::MAX)];
        let captured = value(&source, &terrain);
        assert_eq!(captured.current_speed, None);
        assert!(captured.normal.is_some());
        assert_eq!(captured.rail, source.rail.unwrap());
        source.normal = Err("normal graph failed");
        let captured = value(&source, &terrain);
        assert_eq!(captured.current_speed, None);
        assert_eq!(captured.normal, None);
        assert_eq!(captured.rail, source.rail.unwrap());
        source.normal = row().normal;
        source.rail = Err("rail graph failed");
        let captured = value(&source, &terrain);
        assert!(captured.normal.is_some());
        assert_eq!(captured.rail, None);
    }

    #[test]
    fn borrowed_profile_comparison_uses_the_same_partial_domain_and_span_laws() {
        let terrain = [(1, 99), (3, i64::MIN), (5, i64::MAX)];
        for current in [Ok(None), Ok(Some(0)), Err("current edge failed")] {
            for normal in [row().normal, Err("normal failed"), Ok(None)] {
                for rail in [row().rail, Err("rail failed"), Ok(None)] {
                    let source = carmy::FreshCounterEtaRow {
                        current_speed: current,
                        normal,
                        rail,
                        ..row()
                    };
                    let profile = FreshProfile {
                        row: &source,
                        terrain: &terrain,
                    };
                    let projected = profile.project();
                    assert!(profile.matches(&projected));
                    let Recorded::Value(Some(mut changed)) = projected else {
                        unreachable!();
                    };
                    changed.sample_hour += 1;
                    assert!(!profile.matches(&Recorded::Value(Some(changed))));
                }
            }
        }
        let mut source = row();
        let mut normal = source.normal.unwrap().unwrap();
        normal.terrain_start = usize::MAX;
        normal.terrain_len = 1;
        source.normal = Ok(Some(normal));
        let profile = FreshProfile {
            row: &source,
            terrain: &terrain,
        };
        let projected = profile.project();
        assert!(profile.matches(&projected));
        let Recorded::Value(Some(projected)) = projected else {
            unreachable!();
        };
        assert_eq!(projected.normal, None);
        assert_eq!(projected.current_speed, Some(0));
        assert_eq!(projected.rail, source.rail.unwrap());
    }
}
