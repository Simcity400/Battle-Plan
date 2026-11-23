use collector_characters::extended::characters::{
    advisors::{
        Cache, Section,
        tests::{Observer, context},
    },
    characters_test_kit as kit,
    tests::{ADDRESS, IMAGE},
};
use engine_native_memory::Memory;

#[test]
fn advisor_modifier_sizing_survives_an_unreadable_political_census() {
    let _globals = collector_characters::extended::characters::test_globals();
    struct World<'a>(&'a [u8]);
    impl Memory for World<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == IMAGE + engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&(ADDRESS as u64).to_le_bytes());
                return true;
            }
            if address == ADDRESS + engine_native::cgame_state::COUNTRIES && output.len() == 16 {
                output[..8].copy_from_slice(&((ADDRESS + 0x3000) as u64).to_le_bytes());
                output[8..12].copy_from_slice(&2_u32.to_le_bytes());
                output[12..].copy_from_slice(&2_u32.to_le_bytes());
                return true;
            }
            kit::memory(self.0).copy(address, output)
        }
    }
    let bytes = kit::mapped_fixture();
    let memory = World(&bytes);
    assert!(crate::country_scale::observe_politics(&memory, IMAGE).is_err());
    let demand = crate::country_scale::observe_advisor_modifiers(&memory, IMAGE)
        .unwrap()
        .unwrap();
    assert!(demand.rows > 0);
    let mut cache = Cache::with_modifier_demand(demand);
    cache.sections = [
        Section::with_capacity(16384, 1 << 20, 16384),
        Section::with_capacity(4096, 1 << 18, 8192),
    ];
    cache.capture(&memory, context(), &Observer::new(), None);
    assert_eq!(cache.modifiers.shortfall, 0);
}
