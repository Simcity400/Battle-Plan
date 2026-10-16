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
/// admitted model whatever the allowance.
pub struct Budget {
    remaining: usize,
    steady: bool,
}

impl Budget {
    /// The content's allowance.
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

    /// The remaining allowance for deferred land creations.
    pub fn remaining(&self) -> usize {
        if self.steady {
            usize::MAX
        } else {
            self.remaining
        }
    }

    /// Counts a creation that does not test the allowance.
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
        let defines = installed_content::defines::Defines::new().unwrap();
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
    fn zero_and_negative_block_live_creations_but_dumps_settle() {
        for limit in [0, -7] {
            assert!(!Budget::new(limit, false).take());
            assert!(Budget::new(limit, true).take());
        }
    }
}
