use super::*;
use incremental::{BuildingsBase, FloatingInputs};
use map_display_data::{copy_index::CopyIndex, retained::Retained, value_index::ValueIndex};
use replay_frame::frame::ViewIdentity;

#[derive(Clone)]
struct Paint(PreparedSlot);
impl PartialEq for Paint {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_bits(&other.0)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HarbourCounts {
    pub fogged: usize,
    pub placed: usize,
    pub unknown_fog: usize,
    pub unknown_deployment: usize,
}
impl HarbourCounts {
    pub fn apply(&self, summary: &mut MapBuildingSummary) {
        summary.fogged += self.fogged;
        summary.placed += self.placed;
        if self.unknown_fog != 0 {
            *summary
                .unknown_floating_harbors
                .entry("the fog byte of the harbour's sea province is unknown")
                .or_default() += self.unknown_fog;
        }
        if self.unknown_deployment != 0 {
            *summary
                .unknown_floating_harbors
                .entry("the controller's floating-harbour deployment is unknown")
                .or_default() += self.unknown_deployment;
        }
    }
    pub fn add(&mut self, other: Self) {
        self.fogged += other.fogged;
        self.placed += other.placed;
        self.unknown_fog += other.unknown_fog;
        self.unknown_deployment += other.unknown_deployment;
    }
    pub fn remove(&mut self, other: Self) {
        self.fogged -= other.fogged;
        self.placed -= other.placed;
        self.unknown_fog -= other.unknown_fog;
        self.unknown_deployment -= other.unknown_deployment;
    }
}
pub(super) struct Output {
    branch: u64,
    campaign: ViewIdentity,
    by_province_slot: CopyIndex<(u32, u32), ()>,
    slots: ValueIndex<u32, Retained<Paint>>,
    counts: CopyIndex<u32, HarbourCounts>,
    totals: HarbourCounts,
    pub(super) changes: Retained<Vec<(u32, PreparedSlot)>>,
}
impl Output {
    pub(super) fn iter(&self) -> impl Iterator<Item = &PreparedSlot> {
        self.slots.values().map(|value| &(**value).0)
    }
    pub(super) fn apply(&self, summary: &mut MapBuildingSummary) {
        self.totals.apply(summary);
    }
    pub(super) fn read(
        assets: &MapBuildingAssets,
        frame: &DecodedFrame,
        vision: &Vision,
        inputs: &FloatingInputs,
        previous: Option<&BuildingsBase>,
        fog_changes: Option<&CopyIndex<u32, ()>>,
    ) -> io::Result<Self> {
        let old = previous.map(|base| &base.floating_output).filter(|old| {
            old.branch == frame.branch && old.campaign.is(&frame.tables[TableId::Campaign.index()])
        });
        let mut by_province_slot =
            old.map_or_else(CopyIndex::default, |old| old.by_province_slot.clone());
        let mut dirty = CopyIndex::default();
        let source_changes = inputs.changed_provinces(previous.map(|base| &base.floating))?;
        if let (Some(_), Some(source), Some(fog)) = (old, source_changes.as_ref(), fog_changes) {
            for (&province, _) in source.iter().chain(fog.iter()) {
                for (&(_, slot), _) in by_province_slot
                    .iter_from(&(province, 0))
                    .take_while(|(key, _)| key.0 == province)
                {
                    dirty = dirty.inserted(slot, ())?;
                }
            }
        } else {
            for (index, (position, _)) in assets.floating.iter().enumerate() {
                let id = u32::try_from(index)
                    .map_err(|_| invalid("floating harbour slot identity exceeds u32"))?;
                dirty = dirty.inserted(id, ())?;
                by_province_slot = by_province_slot.inserted((position.adjacent_sea, id), ())?;
            }
        }
        let mut slots = old.map_or_else(ValueIndex::default, |old| old.slots.clone());
        let mut counts = old.map_or_else(CopyIndex::default, |old| old.counts.clone());
        let mut totals = old.map_or_else(HarbourCounts::default, |old| old.totals);
        let mut changes = Vec::with_capacity(dirty.len());
        let ordinary = assets.ordinary_slot_count();
        for (&index, _) in dirty.iter() {
            let (position, _) = &assets.floating[index as usize];
            let id = u32::try_from(
                ordinary
                    .checked_add(index as usize)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?,
            )
            .map_err(|_| invalid("floating harbour slot identity exceeds u32"))?;
            let (paint, tally) = assets.harbour(
                position,
                id,
                |province| vision.province_byte(province),
                |province| inputs.deployed(province),
            );
            if let Some(before) = counts.get(&index) {
                totals.remove(*before);
            }
            totals.add(tally);
            counts = counts.inserted(index, tally)?;
            if slots
                .get(&index)
                .is_none_or(|before| !(**before).0.same_bits(&paint))
            {
                slots = slots.inserted(index, Retained::new(Paint(paint.clone())))?;
                changes.push((id, paint));
            }
        }
        Ok(Self {
            branch: frame.branch,
            campaign: frame.tables[TableId::Campaign.index()].identity(),
            by_province_slot,
            slots,
            counts,
            totals,
            changes: Retained::new(changes),
        })
    }
}
impl MapBuildingAssets {
    pub(super) fn harbour(
        &self,
        position: &BuildingPosition,
        id: u32,
        byte: impl FnOnce(u32) -> Option<u8>,
        deployed: impl FnOnce(u32) -> Option<bool>,
    ) -> (PreparedSlot, HarbourCounts) {
        let mut tally = HarbourCounts::default();
        let entity = (|| {
            let province = position.adjacent_sea;
            if (province as i32) <= 0 {
                return None;
            }
            match byte(province).map(|byte| byte >= FLOATING_HARBOR_VISIBILITY) {
                Some(true) => {}
                Some(false) => {
                    tally.fogged = 1;
                    return None;
                }
                None => {
                    tally.unknown_fog = 1;
                    return None;
                }
            }
            let deployed = deployed(province);
            if deployed.is_none() {
                tally.unknown_deployment = 1;
            }
            let mesh = self.floating_mesh.filter(|_| deployed == Some(true))?;
            tally.placed = 1;
            let selection = Selection {
                admitted: true,
                mesh,
                definition: mesh,
                variant: 0,
            };
            Some(PreparedEntity {
                healthy: selection,
                ruined: selection,
                destroyed: false,
                uncertain: false,
                damaged: false,
                disable_grow_animation: false,
                instance: ModelInstance {
                    position: [position.x, position.y, position.z],
                    rotation: position.rotation,
                    scale: 1.0,
                    animation_seed: id,
                    ..Default::default()
                },
            })
        })();
        (
            PreparedSlot {
                source: None,
                id,
                drawn_at_distance: Some(false),
                entity: entity.map(PreparedEntity::retain),
            },
            tally,
        )
    }
}
