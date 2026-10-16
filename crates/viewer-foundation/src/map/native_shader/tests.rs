use super::*;
use crate::map::test_support::gpu::*;
use installed_content::pdxshader::program::{Program, Stage};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
mod models;
mod parts;
// Supply these native define families.
fn is_gui_effect(name: &str) -> bool {
    name.ends_with(".shader")
        && (name.starts_with("buttonstate")
            || matches!(
                name,
                "static_button.shader"
                    | "text.shader"
                    | "maskedflag.shader"
                    | "progress.shader"
                    | "progress_minmax.shader"
                    | "progress_startend.shader"
                    | "progress_reverse.shader"
                    | "progress_radial.shader"
                    | "circularprogressbar.shader"
            ))
}

// Compile each path only where the authored code tests it; names do not select a model.
fn variants(program: &Program) -> Vec<Vec<String>> {
    let source = [&program.common, &program.vertex, &program.pixel]
        .iter()
        .flat_map(|stage| {
            stage
                .code
                .iter()
                .chain(stage.entries.iter().map(|entry| &entry.source))
        })
        .map(|source| source.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut variants = vec![Vec::new()];
    if source.contains("ifdef ANIMATED") {
        variants.push(vec!["ANIMATED".into()]);
        variants.push(vec!["ANIMATED".into(), "NUM_ANIMATIONS_2".into()]);
    }
    if source.contains("ifdef MASKING") {
        let masked: Vec<_> = variants
            .iter()
            .map(|defines| {
                let mut defines = defines.clone();
                defines.push("MASKING".into());
                defines
            })
            .collect();
        variants.extend(masked);
    }
    variants
}
