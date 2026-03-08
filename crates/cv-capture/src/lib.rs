use cv_core::Frame;
use windows::{
    core::Interface,
    Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_HARDWARE,
        Direct3D11::{
            D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
            D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
            D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        },
        Dxgi::{
            Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, DXGI_ERROR_WAIT_TIMEOUT,
            DXGI_OUTDUPL_FRAME_INFO,
        },
    },
};

struct D3dCtx {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
}

pub struct Capturer {
    ctx: D3dCtx,
    duplication: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl Capturer {
    pub fn new() -> windows::core::Result<Self> {
        let ctx = create_device()?;
        let (duplication, width, height) = create_duplication(&ctx.device)?;
        let staging = create_staging(&ctx.device, width, height)?;
        Ok(Self { ctx, duplication, staging, width, height })
    }

    /// Returns `None` on timeout (no new frame yet), `Err` on device loss.
    pub fn next_frame(&mut self, timeout_ms: u32) -> windows::core::Result<Option<Frame>> {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;

            match self.duplication.AcquireNextFrame(timeout_ms, &mut info, &mut resource) {
                Ok(_) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(e) => return Err(e),
            }

            let texture: ID3D11Texture2D = resource.unwrap().cast()?;
            self.ctx.context.CopyResource(&self.staging, &texture);
            let data = read_staging(&self.ctx.context, &self.staging, self.width, self.height)?;
            self.duplication.ReleaseFrame()?;

            Ok(Some(Frame { width: self.width, height: self.height, data }))
        }
    }

    pub fn reconnect(&mut self) -> windows::core::Result<()> {
        *self = Self::new()?;
        Ok(())
    }
}

fn create_device() -> windows::core::Result<D3dCtx> {
    unsafe {
        let mut device = None;
        let mut context = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        Ok(D3dCtx { device: device.unwrap(), context: context.unwrap() })
    }
}

fn create_duplication(
    device: &ID3D11Device,
) -> windows::core::Result<(IDXGIOutputDuplication, u32, u32)> {
    unsafe {
        let dxgi: IDXGIDevice = device.cast()?;
        let adapter = dxgi.GetAdapter()?;
        let output = adapter.EnumOutputs(0)?;
        let output1: IDXGIOutput1 = output.cast()?;
        let dup = output1.DuplicateOutput(device)?;
        let desc = dup.GetDesc();
        Ok((dup, desc.ModeDesc.Width, desc.ModeDesc.Height))
    }
}

fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> windows::core::Result<ID3D11Texture2D> {
    unsafe {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut tex = None;
        device.CreateTexture2D(&desc, None, Some(&mut tex))?;
        Ok(tex.unwrap())
    }
}

fn read_staging(
    ctx: &ID3D11DeviceContext,
    staging: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

        let row_pitch = mapped.RowPitch as usize;
        let w = width as usize;
        let h = height as usize;
        let mut data = vec![0u8; w * h * 4];
        let src = std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * h);

        for row in 0..h {
            let src_row = &src[row * row_pitch..row * row_pitch + w * 4];
            let dst_row = &mut data[row * w * 4..(row + 1) * w * 4];
            dst_row.copy_from_slice(src_row);
        }

        ctx.Unmap(staging, 0);
        Ok(data)
    }
}
