use crossbeam_channel::Sender;
use windows::{
    core::*,
    Win32::Graphics::{
        Direct3D11::*,
        Dxgi::{Common::*, *},
    },
};

/// Runs the DXGI Desktop Duplication capture loop on the calling thread.
/// Captured frames (as `ID3D11Texture2D`) are sent over `tx`.
/// The loop runs until `running` is set to false.
pub fn capture_loop(
    device: ID3D11Device,
    tx: Sender<ID3D11Texture2D>,
    running: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    loop {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }

        match try_capture_loop(&device, &tx, &running) {
            Ok(_) => break, // clean shutdown
            Err(e) => {
                eprintln!("[capture] error: {e} — reinitialising duplicator…");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}

fn try_capture_loop(
    device: &ID3D11Device,
    tx: &Sender<ID3D11Texture2D>,
    running: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<()> {
    let duplicator = create_duplicator(device)?;

    loop {
        if !running.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        let result = unsafe {
            duplicator.AcquireNextFrame(1000, &mut frame_info, &mut resource)
        };

        match result {
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(e)
                if e.code() == DXGI_ERROR_ACCESS_LOST
                    || e.code() == DXGI_ERROR_DEVICE_REMOVED =>
            {
                return Err(e)
            }
            Err(e) => return Err(e),
            Ok(_) => {}
        }

        if let Some(resource) = resource {
            let texture: ID3D11Texture2D = resource.cast()?;

            // Make a staging-readable copy so we don't hold the frame resource
            let copy = copy_texture(device, &texture)?;
            let _ = tx.try_send(copy); // drop if channel is full (render is busy)
        }

        unsafe { duplicator.ReleaseFrame()? };
    }
}

fn create_duplicator(device: &ID3D11Device) -> Result<IDXGIOutputDuplication> {
    unsafe {
        let dxgi_device: IDXGIDevice = device.cast()?;
        let adapter = dxgi_device.GetAdapter()?;
        let output = adapter.EnumOutputs(0)?;
        let output1: IDXGIOutput1 = output.cast()?;
        output1.DuplicateOutput(device)
    }
}

/// Copies the desktop texture into a GPU-accessible (non-keyed-mutex) texture
/// that the render thread can use as a shader resource view.
fn copy_texture(device: &ID3D11Device, src: &ID3D11Texture2D) -> Result<ID3D11Texture2D> {
    unsafe {
        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        src.GetDesc(&mut src_desc);

        let desc = D3D11_TEXTURE2D_DESC {
            Width: src_desc.Width,
            Height: src_desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: src_desc.Format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };

        let mut texture = None;
        device.CreateTexture2D(&desc, None, Some(&mut texture))?;
        let texture = texture.unwrap();

        let ctx = device.GetImmediateContext()?;
        ctx.CopyResource(&texture, src);

        Ok(texture)
    }
}
