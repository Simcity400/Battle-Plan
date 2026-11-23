
use std::sync::Arc;
use viewer_native_gui::sprites::{Batch, Quad};

pub(crate) fn clockless(batch: &Batch) -> Vec<Quad> {
    use viewer_native_gui::native_shader::draw::SPRITE_TIME;
    let mut quads = batch.quads.clone();
    for quad in &mut quads {
        if let Some(exact) = &mut quad.exact {
            exact.inputs[0] = 0.;
            if let Some(native) = &mut exact.native {
                let native = Arc::make_mut(native);
                if let Some(time) = native.constants.get_mut(SPRITE_TIME as usize / 4) {
                    *time = 0.;
                }
            }
        }
    }
    quads
}

pub(crate) fn first_difference(laid: &[Quad], fresh: &[Quad]) -> Option<String> {
    let name = |quad: &Quad| {
        quad.exact
            .as_ref()
            .and_then(|exact| exact.native.as_ref())
            .map_or_else(|| format!("a quad of texture {}", quad.texture), |native| native.path.clone())
    };
    if let Some(at) = laid.iter().zip(fresh).position(|(a, b)| a != b) {
        let differing = laid.iter().zip(fresh).filter(|(a, b)| a != b).count();
        return Some(format!(
            "quad {at} of {} differs in {} ({differing} of the quads both drew differ): {} (worker) against {} (in place)",
            laid.len(),
            differing_fields(&laid[at], &fresh[at]).join(", "),
            name(&laid[at]),
            name(&fresh[at])
        ));
    }
    (laid.len() != fresh.len()).then(|| {
        let longer = if laid.len() > fresh.len() { &laid[fresh.len()] } else { &fresh[laid.len()] };
        format!("the worker drew {} quads and the frame {}; the first unmatched is {}", laid.len(), fresh.len(), name(longer))
    })
}

pub(crate) fn layers_moved(painting: [usize; 6], fresh: [usize; 6]) -> Option<String> {
    let moved = viewer_counters::counters::layers_moved(painting, fresh);
    (!moved.is_empty()).then(|| moved.join(", "))
}

pub(crate) fn first_report_difference(what: &str, laid: &impl std::fmt::Debug, fresh: &impl std::fmt::Debug) -> String {
    let (laid, fresh) = (format!("{laid:#?}"), format!("{fresh:#?}"));
    let mut lines = laid.lines().zip(fresh.lines()).enumerate().skip_while(|(_, (a, b))| a == b);
    match lines.next() {
        Some((at, (a, b))) => format!("the {what} differ in {}: {} (worker) against {} (in place)", enclosing(&laid, at), a.trim(), b.trim()),
        None => format!("the {what} differ in length: {} lines (worker) against {} (in place)", laid.lines().count(), fresh.lines().count()),
    }
}

fn enclosing(debug: &str, at: usize) -> String {
    let lines: Vec<&str> = debug.lines().take(at + 1).collect();
    let mut path: Vec<&str> = Vec::new();
    let mut inside = lines.last().map_or(usize::MAX, |line| indent(line));
    for line in lines.iter().rev().skip(1) {
        let depth = indent(line);
        if depth < inside && !line.trim().is_empty() {
            path.push(line.trim());
            inside = depth;
        }
    }
    path.reverse();
    path.join(" ")
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn differing_fields(laid: &Quad, fresh: &Quad) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut note = |name: &str, differs: bool| {
        if differs {
            found.push(name.to_owned());
        }
    };
    note("texture", laid.texture != fresh.texture);
    note("rect", laid.rect != fresh.rect);
    note("uv", laid.uv != fresh.uv);
    note("color", laid.color != fresh.color);
    note("rotation", laid.rotation != fresh.rotation);
    note("layer", laid.layer != fresh.layer);
    note("shade", laid.shade != fresh.shade);
    note("anchor", laid.anchor != fresh.anchor);
    note("present", laid.present != fresh.present);
    match (&laid.exact, &fresh.exact) {
        (Some(laid), Some(fresh)) => {
            note("exact.corners", laid.corners != fresh.corners);
            note("exact.uv", laid.uv != fresh.uv);
            note("exact.scissor", laid.scissor != fresh.scissor);
            note("exact.technique", laid.technique != fresh.technique);
            note("exact.blend", laid.blend != fresh.blend);
            note("exact.samplers", laid.samplers != fresh.samplers);
            note("exact.lod_bias", laid.lod_bias != fresh.lod_bias);
            note("exact.second", laid.second != fresh.second);
            note("exact.masking", laid.masking != fresh.masking);
            note("exact.offsets", laid.offsets != fresh.offsets);
            note("exact.inputs", laid.inputs != fresh.inputs);
            note("exact.second_color", laid.second_color != fresh.second_color);
            match (&laid.native, &fresh.native) {
                (Some(laid), Some(fresh)) => {
                    note("native.path", laid.path != fresh.path);
                    note("native.effect", laid.effect != fresh.effect);
                    note("native.declaration", laid.declaration != fresh.declaration);
                    note("native.vertices", laid.vertices != fresh.vertices);
                    note("native.constants", laid.constants != fresh.constants);
                    note("native.known", laid.known != fresh.known);
                    note("native.textures", laid.textures != fresh.textures);
                    note("native.extra_constants", laid.extra_constants != fresh.extra_constants);
                    note("native.font_viewport", laid.font_viewport != fresh.font_viewport);
                    note("native.topology", laid.topology != fresh.topology);
                    if laid != fresh && found.is_empty() {
                        found.push("native, in a part not named here".to_owned());
                    }
                }
                (laid, fresh) => note("native", laid.is_some() != fresh.is_some()),
            }
        }
        (laid, fresh) => note("exact", laid.is_some() != fresh.is_some()),
    }
    if found.is_empty() {
        found.push("a part not named here".to_owned());
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(texture: usize, layer: u64) -> Quad {
        Quad {
            texture,
            rect: [0.; 4],
            uv: [0.; 4],
            color: [1.; 4],
            rotation: None,
            layer,
            shade: 0.,
            exact: None,
            anchor: None,
            present: None,
        }
    }

    #[test]
    fn the_gate_names_the_layers_whose_product_moved_under_it() {
        assert_eq!(layers_moved([1, 2, 3, 4, 5, 6], [1, 2, 3, 4, 5, 6]), None);
        assert_eq!(layers_moved([1, 2, 3, 4, 5, 6], [1, 9, 3, 4, 5, 6]).as_deref(), Some("land combat markers"));
        assert_eq!(
            layers_moved([1, 2, 3, 4, 5, 6], [8, 2, 3, 4, 5, 0]).as_deref(),
            Some("land stacks, air missions")
        );
    }

    #[test]
    fn the_gate_names_the_first_differing_quad() {
        assert_eq!(first_difference(&[quad(1, 0), quad(2, 0)], &[quad(1, 0), quad(2, 0)]), None);
        let differs = first_difference(&[quad(1, 0), quad(2, 0)], &[quad(1, 0), quad(2, 5)]).unwrap();
        assert!(differs.starts_with("quad 1 of 2 differs in layer (1 of the quads both drew differ): a quad of texture 2"), "{differs}");
        let shorter = first_difference(&[quad(1, 0)], &[quad(1, 0), quad(3, 0)]).unwrap();
        assert!(shorter.contains("the worker drew 1 quads and the frame 2") && shorter.contains("texture 3"), "{shorter}");
    }
}
