//! `NGraphics.MAX_MESHES_LOADED_PER_FRAME`, the map entities the game
//! creates per frame. `CGraphicalMap::update@140B49340` zeroes the counter
//! `1433232A4` before `140B4C120` updates the provinces, in ascending ID
//! order (`140B3F5C0`, comparator `140B3E900`), each province's buildings
//! (`14127B2E0`) before its units (`14127D260`).
//!
//! It is not a hard cap. Buildings (`141663E70`) and hulls (`1416542A0`) test
//! and increment inline. `141277810` queues land creations against what is
//! left, in each province, without reserving it; after the province loop
//! `140B3E400` resolves them and `141276C20` creates each changed definition
//! with no further test, so land can take the total past the define.
//!
//! The counter's other readers and writers (`14127C910`'s show-bit hold) are
//! unit ones; trees, cities, aircraft and trains do not use it. Naval combat
//! graphics attach hulls through `1416542A0` too (`1416535A0` ->
//! `14164F560`); the viewer does not draw them, so they spend none of it.

/// One viewer frame's creation allowance. Offline dumps settle every
/// admitted model whatever the allowance.
pub(super) struct Budget {
    remaining: usize,
    steady: bool,
}

impl Budget {
    /// The content's allowance (loader `14084FEE0`).
    pub fn read(defines: &installed_content::defines::Defines) -> std::io::Result<i32> {
        defines.int32("NGraphics", "MAX_MESHES_LOADED_PER_FRAME")
    }

    /// A frame's allowance; a define of zero or less creates nothing live.
    pub fn new(limit: i32, steady: bool) -> Self {
        Self {
            remaining: limit.max(0) as usize,
            steady,
        }
    }

    /// What is left, as `141277810` reads it.
    pub fn remaining(&self) -> usize {
        if self.steady {
            usize::MAX
        } else {
            self.remaining
        }
    }

    /// Counts a creation that does not test the allowance (`141276C20`).
    pub fn charge(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
    }

    /// Spends one creation if any is left, as the inline creators do.
    pub fn take(&mut self) -> bool {
        if self.steady {
            return true;
        }
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
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
            installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let defines = installed_content::defines::read(&layers).unwrap();
        let mut budget = Budget::new(Budget::read(&defines).unwrap(), false);
        assert_eq!((0..5).filter(|_| budget.take()).count(), 3);
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
    fn zero_and_negative_block_live_creations_but_dumps_settle() {
        for limit in [0, -7] {
            assert!(!Budget::new(limit, false).take());
            assert!(Budget::new(limit, true).take());
        }
    }
}
