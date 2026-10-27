use super::*;
use product_codec::{Decoder, Product as BinaryProduct, Schema};
product_codec::product! { impl Carry {
    first: i32,
    last: i32,
    modifiers: bool,
    output: bool,
} default {} }
product_codec::product! { impl ClosedLine {
    appearance: u32,
    line: ScannedLine,
} default {} }
product_codec::product! { impl NativeCountry {
    slot: u32,
    id: u32,
    epoch: u64,
} default {} }
product_codec::product! { impl Alias {
    tag: String,
    first_hour: i32,
    last_hour: i32,
    country: NativeCountry,
} default {} }
product_codec::product! { impl Dormant {
    hour: i32,
    head: pages::PageLog,
    reads: usize,
    stretches: usize,
} default {} }
product_codec::product! { impl ActiveCountry {
    native: NativeCountry,
} default {} }
product_codec::product! { impl CountrySnapshot {
    page: pages::Page,
    head: pages::PageLog,
    sleep: Option<Dormant>,
    history: private::Root,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    pending_bytes: usize,
    last_country: Option<(Vec<u8>, u64)>,
    next_country: u64,
    last: Option<((u32, u32), i32)>,
    open: BTreeMap<u64, Open>,
    carried: bool,
    broken: BTreeMap<u64, Open>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    appearances: BTreeMap<u64, u32>,
    latest: BTreeMap<u64, usize>,
    last_output_due: Option<i32>,
    last_design_revision: Option<u64>,
} default {} }
product_codec::product! { impl Continuation {
    world: input::WorldSaved,
    active: Vec<((u32, u32), ActiveCountry)>,
    native: Vec<NativeCountry>,
    aliases: Vec<String>,
} default {} }
product_codec::product! { impl VariantPage {
    catalogue: VariantCatalogue,
    icons: Vec<(u64, i32, IconChoice, bool)>,
} default {} }
impl BinaryProduct for TagSnapshot {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Absent => 0_u8.encode(out),
            Self::Native(value) => {
                1_u8.encode(out)?;
                value.encode(out)
            }
            Self::Projected(value) => {
                2_u8.encode(out)?;
                value.encode(out)
            }
        }
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Absent),
            1 => Ok(Self::Native(NativeCountry::decode(input)?)),
            2 => Ok(Self::Projected(Box::<CountrySnapshot>::decode(input)?)),
            _ => Err(crate::invalid("Unknown binary production tag snapshot")),
        }
    }
    fn schema(s: &mut Schema) {
        s.field("production-tag-snapshot/v1");
        NativeCountry::schema(s);
        CountrySnapshot::schema(s);
    }
}
