// The preview's image (docs/design/20261028_05_BINARY_FILES_PLAN.md,
// decision 10.3): one texture drawn into a rect of the target, clipped by
// the pass's scissor to the preview's body.

struct Uniforms {
    // [x, y, w, h] of the image, in target pixels.
    rect: vec4<f32>,
    // [target width, target height, 0, 0].
    target: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // A strip of four corners.
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    let px = u.rect.xy + corner * u.rect.zw;
    let ndc = vec2<f32>(px.x / u.target.x * 2.0 - 1.0, 1.0 - px.y / u.target.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
