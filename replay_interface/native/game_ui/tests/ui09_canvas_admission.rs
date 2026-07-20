use game_ui::research_canvas::{
    admit_research_canvas, AdmissionRejectionCode, DoctrineCanvasRequest, FocusCanvasRequest,
    LocalizedTextWitness, PixelRect, ProducerDoctrineLayout, ProducerDoctrineTrack,
    ProducerResolvedFocusPositions, ProducerTechnologyLayout, RecordedFocusTransform,
    ResearchCanvasKind, ResearchCanvasRequest, TechnologyCanvasRequest,
};
use std::collections::BTreeMap;

fn text() -> LocalizedTextWitness {
    LocalizedTextWitness {
        locale: Some("l_english".to_string()),
        required_keys: vec!["FOCUS_NAME".to_string()],
        resolved_strings: BTreeMap::from([(
            "FOCUS_NAME".to_string(),
            "Industrial Effort".to_string(),
        )]),
    }
}

fn focus() -> FocusCanvasRequest {
    FocusCanvasRequest {
        viewport: [2752, 1152],
        previous_viewport: [2752, 1152],
        requested_transform_state_id: "frame38304".to_string(),
        recorded_transform: Some(RecordedFocusTransform {
            source_record_id: "producer:GER:frame38304".to_string(),
            state_id: "frame38304".to_string(),
            viewport: [2752, 1152],
            scale: 0.7698728191475303,
            offset_pixels: [1631.9478310913767, -82.74048494411414],
        }),
        node_ids: vec!["GER_focus_a".to_string(), "joint_focus_0".to_string()],
        resolved_positions: Some(ProducerResolvedFocusPositions {
            source_record_id: "producer:GER:frame38304".to_string(),
            positions: BTreeMap::from([
                ("GER_focus_a".to_string(), [100.0, 88.0]),
                ("joint_focus_0".to_string(), [196.0, 218.0]),
            ]),
        }),
        text: text(),
    }
}

fn technology() -> TechnologyCanvasRequest {
    TechnologyCanvasRequest {
        drawable_ids: vec![
            "infantry_weapons".to_string(),
            "infantry_to_support_link".to_string(),
            "support_weapons".to_string(),
        ],
        producer_layout: Some(ProducerTechnologyLayout {
            source_record_id: "record:technology:42".to_string(),
            drawable_rectangles: Some(BTreeMap::from([
                (
                    "infantry_weapons".to_string(),
                    PixelRect {
                        x: 24,
                        y: 138,
                        width: 183,
                        height: 84,
                    },
                ),
                (
                    "infantry_to_support_link".to_string(),
                    PixelRect {
                        x: 194,
                        y: 164,
                        width: 70,
                        height: 70,
                    },
                ),
                (
                    "support_weapons".to_string(),
                    PixelRect {
                        x: 220,
                        y: 138,
                        width: 72,
                        height: 72,
                    },
                ),
            ])),
            draw_order: Some(vec![
                "infantry_weapons".to_string(),
                "infantry_to_support_link".to_string(),
                "support_weapons".to_string(),
            ]),
            pan_pixels: Some([0.0, -40.0]),
            clip: Some(PixelRect {
                x: 21,
                y: 138,
                width: 2706,
                height: 990,
            }),
        }),
        text: text(),
    }
}

fn doctrine() -> DoctrineCanvasRequest {
    DoctrineCanvasRequest {
        track_ids: vec!["land_doctrine".to_string()],
        producer_layout: Some(ProducerDoctrineLayout {
            source_record_id: "record:doctrine:17".to_string(),
            tracks: Some(BTreeMap::from([(
                "land_doctrine".to_string(),
                ProducerDoctrineTrack {
                    extent: PixelRect {
                        x: 20,
                        y: 225,
                        width: 424,
                        height: 764,
                    },
                    selected_candidate_id: Some("mobile_warfare".to_string()),
                    candidate_order: Some(vec![
                        "mobile_warfare".to_string(),
                        "superior_firepower".to_string(),
                    ]),
                },
            )])),
            clip: Some(PixelRect {
                x: 20,
                y: 225,
                width: 424,
                height: 764,
            }),
        }),
        text: text(),
    }
}

fn rejection(request: ResearchCanvasRequest) -> AdmissionRejectionCode {
    admit_research_canvas(&request).unwrap_err().code
}

#[test]
fn admits_complete_recorded_focus_technology_and_doctrine_inputs() {
    let cases = [
        (
            ResearchCanvasRequest::Focus(focus()),
            ResearchCanvasKind::Focus,
        ),
        (
            ResearchCanvasRequest::Technology(technology()),
            ResearchCanvasKind::Technology,
        ),
        (
            ResearchCanvasRequest::Doctrine(doctrine()),
            ResearchCanvasKind::Doctrine,
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(admit_research_canvas(&request).unwrap().kind(), expected);
    }
}

#[test]
fn rejects_missing_post_resize_focus_affine() {
    let mut request = focus();
    request.previous_viewport = [1920, 1080];
    request.recorded_transform = None;
    assert_eq!(
        rejection(ResearchCanvasRequest::Focus(request)),
        AdmissionRejectionCode::FocusResizeTransformState
    );
}

#[test]
fn admits_post_resize_focus_affine_recorded_for_target_viewport() {
    let mut request = focus();
    request.previous_viewport = [1920, 1080];
    assert_eq!(
        admit_research_canvas(&ResearchCanvasRequest::Focus(request))
            .unwrap()
            .kind(),
        ResearchCanvasKind::Focus
    );
}

#[test]
fn rejects_unrecorded_focus_zoom_state() {
    let mut request = focus();
    request.requested_transform_state_id = "unseen_zoom".to_string();
    assert_eq!(
        rejection(ResearchCanvasRequest::Focus(request)),
        AdmissionRejectionCode::FocusUnobservedZoomState
    );
}

#[test]
fn rejects_unresolved_shared_focus_offsets() {
    let mut request = focus();
    request.resolved_positions = None;
    assert_eq!(
        rejection(ResearchCanvasRequest::Focus(request)),
        AdmissionRejectionCode::FocusSharedOffsetPrecedence
    );
}

#[test]
fn rejects_focus_values_from_different_producer_records() {
    let mut request = focus();
    request
        .resolved_positions
        .as_mut()
        .unwrap()
        .source_record_id = "producer:GER:another-transition".to_string();
    assert_eq!(
        rejection(ResearchCanvasRequest::Focus(request)),
        AdmissionRejectionCode::FocusSharedOffsetPrecedence
    );
}

#[test]
fn rejects_technology_without_complete_producer_canvas_values() {
    let mut missing_rectangles = technology();
    missing_rectangles
        .producer_layout
        .as_mut()
        .unwrap()
        .drawable_rectangles = None;
    let mut missing_order = technology();
    missing_order.producer_layout.as_mut().unwrap().draw_order = None;
    let mut missing_pan = technology();
    missing_pan.producer_layout.as_mut().unwrap().pan_pixels = None;
    let mut missing_clip = technology();
    missing_clip.producer_layout.as_mut().unwrap().clip = None;
    for request in [missing_rectangles, missing_order, missing_pan, missing_clip] {
        assert_eq!(
            rejection(ResearchCanvasRequest::Technology(request)),
            AdmissionRejectionCode::TechnologyLiveCanvas
        );
    }
}

#[test]
fn rejects_doctrine_without_extent_selection_order_and_clip() {
    let mut missing_extent = doctrine();
    missing_extent
        .producer_layout
        .as_mut()
        .unwrap()
        .tracks
        .as_mut()
        .unwrap()
        .get_mut("land_doctrine")
        .unwrap()
        .extent
        .width = 0;
    let mut missing_selection = doctrine();
    missing_selection
        .producer_layout
        .as_mut()
        .unwrap()
        .tracks
        .as_mut()
        .unwrap()
        .get_mut("land_doctrine")
        .unwrap()
        .selected_candidate_id = None;
    let mut missing_order = doctrine();
    missing_order
        .producer_layout
        .as_mut()
        .unwrap()
        .tracks
        .as_mut()
        .unwrap()
        .get_mut("land_doctrine")
        .unwrap()
        .candidate_order = None;
    let mut missing_clip = doctrine();
    missing_clip.producer_layout.as_mut().unwrap().clip = None;
    for request in [
        missing_extent,
        missing_selection,
        missing_order,
        missing_clip,
    ] {
        assert_eq!(
            rejection(ResearchCanvasRequest::Doctrine(request)),
            AdmissionRejectionCode::DoctrineCodeWrittenExtentAndOrder
        );
    }
}

#[test]
fn rejects_runtime_text_without_locale_and_resolved_strings() {
    let mut missing_locale = technology();
    missing_locale.text.locale = None;
    let mut missing_string = technology();
    missing_string.text.resolved_strings.clear();
    for request in [missing_locale, missing_string] {
        assert_eq!(
            rejection(ResearchCanvasRequest::Technology(request)),
            AdmissionRejectionCode::RuntimeLocalizedTextExtent
        );
    }
}
