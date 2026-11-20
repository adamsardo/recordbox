use super::{backend_label, device, htdemucs, models_dir, planar_44k, CATALOG};
use anyhow::{ensure, Context, Result};
use candle_core::Device;
use std::time::Instant;

pub(super) fn relative_error(reference: &[f32], actual: &[f32]) -> Result<f64> {
    ensure!(!reference.is_empty() && reference.len() == actual.len(), "different stem lengths");
    let (mut energy, mut difference) = (0.0, 0.0);
    for (&a, &b) in reference.iter().zip(actual) {
        ensure!(a.is_finite() && b.is_finite(), "non-finite output");
        energy += f64::from(a).powi(2);
        difference += (f64::from(a) - f64::from(b)).powi(2);
    }
    // Use a -100 dBFS floor for silent parts.
    Ok((difference / energy.max(reference.len() as f64 * 1e-10)).sqrt())
}

/// Reuse the application's model, windowing and decoder without consulting the stem cache.
pub fn compare_track(path: &str) -> Result<()> {
    println!("STEMS backend: {}", backend_label());
    let accelerator = device();
    ensure!(super::backend::gpu_active(), "An active GPU is required to compare against the CPU");
    let checkpoint = models_dir().join(CATALOG[1].files[0]);
    ensure!(checkpoint.is_file(), "Download HTDemucs in Preferences → Extensions → STEMS first");
    let decoded = crate::audio::decode_file(path)?;
    let (audio, frames) = planar_44k(&decoded.samples, decoded.sr);
    ensure!(frames > 0, "Empty track");
    let seconds = frames as f64 / f64::from(htdemucs::SAMPLE_RATE);
    println!("HTDemucs · {seconds:.2} seconds of audio · four stereo stems");
    let run = |model: htdemucs::Graph, device: &Device, label: &str| -> Result<_> {
        let start = Instant::now();
        let parts = htdemucs::separate(&model, device, &audio, frames, &Default::default(), &mut |_, _| true)?;
        let elapsed = start.elapsed().as_secs_f64();
        println!("{label}: {elapsed:.2} seconds ({:.2}× real time, excluding model load)", seconds / elapsed);
        ensure!(parts.len() == 8 * frames, "Incomplete {label} output");
        Ok((parts, elapsed))
    };
    let (gpu, gpu_time) = run(htdemucs::Graph::load_selected(&checkpoint)?, &accelerator, "GPU")?;
    let (cpu, cpu_time) = run(htdemucs::Graph::load(&checkpoint, &Device::Cpu)?, &Device::Cpu, "CPU")?;
    println!("CPU time / GPU time: {:.2}×", cpu_time / gpu_time);
    let mut agree = true;
    for ((name, reference), result) in htdemucs::SOURCES.iter().zip(cpu.chunks_exact(2 * frames)).zip(gpu.chunks_exact(2 * frames)) {
        let error = relative_error(reference, result).with_context(|| format!("{name} comparison"))?;
        println!("{name}: {:.3}% relative RMS difference", 100.0 * error);
        agree &= error <= 0.03;
    }
    ensure!(agree, "At least one stem exceeds the preview's 3% comparison tolerance");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_stereo_samples_and_rejects_invalid_output() {
        assert_eq!(relative_error(&[0., 0.], &[0., 0.]).unwrap(), 0.);
        assert!((relative_error(&[1., -1.], &[0.5, -0.5]).unwrap() - 0.5).abs() < 1e-12);
        assert!(relative_error(&[1.], &[]).is_err());
        assert!(relative_error(&[1.], &[f32::NAN]).is_err());
    }
}
