
#[derive(Clone)]
pub struct Budget {
    remaining: usize,
    steady: bool,
    cut: bool,
    floor: usize,
    made: usize,
    held: usize,
}

impl Budget {
    pub fn read(defines: &content_reader_defines::defines::Defines) -> std::io::Result<i32> {
        defines.int32("NGraphics", "MAX_MESHES_LOADED_PER_FRAME")
    }

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

    pub fn with_floor(mut self, floor: usize) -> Self {
        self.set_floor(floor);
        self
    }

    pub fn set_floor(&mut self, floor: usize) {
        self.floor = floor;
    }

    pub fn cut(&mut self) {
        self.cut = true;
    }

    pub fn is_cut(&self) -> bool {
        self.cut
    }

    pub fn holding(&self) -> bool {
        self.cut && self.made >= self.floor
    }

    pub fn remaining(&self) -> usize {
        if self.steady {
            usize::MAX
        } else if self.holding() {
            0
        } else {
            self.remaining
        }
    }

    pub fn held(&self) -> usize {
        self.held
    }

    pub fn hold(&mut self, creations: usize) {
        self.held += creations;
    }

    pub fn charge(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
        self.made += 1;
    }

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
            content_reader_layers::ResolvedLayers::from_folders(&[(&base, &[]), (&addon, &[])]).unwrap();
        let defines = content_reader_defines::defines::read(&layers).unwrap();
        let mut budget = Budget::new(Budget::read(&defines).unwrap(), false);
        assert_eq!((0..5).filter(|_| budget.take()).count(), 3);
        let defines = content_reader_defines::defines::Defines::new().unwrap();
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
