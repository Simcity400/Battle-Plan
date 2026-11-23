use super::*;

pub const EQUIPMENT_DESTRUCTION_PARTIAL_REASON: &str =
    "equipment destruction first/branch interval is partial";
pub const EQUIPMENT_DESTRUCTION_LEGACY_PARTIAL_REASON: &str =
    "equipment destruction first/branch interval is partial";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum EquipmentDestructionCoverage {
    #[default]
    Complete = 0,
    SupplementalUnavailable = 1,
    PartialInterval = 2,
}

impl NativeValue for EquipmentDestructionCoverage {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(0) => Ok(Self::Complete),
            Field::U8(1) => Ok(Self::SupplementalUnavailable),
            Field::U8(2) => Ok(Self::PartialInterval),
            _ => Err(invalid("unsupported equipment destruction coverage")),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum DivisionDeletionDisposition {
    #[default]
    Loss = 0,
    Returned = 1,
    Merged = 2,
    HqTransferred = 3,
    VolunteerTransferred = 4,
    ExileTransferred = 5,
}

impl DivisionDeletionDisposition {
    pub const fn is_temporary_transfer(self) -> bool {
        matches!(self, Self::VolunteerTransferred | Self::ExileTransferred)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loss => "loss",
            Self::Returned => "returned",
            Self::Merged => "merged",
            Self::HqTransferred => "hq_transferred",
            Self::VolunteerTransferred => "volunteer_transferred",
            Self::ExileTransferred => "exile_transferred",
        }
    }
}

impl NativeValue for DivisionDeletionDisposition {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(0) => Ok(Self::Loss),
            Field::U8(1) => Ok(Self::Returned),
            Field::U8(2) => Ok(Self::Merged),
            Field::U8(3) => Ok(Self::HqTransferred),
            Field::U8(4) => Ok(Self::VolunteerTransferred),
            Field::U8(5) => Ok(Self::ExileTransferred),
            Field::U8(_) => Err(invalid("unsupported division deletion disposition")),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentDeletionRow {
    pub division_handle: u64,
    pub country: u32,
    pub expeditionary_owner: u32,
    pub disposition: DivisionDeletionDisposition,
    pub cause: Option<EquipmentDestructionCause>,
}

impl Default for DivisionEquipmentDeletionRow {
    fn default() -> Self {
        Self {
            division_handle: 0,
            country: 0,
            expeditionary_owner: 0,
            disposition: DivisionDeletionDisposition::Loss,
            cause: Some(EquipmentDestructionCause::Unknown),
        }
    }
}

impl DivisionEquipmentDeletionRow {
    pub const fn valid_disposition(&self) -> bool {
        match (self.disposition, self.cause) {
            (DivisionDeletionDisposition::Loss, Some(cause)) => cause.is_division_deletion(),
            (DivisionDeletionDisposition::Loss, None) => false,
            (_, None) => true,
            (_, Some(_)) => false,
        }
    }

    pub const fn event_name(&self) -> &'static str {
        match self.cause {
            Some(cause) => cause.as_str(),
            None => self.disposition.as_str(),
        }
    }
}
typed_codec!(DivisionEquipmentDeletionRow, 64,
|row| row.division_handle as u128,
|key| Self { division_handle: key as u64, ..Default::default() },
{ 0 => (country): U32, 1 => (expeditionary_owner): U32, 2 => (disposition): U8, 3 => (cause): U8 }, |row| {
    if row.division_handle >> 32 == 0 {
        Err(invalid("deleted division handle has a null id"))
    } else if !row.valid_disposition() {
        Err(invalid("invalid division deletion disposition"))
    } else {
        Ok(())
    }
});

replay_rows::native!(EquipmentDestructionCoverage);

replay_rows::native!(DivisionDeletionDisposition);
