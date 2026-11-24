use eframe::egui;
use std::collections::BTreeMap;

pub fn warning_list(ui: &mut egui::Ui, warnings: &[String], max_height: f32) {
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for warning in warnings {
                ui.colored_label(egui::Color32::YELLOW, warning);
            }
        });
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    pub fn assert_diagnostics(collect: impl FnOnce() -> Vec<String>, expected: &[String]) {
        let ctx = egui::Context::default();
        let warnings = collect();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| warning_list(ui, &warnings, 800.0),
        );
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        for line in expected {
            assert!(warnings.contains(line), "missing reported warning: {line}");
            assert!(
                labels.contains(&line.as_str()),
                "missing painted warning: {line}"
            );
        }
    }
}

pub fn tooltip_warning_lines(
    refused: &BTreeMap<String, usize>,
) -> impl Iterator<Item = String> + '_ {
    refused
        .iter()
        .map(|(reason, count)| format!("Tooltip: {count} × {reason}"))
}

