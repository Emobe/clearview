use crossbeam_channel::Receiver;
use windows::{
    core::*,
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D::*,
            Direct3D::Fxc::D3DCompile,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
    },
};

use crate::state::SharedState;

#[repr(C)]
struct Constants {
    center: [f32; 2],
    zoom: f32,
    _pad: f32,
}

pub fn create_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let feature_levels = [D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    let mut ctx = None;
    let mut level = D3D_FEATURE_LEVEL::default();

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut level),
            Some(&mut ctx),
        )?;
    }

    Ok((device.unwrap(), ctx.unwrap()))
}

/// Render loop — runs on a dedicated thread.
pub fn render_loop(
    hwnd: HWND,
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    rx: Receiver<ID3D11Texture2D>,
    state: SharedState,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if let Err(e) = try_render_loop(hwnd, &device, &ctx, &rx, &state, &running) {
        eprintln!("[renderer] fatal: {e}");
    }
}

fn try_render_loop(
    hwnd: HWND,
    device: &ID3D11Device,
    ctx: &ID3D11DeviceContext,
    rx: &Receiver<ID3D11Texture2D>,
    state: &SharedState,
    running: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let (swap_chain, rtv) = create_swap_chain(hwnd, device)?;
    let (vs, ps, layout) = compile_shaders(device)?;
    let sampler = create_sampler(device)?;
    let cb = create_constant_buffer(device)?;

    let mut latest_texture: Option<ID3D11Texture2D> = None;
    let mut latest_srv: Option<ID3D11ShaderResourceView> = None;

    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) } as f32;
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) } as f32;

    let mut last_frame = std::time::Instant::now();

    while running.load(std::sync::atomic::Ordering::Relaxed) {
        let delta = last_frame.elapsed().as_secs_f32();
        last_frame = std::time::Instant::now();

        // Drain the channel; keep only the freshest frame.
        while let Ok(tex) = rx.try_recv() {
            latest_texture = Some(tex);
            latest_srv = None; // invalidate cached SRV
        }

        let (zoom, enabled, smooth_speed) = {
            let s = state.read();
            (s.zoom, s.enabled, s.smooth_speed)
        };

        let (mx, my) = cursor_normalised(screen_w, screen_h);

        // Frame-rate-independent lerp toward cursor
        let alpha = 1.0_f32 - (1.0 - smooth_speed).powf(delta * 60.0);
        {
            let mut s = state.write();
            s.viewport_center[0] += (mx - s.viewport_center[0]) * alpha;
            s.viewport_center[1] += (my - s.viewport_center[1]) * alpha;
        }

        let center = state.read().viewport_center;

        if !enabled {
            unsafe {
                ctx.ClearRenderTargetView(&rtv, &[0.0_f32, 0.0, 0.0, 1.0]);
                swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
            continue;
        }

        // Build SRV for the latest captured texture if needed.
        if let Some(tex) = &latest_texture {
            if latest_srv.is_none() {
                latest_srv = Some(create_srv(device, tex)?);
            }
        }

        update_constant_buffer(
            ctx,
            &cb,
            &Constants {
                center,
                zoom,
                _pad: 0.0,
            },
        );

        unsafe {
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);

            let vp = D3D11_VIEWPORT {
                Width: screen_w,
                Height: screen_h,
                MaxDepth: 1.0,
                ..Default::default()
            };
            ctx.RSSetViewports(Some(&[vp]));

            ctx.VSSetShader(&vs, None);
            ctx.PSSetShader(&ps, None);
            ctx.IASetInputLayout(&layout);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            ctx.IASetVertexBuffers(0, 1, None, None, None);

            ctx.PSSetConstantBuffers(0, Some(&[Some(cb.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(sampler.clone())]));

            if let Some(srv) = &latest_srv {
                ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            }

            ctx.Draw(4, 0);
            swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
        }
    }

    Ok(())
}

fn create_swap_chain(
    hwnd: HWND,
    device: &ID3D11Device,
) -> Result<(IDXGISwapChain1, ID3D11RenderTargetView)> {
    unsafe {
        let dxgi_device: IDXGIDevice = device.cast()?;
        let adapter = dxgi_device.GetAdapter()?;
        let factory: IDXGIFactory2 = adapter.GetParent()?;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            ..Default::default()
        };

        let swap_chain = factory.CreateSwapChainForHwnd(device, hwnd, &desc, None, None)?;

        let back_buffer: ID3D11Texture2D = swap_chain.GetBuffer(0)?;
        let mut rtv = None;
        device.CreateRenderTargetView(&back_buffer, None, Some(&mut rtv))?;

        Ok((swap_chain, rtv.unwrap()))
    }
}

fn compile_shaders(
    device: &ID3D11Device,
) -> Result<(ID3D11VertexShader, ID3D11PixelShader, ID3D11InputLayout)> {
    let src = include_str!("../shaders/magnify.hlsl");
    let src_bytes = src.as_bytes();

    unsafe {
        let mut vs_blob: Option<ID3DBlob> = None;
        let mut errors: Option<ID3DBlob> = None;
        D3DCompile(
            src_bytes.as_ptr() as *const _,
            src_bytes.len(),
            None,
            None,
            None,
            s!("vs_main"),
            s!("vs_5_0"),
            0,
            0,
            &mut vs_blob,
            Some(&mut errors),
        )
        .inspect_err(|_| log_shader_errors(&errors))?;
        let vs_blob: ID3DBlob = vs_blob.unwrap();

        let mut ps_blob: Option<ID3DBlob> = None;
        D3DCompile(
            src_bytes.as_ptr() as *const _,
            src_bytes.len(),
            None,
            None,
            None,
            s!("ps_main"),
            s!("ps_5_0"),
            0,
            0,
            &mut ps_blob,
            Some(&mut errors),
        )
        .inspect_err(|_| log_shader_errors(&errors))?;
        let ps_blob: ID3DBlob = ps_blob.unwrap();

        let vs_bytes = std::slice::from_raw_parts(
            vs_blob.GetBufferPointer() as *const u8,
            vs_blob.GetBufferSize(),
        );
        let ps_bytes = std::slice::from_raw_parts(
            ps_blob.GetBufferPointer() as *const u8,
            ps_blob.GetBufferSize(),
        );

        let mut vs = None;
        device.CreateVertexShader(vs_bytes, None, Some(&mut vs))?;

        let mut ps = None;
        device.CreatePixelShader(ps_bytes, None, Some(&mut ps))?;

        // No vertex buffer — positions are generated in vs_main via SV_VertexID.
        let mut layout = None;
        device.CreateInputLayout(&[], vs_bytes, Some(&mut layout))?;

        Ok((vs.unwrap(), ps.unwrap(), layout.unwrap()))
    }
}

fn log_shader_errors(blob: &Option<ID3DBlob>) {
    if let Some(b) = blob {
        unsafe {
            let ptr = b.GetBufferPointer() as *const u8;
            let len = b.GetBufferSize();
            if let Ok(s) = std::str::from_utf8(std::slice::from_raw_parts(ptr, len)) {
                eprintln!("[shader] {s}");
            }
        }
    }
}

fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        MaxLOD: f32::MAX,
        ..Default::default()
    };
    let mut sampler = None;
    unsafe { device.CreateSamplerState(&desc, Some(&mut sampler))? };
    Ok(sampler.unwrap())
}

fn create_constant_buffer(device: &ID3D11Device) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<Constants>() as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        ..Default::default()
    };
    let mut buf = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf))? };
    Ok(buf.unwrap())
}

fn update_constant_buffer(ctx: &ID3D11DeviceContext, buf: &ID3D11Buffer, data: &Constants) {
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        if ctx
            .Map(buf, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))
            .is_ok()
        {
            std::ptr::copy_nonoverlapping(
                data as *const Constants,
                mapped.pData as *mut Constants,
                1,
            );
            ctx.Unmap(buf, 0);
        }
    }
}

fn create_srv(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };

    let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: desc.Format,
        ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_SRV {
                MostDetailedMip: 0,
                MipLevels: 1,
            },
        },
    };

    let mut srv = None;
    unsafe { device.CreateShaderResourceView(texture, Some(&srv_desc), Some(&mut srv))? };
    Ok(srv.unwrap())
}

fn cursor_normalised(screen_w: f32, screen_h: f32) -> (f32, f32) {
    unsafe {
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt);
        (
            (pt.x as f32 / screen_w).clamp(0.0, 1.0),
            (pt.y as f32 / screen_h).clamp(0.0, 1.0),
        )
    }
}
