use super::*;
use engine_sites::Rva;

const DISRUPTION: usize = 0xa8;
const ATTACK_STAT: usize = 0x30;
const DEFENCE_STAT: usize = 0x2f;
const DEFAULT_SPEED: usize = 0x580;
const COMBAT_ATTACK: u32 = 0x6f;
const COMBAT_DEFENCE: u32 = 0x70;

pub const SPOTTING_FACTORS_RVA: Rva = Rva(0x033e_7208);

#[derive(Default)]
pub struct SpottingFactors {
    header: Option<Result<Array, &'static str>>,
    kinds: [Option<Result<bool, &'static str>>; u32::BITS as usize],
}

impl SpottingFactors {
    pub fn admits(
        &mut self,
        memory: &impl Memory,
        base: usize,
        mask: u32,
    ) -> Result<bool, &'static str> {
        if mask as i32 <= 0 || !mask.is_power_of_two() {
            return Ok(false);
        }
        let index = mask.trailing_zeros() as usize;
        if let Some(value) = self.kinds[index] {
            return value;
        }
        let header = *self.header.get_or_insert_with(|| {
            engine_sites::at(base, SPOTTING_FACTORS_RVA)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("Air patrol spotting-factor array unreadable")
        });
        let value = header.and_then(|array| {
            if index >= array.count as usize {
                return Ok(false);
            }
            array
                .element(index as u32, 8)
                .and_then(|at| engine_native_memory::read::<i64>(memory, at))
                .map(|value| value != 0)
                .ok_or("Air patrol mission spotting factor unreadable")
        });
        self.kinds[index] = Some(value);
        value
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub disruption: Option<i64>,
    pub attack: Option<i64>,
    pub defence: Option<i64>,
    pub speed: Option<i64>,
    pub attack_modifier: Option<i64>,
    pub defence_modifier: Option<i64>,
    pub night_applied: Option<i64>,
}

const NIGHT_APPLIED: usize = 0x268;

pub fn demanded(input: &super::Mission) -> bool {
    matches!((input.current_type, input.effective_aircraft, input.normalization),
        (Some(mask), Some(count), Some(normalization)) if count > 0 && mask as i32 > 0 && mask.is_power_of_two() && mask & 0x1f7fa != 0 && mask & 0x4400 == 0 && normalization.patrol.disruption.is_some_and(|raw| raw != 0))
}

pub struct HeldNight {
    clocks: Array,
    hour: i32,
}

impl HeldNight {
    pub fn read(
        memory: &impl Memory,
        game_state: usize,
        manager: Result<usize, &'static str>,
    ) -> Result<Option<Self>, &'static str> {
        let manager = manager?;
        if field::<[u8; 2]>(memory, manager, cweather_manager::SUSPEND_FLAGS) == Ok([0, 0]) {
            return Ok(None);
        }
        let clocks = manager
            .checked_add(cweather_manager::PROVINCE_CLOCKS)
            .and_then(|at| {
                Array::read(
                    memory,
                    at,
                    engine_native::cweather_manager::province_weather::CLOCK_STRIDE,
                )
            })
            .ok_or("Air patrol held clock array unreadable")?;
        let hour = field(memory, game_state, cgame_state::CURRENT_HOUR)?;
        Ok(Some(Self { clocks, hour }))
    }

    pub fn applied(
        &self,
        memory: &impl Memory,
        base: usize,
        region: usize,
        expected_region: Option<u32>,
    ) -> Result<i64, &'static str> {
        if engine_native_parts::cstrategic_region::id(memory, base, region)? != expected_region
            || expected_region.is_none()
        {
            return Err("Air patrol sampled region identity differs from its registration");
        }
        let provinces = region
            .checked_add(engine_native_parts::cstrategic_region::PROVINCES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("Air patrol sampled region province vector unreadable")?;
        let count = i32::try_from(provinces.count)
            .map_err(|_| "Air patrol sampled region province count overflow")?;
        let index = self
            .hour
            .wrapping_sub(43_800_000)
            .checked_rem(count)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or("Air patrol sampled region has no native province index")?;
        let province = provinces
            .element(index, 8)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|&at| at != 0)
            .ok_or("Air patrol sampled province pointer unreadable")?;
        if field::<u64>(memory, province, 0)? != vtable(base, engine_native::cprovince::VTABLE_RVA)? {
            return Err("Air patrol sampled province identity differs");
        }
        let id: u32 = field(memory, province, engine_native::cprovince::ID)?;
        self.clocks
            .element(id, engine_native::cweather_manager::province_weather::CLOCK_STRIDE)
            .and_then(|at| at.checked_add(NIGHT_APPLIED))
            .and_then(|at| engine_native_memory::read(memory, at))
            .ok_or("Air patrol held night application marker unreadable")
    }
}

pub(super) fn read(
    memory: &impl Memory,
    wing: usize,
    mission: usize,
    record: Option<usize>,
    mask: u32,
    count: i32,
) -> Inputs {
    let mut inputs = Inputs::default();
    if mask as i32 <= 0 || !mask.is_power_of_two() || mask & 0x1f7fa == 0 {
        return inputs;
    }
    inputs.disruption = mission
        .checked_add(DISRUPTION)
        .and_then(|at| engine_native_memory::read(memory, at));
    if inputs.disruption == Some(0) || mask & 0x4400 != 0 || count <= 0 {
        return inputs;
    }
    if inputs.disruption.is_none() {
        return inputs;
    }
    inputs.attack = record
        .and_then(|at| at.checked_add(ATTACK_STAT * 8))
        .and_then(|at| engine_native_memory::read(memory, at));
    inputs.defence = record
        .and_then(|at| at.checked_add(DEFENCE_STAT * 8))
        .and_then(|at| engine_native_memory::read(memory, at));
    inputs.speed = wing
        .checked_add(DEFAULT_SPEED)
        .and_then(|at| engine_native_memory::read(memory, at));
    let accumulator = wing
        .checked_add(cair_wing::MODIFIERS)
        .and_then(|at| header(memory, at));
    inputs.attack_modifier =
        accumulator.and_then(|(data, count)| probe(memory, data, count, COMBAT_ATTACK));
    inputs.defence_modifier =
        accumulator.and_then(|(data, count)| probe(memory, data, count, COMBAT_DEFENCE));
    inputs
}

fn header(memory: &impl Memory, accumulator: usize) -> Option<(usize, usize)> {
    let data = usize::try_from(engine_native_memory::read::<u64>(memory, accumulator)?).ok()?;
    let count = engine_native_memory::read::<i32>(
        memory,
        accumulator.checked_add(modifier_accumulator::COUNT)?,
    )?;
    let count = usize::try_from(count).ok()?;
    data.checked_add(count.checked_mul(16)?)?;
    Some((data, count))
}

fn probe(memory: &impl Memory, data: usize, count: usize, key: u32) -> Option<i64> {
    let mut upper = count;
    let mut lower = 0;
    while lower < upper {
        let mid = lower + (upper - lower) / 2;
        let candidate = data.checked_add(mid.checked_mul(16)?)?;
        let found = engine_native_memory::read::<u32>(memory, candidate)?;
        if found < key {
            lower = mid + 1;
        } else {
            upper = mid;
        }
    }
    if lower == count {
        return Some(0);
    }
    let candidate = data.checked_add(lower.checked_mul(16)?)?;
    if engine_native_memory::read::<u32>(memory, candidate)? != key {
        return Some(0);
    }
    engine_native_memory::read(memory, candidate.checked_add(8)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn raw(memory: &impl Memory, accumulator: usize, key: u32) -> Option<i64> {
        let (data, count) = header(memory, accumulator)?;
        probe(memory, data, count, key)
    }

    #[test]
    fn spotting_factors_keep_dynamic_extent_and_cache_each_original_kind_result() {
        struct Factors {
            header_reads: std::cell::Cell<usize>,
            words: std::cell::RefCell<Vec<usize>>,
            header_fault: bool,
        }
        impl Memory for Factors {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at == SPOTTING_FACTORS_RVA.0 {
                    self.header_reads.set(self.header_reads.get() + 1);
                    if self.header_fault {
                        return false;
                    }
                    let mut bytes = [0; 16];
                    bytes[..8].copy_from_slice(&4096_u64.to_le_bytes());
                    bytes[8..12].copy_from_slice(&45_u32.to_le_bytes());
                    bytes[12..].copy_from_slice(&45_u32.to_le_bytes());
                    out.copy_from_slice(&bytes);
                    return true;
                }
                let index = (at - 4096) / 8;
                self.words.borrow_mut().push(index);
                if index == 2 {
                    return false;
                }
                out.copy_from_slice(&(if index == 0 { 0_i64 } else { -1_i64 }).to_le_bytes());
                true
            }
        }
        let memory = Factors {
            header_reads: Default::default(),
            words: Default::default(),
            header_fault: false,
        };
        let mut cache = SpottingFactors::default();
        for _ in 0..2 {
            assert_eq!(cache.admits(&memory, 0, 1), Ok(false));
            assert!(cache.admits(&memory, 0, 4).is_err());
            assert_eq!(cache.admits(&memory, 0, 8), Ok(true));
            assert_eq!(cache.admits(&memory, 0, 1 << 30), Ok(true));
        }
        assert_eq!(memory.header_reads.get(), 1);
        assert_eq!(*memory.words.borrow(), [0, 2, 3, 30]);
        let memory = Factors {
            header_reads: Default::default(),
            words: Default::default(),
            header_fault: true,
        };
        let mut cache = SpottingFactors::default();
        assert!(cache.admits(&memory, 0, 1).is_err());
        assert!(cache.admits(&memory, 0, 8).is_err());
        assert_eq!(memory.header_reads.get(), 1);
        assert!(memory.words.borrow().is_empty());
    }

    #[test]
    fn updated_night_reuses_the_original_manager_failure_and_reads_only_gates() {
        struct Gates(std::cell::Cell<usize>);
        impl Memory for Gates {
            fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
                self.0.set(self.0.get() + 1);
                assert_eq!(
                    (at, bytes.len()),
                    (512 + cweather_manager::SUSPEND_FLAGS, 2)
                );
                bytes.fill(0);
                true
            }
        }
        let memory = Gates(Default::default());
        assert!(HeldNight::read(&memory, 0, Err("original manager refused")).is_err());
        assert_eq!(memory.0.get(), 0);
        assert!(matches!(HeldNight::read(&memory, 0, Ok(512)), Ok(None)));
        assert_eq!(memory.0.get(), 1);
    }

    #[test]
    fn held_night_samples_original_order_and_preserves_marker_fault() {
        const GAME: usize = 512;
        const MANAGER: usize = 2048;
        const REGION: usize = 4096;
        const LIST: usize = 8192;
        const FIRST: usize = 12288;
        const SECOND: usize = 12800;
        const CLOCKS: usize = 16384;
        let mut bytes = vec![0_u8; 28672];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        put(MANAGER + cweather_manager::SUSPEND_FLAGS, &[1, 0]);
        put(
            MANAGER + cweather_manager::PROVINCE_CLOCKS,
            &(CLOCKS as u64).to_le_bytes(),
        );
        put(
            MANAGER + cweather_manager::PROVINCE_CLOCKS + 8,
            &3_u32.to_le_bytes(),
        );
        put(
            MANAGER + cweather_manager::PROVINCE_CLOCKS + 12,
            &3_u32.to_le_bytes(),
        );
        put(
            GAME + cgame_state::CURRENT_HOUR,
            &43_800_001_i32.to_le_bytes(),
        );
        put(
            REGION,
            &(engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(REGION + engine_native_parts::cstrategic_region::ID, &7_u32.to_le_bytes());
        put(
            REGION + engine_native_parts::cstrategic_region::PROVINCES,
            &(LIST as u64).to_le_bytes(),
        );
        put(
            REGION + engine_native_parts::cstrategic_region::PROVINCES + 8,
            &2_u32.to_le_bytes(),
        );
        put(
            REGION + engine_native_parts::cstrategic_region::PROVINCES_COUNT,
            &2_u32.to_le_bytes(),
        );
        put(LIST, &(FIRST as u64).to_le_bytes());
        put(LIST + 8, &(SECOND as u64).to_le_bytes());
        for (pointer, id) in [(FIRST, 1_u32), (SECOND, 2)] {
            put(
                pointer,
                &(engine_native::cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(pointer + engine_native::cprovince::ID, &id.to_le_bytes());
        }
        let first = CLOCKS + engine_native::cweather_manager::province_weather::CLOCK_STRIDE + NIGHT_APPLIED;
        let second =
            CLOCKS + 2 * engine_native::cweather_manager::province_weather::CLOCK_STRIDE + NIGHT_APPLIED;
        put(first, &(-100_000_i64).to_le_bytes());
        put(second, &50_001_i64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let held = HeldNight::read(&memory, GAME, Ok(MANAGER))
            .unwrap()
            .unwrap();
        assert_eq!(held.applied(&memory, 0, REGION, Some(7)), Ok(50_001));
        struct Fault<'a> {
            bytes: &'a [u8],
            at: usize,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                if at <= self.at && self.at < at + out.len() {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, out)
            }
        }
        let fault = Fault {
            bytes: &bytes,
            at: second,
        };
        assert!(
            held.applied(&fault, 0, REGION, Some(7))
                .unwrap_err()
                .contains("application marker unreadable")
        );
        assert!(held.applied(&memory, 0, REGION, Some(9)).is_err());
    }

    #[test]
    fn raw_combat_probes_keep_signed_values_and_known_absence() {
        const ACCUMULATOR: usize = 16;
        let mut bytes = vec![0_u8; 160];
        bytes[ACCUMULATOR..ACCUMULATOR + 8].copy_from_slice(&64_u64.to_le_bytes());
        bytes[ACCUMULATOR + modifier_accumulator::COUNT
            ..ACCUMULATOR + modifier_accumulator::COUNT + 4]
            .copy_from_slice(&3_i32.to_le_bytes());
        for (index, (key, value)) in [
            (5_u32, 17_i64),
            (COMBAT_ATTACK, -7),
            (COMBAT_DEFENCE, i64::MIN),
        ]
        .into_iter()
        .enumerate()
        {
            let at = 64 + index * 16;
            bytes[at..at + 4].copy_from_slice(&key.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(raw(&memory, ACCUMULATOR, COMBAT_ATTACK), Some(-7));
        assert_eq!(raw(&memory, ACCUMULATOR, COMBAT_DEFENCE), Some(i64::MIN));
        assert_eq!(raw(&memory, ACCUMULATOR, 6), Some(0));
        assert_eq!(raw(&memory, ACCUMULATOR, 0xffff), Some(0));
    }

    #[test]
    fn disruption_gates_skip_combat_reads_and_faults_remain_independent() {
        struct Traced<'a> {
            bytes: &'a [u8],
            denied: Option<usize>,
            reads: std::cell::RefCell<Vec<usize>>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push(at);
                if self
                    .denied
                    .is_some_and(|denied| at <= denied && denied < at + output.len())
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, output)
            }
        }
        const WING: usize = 512;
        const MISSION: usize = WING + cair_wing::MISSION;
        const RECORD: usize = 4096;
        let mut bytes = vec![0_u8; 8192];
        bytes[RECORD + ATTACK_STAT * 8..RECORD + ATTACK_STAT * 8 + 8]
            .copy_from_slice(&17_i64.to_le_bytes());
        bytes[RECORD + DEFENCE_STAT * 8..RECORD + DEFENCE_STAT * 8 + 8]
            .copy_from_slice(&23_i64.to_le_bytes());
        bytes[WING + DEFAULT_SPEED..WING + DEFAULT_SPEED + 8]
            .copy_from_slice(&29_i64.to_le_bytes());
        for (mask, count, disruption) in
            [(8, 7, 0), (1, 7, 9), (0x400, 7, 9), (8, -7, 9), (8, 7, 9)]
        {
            bytes[MISSION + DISRUPTION..MISSION + DISRUPTION + 8]
                .copy_from_slice(&i64::from(disruption).to_le_bytes());
            let memory = Traced {
                bytes: &bytes,
                denied: None,
                reads: Default::default(),
            };
            let inputs = read(&memory, WING, MISSION, Some(RECORD), mask, count);
            if mask == 1 {
                assert!(memory.reads.borrow().is_empty());
            }
            let combat = disruption != 0 && mask == 8 && count > 0;
            assert_eq!(inputs.attack, combat.then_some(17));
            assert_eq!(inputs.defence, combat.then_some(23));
            assert_eq!(inputs.speed, combat.then_some(29));
            assert_eq!(inputs.attack_modifier, combat.then_some(0));
            assert_eq!(inputs.defence_modifier, combat.then_some(0));
        }
        let memory = Traced {
            bytes: &bytes,
            denied: Some(RECORD + ATTACK_STAT * 8),
            reads: Default::default(),
        };
        let inputs = read(&memory, WING, MISSION, Some(RECORD), 8, 7);
        assert_eq!(inputs.attack, None);
        assert_eq!(inputs.defence, Some(23));
        assert_eq!(inputs.speed, Some(29));
        assert_eq!(inputs.disruption, Some(9));
        assert_eq!(inputs.attack_modifier, Some(0));
    }
}
