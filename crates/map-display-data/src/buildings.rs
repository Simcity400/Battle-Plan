use crate::{retained::Retained, value_index::ValueIndex};
use product_codec::{Product, product};
use std::io;
use std::sync::Arc;

product! {
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct AssetLayer {
        pub id: String, pub source: Vec<u8>, pub platform: String,
        pub archive: bool, pub replace_paths: Vec<Vec<u8>>
    }
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub enum AssetKey {
        0 => Unbound {},
        1 => Selected {
            entity: String, namespace: Vec<AssetLayer>, role: u8,
            contract: u32, force_pow2_textures: bool
        }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Selection {
        pub admitted: bool, pub mesh: Arc<AssetKey>, pub definition: Arc<AssetKey>, pub variant: usize
    }
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Instance {
        pub position: [f32; 3], pub rotation: f32, pub scale: f32, pub animation_seed: u32
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Entity {
        pub healthy: Selection, pub ruined: Selection,
        pub destroyed: bool, pub uncertain: bool, pub damaged: bool,
        pub disable_grow_animation: bool, pub instance: Instance
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Slot {
        pub id: u32, pub selected_type: Option<String>, pub recorded_country: Option<u32>,
        pub drawn_at_distance: Option<bool>, pub entity: Option<Entity>
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct AuthoredSlot {
        pub id: u32, pub location: u32, pub bucket: String, pub order: usize,
        pub province: Option<u32>, pub position: [f32; 3], pub rotation: f32, pub floating: bool
    }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Group { pub province: u32, pub bounds: [f32; 4], pub slots: Vec<u32> }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CityDamage { pub health: ValueIndex<u32, Option<i64>>, pub refused: Option<String> }
    #[derive(Clone, Debug, PartialEq)]
    pub struct Buildings {
        pub bank: [u8; 32], pub branch: u64, pub hour: i32,
        pub perspective: u32, pub fog: bool,
        pub shrink_distance: f32, pub popup_speed: f32,
        pub authored: Arc<Vec<AuthoredSlot>>,
        pub slots: SlotChunks, pub groups: Arc<Vec<Group>>,
        pub city_damage: Arc<CityDamage>, pub warnings: Vec<String>
    }
}

impl PartialEq for Instance {
    fn eq(&self, other: &Self) -> bool {
        self.position.map(f32::to_bits) == other.position.map(f32::to_bits)
            && self.rotation.to_bits() == other.rotation.to_bits()
            && self.scale.to_bits() == other.scale.to_bits()
            && self.animation_seed == other.animation_seed
    }
}

#[derive(Clone, Default)]
pub struct SlotChunks(
    ValueIndex<usize, Retained<Vec<Slot>>>,
    Option<Producer>,
);
#[derive(Clone)]
struct Producer {
    owner: u64,
    pages: Arc<dyn std::any::Any + Send + Sync>,
}
impl std::fmt::Debug for SlotChunks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SlotChunks").field(&self.0).finish()
    }
}
impl PartialEq for SlotChunks {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl SlotChunks {
    pub fn with_producer<T: std::any::Any + Send + Sync>(
        mut self,
        owner: u64,
        pages: Arc<T>,
    ) -> Self {
        self.1 = Some(Producer { owner, pages });
        self
    }
    pub fn producer<T: std::any::Any + Send + Sync>(&self, owner: u64) -> Option<Arc<T>> {
        self.1
            .as_ref()
            .filter(|producer| producer.owner == owner)?
            .pages
            .clone()
            .downcast()
            .ok()
    }
    fn with_values(&self, values: ValueIndex<usize, Retained<Vec<Slot>>>) -> Self {
        let producer = self.0.ptr_eq(&values).then(|| self.1.clone()).flatten();
        Self(values, producer)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, index: usize) -> Option<&Retained<Vec<Slot>>> {
        self.0.get(&index)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Retained<Vec<Slot>>> {
        self.0.values()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
    pub fn inserted(&self, index: usize, chunk: Retained<Vec<Slot>>) -> io::Result<Self> {
        if index > self.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "building chunk directory has a gap",
            ));
        }
        Ok(self.with_values(self.0.inserted(index, chunk)?))
    }
    pub fn patched(&self, edits: &[(usize, Option<Retained<Vec<Slot>>>)]) -> io::Result<Self> {
        let mut len = self.len();
        for (index, value) in edits {
            if value.is_none() || *index > len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "building chunk directory has a gap",
                ));
            }
            if *index == len {
                len += 1;
            }
        }
        Ok(self.with_values(self.0.patched(edits)?))
    }
    pub fn visit_changes(
        &self,
        previous: &Self,
        visit: impl FnMut(&usize, Option<&Retained<Vec<Slot>>>, Option<&Retained<Vec<Slot>>>),
    ) {
        self.0.visit_changes(&previous.0, visit);
    }
}
impl std::ops::Index<usize> for SlotChunks {
    type Output = Retained<Vec<Slot>>;
    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("authored building chunk")
    }
}
impl Product for SlotChunks {
    const MIN_BYTES: usize = 8;
    fn encode(&self, out: &mut Vec<u8>) -> io::Result<()> {
        self.len().encode(out)?;
        for chunk in self.iter() {
            (**chunk).encode(out)?;
        }
        Ok(())
    }
    fn decode(input: &mut product_codec::Decoder<'_>) -> io::Result<Self> {
        let count = input.count64(8)?;
        let mut chunks = Self::default();
        for index in 0..count {
            let count = input.count64(Slot::MIN_BYTES)?;
            let mut chunk = Vec::with_capacity(count);
            for _ in 0..count {
                chunk.push(Slot::decode(input)?);
            }
            chunks = chunks.inserted(index, Retained::new(chunk))?;
        }
        Ok(chunks)
    }
    fn schema(schema: &mut product_codec::Schema) {
        <Arc<Vec<Arc<Vec<Slot>>>> as Product>::schema(schema);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn building_operands_keep_float_bits_and_optional_latches() {
        let instance = Instance {
            position: [f32::from_bits(0x7fc0_1234), -0.0, f32::INFINITY],
            rotation: -0.0,
            scale: f32::from_bits(0x7fc0_4321),
            animation_seed: u32::MAX,
        };
        let bytes = product_codec::encode(&instance).unwrap();
        let decoded: Instance = product_codec::decode(&bytes).unwrap();
        assert_eq!(product_codec::encode(&decoded).unwrap(), bytes);
        assert_eq!(
            decoded.position.map(f32::to_bits),
            instance.position.map(f32::to_bits)
        );
        assert_eq!(decoded.scale.to_bits(), instance.scale.to_bits());
        let slots: Vec<_> = [None, Some(false), Some(true)]
            .into_iter()
            .enumerate()
            .map(|(id, latch)| Slot {
                id: id as u32,
                selected_type: None,
                recorded_country: None,
                drawn_at_distance: latch,
                entity: None,
            })
            .collect();
        assert_eq!(
            product_codec::decode::<Vec<Slot>>(&product_codec::encode(&slots).unwrap()).unwrap(),
            slots
        );
    }
    #[test]
    fn slot_directory_matches_dense_wire_and_shares_untouched_chunks() {
        let mut chunks = SlotChunks::default();
        let mut dense = Vec::new();
        for id in 0..1501u32 {
            let values = vec![Slot {
                id,
                selected_type: None,
                recorded_country: None,
                drawn_at_distance: Some(false),
                entity: None,
            }];
            chunks = chunks
                .inserted(id as usize, Retained::new(values.clone()))
                .unwrap();
            dense.push(Arc::new(values));
        }
        let wire = product_codec::encode(&chunks).unwrap();
        let producer = Arc::new(17u32);
        chunks = chunks.with_producer(7, Arc::clone(&producer));
        assert_eq!(product_codec::encode(&chunks).unwrap(), wire);
        assert!(
            product_codec::decode::<SlotChunks>(&wire)
                .unwrap()
                .producer::<u32>(7)
                .is_none()
        );
        let unchanged = chunks.patched(&[]).unwrap();
        assert!(Arc::ptr_eq(
            &unchanged.producer::<u32>(7).unwrap(),
            &producer
        ));
        assert!(unchanged.producer::<u32>(8).is_none());
        let before = chunks.clone();
        let mut changed = (*chunks[777]).clone();
        changed[0].drawn_at_distance = None;
        chunks = chunks
            .inserted(777, Retained::new(changed.clone()))
            .unwrap();
        dense[777] = Arc::new(changed);
        assert!(chunks.producer::<u32>(7).is_none());
        let mut keys = Vec::new();
        chunks.visit_changes(&before, |key, _, _| keys.push(*key));
        assert_eq!(keys, [777]);
        assert!(chunks[776].ptr_eq(&before[776]));
        assert!(chunks[778].ptr_eq(&before[778]));
        let encoded = product_codec::encode(&chunks).unwrap();
        assert_eq!(encoded, product_codec::encode(&dense).unwrap());
        assert_eq!(
            product_codec::decode::<SlotChunks>(&encoded).unwrap(),
            chunks
        );
        assert_eq!(
            product_codec::schema_digest::<SlotChunks>(),
            product_codec::schema_digest::<Arc<Vec<Arc<Vec<Slot>>>>>()
        );
        let a = Instance {
            position: [-0.0, f32::from_bits(0x7fc0_1234), 0.0],
            ..Default::default()
        };
        assert_eq!(a, a);
        assert_ne!(
            a,
            Instance {
                position: [0.0, a.position[1], 0.0],
                ..a
            }
        );
        assert_ne!(
            a,
            Instance {
                position: [-0.0, f32::from_bits(0x7fc0_4321), 0.0],
                ..a
            }
        );
    }
}
