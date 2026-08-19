
use crate::assets::AssetResolver;
use crate::compose::{resolve_element_rect, ComposeOptions, Rect};
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::gui_scene_util::{load_named_element, pair, promote_show_position};
use crate::paradox_text::scalar_i32;
use crate::scrollbar_law::{
    extended_displayed_value, extended_thumb_pixel, extended_value_for_pixel,
    ExtendedScrollbarCarrier, ExtendedScrollbarInverseCarrier, ScrollbarAxis,
    ScrollbarCarrierAuthority,
};
use crate::widget_layout::{gridbox_layout, GridBoxLayout};
use crate::{GameUiError, Result};
use std::collections::BTreeMap;

pub const COUNTRY_TRADE_GUI: &str = "interface/countrytradeview.gui";
pub const COUNTRY_TRADE_ROOT: &str = "countrytradeview";
pub const TRADE_SCROLL_FIXED_SCALE: i64 = 100_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CountryTradeSort {
    #[default]
    Country,
    AvailableExport,
    Exported,
    Convoys,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TradeViewerScroll {
    pub displayed_fixed: i64,
    pub target_fixed: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryTradeViewerState {
    pub selected_resource_index: usize,
    pub selected_country_filter_ids: Vec<u32>,
    pub sort: CountryTradeSort,
    pub descending: bool,
    pub catalogue_scroll: TradeViewerScroll,
    pub resource_filter_scroll: TradeViewerScroll,
    pub country_filter_scroll: TradeViewerScroll,
    pub active_scrollbar_drag: Option<TradeScrollbarDrag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradeScrollTarget {
    ResourceFilters,
    CountryFilters,
    Catalogue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeScrollbarDrag {
    pub target: TradeScrollTarget,
    pub grab_offset_y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TradePointerAction {
    None,
    Close,
    SelectResource(usize),
    ToggleCountryFilter(u32),
    Sort(CountryTradeSort),
    ScrollCatalogue,
    ScrollCatalogueStep(i32),
    ScrollResourceFilters(i32),
    ScrollCountryFilters(i32),
    BeginScrollbarDrag(TradeScrollTarget, i32),
    BeginScrollbarTrackDrag(TradeScrollTarget, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeScrollbarPartGeometry {
    pub rect: Rect,
    pub sprite_name: String,
    pub border_size: Option<(usize, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeScrollbarGeometry {
    pub background: TradeScrollbarPartGeometry,
    pub track: TradeScrollbarPartGeometry,
    pub decrease: TradeScrollbarPartGeometry,
    pub increase: TradeScrollbarPartGeometry,
    pub slider: TradeScrollbarPartGeometry,
    pub thumb_base_y: i32,
    pub thumb_travel_pixels: i32,
    pub thumb_authored_offset_y: i32,
    pub smooth_factor_fixed: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeCollectionLayout {
    pub container: GuiElement,
    pub grid: GuiElement,
    pub container_rect: Rect,
    pub grid_layout: GridBoxLayout,
    pub visible_clip: Rect,
    pub scroll_wheel_pixels: i32,
    pub smooth_scrolling: bool,
    pub scrollbar: Option<TradeScrollbarGeometry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeAuthoredLayout {
    pub root: GuiElement,
    pub root_rect: Rect,
    pub resource_template: GuiElement,
    pub resource_filter_template: GuiElement,
    pub country_filter_template: GuiElement,
    pub country_row_template: GuiElement,
    pub resources: TradeCollectionLayout,
    pub resource_filters: TradeCollectionLayout,
    pub country_filters: TradeCollectionLayout,
    pub catalogue: TradeCollectionLayout,
    pub sort_country_rect: Rect,
    pub sort_export_rect: Rect,
    pub sort_request_rect: Rect,
    pub sort_convoys_rect: Rect,
    pub close_button_rect: Rect,
}

impl TradeAuthoredLayout {
    pub fn load(
        resolver: &AssetResolver,
        viewport_width: usize,
        viewport_height: usize,
    ) -> Result<Self> {
        Self::load_internal(resolver, None, viewport_width, viewport_height)
    }

    pub fn load_with_registry(
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        viewport_width: usize,
        viewport_height: usize,
    ) -> Result<Self> {
        Self::load_internal(
            resolver,
            Some(registry),
            viewport_width,
            viewport_height,
        )
    }

    fn load_internal(
        resolver: &AssetResolver,
        registry: Option<&SpriteRegistry>,
        viewport_width: usize,
        viewport_height: usize,
    ) -> Result<Self> {
        let viewport_width = i32::try_from(viewport_width)
            .map_err(|_| GameUiError::parse("XUI15 viewport width exceeds i32"))?;
        let viewport_height = i32::try_from(viewport_height)
            .map_err(|_| GameUiError::parse("XUI15 viewport height exceeds i32"))?;
        if viewport_width <= 0 || viewport_height <= 0 {
            return Err(GameUiError::parse("XUI15 viewport must be positive"));
        }
        let viewport = Rect::new(0, 0, viewport_width, viewport_height);
        let mut cache: BTreeMap<String, LayoutSet> = BTreeMap::new();
        let mut root =
            load_named_element(&mut cache, resolver, COUNTRY_TRADE_GUI, COUNTRY_TRADE_ROOT)?;
        promote_show_position(&mut root);
        let options = ComposeOptions::new(viewport_width as usize, viewport_height as usize);
        let root_rect = resolve_element_rect(&root, viewport, (0, 0), &options);
        positive_rect(root_rect, "countrytradeview")?;

        let resources = collection(
            &mut cache,
            resolver,
            registry,
            &root,
            "resources",
            "resources_grid",
            root_rect,
            viewport,
            &options,
        )?;
        let resource_filters = collection(
            &mut cache,
            resolver,
            registry,
            &root,
            "filters",
            "resources_grid",
            root_rect,
            viewport,
            &options,
        )?;
        let country_filters = collection(
            &mut cache,
            resolver,
            registry,
            &root,
            "country_filters",
            "country_filters_grid",
            root_rect,
            viewport,
            &options,
        )?;
        let catalogue = collection(
            &mut cache,
            resolver,
            registry,
            &root,
            "trade",
            "trade_grid",
            root_rect,
            viewport,
            &options,
        )?;

        let button_rect = |name: &str| -> Result<Rect> {
            let button = direct_child(&root, name, Some("buttonType"))?;
            let rect = resolve_element_rect(button, root_rect, (0, 0), &options);
            positive_rect(rect, name)?;
            Ok(rect)
        };
        let sort_country_rect = button_rect("sort_country")?;
        let sort_export_rect = button_rect("sort_export")?;
        let sort_request_rect = button_rect("sort_request")?;
        let sort_convoys_rect = button_rect("sort_convoys")?;
        let close_button = direct_child(&root, "close_button", Some("buttonType"))?;
        let close_button_rect = button_rect_with_natural_size(
            close_button,
            root_rect,
            &options,
            resolver,
            registry,
        )?;

        Ok(Self {
            root,
            root_rect,
            resource_template: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_TRADE_GUI,
                "resources_info_entry",
            )?,
            resource_filter_template: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_TRADE_GUI,
                "resources_filter_entry",
            )?,
            country_filter_template: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_TRADE_GUI,
                "filter_entry",
            )?,
            country_row_template: load_named_element(
                &mut cache,
                resolver,
                COUNTRY_TRADE_GUI,
                "country_trade_entry",
            )?,
            resources,
            resource_filters,
            country_filters,
            catalogue,
            sort_country_rect,
            sort_export_rect,
            sort_request_rect,
            sort_convoys_rect,
            close_button_rect,
        })
    }

    pub fn catalogue_max_scroll_fixed(&self, row_count: usize) -> Result<i64> {
        max_grid_scroll_fixed(&self.catalogue, row_count, ScrollAxis::Vertical)
    }

    pub fn resource_filter_max_scroll_fixed(&self, row_count: usize) -> Result<i64> {
        max_grid_scroll_fixed(&self.resource_filters, row_count, ScrollAxis::Horizontal)
    }

    pub fn country_filter_max_scroll_fixed(&self, row_count: usize) -> Result<i64> {
        max_grid_scroll_fixed(&self.country_filters, row_count, ScrollAxis::Vertical)
    }

    pub fn hit_test(
        &self,
        state: &CountryTradeViewerState,
        point: (i32, i32),
        country_filter_ids: &[u32],
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<TradePointerAction> {
        if contains(self.close_button_rect, point) {
            return Ok(TradePointerAction::Close);
        }
        for target in [
            TradeScrollTarget::ResourceFilters,
            TradeScrollTarget::CountryFilters,
            TradeScrollTarget::Catalogue,
        ] {
            let collection = self.collection_for_target(target);
            let scrollbar = collection.scrollbar.as_ref().ok_or_else(|| {
                GameUiError::missing("XUI15 interactive collection lacks scrollbar")
            })?;
            let maximum = self.maximum_for_target(
                target,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            )?;
            let scroll = scroll_for_target(state, target);
            let slider_rect = scrollbar.slider_rect(scroll.displayed_fixed, maximum)?;
            if contains(scrollbar.decrease.rect, point) {
                return Ok(step_action(target, -1));
            }
            if contains(scrollbar.increase.rect, point) {
                return Ok(step_action(target, 1));
            }
            if maximum > 0 && contains(slider_rect, point) {
                return Ok(TradePointerAction::BeginScrollbarDrag(
                    target,
                    point.1.saturating_sub(slider_rect.y()),
                ));
            }
            if maximum > 0 && contains(scrollbar.track.rect, point) {
                return Ok(TradePointerAction::BeginScrollbarTrackDrag(target, point.1));
            }
        }
        for (rect, sort) in [
            (self.sort_country_rect, CountryTradeSort::Country),
            (self.sort_export_rect, CountryTradeSort::AvailableExport),
            (self.sort_request_rect, CountryTradeSort::Exported),
            (self.sort_convoys_rect, CountryTradeSort::Convoys),
        ] {
            if contains(rect, point) {
                return Ok(TradePointerAction::Sort(sort));
            }
        }

        let resource_shift = fixed_pixels(state.resource_filter_scroll.displayed_fixed)?;
        for index in 0..7usize {
            let index_i32 = i32::try_from(index)
                .map_err(|_| GameUiError::parse("XUI15 resource index overflows i32"))?;
            let (x, y) = self
                .resource_filters
                .grid_layout
                .explicit_coordinate_origin((index_i32, 0))?;
            let rect = Rect::new(
                x.checked_sub(resource_shift)
                    .ok_or_else(|| GameUiError::parse("XUI15 resource hit rect overflows"))?,
                y,
                self.resource_filters.grid_layout.slot_size.0,
                self.resource_filters.grid_layout.slot_size.1,
            );
            if intersection(rect, self.resource_filters.visible_clip)
                .is_some_and(|visible| contains(visible, point))
            {
                return Ok(TradePointerAction::SelectResource(index));
            }
        }

        let country_shift = fixed_pixels(state.country_filter_scroll.displayed_fixed)?;
        let columns = self
            .country_filters
            .grid_layout
            .max_slots_x
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                GameUiError::parse("XUI15 country-filter grid has no positive column count")
            })?;
        for (index, filter_id) in country_filter_ids.iter().copied().enumerate() {
            let logical_x = i32::try_from(index % columns)
                .map_err(|_| GameUiError::parse("XUI15 country-filter x overflows i32"))?;
            let logical_y = i32::try_from(index / columns)
                .map_err(|_| GameUiError::parse("XUI15 country-filter y overflows i32"))?;
            let (x, y) = self
                .country_filters
                .grid_layout
                .explicit_coordinate_origin((logical_x, logical_y))?;
            let rect = Rect::new(
                x,
                y.checked_sub(country_shift)
                    .ok_or_else(|| GameUiError::parse("XUI15 country-filter hit rect overflows"))?,
                self.country_filters.grid_layout.slot_size.0,
                self.country_filters.grid_layout.slot_size.1,
            );
            if intersection(rect, self.country_filters.visible_clip)
                .is_some_and(|visible| contains(visible, point))
            {
                return Ok(TradePointerAction::ToggleCountryFilter(filter_id));
            }
        }
        Ok(TradePointerAction::None)
    }

    pub fn wheel_action(
        &self,
        state: &mut CountryTradeViewerState,
        point: (i32, i32),
        signed_units: i32,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<TradePointerAction> {
        if signed_units == 0 {
            return Ok(TradePointerAction::None);
        }
        if contains(self.resource_filters.container_rect, point) {
            return self.wheel_collection(
                state,
                TradeScrollTarget::ResourceFilters,
                signed_units,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            );
        }
        if contains(self.country_filters.container_rect, point) {
            return self.wheel_collection(
                state,
                TradeScrollTarget::CountryFilters,
                signed_units,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            );
        }
        if !contains(self.catalogue.visible_clip, point) {
            return Ok(TradePointerAction::None);
        }
        self.wheel_collection(
            state,
            TradeScrollTarget::Catalogue,
            signed_units,
            resource_filter_row_count,
            country_filter_row_count,
            catalogue_row_count,
        )
    }

    pub fn apply_pointer_action(
        &self,
        state: &mut CountryTradeViewerState,
        action: TradePointerAction,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<()> {
        match action {
            TradePointerAction::ScrollResourceFilters(direction) => self.step_scroll_target(
                state,
                TradeScrollTarget::ResourceFilters,
                direction,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            ),
            TradePointerAction::ScrollCountryFilters(direction) => self.step_scroll_target(
                state,
                TradeScrollTarget::CountryFilters,
                direction,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            ),
            TradePointerAction::ScrollCatalogueStep(direction) => self.step_scroll_target(
                state,
                TradeScrollTarget::Catalogue,
                direction,
                resource_filter_row_count,
                country_filter_row_count,
                catalogue_row_count,
            ),
            TradePointerAction::BeginScrollbarDrag(target, grab_offset_y) => {
                state.active_scrollbar_drag = Some(TradeScrollbarDrag {
                    target,
                    grab_offset_y,
                });
                Ok(())
            }
            TradePointerAction::BeginScrollbarTrackDrag(target, pointer_y) => {
                let maximum = self.maximum_for_target(
                    target,
                    resource_filter_row_count,
                    country_filter_row_count,
                    catalogue_row_count,
                )?;
                let scrollbar = self
                    .collection_for_target(target)
                    .scrollbar
                    .as_ref()
                    .ok_or_else(|| GameUiError::missing("XUI15 collection lacks scrollbar"))?;
                let grab_offset_y = scrollbar.slider.rect.h() / 2;
                self.set_scrollbar_from_pointer(
                    state,
                    target,
                    pointer_y,
                    grab_offset_y,
                    maximum,
                )?;
                state.active_scrollbar_drag = Some(TradeScrollbarDrag {
                    target,
                    grab_offset_y,
                });
                Ok(())
            }
            _ => {
                state.apply_pointer_action(action);
                Ok(())
            }
        }
    }

    pub fn drag_active_scrollbar(
        &self,
        state: &mut CountryTradeViewerState,
        pointer_y: i32,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<bool> {
        let Some(drag) = state.active_scrollbar_drag else {
            return Ok(false);
        };
        let maximum = self.maximum_for_target(
            drag.target,
            resource_filter_row_count,
            country_filter_row_count,
            catalogue_row_count,
        )?;
        self.set_scrollbar_from_pointer(
            state,
            drag.target,
            pointer_y,
            drag.grab_offset_y,
            maximum,
        )?;
        Ok(true)
    }

    pub fn release_active_scrollbar(&self, state: &mut CountryTradeViewerState) -> bool {
        state.active_scrollbar_drag.take().is_some()
    }

    pub fn advance_smooth_scroll(&self, state: &mut CountryTradeViewerState) -> Result<()> {
        for target in [
            TradeScrollTarget::ResourceFilters,
            TradeScrollTarget::CountryFilters,
            TradeScrollTarget::Catalogue,
        ] {
            let collection = self.collection_for_target(target);
            let factor = collection
                .scrollbar
                .as_ref()
                .ok_or_else(|| GameUiError::missing("XUI15 collection lacks scrollbar"))?
                .smooth_factor_fixed;
            let scroll = scroll_for_target_mut(state, target);
            scroll.displayed_fixed = extended_displayed_value(
                scroll.displayed_fixed,
                scroll.target_fixed,
                factor,
                collection.smooth_scrolling,
            )?;
        }
        Ok(())
    }

    fn wheel_collection(
        &self,
        state: &mut CountryTradeViewerState,
        target: TradeScrollTarget,
        signed_units: i32,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<TradePointerAction> {
        let collection = self.collection_for_target(target);
        if collection.scroll_wheel_pixels <= 0 {
            return Ok(TradePointerAction::None);
        }
        let maximum = self.maximum_for_target(
            target,
            resource_filter_row_count,
            country_filter_row_count,
            catalogue_row_count,
        )?;
        let delta = i64::from(signed_units)
            .checked_mul(i64::from(collection.scroll_wheel_pixels))
            .and_then(|value| value.checked_mul(TRADE_SCROLL_FIXED_SCALE))
            .ok_or_else(|| GameUiError::parse("XUI15 wheel delta overflows"))?;
        let scroll = scroll_for_target_mut(state, target);
        scroll.target_fixed = scroll
            .target_fixed
            .checked_sub(delta)
            .ok_or_else(|| GameUiError::parse("XUI15 wheel target overflows"))?
            .clamp(0, maximum);
        if !collection.smooth_scrolling {
            scroll.displayed_fixed = scroll.target_fixed;
        }
        Ok(match target {
            TradeScrollTarget::ResourceFilters => {
                TradePointerAction::ScrollResourceFilters(signed_units)
            }
            TradeScrollTarget::CountryFilters => {
                TradePointerAction::ScrollCountryFilters(signed_units)
            }
            TradeScrollTarget::Catalogue => TradePointerAction::ScrollCatalogue,
        })
    }

    fn step_scroll_target(
        &self,
        state: &mut CountryTradeViewerState,
        target: TradeScrollTarget,
        signed_steps: i32,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<()> {
        let maximum = self.maximum_for_target(
            target,
            resource_filter_row_count,
            country_filter_row_count,
            catalogue_row_count,
        )?;
        let collection = self.collection_for_target(target);
        let authored_slot_pixels = match target {
            TradeScrollTarget::ResourceFilters => collection.grid_layout.slot_size.0,
            TradeScrollTarget::CountryFilters | TradeScrollTarget::Catalogue => {
                collection.grid_layout.slot_size.1
            }
        };
        let scroll = scroll_for_target_mut(state, target);
        self.step_filter_scroll(
            scroll,
            signed_steps,
            maximum,
            authored_slot_pixels,
            collection.smooth_scrolling,
        )
    }

    fn set_scrollbar_from_pointer(
        &self,
        state: &mut CountryTradeViewerState,
        target: TradeScrollTarget,
        pointer_y: i32,
        grab_offset_y: i32,
        maximum: i64,
    ) -> Result<()> {
        let scrollbar = self
            .collection_for_target(target)
            .scrollbar
            .as_ref()
            .ok_or_else(|| GameUiError::missing("XUI15 collection lacks scrollbar"))?;
        let desired_slider_y = pointer_y
            .checked_sub(grab_offset_y)
            .ok_or_else(|| GameUiError::parse("XUI15 scrollbar pointer overflows"))?;
        let value = scrollbar.value_for_slider_y(desired_slider_y, maximum)?;
        let scroll = scroll_for_target_mut(state, target);
        scroll.target_fixed = value;
        scroll.displayed_fixed = value;
        Ok(())
    }

    fn collection_for_target(&self, target: TradeScrollTarget) -> &TradeCollectionLayout {
        match target {
            TradeScrollTarget::ResourceFilters => &self.resource_filters,
            TradeScrollTarget::CountryFilters => &self.country_filters,
            TradeScrollTarget::Catalogue => &self.catalogue,
        }
    }

    fn maximum_for_target(
        &self,
        target: TradeScrollTarget,
        resource_filter_row_count: usize,
        country_filter_row_count: usize,
        catalogue_row_count: usize,
    ) -> Result<i64> {
        match target {
            TradeScrollTarget::ResourceFilters => {
                self.resource_filter_max_scroll_fixed(resource_filter_row_count)
            }
            TradeScrollTarget::CountryFilters => {
                self.country_filter_max_scroll_fixed(country_filter_row_count)
            }
            TradeScrollTarget::Catalogue => {
                self.catalogue_max_scroll_fixed(catalogue_row_count)
            }
        }
    }

    fn step_filter_scroll(
        &self,
        scroll: &mut TradeViewerScroll,
        signed_steps: i32,
        maximum: i64,
        authored_slot_pixels: i32,
        smooth: bool,
    ) -> Result<()> {
        let delta = i64::from(signed_steps)
            .checked_mul(i64::from(authored_slot_pixels))
            .and_then(|value| value.checked_mul(TRADE_SCROLL_FIXED_SCALE))
            .ok_or_else(|| GameUiError::parse("XUI15 filter-scroll delta overflows"))?;
        scroll.target_fixed = scroll
            .target_fixed
            .checked_add(delta)
            .ok_or_else(|| GameUiError::parse("XUI15 filter-scroll target overflows"))?
            .clamp(0, maximum);
        if !smooth {
            scroll.displayed_fixed = scroll.target_fixed;
        }
        Ok(())
    }
}

impl CountryTradeViewerState {
    pub fn apply_pointer_action(&mut self, action: TradePointerAction) {
        match action {
            TradePointerAction::None
            | TradePointerAction::Close
            | TradePointerAction::ScrollCatalogue
            | TradePointerAction::ScrollCatalogueStep(_)
            | TradePointerAction::ScrollResourceFilters(_)
            | TradePointerAction::ScrollCountryFilters(_)
            | TradePointerAction::BeginScrollbarDrag(_, _)
            | TradePointerAction::BeginScrollbarTrackDrag(_, _) => {}
            TradePointerAction::SelectResource(index) => {
                self.selected_resource_index = index;
                self.catalogue_scroll = TradeViewerScroll::default();
            }
            TradePointerAction::ToggleCountryFilter(filter_id) => {
                if let Some(index) = self
                    .selected_country_filter_ids
                    .iter()
                    .position(|selected| *selected == filter_id)
                {
                    self.selected_country_filter_ids.remove(index);
                } else {
                    self.selected_country_filter_ids.push(filter_id);
                }
                self.catalogue_scroll = TradeViewerScroll::default();
            }
            TradePointerAction::Sort(sort) => {
                if self.sort == sort {
                    self.descending = !self.descending;
                } else {
                    self.sort = sort;
                    self.descending = false;
                }
                self.catalogue_scroll = TradeViewerScroll::default();
            }
        }
    }

}

impl TradeScrollbarGeometry {
    pub fn slider_rect(&self, value_fixed: i64, maximum_fixed: i64) -> Result<Rect> {
        let (_, mapped_y) = extended_thumb_pixel(ExtendedScrollbarCarrier {
            authority: ScrollbarCarrierAuthority::ExactA729Runtime,
            axis: ScrollbarAxis::Vertical,
            min_fixed: 0,
            max_fixed: maximum_fixed,
            value_fixed,
            base_pixel: (self.slider.rect.x(), self.thumb_base_y),
            travel_pixels: (0, self.thumb_travel_pixels),
        })?;
        Ok(Rect::new(
            self.slider.rect.x(),
            mapped_y
                .checked_add(self.thumb_authored_offset_y)
                .ok_or_else(|| GameUiError::parse("XUI15 slider y overflows"))?,
            self.slider.rect.w(),
            self.slider.rect.h(),
        ))
    }

    pub(crate) fn value_for_slider_y(
        &self,
        desired_slider_y: i32,
        maximum_fixed: i64,
    ) -> Result<i64> {
        let desired_mapped_y = desired_slider_y
            .checked_sub(self.thumb_authored_offset_y)
            .ok_or_else(|| GameUiError::parse("XUI15 slider inverse y overflows"))?;
        extended_value_for_pixel(ExtendedScrollbarInverseCarrier {
            authority: ScrollbarCarrierAuthority::ExactA729Runtime,
            axis: ScrollbarAxis::Vertical,
            min_fixed: 0,
            max_fixed: maximum_fixed,
            base_pixel: (self.slider.rect.x(), self.thumb_base_y),
            travel_pixels: (0, self.thumb_travel_pixels),
            desired_pixel: (self.slider.rect.x(), desired_mapped_y),
        })
    }
}

fn scroll_for_target(state: &CountryTradeViewerState, target: TradeScrollTarget) -> &TradeViewerScroll {
    match target {
        TradeScrollTarget::ResourceFilters => &state.resource_filter_scroll,
        TradeScrollTarget::CountryFilters => &state.country_filter_scroll,
        TradeScrollTarget::Catalogue => &state.catalogue_scroll,
    }
}

fn scroll_for_target_mut(
    state: &mut CountryTradeViewerState,
    target: TradeScrollTarget,
) -> &mut TradeViewerScroll {
    match target {
        TradeScrollTarget::ResourceFilters => &mut state.resource_filter_scroll,
        TradeScrollTarget::CountryFilters => &mut state.country_filter_scroll,
        TradeScrollTarget::Catalogue => &mut state.catalogue_scroll,
    }
}

fn step_action(target: TradeScrollTarget, direction: i32) -> TradePointerAction {
    match target {
        TradeScrollTarget::ResourceFilters => {
            TradePointerAction::ScrollResourceFilters(direction)
        }
        TradeScrollTarget::CountryFilters => {
            TradePointerAction::ScrollCountryFilters(direction)
        }
        TradeScrollTarget::Catalogue => TradePointerAction::ScrollCatalogueStep(direction),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

fn max_grid_scroll_fixed(
    collection: &TradeCollectionLayout,
    row_count: usize,
    axis: ScrollAxis,
) -> Result<i64> {
    let count = i32::try_from(row_count)
        .map_err(|_| GameUiError::parse("XUI15 collection row count exceeds i32"))?;
    let (columns, rows) = match axis {
        ScrollAxis::Horizontal => (count, i32::from(count > 0)),
        ScrollAxis::Vertical => {
            let columns = i32::try_from(
                collection
                    .grid_layout
                    .max_slots_x
                    .filter(|value| *value > 0)
                    .unwrap_or(1),
            )
            .map_err(|_| GameUiError::parse("XUI15 grid columns exceed i32"))?;
            let rows = count
                .checked_add(columns - 1)
                .map(|value| value / columns)
                .ok_or_else(|| GameUiError::parse("XUI15 grid row count overflows"))?;
            (columns.min(count), rows)
        }
    };
    let content_width = columns
        .checked_mul(collection.grid_layout.slot_size.0)
        .ok_or_else(|| GameUiError::parse("XUI15 grid content width overflows"))?;
    let content_height = rows
        .checked_mul(collection.grid_layout.slot_size.1)
        .ok_or_else(|| GameUiError::parse("XUI15 grid content height overflows"))?;
    let pixels = match axis {
        ScrollAxis::Horizontal => content_width.saturating_sub(collection.visible_clip.w()),
        ScrollAxis::Vertical => content_height.saturating_sub(collection.visible_clip.h()),
    }
    .max(0);
    i64::from(pixels)
        .checked_mul(TRADE_SCROLL_FIXED_SCALE)
        .ok_or_else(|| GameUiError::parse("XUI15 grid fixed scroll range overflows"))
}

fn fixed_pixels(value: i64) -> Result<i32> {
    if value < 0 {
        return Err(GameUiError::parse("XUI15 viewer scroll is negative"));
    }
    i32::try_from(value / TRADE_SCROLL_FIXED_SCALE)
        .map_err(|_| GameUiError::parse("XUI15 viewer scroll exceeds i32"))
}

fn contains(rect: Rect, point: (i32, i32)) -> bool {
    point.0 >= rect.x()
        && point.1 >= rect.y()
        && point.0 < rect.x().saturating_add(rect.w())
        && point.1 < rect.y().saturating_add(rect.h())
}

fn collection(
    cache: &mut BTreeMap<String, LayoutSet>,
    resolver: &AssetResolver,
    registry: Option<&SpriteRegistry>,
    root: &GuiElement,
    container_name: &str,
    grid_name: &str,
    root_rect: Rect,
    viewport: Rect,
    options: &ComposeOptions,
) -> Result<TradeCollectionLayout> {
    let container = direct_child(root, container_name, Some("containerWindowType"))?.clone();
    let grid = direct_child(&container, grid_name, Some("gridBoxType"))?.clone();
    let container_rect = resolve_element_rect(&container, root_rect, (0, 0), options);
    positive_rect(container_rect, container_name)?;
    let grid_layout = gridbox_layout(&grid, container_rect, options)?;
    positive_rect(grid_layout.rect, grid_name)?;

    let clipped_container = intersection(container_rect, viewport).ok_or_else(|| {
        GameUiError::parse(format!(
            "XUI15 {container_name} lies outside the admitted viewport"
        ))
    })?;
    let margin = margins(&container)?;
    let owner_clip = inset(clipped_container, margin)?;
    let visible_clip = if container_name.eq_ignore_ascii_case("trade")
        || container_name.eq_ignore_ascii_case("resources")
    {
        owner_clip
    } else {
        intersection(owner_clip, grid_layout.rect).ok_or_else(|| {
            GameUiError::parse(format!("XUI15 {grid_name} has an empty owner intersection"))
        })?
    };
    positive_rect(visible_clip, &format!("{container_name} visible clip"))?;

    let scroll_wheel_pixels = container
        .raw
        .first_value_ci("scroll_wheel_factor")
        .and_then(scalar_i32)
        .unwrap_or(0);
    if scroll_wheel_pixels < 0 {
        return Err(GameUiError::parse(format!(
            "XUI15 {container_name} scroll_wheel_factor is negative"
        )));
    }
    let smooth_scrolling = match container.raw.first_scalar_ci("smooth_scrolling") {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("yes") || value == "1" => true,
        Some(value) if value.eq_ignore_ascii_case("no") || value == "0" => false,
        Some(_) => {
            return Err(GameUiError::parse(format!(
                "XUI15 {container_name} smooth_scrolling is not boolean"
            )))
        }
    };
    let scrollbar = derive_vertical_scrollbar(
        cache,
        resolver,
        registry,
        &container,
        container_rect,
    )?;
    if !container_name.eq_ignore_ascii_case("resources") && scrollbar.is_none() {
        return Err(GameUiError::missing(format!(
            "XUI15 {container_name} lacks its authored verticalScrollbar"
        )));
    }
    Ok(TradeCollectionLayout {
        container,
        grid,
        container_rect,
        grid_layout,
        visible_clip,
        scroll_wheel_pixels,
        smooth_scrolling,
        scrollbar,
    })
}

pub(crate) fn derive_vertical_scrollbar(
    cache: &mut BTreeMap<String, LayoutSet>,
    resolver: &AssetResolver,
    registry: Option<&SpriteRegistry>,
    container: &GuiElement,
    container_rect: Rect,
) -> Result<Option<TradeScrollbarGeometry>> {
    let Some(style_name) = container
        .raw
        .first_scalar_ci("verticalScrollbar")
        .map(|value| value.trim_matches('"'))
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let style = load_named_element(cache, resolver, "interface/core.gui", style_name)?;
    if !style
        .element_type
        .eq_ignore_ascii_case("extendedScrollbarType")
        || !style
            .orientation
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("upper_right"))
        || !style
            .raw
            .first_scalar_ci("origo")
            .is_some_and(|value| value.trim_matches('"').eq_ignore_ascii_case("upper_right"))
    {
        return Err(GameUiError::parse(format!(
            "XUI15 scrollbar style {style_name:?} is not the exact vertical upper-right extended family"
        )));
    }
    let position = absolute_pair(style.position.as_ref(), "scrollbar position")?;
    let size = style
        .size
        .as_ref()
        .ok_or_else(|| GameUiError::missing("XUI15 scrollbar style has no authored size"))?;
    if size.x.is_percent || size.y.is_percent {
        return Err(GameUiError::parse(
            "XUI15 scrollbar style uses percentage size",
        ));
    }
    let width = crate::compose::measure_to_pixels_for_parent(&size.x, 1);
    if width <= 0 {
        return Err(GameUiError::parse("XUI15 scrollbar width is not positive"));
    }
    let smooth_factor = style
        .raw
        .first_scalar_ci("smooth_scrolling")
        .ok_or_else(|| GameUiError::missing("XUI15 scrollbar style has no smooth factor"))?
        .parse::<f64>()
        .map_err(|_| GameUiError::parse("XUI15 scrollbar smooth factor is not numeric"))?;
    if !smooth_factor.is_finite() || smooth_factor <= 0.0 || smooth_factor > 1.0 {
        return Err(GameUiError::parse(
            "XUI15 scrollbar smooth factor is outside (0, 1]",
        ));
    }
    let smooth_factor_fixed = (smooth_factor * TRADE_SCROLL_FIXED_SCALE as f64).round() as i64;
    let part = |field: &str| -> Result<([i32; 2], [i32; 2], String, Option<(usize, usize)>)> {
        let raw = style.raw.first_value_ci(field).ok_or_else(|| {
            GameUiError::missing(format!("XUI15 scrollbar style lacks {field}"))
        })?;
        let position = raw
            .first_value_ci("position")
            .and_then(crate::gui::parse_measure_pair)
            .as_ref()
            .map(|pair| absolute_pair(Some(pair), field))
            .transpose()?
            .unwrap_or([0, 0]);
        let natural = natural_size_from_raw(raw, resolver, registry, field)?;
        let sprite_name = raw
            .first_scalar_ci("quadTextureSprite")
            .map(|value| value.trim_matches('"'))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| GameUiError::missing(format!("XUI15 {field} has no sprite")))?
            .to_string();
        let border_size = sprite_border_size(registry, &sprite_name)?;
        Ok((position, natural, sprite_name, border_size))
    };
    let (_background_position, _background_size, background_sprite, background_border) =
        part("background")?;
    let (slider_position, slider_size, slider_sprite, slider_border) = part("slider")?;
    let (decrease_position, decrease_size, decrease_sprite, decrease_border) =
        part("decreaseButton")?;
    let (increase_position, increase_size, increase_sprite, increase_border) =
        part("increaseButton")?;
    let (track_position, track_size, track_sprite, track_border) = part("track")?;
    let scrollbar_x = container_rect
        .x()
        .checked_add(container_rect.w())
        .and_then(|value| value.checked_add(position[0]))
        .and_then(|value| value.checked_sub(width))
        .ok_or_else(|| GameUiError::parse("XUI15 scrollbar x overflows"))?;
    let scrollbar_y = container_rect
        .y()
        .checked_add(position[1])
        .ok_or_else(|| GameUiError::parse("XUI15 scrollbar y overflows"))?;
    let checked_offset = |base: i32, offset: i32, label: &str| {
        base.checked_add(offset)
            .ok_or_else(|| GameUiError::parse(format!("XUI15 {label} position overflows")))
    };
    let background_rect = Rect::new(scrollbar_x, scrollbar_y, width, container_rect.h());
    let decrease_rect = Rect::new(
        checked_offset(scrollbar_x, decrease_position[0], "scrollbar decrease x")?,
        checked_offset(scrollbar_y, decrease_position[1], "scrollbar decrease y")?,
        decrease_size[0],
        decrease_size[1],
    );
    let increase_rect = Rect::new(
        scrollbar_x
            .checked_add(width)
            .and_then(|value| value.checked_add(increase_position[0]))
            .ok_or_else(|| GameUiError::parse("XUI15 scrollbar increase x overflows"))?,
        scrollbar_y
            .checked_add(container_rect.h())
            .and_then(|value| value.checked_add(increase_position[1]))
            .ok_or_else(|| GameUiError::parse("XUI15 scrollbar increase y overflows"))?,
        increase_size[0],
        increase_size[1],
    );
    let track_y = decrease_rect
        .y()
        .checked_add(decrease_rect.h())
        .ok_or_else(|| GameUiError::parse("XUI15 scrollbar track y overflows"))?;
    let track_height = increase_rect
        .y()
        .checked_sub(track_y)
        .ok_or_else(|| GameUiError::parse("XUI15 scrollbar track height overflows"))?;
    if track_height <= 0 {
        return Err(GameUiError::parse("XUI15 scrollbar track is not positive"));
    }
    let track_rect = Rect::new(
        checked_offset(scrollbar_x, track_position[0], "scrollbar track x")?,
        track_y,
        track_size[0],
        track_height,
    );
    let slider_rect = Rect::new(
        checked_offset(scrollbar_x, slider_position[0], "scrollbar slider x")?,
        track_y
            .checked_add(slider_position[1])
            .ok_or_else(|| GameUiError::parse("XUI15 scrollbar slider y overflows"))?,
        slider_size[0],
        slider_size[1],
    );
    let thumb_travel_pixels = track_height
        .checked_sub(slider_size[1])
        .ok_or_else(|| GameUiError::parse("XUI15 scrollbar thumb travel overflows"))?;
    positive_rect(background_rect, "XUI15 scrollbar background")?;
    positive_rect(decrease_rect, "XUI15 scrollbar decrease")?;
    positive_rect(increase_rect, "XUI15 scrollbar increase")?;
    positive_rect(track_rect, "XUI15 scrollbar track")?;
    positive_rect(slider_rect, "XUI15 scrollbar slider")?;
    Ok(Some(TradeScrollbarGeometry {
        background: TradeScrollbarPartGeometry {
            rect: background_rect,
            sprite_name: background_sprite,
            border_size: background_border,
        },
        track: TradeScrollbarPartGeometry {
            rect: track_rect,
            sprite_name: track_sprite,
            border_size: track_border,
        },
        decrease: TradeScrollbarPartGeometry {
            rect: decrease_rect,
            sprite_name: decrease_sprite,
            border_size: decrease_border,
        },
        increase: TradeScrollbarPartGeometry {
            rect: increase_rect,
            sprite_name: increase_sprite,
            border_size: increase_border,
        },
        slider: TradeScrollbarPartGeometry {
            rect: slider_rect,
            sprite_name: slider_sprite,
            border_size: slider_border,
        },
        thumb_base_y: track_y,
        thumb_travel_pixels,
        thumb_authored_offset_y: slider_position[1],
        smooth_factor_fixed,
    }))
}

fn sprite_border_size(
    registry: Option<&SpriteRegistry>,
    sprite_name: &str,
) -> Result<Option<(usize, usize)>> {
    let Some(registry) = registry else {
        return Ok(None);
    };
    let sprite = registry
        .get(sprite_name)
        .ok_or_else(|| GameUiError::missing(format!("XUI15 missing sprite {sprite_name}")))?;
    let Some(border) = &sprite.border_size else {
        return Ok(None);
    };
    let parse = |value: &str, axis: &str| -> Result<usize> {
        let value = value.parse::<f64>().map_err(|_| {
            GameUiError::parse(format!("XUI15 {sprite_name} {axis} border is not numeric"))
        })?;
        if !value.is_finite() || value < 0.0 || value > usize::MAX as f64 {
            return Err(GameUiError::parse(format!(
                "XUI15 {sprite_name} {axis} border is invalid"
            )));
        }
        Ok(value.round() as usize)
    };
    Ok(Some((parse(&border.x, "x")?, parse(&border.y, "y")?)))
}

fn absolute_pair(
    pair: Option<&crate::gui::MeasurePair>,
    label: &str,
) -> Result<[i32; 2]> {
    let Some(pair) = pair else {
        return Ok([0, 0]);
    };
    if pair.x.is_percent || pair.y.is_percent {
        return Err(GameUiError::parse(format!(
            "XUI15 {label} uses percentage geometry"
        )));
    }
    Ok([
        crate::compose::measure_to_pixels_for_parent(&pair.x, 1),
        crate::compose::measure_to_pixels_for_parent(&pair.y, 1),
    ])
}

fn natural_size_from_raw(
    raw: &crate::paradox_text::Value,
    resolver: &AssetResolver,
    registry: Option<&SpriteRegistry>,
    label: &str,
) -> Result<[i32; 2]> {
    if let Some(size) = raw
        .first_value_ci("size")
        .and_then(crate::gui::parse_measure_pair)
    {
        let size = absolute_pair(Some(&size), label)?;
        if size[0] > 0 && size[1] > 0 {
            return Ok(size);
        }
    }
    let sprite_name = raw
        .first_scalar_ci("quadTextureSprite")
        .map(|value| value.trim_matches('"'))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| GameUiError::missing(format!("XUI15 {label} has no sprite")))?;
    let registry = registry.ok_or_else(|| {
        GameUiError::missing(format!("XUI15 {label} requires effective sprite geometry"))
    })?;
    let sprite = registry
        .get(sprite_name)
        .ok_or_else(|| GameUiError::missing(format!("XUI15 missing sprite {sprite_name}")))?;
    if let Some(size) = &sprite.size {
        let x = size.x.parse::<i32>().ok();
        let y = size.y.parse::<i32>().ok();
        if let (Some(x), Some(y)) = (x, y) {
            if x > 0 && y > 0 {
                return Ok([x, y]);
            }
        }
    }
    let decoded = resolver.decode_sprite(sprite)?;
    let frame = decoded
        .frames
        .first()
        .ok_or_else(|| GameUiError::missing(format!("XUI15 sprite {sprite_name} has no frame")))?;
    let width = i32::try_from(frame.width)
        .map_err(|_| GameUiError::parse("XUI15 sprite width exceeds i32"))?;
    let height = i32::try_from(frame.height)
        .map_err(|_| GameUiError::parse("XUI15 sprite height exceeds i32"))?;
    if width <= 0 || height <= 0 {
        return Err(GameUiError::missing(format!(
            "XUI15 sprite {sprite_name} has empty geometry"
        )));
    }
    Ok([width, height])
}

fn button_rect_with_natural_size(
    button: &GuiElement,
    parent: Rect,
    options: &ComposeOptions,
    resolver: &AssetResolver,
    registry: Option<&SpriteRegistry>,
) -> Result<Rect> {
    let mut button = button.clone();
    if button.size.is_none() {
        let size = natural_size_from_raw(&button.raw, resolver, registry, &button.name)?;
        button.size = Some(pair(size[0], size[1]));
    }
    let rect = resolve_element_rect(&button, parent, (0, 0), options);
    positive_rect(rect, &button.name)?;
    Ok(rect)
}

fn direct_child<'a>(
    parent: &'a GuiElement,
    name: &str,
    element_type: Option<&str>,
) -> Result<&'a GuiElement> {
    let matches = parent
        .children
        .iter()
        .filter(|child| {
            child.name.eq_ignore_ascii_case(name)
                && element_type.is_none_or(|kind| child.element_type.eq_ignore_ascii_case(kind))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(GameUiError::missing(format!(
            "{COUNTRY_TRADE_GUI}: {} expected exactly one direct {name} child, found {}",
            parent.name,
            matches.len()
        )));
    }
    Ok(matches[0])
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Margins {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn margins(element: &GuiElement) -> Result<Margins> {
    let Some(raw) = element.raw.first_value_ci("margin") else {
        return Ok(Margins::default());
    };
    let parse = |key: &str| raw.first_value_ci(key).and_then(scalar_i32).unwrap_or(0);
    let margins = Margins {
        left: parse("left"),
        top: parse("top"),
        right: parse("right"),
        bottom: parse("bottom"),
    };
    if [margins.left, margins.top, margins.right, margins.bottom]
        .into_iter()
        .any(|value| value < 0)
    {
        return Err(GameUiError::parse(format!(
            "XUI15 {} has a negative margin",
            element.name
        )));
    }
    Ok(margins)
}

fn inset(rect: Rect, margins: Margins) -> Result<Rect> {
    let x = rect
        .x()
        .checked_add(margins.left)
        .ok_or_else(|| GameUiError::parse("XUI15 margin x overflow"))?;
    let y = rect
        .y()
        .checked_add(margins.top)
        .ok_or_else(|| GameUiError::parse("XUI15 margin y overflow"))?;
    let width = rect
        .w()
        .checked_sub(margins.left)
        .and_then(|value| value.checked_sub(margins.right))
        .ok_or_else(|| GameUiError::parse("XUI15 margin width overflow"))?;
    let height = rect
        .h()
        .checked_sub(margins.top)
        .and_then(|value| value.checked_sub(margins.bottom))
        .ok_or_else(|| GameUiError::parse("XUI15 margin height overflow"))?;
    let result = Rect::new(x, y, width, height);
    positive_rect(result, "inset clip")?;
    Ok(result)
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
    let x0 = left.x().max(right.x());
    let y0 = left.y().max(right.y());
    let x1 = left
        .x()
        .checked_add(left.w())?
        .min(right.x().checked_add(right.w())?);
    let y1 = left
        .y()
        .checked_add(left.h())?
        .min(right.y().checked_add(right.h())?);
    (x1 > x0 && y1 > y0).then_some(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

fn positive_rect(rect: Rect, label: &str) -> Result<()> {
    if rect.w() <= 0 || rect.h() <= 0 {
        return Err(GameUiError::parse(format!(
            "XUI15 {label} has non-positive resolved geometry"
        )));
    }
    Ok(())
}
