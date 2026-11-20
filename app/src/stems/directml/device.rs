//! Bind DirectML to an explicit D3D12 queue. The adapter driving the display
//! can differ from the Radeon doing inference, especially in remote sessions.
use anyhow::{ensure, Context, Result};
use ort::{session::builder::SessionBuilder, AsPointer};
use windows::{
    core::Interface,
    Win32::{
        Foundation::LUID,
        Graphics::{
            Direct3D::D3D_FEATURE_LEVEL_11_0,
            Direct3D12::{
                D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT,
                D3D12_COMMAND_QUEUE_DESC,
            },
            Dxgi::{
                CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIFactory4, DXGI_ADAPTER_FLAG_SOFTWARE,
                DXGI_ERROR_NOT_FOUND,
            },
        },
        AI::MachineLearning::DirectML::{DMLCreateDevice, IDMLDevice, DML_CREATE_DEVICE_FLAG_NONE},
    },
};

pub(in crate::stems) enum Binding {
    Radeon(LUID),
    #[cfg(test)]
    Warp,
}

impl Binding {
    fn commands(&self) -> Result<(IDMLDevice, ID3D12CommandQueue)> {
        // Store the adapter identity across threads. The display's enumeration
        // order may change between assessment and loading a checkpoint.
        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory1()? };
        let adapter: IDXGIAdapter = match self {
            Self::Radeon(luid) => unsafe { factory.EnumAdapterByLuid(*luid)? },
            #[cfg(test)]
            Self::Warp => unsafe { factory.EnumWarpAdapter()? },
        };
        // Both COM objects must come from the same D3D12 device. All out parameters
        // are owned COM references, managed by windows-rs.
        let mut d3d: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut d3d) }
            .context("Creating D3D12 device")?;
        let d3d = d3d.context("D3D12 returned no device")?;
        let mut device = None;
        unsafe { DMLCreateDevice(&d3d, DML_CREATE_DEVICE_FLAG_NONE, &mut device) }
            .context("Creating DirectML device")?;
        let desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            ..Default::default()
        };
        let queue = unsafe { d3d.CreateCommandQueue(&desc) }.context("Creating DirectML command queue")?;
        Ok((device.context("DirectML returned no device")?, queue))
    }

    pub(super) fn attach(&self, builder: &mut SessionBuilder) -> ort::Result<()> {
        let (device, queue) = self
            .commands()
            .map_err(|error| ort::Error::new(format!("{error:#}")))?;
        // ORT takes strong references to device and queue on success. Requesting
        // DML1 avoids the legacy provider's extra display-handle lookup.
        unsafe {
            let mut api = std::ptr::null();
            ort::Error::result_from_status((ort::api().GetExecutionProviderApi)(
                c"DML".as_ptr(),
                0,
                &mut api,
            ))?;
            let api = api
                .cast::<ort::sys::OrtDmlApi>()
                .as_ref()
                .ok_or_else(|| ort::Error::new("DirectML provider API unavailable"))?;
            ort::Error::result_from_status((api.SessionOptionsAppendExecutionProvider_DML1)(
                builder.ptr_mut(),
                device.as_raw(),
                queue.as_raw(),
            ))
        }
    }
}

pub(in crate::stems) fn select() -> Result<(Binding, String)> {
    let dll = std::env::current_exe()?.with_file_name("onnxruntime.dll");
    ensure!(
        dll.is_file(),
        "Extract the complete DirectML ZIP with onnxruntime.dll beside recordbox.exe"
    );
    ort::init_from(&dll)
        .context("Loading the packaged ONNX Runtime")?
        .commit();
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut failures = Vec::new();
    for index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(e.into()),
        };
        let desc = unsafe { adapter.GetDesc1()? };
        if desc.VendorId != 0x1002 || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        let binding = Binding::Radeon(desc.AdapterLuid);
        match super::probe(Some(&binding)) {
            Ok(()) => return Ok((binding, format!("DirectML · {name} · DXGI adapter {index}"))),
            Err(error) => failures.push(format!("{name}: {error:#}")),
        }
    }
    ensure!(
        !failures.is_empty(),
        "DirectML found no hardware AMD Radeon adapter"
    );
    anyhow::bail!("DirectML device check failed: {}", failures.join(", "))
}
