//! Generic ribbon emission, allocation and ordered composition for both producers.
use super::{
    province_map::HeightField,
    ribbon_content::*,
    ribbon_fixed::{self as fixed, Point},
    ribbon_native as native,
};

pub fn sort_draws(draws: &mut [OrderDraw]) {
    draws.sort_by(|a, b| {
        a.declaration_index.cmp(&b.declaration_index).then_with(|| {
            b.width
                .partial_cmp(&a.width)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
}

/// Append `more`'s ribbons after `field`'s in the shared arrow manager's
/// draw order. Raid and battle-plan arrows use the same manager, so a
/// definition bucket holds both. Labels and the report stay `field`'s.
pub fn append(field: &OrderField, more: &OrderField) -> OrderField {
    let mut out = field.clone();
    let base = out.native_vertices.len() as u32;
    out.vertices.extend_from_slice(&more.vertices);
    out.native_vertices.extend_from_slice(&more.native_vertices);
    out.raid_units.extend(more.raid_units.iter().cloned());
    out.trade_route_hovers
        .extend_from_slice(&more.trade_route_hovers);
    out.draws.extend(more.draws.iter().map(|draw| {
        OrderDraw {
            native_strips: draw
                .native_strips
                .iter()
                .map(|strip| strip.start + base..strip.end + base)
                .collect(),
            label: None,
            ..draw.clone()
        }
    }));
    sort_draws(&mut out.draws);
    out
}

pub fn ribbon_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    let width = heights.width as f32;
    let x = if x < 0.0 {
        x + width
    } else if x >= width {
        x - width
    } else {
        x
    };
    heights.border_height_at_world(x, z) + 0.01
}

pub fn span(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Smooth the fixed controls once and emit one pair per smoothed control up
/// to the head root, adding no samples between them, then the head.
/// Return the new draw's index.
pub fn emit(
    field: &mut OrderField,
    arrow: &ArrowType,
    controls: &[Point],
    scale: f32,
    color: [f32; 4],
    height: &dyn Fn(f32, f32, usize) -> f32,
) -> Option<usize> {
    let mut smoothed = controls.to_vec();
    fixed::smooth(&mut smoothed, 75_000);
    let points: Vec<_> = smoothed.into_iter().map(fixed::to_world).collect();
    if points.len() < 2 {
        return None;
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut total = 0.0f32;
    lengths.push(0.0);
    for index in 1..points.len() {
        total += span(points[index - 1], points[index]);
        lengths.push(total);
    }
    if total <= 0.0 {
        return None;
    }
    // Geometry follows head_length, independently of shader technique.
    let has_head = arrow.head_length > 0.0;
    let head_length = if has_head {
        arrow.head_length * scale
    } else {
        0.0
    };
    let body_end = total - head_length;
    // A nonzero texture loop replaces the total-length divisor.
    let uv_to_length = if arrow.texture_loop != 0.0 {
        arrow.texture_loop
    } else {
        total
    };
    let base = field.vertices.len() as u32;
    let mut pairs = 0u32;
    let mut head_root = None;
    for (index, &point) in points.iter().enumerate() {
        let along = lengths[index];
        let cut = has_head && index > 0 && along >= body_end;
        let point = if cut {
            point_at(&points, &lengths, body_end)
        } else {
            point
        };
        let (nx, nz) = if cut {
            normal(&[point, points[points.len() - 1]], 0)
        } else {
            normal(&points, index)
        };
        // At the head cut, keep this control's distance for width and U, even
        // though the pair moves back to the head root.
        let half = (arrow.body_width[0]
            + (arrow.body_width[1] - arrow.body_width[0]) * (along / total))
            * scale;
        push_pair(
            field,
            point,
            nx,
            nz,
            half * arrow.body_uv_scale * 2.0,
            along / uv_to_length,
            0.0,
            &|x, z| height(x, z, index),
        );
        pairs += 1;
        if cut {
            head_root = Some((point, (nx, nz), half));
            break;
        }
    }
    let mut bounds = native::bounds(&field.vertices[base as usize..]);
    // A head always cuts the body: the last control lies at `total`.
    if let Some((root, root_normal, half)) = head_root {
        let last = base as usize + (pairs as usize - 1) * 2;
        let edge = [
            field.vertices[last].position,
            field.vertices[last + 1].position,
        ];
        let head_bounds = emit_head(
            field,
            root,
            root_normal,
            edge,
            head_length,
            half,
            points.len(),
            height,
        );
        native::extend_bounds(&mut bounds, head_bounds);
    }
    if pairs < 2 && !has_head {
        return None;
    }
    let (vertices, mut native_strips) =
        native::vertices(&field.vertices[base as usize..], points.len(), has_head);
    let native_base = field.native_vertices.len() as u32;
    for strip in &mut native_strips {
        strip.start += native_base;
        strip.end += native_base;
    }
    field.native_vertices.extend(vertices);
    field.draws.push(OrderDraw {
        air_mode: false,
        arrow_type: arrow.name.clone(),
        texture_choice: 0,
        color,
        total_length: total,
        uv_to_length,
        culling_offset: 0.0,
        declaration_index: arrow.declaration_index,
        // Use the larger body start/end width; the constructor's zero highlight
        // term leaves nothing else in the deselected view.
        width: arrow.body_width[0].max(arrow.body_width[1]),
        label: None,
        native_strips,
        bounds,
        secondary_color: color,
        outer_color: [1.0, 1.0, 1.0, color[3]],
        execution: ExecutionState::default(),
        kind: Default::default(),
    });
    Some(field.draws.len() - 1)
}

/// The head's base uses the body's 3D cross-slope; its tip extends from the
/// root by the authored head length towards the end.
#[allow(clippy::too_many_arguments)]
fn emit_head(
    field: &mut OrderField,
    root: [f32; 2],
    (nx, nz): (f32, f32),
    edge: [[f32; 3]; 2],
    length: f32,
    half: f32,
    count: usize,
    height: &dyn Fn(f32, f32, usize) -> f32,
) -> [[f32; 3]; 2] {
    let tip = [root[0] + nz * length, root[1] - nx * length];
    let base = field.vertices.len() as u32;
    for (point, u, control) in [(root, 0.0, count + 1), (tip, 1.0, count + 2)] {
        push_pair(field, point, nx, nz, half * 2.0, u, 1.0, &|x, z| {
            height(x, z, control)
        });
    }
    // Correct the root cross-slope after updating the bounds.
    let bounds = native::bounds(&field.vertices[base as usize..]);
    let delta = std::array::from_fn::<_, 3, _>(|axis| edge[0][axis] - edge[1][axis]);
    let span = delta.iter().map(|value| value * value).sum::<f32>().sqrt();
    if span > 0.0 {
        for (index, side) in [(0, 1.0), (1, -1.0)] {
            field.vertices[base as usize + index].position = std::array::from_fn(|axis| {
                (edge[0][axis] + edge[1][axis]) * 0.5 + delta[axis] / span * half * side
            });
        }
    }
    bounds
}

#[allow(clippy::too_many_arguments)]
fn push_pair(
    field: &mut OrderField,
    point: [f32; 2],
    nx: f32,
    nz: f32,
    width: f32,
    u: f32,
    is_head: f32,
    height: &dyn Fn(f32, f32) -> f32,
) {
    let half = width * 0.5;
    // The positive perpendicular edge carries v=0; the opposite edge v=1.
    for (side, v) in [(1.0f32, 0.0f32), (-1.0, 1.0)] {
        let x = point[0] + nx * half * side;
        let z = point[1] + nz * half * side;
        let y = height(x, z);
        field.vertices.push(OrderVertex {
            position: [x, y, z],
            uv_head_variant: [u, v, is_head, 0.0],
        });
    }
}

fn normal(points: &[[f32; 2]], index: usize) -> (f32, f32) {
    let p = points[index];
    let previous = points[index.saturating_sub(1)];
    let next = points[(index + 1).min(points.len() - 1)];
    let direction = |a: [f32; 2], b: [f32; 2]| {
        let l = span(a, b);
        if l > 1e-6 {
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        } else {
            [0.0, 0.0]
        }
    };
    let a = direction(previous, p);
    let b = direction(p, next);
    let t = [a[0] + b[0], a[1] + b[1]];
    let l = span([0.0, 0.0], t);
    if l > 1e-6 {
        (-t[1] / l, t[0] / l)
    } else {
        (0.0, 1.0)
    }
}

fn point_at(points: &[[f32; 2]], lengths: &[f32], distance: f32) -> [f32; 2] {
    for index in 1..points.len() {
        if lengths[index] >= distance {
            let run = lengths[index] - lengths[index - 1];
            let t = if run < 1e-6 {
                0.0
            } else {
                (distance - lengths[index - 1]) / run
            };
            return [
                points[index - 1][0] + (points[index][0] - points[index - 1][0]) * t,
                points[index - 1][1] + (points[index][1] - points[index - 1][1]) * t,
            ];
        }
    }
    points[points.len() - 1]
}
