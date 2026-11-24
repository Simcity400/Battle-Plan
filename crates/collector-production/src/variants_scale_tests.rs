
use super::*;
use engine_equipment::cequipment_upgrades_instance;
use engine_native_parts::{cequipment_module, cequipment_type, cequipment_upgrade};
use engine_native_politics::{equipment_module_entry, equipment_upgrade_entry};
use replay_data::rows::TableSchema;

const VARIANT: usize = (cequipment_variant::DESIGN_TEAM_TRAITS + 4).next_multiple_of(16);
const DEFINITION: usize = (cequipment_module::MULTIPLY_STATS + 16).next_multiple_of(16);
const TYPE_DEFINITION: usize = cequipment_type::ARCHETYPE + 8;
const TYPES: usize = 200;
const MODULES: usize = 300;
const UPGRADES: usize = 100;

fn explicit_entity(index: usize) -> String {
    if index.is_multiple_of(7) {
        format!("designer_explicit_entity_{index}")
    } else {
        String::new()
    }
}

fn icon(index: usize) -> (u8, u32, String) {
    if index.is_multiple_of(11) {
        (1, 0, format!("GFX_designer_icon_{index}"))
    } else if index.is_multiple_of(3) {
        (1, 1, ".".into())
    } else {
        (0, 1, String::new())
    }
}

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

pub(super) struct Space {
    pub bytes: Vec<u8>,
    pub base: usize,
    pub game_state: usize,
}

impl Space {
    pub fn memory(&self) -> engine_native_memory::memory::SliceMemory<'_> {
        engine_native_memory::memory::SliceMemory {
            base: self.bytes.as_ptr() as usize,
            bytes: &self.bytes,
        }
    }
}

struct Writer {
    bytes: Vec<u8>,
    used: usize,
}

impl Writer {
    fn start(&self) -> usize {
        self.bytes.as_ptr() as usize
    }

    fn alloc(&mut self, len: usize) -> usize {
        let at = self.used.next_multiple_of(16);
        self.used = at + len;
        assert!(self.used <= self.bytes.len(), "space size estimate");
        self.start() + at
    }

    fn put(&mut self, address: usize, value: &[u8]) {
        let at = address - self.start();
        self.bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn u64(&mut self, address: usize, value: u64) {
        self.put(address, &value.to_le_bytes());
    }

    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }

    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data as u64);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }

    fn string(&mut self, header: usize, text: &[u8]) {
        let len = text.len();
        if len < 16 {
            let mut inline = [0u8; 16];
            inline[..len].copy_from_slice(text);
            self.put(header, &inline);
            self.u64(header + 24, msvc_string::INLINE_CAPACITY);
        } else {
            let heap = self.alloc(len + 1);
            self.put(heap, text);
            self.put(heap + len, &[0]);
            self.u64(header, heap as u64);
            self.u64(header + 24, len as u64);
        }
        self.u64(header + 16, len as u64);
    }
}

pub(super) fn space(countries: usize, per_country: usize) -> Space {
    let reference_extent = engine_native_parts::creference_registry::LOW_ROOTS_RVA.0
        + engine_native_parts::creference_registry::LOW_ROOT_COUNT as usize * 8
        - cgame_state::SINGLETON_RVA.0;
    let variants = countries * per_country;
    let plans: Vec<(usize, usize)> = (0..variants)
        .map(|index| {
            let seed = mix(index as u64);
            let modules = if index.is_multiple_of(97) {
                40
            } else {
                seed as usize % 9
            };
            (modules, (seed >> 8) as usize % 5)
        })
        .collect();
    let size = reference_extent
        + 0x800
        + (TYPES + MODULES + UPGRADES) * (DEFINITION + 48)
        + TYPES * TYPE_DEFINITION
        + variants * (8 + VARIANT + 64 + 48)
        + plans
            .iter()
            .map(|(modules, upgrades)| (modules + upgrades) * 16 + 32)
            .sum::<usize>();
    let mut writer = Writer {
        bytes: vec![0; size],
        used: 0,
    };
    let singleton = writer.alloc(reference_extent);
    let base = singleton - cgame_state::SINGLETON_RVA.0;
    let vtable = |rva: engine_sites::Rva| (base + rva.0) as u64;
    let game_state = writer.alloc(0x720);
    writer.u64(singleton, game_state as u64);
    let definitions = |writer: &mut Writer, count: usize, rva: engine_sites::Rva, kind: &str| {
        (0..count)
            .map(|index| {
                let at = writer.alloc(if kind == "type" {
                    TYPE_DEFINITION
                } else {
                    DEFINITION
                });
                writer.u64(at, vtable(rva));
                writer.u32(at + 8, 0x4000 + (index as u32) * 3 + kind.len() as u32);
                let name = if index.is_multiple_of(3) {
                    format!("{kind}_{index}")
                } else {
                    format!("{kind}_long_definition_name_{index}")
                };
                writer.string(at + 0x18, name.as_bytes());
                at
            })
            .collect::<Vec<_>>()
    };
    let types = definitions(&mut writer, TYPES, cequipment_type::VTABLE_RVA, "type");
    for &equipment in &types {
        writer.u64(equipment + cequipment_type::ARCHETYPE, equipment as u64);
    }
    let modules = definitions(
        &mut writer,
        MODULES,
        cequipment_module::VTABLE_RVA,
        "module",
    );
    for (index, &module) in modules.iter().enumerate() {
        writer.put(
            module + cequipment_module::ACTIVE,
            &[u8::from(!index.is_multiple_of(11))],
        );
    }
    let upgrades = definitions(
        &mut writer,
        UPGRADES,
        cequipment_upgrade::VTABLE_RVA,
        "upgrade",
    );
    let pointers = writer.alloc(variants * 8);
    writer.array(
        game_state + cgame_state::EQUIPMENT_VARIANTS,
        pointers,
        variants as u32,
    );
    let objects = writer.alloc(variants * VARIANT);
    let step = (7_919..)
        .find(|step| gcd(*step, variants.max(1)) == 1)
        .unwrap();
    for (index, &(module_count, upgrade_count)) in plans.iter().enumerate() {
        let seed = mix(index as u64 ^ 0x5eed);
        let variant = objects + (index * step % variants) * VARIANT;
        writer.u64(pointers + index * 8, variant as u64);
        writer.u64(variant, vtable(engine_native_parts::equipment_variant_identity::VTABLE_RVA));
        writer.u64(
            variant + cequipment_variant::TYPE,
            (index as u64 + 1) << 32 | u64::from(cequipment_variant::REFERENCE_TYPE),
        );
        let equipment = types[seed as usize % TYPES];
        let token = u32::from_le_bytes(
            writer.bytes[equipment + 8 - writer.start()..equipment + 12 - writer.start()]
                .try_into()
                .unwrap(),
        );
        writer.u32(variant + cequipment_variant::EQUIPMENT_TOKEN, token);
        let country = (index / per_country.max(1)) as u32 + 1;
        writer.u32(variant + cequipment_variant::CREATOR_LINK, country);
        writer.u64(
            variant + cequipment_variant::INDUSTRIAL_ORGANISATION,
            country as u64,
        );
        writer.u32(
            variant + cequipment_variant::ORGANISATION_TRAIT_COUNT,
            (index % 9) as u32,
        );
        writer.array(variant + cequipment_variant::ORGANISATION_BONUSES, 0, 0);
        writer.u32(
            variant + cequipment_variant::ORIGIN_LINK,
            country ^ (seed as u32 & 1),
        );
        writer.put(
            variant + cequipment_variant::APPEND_VARIANT_NAME,
            &[(seed >> 8) as u8 & 1],
        );
        let display = format!(
            "Design {index}{}",
            if seed.is_multiple_of(5) {
                " Mk. II long"
            } else {
                ""
            }
        );
        writer.string(
            variant + cequipment_variant::DISPLAY_NAME,
            display.as_bytes(),
        );
        writer.string(
            variant + cequipment_variant::VARIANT_NAME,
            format!("v{}", seed % 100).as_bytes(),
        );
        writer.string(
            variant + cequipment_variant::EXPLICIT_ENTITY,
            explicit_entity(index).as_bytes(),
        );
        let (active, kind, icon) = icon(index);
        writer.put(
            variant + cequipment_variant::ICON_OVERRIDE_ACTIVE,
            &[active],
        );
        writer.u32(variant + cequipment_variant::ICON_OVERRIDE_KIND, kind);
        writer.string(
            variant + cequipment_variant::ICON_OVERRIDE_TOKEN,
            icon.as_bytes(),
        );
        writer.u64(variant + cequipment_variant::DEFINITION, equipment as u64);
        writer.u64(variant + cequipment_variant::CLASSIFIER_FLAGS, mix(seed));
        writer.u32(
            variant + cequipment_variant::MISSION_MASK,
            (seed >> 32) as u32,
        );
        writer.put(
            variant + cequipment_variant::OBSOLETE,
            &[(seed >> 16) as u8 & 1],
        );
        for (slot, lane) in cequipment_variant::STAT_LANES.into_iter().enumerate() {
            writer.u64(variant + lane.offset, mix(seed ^ slot as u64));
        }
        let entries = writer.alloc(module_count * equipment_module_entry::STRIDE);
        writer.array(
            variant + cequipment_variant::MODULES + 8,
            entries,
            module_count as u32,
        );
        for ordinal in 0..module_count {
            let at = entries + ordinal * equipment_module_entry::STRIDE;
            let draw = mix(seed ^ ordinal as u64);
            writer.u32(at + equipment_module_entry::SLOT, draw as u32 % 12);
            writer.u64(
                at + equipment_module_entry::DEFINITION,
                modules[(draw >> 16) as usize % MODULES] as u64,
            );
        }
        let instance = variant + cequipment_variant::UPGRADES;
        writer.u64(instance, vtable(cequipment_upgrades_instance::VTABLE_RVA));
        let entries = writer.alloc(upgrade_count * equipment_upgrade_entry::STRIDE);
        writer.array(instance + 8, entries, upgrade_count as u32);
        for ordinal in 0..upgrade_count {
            let at = entries + ordinal * equipment_upgrade_entry::STRIDE;
            let draw = mix(seed ^ 0xabc ^ ordinal as u64);
            writer.u64(
                at + equipment_upgrade_entry::DEFINITION,
                upgrades[draw as usize % UPGRADES] as u64,
            );
            writer.put(
                at + equipment_upgrade_entry::LEVEL_STORAGE,
                &[(draw >> 20) as u8],
            );
        }
    }
    Space {
        bytes: writer.bytes,
        base,
        game_state,
    }
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn capacities(variants: usize) -> Capacities {
    let mut capacities = Capacities::default();
    for (slot, rows) in [
        (3, variants),
        (4, variants * 8),
        (5, variants),
        (6, variants * 4),
    ] {
        capacities.rows[slot] = capacities.rows[slot].max(rows);
    }
    capacities.definition_bytes = capacities.definition_bytes.max(variants * 32);
    capacities
}

fn capture(memory: &impl Memory, space: &Space, variants: usize) -> ([TableView; 4], f64) {
    let slot = replay_data::cadence::refresh_slot(TableId::VariantDefinitions).unwrap() as i32;
    let mut registry = super::registry::Registry::new(variants);
    let mut captured = Captured::with_capacities(&capacities(variants));
    let mut family = captured.take_variants();
    let started = std::time::Instant::now();
    registry.acquire(memory, space.base, space.game_state);
    family.capture(memory, space.base, registry.sources(), slot, false);
    let elapsed = started.elapsed().as_secs_f64() * 1e3;
    captured.restore_variants(family);
    let [_, _, _, definitions, modules, stats, upgrades, ..] = captured.tables();
    ([definitions, modules, stats, upgrades], elapsed)
}

fn digest(views: &[TableView; 4]) -> u64 {
    fn rows<S: TableSchema>(
        text: &mut String,
        view: &TableView,
        fields: impl Fn(&S::Row) -> String,
    ) {
        match view {
            TableView::Complete(rows) => {
                for row in rows.rows::<S>().unwrap() {
                    text.push_str(&fields(row));
                }
            }
            other => text.push_str(&format!("{other:?}")),
        }
    }
    let mut text = String::new();
    rows::<tables::VariantDefinitions>(&mut text, &views[0], |r| {
        format!(
            "{} {} {} {} {} {} {} {} {:?} {:?};",
            r.id,
            r.equipment_token,
            r.creator_link,
            r.origin_link,
            r.append_variant_name,
            r.obsolete,
            r.classifier_flags,
            r.mission_mask,
            r.display_name,
            r.variant_name
        )
    });
    rows::<tables::VariantModules>(&mut text, &views[1], |r| {
        format!(
            "{} {} {} {} {};",
            r.variant, r.source_ordinal, r.slot, r.definition_token, r.definition_active
        )
    });
    rows::<tables::VariantStats>(&mut text, &views[2], |r| {
        format!("{} {:?};", r.variant, r.values)
    });
    rows::<tables::VariantUpgrades>(&mut text, &views[3], |r| {
        format!(
            "{} {} {} {};",
            r.variant, r.source_ordinal, r.definition_token, r.level
        )
    });
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn scaled_variant_rows_match_the_recorded_digests() {
    for (countries, golden) in [(2, 0xb6db_8a07_3fc3_fea7), (440, 0x2e8d_591a_edac_0413)] {
        let space = space(countries, 16);
        let (views, _) = capture(&space.memory(), &space, countries * 16);
        assert!(
            views
                .iter()
                .all(|view| matches!(view, TableView::Complete(_))),
            "{views:?}"
        );
        assert_eq!(digest(&views), golden, "{countries} countries");
        let TableView::Complete(definitions) = &views[0] else {
            unreachable!()
        };
        let definitions = definitions.rows::<tables::VariantDefinitions>().unwrap();
        assert_eq!(definitions.len(), countries * 16);
        for row in definitions {
            let index = (row.id >> 32) as usize - 1;
            assert_eq!(
                row.explicit_entity,
                explicit_entity(index).as_bytes(),
                "variant {index}"
            );
            let (active, kind, icon) = icon(index);
            assert_eq!(
                (row.icon_active, row.icon_kind, row.icon.as_slice()),
                (active, kind, icon.as_bytes()),
                "variant {index}"
            );
        }
        let TableView::Complete(stats) = &views[2] else {
            unreachable!()
        };
        let stats = stats.rows::<tables::VariantStats>().unwrap();
        assert_eq!(stats.len(), countries * 16);
        for row in stats {
            let index = (row.variant >> 32) as usize - 1;
            assert_eq!(row.organisation, (index / 16 + 1) as u64);
            assert_eq!(row.organisation_trait_count, (index % 9) as i32);
            assert!(row.organisation_bonuses.is_empty());
        }
    }
}

