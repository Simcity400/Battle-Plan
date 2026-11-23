use shader_pipeline::*;
use super::*;
use viewer_gpu::test_support::gpu::*;
use content_reader_gui::pdxshader::program::{Program, Stage};
use std::{
    collections::BTreeMap,
    path::Path,
};
mod models;
mod parts;
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
