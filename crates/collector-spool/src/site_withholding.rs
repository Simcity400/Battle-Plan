use collector_memory::read_accounting;
use replay_data::rows::TableId;
use replay_frame::frame::{FrameInput, TableView};
use std::io;

pub(crate) fn apply(source: &mut FrameInput) -> io::Result<()> {
    for table in TableId::ALL {
        if let Some(key) = read_accounting::withheld(table) {
            source.tables[table.index()] = TableView::Unavailable {
                reason: engine_allocation::string(
                    engine_sites::located()
                        .and_then(|located| located.message(key))
                        .unwrap_or("a game site this table reads is withheld on this build"),
                )?,
            }
            .into();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_stability::country_stability::{Capacities, Captured, TABLES};
    use engine_native::{
        Memory, cattache_relation, ccountry, cdiplomacy_status, cgame_state, cidea,
        cideology_group, cpdx_array::Array, cpolitical_party, cpolitical_status, crelation,
        cstatic_modifier, modifier_definition, msvc_string,
    };
    use engine_stability::{cmodifier, country_stability as native, cworld_threat};
    use replay_data::rows::{CountryStabilityLineRow, TableRows};
    use replay_frame::frame::{Metrics, TableInput};
    use std::collections::BTreeMap;

    #[test]
    fn missing_attache_vtable_site_does_not_withhold_country_stability_inputs() {
        struct World {
            bytes: BTreeMap<usize, u8>,
        }
        impl World {
            fn put(&mut self, at: usize, bytes: &[u8]) {
                self.bytes
                    .extend(bytes.iter().enumerate().map(|(i, &b)| (at + i, b)));
            }
            fn word(&mut self, at: usize, value: u64) {
                self.put(at, &value.to_le_bytes());
            }
            fn u32(&mut self, at: usize, value: u32) {
                self.put(at, &value.to_le_bytes());
            }
            fn array(&mut self, at: usize, data: usize, count: u32) {
                self.word(at, data as u64);
                self.word(at + 8, u64::from(count) | (u64::from(count) << 32));
            }
            fn string(&mut self, at: usize, text: &[u8]) {
                let mut storage = [0_u8; 16];
                storage[..text.len()].copy_from_slice(text);
                self.put(at, &storage);
                self.word(at + msvc_string::LENGTH, text.len() as u64);
                self.word(at + msvc_string::CAPACITY, 15);
            }
        }
        impl Memory for World {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = self.bytes.get(&(at + i)).copied().unwrap_or(0);
                }
                true
            }
        }

        const BASE: usize = 0x1_4000_0000;
        const STATE: usize = 0x1000;
        const COUNTRY: usize = 0x10000;
        const STATUS: usize = 0x20000;
        const PARTY: usize = 0x21000;
        const IDEA: usize = 0x22000;
        const GROUP: usize = 0x23000;
        const DIPLOMACY: usize = 0x24000;
        const RELATIONS: usize = 0x24400;
        const ATTACHE_REL: usize = 0x24800;
        const STATS: usize = 0x25000;
        const RING: usize = 0x26000;
        const WORLD: usize = 0x27000;
        const DATABASE: usize = 0x28000;
        const ATTACHE_STATIC: usize = 0x29000;
        const ATTACHE_ENTRIES: usize = 0x29400;
        const SUNK: usize = 0x29800;
        const SUNK_ENTRIES: usize = 0x29c00;
        const CHILD_ATTACHE: usize = 0x2a000;
        const CHILD_SUNK: usize = 0x2a200;
        const CHILDREN: usize = 0x2a400;
        const CHILD_ATTACHE_ENTRIES: usize = 0x2a800;
        const CHILD_SUNK_ENTRIES: usize = 0x2aa00;
        const TABLE: usize = 0x2b000;
        const STABILITY_KEY: u32 = 5;

        let mut memory = World {
            bytes: BTreeMap::new(),
        };
        memory.array(STATE + cgame_state::COUNTRIES, 0x3000, 2);
        memory.array(STATE + cgame_state::COUNTRY_LINKS, 0x4000, 2);
        memory.word(0x3008, COUNTRY as u64);
        memory.u32(0x4004, 1);
        memory.word(COUNTRY, (BASE + ccountry::VTABLE_RVA.0) as u64);
        memory.u32(COUNTRY + ccountry::ID, 1);
        memory.word(COUNTRY + ccountry::STABILITY, 50_000);
        memory.put(COUNTRY + native::PRIDE_OF_THE_FLEET_SUNK_ACTIVE, &[1]);
        memory.word(COUNTRY + ccountry::DIPLOMACY_STATUS, DIPLOMACY as u64);
        memory.word(DIPLOMACY, (BASE + cdiplomacy_status::VTABLE_RVA.0) as u64);
        memory.word(DIPLOMACY + cdiplomacy_status::COUNTRY, COUNTRY as u64);
        memory.word(COUNTRY + ccountry::POLITICAL_STATUS, STATUS as u64);
        memory.word(STATUS, (BASE + cpolitical_status::VTABLE_RVA.0) as u64);
        memory.word(STATUS + cpolitical_status::RULING_PARTY, PARTY as u64);
        memory.word(PARTY, (BASE + cpolitical_party::VTABLE_RVA.0) as u64);
        memory.word(PARTY + cpolitical_party::IDEOLOGY_GROUP, GROUP as u64);
        memory.word(GROUP, (BASE + cideology_group::VTABLE_RVA.0) as u64);
        memory.string(
            GROUP + native::IDEOLOGY_GROUP_MODIFIER + cmodifier::DISPLAY_NAME,
            b"",
        );
        memory.array(STATUS + cpolitical_status::IDEAS, 0x5000, 1);
        memory.word(0x5000, IDEA as u64);
        memory.word(IDEA, (BASE + cidea::VTABLE_RVA.0) as u64);
        memory.word(COUNTRY + native::AIR_STATISTICS, STATS as u64);
        memory.word(STATS + 0x10, RING as u64);
        memory.word(STATE + cgame_state::WORLD_THREAT, WORLD as u64);
        memory.word(WORLD, (BASE + cworld_threat::VTABLE_RVA.0) as u64);
        memory.array(BASE + modifier_definition::TABLE_RVA.0, TABLE, 8);
        memory.u32(
            TABLE
                + STABILITY_KEY as usize * modifier_definition::STRIDE
                + modifier_definition::FLAGS,
            3,
        );
        memory.word(BASE + native::STATIC_DATABASE_RVA.0, DATABASE as u64);
        memory.word(DATABASE + 0x90, ATTACHE_STATIC as u64);
        memory.word(
            ATTACHE_STATIC,
            (BASE + cstatic_modifier::VTABLE_RVA.0) as u64,
        );
        memory.u32(ATTACHE_ENTRIES, STABILITY_KEY);
        memory.word(ATTACHE_ENTRIES + 8, 4_000_u64);
        memory.array(ATTACHE_STATIC + cmodifier::ACCUMULATOR, ATTACHE_ENTRIES, 1);
        memory.string(ATTACHE_STATIC + cmodifier::DISPLAY_NAME, b"Attache");
        memory.string(ATTACHE_STATIC + native::STATIC_NAME, b"attache_sent");
        memory.word(DATABASE + 0xc0, SUNK as u64);
        memory.word(SUNK, (BASE + cstatic_modifier::VTABLE_RVA.0) as u64);
        memory.u32(SUNK_ENTRIES, STABILITY_KEY);
        memory.word(SUNK_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(SUNK + cmodifier::ACCUMULATOR, SUNK_ENTRIES, 1);
        memory.string(SUNK + cmodifier::DISPLAY_NAME, b"Pride sunk");
        memory.string(SUNK + native::STATIC_NAME, b"pride_sunk");
        memory.array(COUNTRY + native::MODIFIER_CHILDREN, CHILDREN, 2);
        memory.word(CHILDREN, CHILD_ATTACHE as u64);
        memory.word(CHILDREN + 8, CHILD_SUNK as u64);
        memory.word(CHILD_ATTACHE, (BASE + cmodifier::VTABLE_RVA.0) as u64);
        memory.u32(CHILD_ATTACHE_ENTRIES, STABILITY_KEY);
        memory.word(CHILD_ATTACHE_ENTRIES + 8, 4_000_u64);
        memory.array(
            CHILD_ATTACHE + cmodifier::ACCUMULATOR,
            CHILD_ATTACHE_ENTRIES,
            1,
        );
        memory.string(CHILD_ATTACHE + cmodifier::DISPLAY_NAME, b"Attache");
        memory.word(CHILD_SUNK, (BASE + cmodifier::VTABLE_RVA.0) as u64);
        memory.u32(CHILD_SUNK_ENTRIES, STABILITY_KEY);
        memory.word(CHILD_SUNK_ENTRIES + 8, (-3_000_i64) as u64);
        memory.array(CHILD_SUNK + cmodifier::ACCUMULATOR, CHILD_SUNK_ENTRIES, 1);
        memory.string(CHILD_SUNK + cmodifier::DISPLAY_NAME, b"Pride sunk");

        let mut keys = [None; native::TRACKED];
        keys[0] = Some(STABILITY_KEY);
        let mut captured = Captured::with_capacities(&Capacities::for_test(2, 4, 64));
        captured.set_keys(native::Keys {
            keys,
            table: Array {
                data: TABLE,
                count: 8,
            },
        });

        for (hour, with_attache) in [(0, false), (1, true)] {
            if with_attache {
                memory.array(DIPLOMACY + cdiplomacy_status::OWNED_RELATIONS, RELATIONS, 1);
                memory.word(RELATIONS, ATTACHE_REL as u64);
                memory.word(ATTACHE_REL, (BASE + cattache_relation::VTABLE_RVA.0) as u64);
                memory.u32(ATTACHE_REL + crelation::TYPE_ID, cattache_relation::TYPE);
                memory.u32(ATTACHE_REL + crelation::FIRST_COUNTRY, 1);
                memory.u32(ATTACHE_REL + crelation::SECOND_COUNTRY, 2);
            }
            engine_sites::with_synthetic_withheld(cattache_relation::VTABLE_RVA, || {
                captured.capture(&memory, BASE, STATE, hour, true, None);
            });
            let mut frame = FrameInput {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|index| {
                    TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
                }),
            };
            let views = captured.tables();
            for (table, view) in TABLES.into_iter().zip(views) {
                frame.tables[table.index()] = view.into();
            }
            apply(&mut frame).unwrap();
            assert_eq!(
                read_accounting::withheld(TableId::CountryStabilityInputs),
                None
            );
            let TableInput::Sampled(TableView::Complete(TableRows::CountryStabilityInputs(inputs))) =
                &frame.tables[TableId::CountryStabilityInputs.index()]
            else {
                panic!(
                    "expected CountryStabilityInputs to remain complete, got {:?}",
                    frame.tables[TableId::CountryStabilityInputs.index()]
                );
            };
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0].stability_raw, 50_000);
            let TableInput::Sampled(TableView::Complete(TableRows::CountryStabilityLines(lines))) =
                &frame.tables[TableId::CountryStabilityLines.index()]
            else {
                panic!("expected CountryStabilityLines to remain complete");
            };
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0].kind, CountryStabilityLineRow::UNPAIRED);
            assert_eq!(lines[0].text, b"Attache");
            assert_eq!(lines[1].kind, CountryStabilityLineRow::STATIC);
            assert_eq!(lines[1].identity, b"pride_sunk");
        }
    }
}
