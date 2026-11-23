use engine_native_memory::{Memory, read};
use engine_allocation as allocation;
use engine_native_parts::{cconvoys, equipment_entry};
use engine_native::{
    ccountry,
    cequipment_variant::pool as cequipment_variant_pool,
};

const TRADE_COUNTRY: usize = 0x18;
const TRADE_CONVOYS: usize = 0x28;
const TRANSPORT_COUNTRY: usize = 0x58;
const TRANSPORT_INVASION: usize = 0x5c;
const TRANSPORT_CONVOYS: usize = 0x90;
const SURFACE_DETECTION: usize = 0x198;
const SUB_DETECTION: usize = 0x1a0;
const SURFACE_VISIBILITY: usize = 0x1a8;
const MAXIMUM_SPEED: usize = 0x208;
const SOURCE_SPEED: usize = 0x1a8;
const SOURCE_TERRAIN_CLASS: usize = 8;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpeed {
    pub base: Option<i64>,
    pub terrain_class: Option<i32>,
}

pub fn read_source_speed(memory: &impl Memory, stats: Option<usize>) -> SourceSpeed {
    SourceSpeed {
        base: stats.and_then(|pointer| word(memory, pointer, SOURCE_SPEED)),
        terrain_class: stats.and_then(|pointer| word(memory, pointer, SOURCE_TERRAIN_CLASS)),
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetOperands {
    pub country: Option<u32>,
    pub convoys: Option<i32>,
    pub invasion: Option<u8>,
}

pub(super) fn target(
    memory: &impl Memory,
    resolved: [Result<Option<usize>, &'static str>; 2],
) -> TargetOperands {
    match resolved[0] {
        Ok(Some(pointer)) => TargetOperands {
            country: word(memory, pointer, TRADE_COUNTRY),
            convoys: word(memory, pointer, TRADE_CONVOYS),
            invasion: None,
        },
        Ok(None) => match resolved[1] {
            Ok(Some(pointer)) => TargetOperands {
                country: word(memory, pointer, TRANSPORT_COUNTRY),
                convoys: word(memory, pointer, TRANSPORT_CONVOYS),
                invasion: word(memory, pointer, TRANSPORT_INVASION),
            },
            _ => TargetOperands::default(),
        },
        Err(_) => TargetOperands::default(),
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvoyDesign {
    pub variant: Option<u64>,
    pub surface_detection: Option<i64>,
    pub sub_detection: Option<i64>,
    pub visibility: Option<i64>,
    pub maximum_speed: Option<i64>,
}

#[derive(Default)]
pub struct ConvoyDesigns {
    rows: Vec<(u32, ConvoyDesign)>,
}
impl ConvoyDesigns {
    pub fn clear(&mut self) {
        self.rows.clear();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.capacity() * size_of::<(u32, ConvoyDesign)>()
    }
    pub fn read<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        countries: &ccountry::Reader<'_, M>,
        raw: u32,
    ) -> ConvoyDesign {
        let country = if raw == 0 {
            countries.reserved()
        } else {
            match countries.link(raw) {
                Ok(None) => countries.reserved(),
                Ok(Some(_)) => countries.resolve(raw),
                Err(reason) => Err(reason),
            }
        };
        let Ok(country) = country else {
            return ConvoyDesign::default();
        };
        let index = match self
            .rows
            .binary_search_by_key(&country.canonical_slot, |row| row.0)
        {
            Ok(index) => return self.rows[index].1,
            Err(index) => index,
        };
        if allocation::reserve(&mut self.rows, 1).is_err() {
            return ConvoyDesign::default();
        }
        let design = first_design(memory, base, country).unwrap_or_default();
        self.rows.insert(index, (country.canonical_slot, design));
        design
    }
}

fn first_design(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<ConvoyDesign, &'static str> {
    let convoy = country
        .pointer
        .checked_add(ccountry::CONVOY_STATE)
        .ok_or("convoy target country address overflow")?;
    if read::<u64>(memory, convoy)
        != engine_sites::at(base, cconvoys::VTABLE_RVA).map(|pointer| pointer as u64)
    {
        return Err("convoy target country embedded convoy identity mismatch");
    }
    if word::<u32>(memory, convoy, cconvoys::COUNTRY) != Some(country.id) {
        return Err("convoy target country embedded convoy owner mismatch");
    }
    let pool = convoy
        .checked_add(cconvoys::EQUIPMENT_POOL)
        .ok_or("convoy target equipment pool address overflow")?;
    let entries = cequipment_variant_pool::entries(memory, base, pool)?;
    for index in 0..entries.count {
        let at = entries
            .element(index, equipment_entry::STRIDE)
            .ok_or("convoy target equipment entry address overflow")?;
        let bytes = read::<[u8; equipment_entry::STRIDE]>(memory, at)
            .ok_or("convoy target equipment entry unreadable")?;
        let amount = i64::from_le_bytes(bytes[8..].try_into().unwrap());
        if amount == 0 {
            continue;
        }
        let pointer = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .map_err(|_| "convoy target variant pointer overflow")?;
        let variant = engine_native_parts::equipment_variant_identity::identity(memory, base, pointer)?;
        return Ok(ConvoyDesign {
            variant: Some(variant),
            surface_detection: word(memory, pointer, SURFACE_DETECTION),
            sub_detection: word(memory, pointer, SUB_DETECTION),
            visibility: word(memory, pointer, SURFACE_VISIBILITY),
            maximum_speed: word(memory, pointer, MAXIMUM_SPEED),
        });
    }
    Ok(ConvoyDesign {
        variant: Some(0),
        ..Default::default()
    })
}

fn word<T: engine_native_memory::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Option<T> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cequipment_variant;
    use std::cell::RefCell;

    struct Observed {
        world: ccountry::synthetic::World,
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Observed {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self
                .fault
                .is_some_and(|at| address <= at && at < address.saturating_add(output.len()))
            {
                return false;
            }
            self.world.memory().copy(address, output)
        }
    }
    fn fixture(amounts: &[i64]) -> (Observed, usize, usize) {
        let mut world = ccountry::synthetic::World::new(0, 2, 0x4000);
        let convoy = world.object(1) + ccountry::CONVOY_STATE;
        let pool = convoy + cconvoys::EQUIPMENT_POOL;
        let entries = world.alloc(amounts.len() * equipment_entry::STRIDE);
        let variant = world.alloc(0x300);
        world.u64(convoy, cconvoys::VTABLE_RVA.0 as u64);
        world.u32(convoy + cconvoys::COUNTRY, 1);
        world.u64(pool, cequipment_variant_pool::VTABLE_RVA.0 as u64);
        world.array(
            pool + cequipment_variant_pool::ITEMS,
            entries,
            amounts.len() as u32,
        );
        world.u64(variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        world.u64(
            variant + 8,
            (7_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
        for (index, amount) in amounts.iter().enumerate() {
            world.u64(
                entries + index * 16,
                if *amount == 0 {
                    usize::MAX as u64
                } else {
                    variant as u64
                },
            );
            world.u64(entries + index * 16 + 8, *amount as u64);
        }
        for (offset, value) in [
            (SURFACE_DETECTION, 11),
            (SUB_DETECTION, -12),
            (SURFACE_VISIBILITY, 13),
            (MAXIMUM_SPEED, 14),
        ] {
            world.u64(variant + offset, value as u64);
        }
        (
            Observed {
                world,
                reads: RefCell::new(Vec::new()),
                fault: None,
            },
            entries,
            variant,
        )
    }
    fn country(memory: &Observed) -> engine_native_memory::country_identity::Country {
        engine_native_memory::country_identity::Country {
            pointer: memory.world.object(1),
            canonical_slot: 1,
            id: 1,
        }
    }
    #[test]
    fn target_trade_priority_and_failed_trade_never_read_transport() {
        let (mut memory, _, _) = fixture(&[]);
        let trade = memory.world.alloc(0xa0);
        let transport = memory.world.alloc(0xa0);
        memory.world.u32(trade + TRADE_COUNTRY, 3);
        memory.world.u32(trade + TRADE_CONVOYS, -4_i32 as u32);
        assert_eq!(
            target(&memory, [Ok(Some(trade)), Ok(Some(transport))]),
            TargetOperands {
                country: Some(3),
                convoys: Some(-4),
                invasion: None
            }
        );
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|(at, _)| *at < transport || *at >= transport + 0xa0)
        );
        memory.reads.borrow_mut().clear();
        assert_eq!(
            target(&memory, [Err("trade unresolved"), Ok(Some(transport))]),
            TargetOperands::default()
        );
        assert!(memory.reads.borrow().is_empty());
    }
    #[test]
    fn target_transport_leaves_fail_independently() {
        let (mut memory, _, _) = fixture(&[]);
        let transport = memory.world.alloc(0xa0);
        memory.world.u32(transport + TRANSPORT_COUNTRY, 2);
        memory.world.u32(transport + TRANSPORT_CONVOYS, 9);
        memory.world.put(transport + TRANSPORT_INVASION, &[7]);
        memory.fault = Some(transport + TRANSPORT_CONVOYS);
        assert_eq!(
            target(&memory, [Ok(None), Ok(Some(transport))]),
            TargetOperands {
                country: Some(2),
                convoys: None,
                invasion: Some(7)
            }
        );
    }
    #[test]
    fn design_skips_zero_accepts_negative_and_stops_before_later_fault() {
        let (mut memory, entries, _) = fixture(&[0, -1, 20]);
        memory.fault = Some(entries + 32);
        let design = first_design(&memory, 0, country(&memory)).unwrap();
        assert_eq!(
            design.variant,
            Some((7_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE))
        );
        assert_eq!(
            (
                design.surface_detection,
                design.sub_detection,
                design.visibility,
                design.maximum_speed
            ),
            (Some(11), Some(-12), Some(13), Some(14))
        );
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|(at, _)| *at != entries + 32 && *at != usize::MAX)
        );
    }
    #[test]
    fn complete_empty_and_zero_pools_prove_no_design() {
        for amounts in [&[][..], &[0, 0][..]] {
            let (memory, _, _) = fixture(amounts);
            assert_eq!(
                first_design(&memory, 0, country(&memory)),
                Ok(ConvoyDesign {
                    variant: Some(0),
                    ..Default::default()
                })
            );
        }
    }
    #[test]
    fn design_stat_failure_keeps_identity_and_other_leaves() {
        let (mut memory, _, variant) = fixture(&[1]);
        memory.fault = Some(variant + SUB_DETECTION);
        let design = first_design(&memory, 0, country(&memory)).unwrap();
        assert!(design.variant.is_some());
        assert_eq!(design.sub_detection, None);
        assert_eq!(design.maximum_speed, Some(14));
    }
    #[test]
    fn source_speed_preserves_independent_class_and_failure_without_retrying_selector() {
        let (mut memory, _, stats) = fixture(&[1]);
        memory
            .world
            .u32(stats + SOURCE_TERRAIN_CLASS, -1_i32 as u32);
        memory.world.u64(stats + SOURCE_SPEED, 1234);
        memory.fault = Some(stats + SOURCE_SPEED);
        assert_eq!(
            read_source_speed(&memory, Some(stats)),
            SourceSpeed {
                base: None,
                terrain_class: Some(-1)
            }
        );
        memory.reads.borrow_mut().clear();
        assert_eq!(read_source_speed(&memory, None), SourceSpeed::default());
        assert!(memory.reads.borrow().is_empty());
    }
    #[test]
    fn same_pass_design_memo_reuses_country_alias_and_clear_rereads() {
        let (mut memory, entries, _) = fixture(&[1]);
        let state = memory.world.game_state();
        let links =
            read::<u64>(&memory, state + engine_native::cgame_state::COUNTRY_LINKS).unwrap() as usize;
        memory.world.u32(links + 8, 1);
        let countries = ccountry::Reader::new(&memory, 0, state).unwrap();
        let mut memo = ConvoyDesigns::default();
        let first = memo.read(&memory, 0, &countries, 1);
        assert_eq!(memo.read(&memory, 0, &countries, 2), first);
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|(at, _)| *at == entries)
                .count(),
            1
        );
        let allocated = memo.allocated_bytes();
        memo.clear();
        assert_eq!(memo.allocated_bytes(), allocated);
        assert_eq!(memo.read(&memory, 0, &countries, 1), first);
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|(at, _)| *at == entries)
                .count(),
            2
        );
    }
}
