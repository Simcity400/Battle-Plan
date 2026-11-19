//! Shared content-defined allowance for mesh creations per frame.
//! The frame resets the counter, then visits provinces in ascending ID order,
//! updating each province's buildings before its units.
//!
//! Buildings and hulls test and increment the counter inline. Land creations
//! queue against the remaining allowance and run after the province walk;
//! their final creation has no budget guard and can exceed the allowance.
//! Held show bits retry on later frames. Naval combat graphics also use the
//! shared allowance, but their hulls are not drawn by this layer.

/// One viewer frame's creation allowance. Offline dumps settle every
/// admitted model whatever the allowance. A copy is what the actors' judge spends
/// ([`crate::map::actor_check`]), so a walk it runs beside the frame's spends nothing.
///
/// Beside the content's count the frame has an allowance of its own
/// ([`viewer_foundation::map::frame_budget`]): a frame that has spent its room for what an hour
/// brings is [`Budget::cut`], and what it then refuses waits for a later frame, as the game's own
/// spent counter makes it wait. The walk is cut only between provinces, so a visit and the judge's
/// copy of it always see the same allowance.
#[derive(Clone)]
pub struct Budget {
    remaining: usize,
    steady: bool,
    /// The frame has spent its own room for what an hour brings.
    cut: bool,
    /// Creations to make before the cut can bite, which the walk sets from what it held back on
    /// the frames before: what one frame refuses is made within [`frame_budget::BOUND`] frames.
    floor: usize,
    made: usize,
    /// Creations the cut refused, for the next frame's floor.
    held: usize,
}

impl Budget {
    /// The content's allowance.
    pub fn read(defines: &installed_content_core::defines::Defines) -> std::io::Result<i32> {
        defines.int32("NGraphics", "MAX_MESHES_LOADED_PER_FRAME")
    }

    /// A frame's allowance; a define of zero or less creates nothing live.
    pub fn new(limit: i32, steady: bool) -> Self {
        Self {
            remaining: limit.max(0) as usize,
            steady,
            cut: false,
            floor: usize::MAX,
            made: 0,
            held: 0,
        }
    }

    /// The creations this frame makes before the frame's own room can cut it short.
    pub fn with_floor(mut self, floor: usize) -> Self {
        self.set_floor(floor);
        self
    }

    pub fn set_floor(&mut self, floor: usize) {
        self.floor = floor;
    }

    /// The frame has spent its room for what an hour brings: from here it creates only what its
    /// floor still owes.
    pub fn cut(&mut self) {
        self.cut = true;
    }

    pub fn is_cut(&self) -> bool {
        self.cut
    }

    /// The frame's own room is spent and its floor is met: what is left waits for a later frame.
    /// What the content's own count refuses is not this, and is not carried.
    pub fn holding(&self) -> bool {
        self.cut && self.made >= self.floor
    }

    /// The remaining allowance for deferred land creations.
    pub fn remaining(&self) -> usize {
        if self.steady {
            usize::MAX
        } else if self.holding() {
            0
        } else {
            self.remaining
        }
    }

    /// Creations the frame's own room refused, which the next frame's floor takes up.
    pub fn held(&self) -> usize {
        self.held
    }

    /// Counts creations the frame's room refused, for the next frame's floor.
    pub fn hold(&mut self, creations: usize) {
        self.held += creations;
    }

    /// Counts a creation that does not test the allowance.
    pub fn charge(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
        self.made += 1;
    }

    /// Spends one creation if any is left, as the inline creators do.
    pub fn take(&mut self) -> bool {
        if self.steady {
            return true;
        }
        if self.holding() {
            self.held += 1;
            return false;
        }
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        self.made += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_stack_define_controls_creation_and_missing_reads_zero() {
        let root = std::env::temp_dir().join(format!("hoi4_mesh_budget_{}", std::process::id()));
        let base = root.join("base");
        let addon = root.join("mod");
        for dir in [&base, &addon] {
            std::fs::create_dir_all(dir.join("common/defines")).unwrap();
        }
        std::fs::write(
            base.join("common/defines/00_base.lua"),
            b"NDefines={NGraphics={MAX_MESHES_LOADED_PER_FRAME=31}}",
        )
        .unwrap();
        std::fs::write(
            addon.join("common/defines/zz_mod.lua"),
            b"NDefines.NGraphics.MAX_MESHES_LOADED_PER_FRAME=1+2",
        )
        .unwrap();
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let defines = installed_content_core::defines::read(&layers).unwrap();
        let mut budget = Budget::new(Budget::read(&defines).unwrap(), false);
        assert_eq!((0..5).filter(|_| budget.take()).count(), 3);
        let defines = installed_content_core::defines::Defines::new().unwrap();
        for file in [
            base.join("common/defines/00_base.lua"),
            addon.join("common/defines/zz_mod.lua"),
        ] {
            defines
                .load(file.to_str().unwrap(), &std::fs::read(&file).unwrap())
                .unwrap();
        }
        defines
            .load(
                "missing.lua",
                b"NDefines.NGraphics.MAX_MESHES_LOADED_PER_FRAME=nil",
            )
            .unwrap();
        // The native integer getter stores 0 for a nil define, which
        // creates nothing live.
        assert_eq!(Budget::read(&defines).unwrap(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_cut_frame_makes_its_floor_then_holds_what_is_left_and_counts_it() {
        let mut budget = Budget::new(10, false).with_floor(2);
        budget.cut();
        assert!(budget.take(), "the floor is made whatever the frame's clock says");
        assert_eq!(budget.remaining(), 9, "and while it is owed the land queue is the content's");
        assert!(budget.take());
        assert!(!budget.take(), "past the floor a cut frame creates nothing");
        assert_eq!(budget.remaining(), 0, "and queues no land");
        assert_eq!(budget.held(), 1);
        budget.hold(4);
        assert_eq!(budget.held(), 5, "what the walk refused to queue counts too");
        let mut whole = Budget::new(2, false);
        assert_eq!((0..5).filter(|_| whole.take()).count(), 2, "an uncut frame spends the content's count");
        assert_eq!(whole.held(), 0);
        let mut dump = Budget::new(0, true).with_floor(0);
        dump.cut();
        assert!(dump.take(), "a dump settles every model whatever the frame spent");
    }

    #[test]
    fn zero_and_negative_block_live_creations_but_dumps_settle() {
        for limit in [0, -7] {
            assert!(!Budget::new(limit, false).take());
            assert!(Budget::new(limit, true).take());
        }
    }
}
