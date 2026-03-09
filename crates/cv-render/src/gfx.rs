use std::num::NonZeroIsize;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::Win32::Foundation::HWND;

const SHADER: &str = include_str!("shader.wgsl");

#[allow(dead_code)] // fields kept alive for GPU resource lifetime
pub struct WgpuState {
    pub device:     wgpu::Device,
    pub queue:      wgpu::Queue,
    surface:        wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    pipeline:       wgpu::RenderPipeline,
    frame_tex:      wgpu::Texture,
    frame_view:     wgpu::TextureView,
    sampler:        wgpu::Sampler,
    uniform_buf:    wgpu::Buffer,
    bgl:            wgpu::BindGroupLayout,
    bind_group:     wgpu::BindGroup,
    pub tex_w:      u32,
    pub tex_h:      u32,
}

impl WgpuState {
    pub fn new(hwnd: HWND, win_w: u32, win_h: u32, tex_w: u32, tex_h: u32) -> Self {
        pollster::block_on(Self::init(hwnd, win_w, win_h, tex_w, tex_h))
    }

    async fn init(hwnd: HWND, win_w: u32, win_h: u32, tex_w: u32, tex_h: u32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..Default::default()
        });

        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                raw_window_handle:  RawWindowHandle::Win32(
                    Win32WindowHandle::new(NonZeroIsize::new(hwnd.0 as isize).unwrap())
                ),
            })
        }
        .expect("create_surface_unsafe failed");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("No DX12 adapter found");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label:              Some("cv-render"),
                    required_features:  wgpu::Features::empty(),
                    required_limits:    wgpu::Limits::default(),
                    memory_hints:       wgpu::MemoryHints::default(),
                },
                None,
            )
            .await
            .expect("request_device failed");

        // Prefer Bgra8Unorm — matches DXGI capture format exactly, no swizzle.
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .find(|&&f| f == wgpu::TextureFormat::Bgra8Unorm)
            .copied()
            .unwrap_or(caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage:                          wgpu::TextureUsages::RENDER_ATTACHMENT,
            format:                         surface_format,
            width:                          win_w.max(1),
            height:                         win_h.max(1),
            present_mode:                   wgpu::PresentMode::Fifo,
            alpha_mode:                     caps.alpha_modes[0],
            view_formats:                   vec![],
            desired_maximum_frame_latency:  2,
        };
        surface.configure(&device, &surface_config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("magnify"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("magnify-bgl"),
            entries: &[
                // binding 0 — Crop uniform (16 bytes)
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                // binding 1 — frame texture
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                // binding 2 — bilinear sampler
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count:      None,
                },
            ],
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:                Some("magnify-pl"),
            bind_group_layouts:   &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("magnify"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         "vs",
                buffers:             &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:      &shader,
                entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState {
                    format:     surface_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive:     wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview:     None,
            cache:         None,
        });

        let (frame_tex, frame_view) = Self::make_frame_texture(&device, tex_w, tex_h);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:           Some("magnify-sampler"),
            address_mode_u:  wgpu::AddressMode::ClampToEdge,
            address_mode_v:  wgpu::AddressMode::ClampToEdge,
            mag_filter:      wgpu::FilterMode::Linear,
            min_filter:      wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("crop-uniform"),
            size:               16,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = Self::make_bind_group(&device, &bgl, &frame_view, &sampler, &uniform_buf);

        Self {
            device, queue, surface, surface_config,
            pipeline, frame_tex, frame_view, sampler,
            uniform_buf, bgl, bind_group,
            tex_w, tex_h,
        }
    }

    fn make_frame_texture(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("frame-tex"),
            size:            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Bgra8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let view = tex.create_view(&Default::default());
        (tex, view)
    }

    fn make_bind_group(
        device:      &wgpu::Device,
        bgl:         &wgpu::BindGroupLayout,
        frame_view:  &wgpu::TextureView,
        sampler:     &wgpu::Sampler,
        uniform_buf: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("magnify-bg"),
            layout:  bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(frame_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Reconfigure the surface after a window resize.
    pub fn resize(&mut self, new_w: u32, new_h: u32) {
        let w = new_w.max(16);
        let h = new_h.max(16);
        self.surface_config.width  = w;
        self.surface_config.height = h;
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Upload a new BGRA8 top-down frame to the GPU texture.
    /// Silently skips if dimensions don't match the texture.
    pub fn upload_frame(&self, data: &[u8], w: u32, h: u32) {
        if w != self.tex_w || h != self.tex_h { return; }
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture:   &self.frame_tex,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset:         0,
                bytes_per_row:  Some(w * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// Write the normalised crop rect [src_x, src_y, src_w, src_h] to the uniform buffer.
    pub fn write_crop(&self, crop: [f32; 4]) {
        let mut bytes = [0u8; 16];
        for (i, &f) in crop.iter().enumerate() {
            bytes[i * 4..(i + 1) * 4].copy_from_slice(&f.to_ne_bytes());
        }
        self.queue.write_buffer(&self.uniform_buf, 0, &bytes);
    }

    /// Execute the render pass and present.
    /// Returns false on surface error (caller should call resize to recover).
    pub fn render(&self) -> bool {
        let output = match self.surface.get_current_texture() {
            Ok(o)  => o,
            Err(e) => {
                eprintln!("[render] surface error: {e:?}");
                return false;
            }
        };

        let view = output.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut rpass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("magnify-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.draw(0..6, 0..1);
        }
        self.queue.submit([enc.finish()]);
        output.present();
        true
    }
}
