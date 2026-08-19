use crate::anchor::MapRenderAnchor;
use crate::memory::{PassiveMemory, PassiveMemoryExt};
use crate::refusal::Refusal;

pub const MAP_RENDER_STATE_SNOW_MUD_FADE_OFFSET: u64 = 0x5ce8;

pub const ANCHOR_CHANGED_REFUSAL: &str = "xmap07.anchor_changed_around_fade_read";

pub const WITNESS_UNREADABLE_REFUSAL: &str = "xmap07.witness_word_unreadable";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassiveSnowMudFade {
    pub fade_f32_bits: u32,
    pub map_render_state: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SnowMudFadeSample {
    pub witness_before: ChainWitness,
    pub fade_f32_bits_first: u32,
    pub fade_f32_bits_second: u32,
    pub witness_after: ChainWitness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChainWitness {
    pub root_slot: u64,
    pub root_vptr: u64,
    pub delegate: u64,
    pub map_object: u64,
    pub map_render_state: u64,
}

fn copy_chain_witness<M: PassiveMemory + ?Sized>(
    memory: &M,
    anchor: &MapRenderAnchor,
) -> Result<ChainWitness, Refusal> {
    let unreadable = |what: &'static str| {
        move || {
            Refusal::new(
                WITNESS_UNREADABLE_REFUSAL,
                format!("{what} was unreadable in the capture bracket"),
            )
        }
    };
    Ok(ChainWitness {
        root_slot: memory
            .read_u64(
                memory
                    .module_base()
                    .wrapping_add(crate::anchor::GAME_ROOT_SINGLETON_RVA),
            )
            .ok_or_else(unreadable("the game-root slot"))?,
        root_vptr: memory
            .read_u64(anchor.game_root)
            .ok_or_else(unreadable("the game-root vptr"))?,
        delegate: memory
            .read_u64_field(anchor.game_root, crate::anchor::GAME_ROOT_DELEGATE_OFFSET)
            .ok_or_else(unreadable("the delegate link"))?,
        map_object: memory
            .read_u64_field(anchor.delegate, crate::anchor::DELEGATE_MAP_OBJECT_OFFSET)
            .ok_or_else(unreadable("the map-object link"))?,
        map_render_state: memory
            .read_u64_field(
                anchor.map_object,
                crate::anchor::MAP_OBJECT_RENDER_STATE_OFFSET,
            )
            .ok_or_else(unreadable("the render-state link"))?,
    })
}

pub fn copy_snow_mud_fade_sample<M: PassiveMemory + ?Sized>(
    memory: &M,
    anchor: &MapRenderAnchor,
) -> Result<SnowMudFadeSample, Refusal> {
    let unreadable = |what: &'static str| {
        move || {
            Refusal::new(
                "xmap07.fade_word_unreadable",
                format!("{what} was unreadable in the capture bracket"),
            )
        }
    };
    let fade_at = anchor
        .map_render_state
        .checked_add(MAP_RENDER_STATE_SNOW_MUD_FADE_OFFSET)
        .ok_or_else(|| {
            Refusal::new(
                "xmap07.fade_address_overflow",
                "map_render_state + 0x5ce8 overflowed",
            )
        })?;
    let witness_before = copy_chain_witness(memory, anchor)?;
    let fade_f32_bits_first = memory
        .read_f32_bits(fade_at)
        .ok_or_else(unreadable("the fade word"))?;
    let fade_f32_bits_second = memory
        .read_f32_bits(fade_at)
        .ok_or_else(unreadable("the fade word on its second read"))?;
    let witness_after = copy_chain_witness(memory, anchor)?;
    Ok(SnowMudFadeSample {
        witness_before,
        fade_f32_bits_first,
        fade_f32_bits_second,
        witness_after,
    })
}

pub fn validate_snow_mud_fade_sample(
    sample: &SnowMudFadeSample,
    anchor: &MapRenderAnchor,
    module_base: u64,
) -> Result<PassiveSnowMudFade, Refusal> {
    let expected = ChainWitness {
        root_slot: anchor.game_root,
        root_vptr: module_base.wrapping_add(crate::anchor::GAME_ROOT_VTABLE_RVA),
        delegate: anchor.delegate,
        map_object: anchor.map_object,
        map_render_state: anchor.map_render_state,
    };
    if sample.witness_before != expected || sample.witness_after != expected {
        return Err(Refusal::new(
            ANCHOR_CHANGED_REFUSAL,
            "a chain witness copied in the bracket does not match the discovered anchor; the \
             fade would come from a detached map render state",
        ));
    }
    if sample.fade_f32_bits_first != sample.fade_f32_bits_second {
        return Err(Refusal::new(
            "xmap07.fade_advanced_during_read",
            "the fade word changed between two reads; the sample straddles the hour cutoff",
        ));
    }
    let value = f32::from_bits(sample.fade_f32_bits_first);
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(Refusal::new(
            "xmap07.fade_out_of_range",
            format!(
                "the fade word {:#010x} decodes to {value}, outside the finite 0..=1 range",
                sample.fade_f32_bits_first
            ),
        ));
    }
    Ok(PassiveSnowMudFade {
        fade_f32_bits: sample.fade_f32_bits_first,
        map_render_state: anchor.map_render_state,
    })
}

pub fn cross_check_reflected_fade(
    passive: &PassiveSnowMudFade,
    reflected_fade_f32_bits: u32,
    reflected_consumer_count: u32,
) -> Result<(), Refusal> {
    if reflected_consumer_count == 0 {
        return Err(Refusal::new(
            "xmap07.reflected_cross_check_empty",
            "the reflected consumer set was empty, so it cross-checks nothing",
        ));
    }
    if reflected_fade_f32_bits != passive.fade_f32_bits {
        return Err(Refusal::new(
            "xmap07.reflected_cross_check_disagrees",
            format!(
                "the passive fade {:#010x} at map_render_state+0x5ce8 disagrees with the \
                 reflected consumer value {reflected_fade_f32_bits:#010x}",
                passive.fade_f32_bits
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::resolve_map_render_anchor;
    use crate::anchor::tests::{anchored_memory, MAP_RENDER_STATE};
    use crate::memory::SliceMemory;

    fn memory_with_fade(bits: u32) -> SliceMemory {
        let mut memory = anchored_memory();
        memory.put_u32(MAP_RENDER_STATE + 0x5ce8, bits);
        memory
    }

    fn copy_and_validate(
        memory: &impl PassiveMemory,
        anchor: &MapRenderAnchor,
    ) -> Result<PassiveSnowMudFade, Refusal> {
        let sample = copy_snow_mud_fade_sample(memory, anchor)?;
        validate_snow_mud_fade_sample(&sample, anchor, memory.module_base())
    }

    #[test]
    fn the_fade_is_read_from_the_anchored_map_render_state() {
        let memory = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&memory).unwrap();
        let fade = copy_and_validate(&memory, &anchor).unwrap();
        assert_eq!(fade.fade_f32_bits, 0.25f32.to_bits());
        assert_eq!(fade.map_render_state, MAP_RENDER_STATE);
    }

    #[test]
    fn an_out_of_range_or_nan_fade_refuses() {
        for bits in [f32::NAN.to_bits(), 2.0f32.to_bits(), (-1.0f32).to_bits()] {
            let memory = memory_with_fade(bits);
            let anchor = resolve_map_render_anchor(&memory).unwrap();
            assert_eq!(
                copy_and_validate(&memory, &anchor).unwrap_err().reason,
                "xmap07.fade_out_of_range"
            );
        }
    }

    #[test]
    fn the_reflected_value_can_only_agree_or_refuse_and_never_becomes_the_source() {
        let memory = memory_with_fade(0.5f32.to_bits());
        let anchor = resolve_map_render_anchor(&memory).unwrap();
        let passive = copy_and_validate(&memory, &anchor).unwrap();
        assert!(cross_check_reflected_fade(&passive, 0.5f32.to_bits(), 3).is_ok());
        assert_eq!(
            cross_check_reflected_fade(&passive, 0.25f32.to_bits(), 3)
                .unwrap_err()
                .reason,
            "xmap07.reflected_cross_check_disagrees"
        );
        assert_eq!(
            cross_check_reflected_fade(&passive, 0.5f32.to_bits(), 0)
                .unwrap_err()
                .reason,
            "xmap07.reflected_cross_check_empty"
        );
        assert_eq!(passive.fade_f32_bits, 0.5f32.to_bits());
    }

    #[test]
    fn a_chain_that_moved_after_discovery_refuses_at_validation() {
        let memory = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&memory).unwrap();
        let sample = copy_snow_mud_fade_sample(&memory, &anchor).unwrap();
        assert_eq!(
            validate_snow_mud_fade_sample(&sample, &anchor, memory.module_base())
                .unwrap()
                .fade_f32_bits,
            0.25f32.to_bits()
        );
        for perturb in [
            |s: &mut SnowMudFadeSample| s.witness_before.root_slot ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_before.root_vptr ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_before.delegate ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_before.map_object ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_before.map_render_state ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_after.root_slot ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_after.root_vptr ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_after.delegate ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_after.map_object ^= 0x1000,
            |s: &mut SnowMudFadeSample| s.witness_after.map_render_state ^= 0x1000,
        ] {
            let mut moved = sample;
            perturb(&mut moved);
            assert_eq!(
                validate_snow_mud_fade_sample(&moved, &anchor, memory.module_base())
                    .unwrap_err()
                    .reason,
                "xmap07.anchor_changed_around_fade_read"
            );
        }
    }

    #[test]
    fn validation_refuses_an_out_of_range_fade_from_the_frozen_sample() {
        let memory = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&memory).unwrap();
        let good = copy_snow_mud_fade_sample(&memory, &anchor).unwrap();
        for bits in [f32::NAN.to_bits(), 2.0f32.to_bits(), (-1.0f32).to_bits()] {
            let mut torn = good;
            torn.fade_f32_bits_first = bits;
            torn.fade_f32_bits_second = bits;
            assert_eq!(
                validate_snow_mud_fade_sample(&torn, &anchor, memory.module_base())
                    .unwrap_err()
                    .reason,
                "xmap07.fade_out_of_range"
            );
        }
    }

    #[test]
    fn a_fade_that_advances_between_reads_refuses() {
        struct AdvancingFade {
            inner: SliceMemory,
            first: std::cell::Cell<bool>,
            address: u64,
        }
        impl PassiveMemory for AdvancingFade {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == self.address && out.len() == 4 {
                    let bits = if self.first.replace(false) {
                        0.25f32.to_bits()
                    } else {
                        0.5f32.to_bits()
                    };
                    out.copy_from_slice(&bits.to_le_bytes());
                    return true;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let inner = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&inner).unwrap();
        let memory = AdvancingFade {
            address: MAP_RENDER_STATE + 0x5ce8,
            inner,
            first: std::cell::Cell::new(true),
        };
        assert_eq!(
            copy_and_validate(&memory, &anchor).unwrap_err().reason,
            "xmap07.fade_advanced_during_read"
        );
    }

    #[test]
    fn the_second_witness_is_copied_after_the_fade_not_before_it() {
        use std::cell::Cell;
        struct DetachesAtTheFadeRead {
            inner: SliceMemory,
            link_addr: u64,
            fade_addr: u64,
            fade_read: Cell<bool>,
        }
        impl PassiveMemory for DetachesAtTheFadeRead {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == self.fade_addr && out.len() == 4 {
                    self.fade_read.set(true);
                    return self.inner.read(addr, out);
                }
                if addr == self.link_addr && out.len() == 8 && self.fade_read.get() {
                    out.copy_from_slice(&(MAP_RENDER_STATE ^ 0x1000).to_le_bytes());
                    return true;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let inner = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&inner).unwrap();
        let memory = DetachesAtTheFadeRead {
            link_addr: anchor.map_object + crate::anchor::MAP_OBJECT_RENDER_STATE_OFFSET,
            fade_addr: MAP_RENDER_STATE + 0x5ce8,
            inner,
            fade_read: Cell::new(false),
        };
        let sample = copy_snow_mud_fade_sample(&memory, &anchor).unwrap();
        assert_ne!(
            sample.witness_before, sample.witness_after,
            "the second witness must observe the chain as it stands AFTER the fade read"
        );
        assert_eq!(
            validate_snow_mud_fade_sample(&sample, &anchor, memory.module_base())
                .unwrap_err()
                .reason,
            ANCHOR_CHANGED_REFUSAL
        );
    }

    #[test]
    fn an_unreadable_link_and_an_unreadable_fade_refuse_by_different_names() {
        struct Unreadable {
            inner: SliceMemory,
            addr: u64,
        }
        impl PassiveMemory for Unreadable {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == self.addr {
                    return false;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let inner = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&inner).unwrap();

        let link_gone = Unreadable {
            addr: anchor.map_object + crate::anchor::MAP_OBJECT_RENDER_STATE_OFFSET,
            inner: memory_with_fade(0.25f32.to_bits()),
        };
        assert_eq!(
            copy_snow_mud_fade_sample(&link_gone, &anchor)
                .unwrap_err()
                .reason,
            WITNESS_UNREADABLE_REFUSAL
        );

        let fade_gone = Unreadable {
            addr: MAP_RENDER_STATE + 0x5ce8,
            inner,
        };
        assert_eq!(
            copy_snow_mud_fade_sample(&fade_gone, &anchor)
                .unwrap_err()
                .reason,
            "xmap07.fade_word_unreadable"
        );
    }

    #[test]
    fn the_first_witness_is_copied_before_the_fade_not_after_it() {
        use std::cell::Cell;
        struct ReattachesAtTheFadeRead {
            inner: SliceMemory,
            link_addr: u64,
            fade_addr: u64,
            fade_read: Cell<bool>,
        }
        impl PassiveMemory for ReattachesAtTheFadeRead {
            fn read(&self, addr: u64, out: &mut [u8]) -> bool {
                if addr == self.fade_addr && out.len() == 4 {
                    self.fade_read.set(true);
                    return self.inner.read(addr, out);
                }
                if addr == self.link_addr && out.len() == 8 && !self.fade_read.get() {
                    out.copy_from_slice(&(MAP_RENDER_STATE ^ 0x1000).to_le_bytes());
                    return true;
                }
                self.inner.read(addr, out)
            }
            fn module_base(&self) -> u64 {
                self.inner.module_base()
            }
        }
        let inner = memory_with_fade(0.25f32.to_bits());
        let anchor = resolve_map_render_anchor(&inner).unwrap();
        let memory = ReattachesAtTheFadeRead {
            link_addr: anchor.map_object + crate::anchor::MAP_OBJECT_RENDER_STATE_OFFSET,
            fade_addr: MAP_RENDER_STATE + 0x5ce8,
            inner,
            fade_read: Cell::new(false),
        };
        let sample = copy_snow_mud_fade_sample(&memory, &anchor).unwrap();
        assert_ne!(
            sample.witness_before, sample.witness_after,
            "the first witness must observe the chain as it stands BEFORE the fade read"
        );
        assert_eq!(
            validate_snow_mud_fade_sample(&sample, &anchor, memory.module_base())
                .unwrap_err()
                .reason,
            ANCHOR_CHANGED_REFUSAL
        );
    }
}
