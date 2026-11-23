
use engine_native_memory::{Memory, read};
use engine_native::ccountry;
use engine_native_parts::{cstrategic_region, modifier_accumulator};
use engine_sites::Rva;

const COUNTRY_REGION_MAP: usize = 0x4d8;
const MAP_SENTINEL: usize = 0x08;
const MAP_SIZE: usize = 0x10;
const MAP_BUCKETS: usize = 0x18;
const MAP_MASK: usize = 0x30;
const MAP_BUCKET_COUNT: usize = 0x38;
const NODE_STRIDE: usize = 0x20;
const NODE_VALUE: usize = 0x18;
const REGION_ACCUMULATOR: usize = 0x50;
const REGION_OBJECT_VTABLE_RVA: Rva = Rva(0x0286_44f0);
pub const SORTIE_EFFICIENCY_KEY: u32 = 0x11;

pub const GROUND_CREWS_ENABLE_RVA: Rva = Rva(0x0073_13f0);
pub const GROUND_CREWS_DISABLE_RVA: Rva = Rva(0x0073_0e80);
pub const GROUND_CREWS_CONSTRUCTOR_RVA: Rva = Rva(0x0071_3070);
const GROUND_CREWS_ENABLED: usize = 0x38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    object: usize,
}

impl Region {
    pub fn object(self) -> usize {
        self.object
    }
    pub fn ground_crews(self, memory: &impl Memory) -> Option<u8> {
        read::<u8>(memory, self.object.checked_add(GROUND_CREWS_ENABLED)?)
            .map(|value| u8::from(value != 0))
    }
    pub fn efficiency(
        self,
        memory: &impl Memory,
    ) -> Result<engine_native_memory::modifier_cache::Value, &'static str> {
        let accumulator = self
            .object
            .checked_add(REGION_ACCUMULATOR)
            .ok_or("air modifier accumulator address overflow")?;
        modifier_accumulator::lookup_at(memory, accumulator, SORTIE_EFFICIENCY_KEY)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryMap {
    sentinel: usize,
    size: u64,
    buckets: usize,
    mask: u64,
}

pub fn country_map(
    memory: &impl Memory,
    base: usize,
    country: usize,
) -> Result<CountryMap, &'static str> {
    authenticate(
        memory,
        base,
        country,
        ccountry::VTABLE_RVA,
        "air modifier country identity invalid",
    )?;
    let map = country
        .checked_add(COUNTRY_REGION_MAP)
        .ok_or("air modifier map address overflow")?;
    let header = read::<[u8; 64]>(memory, map).ok_or("air modifier map header unreadable")?;
    let sentinel = word(&header, MAP_SENTINEL)?;
    let size = word(&header, MAP_SIZE)? as u64;
    let buckets = word(&header, MAP_BUCKETS)?;
    let mask = word(&header, MAP_MASK)? as u64;
    let bucket_count = word(&header, MAP_BUCKET_COUNT)? as u64;
    if sentinel == 0
        || buckets == 0
        || bucket_count == 0
        || !bucket_count.is_power_of_two()
        || mask != bucket_count - 1
    {
        return Err("air modifier map header invalid");
    }
    let links = read::<[u8; 16]>(memory, sentinel).ok_or("air modifier map sentinel unreadable")?;
    let sentinel_prev = word(&links, 0)?;
    let sentinel_next = word(&links, 8)?;
    if (size == 0 && (sentinel_prev != sentinel || sentinel_next != sentinel))
        || (size != 0 && (sentinel_prev == 0 || sentinel_next == 0))
    {
        return Err("air modifier map sentinel invalid");
    }
    Ok(CountryMap {
        sentinel,
        size,
        buckets,
        mask,
    })
}

impl CountryMap {
    pub fn visit(
        self,
        memory: &impl Memory,
        base: usize,
        mut each: impl FnMut(usize, u32, Region) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        let mut current = read::<u64>(
            memory,
            self.sentinel
                .checked_add(8)
                .ok_or("air modifier sentinel overflow")?,
        )
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("air modifier sentinel unreadable")?;
        let mut previous = self.sentinel;
        for _ in 0..self.size {
            if current == 0 || current == self.sentinel {
                return Err("air modifier list ended before its count");
            }
            let node =
                read::<[u8; NODE_STRIDE]>(memory, current).ok_or("air modifier node unreadable")?;
            if word(&node, 0)? != previous {
                return Err("air modifier list backlink invalid");
            }
            let key = word(&node, 0x10)?;
            let value = word(&node, NODE_VALUE)?;
            let id = cstrategic_region::id(memory, base, key)?
                .ok_or("air modifier region key is null")?;
            authenticate(
                memory,
                base,
                value,
                REGION_OBJECT_VTABLE_RVA,
                "air modifier region object identity invalid",
            )?;
            each(key, id, Region { object: value })?;
            previous = current;
            current = word(&node, 8)?;
        }
        if current != self.sentinel {
            return Err("air modifier list exceeds its count");
        }
        Ok(())
    }

    pub fn lookup(
        self,
        memory: &impl Memory,
        base: usize,
        region: usize,
    ) -> Result<engine_native_memory::modifier_cache::Value, &'static str> {
        self.region(memory, base, region)?
            .map_or(Ok(engine_native_memory::modifier_cache::Value::default()), |object| {
                object.efficiency(memory)
            })
    }

    pub fn region(
        self,
        memory: &impl Memory,
        base: usize,
        region: usize,
    ) -> Result<Option<Region>, &'static str> {
        let Self {
            sentinel,
            size,
            buckets,
            mask,
        } = self;
        let secondary = region
            .checked_add(cstrategic_region::SECONDARY)
            .ok_or("air modifier region address overflow")?;
        let [mut primary, mut second] = [[0_u8; 8]; 2];
        let vtable = |rva: Rva| engine_sites::at(base, rva).map(|value| value as u64);
        if region == 0
            || memory.copy_scattered([region, secondary], &mut [&mut primary, &mut second]) != 2
            || Some(u64::from_le_bytes(primary)) != vtable(cstrategic_region::VTABLE_RVA)
        {
            return Err("air modifier region identity invalid");
        }
        if Some(u64::from_le_bytes(second)) != vtable(cstrategic_region::SECONDARY_VTABLE_RVA) {
            return Err("air modifier region secondary identity invalid");
        }
        let index = hash(region as u64) & mask;
        let bucket = buckets
            .checked_add(
                usize::try_from(index)
                    .ok()
                    .and_then(|i| i.checked_mul(16))
                    .ok_or("air modifier bucket address overflow")?,
            )
            .ok_or("air modifier bucket address overflow")?;
        let pair = read::<[u8; 16]>(memory, bucket).ok_or("air modifier bucket unreadable")?;
        let last = word(&pair, 0)?;
        let first = word(&pair, 8)?;
        if first == sentinel && last == sentinel {
            return Ok(None);
        }
        if size == 0 || first == 0 || last == 0 || first == sentinel || last == sentinel {
            return Err("air modifier bucket bounds invalid");
        }

        let mut current = first;
        let mut previous = None;
        let mut selected = None;
        for _ in 0..size {
            if current == 0 || current == sentinel {
                return Err("air modifier bucket ended before last node");
            }
            let node =
                read::<[u8; NODE_STRIDE]>(memory, current).ok_or("air modifier node unreadable")?;
            let prev = word(&node, 0)?;
            let next = word(&node, 8)?;
            let key = word(&node, 0x10)?;
            let value = word(&node, NODE_VALUE)?;
            if prev == 0
                || next == 0
                || prev == current
                || next == current
                || (hash(key as u64) & mask) != index
                || previous.is_some_and(|prior| prev != prior)
            {
                return Err("air modifier bucket node invalid");
            }
            if key == region {
                if value == 0 {
                    return Err("air modifier region value null");
                }
                if selected.replace(value).is_some() {
                    return Err("air modifier region key duplicated");
                }
            }
            if current == last {
                break;
            }
            previous = Some(current);
            current = next;
        }
        if current != last {
            return Err("air modifier bucket exceeds map size");
        }
        let Some(value) = selected else {
            return Ok(None);
        };
        authenticate(
            memory,
            base,
            value,
            REGION_OBJECT_VTABLE_RVA,
            "air modifier region object identity invalid",
        )?;
        Ok(Some(Region { object: value }))
    }
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable_rva: Rva,
    reason: &'static str,
) -> Result<(), &'static str> {
    if pointer == 0 {
        return Err(reason);
    }
    let expected = engine_sites::at(base, vtable_rva).ok_or(reason)?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err(reason);
    }
    Ok(())
}

fn word(bytes: &[u8], offset: usize) -> Result<usize, &'static str> {
    usize::try_from(u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .map_err(|_| "air modifier field width invalid")?,
    ))
    .map_err(|_| "air modifier pointer outside address width")
}

fn hash(pointer: u64) -> u64 {
    pointer
        .to_le_bytes()
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const COUNTRY: usize = 0x1000;
    const REGION: usize = 0x2000;
    const BUCKETS: usize = 0x3000;
    const SENTINEL: usize = 0x4000;
    const NODE_A: usize = 0x5000;
    const NODE_B: usize = 0x5100;
    const VALUE: usize = 0x6000;
    const ENTRIES: usize = 0x7000;

    fn put(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn map(bytes: &mut [u8], size: u64, first: usize, last: usize, region: usize) {
        let root = COUNTRY + COUNTRY_REGION_MAP;
        put(bytes, root + MAP_SENTINEL, SENTINEL as u64);
        put(bytes, root + MAP_SIZE, size);
        put(bytes, root + MAP_BUCKETS, BUCKETS as u64);
        put(bytes, root + MAP_MASK, 7);
        put(bytes, root + MAP_BUCKET_COUNT, 8);
        let bucket = BUCKETS + (hash(region as u64) as usize & 7) * 16;
        put(bytes, bucket, last as u64);
        put(bytes, bucket + 8, first as u64);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        put(&mut bytes, COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        put(&mut bytes, REGION, cstrategic_region::VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            REGION + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, SENTINEL, SENTINEL as u64);
        put(&mut bytes, SENTINEL + 8, SENTINEL as u64);
        map(&mut bytes, 0, SENTINEL, SENTINEL, REGION);
        bytes
    }

    fn present(bytes: &mut [u8], region: usize, raw: i64) {
        put(bytes, SENTINEL, NODE_A as u64);
        put(bytes, SENTINEL + 8, NODE_A as u64);
        map(bytes, 1, NODE_A, NODE_A, region);
        put(bytes, NODE_A, SENTINEL as u64);
        put(bytes, NODE_A + 8, SENTINEL as u64);
        put(bytes, NODE_A + 0x10, region as u64);
        put(bytes, NODE_A + NODE_VALUE, VALUE as u64);
        put(bytes, VALUE, REGION_OBJECT_VTABLE_RVA.0 as u64);
        put(bytes, VALUE + REGION_ACCUMULATOR, ENTRIES as u64);
        bytes[VALUE + REGION_ACCUMULATOR + 8..VALUE + REGION_ACCUMULATOR + 12]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[VALUE + REGION_ACCUMULATOR + 12..VALUE + REGION_ACCUMULATOR + 16]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[ENTRIES..ENTRIES + 4].copy_from_slice(&SORTIE_EFFICIENCY_KEY.to_le_bytes());
        bytes[ENTRIES + 8..ENTRIES + 16].copy_from_slice(&raw.to_le_bytes());
    }

    fn read_fixture(
        bytes: &[u8],
        region: usize,
    ) -> Result<engine_native_memory::modifier_cache::Value, &'static str> {
        let memory = SliceMemory { base: 0, bytes };
        country_map(&memory, 0, COUNTRY)?.lookup(&memory, 0, region)
    }

    #[test]
    fn ground_crews_are_a_separate_optional_byte_of_the_authenticated_region() {
        let mut bytes = fixture();
        present(&mut bytes, REGION, -27_500);
        for (stored, expected) in [(0, 0), (1, 1), (9, 1)] {
            bytes[VALUE + GROUND_CREWS_ENABLED] = stored;
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let object = country_map(&memory, 0, COUNTRY)
                .unwrap()
                .region(&memory, 0, REGION)
                .unwrap()
                .unwrap();
            assert_eq!(object.object(), VALUE);
            assert_eq!(object.ground_crews(&memory), Some(expected));
            assert_eq!(object.efficiency(&memory).unwrap().raw, -27_500);
        }
        struct MissingByte<'a>(SliceMemory<'a>);
        impl Memory for MissingByte<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= VALUE + GROUND_CREWS_ENABLED
                    && address.saturating_add(destination.len()) > VALUE + GROUND_CREWS_ENABLED
                {
                    return false;
                }
                self.0.copy(address, destination)
            }
        }
        let memory = MissingByte(SliceMemory {
            base: 0,
            bytes: &bytes,
        });
        let object = country_map(&memory, 0, COUNTRY)
            .unwrap()
            .region(&memory, 0, REGION)
            .unwrap()
            .unwrap();
        assert_eq!(object.ground_crews(&memory), None);
        assert_eq!(object.efficiency(&memory).unwrap().raw, -27_500);
    }

    #[test]
    fn empty_map_is_absent_and_present_raw_value_preserves_sign() {
        let mut bytes = fixture();
        assert_eq!(
            read_fixture(&bytes, REGION),
            Ok(engine_native_memory::modifier_cache::Value::default())
        );
        present(&mut bytes, REGION, -27_500);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Ok(engine_native_memory::modifier_cache::Value {
                raw: -27_500,
                present: true
            })
        );
        bytes[ENTRIES..ENTRIES + 4].copy_from_slice(&0x10_u32.to_le_bytes());
        assert_eq!(
            read_fixture(&bytes, REGION),
            Ok(engine_native_memory::modifier_cache::Value::default())
        );
    }

    #[test]
    fn walks_collisions_to_the_last_node_and_distinguishes_a_miss() {
        let mut bytes = fixture();
        present(&mut bytes, REGION, 123);
        let other = (REGION + 0x10..0x2f00)
            .step_by(8)
            .find(|&candidate| hash(candidate as u64) & 7 == hash(REGION as u64) & 7)
            .unwrap();
        put(&mut bytes, other, cstrategic_region::VTABLE_RVA.0 as u64);
        put(
            &mut bytes,
            other + cstrategic_region::SECONDARY,
            cstrategic_region::SECONDARY_VTABLE_RVA.0 as u64,
        );
        assert_eq!(
            read_fixture(&bytes, other),
            Ok(engine_native_memory::modifier_cache::Value::default())
        );
        put(&mut bytes, SENTINEL, NODE_B as u64);
        put(&mut bytes, SENTINEL + 8, NODE_A as u64);
        map(&mut bytes, 2, NODE_A, NODE_B, REGION);
        put(&mut bytes, NODE_A + 8, NODE_B as u64);
        put(&mut bytes, NODE_B, NODE_A as u64);
        put(&mut bytes, NODE_B + 8, SENTINEL as u64);
        put(&mut bytes, NODE_B + 0x10, other as u64);
        put(&mut bytes, NODE_B + NODE_VALUE, VALUE as u64);
        assert_eq!(
            read_fixture(&bytes, other),
            Ok(engine_native_memory::modifier_cache::Value {
                raw: 123,
                present: true
            })
        );
        put(&mut bytes, NODE_B + 0x10, REGION as u64);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier region key duplicated")
        );
        put(&mut bytes, NODE_B + 0x10, other as u64);
        put(&mut bytes, NODE_B + 8, NODE_B as u64);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier bucket node invalid")
        );
    }

    #[test]
    fn rejects_bad_identity_header_bucket_and_operand() {
        let mut bytes = fixture();
        put(&mut bytes, REGION, 0);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier region identity invalid")
        );
        put(&mut bytes, REGION, cstrategic_region::VTABLE_RVA.0 as u64);
        put(&mut bytes, COUNTRY + COUNTRY_REGION_MAP + MAP_MASK, 3);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier map header invalid")
        );
        put(&mut bytes, COUNTRY + COUNTRY_REGION_MAP + MAP_MASK, 7);
        present(&mut bytes, REGION, 456);
        put(&mut bytes, NODE_A + NODE_VALUE, 0);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier region value null")
        );
        put(&mut bytes, NODE_A + NODE_VALUE, VALUE as u64);
        put(&mut bytes, VALUE, 0);
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("air modifier region object identity invalid")
        );
        put(&mut bytes, VALUE, REGION_OBJECT_VTABLE_RVA.0 as u64);
        bytes[VALUE + REGION_ACCUMULATOR + 12..VALUE + REGION_ACCUMULATOR + 16]
            .copy_from_slice(&2_u32.to_le_bytes());
        assert_eq!(
            read_fixture(&bytes, REGION),
            Err("modifier accumulator header unreadable or invalid")
        );
    }

    #[test]
    fn short_node_and_accumulator_reads_cannot_look_like_absence() {
        let mut bytes = fixture();
        present(&mut bytes, REGION, 789);
        assert_eq!(
            read_fixture(&bytes[..NODE_A + 8], REGION),
            Err("air modifier node unreadable")
        );
        assert_eq!(
            read_fixture(&bytes[..ENTRIES + 12], REGION),
            Err("modifier accumulator entry unreadable")
        );
    }
}
