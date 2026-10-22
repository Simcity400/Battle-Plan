//! A synthetic game with installed visual mods, for tests of art that follows the selection: a
//! test writes the recorded files and each mod's own, opens a recording over them, and selects
//! mods as the service does from a campaign's settings.

use crate::{Source, presentation::Presentation, products::ContentOptions};
use installed_content_core::content::visual::Choice;
use installed_content_image::Rgba8Image;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];
pub const GREEN: [u8; 4] = [0, 255, 0, 255];

/// A game folder (its user folder inside) and, beside it, the folder installed mods live in;
/// both are removed when it drops.
pub struct Machine {
    pub root: PathBuf,
    pub mods: PathBuf,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.mods);
    }
}

impl Machine {
    /// An empty game named for the test, with the sprite program artwork is drawn through.
    pub fn new(name: &str) -> Self {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let root = std::env::temp_dir().join(format!(
            "hoi4_visual_mods_{name}_{}",
            std::process::id()
        ));
        let mods = root.with_file_name(format!(
            "hoi4_visual_mods_{name}_{}_mods",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&mods);
        std::fs::create_dir_all(&root).unwrap();
        dashboard_text::catalogue::texture_tests::write_sprite_program(&root);
        Self { root, mods }
    }

    /// Writes a file of the recorded game.
    pub fn write(&self, path: &str, bytes: &[u8]) {
        write(&self.root, path, bytes);
    }

    /// Writes a file of the folder mod `name`, installed as `mod/{name}.mod`.
    pub fn write_mod(&self, name: &str, path: &str, bytes: &[u8]) {
        let folder = self.mods.join(name);
        write(&folder, path, bytes);
        write(
            &self.root,
            &format!("user/mod/{name}.mod"),
            format!(
                "name=\"{name}\"\npath=\"{}\"\n",
                folder.display().to_string().replace('\\', "/")
            )
            .as_bytes(),
        );
    }

    /// A recording over the game, unbound until [`select`] shows a selection on it.
    pub fn open(&self) -> (Source, ContentOptions) {
        self.open_frames(vec![super::source(1, 100, Some(1))])
    }

    /// [`Self::open`] over `frames`.
    pub fn open_frames(
        &self,
        frames: Vec<replay_frame::frame::DecodedFrame>,
    ) -> (Source, ContentOptions) {
        let (source, context) = super::source_with_content(frames, &self.root);
        (source, context.content.unwrap())
    }
}

/// Shows the mods `names` on `source`, as the service does from its campaign's settings, and
/// admits the next request of content under them, its mount resolved at its entry.
pub fn select(source: &Source, options: &mut ContentOptions, names: &[&str]) {
    let selected: Vec<Choice> = names
        .iter()
        .map(|name| Choice {
            id: format!("mod/{name}.mod"),
            name: (*name).into(),
        })
        .collect();
    let presentation = Presentation::current(
        &options.options,
        source.replay_ref(),
        &selected,
        &AtomicBool::new(false),
    )
    .unwrap();
    source.bind(
        Some((&options.options, options.allow_unverified)),
        presentation,
    );
    *options = source
        .admit_resolved(crate::source::Work::Content(Arc::new(AtomicBool::new(false))))
        .unwrap();
}

/// A one-pixel PNG of `rgba`.
pub fn png(rgba: [u8; 4]) -> Vec<u8> {
    solid(1, 1, rgba)
}

/// A `width` × `height` PNG of one colour.
pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    dashboard_text::catalogue::texture_tests::png_bytes(&Rgba8Image {
        width,
        height,
        pixels: rgba.repeat((width * height) as usize),
    })
}

/// The first pixel of a PNG data URL.
pub fn first_pixel(url: &str) -> [u8; 4] {
    let image = dashboard_text::catalogue::texture_tests::from_url(url);
    image.pixels[..4].try_into().unwrap()
}

/// What `run` names as drawn in the recording's style, as its reply's line says it.
pub fn fallback_line<T>(run: impl FnOnce() -> T) -> (T, Option<String>) {
    let (value, own) = dashboard_text::catalogue::capture(run);
    (value, own.fallback_line())
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
