//! a729 container content scissor production and the generic draw's clip gate.
//!
//! Layout supplies native logical getter values. These are not sprite bounds.
//! Container writes precede graphical-tree inheritance, which precedes 142228c20
//! and both sprite getters. Repeated raster passes consume the admitted result.
//! Candidate native ancestry/inheritance remains uncorroborated:
//! extractor/native/runs/arrow_native_ui_20260908/ROOT_GEOMETRY_CANDIDATES.json.

use crate::gui::GuiElement;
use crate::paradox_text::Value;
use crate::variant_admission::{DisplayMode, DisplayVariantKey, PlaybackUiProfile};

/// Result of the native display-index-zero query at settings-read time.
/// Missing acquisition is distinct from the native query returning failure.
/// On Windows, the native display list enumerates primary monitors first;
/// its stored desktop mode uses EnumDisplaySettingsW(CURRENT_SETTINGS), then
/// CreateDCW/GetDeviceCaps(HORZRES, VERTRES) when opening that display succeeds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayZeroDesktopMode {
    /// Host has not acquired the native lookup's result.
    Unavailable,
    /// Known nonzero native query return, not merely a host acquisition error.
    QueryFailed,
    /// Width and height from the first successfully registered display.
    Mode([u32; 2]),
}

/// Borrowed result of the successful graphics-block read's +40 finalizer.
/// The authored profile remains unchanged. Private fields prevent treating its
/// gui_scale_f32_bits as the already effective native settings+d4 value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EffectiveGuiScale<'profile> {
    profile: &'profile DisplayVariantKey,
    value: f32,
}

impl<'profile> EffectiveGuiScale<'profile> {
    /// 142215ee0 token c1 -> graphics+18 (1424a9730) -> successful-read +40
    /// (142214530). `borderless` is the actual authored boolean, independent
    /// of fullScreen; DisplayMode alone loses the both-true combination.
    /// The selected profile's size must be the actual bound local viewport.
    pub fn after_graphics_settings_read(
        profile: &'profile DisplayVariantKey,
        current_physical_viewport: [u32; 2],
        borderless: bool,
        desktop: DisplayZeroDesktopMode,
    ) -> Result<Self, ClipGraphicsRefusal> {
        let physical = bound_physical_extent(profile, current_physical_viewport)?;
        if matches!(profile.display_mode, DisplayMode::Windowed) && borderless
            || matches!(profile.display_mode, DisplayMode::BorderlessFullscreen) && !borderless
        {
            return Err(ClipGraphicsRefusal::WindowModeBindingMismatch);
        }
        let authored = f32::from_bits(profile.gui_scale_f32_bits);
        if !authored.is_finite() || authored <= 0.0 {
            return Err(ClipGraphicsRefusal::InvalidGuiScale);
        }
        // +e1 alone selects the query, even when fullScreen (+e0) is true.
        // A nonzero native query return retains the settings+bc/c0 extents.
        let [width, height] = if borderless {
            match desktop {
                DisplayZeroDesktopMode::Unavailable => {
                    return Err(ClipGraphicsRefusal::DesktopModeUnavailable);
                }
                DisplayZeroDesktopMode::QueryFailed => physical,
                DisplayZeroDesktopMode::Mode(size) => {
                    positive_extent(size).ok_or(ClipGraphicsRefusal::InvalidDesktopExtent)?
                }
            }
        } else {
            physical
        };
        // 142214584..5be: DIVSS, MINSS, MULSS, CVTTSS2SI, CVTDQ2PS,
        // DIVSS. Positive finite integer extents make MINSS ties immaterial.
        let width_ratio = width as f32 / 1280.0_f32;
        let height_ratio = height as f32 / 720.0_f32;
        let maximum = cvttss2si(width_ratio.min(height_ratio) * 10.0_f32) as f32 / 10.0_f32;
        // This is ordered branching, not clamp(0.5, maximum): the lower
        // branch returns immediately even when maximum is less than 0.5.
        let value = if authored < 0.5_f32 {
            0.5_f32
        } else if authored > maximum {
            maximum
        } else {
            authored
        };
        if !value.is_finite() || value <= 0.0 {
            return Err(ClipGraphicsRefusal::InvalidEffectiveGuiScale);
        }
        Ok(Self { profile, value })
    }

    pub fn value(self) -> f32 {
        self.value
    }
}

/// The graphics generation used to lay out and draw the admitted hierarchy.
/// 1422206c0 reads effective settings, runs device setup, then computes these
/// values. Ordinary SDL window resize events retain them: 14234cf00 routes
/// subtype 5 to CPdxWindow+18 (14220d770), which only stores an activity flag.
/// The SDL window's current size is separate from the graphics clip extent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeClipGraphics {
    pub logical_size: [i32; 2],
    pub physical_size: [i32; 2],
    pub logical_to_physical: [f32; 2],
    construction_profile: DisplayVariantKey,
    effective_gui_scale: f32,
    window_physical_size: [i32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipGraphicsRefusal {
    InvalidPhysicalExtent,
    InvalidMinimumGuiExtent,
    InvalidGuiScale,
    InvalidEffectiveGuiScale,
    WindowModeBindingMismatch,
    BorderlessSettingUnavailable,
    DesktopModeUnavailable,
    InvalidDesktopExtent,
    ViewportChangedWithoutGraphicsConstruction,
    InvalidNativeLogicalExtent,
}

impl NativeClipGraphics {
    /// Construct once from the actual admitted local profile and an explicit
    /// desktop lookup result. Later window resizes use this generation's
    /// after_window_resize transition without reading or finalizing settings.
    pub fn from_playback_profile(
        profile: &PlaybackUiProfile,
        desktop: DisplayZeroDesktopMode,
        current_physical_viewport: [u32; 2],
    ) -> Result<Self, ClipGraphicsRefusal> {
        let borderless = profile
            .raw_borderless()
            .ok_or(ClipGraphicsRefusal::BorderlessSettingUnavailable)?;
        let effective = EffectiveGuiScale::after_graphics_settings_read(
            profile.key(),
            current_physical_viewport,
            borderless,
            desktop,
        )?;
        Self::from_graphics_construction(effective, current_physical_viewport)
    }

    /// Consume the explicit effective-scale result for the borrowed profile.
    /// Physical size must equal successful device setup. No later settings
    /// setter or display change may intervene between the two modeled stages.
    /// This is the constructor law, not a claim about an in-place resize.
    pub fn from_graphics_construction(
        effective: EffectiveGuiScale<'_>,
        current_physical_viewport: [u32; 2],
    ) -> Result<Self, ClipGraphicsRefusal> {
        let profile = effective.profile;
        let [pw, ph] = bound_physical_extent(profile, current_physical_viewport)?;
        let [mut mw, mut mh] = positive_extent([profile.min_gui.width, profile.min_gui.height])
            .ok_or(ClipGraphicsRefusal::InvalidMinimumGuiExtent)?;
        // 142220ada..142220b24: signed IMUL, comparison and IDIV. Retain
        // overflow of the 32-bit products instead of comparing float ratios.
        let width_product = mw.wrapping_mul(ph);
        let height_product = mh.wrapping_mul(pw);
        if height_product < width_product {
            if pw < mw {
                mh = width_product / pw;
            } else {
                [mw, mh] = [pw, ph];
            }
        } else if ph < mh {
            mw = height_product / ph;
        } else {
            [mw, mh] = [pw, ph];
        }
        // 142220b27..b5d: DIVSS once, MULSS then CVTTSS2SI per axis.
        let reciprocal = 1.0_f32 / effective.value;
        let logical_size = [mw, mh].map(|v| cvttss2si(v as f32 * reciprocal));
        if logical_size.iter().any(|v| *v <= 0) {
            return Err(ClipGraphicsRefusal::InvalidNativeLogicalExtent);
        }
        // 142220c7b..cca: two DIVSS operations per axis, with an f32
        // rounding between them. Direct physical/logical can differ by 1 ULP.
        let inverse_x = logical_size[0] as f32 / pw as f32;
        let inverse_y = logical_size[1] as f32 / ph as f32;
        Ok(Self {
            logical_size,
            physical_size: [pw, ph],
            logical_to_physical: [1.0_f32 / inverse_x, 1.0_f32 / inverse_y],
            construction_profile: profile.clone(),
            effective_gui_scale: effective.value,
            window_physical_size: [pw, ph],
        })
    }

    /// Stage an ordinary positive client-size event on the existing generation.
    /// 1420b2820 -> 14210ab30 updates SDL window+28/+2c and emits subtypes 6/5.
    /// The engine ignores 6; subtype 5 only marks CPdxWindow active. Neither
    /// runs the graphics-settings finalizer or reconstructs CGraphics.
    /// Settings changes, device reconstruction and display-mode changes require
    /// their own construction operation; this method does not model those.
    /// The caller commits this candidate only with its successful UI operation.
    pub fn after_window_resize(
        &self,
        current_physical_viewport: [u32; 2],
    ) -> Result<Self, ClipGraphicsRefusal> {
        let window_physical_size = positive_extent(current_physical_viewport)
            .ok_or(ClipGraphicsRefusal::InvalidPhysicalExtent)?;
        let mut candidate = self.clone();
        candidate.window_physical_size = window_physical_size;
        Ok(candidate)
    }

    pub fn construction_profile(&self) -> &DisplayVariantKey {
        &self.construction_profile
    }

    pub fn effective_gui_scale(&self) -> f32 {
        self.effective_gui_scale
    }

    pub fn window_physical_size(&self) -> [i32; 2] {
        self.window_physical_size
    }
}

fn positive_extent(size: [u32; 2]) -> Option<[i32; 2]> {
    let [width, height] = size.map(|value| i32::try_from(value).ok().filter(|v| *v > 0));
    Some([width?, height?])
}

fn bound_physical_extent(
    profile: &DisplayVariantKey,
    current: [u32; 2],
) -> Result<[i32; 2], ClipGraphicsRefusal> {
    let physical = positive_extent(current).ok_or(ClipGraphicsRefusal::InvalidPhysicalExtent)?;
    if current
        != [
            profile.physical_viewport.width,
            profile.physical_viewport.height,
        ]
    {
        return Err(ClipGraphicsRefusal::ViewportChangedWithoutGraphicsConstruction);
    }
    Ok(physical)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LogicalClipRect(pub [i32; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PhysicalClipRect(pub [i32; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerClipRefusal {
    UnsupportedContainerType,
    MalformedClipping,
    MalformedMargin,
    UnresolvedAuthoredVariable,
    BackgroundCountRange,
    CustomClipProvider,
    StandardListRowClipLifecycle,
    SmoothListRowClipLifecycle,
}

/// The materialized owner route, not an inference from an element's geometry.
/// Grid rows using 1422b2210 may forward into an Ordinary container. List rows
/// require their distinct insertion, parent-scissor and scroll lifecycle laws.
/// Ordinary requires the actual 14229f050 base-container route without a later
/// provider installation; element_type alone does not prove this. Its +15f0
/// and +15f8 nodes are constructed by 142220670 with GraphicalClipMode::Inherit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerClipOwner {
    Ordinary,
    CustomProvider,
    StandardListRow,
    SmoothListRow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Margin {
    value: i32,
    percent: bool,
}

impl Margin {
    fn assign(&mut self, value: &Value) -> Result<(), ContainerClipRefusal> {
        let text = value
            .as_scalar()
            .ok_or(ContainerClipRefusal::MalformedMargin)?;
        let text = text.split('\0').next().unwrap_or("");
        if text.starts_with('@') {
            return Err(ContainerClipRefusal::UnresolvedAuthoredVariable);
        }
        // 1422efa70 ORs relative bits; an absolute assignment does not clear
        // a prior percentage. Its numeric helpers both use decimal strtol.
        let scalar = if let Some(number) = text.strip_suffix('%') {
            self.percent = true;
            number
        } else {
            text
        };
        self.value = map_viewer::authored_fixed::parse_native_decimal_i32(scalar);
        Ok(())
    }

    fn pixels(self, extent: i32) -> i32 {
        if !self.percent {
            return self.value;
        }
        // 1422b03e0: IMUL before CVTSI2SS, then exact 0x3c23d70a.
        let value = self.value.wrapping_mul(extent) as f32 * f32::from_bits(0x3c23d70a);
        // Native ceil(value-.5) / floor(value+.5), followed by CVTTSS2SI.
        cvttss2si(if value < 0.0 {
            value - 0.5
        } else {
            value + 0.5
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContainerClipProfile {
    clipping: bool,
    margins: [Margin; 4], // left, top, right, bottom
}

impl ContainerClipProfile {
    /// The witnessed null-parent, clipping-disabled root branch of 1422b03e0
    /// consumes no position or margin getter. None requires the layout branch;
    /// it does not authorize an empty rectangle or a replacement viewport.
    pub(crate) fn unclipped_root_content(
        self,
        logical_graphics_size: [i32; 2],
    ) -> Option<LogicalClipRect> {
        (!self.clipping).then_some(LogicalClipRect([
            0, 0, logical_graphics_size[0], logical_graphics_size[1],
        ]))
    }

    /// Requires successful authenticated CContainerWindow materialization.
    /// 1422faf80 token9c appends each background definition; 1422fa2e0 creates
    /// its vector entry. Thus authored background count is the ctor's +10e4
    /// count, not the count of all descendants or of visible/background pixels.
    pub(crate) fn from_element(element: &GuiElement) -> Result<Self, ContainerClipRefusal> {
        if !element
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
        {
            return Err(ContainerClipRefusal::UnsupportedContainerType);
        }
        let mut clipping = None;
        let mut margins = [Margin::default(); 4];
        let mut background_count = 0_i32;
        for field in element.raw.fields() {
            let Some(key) = field.key.as_deref() else {
                continue;
            };
            if key.eq_ignore_ascii_case("clipping") {
                let text = field
                    .value
                    .as_scalar()
                    .ok_or(ContainerClipRefusal::MalformedClipping)?;
                let text = text.split('\0').next().unwrap_or("");
                if text.starts_with('@') {
                    return Err(ContainerClipRefusal::UnresolvedAuthoredVariable);
                }
                // 1422faf80 initializes the bool local to false each time.
                // 1424b06e0 only assigns for exact yes/no; failure retains false.
                clipping = Some(text == "yes");
            } else if key.eq_ignore_ascii_case("margin") {
                for edge in field
                    .value
                    .as_object()
                    .ok_or(ContainerClipRefusal::MalformedMargin)?
                {
                    let index = match edge.key.as_deref().map(str::to_ascii_lowercase).as_deref() {
                        Some("left") => 0,
                        Some("top") => 1,
                        Some("right") => 2,
                        Some("bottom") => 3,
                        _ => return Err(ContainerClipRefusal::MalformedMargin),
                    };
                    margins[index].assign(&edge.value)?;
                }
            } else if key.eq_ignore_ascii_case("background") {
                background_count = background_count
                    .checked_add(1)
                    .ok_or(ContainerClipRefusal::BackgroundCountRange)?;
            }
        }
        // CContainerWindowType+592=-1; CContainerWindow settles unspecified
        // after 1422a9690. Explicit no never becomes clipping because of a bg.
        Ok(Self {
            clipping: clipping.unwrap_or(background_count > 0),
            margins,
        })
    }

    /// 1422b03e0. Parent is the CURRENT value behind +1628, including alias
    /// updates from its owner; absence means a witnessed null pointer.
    fn own_rect(
        self,
        layout: ContainerClipLayout,
        parent: Option<LogicalClipRect>,
        logical_graphics_size: [i32; 2],
    ) -> LogicalClipRect {
        if !self.clipping {
            // This is an explicit native null-parent branch, not a fallback
            // for unavailable geometry or an unproved list/custom provider.
            return parent.unwrap_or(LogicalClipRect([
                0,
                0,
                logical_graphics_size[0],
                logical_graphics_size[1],
            ]));
        }
        let [x, y] = layout.screen_position;
        let [width, height] = layout.size;
        let [left, top, right, bottom] = [
            self.margins[0].pixels(width),
            self.margins[1].pixels(height),
            self.margins[2].pixels(width),
            self.margins[3].pixels(height),
        ];
        let rect = LogicalClipRect([
            x.wrapping_add(left),
            y.wrapping_add(top),
            x.wrapping_add(width).wrapping_sub(right),
            y.wrapping_add(height).wrapping_sub(bottom),
        ]);
        LogicalClipRect(parent.map_or(rect.0, |parent| intersect(rect.0, parent.0)))
    }

    /// Ordered writes of 1422ae870. The caller applies `background_clip_write`
    /// first, then forwards `content_clip` through the native child families.
    /// No rectangle emptiness filtering occurs during this propagation.
    pub(crate) fn propagate(
        self,
        owner: ContainerClipOwner,
        operation: ContainerClipOperation,
        layout: ContainerClipLayout,
        logical_graphics_size: [i32; 2],
    ) -> Result<ContainerClipWrites, ContainerClipRefusal> {
        match owner {
            ContainerClipOwner::Ordinary => {}
            ContainerClipOwner::CustomProvider => {
                return Err(ContainerClipRefusal::CustomClipProvider)
            }
            ContainerClipOwner::StandardListRow => {
                return Err(ContainerClipRefusal::StandardListRowClipLifecycle)
            }
            ContainerClipOwner::SmoothListRow => {
                return Err(ContainerClipRefusal::SmoothListRowClipLifecycle)
            }
        }
        match operation {
            ContainerClipOperation::ConstructRoot => {
                let own = self.own_rect(layout, None, logical_graphics_size);
                Ok(ContainerClipWrites {
                    background_clip_write: None,
                    own_clip_write: Some(own),
                    content_clip: own,
                })
            }
            ContainerClipOperation::RecomputeOwn {
                current_parent_clip,
            } => {
                let own = self.own_rect(layout, current_parent_clip, logical_graphics_size);
                Ok(ContainerClipWrites {
                    background_clip_write: None,
                    own_clip_write: Some(own),
                    content_clip: own,
                })
            }
            ContainerClipOperation::ReceiveParent { incoming } => {
                let own = self
                    .clipping
                    .then(|| self.own_rect(layout, Some(incoming), logical_graphics_size));
                Ok(ContainerClipWrites {
                    background_clip_write: Some(incoming),
                    own_clip_write: own,
                    content_clip: own.unwrap_or(incoming),
                })
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContainerClipLayout {
    /// Return from container secondary+30 virtual+30, in logical pixels.
    pub screen_position: [i32; 2],
    /// Return from container primary virtual+e8 at this operation.
    pub size: [i32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerClipOperation {
    /// Fresh root: +1628=null; ctor computes own+1594 and passes its address.
    ConstructRoot,
    /// Layout/reparent computes own+1594 and passes that same address. This
    /// skips background writes even when own and incoming coordinates equal.
    RecomputeOwn {
        current_parent_clip: Option<LogicalClipRect>,
    },
    /// A DISTINCT parent-owned pointer: store its identity at +1628, write its
    /// current value to backgrounds/scrollbars, then select the content pointer.
    ReceiveParent { incoming: LogicalClipRect },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ContainerClipWrites {
    /// None means NO WRITE, not disable clipping or reset to the viewport.
    pub background_clip_write: Option<LogicalClipRect>,
    pub own_clip_write: Option<LogicalClipRect>,
    pub content_clip: LogicalClipRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StoredSpriteClip {
    pub enabled: bool,
    pub rect: PhysicalClipRect,
}

impl StoredSpriteClip {
    /// Explicit 1422e9060 setter followed by 1422225b0 for each corner.
    /// `gui_scale` is graphics+1d8/+1dc, not widget authored `scale`.
    pub(crate) fn from_container_write(rect: LogicalClipRect, gui_scale: [f32; 2]) -> Self {
        Self {
            enabled: true,
            rect: PhysicalClipRect(std::array::from_fn(|i| {
                cvttss2si(rect.0[i] as f32 * gui_scale[i % 2])
            })),
        }
    }

    /// Clip-only projection of 1422998c0 for an already reached drawable.
    /// Mode zero forwards inherited clip and REPLACES a drawable's explicit
    /// stored scissor; mode one intersects its own raw scissor with inherited.
    /// Neither mode rejects an empty rectangle or clears a previous enable.
    pub(crate) fn after_graphical_inheritance(
        self,
        mode: GraphicalClipMode,
        inherited: Option<PhysicalClipRect>,
    ) -> Self {
        let Some(inherited) = inherited else {
            return self;
        };
        let rect = match mode {
            GraphicalClipMode::Inherit => inherited,
            GraphicalClipMode::Own => PhysicalClipRect(intersect(self.rect.0, inherited.0)),
        };
        Self {
            enabled: true,
            rect,
        }
    }

    /// Outgoing traversal-entry clip (1422998c0 return bit10). Graphical
    /// parents call this even when they are not themselves drawables.
    pub(crate) fn graphical_clip_for_children(
        self,
        mode: GraphicalClipMode,
        inherited: Option<PhysicalClipRect>,
    ) -> Option<PhysicalClipRect> {
        match mode {
            GraphicalClipMode::Inherit => inherited,
            GraphicalClipMode::Own => {
                Some(PhysicalClipRect(inherited.map_or(self.rect.0, |parent| {
                    intersect(self.rect.0, parent.0)
                })))
            }
        }
    }

    /// 142228c20: execute before +b0/+88. Queue eligibility is a separate
    /// producer. This signature deliberately contains no sprite geometry.
    pub(crate) fn draw_gate(self, physical_graphics_size: [i32; 2]) -> SpriteClipDrawGate {
        if !self.enabled {
            return SpriteClipDrawGate::Unclipped;
        }
        let [left, top, right, bottom] = self.rect.0;
        if left >= right || top >= bottom {
            return SpriteClipDrawGate::Rejected;
        }
        let rect = [
            left.max(0),
            top.max(0),
            right.min(physical_graphics_size[0]),
            bottom.min(physical_graphics_size[1]),
        ];
        if rect[0] >= rect[2] || rect[1] >= rect[3] {
            return SpriteClipDrawGate::Rejected;
        }
        SpriteClipDrawGate::Clipped(PhysicalClipRect(rect))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GraphicalClipMode {
    Inherit,
    Own,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpriteClipDrawGate {
    Unclipped,
    Clipped(PhysicalClipRect),
    Rejected,
}

fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

fn cvttss2si(value: f32) -> i32 {
    if !value.is_finite() || !(-2147483648.0_f32..2147483648.0_f32).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display_key(physical: [u32; 2], minimum: [u32; 2], scale: f32) -> DisplayVariantKey {
        use crate::variant_admission::{DisplayMode, InputVariant, PixelExtent};
        DisplayVariantKey {
            physical_viewport: PixelExtent {
                width: physical[0],
                height: physical[1],
            },
            min_gui: PixelExtent {
                width: minimum[0],
                height: minimum[1],
            },
            gui_scale_f32_bits: scale.to_bits(),
            display_mode: DisplayMode::Windowed,
            localisation_language: "english".into(),
            input_variant: InputVariant::MouseKeyboard,
        }
    }

    // Constructor-only fixtures set effective native fields directly. Actual
    // callers outside this module must obtain the opaque finalizer result.
    fn effective_fixture(profile: &DisplayVariantKey) -> EffectiveGuiScale<'_> {
        EffectiveGuiScale {
            profile,
            value: f32::from_bits(profile.gui_scale_f32_bits),
        }
    }

    fn playback_environment(
        borderless: Option<&str>,
        mode: &str,
    ) -> map_viewer::PlaybackEnvironment {
        let mut settings = std::collections::BTreeMap::from([
            ("physical_width".into(), "1280".into()),
            ("physical_height".into(), "720".into()),
            ("min_gui_width".into(), "1280".into()),
            ("min_gui_height".into(), "720".into()),
            ("gui_scale_f32_bits".into(), 2.0_f32.to_bits().to_string()),
            ("display_mode".into(), mode.into()),
            ("settings_file_sha256".into(), "a".repeat(64)),
        ]);
        if let Some(value) = borderless {
            settings.insert("borderless".into(), value.into());
        }
        map_viewer::PlaybackEnvironment {
            source_game: replay_core::SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256:
                    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
                build_profile: "exact-a729".into(),
            },
            source_profile: replay_core::SourceProfileIdentity {
                profile_id: "local-profile".into(),
                playset_id: "local-playset".into(),
                selection_sha256: "b".repeat(64),
            },
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            settings,
        }
    }

    fn admitted_profile(borderless: Option<&str>, mode: &str) -> PlaybackUiProfile {
        PlaybackUiProfile::admit(
            &playback_environment(borderless, mode),
            crate::variant_admission::InputVariant::MouseKeyboard,
        )
        .unwrap()
    }

    #[test]
    fn playback_profile_absent_borderless_remains_unknown_and_clip_factory_refuses() {
        let profile = admitted_profile(None, "windowed");
        assert_eq!(profile.raw_borderless(), None);
        assert_eq!(
            NativeClipGraphics::from_playback_profile(
                &profile,
                DisplayZeroDesktopMode::Mode([2560, 1440]),
                [1280, 720],
            ),
            Err(ClipGraphicsRefusal::BorderlessSettingUnavailable)
        );
    }

    #[test]
    fn playback_profile_false_borderless_uses_bound_physical_extent() {
        let profile = admitted_profile(Some("false"), "windowed");
        assert_eq!(profile.raw_borderless(), Some(false));
        let graphics = NativeClipGraphics::from_playback_profile(
            &profile,
            DisplayZeroDesktopMode::Unavailable,
            [1280, 720],
        )
        .unwrap();
        assert_eq!(graphics.effective_gui_scale(), 1.0);
        assert_eq!(profile.key().gui_scale_f32_bits, 2.0_f32.to_bits());
        assert_eq!(graphics.construction_profile(), profile.key());
        assert_eq!(
            NativeClipGraphics::from_playback_profile(
                &profile,
                DisplayZeroDesktopMode::Unavailable,
                [1281, 720],
            ),
            Err(ClipGraphicsRefusal::ViewportChangedWithoutGraphicsConstruction)
        );
    }

    #[test]
    fn playback_profile_true_borderless_requires_explicit_desktop_result() {
        let profile = admitted_profile(Some("true"), "borderless_fullscreen");
        assert_eq!(profile.raw_borderless(), Some(true));
        assert_eq!(
            NativeClipGraphics::from_playback_profile(
                &profile,
                DisplayZeroDesktopMode::Unavailable,
                [1280, 720],
            ),
            Err(ClipGraphicsRefusal::DesktopModeUnavailable)
        );
        let graphics = NativeClipGraphics::from_playback_profile(
            &profile,
            DisplayZeroDesktopMode::Mode([2560, 1440]),
            [1280, 720],
        )
        .unwrap();
        assert_eq!(graphics.effective_gui_scale(), 2.0);
        assert_eq!(graphics.logical_size, [640, 360]);
    }

    #[test]
    fn playback_profile_fullscreen_with_borderless_true_still_queries_desktop() {
        let profile = admitted_profile(Some("true"), "fullscreen");
        assert_eq!(profile.key().display_mode, DisplayMode::Fullscreen);
        assert_eq!(profile.raw_borderless(), Some(true));
        let graphics = NativeClipGraphics::from_playback_profile(
            &profile,
            DisplayZeroDesktopMode::Mode([2560, 1440]),
            [1280, 720],
        )
        .unwrap();
        assert_eq!(graphics.effective_gui_scale(), 2.0);
    }

    #[test]
    fn playback_profile_conflicting_borderless_summary_is_refused_by_clip_factory() {
        for (value, mode) in [("true", "windowed"), ("false", "borderless_fullscreen")] {
            let profile = admitted_profile(Some(value), mode);
            assert_eq!(
                NativeClipGraphics::from_playback_profile(
                    &profile,
                    DisplayZeroDesktopMode::Mode([2560, 1440]),
                    [1280, 720],
                ),
                Err(ClipGraphicsRefusal::WindowModeBindingMismatch)
            );
        }
    }

    #[test]
    fn playback_profile_rejects_noncanonical_borderless_values() {
        for value in ["yes", "no", "True", "FALSE", "1", "", " true"] {
            assert!(matches!(
                PlaybackUiProfile::admit(
                    &playback_environment(Some(value), "windowed"),
                    crate::variant_admission::InputVariant::MouseKeyboard,
                ),
                Err(crate::variant_admission::VariantRejection::InvalidEnvelope(message))
                    if message == "local playback profile borderless is not canonical true/false"
            ));
        }
    }

    #[test]
    fn settings_read_clamps_effective_scale_without_rewriting_authored_profile() {
        let key = display_key([1920, 1080], [1280, 720], 2.0);
        let effective = EffectiveGuiScale::after_graphics_settings_read(
            &key,
            [1920, 1080],
            false,
            DisplayZeroDesktopMode::Unavailable,
        )
        .unwrap();
        assert_eq!(effective.value().to_bits(), 1.5_f32.to_bits());
        assert_eq!(key.gui_scale_f32_bits, 2.0_f32.to_bits());
        let graphics =
            NativeClipGraphics::from_graphics_construction(effective, [1920, 1080]).unwrap();
        assert_eq!(graphics.logical_size, [1280, 720]);
        assert_eq!(graphics.logical_to_physical, [1.5; 2]);

        // The first branch returns 0.5 even though this viewport's maximum
        // is 0.4. An authored 0.5 instead takes the upper branch to 0.4.
        for (authored, expected) in [(0.4_f32, 0.5_f32), (0.5, 0.4)] {
            let key = display_key([512, 300], [512, 300], authored);
            assert_eq!(
                EffectiveGuiScale::after_graphics_settings_read(
                    &key,
                    [512, 300],
                    false,
                    DisplayZeroDesktopMode::Unavailable,
                )
                .unwrap()
                .value()
                .to_bits(),
                expected.to_bits()
            );
        }
    }

    #[test]
    fn borderless_uses_display_zero_and_distinguishes_unknown_from_query_failure() {
        let mut key = display_key([1280, 720], [1280, 720], 2.0);
        // Both native booleans can be true. The DisplayMode summary gives
        // fullscreen priority, but +e1 still selects the desktop query.
        key.display_mode = DisplayMode::Fullscreen;
        let read = |borderless, desktop| {
            EffectiveGuiScale::after_graphics_settings_read(&key, [1280, 720], borderless, desktop)
        };
        assert_eq!(
            read(true, DisplayZeroDesktopMode::Unavailable),
            Err(ClipGraphicsRefusal::DesktopModeUnavailable)
        );
        assert_eq!(
            read(true, DisplayZeroDesktopMode::Mode([2560, 1440]))
                .unwrap()
                .value(),
            2.0
        );
        assert_eq!(
            read(true, DisplayZeroDesktopMode::QueryFailed)
                .unwrap()
                .value(),
            1.0
        );
        assert_eq!(
            read(false, DisplayZeroDesktopMode::Mode([2560, 1440]))
                .unwrap()
                .value(),
            1.0
        );
        key.display_mode = DisplayMode::Windowed;
        assert_eq!(
            EffectiveGuiScale::after_graphics_settings_read(
                &key,
                [1280, 720],
                true,
                DisplayZeroDesktopMode::Mode([2560, 1440]),
            ),
            Err(ClipGraphicsRefusal::WindowModeBindingMismatch)
        );
    }

    #[test]
    fn graphics_construction_preserves_minimum_aspect_and_float_operation_order() {
        for (physical, expected) in [
            ([800, 600], [1024, 768]),
            ([1200, 600], [1536, 768]),
            ([800, 900], [1024, 1152]),
        ] {
            let key = display_key(physical, [1024, 768], 1.0);
            let graphics =
                NativeClipGraphics::from_graphics_construction(effective_fixture(&key), physical)
                    .unwrap();
            assert_eq!(graphics.logical_size, expected);
            assert_eq!(
                graphics.logical_to_physical.map(f32::to_bits),
                [0x3f480000; 2]
            );
        }
        let key = display_key([801, 600], [640, 480], 1.1);
        let graphics =
            NativeClipGraphics::from_graphics_construction(effective_fixture(&key), [801, 600])
                .unwrap();
        assert_eq!(graphics.logical_size, [728, 545]);
        assert_eq!(graphics.logical_to_physical[0].to_bits(), 0x3f8cd5ce);
        assert_eq!((801.0_f32 / 728.0_f32).to_bits(), 0x3f8cd5cd);
    }

    #[test]
    fn graphics_construction_refuses_unbound_resize_and_invalid_native_results() {
        let key = display_key([800, 600], [640, 480], 1.0);
        assert_eq!(
            NativeClipGraphics::from_graphics_construction(effective_fixture(&key), [801, 600]),
            Err(ClipGraphicsRefusal::ViewportChangedWithoutGraphicsConstruction)
        );
        let key = display_key([800, 600], [640, 480], f32::MAX);
        assert_eq!(
            NativeClipGraphics::from_graphics_construction(effective_fixture(&key), [800, 600]),
            Err(ClipGraphicsRefusal::InvalidNativeLogicalExtent)
        );
    }

    #[test]
    fn window_resize_retains_graphics_generation_and_stages_without_mutation() {
        let key = display_key([1920, 1080], [1280, 720], 2.0);
        let effective = EffectiveGuiScale::after_graphics_settings_read(
            &key,
            [1920, 1080],
            false,
            DisplayZeroDesktopMode::Unavailable,
        )
        .unwrap();
        let graphics =
            NativeClipGraphics::from_graphics_construction(effective, [1920, 1080]).unwrap();
        let resized = graphics.after_window_resize([800, 600]).unwrap();
        assert_eq!(graphics.window_physical_size(), [1920, 1080]);
        assert_eq!(resized.window_physical_size(), [800, 600]);
        assert_eq!(resized.logical_size, [1280, 720]);
        assert_eq!(resized.physical_size, [1920, 1080]);
        assert_eq!(resized.logical_to_physical, [1.5; 2]);
        assert_eq!(resized.effective_gui_scale(), 1.5);
        assert_eq!(resized.construction_profile(), &key);
        // Returning to the initial numbers does not create another generation.
        assert_eq!(resized.after_window_resize([1920, 1080]).unwrap(), graphics);
        assert_eq!(
            resized.after_window_resize([0, 600]),
            Err(ClipGraphicsRefusal::InvalidPhysicalExtent)
        );
        // Enabled clip gate still consumes the retained CGraphics extent.
        let clip = StoredSpriteClip {
            enabled: true,
            rect: PhysicalClipRect([900, 100, 1000, 200]),
        };
        assert_eq!(
            clip.draw_gate(resized.physical_size),
            SpriteClipDrawGate::Clipped(clip.rect)
        );
    }

    fn profile(source: &str) -> ContainerClipProfile {
        let element = GuiElement {
            element_type: "containerWindowType".into(),
            name: "test".into(),
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: crate::paradox_text::parse_str(source).root,
        };
        ContainerClipProfile::from_element(&element).unwrap()
    }

    fn layout() -> ContainerClipLayout {
        ContainerClipLayout {
            screen_position: [20, 30],
            size: [100, 80],
        }
    }

    fn root_clip(source: &str) -> LogicalClipRect {
        profile(source)
            .propagate(
                ContainerClipOwner::Ordinary,
                ContainerClipOperation::ConstructRoot,
                layout(),
                [800, 600],
            )
            .unwrap()
            .content_clip
    }

    #[test]
    fn omitted_clipping_depends_on_background_census_not_sprite_geometry() {
        assert_eq!(root_clip(""), LogicalClipRect([0, 0, 800, 600]));
        assert_eq!(
            root_clip("background = {}"),
            LogicalClipRect([20, 30, 120, 110])
        );
        assert_eq!(
            root_clip("background = {} clipping = no"),
            LogicalClipRect([0, 0, 800, 600])
        );
        assert_eq!(
            root_clip("clipping = yes"),
            LogicalClipRect([20, 30, 120, 110])
        );
        // Every assignment has a fresh false local; unrecognized boolean
        // text does not preserve the preceding yes.
        assert_eq!(
            root_clip("clipping = yes clipping = UNKNOWN"),
            LogicalClipRect([0, 0, 800, 600])
        );
    }

    #[test]
    fn margin_assignment_retains_percentage_and_preserves_inverted_rectangles() {
        let p = profile(
            "clipping = yes margin = { left = 50% top = -3 right = 101 } margin = { left = 3 }",
        );
        let output = p
            .propagate(
                ContainerClipOwner::Ordinary,
                ContainerClipOperation::ConstructRoot,
                ContainerClipLayout {
                    screen_position: [20, 30],
                    size: [50, 80],
                },
                [800, 600],
            )
            .unwrap();
        // Retained percentage bit: round(3*50*.01)=2, rather than 3 pixels.
        assert_eq!(output.content_clip, LogicalClipRect([22, 27, -31, 110]));
        assert_eq!(
            StoredSpriteClip::from_container_write(output.content_clip, [1.0; 2])
                .draw_gate([800, 600]),
            SpriteClipDrawGate::Rejected
        );
    }

    #[test]
    fn own_pointer_and_equal_parent_pointer_have_different_write_sets() {
        let p = profile("clipping = yes");
        let incoming = LogicalClipRect([20, 30, 120, 110]);
        let own = p
            .propagate(
                ContainerClipOwner::Ordinary,
                ContainerClipOperation::RecomputeOwn {
                    current_parent_clip: Some(incoming),
                },
                layout(),
                [800, 600],
            )
            .unwrap();
        let parent = p
            .propagate(
                ContainerClipOwner::Ordinary,
                ContainerClipOperation::ReceiveParent { incoming },
                layout(),
                [800, 600],
            )
            .unwrap();
        assert_eq!(own.content_clip, parent.content_clip);
        assert_eq!(own.background_clip_write, None);
        assert_eq!(parent.background_clip_write, Some(incoming));
        let no = profile("clipping = no")
            .propagate(
                ContainerClipOwner::Ordinary,
                ContainerClipOperation::ReceiveParent { incoming },
                layout(),
                [800, 600],
            )
            .unwrap();
        assert_eq!(no.own_clip_write, None);
        assert_eq!(no.content_clip, incoming);
    }

    #[test]
    fn logical_scaling_then_graphical_inheritance_precedes_the_draw_gate() {
        let stored =
            StoredSpriteClip::from_container_write(LogicalClipRect([-3, 1, 5, 9]), [0.5, 1.5]);
        assert_eq!(stored.rect, PhysicalClipRect([-1, 1, 2, 13]));
        let inherited = PhysicalClipRect([10, 10, 40, 40]);
        // Mode zero replaces an explicit rectangle; intersecting it would
        // wrongly suppress this native visit and its time/effect getters.
        let replaced =
            stored.after_graphical_inheritance(GraphicalClipMode::Inherit, Some(inherited));
        assert_eq!(
            replaced.draw_gate([30, 30]),
            SpriteClipDrawGate::Clipped(PhysicalClipRect([10, 10, 30, 30]))
        );
        let own = stored.after_graphical_inheritance(GraphicalClipMode::Own, Some(inherited));
        assert_eq!(own.draw_gate([30, 30]), SpriteClipDrawGate::Rejected);
        assert_eq!(
            stored.graphical_clip_for_children(GraphicalClipMode::Inherit, None),
            None
        );
        assert_eq!(
            stored.graphical_clip_for_children(GraphicalClipMode::Own, None),
            Some(stored.rect)
        );
        assert_eq!(
            stored.after_graphical_inheritance(GraphicalClipMode::Inherit, None),
            stored
        );
        assert_eq!(
            StoredSpriteClip {
                enabled: false,
                rect: PhysicalClipRect([4, 4, -1, -1])
            }
            .draw_gate([0, 0]),
            SpriteClipDrawGate::Unclipped
        );
    }

    #[test]
    fn special_owner_routes_cannot_fall_back_to_an_ordinary_root_clip() {
        for (owner, refusal) in [
            (
                ContainerClipOwner::CustomProvider,
                ContainerClipRefusal::CustomClipProvider,
            ),
            (
                ContainerClipOwner::StandardListRow,
                ContainerClipRefusal::StandardListRowClipLifecycle,
            ),
            (
                ContainerClipOwner::SmoothListRow,
                ContainerClipRefusal::SmoothListRowClipLifecycle,
            ),
        ] {
            assert_eq!(
                profile("").propagate(
                    owner,
                    ContainerClipOperation::ConstructRoot,
                    layout(),
                    [800, 600]
                ),
                Err(refusal)
            );
        }
        assert_eq!(cvttss2si(f32::NAN), i32::MIN);
        assert_eq!(cvttss2si(2147483648.0), i32::MIN);
        assert_eq!(cvttss2si(-3.9), -3);
    }
}
