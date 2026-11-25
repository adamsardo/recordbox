//! Vulkan compute used by the Linux HTDemucs transformer projections.
use super::backend::{backend_hold, hold_vulkan};
use anyhow::{anyhow, bail, ensure, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TILE: usize = 16;
const MAX_BUFFER_BYTES: usize = 32 * 1024 * 1024;
const MAX_INNER: usize = 4096;
const MAX_TILE_FMAS: usize = 32 * 1024 * 1024;
const MAX_ROWS_PER_DISPATCH: usize = 256;
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

fn adapters() -> Vec<wgpu::Adapter> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    instance.enumerate_adapters(wgpu::Backends::VULKAN)
}

fn adapter_label(info: &wgpu::AdapterInfo) -> String {
    let driver = [info.driver.trim(), info.driver_info.trim()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if driver.is_empty() {
        info.name.clone()
    } else {
        format!("{} ({driver})", info.name)
    }
}

fn eligible(adapter: &wgpu::Adapter, software: bool) -> bool {
    let kind = adapter.get_info().device_type;
    let matching_type = if software {
        kind == wgpu::DeviceType::Cpu
    } else {
        matches!(
            kind,
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
        )
    };
    matching_type
        && adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
}

// Even passive enumeration and shader compilation can enter the vendor driver.
fn initialize<T: Send + 'static>(
    timeout: Duration,
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    ready()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("stems-vulkan-init".into())
        .spawn(move || {
            let _ = sender.send(ready().and_then(|()| operation()));
        })?;
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(err) => Err(terminal(format!(
            "Vulkan device check did not finish: {err}"
        ))),
    }
}

pub fn adapter_names() -> Result<Vec<String>> {
    initialize(Duration::from_secs(5), || {
        Ok(adapters()
            .into_iter()
            .filter(|adapter| eligible(adapter, false))
            .map(|adapter| adapter_label(&adapter.get_info()))
            .collect())
    })
}

fn ready() -> Result<()> {
    match backend_hold() {
        Some(reason) => bail!("{reason}"),
        None => Ok(()),
    }
}

fn terminal(reason: String) -> anyhow::Error {
    hold_vulkan(reason.clone());
    anyhow!(reason)
}

fn check_readback(values: &[f32], adapter: &str) -> Result<()> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(terminal(format!(
            "{adapter} returned a non-finite Vulkan projection"
        )));
    }
    Ok(())
}

/// Each buffer is independently bounded (two inputs, one output and its readback).
/// Sizes in elements also fit the shader's unsigned 32-bit indexing.
fn matrix_bytes(a: usize, b: usize, limit: usize) -> Result<usize> {
    let bytes = a
        .checked_mul(b)
        .and_then(|n| n.checked_mul(size_of::<f32>()));
    match bytes {
        Some(n) if n > 0 && n <= limit => Ok(n),
        _ => bail!("Vulkan matrix {a} × {b} exceeds the {limit}-byte buffer limit or is empty"),
    }
}

fn dispatch_rows(inner: usize, cols: usize, group_limit: usize) -> Result<usize> {
    ensure!(
        (1..=MAX_INNER).contains(&inner),
        "Unsupported Vulkan projection width {inner}"
    );
    let col_groups = cols.div_ceil(TILE);
    ensure!(
        col_groups > 0 && col_groups <= group_limit,
        "Unsupported Vulkan output width {cols}"
    );
    // Account for padded lanes and the last partial inner tile, not only useful outputs.
    let tile_row_fmas = TILE * (col_groups * TILE) * (inner.div_ceil(TILE) * TILE);
    let tile_rows = (MAX_TILE_FMAS / tile_row_fmas)
        .min(MAX_ROWS_PER_DISPATCH / TILE)
        .min(group_limit);
    ensure!(
        tile_rows > 0,
        "Vulkan projection exceeds the per-submission work limit"
    );
    Ok(tile_rows * TILE)
}

pub struct Engine {
    label: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    run: Mutex<()>,
    completed: AtomicU64,
}

impl Engine {
    pub fn new() -> Result<Self> {
        initialize(STARTUP_TIMEOUT, || Self::select(false))
    }

    #[cfg(test)]
    pub fn new_for_test() -> Result<Self> {
        initialize(STARTUP_TIMEOUT, || Self::select(true))
    }

    fn select(software: bool) -> Result<Self> {
        let mut reasons = Vec::new();
        for adapter in adapters() {
            ready()?;
            let info = adapter.get_info();
            if !eligible(&adapter, software) {
                continue;
            }
            match Self::open(adapter, adapter_label(&info)) {
                Ok(engine) => return Ok(engine),
                Err(err) => reasons.push(format!("{}: {err:#}", info.name)),
            }
        }
        ready()?;
        if reasons.is_empty() {
            bail!(
                "No eligible {} Vulkan compute adapter",
                if software { "software" } else { "physical" }
            );
        }
        bail!("Vulkan device check failed: {}", reasons.join(", "))
    }

    fn open(adapter: wgpu::Adapter, label: String) -> Result<Self> {
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("STEMS Vulkan"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
            },
            None,
        ))?;
        device.on_uncaptured_error(Box::new(|error| {
            hold_vulkan(format!("Vulkan compute error: {error}"));
        }));
        device.set_device_lost_callback(|reason, message| {
            if reason == wgpu::DeviceLostReason::Unknown {
                hold_vulkan(format!("Vulkan device lost: {message}"));
            }
        });
        ready()?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HTDemucs linear projection"),
            source: wgpu::ShaderSource::Wgsl(include_str!("linear.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("HTDemucs linear projection"),
            layout: None,
            module: &shader,
            entry_point: Some("linear"),
            compilation_options: Default::default(),
            cache: None,
        });
        ready()?;
        let layout = pipeline.get_bind_group_layout(0);
        let engine = Self {
            label,
            device,
            queue,
            layout,
            pipeline,
            run: Mutex::new(()),
            completed: AtomicU64::new(0),
        };
        let probe = engine.linear(
            &[1., 2., 3., 4., 5., 6.],
            &[1., 0., -1., 0., 2., 1.],
            2,
            3,
            2,
            &mut || Ok(()),
        )?;
        if probe != [-2., 7., -2., 16.] {
            return Err(terminal(format!(
                "{} produced incorrect Vulkan projection self-test results",
                engine.label
            )));
        }
        engine.completed.store(0, Ordering::Relaxed);
        Ok(engine)
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn completed_matmuls(&self) -> u64 {
        self.completed.load(Ordering::Relaxed)
    }

    // Poll without asking the driver to block on an unsignaled fence.
    fn wait<T>(&self, receiver: mpsc::Receiver<T>) -> Result<T> {
        let deadline = Instant::now() + DISPATCH_TIMEOUT;
        loop {
            ready()?;
            self.device.poll(wgpu::Maintain::Poll);
            ready()?;
            match receiver.recv_timeout(Duration::from_millis(1)) {
                Ok(value) => return Ok(value),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => continue,
                Err(err) => {
                    return Err(terminal(format!("Vulkan submission did not finish: {err}")))
                }
            }
        }
    }

    /// Computes input[rows, inner] × weights[cols, inner]ᵀ with the model's native layout.
    pub fn linear(
        &self,
        lhs: &[f32],
        weights: &[f32],
        rows: usize,
        inner: usize,
        cols: usize,
        checkpoint: &mut dyn FnMut() -> Result<()>,
    ) -> Result<Vec<f32>> {
        ready()?;
        // Concurrent deck and library jobs share one queue and allocation budget.
        let _guard = loop {
            if let Some(guard) = self.run.try_lock_for(Duration::from_millis(20)) {
                break guard;
            }
            ready()?;
            checkpoint()?;
        };
        ready()?;
        checkpoint()?;
        let limits = self.device.limits();
        let limit = MAX_BUFFER_BYTES
            .min(limits.max_storage_buffer_binding_size as usize)
            .min(limits.max_buffer_size as usize);
        let input_bytes = matrix_bytes(rows, inner, limit)?;
        let weight_bytes = matrix_bytes(cols, inner, limit)?;
        let output_bytes = matrix_bytes(rows, cols, limit)?;
        ensure!(
            input_bytes / 4 == lhs.len() && weight_bytes / 4 == weights.len(),
            "Vulkan projection shape does not match its buffers"
        );
        let strip = dispatch_rows(
            inner,
            cols,
            limits.max_compute_workgroups_per_dimension as usize,
        )?;

        let allocate = |label, size, usage| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage_input = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let input = allocate("projection input", input_bytes as u64, storage_input);
        let weight = allocate("projection weights", weight_bytes as u64, storage_input);
        let output = allocate(
            "projection output",
            output_bytes as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = allocate(
            "projection readback",
            output_bytes as u64,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let params = allocate(
            "projection shape",
            16,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        ready()?;
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HTDemucs projection"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: weight.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        for first in (0..rows).step_by(strip) {
            ready()?;
            checkpoint()?;
            if first == 0 {
                self.queue
                    .write_buffer(&input, 0, bytemuck::cast_slice(lhs));
                self.queue
                    .write_buffer(&weight, 0, bytemuck::cast_slice(weights));
                ready()?;
            }
            let nrows = (rows - first).min(strip);
            let shape = [rows as u32, inner as u32, cols as u32, first as u32];
            self.queue
                .write_buffer(&params, 0, bytemuck::cast_slice(&shape));
            let mut commands = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = commands.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(
                    cols.div_ceil(TILE) as u32,
                    nrows.div_ceil(TILE) as u32,
                    1,
                );
            }
            let last_strip = first + nrows == rows;
            if last_strip {
                commands.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes as u64);
            }
            let buffer = commands.finish();
            ready()?;
            self.queue.submit([buffer]);
            if last_strip {
                // The readback can be mapped only after this submission's copy finishes.
                break;
            }
            let (tx, rx) = mpsc::channel();
            self.queue.on_submitted_work_done(move || {
                let _ = tx.send(());
            });
            self.wait(rx)?;
        }
        ready()?;
        let slice = readback.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.wait(rx)?
            .map_err(|e| terminal(format!("Vulkan readback failed: {e}")))?;
        let values = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();
        readback.unmap();
        check_readback(&values, &self.label)?;
        ready()?;
        self.completed.fetch_add(1, Ordering::Relaxed);
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_limits_bound_every_submission_and_buffer() {
        for (inner, cols) in [(512, 512), (512, 2048), (2048, 512), (31, 127)] {
            let rows = dispatch_rows(inner, cols, 65_535).unwrap();
            assert_eq!(rows % TILE, 0);
            assert!(
                rows * inner.next_multiple_of(TILE) * cols.next_multiple_of(TILE) <= MAX_TILE_FMAS
            );
        }
        assert!(dispatch_rows(4097, 256, 65_535).is_err());
        assert!(dispatch_rows(4096, 4096, 65_535).is_err());
        assert!(dispatch_rows(32, 32, 1).is_err());
        assert!(matrix_bytes(usize::MAX, 2, MAX_BUFFER_BYTES).is_err());
        assert!(matrix_bytes(0, 20, MAX_BUFFER_BYTES).is_err());
    }

    #[test]
    #[ignore = "sets the process-wide Vulkan hold; run in a fresh process"]
    fn nonfinite_readback_pauses_vulkan() {
        check_readback(&[1., -2., 0.], "test adapter").unwrap();
        assert!(backend_hold().is_none());
        assert!(check_readback(&[1., f32::NAN], "test adapter").is_err());
        assert!(backend_hold()
            .unwrap()
            .contains("non-finite Vulkan projection"));
        assert!(ready().is_err());
    }

    #[test]
    #[ignore = "sets the process-wide Vulkan hold; run in a fresh process"]
    fn initialization_deadline_pauses_vulkan() {
        let (release, blocked) = mpsc::channel::<()>();
        let first = initialize(Duration::from_millis(10), move || {
            blocked.recv()?;
            Ok(())
        });
        // The worker cannot finish before the deadline, independent of scheduling.
        drop(release);
        assert!(first.unwrap_err().to_string().contains("did not finish"));
        let reason = backend_hold().unwrap();
        assert!(reason.contains("did not finish"));

        let (attempt, called) = mpsc::channel();
        let next = initialize(STARTUP_TIMEOUT, move || {
            attempt.send(())?;
            Ok(())
        });
        assert_eq!(next.unwrap_err().to_string(), reason);
        assert!(matches!(
            called.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    // Run explicitly with Mesa Lavapipe. Production never selects a software Vulkan adapter.
    #[test]
    #[ignore]
    fn software_vulkan_linear_matches_reference() -> Result<()> {
        let gpu = Engine::new_for_test()?;
        assert_eq!(gpu.completed_matmuls(), 0);
        let (rows, inner, cols) = (273, 37, 19);
        let lhs: Vec<f32> = (0..rows * inner)
            .map(|i| ((i % 17) as f32 - 8.) / 7.)
            .collect();
        let weights: Vec<f32> = (0..cols * inner)
            .map(|i| ((i % 13) as f32 - 6.) / 11.)
            .collect();
        let actual = gpu.linear(&lhs, &weights, rows, inner, cols, &mut || Ok(()))?;
        for i in 0..rows {
            for j in 0..cols {
                let reference = (0..inner)
                    .map(|k| lhs[i * inner + k] * weights[j * inner + k])
                    .sum::<f32>();
                assert!(
                    (reference - actual[i * cols + j]).abs() < 2e-5,
                    "at output [{i}, {j}]"
                );
            }
        }
        let mut checkpoints = 0;
        let canceled = gpu.linear(&lhs, &weights, rows, inner, cols, &mut || {
            checkpoints += 1;
            ensure!(checkpoints < 3, "canceled between strips");
            Ok(())
        });
        assert!(canceled
            .unwrap_err()
            .to_string()
            .contains("canceled between strips"));
        assert!(gpu
            .linear(&[], &weights, rows, inner, cols, &mut || Ok(()))
            .is_err());
        assert_eq!(gpu.completed_matmuls(), 1);
        Ok(())
    }
}
