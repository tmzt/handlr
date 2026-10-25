// Navy background panel shader for the overview screen.
// Renders signed-distance rounded rectangles with anti-aliasing, borders,
// and subtle physical depth gradients.

struct Panel {
    rect: vec4<f32>,         // [x, y, w, h] in physical pixels
    color: vec4<f32>,        // [r, g, b, a] surface
    border_color: vec4<f32>, // [r, g, b, a] border
    clip: vec4<f32>,         // [clip_x, clip_y, clip_w, clip_h] (0 if unclipped)
    params: vec4<f32>,       // [radius, border_thickness, gradient_strength, _pad]
}

struct Uniforms {
    viewport: vec4<f32>,     // [width, height, scale, panel_count]
    bg_color: vec4<f32>,     // [r, g, b, a] screen background
    panels: array<Panel, 32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.position = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// Signed distance to a rounded box with half-size `half` and radius `r`.
fn sd_rounded_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coord = in.position.xy;
    var color = u.bg_color;
    let count = min(u32(u.viewport.w), 32u);

    for (var i = 0u; i < count; i = i + 1u) {
        let p = u.panels[i];
        if (p.clip.z > 0.0 && p.clip.w > 0.0) {
            if (coord.x < p.clip.x || coord.x > (p.clip.x + p.clip.z) ||
                coord.y < p.clip.y || coord.y > (p.clip.y + p.clip.w)) {
                continue;
            }
        }

        let r = p.rect;
        let half = r.zw * 0.5;
        let center = r.xy + half;
        let local_p = coord - center;
        let radius = max(p.params.x, 0.0);
        let border_w = p.params.y;
        let grad_factor = p.params.z;

        let d = sd_rounded_box(local_p, half, radius);
        if (d <= 1.0) {
            let edge_alpha = 1.0 - smoothstep(-0.5, 0.5, d);

            // Subtle vertical gradient on card surface
            let v_ratio = (local_p.y + half.y) / max(r.w, 1.0);
            let grad = (0.5 - v_ratio) * 0.03 * grad_factor;
            var surf_col = vec4<f32>(p.color.rgb + vec3<f32>(grad), p.color.a);

            if (border_w > 0.1) {
                let border_d = abs(d + border_w * 0.5) - border_w * 0.5;
                let border_alpha = 1.0 - smoothstep(-0.5, 0.5, border_d);
                surf_col = mix(surf_col, p.border_color, border_alpha);
            }

            color = mix(color, surf_col, edge_alpha * surf_col.a);
        }
    }

    return color;
}
