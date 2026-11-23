
pub const TOKEN: usize = 0x08;
pub const CACHED_NAME: usize = 0x10;

pub(crate) fn cached_name(
    memory: &impl Memory,
    pointer: usize,
) -> Result<engine_native_memory::msvc_string::Descriptor, &'static str> {
    let address = pointer
        .checked_add(CACHED_NAME)
        .ok_or("peace name address overflow")?;
    engine_native_memory::msvc_string::Descriptor::read(memory, address)
}

use engine_native_memory::{Memory, read};
use crate::cstate_peace_action as state;

pub const TAKER: usize = 0x30;
pub const GIVER: usize = 0x34;
pub const NEGOTIATOR: usize = 0x38;
pub const STATUS: usize = 0x3c;
pub const COST: usize = 0x40;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Form {
    #[default]
    TakeState,
    Liberate,
    Puppet,
    Government,
    Navy,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub token: u32,
    pub taker: u32,
    pub giver: u32,
    pub negotiator: u32,
    pub status: i32,
    pub cost: i32,
    pub form: Form,
    pub state: Option<i32>,
    pub flags: Option<[u8; 4]>,
    pub country_operand: Option<u32>,
    pub puppet_auxiliary: Option<u64>,
    pub ship: Option<u64>,
    pub ratio: Option<i64>,
    pub name_key: Option<[u8; crate::ctake_navy_peace_action::LABEL_LITERAL_BYTES]>,
}

fn kept_in_settlement(status: i32) -> bool {
    !matches!(status, 3 | 4)
}

impl Row {
    pub fn included_in_settlement(self) -> bool {
        kept_in_settlement(self.status)
    }
}

fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(Form, [u8; 12]), &'static str> {
    use crate::{
        cforce_government_action as government, cliberate_country_action as liberate,
        cpuppet_country_action as puppet, ctake_navy_peace_action as navy,
        ctake_state_action as take,
    };
    let header = read::<[u8; 12]>(memory, pointer).ok_or("peace action header unreadable")?;
    let vtable = u64::from_le_bytes(header[..8].try_into().unwrap()) as usize;
    let form = match engine_sites::which(
        base,
        vtable,
        &[
            take::VTABLE_RVA,
            liberate::VTABLE_RVA,
            puppet::VTABLE_RVA,
            government::VTABLE_RVA,
            navy::VTABLE_RVA,
        ],
    ) {
        Some(take::VTABLE_RVA) => Form::TakeState,
        Some(liberate::VTABLE_RVA) => Form::Liberate,
        Some(puppet::VTABLE_RVA) => Form::Puppet,
        Some(government::VTABLE_RVA) => Form::Government,
        Some(navy::VTABLE_RVA) => Form::Navy,
        _ => return Err("peace action vtable unsupported"),
    };
    Ok((form, header))
}

pub fn included_in_settlement(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<bool, &'static str> {
    identity(memory, base, pointer)?;
    let status = pointer
        .checked_add(STATUS)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("peace action status unreadable")?;
    Ok(kept_in_settlement(status))
}

pub fn copy(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    use crate::{
        cforce_government_action as government, cliberate_country_action as liberate,
        cpuppet_country_action as puppet, ctake_navy_peace_action as navy,
    };
    let (form, header) = identity(memory, base, pointer)?;
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("peace action address overflow")
    };
    let end = match form {
        Form::TakeState => state::FLAGS + 4,
        Form::Liberate => liberate::LIBERATE + 4,
        Form::Government => government::COUNTRY + 4,
        Form::Puppet => puppet::AUXILIARY + 8,
        Form::Navy => navy::RATIO + 8,
    };
    let mut bytes = [0_u8; puppet::AUXILIARY + 8 - TAKER];
    if memory.copy_scattered([at(TAKER)?], &mut [&mut bytes[..end - TAKER]]) != 1 {
        return Err("peace action operands unreadable");
    }
    let word = |offset: usize| {
        u32::from_le_bytes(
            bytes[offset - TAKER..offset - TAKER + 4]
                .try_into()
                .unwrap(),
        )
    };
    let wide = |offset: usize| {
        u64::from_le_bytes(
            bytes[offset - TAKER..offset - TAKER + 8]
                .try_into()
                .unwrap(),
        )
    };
    let mut row = Row {
        token: u32::from_le_bytes(header[8..].try_into().unwrap()),
        taker: word(TAKER),
        giver: word(GIVER),
        negotiator: word(NEGOTIATOR),
        status: word(STATUS) as i32,
        cost: word(COST) as i32,
        form,
        ..Default::default()
    };
    if form == Form::Navy {
        let key = engine_sites::at(base, navy::LABEL_LITERAL_RVA)
            .and_then(|at| read::<[u8; navy::LABEL_LITERAL_BYTES]>(memory, at))
            .ok_or("peace navy naming key unreadable")?;
        if !key.contains(&0) {
            return Err("peace navy naming literal has no terminator");
        }
        row.name_key = Some(key);
        let selected = wide(navy::SHIP);
        row.ship = Some(if selected == 0 {
            0
        } else {
            let identity = read::<[u8; 16]>(memory, selected as usize)
                .ok_or("peace selected ship identity unreadable")?;
            if engine_sites::at(base, engine_native::cship::VTABLE_RVA).map(|value| value as u64)
                != Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
            {
                return Err("peace selected ship vtable mismatch");
            }
            let reference = u64::from_le_bytes(identity[8..].try_into().unwrap());
            if reference == 0 {
                return Err("peace selected ship has null reference");
            }
            reference
        });
        row.ratio = Some(wide(navy::RATIO) as i64);
    } else {
        row.state = Some(word(state::STATE) as i32);
        row.flags = Some(
            bytes[state::FLAGS - TAKER..state::FLAGS - TAKER + 4]
                .try_into()
                .unwrap(),
        );
        row.country_operand = match form {
            Form::Liberate => Some(word(liberate::LIBERATE)),
            Form::Puppet => Some(word(puppet::PUPPET)),
            Form::Government => Some(word(government::COUNTRY)),
            _ => None,
        };
        if form == Form::Puppet {
            row.puppet_auxiliary = Some(wide(puppet::AUXILIARY));
        }
    }
    Ok(row)
}
