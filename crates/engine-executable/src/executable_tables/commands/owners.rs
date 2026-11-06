use super::Target;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Object {
    Unit,
    Group,
    Theatre,
    TheatreGroup,
    Leader,
    IndustrialOrganisation,
    Wing,
    WingPool,
    Ship,
    Fleet,
    ProductionLine,
    DivisionTemplate,
    Character,
    Ace,
    AirGroup,
    CountryReference,
    CountryToken,
    NavalTheatreGroup,
    Raid,
    Conveyor,
    ConveyorLine,
    Front,
    NavalCombat,
    DiplomaticAction,
    MarketAccessRequest,
}

impl Object {
    pub(super) fn mask(self) -> u32 {
        1 << self as u8
    }

    pub fn secondary(self) -> usize {
        match self {
            Self::Unit | Self::Wing | Self::NavalCombat => 16,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Source {
    Country(u32),
    Reference(u32, Object),
    References(u32, Object),
    Pointer(u32, Object),
    Province(u32),
    Building { definition: u32, state: u32 },
    NavalSide { combat: u32, side: u32 },
    ScriptCountry { pointer: u32, length: u32 },
}

impl Source {
    /// The command fields this source reads.
    pub(super) fn fields(self) -> [Option<u32>; 2] {
        match self {
            Self::Country(field)
            | Self::Reference(field, _)
            | Self::References(field, _)
            | Self::Pointer(field, _)
            | Self::Province(field) => [Some(field), None],
            Self::Building { definition, state } => [Some(definition), Some(state)],
            Self::NavalSide { combat, side } => [Some(combat), Some(side)],
            Self::ScriptCountry { pointer, length } => [Some(pointer), Some(length)],
        }
    }
}

/// The owner operands each command class's permission check reads, by class hash.
pub(super) fn target(class: u64) -> Target {
    use Object::*;
    use Source::{Country, Reference as R, References as A};
    const OWNERS: [(Target, &[u64]); 70] = [
        (
            Target::Country(0x28),
            &[
                0x007a_58c2_d91c_0192,
                0x047f_c567_63d7_b186,
                0x05cd_cc16_20e4_93f3,
                0x0699_a016_7d7b_131b,
                0x0719_d0b1_f76b_fda2,
                0x08ad_c618_8b9e_32a3,
                0x0f42_1538_2edc_cf04,
                0x1037_7452_40c7_6931,
                0x140c_5172_b10b_1a63,
                0x147d_dba4_afe9_dc03,
                0x1589_ede4_11ed_bac4,
                0x15a2_c266_58d7_34bd,
                0x1679_fc1f_0d46_0b11,
                0x1761_88bc_683e_2a05,
                0x1816_0276_80c7_ebd2,
                0x1ba5_6e81_c9f3_188d,
                0x1c40_6967_abe1_5981,
                0x1fb1_1af7_0283_4351,
                0x241f_80da_62c7_2686,
                0x2426_7133_5a6a_040b,
                0x2641_bf9b_570c_8ff7,
                0x2b79_a737_68e7_4869,
                0x2cb8_532f_d809_6d10,
                0x2f6a_eabf_e9e1_a803,
                0x353f_d1be_857e_d26f,
                0x3a67_18eb_92d5_cd25,
                0x3d05_930d_4dd1_b54d,
                0x3d18_c9be_70be_5214,
                0x3d3a_60d4_e57e_895b,
                0x4b59_4a5c_00f9_d7f9,
                0x4c87_07e7_46ce_bf32,
                0x4d67_38d7_6428_43c7,
                0x5464_d4e3_6a09_61bd,
                0x54f1_180a_3009_8798,
                0x5a32_ef72_0ae0_1d32,
                0x5a7b_6efb_2a7a_cfd6,
                0x5cfa_5a50_57e6_bf6b,
                0x615d_f82e_b5ec_7e39,
                0x62f1_2369_89c9_d72b,
                0x6558_d619_b446_eddb,
                0x6792_f018_620c_afc2,
                0x6acd_e514_2270_5325,
                0x6b7c_155d_0463_a83e,
                0x6cc7_c97f_91b5_3cc8,
                0x6e69_bb41_3126_3372,
                0x6f2b_4553_e229_c07b,
                0x6f57_b958_fa4c_8d99,
                0x6fc7_6ba4_d61d_5d1c,
                0x739b_e4c0_9f2d_c90f,
                0x77fc_6279_a84e_7e27,
                0x78b7_ce76_010e_3fa2,
                0x7d51_0f7b_24de_13fb,
                0x8151_74f5_1036_66ca,
                0x89f6_a44a_65b6_91b2,
                0x8b2e_ea58_5a4b_b2aa,
                0x8b88_4700_1488_c028,
                0x8f6f_6fc6_baea_1a56,
                0x919d_8ae7_a3d6_afad,
                0x91f1_2b63_b450_6618,
                0x9256_73de_08ff_c7b6,
                0x925d_0c1e_e6d5_f6d4,
                0x9306_b320_518d_a333,
                0x93b3_7ca0_4d87_56a2,
                0x9580_0b43_7b7b_3125,
                0x95af_fd48_c389_2d27,
                0x973b_ee71_aa71_2c98,
                0x9825_339d_934b_2012,
                0x9872_a03d_805d_8079,
                0x9903_808d_99d9_519c,
                0x9b55_cf4c_2e91_a528,
                0x9dbd_3763_84fa_8210,
                0xa28c_07da_83f0_63e6,
                0xa59a_83a5_814a_f455,
                0xa819_cb61_861b_8132,
                0xb350_192d_cc10_bc5d,
                0xb3f8_aab5_c428_bb9e,
                0xb536_2e82_49eb_3d9a,
                0xb8af_93b7_7492_5c91,
                0xbabb_4967_9aa5_5373,
                0xbb26_f862_67c2_9bfe,
                0xbd75_8576_18ad_a475,
                0xc358_88a9_a9b7_025b,
                0xc8c2_f70b_b624_7e5e,
                0xce27_99b6_6d98_13eb,
                0xcfd1_3aec_7306_6db5,
                0xcffa_0f47_107c_cd6d,
                0xd725_5454_4ba7_00e2,
                0xd737_abbf_42e1_45fd,
                0xd82b_83cc_6870_f982,
                0xdd5b_37ad_be5f_0e40,
                0xde74_307b_faf1_5081,
                0xe38c_b75d_ab8b_6eaf,
                0xe7ae_5ef5_ff64_5c04,
                0xe94c_263e_8dac_b828,
                0xed96_f913_ce47_4ff3,
                0xee7d_4e95_9421_1e3a,
                0xf1e1_eb75_3f6a_69e5,
                0xf45f_a044_00ab_045b,
                0xf621_1be8_735a_c9b9,
                0xf63f_f721_b2ef_c10d,
                0xf7f6_7629_9956_7871,
                0xfad4_3be2_df14_5848,
                0xfb0b_fd12_cd06_c45b,
                0xfe00_cd81_81c6_6a58,
            ],
        ),
        (
            Target::Group(0x28),
            &[
                0x0c89_e5eb_2aa1_a07f,
                0x11e4_8149_59c4_7bde,
                0x15df_bb82_175e_e36d,
                0x16a8_4364_f230_0c56,
                0x1926_f0af_e4e7_1ddf,
                0x1cdb_0e17_2ff5_c2e9,
                0x2c4a_47a2_175a_e841,
                0x4242_8fb2_508d_02e8,
                0x4420_41f0_8772_7cd2,
                0x486e_af45_5b7d_45c7,
                0x602b_c33f_24c6_b9d2,
                0x6290_a3a3_ea26_6c7d,
                0x6348_e1e8_6dd2_66b3,
                0x7381_ab9e_0a1c_34f8,
                0x903c_a2cd_d93b_6cda,
                0x9668_b430_6be0_0d13,
                0x9694_98e0_2de2_9142,
                0x98f6_8bf7_4d2d_bc06,
                0xa1ff_2e88_bf3c_451d,
                0xa899_2510_d85d_2789,
                0xaef3_9492_27a5_e696,
                0xb16e_7d3e_6cdc_18c8,
                0xbf62_d118_aa1b_592d,
                0xc8eb_a0c4_fc7c_3b02,
                0xce01_46c5_c0d2_d387,
                0xdba1_55ce_973b_a7f8,
                0xdf89_e3b9_6465_d6e6,
                0xed73_c676_3398_3b13,
            ],
        ),
        (Target::Group(0x40), &[0x2874_ddda_66af_eec0]),
        (
            Target::Unit(0x38),
            &[0x61d0_58b7_c19f_ec6b, 0xac31_fa8e_475f_feac],
        ),
        (Target::Units(0x40), &[0x3fc9_6e71_07e7_d2ec]),
        (
            Target::Units(0x28),
            &[
                0x1da7_c1d8_c31d_7f0e,
                0x2ca2_45fc_b2c0_aacc,
                0x2e5c_f78d_5bda_0cbd,
                0x51c7_23d3_9cd8_e0d6,
                0x7469_0db2_c92a_06b2,
                0x8836_f3ce_5e50_44d2,
                0x9b3b_6dd4_f64e_d460,
                0xb2af_d431_7a00_b80b,
                0xbd5a_7fab_6ae9_d497,
                0xc38b_0a84_6769_566e,
                0xcf17_1ae6_5e58_02d9,
                0xe09c_ed56_4c0d_c4ce,
                0xeedb_3f86_df75_f2a2,
                0xf8df_8ea9_e434_93fc,
            ],
        ),
        (
            Target::Wings(0x28),
            &[
                0x4915_687a_7135_dbec,
                0x7701_a5b0_1ed2_60a3,
                0x8c65_ba84_b724_e40e,
                0x8f6c_1bf5_2003_da5d,
                0xf445_02cb_3609_c29a,
            ],
        ),
        (
            Target::Sources(&[Country(0x34)]),
            &[
                0x1f7f_68b7_8387_676a,
                0x6175_5ea5_9657_6378,
                0x622e_5c78_2d7c_4788,
                0xb7c1_942c_19bd_fb70,
            ],
        ),
        (
            Target::Sources(&[Country(0x28)]),
            &[
                0x02ec_a8fa_3250_b139,
                0x1944_9579_4844_d519,
                0x2202_99ae_3adb_2a60,
                0x29ad_d984_bbb6_039d,
                0x2cb0_cbe3_a749_e80b,
                0x2f7a_d2d9_bc85_8471,
                0x3640_a6ac_e84d_5323,
                0x4547_42c9_176e_0f02,
                0x46c3_c372_fe31_5808,
                0x4c73_824e_b786_7d4d,
                0x58e7_7b2e_b338_8fa4,
                0x6243_395d_37d3_6223,
                0x7363_ce61_5817_e0b7,
                0x7d29_b0a6_52e1_7fdd,
                0x887c_7129_d199_f444,
                0x8b59_459b_2182_d390,
                0x9da6_d13f_0a5f_a739,
                0xaada_5a10_50f0_3116,
                0xad75_bc7a_9d06_6d12,
                0xb02e_b540_37f4_027d,
                0xbb98_f608_9794_020a,
                0xda35_13ed_8df4_bb20,
                0xf957_90e4_1b1f_475c,
            ],
        ),
        (
            Target::Sources(&[Country(0x30)]),
            &[
                0x09b2_886c_1d9e_e136,
                0x658c_c819_7985_e41c,
                0x65c2_2693_a969_b241,
                0x7c85_5438_bae8_4c3f,
                0x7f6c_d9cb_eace_7231,
                0x8df7_1812_cd53_a045,
                0x9f8c_eeeb_1200_db6f,
                0xa53d_8a5d_d345_a98c,
                0xc9dd_8d6f_f9cb_ad90,
                0xcb15_e55c_c60c_22b8,
                0xd875_806e_6d88_f4c7,
            ],
        ),
        (Target::Sources(&[Country(0x48)]), &[0xd39c_b7e4_e283_81e8]),
        (
            Target::Sources(&[Country(0x40)]),
            &[
                0x2eb3_fb57_8453_173c,
                0x9c2a_7fbc_18ca_c88b,
                0xc562_837c_1efa_4e8f,
                0xff57_48ab_4615_6e39,
            ],
        ),
        (
            Target::Sources(&[Country(0x2c)]),
            &[
                0x12d3_7630_ac91_ecdc,
                0x2dfc_0536_a037_9eae,
                0x4511_dc58_10ac_c705,
                0x4f84_c14d_3e11_c12c,
                0x79f6_3f14_7dfb_79af,
                0xd00f_9059_7da4_9dcb,
                0xe519_3f0f_6079_3d32,
            ],
        ),
        (Target::Sources(&[Country(0x50)]), &[0x0b77_73bc_1c6d_d5e2]),
        (Target::Sources(&[Country(0x3c)]), &[0x34e8_db8d_7a80_60b4]),
        (Target::Sources(&[Country(0xa8)]), &[0xf8a5_f1a2_41aa_6b62]),
        (Target::Sources(&[Country(0x270)]), &[0xd62f_399b_e83c_06b4]),
        (Target::Sources(&[Country(0x278)]), &[0x8e2d_42c7_8f0d_3997]),
        (
            Target::Sources(&[Source::ScriptCountry {
                pointer: 0x40,
                length: 0x4c,
            }]),
            &[0xd0a4_2bd8_3956_00ba],
        ),
        (
            Target::Sources(&[Source::Pointer(0x28, DiplomaticAction)]),
            &[0xdde4_7195_5485_1a57],
        ),
        (
            Target::Sources(&[Source::Building {
                definition: 0x28,
                state: 0x2c,
            }]),
            &[0x0b93_f9b2_6f6a_a499],
        ),
        (
            Target::Sources(&[
                Country(0x88),
                Source::Building {
                    definition: 0x50,
                    state: 0x54,
                },
            ]),
            &[0x39db_d043_2ab4_7a6b],
        ),
        (
            Target::Sources(&[Source::Building {
                definition: 0x28,
                state: 0x30,
            }]),
            &[0x7c61_c87b_1511_4a56],
        ),
        (
            Target::Sources(&[Source::Province(0x28)]),
            &[0x796f_931a_e8d4_4a13],
        ),
        (
            Target::Sources(&[Source::NavalSide {
                combat: 0x28,
                side: 0x30,
            }]),
            &[0xb262_a7b7_fddb_bc58],
        ),
        (
            Target::Sources(&[R(0x28, ProductionLine)]),
            &[
                0x0207_4b43_5cf2_d68f,
                0x1264_0cc7_ae82_6fa7,
                0x19c4_ddad_7577_8d53,
                0x1fa7_7863_4803_37da,
                0x303d_ce01_7608_9c3e,
                0x59b1_de1a_6aaf_d68d,
                0x663e_e061_7033_4ab4,
                0x69f5_cb74_4df0_34b8,
                0x7821_0f3a_5da9_b02c,
                0x7bf7_6bfb_5836_cde7,
                0x81cb_dadb_fc44_c905,
                0x8d4c_9f3a_49ee_62c8,
                0x9110_21fd_c341_dcaf,
                0xa263_e3b9_2a0c_2f24,
                0xa5b9_3a6c_72ce_fc49,
                0xc2a6_1acb_3cb9_a043,
                0xd998_5f34_fb3d_463b,
                0xf124_82b4_de4b_4b83,
            ],
        ),
        (
            Target::Sources(&[A(0x28, ProductionLine)]),
            &[
                0x0e50_89eb_9eec_968b,
                0x0f7b_69c5_b684_a7b5,
                0x19bc_b07d_075c_3ad7,
                0xa2f7_dfda_1a9d_cb29,
                0xba49_b76b_3c99_5419,
                0xc7af_bf0d_d9e1_6bb0,
            ],
        ),
        (
            Target::Sources(&[R(0x28, Leader)]),
            &[
                0x4319_1f9d_302e_eaf9,
                0x5db8_d46b_773b_1e02,
                0x6b16_0e0a_804e_919c,
                0x8e13_14a6_04d8_336e,
                0xc9c0_9148_d462_d5a8,
                0xf0fc_1f57_919b_a214,
            ],
        ),
        (
            Target::Sources(&[R(0x28, IndustrialOrganisation)]),
            &[0x8c53_c475_542c_683e, 0xe9e4_807c_4ab7_e033],
        ),
        (
            Target::Sources(&[R(0x30, Leader)]),
            &[0xbfa3_4661_67fc_0058],
        ),
        (
            Target::Sources(&[R(0x28, Character)]),
            &[
                0x38da_ed51_9236_1d36,
                0x7cdd_e47d_219a_ed6d,
                0xa42b_6c5f_b6b1_4b7f,
                0xb58b_00a0_4931_6c2b,
                0xd701_7c8a_8d19_2d73,
                0xe05a_5c43_51e7_7d04,
            ],
        ),
        (
            Target::Sources(&[R(0x30, Character)]),
            &[0xd802_020b_2391_83b5],
        ),
        (
            Target::Sources(&[R(0x28, Ace), R(0x30, Wing)]),
            &[0x2062_8863_e200_e2b7],
        ),
        (
            Target::Sources(&[R(0x28, Ship)]),
            &[
                0x2566_460a_ad19_f7ad,
                0x7935_392c_6cfe_f874,
                0xbb73_8a58_3489_5917,
                0xcd29_0242_e423_bb28,
            ],
        ),
        (
            Target::Sources(&[R(0x2c, Ship)]),
            &[0x3225_e6d5_ae9c_5a9d, 0x69b8_1828_9f2f_c57c],
        ),
        (
            Target::Sources(&[R(0x28, CountryToken)]),
            &[
                0x402d_e946_b022_b8d8,
                0x731c_d419_ecd5_551a,
                0x8723_7b5b_3407_e529,
                0x8dbd_0d76_3e7f_877d,
                0xa092_32e7_7037_6342,
                0xb1fc_fc4b_3090_9eb3,
                0xb3d8_e88f_e4a5_ed91,
                0xb629_51d0_126f_d2f7,
                0xb633_443e_a357_e023,
                0xd03a_1339_b351_7db6,
                0xde1e_3841_8ed2_9115,
                0xfc86_d6ad_00ed_cfdd,
            ],
        ),
        (
            Target::Sources(&[R(0x28, DivisionTemplate)]),
            &[
                0x08e1_0976_cad1_044b,
                0x3223_d60f_0633_5f1c,
                0x51b6_1400_bb27_9004,
                0x821c_0cbe_3179_d405,
                0xbc4b_4246_78aa_5330,
                0xd360_b21c_cac5_7450,
            ],
        ),
        (
            Target::Sources(&[R(0x28, Unit)]),
            &[
                0x07b1_2417_f3ed_a6ae,
                0x103f_8929_34b6_771f,
                0x2688_137f_768b_4dac,
                0x44d5_6a2c_3d10_1e41,
                0x6bab_ea00_eb27_8bb4,
                0x707f_9be6_9501_e726,
                0x80ed_da20_a457_df29,
                0x830b_7a47_07de_c504,
                0x9626_c18e_a6f9_9d18,
                0xa229_9c6a_675c_b672,
                0xc2d8_b538_2555_7d6a,
                0xe311_9481_c6c9_e07a,
                0xe335_90c8_ea80_8767,
                0xf2de_c0bf_cf48_5636,
                0xf3fa_dc61_2614_4fbc,
            ],
        ),
        (
            Target::Sources(&[A(0x28, Unit)]),
            &[
                0x0de9_17ef_adfb_76da,
                0x4ace_900b_9ba8_bc7b,
                0x7233_de69_e4bc_55e4,
                0x8c17_11f3_8fb8_2fc1,
                0x9e00_cb4f_d274_ea41,
                0xdc71_8c29_2aea_c2e2,
                0xea18_87bf_6acc_3742,
            ],
        ),
        (Target::Sources(&[R(0x38, Unit)]), &[0xade5_925e_e7f5_e3c2]),
        (
            Target::Sources(&[R(0x28, Group), R(0x30, DivisionTemplate)]),
            &[0x9e53_1a5b_c4e3_eb50],
        ),
        (
            Target::Sources(&[R(0x28, Leader), R(0x30, Group)]),
            &[0xbf83_f2d2_6176_897d],
        ),
        (
            Target::Sources(&[A(0x28, Ship), R(0x40, Unit), R(0x48, Fleet)]),
            &[0x199f_f66b_ea9d_ae73],
        ),
        (
            Target::Sources(&[A(0x28, Ship), R(0x40, Unit)]),
            &[0x3a15_725e_22e0_eb71, 0xda23_774d_ed83_b4e8],
        ),
        (
            Target::Sources(&[A(0x28, Unit), A(0x40, Ship)]),
            &[0x3ec7_0369_57e1_a46d],
        ),
        (
            Target::Sources(&[A(0x28, Ship), R(0x40, Unit), R(0x48, CountryToken)]),
            &[0x9fb4_917d_fd10_7958],
        ),
        (
            Target::Sources(&[R(0x28, Fleet)]),
            &[
                0x5466_947d_555d_654a,
                0x6817_d423_804c_97f0,
                0x79f4_34a5_a9dc_e86a,
                0x7f71_1e31_3280_8bed,
                0xb4b5_8334_25dd_f0b0,
                0xb5d1_c3ec_0ce3_5850,
                0xb605_bb3a_b362_1885,
            ],
        ),
        (
            Target::Sources(&[R(0x28, Fleet), A(0x30, Unit)]),
            &[0xc8fd_5164_ca13_0c7f],
        ),
        (
            Target::Sources(&[
                R(0x28, Fleet),
                A(0x30, Unit),
                A(0x48, Ship),
                R(0x60, NavalTheatreGroup),
            ]),
            &[0x2876_e36c_ddd2_331f],
        ),
        (
            Target::Sources(&[
                R(0x48, Theatre),
                R(0x50, TheatreGroup),
                R(0x58, Group),
                A(0x30, Unit),
            ]),
            &[0x8e27_a2a4_761c_599a],
        ),
        (
            Target::Sources(&[R(0x28, Theatre), A(0x30, Group), R(0x60, Front)]),
            &[0x2e42_87af_db7b_e51f],
        ),
        (
            Target::Sources(&[A(0x28, Group), R(0x40, Group)]),
            &[0x1420_5401_904c_a89b],
        ),
        (
            Target::Sources(&[A(0x28, Group)]),
            &[0x1471_1a26_f12a_4a3f, 0x6589_ce5e_2805_a9fb],
        ),
        (
            Target::Sources(&[R(0x28, Group)]),
            &[
                0x0f1e_ebec_96f9_20e3,
                0x4787_d8a3_a19a_39de,
                0x4b4b_9647_01b5_d2f3,
                0x5cd3_55b0_cfa9_6584,
                0x5f19_eae1_fccd_4f72,
                0x79b6_6606_aacb_19d0,
                0x7a2d_ed34_313f_ef33,
                0x7bf4_7c75_1c64_11d6,
                0x9195_c3a9_b34b_cbcc,
                0xaefd_acfe_5a74_c5c0,
                0xc12f_aa36_e175_60c0,
                0xcc4a_315e_bc47_7d75,
                0xd36d_8583_92dd_602c,
                0xd4a2_1c99_70e8_95cd,
                0xd801_78da_c339_d275,
                0xddfa_966b_10d9_d09c,
                0xdfc9_c3ca_3491_b9ed,
                0xec26_736d_25c6_0d7a,
            ],
        ),
        (
            Target::Sources(&[R(0x40, Group), A(0x28, Unit)]),
            &[0xd7db_9fcd_d95c_8999],
        ),
        (Target::Sources(&[R(0x28, Front)]), &[0xed68_dda6_e103_2367]),
        (
            Target::Sources(&[R(0x30, Group), R(0x28, Leader)]),
            &[0xe1d4_220e_7ab4_fb63],
        ),
        (
            Target::Sources(&[R(0x30, Fleet), R(0x28, Leader)]),
            &[0x2933_151d_cec8_90e0],
        ),
        (
            Target::Sources(&[A(0x28, Wing), A(0x40, Wing)]),
            &[0x1f75_6031_8230_b398],
        ),
        (
            Target::Sources(&[R(0x28, Wing)]),
            &[
                0x2e61_1962_69d6_53ec,
                0x3e13_6165_31ab_9ebd,
                0x4886_b411_ee37_3561,
                0x4b69_97f0_dece_25af,
                0x5472_db54_a421_9ae7,
                0x9349_c795_b69e_2c73,
                0xa44a_4c90_66b5_91d2,
                0xb3ac_be2f_b9a2_9217,
            ],
        ),
        (
            Target::Sources(&[R(0x28, WingPool)]),
            &[0xdbb8_e797_0dba_1945],
        ),
        (
            Target::Sources(&[R(0x40, Group), A(0x28, Wing)]),
            &[0xed48_0846_5bdc_9b63],
        ),
        (
            Target::Sources(&[R(0x28, CountryReference)]),
            &[
                0x2afb_610d_ffe8_5088,
                0x34eb_8631_f189_fd41,
                0x38e9_36e0_4ffc_102e,
                0x4e3e_c2b5_5337_5e68,
                0x5d6b_f502_2d57_67b8,
                0x6fe3_e38f_b224_2739,
                0x8790_ee4d_642c_6be7,
                0x96bc_25f9_9d12_11f6,
            ],
        ),
        (
            Target::Sources(&[R(0x28, AirGroup)]),
            &[0x2107_47c6_f2e2_ff68, 0x25dd_6ad1_677b_3362],
        ),
        (
            Target::Sources(&[R(0x28, TheatreGroup), A(0x30, Group)]),
            &[0x5304_e421_bb98_cd2f],
        ),
        (
            Target::Sources(&[R(0x28, TheatreGroup)]),
            &[
                0x11c2_005f_4d36_7586,
                0x2e17_9699_aa2a_ecd7,
                0x6d55_681c_13d8_04cc,
                0x8f3f_bec4_cc60_eaba,
            ],
        ),
        (
            Target::Sources(&[R(0x28, NavalTheatreGroup)]),
            &[
                0x2d17_5777_db9d_8edd,
                0x321e_48e3_851d_831a,
                0x9c08_b367_4d15_db2a,
            ],
        ),
        (
            Target::Sources(&[R(0x28, Raid)]),
            &[
                0x267f_a745_a71b_c9a1,
                0x33aa_4bfb_e9b2_d247,
                0x3de8_6910_8763_6150,
                0x3de8_6910_8763_6150,
                0x3de8_6910_8763_6150,
                0x3de8_6910_8763_6150,
                0x3de8_6910_8763_6150,
                0x3de8_6910_8763_6150,
                0x440d_d200_816e_fd10,
                0x5afe_603f_b2ca_92cc,
                0xaeb8_1df6_cd03_fbf4,
            ],
        ),
        (
            Target::Sources(&[R(0x28, Conveyor)]),
            &[
                0x1881_18d1_0a1f_415e,
                0x1f70_5488_4eba_30fd,
                0x38e9_0c90_f126_064f,
                0x5ccb_998d_c63e_9549,
                0x88fa_fb55_822b_f549,
                0x8a51_3165_bcce_25a5,
                0x92ec_0992_bbd4_ec7f,
                0xa395_f90b_5e6c_1dbc,
                0xc1ad_3c71_374d_6ea0,
                0xdf02_f7c4_bd3e_02b3,
                0xfb4c_9c90_e036_6f19,
            ],
        ),
        (
            Target::Sources(&[R(0x28, ConveyorLine)]),
            &[
                0x39c9_673c_c003_b8eb,
                0x4d70_0f1b_19ef_9916,
                0x8adb_1af6_bea9_5fc1,
            ],
        ),
    ];
    OWNERS
        .iter()
        .find(|(_, classes)| classes.contains(&class))
        .map_or(Target::Unknown, |&(target, _)| target)
}

// FNV-1a hashes of the decorated class names whose objects command operands reference.
const FAMILIES: [(u64, Object); 27] = [
    (0x1256_3b1d_6ded_890e, Object::Fleet),
    (0x15da_6aad_d0a2_5798, Object::Ship),
    (0x23b6_196e_980f_e3c3, Object::ProductionLine),
    (0x36dd_a35a_4c45_1ca2, Object::TheatreGroup),
    (0x3704_5382_d3cb_13ac, Object::DivisionTemplate),
    (0x3fb4_fb26_4b34_b749, Object::Ace),
    (0x413a_1964_2117_971b, Object::WingPool),
    (0x4145_a4f2_4ff5_9d5b, Object::CountryToken),
    (0x4a26_985b_1465_5ee5, Object::CountryReference),
    (0x6009_2477_f868_e0eb, Object::IndustrialOrganisation),
    (0x65aa_09f7_8d0a_b622, Object::Group),
    (0x6c83_5408_6fc5_c5c6, Object::DiplomaticAction),
    (0x75c0_0b44_e341_39d1, Object::CountryReference),
    (0x7724_2b5d_c20d_0291, Object::AirGroup),
    (0x7a93_2316_b846_7167, Object::Conveyor),
    (0x9205_687f_6930_43ad, Object::Character),
    (0x9251_363b_dce1_0e1d, Object::Front),
    (0x9937_08a4_8664_b634, Object::Raid),
    (0xa06a_5b54_1e30_f3ce, Object::CountryToken),
    (0xb55c_b44a_000b_df34, Object::Unit),
    (0xb84f_59ab_a9b2_812d, Object::Wing),
    (0xbfbf_fa5d_33ad_0a5a, Object::NavalCombat),
    (0xc8fc_d515_72f6_237e, Object::NavalTheatreGroup),
    (0xd771_5bcc_d1f6_45cd, Object::Leader),
    (0xda21_432f_0826_4ee2, Object::MarketAccessRequest),
    (0xec3a_eafa_f4e5_8275, Object::Theatre),
    (0xfa45_06ab_ff95_ef58, Object::ConveyorLine),
];

/// The object family of a decorated class name's hash.
pub(super) fn family(class: u64) -> Option<Object> {
    FAMILIES
        .binary_search_by_key(&class, |&(hash, _)| hash)
        .ok()
        .map(|index| FAMILIES[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theatre_group_assignment_alone_falls_back_to_orders_group_owners() {
        assert!(matches!(
            target(0x5304_e421_bb98_cd2f),
            Target::Sources([
                Source::Reference(0x28, Object::TheatreGroup),
                Source::References(0x30, Object::Group),
            ])
        ));
        assert!(matches!(
            target(0x6d55_681c_13d8_04cc),
            Target::Sources([Source::Reference(0x28, Object::TheatreGroup)])
        ));
    }
}
