//! Symphony: live backdrop blur.
//!
//! egui draws a frame in one render pass, so nothing can read what is
//! already drawn behind a window. A [`BackdropBlur`] paint callback marks a
//! point in the frame: the painter ends the pass there, copies the frame so
//! far, blurs it on the GPU (downscale to a quarter, then a horizontal and a
//! vertical Gaussian pass), and carries on. The blurred image is an ordinary
//! egui texture ([`backdrop_texture_id`]), so the app draws it into any
//! shape (a rounded window) right after the marker.

use std::sync::Mutex;

/// The marker: put it (as an `epaint::PaintCallback`) where the blur should
/// be taken, then draw the blurred texture.
pub struct BackdropBlur;

static TEXTURE: Mutex<Option<epaint::TextureId>> = Mutex::new(None);

/// The blurred backdrop as an egui texture, covering the whole screen, once
/// a frame has made one.
pub fn backdrop_texture_id() -> Option<epaint::TextureId> {
    *TEXTURE.lock().ok()?
}

struct Pass {
    target: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

pub(crate) struct Blur {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    copy: Option<wgpu::Texture>,
    /// Downscale, horizontal, vertical; the last pass's target is the
    /// result shown by egui.
    passes: Vec<Pass>,
    texture_id: Option<epaint::TextureId>,
}

impl Blur {
    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("symphony_backdrop_blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("backdrop.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("symphony_backdrop_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("symphony_backdrop_pipeline_layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("symphony_backdrop_pipeline"),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("symphony_backdrop_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            format,
            size: [0, 0],
            copy: None,
            passes: Vec::new(),
            texture_id: None,
        }
    }

    /// Make the textures for a `size` frame and register the result with
    /// egui (once, then updated on resize).
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        renderer: &mut crate::Renderer,
        size: [u32; 2],
    ) {
        if size == self.size && self.copy.is_some() {
            return;
        }
        let size = [size[0].max(4), size[1].max(4)];
        self.size = size;
        let texture = |label: &str, w: u32, h: u32, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage,
                view_formats: &[],
            })
        };
        let copy = texture(
            "symphony_backdrop_copy",
            size[0],
            size[1],
            wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let (qw, qh) = ((size[0] / 4).max(1), (size[1] / 4).max(1));
        let use_rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let targets = [
            texture("symphony_backdrop_down", qw, qh, use_rt),
            texture("symphony_backdrop_h", qw, qh, use_rt),
            texture("symphony_backdrop_v", qw, qh, use_rt),
        ];
        let texel = [1.0 / qw as f32, 1.0 / qh as f32];
        let dirs = [[0.0, 0.0], [texel[0], 0.0], [0.0, texel[1]]];
        let mut passes = Vec::new();
        let mut source_view = copy.create_view(&wgpu::TextureViewDescriptor::default());
        for (target, dir) in targets.into_iter().zip(dirs) {
            use wgpu::util::DeviceExt as _;
            let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("symphony_backdrop_params"),
                contents: bytemuck::cast_slice(&[dir[0], dir[1], 0.0f32, 0.0f32]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("symphony_backdrop_bind_group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            source_view = view.clone();
            passes.push(Pass {
                target,
                view,
                bind_group,
            });
        }
        let result = &passes[passes.len() - 1].view;
        match self.texture_id {
            Some(id) => renderer.update_egui_texture_from_wgpu_texture(
                device,
                result,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture_id =
                    Some(renderer.register_native_texture(device, result, wgpu::FilterMode::Linear));
            }
        }
        if let Ok(mut t) = TEXTURE.lock() {
            *t = self.texture_id;
        }
        self.copy = Some(copy);
        self.passes = passes;
    }

    /// Copy `frame` (what's drawn so far) and blur it into the result.
    pub(crate) fn run(&self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture) {
        let Some(copy) = &self.copy else { return };
        let size = frame.size();
        // A surface that can't be copied (some platforms) just isn't blurred.
        if [size.width, size.height] != self.size
            || frame.format() != self.format
            || !frame.usage().contains(wgpu::TextureUsages::COPY_SRC)
        {
            return;
        }
        encoder.copy_texture_to_texture(
            frame.as_image_copy(),
            copy.as_image_copy(),
            size,
        );
        for pass in &self.passes {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("symphony_backdrop_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pass.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &pass.bind_group, &[]);
            rp.draw(0..3, 0..1);
            let _ = &pass.target;
        }
    }
}
