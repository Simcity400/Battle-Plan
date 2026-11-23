use product_codec::{Product, product};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

product! {
    #[derive(Clone, Copy, Debug)]
    pub struct Style { pub color: [u8; 4], pub pulse: bool, pub fill: f32 }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct StrategicLayer {
        pub region_of: ProvinceField, pub styles: crate::value_index::ValueIndex<u32, Style>, pub naval_fx: bool
    }
    #[derive(Clone, Debug)]
    pub struct ModeLayer {
        pub region_of: ProvinceField, pub styles: crate::value_index::ValueIndex<u32, Style>,
        pub thickness: Option<f32>, pub gradient: Option<u8>, pub channel: [f32; 2]
    }
    #[derive(Clone, Debug)]
    pub struct Draw {
        pub channels: [Source; 2], pub strategic: Option<StrategicLayer>,
        pub country: Option<ModeLayer>, pub layer: Option<ModeLayer>,
        pub second: Option<ModeLayer>, pub political: [f32; 2]
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Border { pub provinces: Vec<u32>, pub style: Option<Style> }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct LayerState {
        pub borders: BTreeMap<TargetId, Border>, pub refusals: Vec<String>, pub refused: bool
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ColourState { pub mode: String, pub layers: [LayerState; 2] }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct ModeFill {
        pub draw: Option<Draw>, pub warnings: Vec<String>, pub colour: Option<Arc<ColourState>>
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ColourOp {
        0 => Keep {}, 1 => Set { style: Style }, 2 => Clear {}
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct TargetOp {
        pub id: TargetId, pub provinces: ProvinceIds, pub operation: ColourOp
    }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct LayerProgram {
        pub targets: TargetPrograms, pub refusals: Vec<String>, pub refused: bool
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum RefreshDecision {
        0 => Value { forced: bool }, 1 => Refused { reason: String }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum NativeLayer { 0 => Country {}, 1 => States {}, 2 => Diplomacy {} }
    #[derive(Clone, Debug, PartialEq)]
    pub struct ColourSettings {
        pub thickness: Option<f32>, pub gradient: Option<u8>,
        pub channel: [f32; 2], pub province_count: usize
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ClosedLayer {
        0 => Empty {},
        1 => Game { kind: NativeLayer, layer: Option<ModeLayer>, warnings: Vec<String> },
        2 => Colour { program: LayerProgram, settings: ColourSettings }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct ScriptedProjection {
        pub mode: String, pub hour: i32, pub update_daily: bool,
        pub forced: RefreshDecision, pub refresh: Option<crate::identities::RefreshInterval>, pub political: [f32; 2], pub layers: [ClosedLayer; 2]
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ModeProjection {
        0 => Stock { fill: ModeFill }, 1 => Scripted { program: ScriptedProjection }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct CountryStyles {
        pub country: crate::identities::CountryKey, pub mode: String,
        pub projection: Arc<ModeProjection>, pub evidence: Vec<crate::Evidence>
    }
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct RailGraph {
        pub graph: BTreeMap<u32, BTreeMap<u32, i32>>,
        pub sources: BTreeSet<u32>, pub ports: BTreeSet<u32>
    }
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Attributes { pub control: u8, pub contested: u8, pub gradient: Option<u8> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Regions {
        pub country_of: ProvinceField, pub owner_of: ProvinceField, pub state_of: ProvinceField,
        pub country_colors: crate::value_index::ValueIndex<u32, [u8; 3]>, pub warnings: Vec<String>,
        pub strategic_attributes: BTreeMap<u32, Attributes>, pub mode: Option<Arc<Draw>>
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct TerritoryProvince {
        pub province: u32, pub state: u32, pub owner_raw: u32, pub controller_raw: u32,
        pub owner_slot: Option<u32>, pub controller_slot: Option<u32>, pub victory_points: i32
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Territory {
        pub paint: Arc<Regions>, pub countries: CountryFacts,
        pub links: crate::value_index::ValueIndex<u32, u32>, pub provinces: ProvinceFacts
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct TerritoryProjection { pub facts: Arc<Territory>, pub evidence: Vec<crate::Evidence> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct RailProjection { pub graph: Arc<RailGraph>, pub evidence: Vec<crate::Evidence> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct WorldName {
        pub key: String, pub strategic_provinces: Option<Arc<[u32]>>, pub text: String,
        pub pages: Vec<usize>, pub corners: Vec<[f32; 3]>, pub uvs: Vec<[f32; 4]>,
        pub colors: Vec<[f32; 4]>,
        pub aabb: [f32; 4], pub wrapped: bool, pub font_size: f32, pub spaces: usize, pub alpha: f32
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct LabelCandidate {
        pub province: u32, pub victory_points: i32, pub capital: bool,
        pub controller: Option<u32>, pub name: LabelName
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum LabelName {
        0 => Value { text: Option<String> }, 1 => Refused { reason: String }
    }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LabelFacts { pub tags: BTreeMap<u32, String>, pub candidates: Arc<Vec<LabelCandidate>> }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct Labels {
        pub perspective: Option<String>, pub candidates: Arc<Vec<LabelCandidate>>,
        pub allegiances: BTreeMap<u32, u32>
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placements {
    pub country: Vec<WorldName>,
    pub warnings: Vec<String>,
    pub generation: u64,
    pub near: Option<Arc<Placements>>,
}
impl Product for Placements {
    const MIN_BYTES: usize = 17;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        self.country.encode(out)?;
        self.warnings.encode(out)?;
        self.near.encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        Ok(Self {
            country: Vec::decode(input)?,
            warnings: Vec::decode(input)?,
            generation: 0,
            near: Option::decode(input)?,
        })
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("placements/content/v1");
        Vec::<WorldName>::schema(schema);
        Vec::<String>::schema(schema);
        schema.field("optional-near/placements");
    }
}

pub use crate::province_field::ProvinceField;
impl Regions {
    pub fn country_color(&self, key: u32) -> [u8; 3] {
        self.country_colors.get(&key).copied().unwrap_or([0, 0, 0])
    }
}
impl Attributes {
    pub fn packed(self) -> u32 {
        u32::from(self.control)
            | (u32::from(self.contested) << 8)
            | self
                .gradient
                .map_or(0, |value| (1 << 24) | (u32::from(value) << 16))
    }
}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.color == other.color
            && self.pulse == other.pulse
            && self.fill.to_bits() == other.fill.to_bits()
    }
}
impl Eq for Style {}
impl PartialEq for ModeLayer {
    fn eq(&self, other: &Self) -> bool {
        self.region_of == other.region_of
            && self.styles == other.styles
            && self.thickness.map(f32::to_bits) == other.thickness.map(f32::to_bits)
            && self.gradient == other.gradient
            && self.channel.map(f32::to_bits) == other.channel.map(f32::to_bits)
    }
}
impl Eq for ModeLayer {}
impl PartialEq for Draw {
    fn eq(&self, other: &Self) -> bool {
        self.channels == other.channels
            && self.strategic == other.strategic
            && self.country == other.country
            && self.layer == other.layer
            && self.second == other.second
            && self.political.map(f32::to_bits) == other.political.map(f32::to_bits)
    }
}
impl Eq for Draw {}
impl Draw {
    pub fn of(channels: [Source; 2], political: [f32; 2]) -> Self {
        Self {
            channels,
            strategic: None,
            country: None,
            layer: None,
            second: None,
            political,
        }
    }
    pub fn channel_constants(&self) -> [Option<[f32; 2]>; 2] {
        self.channels.map(|source| match source {
            Source::Country => Some(
                self.country
                    .as_ref()
                    .map_or(self.political, |layer| layer.channel),
            ),
            Source::Mode => self.layer.as_ref().map(|layer| layer.channel),
            Source::Second => self.second.as_ref().map(|layer| layer.channel),
            Source::Strategic | Source::Empty => None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Source {
    #[default]
    Strategic,
    Country,
    Mode,
    Second,
    Empty,
}
impl product_codec::Product for Source {
    const MIN_BYTES: usize = 1;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        (*self as u8).encode(out)
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Strategic),
            1 => Ok(Self::Country),
            2 => Ok(Self::Mode),
            3 => Ok(Self::Second),
            4 => Ok(Self::Empty),
            _ => Err(product_codec::invalid("unknown map mode source")),
        }
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("mode-source/strategic/country/mode/second/empty");
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetId {
    Country(u32),
    State(u32),
    Portion { state: u32, controller: u32 },
}
impl product_codec::Product for TargetId {
    const MIN_BYTES: usize = 5;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        match self {
            Self::Country(slot) => {
                0u8.encode(out)?;
                slot.encode(out)
            }
            Self::State(id) => {
                1u8.encode(out)?;
                id.encode(out)
            }
            Self::Portion { state, controller } => {
                2u8.encode(out)?;
                state.encode(out)?;
                controller.encode(out)
            }
        }
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Country(u32::decode(input)?)),
            1 => Ok(Self::State(u32::decode(input)?)),
            2 => Ok(Self::Portion {
                state: u32::decode(input)?,
                controller: u32::decode(input)?,
            }),
            _ => Err(product_codec::invalid("unknown colour target identity")),
        }
    }
    fn schema(schema: &mut product_codec::Schema) {
        schema.field("target/country/state/portion/u32");
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProvinceIds(pub crate::value_index::ValueIndex<u32, ()>);
impl PartialEq for ProvinceIds {
    fn eq(&self, other: &Self) -> bool {
        if self.0.ptr_eq(&other.0) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        let mut equal = true;
        self.0.visit_changes(&other.0, |_, _, _| equal = false);
        equal
    }
}
impl Eq for ProvinceIds {}
impl ProvinceIds {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &u32> {
        self.0.keys()
    }
    pub fn to_vec(&self) -> Vec<u32> {
        self.iter().copied().collect()
    }
}
impl Product for ProvinceIds {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        (self.len() as u64).encode(out)?;
        for id in self.iter() {
            id.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        let count = input.count64(4)?;
        let mut values = crate::value_index::ValueIndex::default();
        for _ in 0..count {
            let id = u32::decode(input)?;
            if values
                .at(values.len().saturating_sub(1))
                .is_some_and(|(last, _)| *last >= id)
            {
                return Err(product_codec::invalid("unordered target provinces"));
            }
            values = values.inserted(id, ())?;
        }
        Ok(Self(values))
    }
    fn schema(schema: &mut product_codec::Schema) {
        Vec::<u32>::schema(schema);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetPrograms(
    pub crate::value_index::ValueIndex<TargetId, crate::retained::Retained<TargetOp>>,
);
impl TargetPrograms {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl<'a> IntoIterator for &'a TargetPrograms {
    type Item = &'a TargetOp;
    type IntoIter = std::iter::Map<
        crate::value_index::Iter<'a, TargetId, crate::retained::Retained<TargetOp>>,
        fn((&'a TargetId, &'a crate::retained::Retained<TargetOp>)) -> &'a TargetOp,
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(_, value)| &**value)
    }
}
impl Product for TargetPrograms {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        (self.len() as u64).encode(out)?;
        for target in self {
            target.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        let count = input.count64(TargetOp::MIN_BYTES)?;
        let mut values = crate::value_index::ValueIndex::default();
        for _ in 0..count {
            let id = TargetId::decode(input)?;
            if values
                .last_at_or_before(&id)
                .is_some_and(|(last, _)| *last == id)
                || values
                    .at(values.len().saturating_sub(1))
                    .is_some_and(|(last, _)| *last > id)
            {
                return Err(product_codec::invalid("unordered colour targets"));
            }
            let provinces = ProvinceIds::decode(input)?;
            let operation = ColourOp::decode(input)?;
            let target = crate::retained::Retained::new(TargetOp {
                id,
                provinces,
                operation,
            });
            values = values.inserted(id, target)?;
        }
        Ok(Self(values))
    }
    fn schema(schema: &mut product_codec::Schema) {
        Vec::<TargetOp>::schema(schema);
    }
}

pub fn apply_layer(program: &LayerProgram, previous: Option<&LayerState>) -> LayerState {
    if program.refused {
        return LayerState {
            borders: BTreeMap::new(),
            refusals: program.refusals.clone(),
            refused: true,
        };
    }
    let mut borders: BTreeMap<TargetId, Border> = BTreeMap::new();
    for target in &program.targets {
        let old = borders
            .get(&target.id)
            .or_else(|| previous.and_then(|previous| previous.borders.get(&target.id)))
            .filter(|border| border.provinces.iter().eq(target.provinces.iter()));
        let style = match target.operation {
            ColourOp::Keep {} => old.and_then(|border| border.style),
            ColourOp::Set { style } => Some(style),
            ColourOp::Clear {} => None,
        };
        borders.insert(
            target.id,
            Border {
                provinces: target.provinces.to_vec(),
                style,
            },
        );
    }
    LayerState {
        borders,
        refusals: program.refusals.clone(),
        refused: false,
    }
}

fn materialize(
    settings: &ColourSettings,
    borders: &BTreeMap<TargetId, Border>,
) -> std::io::Result<ModeLayer> {
    let count = borders
        .values()
        .flat_map(|border| border.provinces.iter().copied())
        .max()
        .map_or(settings.province_count, |id| {
            settings.province_count.max(id as usize + 1)
        });
    let mut region_of = vec![0; count];
    let mut styles = BTreeMap::new();
    for (index, border) in borders.values().enumerate() {
        let Some(style) = border.style else {
            continue;
        };
        let key = index as u32 + 1;
        for &province in &border.provinces {
            if let Some(slot) = region_of.get_mut(province as usize) {
                *slot = key;
            }
        }
        styles.insert(key, style);
    }
    Ok(ModeLayer {
        region_of: region_of.into(),
        styles: crate::value_index::ValueIndex::try_from_iter(styles)?,
        thickness: settings.thickness,
        gradient: settings.gradient,
        channel: settings.channel,
    })
}

pub fn apply_mode(
    projection: &ModeProjection,
    previous: Option<&ColourState>,
) -> std::io::Result<ModeFill> {
    let ModeProjection::Scripted { program } = projection else {
        let ModeProjection::Stock { fill } = projection else {
            unreachable!()
        };
        return Ok(fill.clone());
    };
    let previous = previous.filter(|previous| previous.mode == program.mode);
    let scheduled =
        previous.is_none() || (program.update_daily && program.hour.rem_euclid(24) == 0);
    let forced = if scheduled {
        &RefreshDecision::Value { forced: false }
    } else {
        &program.forced
    };
    let refresh = scheduled || matches!(forced, RefreshDecision::Value { forced: true });
    let mut draw = Draw::of([Source::Empty; 2], program.political);
    let mut warnings = Vec::new();
    let mut kept: [LayerState; 2] = Default::default();
    let mut coloured = false;
    let mut held: Vec<(NativeLayer, Source)> = Vec::new();
    for (channel, layer) in program.layers.iter().enumerate() {
        draw.channels[channel] = match layer {
            ClosedLayer::Empty {} => Source::Empty,
            ClosedLayer::Game {
                kind: NativeLayer::Country {},
                layer,
                warnings: notes,
            } => {
                warnings.extend(notes.iter().cloned());
                let source = if layer.is_some() {
                    Source::Country
                } else {
                    Source::Empty
                };
                if channel == 1 && draw.channels[0] == Source::Country {
                    draw.channels[0] = source;
                }
                draw.country = layer.clone();
                source
            }
            ClosedLayer::Game {
                kind,
                layer,
                warnings: notes,
            } => {
                if let Some((_, source)) = held.iter().find(|(old, _)| old == kind) {
                    *source
                } else {
                    warnings.extend(notes.iter().cloned());
                    let (slot, source) = if draw.layer.is_none() {
                        (&mut draw.layer, Source::Mode)
                    } else {
                        (&mut draw.second, Source::Second)
                    };
                    let source = if layer.is_some() {
                        source
                    } else {
                        Source::Empty
                    };
                    *slot = layer.clone();
                    held.push((*kind, source));
                    source
                }
            }
            ClosedLayer::Colour { program, settings } => {
                coloured = true;
                let borders = if let RefreshDecision::Refused { reason } = forced {
                    warnings.push(reason.clone());
                    LayerState {
                        borders: BTreeMap::new(),
                        refusals: vec![reason.clone()],
                        refused: true,
                    }
                } else {
                    let state = if refresh {
                        apply_layer(program, previous.map(|previous| &previous.layers[channel]))
                    } else {
                        previous
                            .map(|previous| previous.layers[channel].clone())
                            .unwrap_or_default()
                    };
                    warnings.extend(state.refusals.iter().cloned());
                    state
                };
                let layer = (!borders.refused)
                    .then(|| materialize(settings, &borders.borders))
                    .transpose()?;
                kept[channel] = borders;
                let (slot, source) = if draw.layer.is_none() {
                    (&mut draw.layer, Source::Mode)
                } else {
                    (&mut draw.second, Source::Second)
                };
                let source = if layer.is_some() {
                    source
                } else {
                    Source::Empty
                };
                *slot = layer;
                source
            }
        };
    }
    Ok(ModeFill {
        draw: Some(draw),
        warnings,
        colour: coloured.then(|| {
            Arc::new(ColourState {
                mode: program.mode.clone(),
                layers: kept,
            })
        }),
    })
}

impl WorldName {
    pub fn from_glyphs(
        key: String,
        text: String,
        glyphs: impl IntoIterator<Item = ([[f32; 3]; 4], [f32; 4], usize, [f32; 4])>,
        alpha: f32,
    ) -> Option<Self> {
        let mut corners = Vec::new();
        let mut uvs = Vec::new();
        let mut pages = Vec::new();
        let mut colors = Vec::new();
        let mut aabb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for (quad, uv, page, color) in glyphs {
            for corner in quad {
                aabb[0] = aabb[0].min(corner[0]);
                aabb[1] = aabb[1].min(corner[2]);
                aabb[2] = aabb[2].max(corner[0]);
                aabb[3] = aabb[3].max(corner[2]);
                corners.push(corner);
            }
            uvs.push(uv);
            pages.push(page);
            colors.push(color);
        }
        (!uvs.is_empty()).then_some(Self {
            key,
            strategic_provinces: None,
            text,
            pages,
            corners,
            uvs,
            colors,
            aabb,
            wrapped: false,
            font_size: 0.0,
            spaces: 0,
            alpha,
        })
    }

    pub fn glyph_count(&self) -> usize {
        self.uvs.len()
    }

    pub fn summary(&self) -> String {
        format!(
            "{}: {:?} font size {:.2}, {} spaces between characters, {} glyphs,              world x {:.0}..{:.0} z {:.0}..{:.0}",
            self.key,
            self.text,
            self.font_size,
            self.spaces,
            self.glyph_count(),
            self.aabb[0],
            self.aabb[2],
            self.aabb[1],
            self.aabb[3]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_refresh_preserves_kept_cleared_and_replaced_membership_without_source() {
        let id = TargetId::Country(1501);
        let style = Style {
            color: [1, 2, 3, 4],
            pulse: true,
            fill: f32::from_bits(0x7fc0_1234),
        };
        let previous = LayerState {
            borders: [(
                id,
                Border {
                    provinces: vec![7],
                    style: Some(style),
                },
            )]
            .into(),
            ..Default::default()
        };
        let program = |provinces: Vec<u32>, operation| LayerProgram {
            targets: TargetPrograms(
                crate::value_index::ValueIndex::try_from_iter([(
                    id,
                    crate::retained::Retained::new(TargetOp {
                        id,
                        provinces: ProvinceIds(
                            crate::value_index::ValueIndex::try_from_iter(
                                provinces.into_iter().map(|id| (id, ())),
                            )
                            .unwrap(),
                        ),
                        operation,
                    }),
                )])
                .unwrap(),
            ),
            ..Default::default()
        };
        assert_eq!(
            apply_layer(&program(vec![7], ColourOp::Keep {}), Some(&previous)),
            previous
        );
        assert_eq!(
            apply_layer(&program(vec![8], ColourOp::Keep {}), Some(&previous)).borders[&id].style,
            None
        );
        assert_eq!(
            apply_layer(&program(vec![7], ColourOp::Clear {}), Some(&previous)).borders[&id].style,
            None
        );
        let set = program(vec![8], ColourOp::Set { style });
        let encoded = product_codec::encode(&set).unwrap();
        let decoded: LayerProgram = product_codec::decode(&encoded).unwrap();
        assert_eq!(decoded, set);
        assert_eq!(
            apply_layer(&decoded, None).borders[&id]
                .style
                .unwrap()
                .fill
                .to_bits(),
            0x7fc0_1234
        );
    }
    #[test]
    fn persistent_script_target_directories_keep_the_native_ordered_wire() {
        let id = TargetId::Country(1501);
        let provinces = ProvinceIds(
            crate::value_index::ValueIndex::try_from_iter([(7, ()), (99, ())]).unwrap(),
        );
        let target = crate::retained::Retained::new(TargetOp {
            id,
            provinces,
            operation: ColourOp::Keep {},
        });
        let targets =
            TargetPrograms(crate::value_index::ValueIndex::try_from_iter([(id, target)]).unwrap());
        let mut expected = Vec::new();
        1u64.encode(&mut expected).unwrap();
        id.encode(&mut expected).unwrap();
        vec![7u32, 99].encode(&mut expected).unwrap();
        ColourOp::Keep {}.encode(&mut expected).unwrap();
        assert_eq!(product_codec::encode(&targets).unwrap(), expected);
        assert_eq!(
            product_codec::decode::<TargetPrograms>(&expected).unwrap(),
            targets
        );
        let first = ProvinceIds(
            crate::value_index::ValueIndex::try_from_iter((1..=1501).map(|id| (id, ()))).unwrap(),
        );
        let second = ProvinceIds(first.0.removed(&777).unwrap().inserted(2000, ()).unwrap());
        assert_ne!(first, second);
        let mut edits = Vec::new();
        second.0.visit_changes(&first.0, |id, _, _| edits.push(*id));
        assert_eq!(edits, [777, 2000]);
    }

    #[test]
    fn country_indices_have_no_encoded_population_ceiling() {
        let graph = RailGraph {
            graph: (0..5001).map(|id| (id, [(id + 1, 3)].into())).collect(),
            sources: (0..5001).collect(),
            ports: [0, 1500, 5000].into(),
        };
        let bytes = product_codec::encode(&graph).unwrap();
        assert_eq!(product_codec::decode::<RailGraph>(&bytes).unwrap(), graph);
        let layer = ModeLayer {
            region_of: (0..5001).collect::<Vec<_>>().into(),
            styles: crate::value_index::ValueIndex::try_from_iter((0..5001).map(|id| {
                (
                    id,
                    Style {
                        color: [id as u8, 2, 3, 4],
                        pulse: id % 2 == 0,
                        fill: -0.0,
                    },
                )
            }))
            .unwrap(),
            thickness: Some(f32::from_bits(0x7fc0_1234)),
            gradient: None,
            channel: [-0.0, 1.0],
        };
        assert_eq!(
            product_codec::decode::<ModeLayer>(&product_codec::encode(&layer).unwrap()).unwrap(),
            layer
        );
        let shared = layer.clone();
        assert!(Arc::ptr_eq(&layer.region_of.0, &shared.region_of.0));
    }
    #[test]
    fn placement_upload_generation_is_not_persisted_as_content() {
        let first = Placements {
            generation: 7,
            ..Default::default()
        };
        let second = Placements {
            generation: 999,
            ..Default::default()
        };
        let bytes = product_codec::encode(&first).unwrap();
        assert_eq!(bytes, product_codec::encode(&second).unwrap());
        assert_eq!(
            product_codec::decode::<Placements>(&bytes)
                .unwrap()
                .generation,
            0
        );
    }
    #[test]
    fn graph_rejects_duplicate_set_members() {
        let mut bytes = Vec::new();
        0u64.encode(&mut bytes).unwrap();
        2u64.encode(&mut bytes).unwrap();
        7u32.encode(&mut bytes).unwrap();
        7u32.encode(&mut bytes).unwrap();
        0u64.encode(&mut bytes).unwrap();
        assert!(product_codec::decode::<RailGraph>(&bytes).is_err());
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceFacts(pub crate::value_index::ValueIndex<u32, TerritoryProvince>);
impl ProvinceFacts {
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &TerritoryProvince> {
        self.0.values()
    }
}
impl std::ops::Index<usize> for ProvinceFacts {
    type Output = TerritoryProvince;
    fn index(&self, at: usize) -> &Self::Output {
        self.0.at(at).expect("recorded province ordinal").1
    }
}
impl Product for ProvinceFacts {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        (self.len() as u64).encode(out)?;
        for value in self.iter() {
            value.encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        let count = input.count64(TerritoryProvince::MIN_BYTES)?;
        let mut values = crate::value_index::ValueIndex::default();
        for _ in 0..count {
            let row = TerritoryProvince::decode(input)?;
            if values
                .at(values.len().saturating_sub(1))
                .is_some_and(|(id, _)| *id >= row.province)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unordered territory provinces",
                ));
            }
            values = values.inserted(row.province, row)?;
        }
        Ok(Self(values))
    }
    fn schema(schema: &mut product_codec::Schema) {
        Vec::<TerritoryProvince>::schema(schema);
    }
}

#[derive(Clone, Debug, Default)]
pub struct CountryFacts {
    pub values: crate::value_index::ValueIndex<
        u32,
        crate::retained::Retained<crate::identities::CountryKey>,
    >,
    pub observed_hour: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountryObservation<'a> {
    pub branch: u64,
    pub native: u32,
    pub canonical_slot: u32,
    pub observed_hour: i32,
    pub continuity: &'a str,
}
impl CountryFacts {
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn get(&self, slot: &u32) -> Option<CountryObservation<'_>> {
        self.values.get(slot).map(|country| CountryObservation {
            branch: country.branch,
            native: country.native,
            canonical_slot: country.canonical_slot,
            observed_hour: self.observed_hour,
            continuity: &country.continuity,
        })
    }
    pub fn iter(&self) -> impl Iterator<Item = (&u32, CountryObservation<'_>)> {
        self.values
            .keys()
            .map(|slot| (slot, self.get(slot).unwrap()))
    }
}
impl PartialEq for CountryFacts {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl Product for CountryFacts {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> std::io::Result<()> {
        (self.len() as u64).encode(out)?;
        for (slot, value) in self.iter() {
            slot.encode(out)?;
            value.branch.encode(out)?;
            value.native.encode(out)?;
            value.canonical_slot.encode(out)?;
            value.observed_hour.encode(out)?;
            (value.continuity.len() as u64).encode(out)?;
            out.extend_from_slice(value.continuity.as_bytes());
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> std::io::Result<Self> {
        let values = crate::value_index::ValueIndex::<
            u32,
            crate::retained::Retained<crate::identities::CountryKey>,
        >::decode(input)?;
        let observed_hour = values
            .iter()
            .next()
            .map_or(0, |(_, country)| country.observed_hour);
        if values
            .values()
            .any(|country| country.observed_hour != observed_hour)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "territory countries disagree on supplying hour",
            ));
        }
        Ok(Self {
            values,
            observed_hour,
        })
    }
    fn schema(schema: &mut product_codec::Schema) {
        <crate::value_index::ValueIndex<
            u32,
            crate::retained::Retained<crate::identities::CountryKey>,
        > as Product>::schema(schema);
    }
}

#[cfg(test)]
mod country_fact_tests {
    use super::*;
    #[test]
    fn supplying_hour_rebinds_without_rebuilding_country_values_or_changing_wire_bytes() {
        let mut values = crate::value_index::ValueIndex::default();
        let mut oracle = BTreeMap::new();
        for slot in 0..1501u32 {
            let country = crate::identities::CountryKey {
                branch: 1,
                native: slot + 3000,
                canonical_slot: slot,
                observed_hour: 10,
                continuity: "observation only".into(),
            };
            values = values
                .inserted(slot, crate::retained::Retained::new(country.clone()))
                .unwrap();
            oracle.insert(
                slot,
                crate::identities::CountryKey {
                    observed_hour: 20,
                    ..country
                },
            );
        }
        let facts = CountryFacts {
            values: values.clone(),
            observed_hour: 20,
        };
        assert!(facts.values.ptr_eq(&values));
        assert_eq!(facts.get(&1500).unwrap().observed_hour, 20);
        let bytes = product_codec::encode(&facts).unwrap();
        assert_eq!(bytes, product_codec::encode(&oracle).unwrap());
        assert_eq!(
            product_codec::decode::<CountryFacts>(&bytes).unwrap(),
            facts
        );
    }
}
