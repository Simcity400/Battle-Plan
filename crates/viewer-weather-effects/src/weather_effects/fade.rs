
use super::{CameraDefines, gates};
use content_reader_regions::regions::StrategicRegion;
use std::collections::BTreeMap;
use viewer_gpu::province_schedule::Schedule;

#[derive(Clone, Debug, Default)]
pub struct RegionProvinces(BTreeMap<u32, Box<[u32]>>);

impl RegionProvinces {
    pub fn new(regions: &BTreeMap<u32, StrategicRegion>, defined: impl Fn(u32) -> bool) -> Self {
        Self(
            regions
                .iter()
                .map(|(&id, region)| {
                    let provinces = region.provinces.iter().copied().filter(|&province| defined(province));
                    (id, provinces.collect())
                })
                .collect(),
        )
    }

    pub(super) fn contains(&self, region: u32) -> bool {
        self.0.contains_key(&region)
    }

    pub(super) fn listed(&self, region: u32, schedule: &Schedule) -> bool {
        self.0
            .get(&region)
            .is_some_and(|provinces| provinces.iter().any(|&province| schedule.listed(province)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Member {
    pub(super) active: bool,
    pub(super) alpha: f32,
}

impl Member {
    const CREATED: Self = Self {
        active: true,
        alpha: 1.0,
    };
    const ABSENT: Self = Self {
        active: false,
        alpha: 0.0,
    };

    fn set(&mut self, fade: f32) {
        if fade > 0.0 {
            *self = Self {
                active: true,
                alpha: fade,
            };
        } else {
            self.active = false;
        }
    }
}

fn run(members: &mut [Member; 2], height: f32, defines: &CameraDefines) {
    let (fade, far, _) = gates(height, defines);
    members[1].set(fade);
    if far {
        members[0].set(fade);
    }
}

pub(super) fn created(height: f32, defines: &CameraDefines) -> [Member; 2] {
    let mut members = [Member::CREATED; 2];
    run(&mut members, height, defines);
    members
}

#[derive(Default)]
pub(super) struct Fades(BTreeMap<u32, [Member; 2]>, Born);

type Born = BTreeMap<u32, Option<f32>>;

impl Fades {
    pub(super) fn members(&self, region: u32) -> [Member; 2] {
        self.0.get(&region).copied().unwrap_or([Member::ABSENT; 2])
    }

    pub(super) fn stamp(&mut self, time: f32) {
        for born in self.1.values_mut() {
            if born.is_none_or(|at| at > time) {
                *born = Some(time);
            }
        }
    }

    pub(super) fn born(&self, region: u32) -> Option<f32> {
        self.1.get(&region).copied().flatten()
    }

    pub(super) fn produce(
        &mut self,
        regions: impl IntoIterator<Item = u32>,
        height: f32,
        defines: &CameraDefines,
        listed: impl Fn(u32) -> bool,
    ) {
        for region in regions.into_iter().filter(|&region| listed(region)) {
            self.1.entry(region).or_insert(None);
            run(
                self.0.entry(region).or_insert([Member::CREATED; 2]),
                height,
                defines,
            );
        }
    }

    pub(super) fn remove(&mut self, region: u32) {
        self.0.remove(&region);
        self.1.remove(&region);
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
        self.1.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Member = Member::CREATED;

    fn active(alpha: f32) -> Member {
        Member {
            active: true,
            alpha,
        }
    }

    fn inactive(alpha: f32) -> Member {
        Member {
            active: false,
            alpha,
        }
    }

    fn run_at(fades: &mut Fades, height: f32) -> [Member; 2] {
        fades.produce([7], height, &super::super::tests::reference_defines(), |_| true);
        fades.members(7)
    }

    #[test]
    fn the_fading_entity_draws_at_every_height_its_fade_is_positive() {
        let mut fades = Fades::default();
        assert_eq!(run_at(&mut fades, 800.0), [ON, active(1.0)]);
        let [_, fading] = run_at(&mut fades, 450.0);
        assert!(fading.active && (fading.alpha - 92.0 / 220.0).abs() < 1e-6);
        assert_eq!(run_at(&mut fades, 300.0)[1], inactive(fading.alpha));
        assert_eq!(run_at(&mut fades, 1300.0)[1], active(0.5));
        assert_eq!(run_at(&mut fades, 1600.0)[1], inactive(0.5));
    }

    #[test]
    fn the_entity_keeps_full_alpha_below_the_far_band_after_creation() {
        for height in [50.0, 300.0, 450.0, 800.0, 1100.0] {
            let mut fades = Fades::default();
            assert_eq!(run_at(&mut fades, height)[0], ON, "created at {height}");
            for later in [300.0, 450.0, 1100.0] {
                assert_eq!(run_at(&mut fades, later)[0], ON, "created at {height}, then {later}");
            }
        }
    }

    #[test]
    fn the_entity_keeps_its_far_band_state_when_the_camera_comes_back_down() {
        let mut fades = Fades::default();
        assert_eq!(run_at(&mut fades, 1300.0), [active(0.5), active(0.5)]);
        assert_eq!(run_at(&mut fades, 800.0), [active(0.5), active(1.0)]);
        assert_eq!(run_at(&mut fades, 300.0), [active(0.5), inactive(1.0)]);
        assert_eq!(run_at(&mut fades, 1600.0), [inactive(0.5), inactive(1.0)]);
        assert_eq!(run_at(&mut fades, 800.0), [inactive(0.5), active(1.0)]);
        assert_eq!(run_at(&mut fades, 1167.0)[0], active((1500.0 - 1167.0) / 400.0));
    }

    #[test]
    fn an_unlisted_region_keeps_its_states_and_new_weather_waits_for_its_run() {
        let defines = super::super::tests::reference_defines();
        let mut fades = Fades::default();
        fades.produce([1, 2], 1300.0, &defines, |region| region == 1);
        assert_eq!(fades.members(1), [active(0.5); 2]);
        assert_eq!(fades.members(2), [Member::ABSENT; 2], "no run has created its pairs");
        fades.produce([1, 2], 800.0, &defines, |region| region == 2);
        assert_eq!(fades.members(1), [active(0.5); 2], "region 1 was not listed");
        assert_eq!(fades.members(2), [ON, active(1.0)]);
        assert_eq!(created(800.0, &defines), fades.members(2));
        fades.remove(1);
        assert_eq!(fades.members(1), [Member::ABSENT; 2]);
        fades.produce([1, 2], 800.0, &defines, |_| true);
        assert_eq!(fades.members(1), [ON, active(1.0)]);
    }

    #[test]
    fn a_region_is_listed_while_a_defined_province_of_it_is_in_the_update_list() {
        let regions = BTreeMap::from([
            (10, StrategicRegion { id: 10, provinces: vec![1, 2], ..Default::default() }),
            (20, StrategicRegion { id: 20, provinces: vec![3, 999], ..Default::default() }),
        ]);
        let provinces = RegionProvinces::new(&regions, |province| province != 999);
        let mut schedule = Schedule::default();
        for phase in 1..=4 {
            schedule.advance([1, 2, 3], |_| false, false);
            assert_eq!(
                [provinces.listed(10, &schedule), provinces.listed(20, &schedule)],
                [phase <= 2, phase <= 3],
                "phase {phase}"
            );
        }
        assert!(!provinces.listed(30, &schedule));
        schedule.advance([1, 2, 3], |id| id == 2, false);
        assert!(provinces.listed(10, &schedule) && !provinces.listed(20, &schedule));
    }
}
