//! The saved wing replacement preference and its two independent writers.
pub const SETTING: usize = 0x9cc;
pub const COMMAND_WINGS: usize = 0x28;
pub struct Mutation {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "DeployWing",
        rva: 0x00c4fd20,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
    },
    Mutation {
        name: "SetWingReplacement",
        rva: 0x01946430,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
    },
];
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_writer_sites_authenticate_the_supported_executable() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let path =
            std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).unwrap()).join("hoi4.exe");
        let image =
            engine_executable::executable_tables::PeImage::parse(std::fs::read(path).unwrap())
                .unwrap();
        for site in MUTATIONS {
            assert_eq!(
                image.bytes(site.rva as u32, 16).unwrap(),
                site.sentinel,
                "{}",
                site.name
            );
            assert_eq!(image.function(site.rva as u32).unwrap().0, site.rva as u32);
        }
    }
}
