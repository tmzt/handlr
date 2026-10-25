//! Custom GPU panel shader pipeline for the overview screen
//! (docs/design/20261025_03_NAVY_OVERVIEW_THEME_PLAN.md).
//!
//! Renders anti-aliased signed distance field rounded panels with subtle
//! physical lighting gradients and crisp borders directly on the GPU,
//! composited under `libmsdf`'s glyph and UI pass.

use libmorph::wgpu;

/// A single panel instance sent to the shader.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelInstance {
    /// Bounding rectangle `[x, y, width, height]` in physical pixels.
    pub rect: [f32; 4],
    /// Surface fill color `[r, g, b, a]`.
    pub color: [f32; 4],
    /// Border stroke color `[r, g, b, a]`.
    pub border_color: [f32; 4],
    /// Optional clipping rectangle `[clip_x, clip_y, clip_w, clip_h]`.
    pub clip: [f32; 4],
    /// `[radius, border_thickness, gradient_strength, 0.0]`.
    pub params: [f32; 4],
}

pub const MAX_PANELS: usize = 32;
pub const UNIFORM_FLOATS: usize = 4 + 4 + MAX_PANELS * 20;

/// Packs viewport, clear color, and up to `MAX_PANELS` panel instances into
/// a contiguous float buffer matching `Uniforms` in `panel.wgsl`.
pub fn pack_uniforms(
    width: f32,
    height: f32,
    scale: f32,
    bg: [f32; 4],
    panels: &[PanelInstance],
) -> [f32; UNIFORM_FLOATS] {
    let mut u = [0.0f32; UNIFORM_FLOATS];
    let count = panels.len().min(MAX_PANELS);
    u[0] = width;
    u[1] = height;
    u[2] = scale;
    u[3] = count as f32;
    u[4..8].copy_from_slice(&bg);
    for (i, p) in panels.iter().take(MAX_PANELS).enumerate() {
        let base = 8 + i * 20;
        u[base..base + 4].copy_from_slice(&p.rect);
        u[base + 4..base + 8].copy_from_slice(&p.color);
        u[base + 8..base + 12].copy_from_slice(&p.border_color);
        u[base + 12..base + 16].copy_from_slice(&p.clip);
        u[base + 16..base + 20].copy_from_slice(&p.params);
    }
    u
}

pub struct PanelShader {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    format: wgpu::TextureFormat,
}

impl PanelShader {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("companion panel shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("panel.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("companion panel bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("companion panel pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("companion panel pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("companion panel uniforms"),
            size: (UNIFORM_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("companion panel bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buf.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            uniform_buf,
            bind_group,
            format,
        }
    }

    /// Renders the background and panel instances into `view`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        scale: f32,
        bg: [f32; 4],
        panels: &[PanelInstance],
    ) {
        if self.format != format {
            *self = Self::new(device, format);
        }
        let u = pack_uniforms(width as f32, height as f32, scale, bg, panels);
        let bytes: Vec<u8> = u.iter().flat_map(|f| f.to_ne_bytes()).collect();
        queue.write_buffer(&self.uniform_buf, 0, &bytes);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("companion panel pass"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("companion panel pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0] as f64,
                            g: bg[1] as f64,
                            b: bg[2] as f64,
                            a: bg[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniforms_pack_correctly() {
        let p = PanelInstance {
            rect: [10.0, 20.0, 300.0, 200.0],
            color: [0.1, 0.2, 0.3, 1.0],
            border_color: [0.5, 0.6, 0.7, 1.0],
            clip: [0.0, 0.0, 500.0, 500.0],
            params: [12.0, 2.0, 1.0, 0.0],
        };
        let u = pack_uniforms(1080.0, 2340.0, 2.0, [0.05, 0.05, 0.1, 1.0], &[p]);
        assert_eq!(u[0], 1080.0);
        assert_eq!(u[1], 2340.0);
        assert_eq!(u[2], 2.0);
        assert_eq!(u[3], 1.0); // 1 panel
        assert_eq!(&u[4..8], &[0.05, 0.05, 0.1, 1.0]); // bg
        assert_eq!(&u[8..12], &[10.0, 20.0, 300.0, 200.0]); // rect
        assert_eq!(&u[12..16], &[0.1, 0.2, 0.3, 1.0]); // color
        assert_eq!(&u[16..20], &[0.5, 0.6, 0.7, 1.0]); // border_color
        assert_eq!(&u[20..24], &[0.0, 0.0, 500.0, 500.0]); // clip
        assert_eq!(&u[24..28], &[12.0, 2.0, 1.0, 0.0]); // params
    }
}
