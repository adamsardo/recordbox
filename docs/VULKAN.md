# Experimental Linux Vulkan STEMS

The Linux x64 Vulkan preview runs HTDemucs transformer linear layers on a physical GPU through Vulkan compute. The CPU runs the rest of the model, including the convolutions and Fourier transforms. CPU time and transfers between the CPU and GPU both affect total separation time. The single-checkpoint **Prioritize speed** model is a good first test.

## Try it on Linux

1. Install the Vulkan loader and a driver for your GPU. On Debian with Radeon Vega graphics, the packages are `libvulkan1` and `mesa-vulkan-drivers`. The optional `vulkan-tools` package provides `vulkaninfo --summary`. Look for your Radeon in that output. A device named `llvmpipe` runs on the CPU.
2. Download the `vulkan-preview-linux-x86_64-<commit>` artifact from a successful **Build and package** workflow run for this change. Unzip the artifact, run `sha256sum -c *.sha256` beside the tarball, then unpack `Recordbox-<version>-linux-x86_64-vulkan.tar.gz` into its own directory. [Build and package runs](https://github.com/fantuz/recordbox/actions/workflows/build.yml)
3. Open a terminal in the extracted directory and inspect the device.

   ```sh
   ./recordbox --stems-status
   ./recordbox --stems-device
   ```

   The first command lists detected hardware and installed checkpoints. The second runs a small calculation and reports the selected backend. A successful GPU probe identifies **Vulkan + CPU** and the adapter name. A CPU result gives the reason. A full-model test exercises much more of the driver than the probe does.
4. In **Preferences → Extensions → STEMS**, enable STEMS, select **Prioritize speed** and download **HTDemucs**. Load a track and choose **STEMS → Separate**. In this preview, loading a track can load cached parts. Starting a new deck separation requires pressing **Separate**, including when earlier versions saved an automatic-separation preference.

## Compare one track

Use a short, non-silent audio clip for the first run.

```sh
./recordbox --compare-stems "/path/to/clip.wav"
```

The comparison bypasses the stem cache and runs the same speed-model checkpoint twice, first with Vulkan and CPU together, then with CPU alone. It prints separation times and relative RMS differences for drums, bass, other and vocals. Each stem must be within 3% of its CPU reference. The times exclude model loading. Compare elapsed time on your own machine before choosing the preview for longer tracks.

Each GPU submission covers a bounded strip of a matrix. Recordbox waits for its completion before submitting another. Individual projection buffers are capped at 32 MiB. The Vulkan waits have deadlines, but an application cannot guarantee recovery if the graphics driver hangs inside a system call.

After a Vulkan timeout or device loss, GPU work pauses for the rest of the session. **Retry CPU** repeats the failed track with its original model. To start directly on the CPU, launch `./recordbox --stems-cpu`. The comparison requires a working GPU backend.

## Build the preview

Install the normal Linux build prerequisites and run the following from the repository root. [Build requirements](BUILD-AND-PACKAGING.md)

```sh
cargo build --release --locked --manifest-path app/Cargo.toml --features vulkan
```
