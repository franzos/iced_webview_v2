use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use iced::mouse::{self, Interaction};
use iced::wgpu;
use iced::widget::shader;
use iced::{keyboard, Event, Point, Rectangle, Size};

use crate::webview::basic::Action;
use crate::{FramePixels, ImageInfo};

/// Shader-based rendering for servo webview content.
///
/// Uses direct GPU texture updates (`queue.write_texture()`) instead of iced's
/// image Handle cache, avoiding the texture allocation churn and visible
/// flickering that happens during rapid frame updates (e.g. scrolling).
pub struct WebViewShaderProgram<'a> {
    image_info: &'a ImageInfo,
    frame_viewport: Rectangle,
    cursor: Interaction,
    scale_observer: Arc<AtomicU32>,
}

impl<'a> WebViewShaderProgram<'a> {
    pub fn new(
        image_info: &'a ImageInfo,
        frame_viewport: Rectangle,
        cursor: Interaction,
        scale_observer: Arc<AtomicU32>,
    ) -> Self {
        Self {
            image_info,
            frame_viewport,
            cursor,
            scale_observer,
        }
    }
}

#[derive(Default)]
pub struct ShaderState {
    bounds: Size<u32>,
}

pub struct WebViewPrimitive {
    pub(crate) pixels: FramePixels,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Slice of the frame that covers the widget, in frame pixels.
    pub(crate) frame_viewport: Rectangle,
    pub(crate) scale_observer: Arc<AtomicU32>,
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
    /// UV offset and scale selecting the viewport's slice of the texture.
    uniforms: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    texture_size: (u32, u32),
    texture_format: wgpu::TextureFormat,
    /// Generation of the last-uploaded pixel buffer; skips redundant uploads.
    last_uploaded: Option<u64>,
}

impl WebViewPipeline {
    fn recreate_texture(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let (texture, texture_view) =
            create_texture(device, width.max(1), height.max(1), self.texture_format);

        self.bind_group = create_bind_group(
            device,
            &self.bind_group_layout,
            &texture_view,
            &self.sampler,
            &self.uniforms,
        );

        self.texture = texture;
        self.texture_view = texture_view;
        self.texture_size = (width, height);
        self.last_uploaded = None;
    }
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("webview_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniforms.as_entire_binding(),
            },
        ],
    })
}

/// UV offset and scale mapping the widget onto `viewport` within a
/// `width` by `height` texture.
fn uv_transform(viewport: Rectangle, width: u32, height: u32) -> [f32; 4] {
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    [
        viewport.x / w,
        viewport.y / h,
        viewport.width / w,
        viewport.height / h,
    ]
}

// Match the texture format to the surface's color space. The engine produces
// sRGB-encoded bytes: an sRGB surface needs an sRGB texture (decoded on sample,
// re-encoded on write); a non-sRGB (web-colors) surface needs a plain texture
// so the bytes pass through untouched.
fn pick_texture_format(surface: wgpu::TextureFormat) -> wgpu::TextureFormat {
    if surface.is_srgb() {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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

        if (self.width, self.height) != pipeline.texture_size {
            pipeline.recreate_texture(device, self.width, self.height);
        }

        let uv = uv_transform(self.frame_viewport, self.width, self.height);
        let uv_bytes: Vec<u8> = uv.iter().flat_map(|f| f.to_ne_bytes()).collect();
        queue.write_buffer(&pipeline.uniforms, 0, &uv_bytes);

        if pipeline.last_uploaded == Some(self.pixels.generation) {
            return;
        }

        let expected_len = 4 * self.width as usize * self.height as usize;
        if self.pixels.data.len() == expected_len && self.width > 0 && self.height > 0 {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &pipeline.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.pixels.data,
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
            pipeline.last_uploaded = Some(self.pixels.generation);
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

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("webview_uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = create_bind_group(
            device,
            &bind_group_layout,
            &texture_view,
            &sampler,
            &uniforms,
        );

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("webview_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("webview_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
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
            multiview: None,
            cache: None,
        });

        Self {
            texture,
            texture_view,
            sampler,
            uniforms,
            bind_group_layout,
            bind_group,
            render_pipeline,
            texture_size: (1, 1),
            texture_format,
            last_uploaded: None,
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
            frame_viewport: self.frame_viewport,
            scale_observer: self.scale_observer.clone(),
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

struct Uniforms {
    uv_offset: vec2<f32>,
    uv_scale: vec2<f32>,
};

@group(0) @binding(0) var t_texture: texture_2d<f32>;
@group(0) @binding(1) var t_sampler: sampler;
@group(0) @binding(2) var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    // Fullscreen triangle: 3 vertices covering [-1,3] in clip space
    var out: VertexOutput;
    let x = f32(i32(vi & 1u)) * 4.0 - 1.0;
    let y = f32(i32(vi >> 1u)) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    let base = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    out.uv = base * u.uv_scale + u.uv_offset;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_texture, t_sampler, in.uv);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_format_follows_surface_color_space() {
        assert_eq!(
            pick_texture_format(wgpu::TextureFormat::Bgra8UnormSrgb),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            pick_texture_format(wgpu::TextureFormat::Rgba8UnormSrgb),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            pick_texture_format(wgpu::TextureFormat::Bgra8Unorm),
            wgpu::TextureFormat::Rgba8Unorm
        );
    }
}
