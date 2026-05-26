use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use iced::mouse::{self, Interaction};
use iced::wgpu;
use iced::widget::shader;
use iced::{keyboard, Event, Point, Rectangle, Size};

use crate::webview::basic::Action;
use crate::ImageInfo;

#[cfg(feature = "blitz")]
use crate::engines::GpuFrameHandle;

/// Shader-based rendering for servo webview content.
///
/// Uses direct GPU texture updates (`queue.write_texture()`) instead of iced's
/// image Handle cache, avoiding the texture allocation churn and visible
/// flickering that happens during rapid frame updates (e.g. scrolling).
pub struct WebViewShaderProgram<'a> {
    image_info: &'a ImageInfo,
    cursor: Interaction,
    scale_observer: Arc<AtomicU32>,
    #[cfg(feature = "blitz")]
    gpu_frame: Option<GpuFrameHandle>,
}

impl<'a> WebViewShaderProgram<'a> {
    pub fn new(
        image_info: &'a ImageInfo,
        cursor: Interaction,
        scale_observer: Arc<AtomicU32>,
    ) -> Self {
        Self {
            image_info,
            cursor,
            scale_observer,
            #[cfg(feature = "blitz")]
            gpu_frame: None,
        }
    }

    #[cfg(feature = "blitz")]
    pub fn with_gpu_frame(mut self, handle: Option<GpuFrameHandle>) -> Self {
        self.gpu_frame = handle;
        self
    }
}

#[derive(Default)]
pub struct ShaderState {
    bounds: Size<u32>,
}

pub struct WebViewPrimitive {
    pub(crate) pixels: Arc<Vec<u8>>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_observer: Arc<AtomicU32>,
    #[cfg(feature = "blitz")]
    pub(crate) gpu_frame: Option<GpuFrameHandle>,
}

impl std::fmt::Debug for WebViewPrimitive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebViewPrimitive")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

pub struct WebViewPipeline {
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    texture_size: (u32, u32),
    texture_format: wgpu::TextureFormat,
    /// Vello renderer bound to iced's wgpu device. `Mutex` is needed because
    /// iced's `Pipeline` trait requires `Sync` and `vello::Renderer` contains
    /// internal `RefCell`s. Lock contention is nil — only `prepare` touches it.
    #[cfg(feature = "blitz")]
    vello: std::sync::Mutex<Option<vello::Renderer>>,
}

impl WebViewPipeline {
    fn recreate_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (texture, texture_view) =
            create_texture(device, width.max(1), height.max(1), self.texture_format);

        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("webview_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_size = (width, height);
    }
}

// Blitz on the GPU path writes via vello (a compute pipeline), which requires
// STORAGE_BINDING and rules out sRGB-suffixed formats. We always use linear
// Rgba8Unorm: vello produces linear RGBA, sRGB surfaces re-encode on present.
// The CPU path (legacy engines) loses its sRGB matching here, acceptable on
// this PoC branch.
fn pick_texture_format(_surface: wgpu::TextureFormat) -> wgpu::TextureFormat {
    wgpu::TextureFormat::Rgba8Unorm
}

fn create_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("webview_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// -- Primitive ----------------------------------------------------------------

impl shader::Primitive for WebViewPrimitive {
    type Pipeline = WebViewPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        self.scale_observer
            .store(viewport.scale_factor().to_bits(), Ordering::Relaxed);

        #[cfg(feature = "blitz")]
        if let Some(handle) = &self.gpu_frame {
            if let Some(frame) = handle.lock().unwrap().take() {
                if frame.width > 0 && frame.height > 0 {
                    if (frame.width, frame.height) != pipeline.texture_size {
                        pipeline.recreate_texture(device, frame.width, frame.height);
                    }
                    let mut guard = pipeline.vello.lock().unwrap();
                    let renderer = guard.get_or_insert_with(|| {
                        vello::Renderer::new(
                            device,
                            vello::RendererOptions {
                                use_cpu: false,
                                num_init_threads: std::num::NonZeroUsize::new(1),
                                antialiasing_support: vello::AaSupport::area_only(),
                                pipeline_cache: None,
                            },
                        )
                        .expect("vello renderer init")
                    });
                    renderer
                        .render_to_texture(
                            device,
                            queue,
                            &frame.scene,
                            &pipeline.texture_view,
                            &vello::RenderParams {
                                base_color: vello::peniko::Color::TRANSPARENT,
                                width: frame.width,
                                height: frame.height,
                                antialiasing_method: vello::AaConfig::Area,
                            },
                        )
                        .expect("vello render_to_texture");
                }
            }
            return;
        }

        if (self.width, self.height) != pipeline.texture_size {
            pipeline.recreate_texture(device, self.width, self.height);
        }

        let expected_len = 4 * self.width as usize * self.height as usize;
        if self.pixels.len() == expected_len && self.width > 0 && self.height > 0 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pipeline.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * self.width),
                    rows_per_image: Some(self.height),
                },
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    fn draw(&self, pipeline: &Self::Pipeline, render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        if self.width == 0 || self.height == 0 {
            return true;
        }
        render_pass.set_pipeline(&pipeline.render_pipeline);
        render_pass.set_bind_group(0, &pipeline.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
        true
    }
}

// -- Pipeline -----------------------------------------------------------------

impl shader::Pipeline for WebViewPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let texture_format = pick_texture_format(format);
        let (texture, texture_view) = create_texture(device, 1, 1, texture_format);

        // Buffer is rasterized at physical resolution, so it maps ~1:1 to the
        // surface — nearest-neighbor keeps text crisp instead of resampling.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("webview_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("webview_bgl"),
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
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("webview_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("webview_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("webview_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("webview_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            texture,
            texture_view,
            sampler,
            bind_group_layout,
            bind_group,
            render_pipeline,
            texture_size: (1, 1),
            texture_format,
            #[cfg(feature = "blitz")]
            vello: std::sync::Mutex::new(None),
        }
    }
}

// -- Program ------------------------------------------------------------------

impl<'a> shader::Program<Action> for WebViewShaderProgram<'a> {
    type State = ShaderState;
    type Primitive = WebViewPrimitive;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Action>> {
        let size = Size::new(bounds.width.round() as u32, bounds.height.round() as u32);
        if state.bounds != size {
            state.bounds = size;
            return Some(shader::Action::publish(Action::Resize(size)));
        }

        match event {
            Event::Keyboard(event) => {
                if let keyboard::Event::KeyPressed {
                    key: keyboard::Key::Character(c),
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.command() && c.as_str() == "c" {
                        return Some(shader::Action::publish(Action::CopySelection));
                    }
                }
                Some(shader::Action::publish(Action::SendKeyboardEvent(
                    event.clone(),
                )))
            }
            Event::Mouse(event) => {
                if let Some(point) = cursor.position_in(bounds) {
                    Some(shader::Action::publish(Action::SendMouseEvent(
                        *event, point,
                    )))
                } else if matches!(event, mouse::Event::CursorLeft) {
                    Some(shader::Action::publish(Action::SendMouseEvent(
                        *event,
                        Point::ORIGIN,
                    )))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        WebViewPrimitive {
            pixels: self.image_info.pixels(),
            width: self.image_info.image_width(),
            height: self.image_info.image_height(),
            scale_observer: self.scale_observer.clone(),
            #[cfg(feature = "blitz")]
            gpu_frame: self.gpu_frame.clone(),
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Interaction {
        self.cursor
    }
}

// -- WGSL Shader --------------------------------------------------------------

const SHADER_SOURCE: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    // Fullscreen triangle: 3 vertices covering [-1,3] in clip space
    var out: VertexOutput;
    let x = f32(i32(vi & 1u)) * 4.0 - 1.0;
    let y = f32(i32(vi >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var t_texture: texture_2d<f32>;
@group(0) @binding(1) var t_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_texture, t_sampler, in.uv);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_format_is_linear_for_vello_storage() {
        for surface in [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
        ] {
            assert_eq!(
                pick_texture_format(surface),
                wgpu::TextureFormat::Rgba8Unorm
            );
        }
    }
}
