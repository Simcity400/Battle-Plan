use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice};
use crate::{
    allocation,
    ncombat_log_cequipment_loss as loss,
    ncombat_log_corders_group_logs as group,
};
use engine_native::cequipment_variant::pool as pool;
use engine_native_parts::equipment_entry;
use engine_sites::Rva;

pub const ENEMY_KIND: u8 = 5;
pub const MANPOWER: usize = 0x98;
pub const TEMPLATES: usize = 0xb0;
pub const REFERENCES: usize = 0xc8;
pub const MANPOWER_VTABLE_RVA: Rva = Rva(0x029d_9f40);
pub const TEMPLATE_VTABLE_RVA: Rva = Rva(0x029d_a1c0);
pub const MANPOWER_SIZE: usize = 0x38;
pub const TEMPLATE_SIZE: usize = 0x38;
pub const TEMPLATE_REFERENCE: usize = 0x08;
pub const TEMPLATE_WINS: usize = 0x18;
pub const TEMPLATE_TOTAL: usize = 0x1c;
pub const TEMPLATE_DATE: usize = 0x28;
pub const MANPOWER_LOSSES: usize = 0x28;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Equipment {
    pub variant: u64,
    pub amount_raw: i64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EquipmentMonth {
    pub kind: u8,
    pub reason: i32,
    pub date: i32,
    pub allow_zero: bool,
    pub enemy: Vec<Equipment>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ManpowerMonth {
    pub reason: i32,
    pub date: i32,
    pub losses: [i32; 3],
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TemplateMonth {
    pub template: u64,
    pub date: i32,
    pub wins: i32,
    pub total: i32,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BattleReference {
    pub id: i32,
    pub attacker: bool,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct History {
    pub months: Vec<EquipmentMonth>,
    pub manpower: Vec<ManpowerMonth>,
    pub templates: Vec<TemplateMonth>,
    pub battles: Vec<BattleReference>,
}

fn push<T>(values: &mut Vec<T>, value: T) -> Result<(), &'static str> {
    allocation::reserve_push_batch(values, 1)
        .map_err(|_| "combat log history allocation failed")?;
    values.push(value);
    Ok(())
}
fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("combat log history address overflow")
}
fn module(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("combat log history address overflow")
}
fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("combat log history field unreadable")
}
fn header<const N: usize>(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: Rva,
) -> Result<[u8; N], &'static str> {
    if pointer == 0 {
        return Err("combat log history object null");
    }
    let mut bytes = [0; N];
    if !read_slice(memory, pointer, &mut bytes) {
        return Err("combat log history object unreadable");
    }
    if word(&bytes, 0) != module(base, vtable)? as u64 {
        return Err("combat log history vtable mismatch");
    }
    Ok(bytes)
}
fn word(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn int(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn array(bytes: &[u8], offset: usize, stride: usize) -> Result<Array, &'static str> {
    Array::from_header(bytes[offset..offset + 16].try_into().unwrap(), stride)
        .ok_or("combat log history array invalid")
}
fn pointers(
    memory: &impl Memory,
    bytes: &[u8],
    offset: usize,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    engine_native_memory::cpdx_array::pointers(
        memory,
        array(bytes, offset, 8)?,
        "combat log history pointer unreadable",
        |_, pointer| visit(pointer),
    )
}

pub fn read_group(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
) -> Result<History, &'static str> {
    if bytes.len() != group::SIZE || word(bytes, 0) != module(base, group::VTABLE_RVA)? as u64 {
        return Err("combat log history group header invalid");
    }
    let mut output = History::default();
    for kind in 0..=ENEMY_KIND {
        let offset = match kind {
            0..=3 => group::REASONS + kind as usize * group::REASON_STRIDE,
            4 => group::RECOVERED,
            _ => group::ENEMY_LOSSES,
        };
        pointers(memory, bytes, offset, |pointer| {
            let entry = header::<{ loss::SIZE }>(memory, base, pointer, loss::VTABLE_RVA)?;
            if word(&entry, loss::POOL) != module(base, pool::VTABLE_RVA)? as u64 {
                return Err("combat log history equipment pool vtable mismatch");
            }
            let mut month = EquipmentMonth {
                kind,
                reason: int(&entry, loss::REASON),
                date: int(&entry, loss::DATE),
                allow_zero: entry[loss::POOL + pool::ALLOW_ZERO] != 0,
                enemy: Vec::new(),
            };
            if kind == ENEMY_KIND {
                let entries = array(&entry, loss::POOL + pool::ITEMS, equipment_entry::STRIDE)?;
                for ordinal in 0..entries.count {
                    let item = equipment_entry::read(
                        memory,
                        base,
                        entries
                            .element(ordinal, equipment_entry::STRIDE)
                            .ok_or("combat log enemy equipment address overflow")?,
                        ordinal,
                    )?;
                    push(
                        &mut month.enemy,
                        Equipment {
                            variant: item.variant_handle,
                            amount_raw: item.amount_raw,
                        },
                    )?;
                }
            }
            push(&mut output.months, month)
        })?;
    }
    pointers(memory, bytes, MANPOWER, |pointer| {
        let entry = header::<MANPOWER_SIZE>(memory, base, pointer, MANPOWER_VTABLE_RVA)?;
        push(
            &mut output.manpower,
            ManpowerMonth {
                reason: int(&entry, loss::REASON),
                date: int(&entry, loss::DATE),
                losses: std::array::from_fn(|i| int(&entry, MANPOWER_LOSSES + i * 4)),
            },
        )
    })?;
    pointers(memory, bytes, TEMPLATES, |pointer| {
        let entry = header::<TEMPLATE_SIZE>(memory, base, pointer, TEMPLATE_VTABLE_RVA)?;
        push(
            &mut output.templates,
            TemplateMonth {
                template: word(&entry, TEMPLATE_REFERENCE),
                date: int(&entry, TEMPLATE_DATE),
                wins: int(&entry, TEMPLATE_WINS),
                total: int(&entry, TEMPLATE_TOTAL),
            },
        )
    })?;
    let references = array(bytes, REFERENCES, 8)?;
    for ordinal in 0..references.count {
        let pointer = references
            .element(ordinal, 8)
            .ok_or("combat log battle reference address overflow")?;
        let value: [u8; 8] = field(memory, pointer, 0)?;
        push(
            &mut output.battles,
            BattleReference {
                id: int(&value, 0),
                attacker: value[4] != 0,
            },
        )?;
    }
    Ok(output)
}

pub const BATTLE_POOL_RVA: Rva = Rva(0x033e_a870);
pub const BATTLE_COUNTS_RVA: Rva = Rva(0x033e_a888);
pub const BATTLE_USED_RVA: Rva = Rva(0x033e_a8a0);
pub const BATTLE_SIZE: usize = 0x448;
pub const BATTLE_VTABLE_RVA: Rva = Rva(0x029d_a120);
pub const SIDE_VTABLE_RVA: Rva = Rva(0x029d_9fe0);
pub const SIDE_SIZE: usize = 0x210;
pub const BATTLE_ATTACKER: usize = 0x08;
pub const BATTLE_DEFENDER: usize = 0x218;
pub const BATTLE_DATE: usize = 0x430;
pub const BATTLE_PROVINCE: usize = 0x440;
pub const BATTLE_FLAGS: usize = 0x444;
pub const SIDE_MANPOWER: usize = 0x08;
pub const SIDE_AIR_FACTOR: usize = 0x10;
pub const SIDE_EQUIPMENT: usize = 0x18;
pub const SIDE_CAPTURED_BY_ENEMY: usize = 0x158;
pub const SIDE_RECOVERED: usize = 0x198;
pub const SIDE_MODIFIERS: usize = 0x1d8;
pub const SIDE_COUNTRIES: usize = 0x1f0;
pub const SIDE_LEADER: usize = 0x208;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pool {
    pub serialized: bool,
    pub allow_zero: bool,
    pub equipment: Vec<Equipment>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModifierHours {
    pub modifier: i32,
    pub hours: i32,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SideData {
    pub manpower: i32,
    pub air_factor_raw: i64,
    pub equipment: Pool,
    pub captured_by_enemy: Pool,
    pub recovered: Pool,
    pub modifiers: Vec<ModifierHours>,
    pub countries: Vec<u32>,
    pub leader: u64,
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Battle {
    pub ref_count: i32,
    pub date: i32,
    pub province: u32,
    pub flags: [u8; 4],
    pub attacker: SideData,
    pub defender: SideData,
}

fn read_pool(
    memory: &impl Memory,
    base: usize,
    bytes: &[u8],
    offset: usize,
) -> Result<Pool, &'static str> {
    if word(bytes, offset) != module(base, pool::VTABLE_RVA)? as u64 {
        return Err("combat log battle pool vtable mismatch");
    }
    let mut output = Pool {
        serialized: false,
        allow_zero: bytes[offset + pool::ALLOW_ZERO] != 0,
        equipment: Vec::new(),
    };
    let allocations = array(bytes, offset + pool::ALLOCATIONS, 24)?;
    for ordinal in 0..allocations.count {
        let pointer = allocations
            .element(ordinal, 24)
            .ok_or("combat log allocation address overflow")?;
        if field::<i64>(memory, pointer, 16)? != 0 {
            output.serialized = true;
            break;
        }
    }
    if !output.serialized {
        return Ok(output);
    }
    let entries = array(bytes, offset + pool::ITEMS, equipment_entry::STRIDE)?;
    for ordinal in 0..entries.count {
        let entry = equipment_entry::read(
            memory,
            base,
            entries
                .element(ordinal, equipment_entry::STRIDE)
                .ok_or("combat log battle equipment address overflow")?,
            ordinal,
        )?;
        push(
            &mut output.equipment,
            Equipment {
                variant: entry.variant_handle,
                amount_raw: entry.amount_raw,
            },
        )?;
    }
    Ok(output)
}

fn read_side(memory: &impl Memory, base: usize, bytes: &[u8]) -> Result<SideData, &'static str> {
    if bytes.len() != SIDE_SIZE || word(bytes, 0) != module(base, SIDE_VTABLE_RVA)? as u64 {
        return Err("combat log battle side vtable mismatch");
    }
    let mut output = SideData {
        manpower: int(bytes, SIDE_MANPOWER),
        air_factor_raw: word(bytes, SIDE_AIR_FACTOR) as i64,
        equipment: read_pool(memory, base, bytes, SIDE_EQUIPMENT)?,
        captured_by_enemy: read_pool(memory, base, bytes, SIDE_CAPTURED_BY_ENEMY)?,
        recovered: read_pool(memory, base, bytes, SIDE_RECOVERED)?,
        leader: word(bytes, SIDE_LEADER),
        ..Default::default()
    };
    let modifiers = array(bytes, SIDE_MODIFIERS, 16)?;
    for ordinal in 0..modifiers.count {
        let pointer = modifiers
            .element(ordinal, 16)
            .ok_or("combat log modifier address overflow")?;
        let entry = header::<16>(memory, base, pointer, MODIFIER_HOURS_VTABLE_RVA)?;
        push(
            &mut output.modifiers,
            ModifierHours {
                modifier: int(&entry, 8),
                hours: int(&entry, 12),
            },
        )?;
    }
    let countries = array(bytes, SIDE_COUNTRIES, 4)?;
    for ordinal in 0..countries.count {
        let pointer = countries
            .element(ordinal, 4)
            .ok_or("combat log country address overflow")?;
        push(&mut output.countries, field(memory, pointer, 0)?)?;
    }
    Ok(output)
}

pub struct BattlePool {
    records: Array,
    counts: Array,
    used: Array,
}
impl BattlePool {
    pub fn read(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let records = Array::read(memory, module(base, BATTLE_POOL_RVA)?, BATTLE_SIZE)
            .ok_or("combat log battle array invalid")?;
        let counts = Array::read(memory, module(base, BATTLE_COUNTS_RVA)?, 2)
            .ok_or("combat log battle reference-count array invalid")?;
        let used = Array::read(memory, module(base, BATTLE_USED_RVA)?, 1)
            .ok_or("combat log battle occupancy array invalid")?;
        if records.count != counts.count || records.count != used.count {
            return Err("combat log battle arrays have different lengths");
        }
        Ok(Self {
            records,
            counts,
            used,
        })
    }
    pub fn read_record(
        &self,
        memory: &impl Memory,
        base: usize,
        id: i32,
    ) -> Result<Option<Battle>, &'static str> {
        let id = u32::try_from(id).map_err(|_| "combat log battle index negative")?;
        let pointer = self
            .records
            .element(id, BATTLE_SIZE)
            .ok_or("combat log battle index out of range")?;
        let count = field::<i16>(
            memory,
            self.counts
                .element(id, 2)
                .ok_or("combat log count index invalid")?,
            0,
        )?;
        let used = field::<u8>(
            memory,
            self.used
                .element(id, 1)
                .ok_or("combat log occupancy index invalid")?,
            0,
        )?;
        if (count > 0) != (used != 0) || count < 0 {
            return Err("combat log battle occupancy differs from reference count");
        }
        if count == 0 {
            return Ok(None);
        }
        let bytes = header::<BATTLE_SIZE>(memory, base, pointer, BATTLE_VTABLE_RVA)?;
        Ok(Some(Battle {
            ref_count: i32::from(count),
            date: int(&bytes, BATTLE_DATE),
            province: int(&bytes, BATTLE_PROVINCE) as u32,
            flags: bytes[BATTLE_FLAGS..BATTLE_FLAGS + 4].try_into().unwrap(),
            attacker: read_side(
                memory,
                base,
                &bytes[BATTLE_ATTACKER..BATTLE_ATTACKER + SIDE_SIZE],
            )?,
            defender: read_side(
                memory,
                base,
                &bytes[BATTLE_DEFENDER..BATTLE_DEFENDER + SIDE_SIZE],
            )?,
        }))
    }
}

pub const MODIFIER_HOURS_VTABLE_RVA: Rva = Rva(0x029d_9f90);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    const BASE: usize = 0x140000000;
    #[derive(Default)]
    struct Fixture {
        bytes: BTreeMap<usize, u8>,
        next: usize,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (i, byte) in destination.iter_mut().enumerate() {
                let Some(value) = address.checked_add(i).and_then(|a| self.bytes.get(&a)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl Fixture {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.bytes
                .extend(bytes.iter().enumerate().map(|(i, b)| (address + i, *b)));
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        fn i32(&mut self, address: usize, value: i32) {
            self.put(address, &value.to_le_bytes());
        }
        fn alloc(&mut self, size: usize) -> usize {
            let pointer = 0x10000 + self.next;
            self.next += size + 32;
            self.put(pointer, &vec![0; size]);
            pointer
        }
        fn array(&mut self, address: usize, pointer: usize, count: u32) {
            self.u64(address, pointer as u64);
            self.put(address + 8, &count.to_le_bytes());
            self.put(address + 12, &count.to_le_bytes());
        }
        fn pointer_array(&mut self, address: usize, pointer: usize) {
            let list = self.alloc(8);
            self.u64(list, pointer as u64);
            self.array(address, list, 1);
        }
        fn group(&mut self) -> usize {
            let p = self.alloc(group::SIZE);
            self.u64(p, (BASE + group::VTABLE_RVA.0) as u64);
            p
        }
        fn group_bytes(&self, pointer: usize) -> [u8; group::SIZE] {
            let mut bytes = [0; group::SIZE];
            assert!(self.copy(pointer, &mut bytes));
            bytes
        }
        fn month(&mut self, reason: i32, date: i32) -> usize {
            let p = self.alloc(loss::SIZE);
            self.u64(p, (BASE + loss::VTABLE_RVA.0) as u64);
            self.i32(p + loss::REASON, reason);
            self.i32(p + loss::DATE, date);
            self.u64(p + loss::POOL, (BASE + pool::VTABLE_RVA.0) as u64);
            p
        }
    }
    #[test]
    fn event_settlement_reads_only_requested_counts_and_occupancy() {
        use crate::combat_log_events::settled_counts;
        let mut f = Fixture::default();
        let counts = 0x9000;
        let used = 0xa000;
        f.array(BASE + BATTLE_COUNTS_RVA.0, counts, 5000);
        f.array(BASE + BATTLE_USED_RVA.0, used, 5000);
        f.put(counts + 4999 * 2, &2_i16.to_le_bytes());
        f.put(used + 4999, &[1]);
        let mut got = Vec::new();
        settled_counts(&f, BASE, &[4999], &mut |id, count| got.push((id, count))).unwrap();
        assert_eq!(got, [(4999, 2)]);
        f.put(counts + 4999 * 2, &0_i16.to_le_bytes());
        f.put(used + 4999, &[0]);
        got.clear();
        settled_counts(&f, BASE, &[4999], &mut |id, count| got.push((id, count))).unwrap();
        assert_eq!(got, [(4999, 0)]);
        f.put(used + 4999, &[1]);
        assert!(settled_counts(&f, BASE, &[4999], &mut |_, _| {}).is_err());
        assert!(settled_counts(&f, BASE, &[5000], &mut |_, _| {}).is_err());
        settled_counts(&Fixture::default(), BASE, &[], &mut |_, _| panic!()).unwrap();
    }
    #[test]
    fn event_reader_demands_only_the_changed_last_month_and_keeps_empty_entries() {
        use crate::combat_log_events as events;
        let mut f = Fixture::default();
        let p = f.group();
        let m = f.month(0, 60_000_000);
        let pointers = f.alloc(16);
        f.u64(pointers, 1);
        f.u64(pointers + 8, m as u64);
        f.array(p + group::ENEMY_LOSSES, pointers, 2);
        let mut facts = [events::Fact::default(); 1];
        let mut count = 0;
        events::last_month(&f, BASE, p, ENEMY_KIND, &mut |fact| {
            facts[count] = fact;
            count += 1;
        })
        .unwrap();
        assert_eq!(count, 1);
        assert_eq!(facts[0].kind, events::MONTH);
        assert_eq!(facts[0].item, m as u64);
        assert_eq!(facts[0].values, [5, 0, 60_000_000, 0, 0]);
    }
    #[test]
    fn template_event_reads_the_reached_object_and_matches_wrapping_native_total() {
        use crate::combat_log_events as events;
        let mut f = Fixture::default();
        let t = f.alloc(TEMPLATE_SIZE);
        f.u64(t, (BASE + TEMPLATE_VTABLE_RVA.0) as u64);
        f.u64(t + TEMPLATE_REFERENCE, 0xabc0_1234_0000_0036);
        f.i32(t + TEMPLATE_WINS, -17);
        f.i32(t + TEMPLATE_TOTAL, i32::MAX);
        let mut observed = None;
        events::template(&f, BASE, 1, t, true, &mut |fact| observed = Some(fact)).unwrap();
        let fact = observed.unwrap();
        assert_eq!(fact.root, 1);
        assert_eq!(fact.values[0], 0xabc0_1234_0000_0036);
        assert_eq!(fact.values[2] as i32, -17);
        assert_eq!(fact.values[3] as i32, i32::MIN);
    }
    #[test]
    fn captures_empty_months_all_manpower_counters_templates_and_reference_order() {
        let mut f = Fixture::default();
        let p = f.group();
        let m = f.month(0, 60_000_000);
        f.put(m + loss::POOL + pool::ALLOW_ZERO, &[1]);
        f.pointer_array(p + group::ENEMY_LOSSES, m);
        let own = f.month(2, 60_000_001);
        f.pointer_array(p + group::REASONS + 2 * group::REASON_STRIDE, own);
        let mp = f.alloc(MANPOWER_SIZE);
        f.u64(mp, (BASE + MANPOWER_VTABLE_RVA.0) as u64);
        f.i32(mp + loss::REASON, 0);
        f.i32(mp + loss::DATE, 60_000_002);
        for (i, v) in [i32::MIN, 0, i32::MAX].iter().enumerate() {
            f.i32(mp + MANPOWER_LOSSES + i * 4, *v);
        }
        f.pointer_array(p + MANPOWER, mp);
        let t = f.alloc(TEMPLATE_SIZE);
        f.u64(t, (BASE + TEMPLATE_VTABLE_RVA.0) as u64);
        f.u64(t + TEMPLATE_REFERENCE, 7 << 32 | 52);
        f.i32(t + TEMPLATE_WINS, -1);
        f.i32(t + TEMPLATE_TOTAL, 5);
        f.i32(t + TEMPLATE_DATE, 60_000_003);
        f.pointer_array(p + TEMPLATES, t);
        let r = f.alloc(16);
        f.i32(r, 0);
        f.put(r + 4, &[1]);
        f.i32(r + 8, 0);
        f.array(p + REFERENCES, r, 2);
        let value = read_group(&f, BASE, &f.group_bytes(p)).unwrap();
        assert_eq!(value.months.len(), 2);
        assert_eq!(value.months[0].kind, 2);
        assert_eq!(value.months[1].kind, ENEMY_KIND);
        assert!(value.months[1].allow_zero);
        assert!(value.months[1].enemy.is_empty());
        assert_eq!(value.manpower[0].losses, [i32::MIN, 0, i32::MAX]);
        assert_eq!(value.templates[0].wins, -1);
        assert_eq!(
            value.battles,
            [
                BattleReference {
                    id: 0,
                    attacker: true
                },
                BattleReference {
                    id: 0,
                    attacker: false
                }
            ]
        );
        f.bytes.remove(&(mp + MANPOWER_LOSSES));
        assert_eq!(
            read_group(&f, BASE, &f.group_bytes(p)),
            Err("combat log history object unreadable")
        );
    }
    #[test]
    fn omitted_side_pool_does_not_demand_unserialized_variant_payload() {
        let mut f = Fixture::default();
        let p = f.alloc(64);
        f.u64(p, (BASE + pool::VTABLE_RVA.0) as u64);
        f.array(p + pool::ITEMS, 0xdead_0000, 1);
        let mut bytes = [0; 64];
        assert!(f.copy(p, &mut bytes));
        assert_eq!(read_pool(&f, BASE, &bytes, 0).unwrap(), Pool::default());
        let allocation = f.alloc(24);
        f.u64(allocation + 16, 1);
        f.array(p + pool::ALLOCATIONS, allocation, 1);
        assert!(f.copy(p, &mut bytes));
        assert_eq!(
            read_pool(&f, BASE, &bytes, 0),
            Err("equipment entry unreadable")
        );
    }

    #[test]
    fn enemy_pool_preserves_full_handles_repeated_occurrences_and_signed_amounts() {
        let mut f = Fixture::default();
        let p = f.group();
        let month = f.month(0, 60_000_000);
        f.pointer_array(p + group::ENEMY_LOSSES, month);
        let variant = f.alloc(16);
        f.u64(
            variant,
            (BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0) as u64,
        );
        let handle =
            u64::from(u32::MAX) << 32 | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE);
        f.u64(variant + 8, handle);
        let entries = f.alloc(3 * equipment_entry::STRIDE);
        f.array(month + loss::POOL + pool::ITEMS, entries, 3);
        for (i, amount) in [i64::MIN, 0, i64::MAX].into_iter().enumerate() {
            f.u64(entries + i * equipment_entry::STRIDE, variant as u64);
            f.u64(
                entries + i * equipment_entry::STRIDE + equipment_entry::AMOUNT,
                amount as u64,
            );
        }
        let value = read_group(&f, BASE, &f.group_bytes(p)).unwrap();
        assert_eq!(
            value.months[0].enemy,
            [i64::MIN, 0, i64::MAX].map(|amount_raw| Equipment {
                variant: handle,
                amount_raw
            })
        );
        f.u64(variant, 0);
        assert!(read_group(&f, BASE, &f.group_bytes(p)).is_err());
    }

    #[test]
    fn empty_group_history_has_no_population_limit() {
        let mut f = Fixture::default();
        for _ in 0..5000 {
            let p = f.group();
            assert_eq!(
                read_group(&f, BASE, &f.group_bytes(p)).unwrap(),
                History::default()
            );
        }
    }
    #[test]
    fn battle_pool_reads_only_the_requested_live_index_and_allows_reuse() {
        let mut f = Fixture::default();
        let records = f.alloc(2 * BATTLE_SIZE);
        let counts = f.alloc(4);
        let used = f.alloc(2);
        f.array(BASE + BATTLE_POOL_RVA.0, records, 2);
        f.array(BASE + BATTLE_COUNTS_RVA.0, counts, 2);
        f.array(BASE + BATTLE_USED_RVA.0, used, 2);
        for i in 0..BATTLE_SIZE {
            f.bytes.remove(&(records + i));
        }
        f.put(counts + 2, &2_i16.to_le_bytes());
        f.put(used + 1, &[1]);
        let p = records + BATTLE_SIZE;
        f.u64(p, (BASE + BATTLE_VTABLE_RVA.0) as u64);
        f.i32(p + BATTLE_DATE, 60_000_000);
        f.i32(p + BATTLE_PROVINCE, 9);
        f.put(p + BATTLE_FLAGS, &[1, 0, 1, 0]);
        for offset in [BATTLE_ATTACKER, BATTLE_DEFENDER] {
            f.u64(p + offset, (BASE + SIDE_VTABLE_RVA.0) as u64);
            for pool_offset in [SIDE_EQUIPMENT, SIDE_CAPTURED_BY_ENEMY, SIDE_RECOVERED] {
                f.u64(p + offset + pool_offset, (BASE + pool::VTABLE_RVA.0) as u64);
            }
        }
        let pool = BattlePool::read(&f, BASE).unwrap();
        let b = pool.read_record(&f, BASE, 1).unwrap().unwrap();
        assert_eq!(b.ref_count, 2);
        assert_eq!(b.province, 9);
        assert!(!b.attacker.equipment.serialized);
        f.put(counts + 2, &0_i16.to_le_bytes());
        f.put(used + 1, &[0]);
        assert_eq!(pool.read_record(&f, BASE, 1).unwrap(), None);
        f.put(counts + 2, &1_i16.to_le_bytes());
        f.put(used + 1, &[1]);
        f.i32(p + BATTLE_PROVINCE, 12);
        assert_eq!(pool.read_record(&f, BASE, 1).unwrap().unwrap().province, 12);
        f.put(used + 1, &[0]);
        assert!(pool.read_record(&f, BASE, 1).is_err());
    }
}
