
use super::{Drawn, Values, binding::Bindings, gui, gui_resources};
use std::io;
use viewer_native_gui::native_gui::{self, lifecycle, pointer::Input};

pub const CONFIRMATION_WINDOW: &str = "default_confirmation_popup";
pub const TEMPLATES: &[&[u8]] = &[b"default_confirmation_popup"];

const POPUP_LAYERS: u32 = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Accept,
    Decline,
    Window,
}

fn control(name: &str) -> Option<Control> {
    match name {
        "accept_button" => Some(Control::Accept),
        "decline_button" => Some(Control::Decline),
        _ => None,
    }
}

#[derive(Clone, Default)]
pub struct ConfirmationDrawn {
    pub drawn: Drawn,
    pub controls: Vec<([f32; 4], Control)>,
    pub clicked: Vec<(Control, usize)>,
}

impl ConfirmationDrawn {
    pub fn control_at(&self, point: [f32; 2]) -> Option<Control> {
        self.controls
            .iter()
            .rev()
            .find(|(rect, _)| viewer_runtime::geometry::contains(*rect, point))
            .map(|(_, control)| *control)
    }
}

#[derive(Default)]
pub struct Confirmation {
    built: Option<([i32; 2], viewer_native_gui::native_windows::NativeWindows)>,
    root: Option<gui::Retained>,
    generation: u64,
    texts: Option<(String, String)>,
}

impl Confirmation {
    pub fn holding(&self) -> bool {
        self.root.as_ref().is_some_and(gui::Retained::holding)
    }

    pub fn pointing(&self) -> bool {
        self.root.as_ref().is_some_and(gui::Retained::pointing)
    }

    #[expect(clippy::too_many_arguments, reason = "the window's texts, extent, clock and pointer, each from another owner")]
    pub fn draw(
        &mut self,
        content: &super::content::Content,
        title: &str,
        description: &str,
        extent: [f32; 2],
        hovers: bool,
        time: f32,
        advance: bool,
        input: Option<Input>,
    ) -> io::Result<ConfirmationDrawn> {
        let viewport = extent.map(|value| value as i32);
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        if self
            .built
            .as_ref()
            .is_none_or(|(size, built)| *size != viewport || !built.same(&windows))
        {
            *self = Self { built: Some((viewport, windows.clone())), ..Self::default() };
        }
        if self.texts.as_ref().is_none_or(|(held, body)| held != title || body != description) {
            self.texts = Some((title.to_owned(), description.to_owned()));
            self.generation += 1;
        }
        let assets = content.assets.as_ref();
        let inventory = gui_resources::GuiAssets::new(assets, &Bindings::default());
        let resources = gui_resources::GuiResources::new(
            assets,
            &windows,
            &content.localisation,
            &content.small_flags,
            &content.large_flags,
            &inventory,
        );
        let mut out = ConfirmationDrawn::default();
        let root = gui::template(&windows, CONFIRMATION_WINDOW).map_err(super::invalid)?;
        let mut values = Values::new();
        values.entry("title".into()).or_default().text = Some(title.to_owned());
        values.entry("description".into()).or_default().text = Some(description.to_owned());
        let retained = gui::ensure(&mut self.root, CONFIRMATION_WINDOW, root, viewport, &resources)
            .map_err(super::invalid)?;
        let frame = native_gui::container_frame(root, Some(viewport)).map_err(super::invalid)?;
        retained.instance.set_position(root, root, frame.at).map_err(super::invalid)?;
        let Some(prepared) = retained.prepare(
            root,
            &values,
            Vec::new(),
            &windows,
            &resources,
            lifecycle::Tick { time, dt: 0.0 },
            self.generation,
            advance,
            hovers,
            true,
            input.as_ref().map(|input| input.local(frame.at.map(|value| value as f32))),
            &mut out.drawn.refused,
        ) else {
            return Ok(out);
        };
        out.drawn.sounds.extend(prepared.sounds.iter().cloned());
        let origin = frame.at;
        if !gui::draw_prepared(&prepared.layout, &prepared.items, origin, &resources, &mut out.drawn) {
            return Ok(out);
        }
        for (index, quad) in out.drawn.batch.quads.iter_mut().enumerate() {
            quad.layer = viewer_native_gui::sprites::gui_layer(POPUP_LAYERS.saturating_add(u32::try_from(index).unwrap_or(u32::MAX)), 0);
        }
        let named: std::collections::BTreeMap<_, _> = gui::elements(root)
            .into_iter()
            .map(|(path, element)| (element.path.clone(), path))
            .collect();
        out.controls.push((
            [origin[0] as f32, origin[1] as f32, prepared.layout.size[0] as f32, prepared.layout.size[1] as f32],
            Control::Window,
        ));
        for region in &prepared.layout.regions {
            if let Some(control) = named.get(&region.path).and_then(|name| control(name))
                && let Some(rect) = gui::region_rect(region, origin)
            {
                out.controls.push((rect, control));
            }
        }
        out.clicked.extend(prepared.clicks.iter().filter_map(|(key, path, order)| {
            key.is_none().then_some(())?;
            Some((control(named.get(path)?)?, *order))
        }));
        if hovers {
            gui::append_hovers(&mut out.drawn.tooltips, prepared.hovers, origin);
        }
        Ok(out)
    }

    pub fn shortcuts(content: &super::content::Content, extent: [f32; 2]) -> io::Result<Vec<(Control, String)>> {
        let windows = content.window_cache.at(extent.map(|value| value as u32))?;
        let root = gui::template(&windows, CONFIRMATION_WINDOW).map_err(super::invalid)?;
        Ok(gui::elements(root)
            .into_iter()
            .filter_map(|(name, element)| {
                let shortcut = element.button()?.shortcuts.first()?;
                Some((control(&name)?, std::str::from_utf8(shortcut).ok()?.to_owned()))
            })
            .collect())
    }
}
