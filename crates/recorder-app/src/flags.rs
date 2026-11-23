use content_reader_layers::{ResolveOptions, ResolvedLayers};
use content_reader_gui::flags::FlagSet;
use content_reader_image::Rgba8Image;
use replay_data::identity::Identity;
use serde::Serialize;
use std::io;
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

pub(crate) struct BackgroundPriority;

impl BackgroundPriority {
    pub(crate) fn enter() -> io::Result<Self> {
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
}

const MAX_FLAG_EDGE: u32 = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FlagImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn flag_keys(player: &serde_json::Value) -> Option<Vec<String>> {
    let text = |key: &str| player[key].as_str();
    text("tag")?;
    let keys = content_reader_country_names::country_names::flag_keys(
        text("original"),
        text("cosmetic")?,
        Some(text("group")?),
    );
    (!keys.is_empty()).then_some(keys)
}

fn safe_flag_key(key: &str) -> bool {
    key.split('/').all(|part| {
        !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', '\0', ':'])
    })
}

pub struct FlagContent {
    layers: ResolvedLayers,
    cell: [u32; 2],
}

impl FlagContent {
    pub fn load(identity: &Identity, options: &ResolveOptions) -> io::Result<Self> {
        let content = content_reader_layers::resolve(identity, options)?.exact()?;
        let defines = content_reader_defines::defines::read(&content.layers)?;
        Ok(Self {
            cell: FlagSet::Normal.cell(&defines)?,
            layers: content.layers,
        })
    }

    pub fn find(&self, keys: &[String]) -> io::Result<Option<FlagImage>> {
        search(keys, |asset| match self.layers.read_uncovered(asset) {
            Ok(bytes) => Ok(content_reader_gui::flags::decode_checked(&bytes, self.cell).ok()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        })
    }
}

fn search(
    keys: &[String],
    mut read: impl FnMut(&str) -> io::Result<Option<Rgba8Image>>,
) -> io::Result<Option<FlagImage>> {
    for key in keys {
        if !safe_flag_key(key) {
            return Ok(None);
        }
        let asset = format!("{}{key}.tga", FlagSet::Normal.directory());
        if let Some(image) = read(&asset)? {
            return Ok(Some(bounded(&image)));
        }
    }
    Ok(None)
}

fn bounded(image: &Rgba8Image) -> FlagImage {
    let scale = (image.width.max(image.height) as f64 / MAX_FLAG_EDGE as f64).max(1.0);
    let width = ((image.width as f64 / scale).round() as u32).max(1);
    let height = ((image.height as f64 / scale).round() as u32).max(1);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at = (((y * image.height / height) * image.width + x * image.width / width) * 4)
                as usize;
            pixels.extend_from_slice(&image.pixels[at..at + 4]);
        }
    }
    FlagImage {
        width,
        height,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    fn image(width: u32, height: u32, value: u8) -> Rgba8Image {
        Rgba8Image {
            width,
            height,
            pixels: vec![value; (width * height * 4) as usize],
        }
    }

    #[test]
    fn published_operands_become_the_games_flag_keys() {
        let player = json!({"tag":"OWN","cosmetic":"variants/red","original":"ORIGINAL","group":"synthetic_group"});
        assert_eq!(
            flag_keys(&player).unwrap(),
            [
                "variants/red_synthetic_group",
                "variants/red",
                "ORIGINAL_synthetic_group",
                "ORIGINAL"
            ]
        );
        let player = json!({"tag":"OWN","cosmetic":"","original":null,"group":"g"});
        assert_eq!(flag_keys(&player), None);
        let player = json!({"tag":"OWN","cosmetic":"","original":"OWN","group":"g"});
        assert_eq!(flag_keys(&player).unwrap(), ["OWN_g", "OWN"]);
        assert_eq!(flag_keys(&serde_json::Value::Null), None);
        assert_eq!(
            flag_keys(&json!({"tag":"OWN","original":"OWN","group":"g"})),
            None
        );
    }

    #[test]
    fn the_search_takes_the_first_decodable_candidate_in_the_flag_folder() {
        let keys: Vec<String> = ["COSMETIC_g", "COSMETIC", "TAG_g", "TAG"]
            .map(String::from)
            .to_vec();
        let asked = RefCell::new(Vec::new());
        let found = search(&keys, |asset| {
            asked.borrow_mut().push(asset.to_owned());
            Ok((asset == "gfx/flags/COSMETIC.tga").then(|| image(82, 52, 7)))
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            asked.into_inner(),
            ["gfx/flags/COSMETIC_g.tga", "gfx/flags/COSMETIC.tga"]
        );
        assert_eq!((found.width, found.height), (82, 52));
        let unsafe_keys = ["../escape".to_owned(), "TAG".to_owned()];
        assert_eq!(
            search(&unsafe_keys, |_| Ok(Some(image(82, 52, 1)))).unwrap(),
            None
        );
        assert_eq!(search(&keys, |_| Ok(None)).unwrap(), None);
    }

    #[test]
    fn large_mod_flags_are_bounded_for_the_panel() {
        let flag = bounded(&image(1024, 640, 9));
        assert_eq!((flag.width, flag.height), (128, 80));
        assert_eq!(flag.pixels.len(), 128 * 80 * 4);
        assert!(flag.pixels.iter().all(|&value| value == 9));
        let small = bounded(&image(82, 52, 3));
        assert_eq!((small.width, small.height), (82, 52));
    }
}
