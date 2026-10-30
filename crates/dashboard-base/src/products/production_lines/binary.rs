use super::*;
use crate::products::equipment_icon::Probe;
use product_codec::{Decoder, Product as BinaryProduct, Schema};
product_codec::product! { impl RawRun {
    first_hour: i32,
    last_hour: i32,
    starts_segment: bool,
    variant: u64,
    mio: u64,
    active_factories: i32,
    requested_factories: i32,
    max_factories: i32,
    cost_raw: i64,
    speed_raw: i64,
    efficiency_raw: i64,
    priority: i32,
    amount: i32,
} default {} }
product_codec::product! { impl RawPart {
    variant: u64,
    mio: u64,
    units: i64,
    exact: bool,
} default {} }
product_codec::product! { impl RawOutput {
    first_hour: i32,
    last_hour: i32,
    parts: Vec<RawPart>,
    since_hour: i32,
    whole_life: bool,
} default {} }
product_codec::product! { impl ScannedLine {
    key: String,
    handle: u64,
    kind: Option<LineKind>,
    runs: Vec<RawRun>,
    output: Vec<RawOutput>,
    final_output: Option<RawOutput>,
    ended_by: Option<i32>,
    before_recording: bool,
    between_reads: Option<crate::products::country_groups::HourSpan>,
    variants: Vec<u64>,
} default {} }
product_codec::product! { impl OrganisationState {
    read_hour: Option<i32>,
    size: i32,
    funds_raw: String,
    unspent_points: i32,
    base_task_capacity: i32,
    active_tasks: u32,
    task_slots: u32,
    unlocked: Vec<String>,
    planned: Vec<String>,
    queue_index: i32,
    policy_token: Option<String>,
    policy_name: Option<String>,
    cooldown_hour: Option<i32>,
    auto_update_designs: bool,
} default {} }
product_codec::product! { impl RawOrganisation {
    first_hour: i32,
    token: String,
    name_key: String,
    icon: String,
    owner_tag: Option<String>,
    owner_unknown: bool,
    state: Option<OrganisationState>,
} default {} }
product_codec::product! { impl DesignRead {
    first_hour: i32,
    read_hour: Option<i32>,
    author: Option<u64>,
    trait_count: Option<i32>,
    bonus_count: Option<u32>,
} default {} }
product_codec::product! { impl crate::products::country_groups::HourSpan { first_hour:i32, last_hour:i32 } default {} }
impl BinaryProduct for LineKind {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let tag = match self {
            Self::Military => 0,
            Self::Naval => 1,
            Self::ShipRefit => 2,
            Self::RailwayGun => 3,
        };
        BinaryProduct::encode(&(tag as u8), out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Military),
            1 => Ok(Self::Naval),
            2 => Ok(Self::ShipRefit),
            3 => Ok(Self::RailwayGun),
            _ => Err(crate::invalid("Unknown binary production line kind")),
        }
    }
    fn schema(s: &mut Schema) {
        s.field("production-line-kind/v1");
    }
}
impl BinaryProduct for Probe {
    const MIN_BYTES: usize = 9;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let (tag, value) = match self {
            Self::Sprite(v) => (0_u8, v),
            Self::File(v) => (1, v),
            Self::Existing(v) => (2, v),
        };
        tag.encode(out)?;
        value.encode(out)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let tag = input.u8()?;
        let value = String::decode(input)?;
        match tag {
            0 => Ok(Self::Sprite(value)),
            1 => Ok(Self::File(value)),
            2 => Ok(Self::Existing(value)),
            _ => Err(crate::invalid("Unknown binary production icon probe")),
        }
    }
    fn schema(s: &mut Schema) {
        s.field("production-icon-probe/v1");
    }
}
product_codec::product! { impl IconChoice { probes:Vec<Probe>, otherwise:Result<(),String> } default {} }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_records_keep_every_value_they_are_read_with() {
        let output = RawOutput {
            first_hour: -4,
            last_hour: 7,
            parts: vec![RawPart {
                variant: u64::MAX,
                mio: 4,
                units: i64::MAX,
                exact: false,
            }],
            since_hour: -7,
            whole_life: false,
        };
        let organisation = RawOrganisation {
            first_hour: 1,
            token: "mio".into(),
            name_key: "name".into(),
            icon: "sprite".into(),
            owner_tag: None,
            owner_unknown: true,
            state: Some(OrganisationState {
                read_hour: Some(4),
                size: -2,
                funds_raw: "9007199254740993".into(),
                unspent_points: 3,
                base_task_capacity: 4,
                active_tasks: u32::MAX,
                task_slots: 6,
                unlocked: vec!["a".into()],
                planned: vec!["b".into()],
                queue_index: -1,
                policy_token: Some("p".into()),
                policy_name: None,
                cooldown_hour: Some(-2),
                auto_update_designs: true,
            }),
        };
        let icon = IconChoice {
            probes: vec![
                Probe::Sprite("sprite".into()),
                Probe::File("texture".into()),
                Probe::Existing("pool".into()),
            ],
            otherwise: Err("recorded refusal".into()),
        };
        let design = DesignRead {
            first_hour: -1,
            read_hour: Some(2),
            author: Some(7),
            trait_count: Some(-1),
            bonus_count: Some(u32::MAX),
        };
        let value = (
            output.clone(),
            organisation.clone(),
            icon.clone(),
            design,
            Some(LineKind::ShipRefit),
        );
        let bytes = product_codec::encode(&value).unwrap();
        let decoded: (
            RawOutput,
            RawOrganisation,
            IconChoice,
            DesignRead,
            Option<LineKind>,
        ) = product_codec::decode(&bytes).unwrap();
        assert_eq!(decoded.0, output);
        assert_eq!(decoded.1, organisation);
        assert_eq!(decoded.2, icon);
        assert_eq!(decoded.3, design);
        assert_eq!(decoded.4, Some(LineKind::ShipRefit));
        for end in 0..bytes.len() {
            assert!(
                product_codec::decode::<(
                    RawOutput,
                    RawOrganisation,
                    IconChoice,
                    DesignRead,
                    Option<LineKind>,
                )>(&bytes[..end])
                .is_err()
            );
        }
    }
}
