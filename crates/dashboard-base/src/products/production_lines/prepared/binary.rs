use super::*;
use product_codec::{Decoder, Product as BinaryProduct, Schema};
product_codec::product! { impl Carry {
    first: i32,
    last: i32,
    modifiers: bool,
    output: bool,
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
product_codec::product! { impl Handle {
    lifetime: u64,
    appearance: u32,
} default {} }
product_codec::product! { impl LineLedger {
    lifetime: u64,
    appearance: u32,
    run: Option<(u64, RawRun)>,
    step: Option<(u64, u64, i32, i32)>,
    output: Option<(u64, RawOutput)>,
    modifier: Option<i32>,
    operands: Option<Vec<u8>>,
} default {} }
product_codec::product! { impl CoreLedger {
    last: Option<((u32, u32), i32)>,
    open: Vec<(u64, LineLedger)>,
    carried: bool,
    broken: Vec<(u64, LineLedger)>,
    last_read_identity: Option<(u32, u32)>,
    broken_identity: Option<(u32, u32)>,
    first_read: bool,
    output_open: bool,
    organisations_open: bool,
    last_census: Option<i32>,
    active_variants: BTreeSet<u64>,
    carry: Option<Carry>,
    last_output_due: Option<i32>,
    last_modifier_country: Option<Vec<u8>>,
    last_design_revision: Option<u64>,
    counters: index::Counters,
    stream: Option<(u64, Vec<HourSpan>)>,
    ids: (u64, u64, u64, u64),
    through: Option<i32>,
    overview: Option<u64>,
} default {} }
product_codec::product! { impl KeysLedger {
    root: Option<keys::NodeRef>,
    handles: Vec<(u64, Handle)>,
    designs: Vec<(u64, (DesignRead, u64))>,
    organisations: Vec<(u64, (RawOrganisation, u64))>,
    icons: Vec<(u64, IconChoice)>,
} default {} }
product_codec::product! { impl HistoryLedger {
    parent: Option<(u64, u64)>,
    sleep: Option<Sleep>,
    full: bool,
    state: StateLedger,
} default {} }
product_codec::product! { impl LedgerRecord {
    full: bool,
    chunks: u64,
    through: Option<i32>,
    flushed_through: Option<i32>,
    next_history: u64,
    world: input::WorldSaved,
    absent: u64,
    active: Vec<((u32, u32), NativeCountry)>,
    links: Vec<((u32, u32), NativeCountry)>,
    epochs: Vec<((u32, u32), u64)>,
    natives: Vec<(NativeCountry, u64)>,
    retired: Vec<(NativeCountry, u64)>,
    aliases: Vec<(String, TagLedger)>,
    dormant: Vec<(String, TagLedger)>,
    spans: Vec<Alias>,
    histories: Vec<(u64, HistoryLedger)>,
} default {} }
impl BinaryProduct for TagLedger {
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
            2 => Ok(Self::Projected(u64::decode(input)?)),
            _ => Err(crate::invalid("Unknown binary production tag ledger")),
        }
    }
    fn schema(s: &mut Schema) {
        s.field("production-tag-ledger/v1");
        NativeCountry::schema(s);
    }
}
