pub mod buildings;
pub mod copy_index;
pub mod dirty;
pub mod identities;
pub mod landscape;
pub mod province_field;
pub mod retained;
pub mod value_index;
use product_codec::{Product, product};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};

mod paged;
pub use paged::{PAGE_ITEMS, Paged};

pub const CONTRACT: &str = "map-display/fog-weather/v1";

product! {
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Identity { pub digest: [u8; 32], pub bytes: u64 }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Availability {
        0 => Known {},
        1 => KnownEmpty {},
        2 => Unknown { reason: String, evidence_hour: i32, scope: String, continuity: String },
        3 => NotRecorded { component: String, capture_contract: String },
        4 => Unavailable { reason: String, extent: String },
        5 => Refused { construct: String, reason: String, order: u64 }
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Evidence {
        pub component: String, pub supplying_hour: Option<i32>, pub status: Availability
    }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Header { pub viewing: Option<String> }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Music { pub weights: Vec<Option<f64>>, pub refusals: Vec<String> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Battle {
        pub key: u64, pub provinces: Vec<u32>, pub center: [f32; 2], pub radius: f32,
        pub divisions: usize, pub sub_units: BTreeMap<String, usize>
    }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Battles { pub battles: Vec<Battle>, pub refusals: Vec<String> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Impact {
        pub id: u64, pub province: u32, pub effect: u8,
        pub position: [f32; 3], pub bounds: [f32; 4]
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Popup {
        pub sequence: u64, pub family: u8, pub province: u32, pub style: u8, pub text: Vec<u8>
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum PopupResult { 0 => Value { popup: Option<Popup> }, 1 => Refused { reason: String } }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ImpactResult { 0 => Value { impact: Impact }, 1 => Refused { reason: String } }
    #[derive(Clone, Debug, PartialEq)]
    pub struct MapEvent {
        pub sequence: u64, pub family: u8, pub popup: PopupResult,
        pub explosion: Option<ImpactResult>
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Interval {
        pub branch: u64, pub epoch: u64, pub from_hour: i32, pub to_hour: i32,
        pub first: u64, pub next: u64, pub in_flight: bool, pub coverage: Vec<bool>,
        pub coverage_refusals: Vec<String>, pub events: Vec<MapEvent>
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum MapTextResult { 0 => Value { interval: Interval }, 1 => Unavailable { reason: String } }
    #[derive(Clone, Debug, PartialEq)]
    pub struct MapText { pub view: u32, pub interval: MapTextResult }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Fog {
        pub intel: Arc<Intel>, pub limitations: Vec<String>, pub header: Header,
        pub map_text: MapText, pub music: Option<Music>, pub battles: Option<Battles>
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum LayerResult {
        0 => Value { payload: Vec<u8> },
        1 => Failed { kind: String, display: String }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Snapshot {
        pub branch: u64, pub hour: i32, pub selector: String, pub content: [u8; 32],
        pub evidence: Vec<Evidence>, pub result: LayerResult
    }
}

pub fn identity<T: Product>(value: &T) -> io::Result<Identity> {
    let bytes = product_codec::encode(value)?;
    Ok(Identity {
        digest: {
            let mut digest = Sha256::new();
            digest.update(product_codec::schema_digest::<T>());
            digest.update(&bytes);
            digest.finalize().into()
        },
        bytes: u64::try_from(bytes.len())
            .map_err(|_| product_codec::invalid("display length exceeds u64"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_preserves_float_bits_unknowns_and_rejects_trailing_bytes() {
        let value = Music {
            weights: vec![None, Some(f64::from_bits(0x7ff8000000000123)), Some(-0.0)],
            refusals: vec!["synthetic refusal".into()],
        };
        let mut bytes = product_codec::encode(&value).unwrap();
        let restored: Music = product_codec::decode(&bytes).unwrap();
        assert_eq!(product_codec::encode(&restored).unwrap(), bytes);
        assert_ne!(
            identity(&Intel::new(vec![0], false)).unwrap(),
            identity(&Intel::new(vec![0], true)).unwrap()
        );
        bytes.push(0);
        assert!(product_codec::decode::<Music>(&bytes).is_err());
        assert!(product_codec::decode::<Availability>(&[255]).is_err());
    }

}

#[derive(Clone, Debug)]
pub struct SnowMud {
    pub width: u32,
    pub height: u32,
    pub texels: Paged<u8>,
    pub unknown: Paged<u8>,
    pub province_snow: Paged<Option<i64>>,
    pub post_factors: Option<winter::SnowFactors>,
    pub build: Duration,
    pub summary: Summary,
    pub refresh: Option<Refresh>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub interval: i64,
    pub identity: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub provinces: usize,
    pub complete: bool,
    pub with_snow: usize,
    pub shown_snow: usize,
    pub shown_mud: usize,
    pub missing_edges: usize,
}

product_codec::product! {
    impl Refresh { interval: i64, identity: u64 } default {}
    impl Summary {
        provinces: usize, with_snow: usize, shown_snow: usize,
        shown_mud: usize, missing_edges: usize, complete: bool,
    } default {}

}

impl Summary {
    pub fn line(&self) -> String {
        format!(
            "{} provinces{}, {} with snow, {} drawn snowed, {} drawn muddy{}",
            self.provinces,
            if self.complete { "" } else { " (incomplete)" },
            self.with_snow,
            self.shown_snow,
            self.shown_mud,
            if self.missing_edges == 0 {
                String::new()
            } else {
                format!(
                    ", {} retained snow/mud edge cells unavailable",
                    self.missing_edges
                )
            },
        )
    }
}

impl PartialEq for SnowMud {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.texels == other.texels
            && self.unknown == other.unknown
            && self.province_snow == other.province_snow
            && self.summary == other.summary
            && self.refresh == other.refresh
            && match (&self.post_factors, &other.post_factors) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
    }
}
impl Eq for SnowMud {}

pub type Snow = SnowMud;

impl Intel {
    pub fn new(texels: Vec<u8>, fog: bool) -> Self {
        let fogged_texels = texels.iter().filter(|&&byte| byte > 0).count();
        Self {
            texels: texels.into(),
            fog,
            fogged_texels,
        }
    }
    pub fn fogged_texels(&self) -> usize {
        self.fogged_texels
    }
}

pub mod winter {
    use super::*;
    #[derive(Clone, Debug, Default)]
    pub struct SnowFactors(pub(super) Paged<Option<f32>>);
    fn same(a: &Option<f32>, b: &Option<f32>) -> bool {
        a.map(f32::to_bits) == b.map(f32::to_bits)
    }
    impl SnowFactors {
        pub fn try_from_iter(
            len: usize,
            values: impl IntoIterator<Item = Option<f32>>,
        ) -> io::Result<Self> {
            Ok(Self(Paged::try_from_iter(len, values)?))
        }
        pub fn try_edited(
            &self,
            edits: impl IntoIterator<Item = io::Result<(usize, Option<f32>)>>,
        ) -> io::Result<Self> {
            Ok(Self(self.0.try_edited_results_by(edits, same)?))
        }
        pub fn ptr_eq(&self, other: &Self) -> bool {
            self.0.ptr_eq(&other.0)
        }
        pub fn changed_pages(&self, previous: &Self) -> Vec<usize> {
            self.0.changed_pages_by(&previous.0, same)
        }
        pub fn changed_page_indices<'a>(
            &'a self,
            previous: &'a Self,
        ) -> impl Iterator<Item = usize> + 'a {
            self.0.changed_page_indices_by(&previous.0, same)
        }
    }
    impl std::ops::Deref for SnowFactors {
        type Target = Paged<Option<f32>>;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl std::ops::DerefMut for SnowFactors {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }
    impl PartialEq for SnowFactors {
        fn eq(&self, other: &Self) -> bool {
            self.0.eq_by(&other.0, same)
        }
    }
    impl Eq for SnowFactors {}
    impl From<Vec<Option<f32>>> for SnowFactors {
        fn from(values: Vec<Option<f32>>) -> Self {
            Self::try_from_iter(values.len(), values).expect("snow factor fixture admission")
        }
    }
    impl From<Arc<[Option<f32>]>> for SnowFactors {
        fn from(values: Arc<[Option<f32>]>) -> Self {
            Self::try_from_iter(values.len(), values.iter().copied())
                .expect("snow factor fixture admission")
        }
    }
    impl FromIterator<Option<f32>> for SnowFactors {
        fn from_iter<I: IntoIterator<Item = Option<f32>>>(values: I) -> Self {
            values.into_iter().collect::<Vec<_>>().into()
        }
    }
}
impl Product for winter::SnowFactors {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.0.encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        Ok(Self(Paged::decode(input)?))
    }
    fn schema(schema: &mut product_codec::Schema) {
        Arc::<[Option<f32>]>::schema(schema);
    }
}

#[cfg(test)]
mod snow_wire_oracle {
    use super::*;
    #[test]
    fn factor_edits_copy_only_dirty_pages_and_preserve_bits_and_rollback() {
        let base = winter::SnowFactors(
            Paged::try_from_iter(
                PAGE_ITEMS * 3,
                std::iter::repeat_n(Some(0.0), PAGE_ITEMS * 3),
            )
            .unwrap(),
        );
        let changed = base.try_edited([Ok((PAGE_ITEMS * 2, Some(-0.0)))]).unwrap();
        assert_eq!(changed.changed_pages(&base), [2]);
        assert_ne!(base, changed);
        assert_eq!(base[PAGE_ITEMS * 2].unwrap().to_bits(), 0.0f32.to_bits());
        assert_eq!(
            changed[PAGE_ITEMS * 2].unwrap().to_bits(),
            (-0.0f32).to_bits()
        );
        let same = changed
            .try_edited([Ok((PAGE_ITEMS * 2, Some(-0.0)))])
            .unwrap();
        assert!(same.ptr_eq(&changed));
        let error = changed
            .try_edited([
                Ok((0, None)),
                Err(io::Error::from(io::ErrorKind::WouldBlock)),
            ])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(changed[0], Some(0.0));
        let nan = f32::from_bits(0x7fc0_1234);
        let with_nan = changed.try_edited([Ok((0, Some(nan)))]).unwrap();
        assert_eq!(with_nan.changed_pages(&changed), [0]);
        let copied_nan =
            winter::SnowFactors::try_from_iter(with_nan.len(), with_nan.iter().copied()).unwrap();
        assert!(with_nan.changed_pages(&copied_nan).is_empty());
        assert!(
            with_nan
                .try_edited([Ok((0, Some(nan)))])
                .unwrap()
                .ptr_eq(&with_nan)
        );
    }
    #[test]
    fn admitted_factors_keep_the_published_arc_slice_schema() {
        assert_eq!(
            product_codec::schema_digest::<winter::SnowFactors>(),
            product_codec::schema_digest::<Arc<[Option<f32>]>>()
        );
    }
    mod legacy {
        use std::{sync::Arc, time::Duration};
        pub mod winter {
            pub type SnowFactors = std::sync::Arc<[Option<f32>]>;
        }
        pub struct Refresh {
            pub interval: i64,
            pub identity: u64,
        }
        pub struct Summary {
            pub provinces: usize,
            pub with_snow: usize,
            pub shown_snow: usize,
            pub shown_mud: usize,
            pub missing_edges: usize,
            pub complete: bool,
        }
        pub struct SnowMud {
            pub width: u32,
            pub height: u32,
            pub texels: Vec<u8>,
            pub unknown: Vec<u8>,
            pub province_snow: Vec<Option<i64>>,
            pub post_factors: Option<Arc<[Option<f32>]>>,
            pub refresh: Option<Refresh>,
            pub summary: Summary,
            pub build: Duration,
        }
        product_codec::product! {
            impl Refresh { interval:i64,identity:u64 } default {}
            impl Summary { provinces:usize,with_snow:usize,shown_snow:usize,shown_mud:usize,missing_edges:usize,complete:bool } default {}
            impl SnowMud { width:u32,height:u32,texels:Vec<u8>,unknown:Vec<u8>,province_snow:Vec<Option<i64>>,post_factors:Option<winter::SnowFactors>,refresh:Option<Refresh>,summary:Summary } default { build:Duration::ZERO }
        }
    }
    #[test]
    fn moved_snow_preserves_the_existing_raw_bytes_and_schema_digest() {
        let factors: Arc<[Option<f32>]> =
            vec![None, Some(-0.0), Some(f32::from_bits(0x7fc00123))].into();
        let old = legacy::SnowMud {
            width: 1,
            height: 1,
            texels: vec![1, 2, 3, 4],
            unknown: vec![1],
            province_snow: vec![None, Some(i64::MIN)],
            post_factors: Some(Arc::clone(&factors)),
            refresh: Some(legacy::Refresh {
                interval: -7,
                identity: u64::MAX,
            }),
            summary: legacy::Summary {
                provinces: 2,
                with_snow: 1,
                shown_snow: 1,
                shown_mud: 1,
                missing_edges: 3,
                complete: false,
            },
            build: Duration::from_secs(5),
        };
        let new = SnowMud {
            width: 1,
            height: 1,
            texels: vec![1, 2, 3, 4].into(),
            unknown: vec![1].into(),
            province_snow: vec![None, Some(i64::MIN)].into(),
            post_factors: Some(factors.into()),
            refresh: Some(Refresh {
                interval: -7,
                identity: u64::MAX,
            }),
            summary: Summary {
                provinces: 2,
                with_snow: 1,
                shown_snow: 1,
                shown_mud: 1,
                missing_edges: 3,
                complete: false,
            },
            build: Duration::from_secs(9),
        };
        assert_ne!(old.build, new.build);
        let bytes = product_codec::encode(&old).unwrap();
        assert_eq!(product_codec::encode(&new).unwrap(), bytes);
        assert_eq!(
            product_codec::schema_digest::<legacy::SnowMud>(),
            product_codec::schema_digest::<SnowMud>()
        );
        let restored: SnowMud = product_codec::decode(&bytes).unwrap();
        assert_eq!(restored, new);
        assert_eq!(restored.build, Duration::ZERO);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intel {
    pub texels: Paged<u8>,
    pub fog: bool,
    pub fogged_texels: usize,
}

impl Product for Intel {
    const MIN_BYTES: usize = 17;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.texels.encode(out)?;
        self.fog.encode(out)?;
        self.fogged_texels.encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            texels: Paged::decode(input)?,
            fog: bool::decode(input)?,
            fogged_texels: usize::decode(input)?,
        })
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("struct");
        schema.field("Intel");
        schema.field("texels");
        schema.field("Vec<u8>");
        Vec::<u8>::schema(schema);
        schema.field("fog");
        schema.field("bool");
        bool::schema(schema);
        schema.field("fogged_texels");
        schema.field("usize");
        usize::schema(schema);
        schema.field("end");
    }
}
impl Product for SnowMud {
    const MIN_BYTES: usize = 75;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.width.encode(out)?;
        self.height.encode(out)?;
        self.texels.encode(out)?;
        self.unknown.encode(out)?;
        self.province_snow.encode(out)?;
        self.post_factors.encode(out)?;
        self.refresh.encode(out)?;
        self.summary.encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        Ok(Self {
            width: u32::decode(input)?,
            height: u32::decode(input)?,
            texels: Paged::decode(input)?,
            unknown: Paged::decode(input)?,
            province_snow: Paged::decode(input)?,
            post_factors: Option::decode(input)?,
            refresh: Option::decode(input)?,
            summary: Summary::decode(input)?,
            build: Duration::ZERO,
        })
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("struct");
        schema.field("SnowMud");
        macro_rules! field {
            ($name:ident, $ty:ty) => {
                schema.field(stringify!($name));
                schema.field(stringify!($ty));
                <$ty>::schema(schema);
            };
        }
        field!(width, u32);
        field!(height, u32);
        field!(texels, Vec<u8>);
        field!(unknown, Vec<u8>);
        field!(province_snow, Vec<Option<i64>>);
        field!(post_factors, Option<winter::SnowFactors>);
        field!(refresh, Option<Refresh>);
        field!(summary, Summary);
        schema.field("end");
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherField {
    pub branch: u64,
    pub hour: i32,
    pub kinds: copy_index::CopyIndex<u32, usize>,
}
impl Product for WeatherField {
    const MIN_BYTES: usize = 20;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.branch.encode(out)?;
        self.hour.encode(out)?;
        product_codec::put_count64(out, self.kinds.len())?;
        for (key, value) in self.kinds.iter() {
            key.encode(out)?;
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        let branch = u64::decode(input)?;
        let hour = i32::decode(input)?;
        let count = input.count64(u32::MIN_BYTES + usize::MIN_BYTES)?;
        let mut kinds = copy_index::CopyIndex::default();
        let mut prior = None;
        for _ in 0..count {
            let key = u32::decode(input)?;
            if prior.is_some_and(|previous| previous >= key) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "product map keys are not strictly ordered",
                ));
            }
            kinds = kinds.inserted(key, usize::decode(input)?)?;
            prior = Some(key);
        }
        Ok(Self {
            branch,
            hour,
            kinds,
        })
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("struct");
        schema.field("WeatherField");
        macro_rules! field {
            ($name:ident, $ty:ty) => {
                schema.field(stringify!($name));
                schema.field(stringify!($ty));
                <$ty>::schema(schema);
            };
        }
        field!(branch, u64);
        field!(hour, i32);
        field!(kinds, BTreeMap<u32, usize>);
        schema.field("end");
    }
}

#[cfg(test)]
mod fog_weather_wire_oracles {
    use super::*;
    mod legacy {
        use super::*;
        product! {
            pub struct Intel { pub texels: Vec<u8>, pub fog: bool, pub fogged_texels: usize }
            pub struct WeatherField { pub branch: u64, pub hour: i32, pub kinds: BTreeMap<u32, usize> }
        }
    }
    #[test]
    fn shared_display_storage_preserves_frozen_bytes_and_schema() {
        let old = legacy::Intel {
            texels: vec![0, 1, 255],
            fog: true,
            fogged_texels: 2,
        };
        let new = Intel::new(old.texels.clone(), old.fog);
        assert_eq!(
            product_codec::encode(&old).unwrap(),
            product_codec::encode(&new).unwrap()
        );
        assert_eq!(
            product_codec::schema_digest::<legacy::Intel>(),
            product_codec::schema_digest::<Intel>()
        );
        let old = legacy::WeatherField {
            branch: u64::MAX,
            hour: i32::MIN,
            kinds: [(0, 4), (u32::MAX, 1)].into(),
        };
        let new = WeatherField {
            branch: old.branch,
            hour: old.hour,
            kinds: old.kinds.clone().into(),
        };
        assert_eq!(
            product_codec::encode(&old).unwrap(),
            product_codec::encode(&new).unwrap()
        );
        assert_eq!(
            product_codec::schema_digest::<legacy::WeatherField>(),
            product_codec::schema_digest::<WeatherField>()
        );
    }
}
