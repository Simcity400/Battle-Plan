use content_reader_settings::{
    settings::Clamp, settings::Desktop, settings::Display, settings::DisplayMode,
};

pub fn borderless() -> Display {
    borderless_at(1.0)
}

pub(crate) fn borderless_at(gui_scale: f32) -> Display {
    Display {
        clamps: vec![Clamp {
            gui_scale: Some(gui_scale),
            size: Some([2048, 1152]),
            borderless: true,
        }],
        size: Some([2560, 1440]),
        min_gui: [1280, 720],
        mode: DisplayMode::Borderless,
        display_index: 0,
    }
}

pub const MONITOR: Desktop = Desktop {
    physical: [2560, 1440],
    dpi: Ok(120),
};
