use game_ui::compose::Rect;
use game_ui::scrollbar_law::{
    extended_displayed_value, extended_thumb_pixel, extended_value_for_pixel, legacy_thumb_rect,
    ExtendedScrollbarCarrier, ExtendedScrollbarInverseCarrier, LegacyScrollbarCarrier,
    ScrollbarAxis, ScrollbarCarrierAuthority,
};

const A729: ScrollbarCarrierAuthority = ScrollbarCarrierAuthority::ExactA729Runtime;

#[test]
fn retained_frame36172_legacy_geometry_joins_fixed_thumb_law() {
    let rect = legacy_thumb_rect(LegacyScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed: 1_700_000,
        value_f32: 9.0,
        base_pixel_f32: (661.0, 560.0),
        leading_axis_position: 544,
        trailing_axis_position: 1120,
        leading_axis_size: 16,
        fixed_thumb_size: (16, 16),
    })
    .expect("exact legacy carrier");
    assert_eq!(rect, Rect::new(661, 848, 16, 16));
}

#[test]
fn extended_nonzero_minimum_and_half_away_rounding_are_exact() {
    let pixel = extended_thumb_pixel(ExtendedScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Horizontal,
        min_fixed: 100_000,
        max_fixed: 1_300_000,
        value_fixed: 700_000,
        base_pixel: (10, 20),
        travel_pixels: (101, 0),
    })
    .expect("exact extended carrier");
    assert_eq!(pixel, (61, 20));
}

#[test]
fn extended_zero_range_stays_at_base() {
    let pixel = extended_thumb_pixel(ExtendedScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 500_000,
        max_fixed: 500_000,
        value_fixed: 500_000,
        base_pixel: (7, 11),
        travel_pixels: (0, -37),
    })
    .expect("zero range is the proven t=0 branch");
    assert_eq!(pixel, (7, 11));
}

#[test]
fn legacy_final_conversion_truncates_toward_zero() {
    let rect = legacy_thumb_rect(LegacyScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Horizontal,
        min_fixed: 0,
        max_fixed: 300_000,
        value_f32: 1.0,
        base_pixel_f32: (-10.75, 4.9),
        leading_axis_position: 0,
        trailing_axis_position: 32,
        leading_axis_size: 2,
        fixed_thumb_size: (3, 5),
    })
    .expect("exact legacy carrier");
    assert_eq!(rect, Rect::new(-1, 4, 3, 5));
}

#[test]
fn unvalidated_or_unresolved_carriers_fail_closed() {
    let error = extended_thumb_pixel(ExtendedScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed: 100_000,
        value_fixed: 100_001,
        base_pixel: (0, 0),
        travel_pixels: (0, 10),
    })
    .expect_err("out-of-range value must not be inferred or clamped here");
    assert!(error.message.contains("UI06_FAIL_CLOSED"));

    let error = legacy_thumb_rect(LegacyScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed: 100_000,
        value_f32: 0.5,
        base_pixel_f32: (0.0, 0.0),
        leading_axis_position: 0,
        trailing_axis_position: 10,
        leading_axis_size: 0,
        fixed_thumb_size: (0, 0),
    })
    .expect_err("missing resolved thumb size must remain closed");
    assert!(error.message.contains("UI06_FAIL_CLOSED"));
}

#[test]
fn smooth_update_uses_integer_order_and_exact_snap_boundary() {
    assert_eq!(
        extended_displayed_value(0, 100_000, 25_000, true).unwrap(),
        25_000
    );
    assert_eq!(
        extended_displayed_value(90_000, 100_000, 25_000, true).unwrap(),
        100_000
    );
    assert_eq!(
        extended_displayed_value(0, 100_000, 25_000, false).unwrap(),
        100_000
    );
}

#[test]
fn extended_inverse_is_bounded_and_round_trips_exact_thumb_pixels() {
    let carrier = ExtendedScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed: 9_000_000,
        value_fixed: 4_500_000,
        base_pixel: (7, 100),
        travel_pixels: (0, 91),
    };
    let pixel = extended_thumb_pixel(carrier).unwrap();
    let value = extended_value_for_pixel(ExtendedScrollbarInverseCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: carrier.min_fixed,
        max_fixed: carrier.max_fixed,
        base_pixel: carrier.base_pixel,
        travel_pixels: carrier.travel_pixels,
        desired_pixel: pixel,
    })
    .expect("invert exact mapped pixel");
    assert_eq!(
        extended_thumb_pixel(ExtendedScrollbarCarrier {
            value_fixed: value,
            ..carrier
        })
        .unwrap(),
        pixel
    );
    assert_eq!(
        extended_value_for_pixel(ExtendedScrollbarInverseCarrier {
            desired_pixel: (7, -10_000),
            ..ExtendedScrollbarInverseCarrier {
                authority: A729,
                axis: ScrollbarAxis::Vertical,
                min_fixed: carrier.min_fixed,
                max_fixed: carrier.max_fixed,
                base_pixel: carrier.base_pixel,
                travel_pixels: carrier.travel_pixels,
                desired_pixel: pixel,
            }
        })
        .unwrap(),
        0
    );
    assert_eq!(
        extended_value_for_pixel(ExtendedScrollbarInverseCarrier {
            authority: A729,
            axis: ScrollbarAxis::Vertical,
            min_fixed: 0,
            max_fixed: 100_000,
            base_pixel: (0, 20),
            travel_pixels: (0, -4),
            desired_pixel: (0, 16),
        })
        .expect("invert exact signed short-track travel"),
        87_500
    );
}

#[test]
fn pixel_conversion_rejects_i32_max_plus_one_instead_of_saturating() {
    let error = legacy_thumb_rect(LegacyScrollbarCarrier {
        authority: A729,
        axis: ScrollbarAxis::Vertical,
        min_fixed: 0,
        max_fixed: 100_000,
        value_f32: 0.0,
        base_pixel_f32: (2_147_483_648.0, 0.0),
        leading_axis_position: 0,
        trailing_axis_position: 20,
        leading_axis_size: 1,
        fixed_thumb_size: (1, 1),
    })
    .expect_err("Rust saturating casts must not replace CVTTSS2SI failure semantics");
    assert!(error.message.contains("UI06_FAIL_CLOSED"));
}
