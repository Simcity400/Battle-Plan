
use replay_data::rows::Recorded;

use super::{
    AirBaseRow, AirMissionInputRow, AirWingRow, Caps, FlightDefines, MissionCountry, Random,
    SteppedFamily, rig_key, scenario_name,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct ScenarioWing<'a> {
    pub wing: &'a AirWingRow,
    pub inputs: &'a AirMissionInputRow,
    pub base: &'a AirBaseRow,
    pub mission_country: MissionCountry,
    pub country: u32,
    pub side: usize,
}

pub(super) const MISSILE_KIND: u8 = 15;
const LAUNCH_MISSION: u32 = 1 << 17;
const LAUNCH_DEFINITION: u64 = 1 << 33;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LaunchKind {
    Mission,
    Definition,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LaunchRequest<'a> {
    pub wing: ScenarioWing<'a>,
    pub kind: LaunchKind,
}

#[derive(Default)]
pub(super) struct RegionGroups<'a> {
    pub arrays: [[Vec<ScenarioWing<'a>>; 5]; 2],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ScenarioDraw<'a> {
    pub batch: usize,
    pub attacker: ScenarioWing<'a>,
    pub defender: Option<ScenarioWing<'a>>,
    pub kind: u8,
    pub outcome: u8,
    pub province: Option<u32>,
}

type NavalSelector<'a, 's> =
    dyn Fn(&ScenarioWing<'a>, &mut Random) -> Result<(Option<u32>, u8), &'static str> + 's;

struct Scheduler<'a, 's> {
    region: u32,
    defines: FlightDefines,
    count: &'s dyn Fn(&ScenarioWing<'a>) -> Result<i32, &'static str>,
    naval: &'s NavalSelector<'a, 's>,
    random: &'s mut Random,
    caps: &'s mut Caps,
    diagnostics: &'s mut Vec<String>,
    draws: Vec<ScenarioDraw<'a>>,
    next_batch: usize,
    fighters_uncertain: bool,
    launches: Vec<LaunchRequest<'a>>,
}

impl<'a> Scheduler<'a, '_> {
    fn planes(&mut self, wing: &ScenarioWing<'a>) -> Option<i32> {
        match (self.count)(wing) {
            Ok(count) => Some(count),
            Err(reason) => {
                self.diagnostics.push(format!(
                    "wing {} scenario count omitted: {reason}",
                    wing.wing.id
                ));
                None
            }
        }
    }

    fn open(&self, kind: u8, outcome: u8, limit: usize) -> bool {
        self.caps
            .counts
            .get(&(self.region, rig_key(scenario_name(kind), outcome)))
            .copied()
            .unwrap_or(0)
            < limit
    }

    fn emit(&mut self, mut draw: ScenarioDraw<'a>, batches: usize) {
        draw.batch = self.next_batch;
        self.next_batch += 1;
        *self
            .caps
            .counts
            .entry((self.region, rig_key(scenario_name(draw.kind), draw.outcome)))
            .or_default() += batches;
        self.draws.extend(std::iter::repeat_n(draw, batches));
    }

    fn stepped(
        &mut self,
        wings: &mut Vec<ScenarioWing<'a>>,
        family: SteppedFamily,
        kinds: [u8; 2],
        counting_wings: bool,
    ) {
        while !wings.is_empty() {
            let before = wings.len();
            let wing = take(wings, self.random);
            let Some(planes) = self.planes(&wing) else {
                continue;
            };
            let units = family.sizing.units(planes);
            let triple = units >= family.sizing.triple;
            if !triple
                && if counting_wings {
                    before < family.single as usize
                } else {
                    units < family.single
                }
            {
                break;
            }
            let step = if triple {
                family.sizing.triple
            } else {
                family.single
            };
            let kind = kinds[usize::from(triple)];
            if !self.open(kind, 0, family.sizing.limit) {
                break;
            }
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker: wing,
                    defender: None,
                    kind,
                    outcome: 0,
                    province: matches!(kind, 19 | 20)
                        .then_some(wing.inputs.last_target_province as u32),
                },
                (units / step) as usize,
            );
        }
    }

    fn missiles(&mut self, wings: &mut Vec<ScenarioWing<'a>>, opposed: bool) {
        let family = match self.defines.missile {
            Ok(family) => family,
            Err(reason) => {
                if !wings.is_empty() {
                    self.diagnostics.push(format!(
                        "region {} omits {} base-equipment wings' missile scenarios and launch effects: {reason}",
                        self.region,
                        wings.len()
                    ));
                }
                return;
            }
        };
        while !wings.is_empty() {
            if !self.open(MISSILE_KIND, 0, family.limit) {
                break;
            }
            let wing = take(wings, self.random);
            let Some(planes) = self.planes(&wing) else {
                continue;
            };
            if planes <= 0 {
                continue;
            }
            let launch = if wing.wing.mission_mask & LAUNCH_MISSION != 0 {
                Some(opposed.then_some(LaunchKind::Mission))
            } else if wing.inputs.definition_map_flags & LAUNCH_DEFINITION != 0 {
                Some(Some(LaunchKind::Definition))
            } else {
                None
            };
            if let Some(kind) = launch {
                self.launches
                    .extend(kind.map(|kind| LaunchRequest { wing, kind }));
                continue;
            }
            let target = wing.inputs.last_target_province;
            for _ in 0..family.units(planes) {
                self.emit(
                    ScenarioDraw {
                        batch: 0,
                        attacker: wing,
                        defender: None,
                        kind: MISSILE_KIND,
                        outcome: 0,
                        province: (target != 0).then_some(target as u32),
                    },
                    1,
                );
            }
        }
    }

    fn fighters(&mut self, wings: &mut Vec<ScenarioWing<'a>>, side: usize) {
        while !wings.is_empty() {
            let before = wings.len();
            let wing = swap_selected(wings, self.random);
            let Some(planes) = self.planes(&wing) else {
                self.fighters_uncertain = true;
                wings.pop();
                continue;
            };
            let family = self.defines.fighter;
            let units = family.sizing.units(planes);
            let triple = units >= family.sizing.triple;
            let naval = side == 0 && wing.wing.mission_mask & 0x80 != 0;
            let (province, outcome) = if naval {
                match (self.naval)(&wing, self.random) {
                    Ok(choice) => choice,
                    Err(reason) => {
                        self.diagnostics.push(format!(
                            "wing {} naval-patrol scenario omitted: {reason}",
                            wing.wing.id
                        ));
                        self.fighters_uncertain = true;
                        wings.pop();
                        continue;
                    }
                }
            } else {
                (None, 0)
            };
            if !triple
                && if side == 0 && !naval {
                    before < family.single as usize
                } else {
                    units < family.single
                }
            {
                break;
            }
            wings.pop();
            let kind = (if naval { 16 } else { 0 }) + u8::from(triple);
            if !self.open(kind, outcome, family.sizing.limit) {
                break;
            }
            let step = if triple {
                family.sizing.triple
            } else {
                family.single
            };
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker: wing,
                    defender: None,
                    kind,
                    outcome,
                    province,
                },
                (units / step) as usize,
            );
        }
    }

    fn bombers(&mut self, wings: &mut Vec<ScenarioWing<'a>>) {
        while !wings.is_empty() {
            let wing = take(wings, self.random);
            let planes = wing.wing.planes;
            if planes <= 0 {
                continue;
            }
            let units = self.defines.bomber.units(planes);
            let kind = match bomber_kind(wing.wing, units, self.defines) {
                Ok(kind) => kind,
                Err(reason) => {
                    self.diagnostics.push(format!(
                        "wing {} bombing scenario omitted: {reason}",
                        wing.wing.id
                    ));
                    continue;
                }
            };
            if !self.open(kind, 0, self.defines.bomber.limit) {
                continue;
            }
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker: wing,
                    defender: None,
                    kind,
                    outcome: 0,
                    province: Some(wing.inputs.last_target_province as u32),
                },
                1,
            );
        }
    }

    fn divert_cas(&mut self, bombers: &mut Vec<ScenarioWing<'a>>) -> bool {
        let mut candidates = Vec::new();
        let mut weighted = 0i64;
        let mut planes_sum = 0i32;
        for wing in bombers.iter() {
            if wing.wing.mission_mask & 0x202 == 0 {
                continue;
            }
            let Some(planes) = self.planes(wing) else {
                return false;
            };
            if planes / self.defines.bomber.factor < self.defines.bomber_single {
                continue;
            }
            let ratio = match wing.inputs.cas_ratio_raw {
                Recorded::Value(Some(ratio)) => ratio,
                Recorded::Value(None) | Recorded::NotRecorded => {
                    self.diagnostics.push(format!("wing {} opposing bomber scenarios omitted: CAS mission probability is unavailable", wing.wing.id));
                    return false;
                }
            };
            candidates.push(*wing);
            weighted = weighted.wrapping_add(
                (i64::from(planes).wrapping_mul(100_000).wrapping_mul(ratio)) / 100_000,
            );
            planes_sum = planes_sum.wrapping_add(planes);
        }
        let ratio = cas_ratio(weighted, planes_sum);
        if i64::from(self.random.next() % 100_000) > ratio {
            return true;
        }
        while !candidates.is_empty() {
            if !self.open(4, 0, self.defines.bomber.limit) {
                break;
            }
            let wing = take(&mut candidates, self.random);
            bombers.retain(|candidate| !std::ptr::eq(candidate.inputs, wing.inputs));
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker: wing,
                    defender: None,
                    kind: 4,
                    outcome: 0,
                    province: Some(wing.inputs.last_target_province as u32),
                },
                1,
            );
        }
        true
    }

    fn opposed(
        &mut self,
        wings: &mut Vec<ScenarioWing<'a>>,
        fighters: &[ScenarioWing<'a>],
        kind: u8,
        outcome: u8,
    ) {
        let family = if kind == 12 {
            self.defines.bomber
        } else {
            self.defines.transport.sizing
        };
        while !wings.is_empty() && !fighters.is_empty() {
            if !self.open(kind, outcome, family.limit) {
                break;
            }
            let wing = take(wings, self.random);
            let fighter = fighters[self.random.next() as usize % fighters.len()];
            let Some(planes) = self.planes(&wing) else {
                continue;
            };
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker: fighter,
                    defender: Some(wing),
                    kind,
                    outcome,
                    province: Some(wing.inputs.last_target_province as u32),
                },
                family.units(planes) as usize,
            );
        }
    }

    fn dogfights(
        &mut self,
        a: &mut Vec<ScenarioWing<'a>>,
        b: &mut Vec<ScenarioWing<'a>>,
        outcome: u8,
    ) {
        let family = self.defines.dogfight;
        while !a.is_empty() && !b.is_empty() {
            let attacker = swap_selected(a, self.random);
            let defender = swap_selected(b, self.random);
            let Some(a_planes) = self.planes(&attacker) else {
                self.fighters_uncertain = true;
                a.pop();
                b.pop();
                continue;
            };
            let Some(b_planes) = self.planes(&defender) else {
                self.fighters_uncertain = true;
                a.pop();
                b.pop();
                continue;
            };
            let a_units = family.sizing.units(a_planes);
            let b_units = family.sizing.units(b_planes);
            let triple = a_units >= family.sizing.triple && b_units >= family.sizing.triple;
            if !triple && (a.len() < family.single as usize || b.len() < family.single as usize) {
                break;
            }
            a.pop();
            b.pop();
            let kind = 13 + u8::from(triple);
            if !self.open(kind, outcome, family.sizing.limit) {
                break;
            }
            let step = if triple {
                family.sizing.triple
            } else {
                family.single
            };
            let batches = (a_units.min(b_units) / step) as usize;
            self.emit(
                ScenarioDraw {
                    batch: 0,
                    attacker,
                    defender: Some(defender),
                    kind,
                    outcome,
                    province: None,
                },
                batches,
            );
        }
    }
}

impl<'a> Scheduler<'a, '_> {
    fn generated(self) -> Generated<'a> {
        Generated {
            draws: self.draws,
            launches: self.launches,
        }
    }
}

fn take<T: Copy>(wings: &mut Vec<T>, random: &mut Random) -> T {
    wings.swap_remove(random.next() as usize % wings.len())
}

fn swap_selected<T: Copy>(wings: &mut [T], random: &mut Random) -> T {
    let index = random.next() as usize % wings.len();
    let selected = wings[index];
    wings[index] = wings[wings.len() - 1];
    selected
}

fn cas_ratio(weighted: i64, planes: i32) -> i64 {
    let denominator = i64::from(planes) * 100_000;
    if denominator == 0 {
        return 0;
    }
    if (weighted as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return 0xffff_ffff;
    }
    weighted.wrapping_mul(100_000) / denominator
}

fn bomber_kind(wing: &AirWingRow, units: i32, defines: FlightDefines) -> Result<u8, &'static str> {
    let triple = u8::from(units >= defines.bomber.triple);
    if wing.mission_mask & 0x202 != 0 {
        return Ok(4 + triple);
    }
    if wing.mission_mask & 8 != 0 {
        let stat = match wing.strategic_bombing_raw {
            Recorded::Value(Some(stat)) => stat,
            Recorded::Value(None) | Recorded::NotRecorded => {
                return Err("selected strategic-bombing stat is unavailable");
            }
        };
        if stat >= defines.fire_threshold {
            return Ok(10 + triple);
        }
        if stat >= defines.carpet_threshold {
            return Ok(8 + triple);
        }
    }
    Ok(6 + triple)
}

fn dogfight_outcome(a: i64, b: i64) -> u8 {
    if a.wrapping_mul(110_000) / 100_000 < b {
        2
    } else if b.wrapping_mul(110_000) / 100_000 < a {
        3
    } else {
        1
    }
}

pub(super) struct Generated<'a> {
    pub draws: Vec<ScenarioDraw<'a>>,
    pub launches: Vec<LaunchRequest<'a>>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate<'a>(
    region: u32,
    groups: RegionGroups<'a>,
    defines: FlightDefines,
    history: Result<[i64; 8], &'static str>,
    count: &dyn Fn(&ScenarioWing<'a>) -> Result<i32, &'static str>,
    random: &mut Random,
    caps: &mut Caps,
    diagnostics: &mut Vec<String>,
    naval: &NavalSelector<'a, '_>,
) -> Generated<'a> {
    let present = groups
        .arrays
        .each_ref()
        .map(|side| side[..4].iter().any(|array| !array.is_empty()));
    let [
        [
            mut bombers_a,
            mut fighters_a,
            mut missiles_a,
            mut transports_a,
            mut scouts,
        ],
        [
            mut bombers_b,
            mut fighters_b,
            mut missiles_b,
            mut transports_b,
            _,
        ],
    ] = groups.arrays;
    let mut scheduler = Scheduler {
        region,
        defines,
        count,
        naval,
        random,
        caps,
        diagnostics,
        draws: Vec::new(),
        next_batch: 0,
        fighters_uncertain: false,
        launches: Vec::new(),
    };
    scheduler.stepped(&mut scouts, defines.scout, [2, 3], true);
    scheduler.missiles(&mut missiles_a, present[1]);
    scheduler.missiles(&mut missiles_b, present[0]);
    if bombers_a.is_empty() && fighters_a.is_empty() {
        scheduler.fighters(&mut fighters_b, 1);
        scheduler.bombers(&mut bombers_b);
    } else if bombers_b.is_empty() && fighters_b.is_empty() {
        scheduler.fighters(&mut fighters_a, 0);
        scheduler.bombers(&mut bombers_a);
    } else if let Ok(channels) = history {
        let a_damage = channels[0].wrapping_add(channels[1]);
        let b_damage = channels[3].wrapping_add(channels[4]);
        if !bombers_a.is_empty() && !fighters_b.is_empty() && scheduler.divert_cas(&mut bombers_a) {
            scheduler.opposed(
                &mut bombers_a,
                &fighters_b,
                12,
                u8::from(b_damage < a_damage) + 1,
            );
        }
        if bombers_b.is_empty() || !fighters_a.is_empty() {
            if !bombers_b.is_empty() && scheduler.divert_cas(&mut bombers_b) {
                scheduler.opposed(
                    &mut bombers_b,
                    &fighters_a,
                    12,
                    u8::from(a_damage < b_damage) + 1,
                );
            }
            scheduler.dogfights(
                &mut fighters_a,
                &mut fighters_b,
                dogfight_outcome(channels[2], channels[5]),
            );
        }
    } else if let Err(reason) = history {
        scheduler.diagnostics.push(format!(
            "region {region} opposing aircraft scenarios omitted: {reason}"
        ));
        return scheduler.generated();
    }
    if scheduler.fighters_uncertain {
        scheduler.diagnostics.push(format!(
            "region {region} transport scenarios omitted: residual fighter arrays are unavailable"
        ));
        return scheduler.generated();
    }
    if transports_a.is_empty() && fighters_a.is_empty() {
        scheduler.stepped(&mut transports_b, defines.transport, [19, 20], false);
    } else if transports_b.is_empty() && fighters_b.is_empty() {
        scheduler.stepped(&mut transports_a, defines.transport, [19, 20], false);
    } else if let Ok(channels) = history {
        scheduler.opposed(
            &mut transports_a,
            &fighters_b,
            18,
            u8::from(channels[7] < channels[6]) + 1,
        );
        scheduler.opposed(
            &mut transports_b,
            &fighters_a,
            18,
            u8::from(channels[6] < channels[7]) + 1,
        );
    } else if let Err(reason) = history {
        scheduler.diagnostics.push(format!(
            "region {region} opposing transport scenarios omitted: {reason}"
        ));
    }
    scheduler.generated()
}

#[cfg(test)]
mod tests {
    use super::super::FamilyDefines;
    use super::*;
    use std::collections::BTreeSet;

    fn defines() -> FlightDefines {
        let family = SteppedFamily {
            sizing: FamilyDefines {
                factor: 10,
                triple: 3,
                limit: 1,
            },
            single: 1,
        };
        FlightDefines {
            fighter: family,
            scout: family,
            bomber: family.sizing,
            bomber_single: 1,
            dogfight: family,
            transport: family,
            training: family,
            missile: Ok(family.sizing),
            fire_threshold: 200_000,
            carpet_threshold: 100_000,
            clock_speed: 1.0,
            curve_density: 1.0,
            curve_extrapolation: 1.0,
            curve_elevation: 1.0,
            scale_takeoff: 1.0,
            scale_landing: 1.0,
            scale_min: 1.0,
            smooth_move: 0.13,
            smooth_turn: 0.12,
        }
    }

    fn wing(id: u64, planes: i32) -> AirWingRow {
        AirWingRow {
            id,
            planes,
            ..Default::default()
        }
    }

    fn participant<'a>(
        wing: &'a AirWingRow,
        inputs: &'a AirMissionInputRow,
        base: &'a AirBaseRow,
        side: usize,
    ) -> ScenarioWing<'a> {
        ScenarioWing {
            wing,
            inputs,
            base,
            mission_country: MissionCountry { slot: wing.owner },
            country: wing.owner,
            side,
        }
    }

    fn run<'a>(
        groups: RegionGroups<'a>,
        defines: FlightDefines,
        history: Result<[i64; 8], &'static str>,
    ) -> (Vec<ScenarioDraw<'a>>, Vec<String>) {
        let mut diagnostics = Vec::new();
        let draws = generate(
            7,
            groups,
            defines,
            history,
            &|wing| Ok(wing.wing.planes),
            &mut Random::new(37),
            &mut Caps::default(),
            &mut diagnostics,
            &|_, _| Ok((Some(99), 2)),
        )
        .draws;
        (draws, diagnostics)
    }

    #[test]
    fn missile_wings_follow_scouts_with_their_cap_division_and_launch_rules() {
        use replay_data::rows::AIR_MISSION_BASE_EQUIPMENT_FLAGS;
        let base = AirBaseRow {
            province: Some(40),
            ..Default::default()
        };
        let launch_base = AirBaseRow {
            province: Some(41),
            ..Default::default()
        };
        let rockets = [wing(1, 25), wing(2, 5)];
        let target = AirMissionInputRow {
            last_target_province: 77,
            definition_map_flags: AIR_MISSION_BASE_EQUIPMENT_FLAGS & !(1 << 33),
            ..Default::default()
        };
        let mut defines = defines();
        defines.missile = Ok(FamilyDefines {
            limit: 3,
            ..defines.missile.unwrap()
        });
        let mut groups = RegionGroups::default();
        groups.arrays[0][2] = rockets
            .iter()
            .map(|rocket| participant(rocket, &target, &base, 0))
            .collect();
        let (draws, diagnostics) = run(groups, defines, Ok([0; 8]));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let missiles: Vec<_> = draws
            .iter()
            .filter(|draw| draw.kind == MISSILE_KIND)
            .collect();
        assert_eq!(missiles.len(), 3);
        assert!(missiles.iter().all(|draw| draw.province == Some(77)));
        let batches: BTreeSet<_> = missiles.iter().map(|draw| draw.batch).collect();
        assert_eq!(batches.len(), 3);
        let first_units = [2, 1][Random::new(37).next() as usize % 2];
        for (limit, expected) in [
            (1, first_units),
            (2, if first_units == 2 { 2 } else { 3 }),
            (3, 3),
        ] {
            defines.missile = Ok(FamilyDefines {
                limit,
                ..defines.missile.unwrap()
            });
            let mut groups = RegionGroups::default();
            groups.arrays[0][2] = rockets
                .iter()
                .map(|rocket| participant(rocket, &target, &base, 0))
                .collect();
            let (draws, _) = run(groups, defines, Ok([0; 8]));
            assert_eq!(
                draws
                    .iter()
                    .filter(|draw| draw.kind == MISSILE_KIND)
                    .count(),
                expected,
                "limit {limit}"
            );
        }
        let mut refused = defines;
        refused.missile = Err("NDefines.NAirGfx.MISSILES_DIVISION_FACTOR is zero");
        let mut groups = RegionGroups::default();
        groups.arrays[0][2] = rockets
            .iter()
            .map(|rocket| participant(rocket, &target, &base, 0))
            .collect();
        let (draws, diagnostics) = run(groups, refused, Ok([0; 8]));
        assert!(draws.is_empty());
        assert_eq!(
            diagnostics,
            [
                "region 7 omits 2 base-equipment wings' missile scenarios and launch effects: NDefines.NAirGfx.MISSILES_DIVISION_FACTOR is zero"
            ]
        );
        let launch = AirMissionInputRow {
            last_target_province: 77,
            definition_map_flags: AIR_MISSION_BASE_EQUIPMENT_FLAGS,
            ..Default::default()
        };
        let interceptor = AirWingRow {
            mission_mask: 1 << 17,
            ..wing(3, 25)
        };
        let enemy = wing(9, 30);
        let none = AirMissionInputRow::default();
        let generate_with = |opposed: bool| {
            let mut groups = RegionGroups::default();
            groups.arrays[0][2] = vec![
                participant(&rockets[0], &launch, &launch_base, 0),
                participant(&interceptor, &none, &base, 0),
            ];
            if opposed {
                groups.arrays[1][1].push(participant(&enemy, &none, &base, 1));
            }
            generate(
                7,
                groups,
                defines,
                Ok([0; 8]),
                &|wing| Ok(wing.wing.planes),
                &mut Random::new(37),
                &mut Caps::default(),
                &mut Vec::new(),
                &|_, _| Ok((Some(99), 2)),
            )
        };
        let alone = generate_with(false);
        let launches = |generated: &Generated<'_>| {
            generated
                .launches
                .iter()
                .map(|launch| (launch.wing.wing.id, launch.kind))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            launches(&alone),
            BTreeSet::from([(1, LaunchKind::Definition)])
        );
        assert!(alone.draws.iter().all(|draw| draw.kind != MISSILE_KIND));
        assert_eq!(
            launches(&generate_with(true)),
            BTreeSet::from([(1, LaunchKind::Definition), (3, LaunchKind::Mission)])
        );
        assert_eq!(generate_with(true).launches.len(), 2);
    }

    #[test]
    fn missile_wings_draw_from_the_scheduler_random_before_the_other_arms() {
        let base = AirBaseRow::default();
        let rocket = wing(1, 10);
        let target = AirMissionInputRow {
            last_target_province: 77,
            ..Default::default()
        };
        let bombers = [wing(5, 30), wing(6, 30), wing(7, 30)];
        let missions = [4, 5, 6].map(|province| AirMissionInputRow {
            last_target_province: province,
            ..Default::default()
        });
        let mut defines = defines();
        defines.bomber.limit = 3;
        let order = |with_missile: bool| {
            let mut groups = RegionGroups::default();
            groups.arrays[0][0] = bombers
                .iter()
                .zip(&missions)
                .map(|(bomber, mission)| participant(bomber, mission, &base, 0))
                .collect();
            if with_missile {
                groups.arrays[0][2].push(participant(&rocket, &target, &base, 0));
            }
            run(groups, defines, Ok([0; 8]))
                .0
                .into_iter()
                .filter(|draw| draw.kind != MISSILE_KIND)
                .map(|draw| draw.attacker.wing.id)
                .collect::<Vec<_>>()
        };
        let taken = |skip: usize| {
            let mut random = Random::new(37);
            for _ in 0..skip {
                random.next();
            }
            let mut left = bombers.iter().map(|bomber| bomber.id).collect::<Vec<_>>();
            let mut taken = Vec::new();
            while !left.is_empty() {
                taken.push(left.swap_remove(random.next() as usize % left.len()));
            }
            taken
        };
        assert_eq!(order(false), taken(0));
        assert_eq!(order(true), taken(1));
    }

    #[test]
    fn opposing_bomber_batch_reuses_fighter_and_can_exceed_cap() {
        let bombers = [wing(1, 30), wing(2, 30)];
        let fighter = wing(9, 30);
        let missions = [
            AirMissionInputRow {
                last_target_province: 4,
                ..Default::default()
            },
            AirMissionInputRow {
                last_target_province: 5,
                ..Default::default()
            },
        ];
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0] = bombers
            .iter()
            .zip(&missions)
            .map(|(wing, inputs)| participant(wing, inputs, &base, 0))
            .collect();
        groups.arrays[1][1].push(participant(&fighter, &input, &base, 1));
        let (draws, diagnostics) = run(groups, defines(), Ok([20, 10, 0, 1, 2, 0, 0, 0]));
        assert!(diagnostics.is_empty());
        assert_eq!(draws.len(), 3);
        assert!(
            draws
                .iter()
                .all(|draw| draw.kind == 12 && draw.outcome == 2 && draw.attacker.wing.id == 9)
        );
        assert!(
            draws
                .iter()
                .all(|draw| draw.batch == draws[0].batch && draw.province == draws[0].province)
        );
    }

    #[test]
    fn bomber_pairing_does_not_consume_later_dogfight_fighter() {
        let bomber = wing(1, 20);
        let fighters = [wing(2, 30), wing(3, 30)];
        let input = AirMissionInputRow {
            last_target_province: 4,
            ..Default::default()
        };
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0].push(participant(&bomber, &input, &base, 0));
        groups.arrays[0][1].push(participant(&fighters[0], &input, &base, 0));
        groups.arrays[1][1].push(participant(&fighters[1], &input, &base, 1));
        let (draws, _) = run(groups, defines(), Ok([0; 8]));
        assert_eq!(
            draws.iter().map(|draw| draw.kind).collect::<Vec<_>>(),
            [12, 12, 14]
        );
        assert_eq!(draws[2].attacker.wing.id, 2);
        assert_eq!(draws[2].defender.unwrap().wing.id, 3);
    }

    #[test]
    fn cas_diversion_uses_raw_probability_and_spawns_one_single() {
        let bomber = AirWingRow {
            id: 1,
            planes: 40,
            mission_mask: 2,
            ..Default::default()
        };
        let fighter = wing(2, 30);
        let input = AirMissionInputRow {
            last_target_province: 4,
            cas_ratio_raw: Recorded::Value(Some(100_000)),
            ..Default::default()
        };
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0].push(participant(&bomber, &input, &base, 0));
        groups.arrays[1][1].push(participant(&fighter, &input, &base, 1));
        let (draws, _) = run(groups, defines(), Ok([0; 8]));
        assert_eq!(draws.len(), 1);
        assert_eq!(
            (draws[0].kind, draws[0].outcome, draws[0].province),
            (4, 0, Some(4))
        );
    }

    #[test]
    fn failed_fighter_single_gate_keeps_array_for_transport_pairing() {
        let fighter = wing(1, 10);
        let transport = wing(2, 20);
        let input = AirMissionInputRow {
            last_target_province: 4,
            ..Default::default()
        };
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][1].push(participant(&fighter, &input, &base, 0));
        groups.arrays[1][3].push(participant(&transport, &input, &base, 1));
        let mut defines = defines();
        defines.fighter.single = 2;
        let (draws, _) = run(groups, defines, Ok([0, 0, 0, 0, 0, 0, 50, 1]));
        assert_eq!(draws.len(), 2);
        assert!(
            draws
                .iter()
                .all(|draw| draw.kind == 18 && draw.outcome == 1 && draw.attacker.wing.id == 1)
        );
    }

    #[test]
    fn dogfight_single_gate_includes_selected_pair() {
        let fighters = [wing(1, 10), wing(2, 10)];
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][1].push(participant(&fighters[0], &input, &base, 0));
        groups.arrays[1][1].push(participant(&fighters[1], &input, &base, 1));
        let (draws, _) = run(groups, defines(), Ok([0; 8]));
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].kind, 13);
    }

    #[test]
    fn scout_single_gate_compares_array_count_and_runs_before_bombers() {
        let scouts = [wing(1, 20), wing(2, 20)];
        let bomber = wing(3, 10);
        let inputs = [AirMissionInputRow::default(), AirMissionInputRow::default()];
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][4] = scouts
            .iter()
            .zip(&inputs)
            .map(|(wing, inputs)| participant(wing, inputs, &base, 0))
            .collect();
        groups.arrays[0][0].push(participant(&bomber, &inputs[0], &base, 0));
        let mut defines = defines();
        defines.scout.single = 2;
        let (draws, _) = run(groups, defines, Err("old format"));
        assert_eq!(
            draws.iter().map(|draw| draw.kind).collect::<Vec<_>>(),
            [2, 6]
        );
    }

    #[test]
    fn independent_bomber_uses_own_planes_without_effective_count_or_history() {
        let wing = wing(1, 30);
        let input = AirMissionInputRow {
            last_target_province: 4,
            ..Default::default()
        };
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0].push(participant(&wing, &input, &base, 0));
        let mut diagnostics = Vec::new();
        let draws = generate(
            7,
            groups,
            defines(),
            Err("old format"),
            &|_| Err("not requested"),
            &mut Random::new(37),
            &mut Caps::default(),
            &mut diagnostics,
            &|_, _| Err("not requested"),
        )
        .draws;
        assert!(diagnostics.is_empty());
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].kind, 7);
    }

    #[test]
    fn independent_bomber_cap_skips_wing_and_continues_another_kind() {
        let wings = [
            wing(1, 10),
            AirWingRow {
                id: 2,
                planes: 10,
                mission_mask: 8,
                strategic_bombing_raw: Recorded::Value(Some(200_000)),
                ..Default::default()
            },
            wing(3, 10),
        ];
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0] = wings
            .iter()
            .map(|wing| participant(wing, &input, &base, 0))
            .collect();
        let (draws, diagnostics) = run(groups, defines(), Err("old format"));
        assert!(diagnostics.is_empty());
        assert_eq!(
            draws.iter().map(|draw| draw.kind).collect::<Vec<_>>(),
            [6, 10]
        );
    }

    #[test]
    fn b_naval_patrol_mask_uses_plain_patrol_scenario() {
        let wing = AirWingRow {
            id: 1,
            planes: 30,
            mission_mask: 0x80,
            ..Default::default()
        };
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[1][1].push(participant(&wing, &input, &base, 1));
        let (draws, diagnostics) = run(groups, defines(), Err("old format"));
        assert!(diagnostics.is_empty());
        assert_eq!(draws.len(), 1);
        assert_eq!(
            (draws[0].kind, draws[0].outcome, draws[0].province),
            (1, 0, None)
        );
    }

    #[test]
    fn a_naval_insufficient_units_stop_where_normal_patrol_counts_wings() {
        let mut wings = [
            AirWingRow {
                id: 1,
                planes: 10,
                mission_mask: 0x80,
                ..Default::default()
            },
            wing(2, 30),
        ];
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut defines = defines();
        defines.fighter.single = 2;
        let mut groups = RegionGroups::default();
        groups.arrays[0][1] = wings
            .iter()
            .map(|wing| participant(wing, &input, &base, 0))
            .collect();
        let (draws, diagnostics) = run(groups, defines, Err("old format"));
        assert!(diagnostics.is_empty());
        assert!(draws.is_empty());
        drop(draws);
        wings[0].mission_mask = 0;
        let mut groups = RegionGroups::default();
        groups.arrays[0][1] = wings
            .iter()
            .map(|wing| participant(wing, &input, &base, 0))
            .collect();
        let (draws, _) = run(groups, defines, Err("old format"));
        assert_eq!(draws.len(), 1);
        assert_eq!((draws[0].kind, draws[0].attacker.wing.id), (1, 2));
    }

    #[test]
    fn missing_history_refuses_opposed_groups_without_independent_fallback() {
        let wings = [wing(1, 30), wing(2, 30)];
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][0].push(participant(&wings[0], &input, &base, 0));
        groups.arrays[1][1].push(participant(&wings[1], &input, &base, 1));
        let (draws, diagnostics) = run(groups, defines(), Err("window zero not recorded"));
        assert!(draws.is_empty());
        assert_eq!(
            diagnostics,
            ["region 7 opposing aircraft scenarios omitted: window zero not recorded"]
        );
    }

    #[test]
    fn naval_success_uses_its_own_cap_and_one_batch_target() {
        let wing = AirWingRow {
            id: 1,
            planes: 20,
            mission_mask: 0x80,
            ..Default::default()
        };
        let input = AirMissionInputRow::default();
        let base = AirBaseRow::default();
        let mut groups = RegionGroups::default();
        groups.arrays[0][1].push(participant(&wing, &input, &base, 0));
        let mut caps = Caps::default();
        caps.counts.insert((7, rig_key(scenario_name(16), 0)), 1);
        let mut diagnostics = Vec::new();
        let draws = generate(
            7,
            groups,
            defines(),
            Ok([0; 8]),
            &|wing| Ok(wing.wing.planes),
            &mut Random::new(37),
            &mut caps,
            &mut diagnostics,
            &|_, _| Ok((Some(99), 2)),
        )
        .draws;
        assert!(diagnostics.is_empty());
        assert_eq!(draws.len(), 2);
        assert!(draws.iter().all(|draw| draw.outcome == 2
            && draw.province == Some(99)
            && draw.batch == draws[0].batch));
    }

    #[test]
    fn strategic_thresholds_are_inclusive_and_cas_wins() {
        let defines = defines();
        let mut wing = AirWingRow {
            mission_mask: 8,
            strategic_bombing_raw: Recorded::Value(Some(200_000)),
            ..Default::default()
        };
        assert_eq!(bomber_kind(&wing, 3, defines), Ok(11));
        wing.strategic_bombing_raw = Recorded::Value(Some(199_999));
        assert_eq!(bomber_kind(&wing, 1, defines), Ok(8));
        wing.strategic_bombing_raw = Recorded::Value(Some(99_999));
        assert_eq!(bomber_kind(&wing, 1, defines), Ok(6));
        wing.strategic_bombing_raw = Recorded::NotRecorded;
        assert!(bomber_kind(&wing, 1, defines).is_err());
        wing.mission_mask |= 2;
        assert_eq!(bomber_kind(&wing, 3, defines), Ok(5));
    }

    #[test]
    fn fixed5_cas_weight_and_fighter_tolerance() {
        assert_eq!(cas_ratio(10 * 25_000 + 30 * 75_000, 40), 62_500);
        assert_eq!(cas_ratio(0, 0), 0);
        assert_eq!(cas_ratio(i64::MAX, 1), 0xffff_ffff);
        assert_eq!(dogfight_outcome(100_000, 110_000), 1);
        assert_eq!(dogfight_outcome(100_000, 110_001), 2);
        assert_eq!(dogfight_outcome(110_001, 100_000), 3);
    }
}
