use super::{Target, invalid};
use crate::executable_tables::pe::PeImage;
use std::io;

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

/// The owner operands each command permission check reads, by the check's address.
pub(super) fn target(permission: u32) -> Target {
    use Object::*;
    use Source::{Country, Reference as R, References as A};
    let sources: &'static [Source] = match permission {
        0x015d_6280 => &[Country(0x34)],
        0x01ab_b880 | 0x011e_c410 | 0x011e_c3c0 | 0x011e_b8e0 | 0x013e_cf80 | 0x019d_3870
        | 0x019d_38d0 | 0x019d_3650 | 0x019d_36d0 | 0x01a3_25a0 | 0x01a3_afd0 | 0x01ab_b930
        | 0x01c3_7cc0 | 0x013e_d580 | 0x013e_d640 => &[Country(0x28)],
        0x01a3_af50 | 0x013e_d870 | 0x01ab_b830 | 0x01c3_7c70 | 0x011e_b7a0 => &[Country(0x30)],
        0x01ab_b8d0 | 0x01c3_7d80 => &[Country(0x34)],
        0x01ab_b820 => &[Country(0x48)],
        0x011e_be70 | 0x011e_bea0 | 0x019d_35f0 => &[Country(0x40)],
        0x011e_c4d0 => &[Country(0x2c)],
        0x01ab_b8e0 => &[Country(0x50)],
        0x01b1_0300 => &[Country(0x3c)],
        0x01ba_e720 => &[Country(0xa8)],
        0x01c2_d920 => &[Country(0x270)],
        0x01c2_daa0 => &[Country(0x278)],
        0x011e_be80 => &[Source::ScriptCountry {
            pointer: 0x40,
            length: 0x4c,
        }],
        0x011c_10e0 => &[Source::Pointer(0x28, DiplomaticAction)],
        0x011e_c0a0 => &[Source::Building {
            definition: 0x28,
            state: 0x2c,
        }],
        0x011e_c260 => &[
            Country(0x88),
            Source::Building {
                definition: 0x50,
                state: 0x54,
            },
        ],
        0x011e_bcd0 => &[Source::Building {
            definition: 0x28,
            state: 0x30,
        }],
        0x013e_ce20 => &[Source::Province(0x28)],
        0x013e_d000 => &[Source::NavalSide {
            combat: 0x28,
            side: 0x30,
        }],
        0x011e_b800 | 0x011e_c150 | 0x011e_c460 | 0x011e_b870 | 0x011e_c0e0 | 0x011e_bbf0
        | 0x011e_bc60 => &[R(0x28, ProductionLine)],
        0x011e_b7f0 | 0x011e_c000 | 0x011e_c4e0 => &[A(0x28, ProductionLine)],
        0x011e_c350 | 0x013f_eb00 => &[R(0x28, Leader)],
        0x01a3_2530 | 0x01a3_25f0 => &[R(0x28, IndustrialOrganisation)],
        0x018d_d0e0 => &[R(0x30, Leader)],
        0x011e_c580 => &[Country(0x28)],
        0x011e_b730 | 0x011e_c1c0 => &[R(0x28, Character)],
        0x01c3_7d10 => &[R(0x30, Character)],
        0x011e_b930 => &[R(0x28, Ace), R(0x30, Wing)],
        0x011e_c6a0 | 0x013f_e7a0 => &[R(0x28, Ship)],
        0x013e_d510 => &[R(0x2c, Ship)],
        0x01a3_af60 | 0x01f6_b990 => &[R(0x28, CountryToken)],
        0x01c2_da30 => &[R(0x28, DivisionTemplate)],
        0x011e_c2c0 => &[Country(0x28)],
        0x011e_c630 | 0x013f_e840 | 0x013f_ec10 | 0x013f_ebc0 | 0x013f_e920 => &[R(0x28, Unit)],
        0x011e_bf50 | 0x013f_ecc0 | 0x013f_eb70 => &[A(0x28, Unit)],
        0x013f_ecd0 => &[R(0x38, Unit)],
        0x013f_e850 => &[R(0x28, Group), R(0x30, DivisionTemplate)],
        0x013f_ec20 => &[R(0x28, Leader), R(0x30, Group)],
        0x013f_e9b0 => &[A(0x28, Ship), R(0x40, Unit), R(0x48, Fleet)],
        0x013e_d410 => &[A(0x28, Ship), R(0x40, Unit)],
        0x013e_d3c0 => &[A(0x28, Unit), A(0x40, Ship)],
        0x013e_d460 => &[A(0x28, Ship), R(0x40, Unit), R(0x48, CountryToken)],
        0x013e_d500 => &[R(0x28, Unit)],
        0x013e_cdb0 => &[R(0x28, Fleet)],
        0x013e_d7d0 => &[R(0x28, Fleet), A(0x30, Unit)],
        0x013e_d710 => &[
            R(0x28, Fleet),
            A(0x30, Unit),
            A(0x48, Ship),
            R(0x60, NavalTheatreGroup),
        ],
        0x018d_d670 => &[
            R(0x48, Theatre),
            R(0x50, TheatreGroup),
            R(0x58, Group),
            A(0x30, Unit),
        ],
        0x018d_cfd0 => &[R(0x28, Theatre), A(0x30, Group), R(0x60, Front)],
        0x018d_d220 => &[A(0x28, Group), R(0x40, Group)],
        0x018d_d2e0 => &[A(0x28, Group)],
        0x018d_d2f0 | 0x018d_d5d0 | 0x018d_d760 | 0x018d_d8d0 | 0x018d_d530 | 0x018d_d800
        | 0x018d_da60 | 0x018d_db00 | 0x018d_ddc0 | 0x018d_d150 | 0x018d_d980 | 0x018d_d360 => {
            &[R(0x28, Group)]
        }
        0x018d_dba0 => &[R(0x40, Group), A(0x28, Unit)],
        0x018d_d270 => &[R(0x28, Front)],
        0x018d_dc40 => &[R(0x30, Group), R(0x28, Leader)],
        0x018d_dd00 => &[R(0x30, Fleet), R(0x28, Leader)],
        0x019d_3820 => &[A(0x28, Wing), A(0x40, Wing)],
        0x019d_37a0 => &[R(0x28, Wing)],
        0x019d_37b0 => &[R(0x28, WingPool)],
        0x019d_34f0 => &[R(0x40, Group), A(0x28, Wing)],
        0x019d_3730 | 0x01c3_7dd0 | 0x01f6_b8f0 | 0x01f6_ba00 => &[R(0x28, CountryReference)],
        0x019d_3580 => &[R(0x28, AirGroup)],
        0x01b1_3e70 => &[R(0x28, TheatreGroup), A(0x30, Group)],
        0x01b1_3f50 => &[R(0x28, TheatreGroup)],
        0x01b1_3fc0 => &[R(0x28, NavalTheatreGroup)],
        0x01b1_4030 => &[Country(0x28)],
        0x01bc_5200 => &[R(0x28, Raid)],
        0x01c3_1250 | 0x01c3_13d0 | 0x01c3_12c0 => &[R(0x28, Conveyor)],
        0x01c3_1360 => &[R(0x28, ConveyorLine)],
        0x00f0_1d50 | 0x00f0_1da0 => &[A(0x28, Unit)],
        0x00f0_1db0 => &[R(0x28, Unit)],
        _ => return Target::Unknown,
    };
    Target::Sources(sources)
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

fn family(name: &str) -> Option<Object> {
    let name = super::class_hash(name);
    FAMILIES
        .binary_search_by_key(&name, |&(hash, _)| hash)
        .ok()
        .map(|index| FAMILIES[index].1)
}

pub(super) fn objects(image: &PeImage) -> io::Result<Vec<(u32, u32)>> {
    let mut result = Vec::new();
    for (section, bytes) in image.data_sections() {
        for offset in (0..bytes.len().saturating_sub(16)).step_by(8) {
            let pointer = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
            let Ok(locator) = image.va_to_rva(pointer) else {
                continue;
            };
            if image.u32(locator).ok() != Some(1)
                || image.u32(locator + 4).ok() != Some(0)
                || image.u32(locator + 20).ok() != Some(locator)
            {
                continue;
            }
            let method = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            if !image.va_to_rva(method).is_ok_and(|rva| {
                image
                    .executable_ranges()
                    .iter()
                    .any(|&(start, end)| (start..end).contains(&rva))
            }) {
                continue;
            }
            let hierarchy = image.u32(locator + 16)?;
            let count = image.u32(hierarchy + 8)?;
            let bases = image.u32(hierarchy + 12)?;
            image.bytes(bases, count as usize * 4)?;
            let mut mask = 0;
            for index in 0..count {
                let descriptor = image.u32(bases + index * 4)?;
                if image.u32(descriptor + 8)? != 0 {
                    continue;
                }
                let name = image.u32(descriptor)?;
                if let Some(object) = family(image.string(name + 16)?) {
                    mask |= object.mask();
                }
            }
            if mask != 0 {
                result.push((section + offset as u32 + 8, mask));
            }
        }
    }
    result.sort_unstable();
    result.dedup();
    if result.is_empty() {
        return Err(invalid("command operand identities missing"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theatre_group_assignment_alone_falls_back_to_orders_group_owners() {
        assert!(matches!(
            target(0x01b1_3e70),
            Target::Sources([
                Source::Reference(0x28, Object::TheatreGroup),
                Source::References(0x30, Object::Group),
            ])
        ));
        assert!(matches!(
            target(0x01b1_3f50),
            Target::Sources([Source::Reference(0x28, Object::TheatreGroup)])
        ));
    }
}
