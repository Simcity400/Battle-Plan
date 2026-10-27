pub use product_codec::credit;
pub mod buildings;
pub mod copy_index;
pub mod identities;
pub mod landscape;
pub mod province_field;
pub mod retained;
pub mod value_index;
use product_codec::{Product, product};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};

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
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Intel { pub texels: Vec<u8>, pub fog: bool, pub fogged_texels: usize }
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
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct WeatherField { pub branch: u64, pub hour: i32, pub kinds: BTreeMap<u32, usize> }
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

/// One hour's `SnowMudData` payload.
#[derive(Clone, Debug)]
pub struct SnowMud {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA8 texels, row 0 at world `z = 0`.
    pub texels: Vec<u8>,
    /// Raw winter-grading snow words by province; absent rows remain unknown.
    pub province_snow: Vec<Option<i64>>,
    /// Content-ordered winter factors, including unknown representatives.
    pub post_factors: Option<winter::SnowFactors>,
    /// Wall time of the rasterisation, for reporting.
    pub build: Duration,
    pub summary: Summary,
    pub refresh: Option<Refresh>,
}

impl SnowMud {
    pub fn owned_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.texels.capacity()
            + self.province_snow.capacity() * std::mem::size_of::<Option<i64>>()
            + self.post_factors.as_ref().map_or(0, |values| {
                std::mem::size_of_val(&**values) + 2 * std::mem::size_of::<usize>()
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refresh {
    pub interval: i64,
    pub identity: u64,
}

/// What the hour's weather table held, for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Recorded `WeatherProvinces` rows.
    pub provinces: usize,
    /// Whether the table is whole; missing provinces draw bare ground.
    pub complete: bool,
    /// Rows whose native snow word is above zero.
    pub with_snow: usize,
    /// Rows with a nonzero snow display byte.
    pub shown_snow: usize,
    /// Rows with a nonzero mud display byte.
    pub shown_mud: usize,
    /// Retained native edge cells unavailable in this sample.
    pub missing_edges: usize,
}

product_codec::product! {
    impl Refresh { interval: i64, identity: u64 } default {}
    impl Summary {
        provinces: usize, with_snow: usize, shown_snow: usize,
        shown_mud: usize, missing_edges: usize, complete: bool,
    } default {}
    impl SnowMud {
        width: u32, height: u32, texels: Vec<u8>, province_snow: Vec<Option<i64>>,
        post_factors: Option<winter::SnowFactors>, refresh: Option<Refresh>, summary: Summary,
    } default { build: Duration::ZERO }
}

impl Summary {
    /// One line for the status bar and the headless dump.
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
            }
        )
    }
}

impl PartialEq for SnowMud {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.texels == other.texels
            && self.province_snow == other.province_snow
            && self.summary == other.summary
            && self.refresh == other.refresh
            && match (&self.post_factors, &other.post_factors) {
                (None, None) => true,
                (Some(a), Some(b)) => {
                    a.len() == b.len()
                        && a.iter()
                            .zip(b.iter())
                            .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits))
                }
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
            texels,
            fog,
            fogged_texels,
        }
    }
    pub fn fogged_texels(&self) -> usize {
        self.fogged_texels
    }
}

pub mod winter {
    pub type SnowFactors = std::sync::Arc<[Option<f32>]>;
}

#[cfg(test)]
mod snow_wire_oracle {
    use super::*;
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
            pub province_snow: Vec<Option<i64>>,
            pub post_factors: Option<Arc<[Option<f32>]>>,
            pub refresh: Option<Refresh>,
            pub summary: Summary,
            pub build: Duration,
        }
        product_codec::product! {
            impl Refresh { interval:i64,identity:u64 } default {}
            impl Summary { provinces:usize,with_snow:usize,shown_snow:usize,shown_mud:usize,missing_edges:usize,complete:bool } default {}
            impl SnowMud { width:u32,height:u32,texels:Vec<u8>,province_snow:Vec<Option<i64>>,post_factors:Option<winter::SnowFactors>,refresh:Option<Refresh>,summary:Summary } default { build:Duration::ZERO }
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
            texels: vec![1, 2, 3, 4],
            province_snow: vec![None, Some(i64::MIN)],
            post_factors: Some(factors),
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
