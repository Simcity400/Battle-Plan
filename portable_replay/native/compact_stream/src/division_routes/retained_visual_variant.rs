//! D2RB optional retained byte and custody of the existing owner projection.
use super::{
    word, CombatMembershipCause as Cause, CombatMembershipStage as Stage,
    RetainedVisualVariantField as Field, ROW_LEN,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetainedVisualVariantRefusal {
    pub cause: Cause,
    pub field: Field,
    pub stage: Stage,
}

impl Default for RetainedVisualVariantRefusal {
    fn default() -> Self {
        Self {
            cause: Cause::NotObserved,
            field: Field::FrameRoot,
            stage: Stage::Admission,
        }
    }
}

impl RetainedVisualVariantRefusal {
    pub fn validate(self) -> Result<(), String> {
        if self.cause == Cause::NotObserved && self != Self::default() {
            return Err("noncanonical retained visual not-observed refusal".into());
        }
        Ok(())
    }

    fn packed(self) -> Result<u32, String> {
        self.validate()?;
        Ok(self.cause.code() | self.field.code() << 8 | self.stage.code() << 16)
    }

    fn unpack(raw: u32) -> Result<Self, String> {
        if raw >> 24 != 0 {
            return Err("retained visual refusal has reserved bits".into());
        }
        let result = Self {
            cause: Cause::from_raw(raw & 255)?,
            field: Field::from_raw((raw >> 8) & 255)?,
            stage: Stage::from_raw((raw >> 16) & 255)?,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn json(self) -> String {
        format!(
            "{{\"cause\":\"{}\",\"field\":\"{}\",\"stage\":\"{}\"}}",
            self.cause.name(),
            self.field.name(),
            self.stage.name()
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedVisualVariantObservation {
    Measured(u8),
    Unavailable(RetainedVisualVariantRefusal),
}

impl Default for RetainedVisualVariantObservation {
    fn default() -> Self {
        Self::Unavailable(Default::default())
    }
}

impl RetainedVisualVariantObservation {
    pub fn json(self) -> String {
        match self {
            Self::Measured(raw) => {
                format!("{{\"schema\":1,\"status\":\"measured\",\"raw_u8\":{raw}}}")
            }
            Self::Unavailable(reason) => format!(
                "{{\"schema\":1,\"status\":\"unavailable\",\"refusal\":{}}}",
                reason.json()
            ),
        }
    }
}

/// Witnessed qualifies the owner already present at fixed-row offset 56. It
/// does not assert the independently established admitted-CArmy receiver/self
/// identity at the native selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivisionOwnerAuthority {
    Witnessed,
    Unavailable(RetainedVisualVariantRefusal),
}

impl Default for DivisionOwnerAuthority {
    fn default() -> Self {
        Self::Unavailable(Default::default())
    }
}

impl DivisionOwnerAuthority {
    pub fn json(self) -> String {
        match self {
            Self::Witnessed => "{\"schema\":1,\"status\":\"witnessed\"}".into(),
            Self::Unavailable(reason) => format!(
                "{{\"schema\":1,\"status\":\"unavailable\",\"refusal\":{}}}",
                reason.json()
            ),
        }
    }
}

pub fn write_retained_visual_variant(
    payload: &mut [u8],
    index: usize,
    value: RetainedVisualVariantObservation,
    owner: DivisionOwnerAuthority,
) {
    let variant = match value {
        RetainedVisualVariantObservation::Measured(raw) => (1u32, u32::from(raw)),
        RetainedVisualVariantObservation::Unavailable(reason) => {
            (0, reason.packed().expect("valid retained visual refusal"))
        }
    };
    let owner = match owner {
        DivisionOwnerAuthority::Witnessed => (1u32, 0u32),
        DivisionOwnerAuthority::Unavailable(reason) => {
            (0, reason.packed().expect("valid owner refusal"))
        }
    };
    let base = 32 + index * ROW_LEN;
    for (offset, (status, aux)) in [(312, variant), (320, owner)] {
        payload[base + offset..base + offset + 4].copy_from_slice(&status.to_le_bytes());
        payload[base + offset + 4..base + offset + 8].copy_from_slice(&aux.to_le_bytes());
    }
}

pub(super) fn read(
    payload: &[u8],
    index: usize,
) -> Result<(RetainedVisualVariantObservation, DivisionOwnerAuthority), String> {
    let base = 32 + index * ROW_LEN;
    let value = match (word(payload, base + 312), word(payload, base + 316)) {
        (0, aux) => RetainedVisualVariantObservation::Unavailable(
            RetainedVisualVariantRefusal::unpack(aux)?,
        ),
        (1, raw) if raw <= 255 => RetainedVisualVariantObservation::Measured(raw as u8),
        _ => return Err("invalid retained visual byte cell".into()),
    };
    let owner = match (word(payload, base + 320), word(payload, base + 324)) {
        (0, aux) => DivisionOwnerAuthority::Unavailable(RetainedVisualVariantRefusal::unpack(aux)?),
        (1, 0)
            if word(payload, base + 8) & (1 << 5) != 0
                && word(payload, base + 56) > 0
                && word(payload, base + 56) < i32::MAX as u32 =>
        {
            DivisionOwnerAuthority::Witnessed
        }
        _ => return Err("invalid division owner authority or projection".into()),
    };
    if word(payload, base + 8) & (1 << 12) != 0
        && (value != RetainedVisualVariantObservation::default()
            || owner != DivisionOwnerAuthority::default())
    {
        return Err("deleted division carries retained visual inputs".into());
    }
    Ok((value, owner))
}

#[cfg(test)]
mod tests {
    use super::super::{
        append_fixture_inputs, unavailable_railway_roster, unavailable_roster, DivisionRoutes,
    };
    use super::*;

    fn payload() -> Vec<u8> {
        let mut bytes = vec![0; 32 + ROW_LEN];
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
        bytes[40..44].copy_from_slice(&(1u32 << 5).to_le_bytes());
        bytes[88..92].copy_from_slice(&7u32.to_le_bytes());
        append_fixture_inputs(
            &mut bytes,
            std::iter::once((None, None)),
            std::iter::once(None),
            std::iter::once(None),
            &unavailable_roster(),
            &unavailable_railway_roster(),
        );
        bytes
    }

    #[test]
    fn all_raw_bytes_round_trip_without_signed_normalization_or_owner_duplication() {
        for raw in 0..=255 {
            let mut bytes = payload();
            write_retained_visual_variant(
                &mut bytes,
                0,
                RetainedVisualVariantObservation::Measured(raw),
                DivisionOwnerAuthority::Witnessed,
            );
            assert_eq!(
                DivisionRoutes::parse(&bytes)
                    .unwrap()
                    .retained_visual_variant(0),
                (
                    RetainedVisualVariantObservation::Measured(raw),
                    DivisionOwnerAuthority::Witnessed
                )
            );
            assert_eq!(word(&bytes, 88), 7);
        }
    }

    #[test]
    fn independent_unavailable_cells_preserve_cause_field_stage_and_zero_default() {
        let default = payload();
        assert_eq!(
            DivisionRoutes::parse(&default)
                .unwrap()
                .retained_visual_variant(0),
            (Default::default(), Default::default())
        );
        for field in [Field::RetainedVisualVariant, Field::DivisionOwner] {
            for stage in [Stage::Admission, Stage::Open, Stage::Close] {
                let reason = RetainedVisualVariantRefusal {
                    cause: Cause::Unreadable,
                    field,
                    stage,
                };
                let mut bytes = payload();
                write_retained_visual_variant(
                    &mut bytes,
                    0,
                    RetainedVisualVariantObservation::Unavailable(reason),
                    DivisionOwnerAuthority::Unavailable(reason),
                );
                assert_eq!(
                    DivisionRoutes::parse(&bytes)
                        .unwrap()
                        .retained_visual_variant(0),
                    (
                        RetainedVisualVariantObservation::Unavailable(reason),
                        DivisionOwnerAuthority::Unavailable(reason)
                    )
                );
            }
        }
    }

    #[test]
    fn strict_cells_refuse_unknown_status_width_reserved_bits_and_enum_codes() {
        for (offset, bad) in [
            (312, 2u32),
            (316, 0xff),
            (316, 27 << 8 | 1),
            (316, 3 << 16 | 1),
            (316, 1 << 24),
            (316, 25 << 8),
            (320, 2),
            (324, 0xff),
            (324, 1 << 24),
        ] {
            let mut bytes = payload();
            bytes[32 + offset..36 + offset].copy_from_slice(&bad.to_le_bytes());
            assert!(
                DivisionRoutes::parse(&bytes).is_err(),
                "offset={offset}, bad={bad}"
            );
        }
        let mut bytes = payload();
        write_retained_visual_variant(
            &mut bytes,
            0,
            RetainedVisualVariantObservation::Measured(0),
            DivisionOwnerAuthority::Witnessed,
        );
        for (offset, bad) in [(316, 256u32), (324, 1)] {
            let mut bad_bytes = bytes.clone();
            bad_bytes[32 + offset..36 + offset].copy_from_slice(&bad.to_le_bytes());
            assert!(DivisionRoutes::parse(&bad_bytes).is_err());
        }
    }

    #[test]
    fn witnessed_owner_requires_existing_positive_signed_projection_and_presence() {
        let mut bytes = payload();
        write_retained_visual_variant(
            &mut bytes,
            0,
            Default::default(),
            DivisionOwnerAuthority::Witnessed,
        );
        for owner in [0u32, i32::MAX as u32, u32::MAX] {
            let mut bad = bytes.clone();
            bad[88..92].copy_from_slice(&owner.to_le_bytes());
            assert!(DivisionRoutes::parse(&bad).is_err());
        }
        bytes[40..44].fill(0);
        assert!(DivisionRoutes::parse(&bytes).is_err());
    }

    #[test]
    fn deleted_rows_require_canonical_unobserved_byte_and_owner_cells() {
        let mut bytes = payload();
        bytes[40..44].copy_from_slice(&(1u32 << 12).to_le_bytes());
        bytes[88..92].fill(0);
        assert!(DivisionRoutes::parse(&bytes).is_ok());
        for offset in 312..328 {
            let mut bad = bytes.clone();
            bad[32 + offset] = 1;
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "deleted offset {offset}"
            );
        }
    }

    #[test]
    fn prior_marker_short_rows_and_unowned_trailing_bytes_are_rejected() {
        let bytes = payload();
        let mut old = bytes.clone();
        old[28..32].copy_from_slice(b"D2RA");
        assert!(DivisionRoutes::parse(&old).is_err());
        let mut short = bytes.clone();
        short.drain(32 + 312..32 + ROW_LEN);
        assert!(DivisionRoutes::parse(&short).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(DivisionRoutes::parse(&trailing).is_err());
        for end in 32..32 + ROW_LEN {
            assert!(DivisionRoutes::parse(&bytes[..end]).is_err());
        }
    }
}
