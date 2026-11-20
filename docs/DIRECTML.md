# Radeon STEMS on Windows

The Windows x64 DirectML ZIP contains the complete Recordbox application and the Microsoft runtime. It targets Radeon Vega 8 and other physical AMD GPUs with a Windows DirectX 12 driver. An integrated Vega uses system memory. Run the comparison below to measure whether it accelerates your machine.

1. Install your laptop manufacturer's or AMD's current graphics driver for your adapter. Extract the entire **Windows x64, Radeon DirectML STEMS preview** ZIP into its own directory.
2. Open PowerShell in that directory and run `.\recordbox.exe --stems-device`. A successful check prints **DirectML**, the Radeon name and its DXGI adapter number after a small FP32 convolution and matrix multiplication. The full HTDemucs network uses more memory and additional operators. Recordbox searches the physical AMD adapters on multi-GPU laptops. If initialization fails, the output explains the CPU selection.
3. In **Preferences → Extensions → STEMS**, choose **Prioritize speed** and download **HTDemucs**. The GPU and CPU paths share the same local checkpoint. The fine-tuned specialists also use the selected backend.
4. Run the command below with one local track. It bypasses the stem cache and prints the time for each backend and the relative RMS difference for drums, bass, other and vocals. A ratio above 1 means the GPU finished faster. Both timings exclude model initialization.

```powershell
.\recordbox.exe --compare-stems "C:\Music\example.wav"
```

Run `.\recordbox.exe --stems-cpu` to use CPU separation for a session. If the full model fails after the device check succeeds, hover over the failure in the deck’s STEMS strip for the provider message. Click it to copy the full details, including the operation, adapter and available driver diagnostics. Choose **Retry CPU** to restart separation for that track with the same model. Your next track can use DirectML again.

At startup and in Preferences, Recordbox reads the adapter name. The STEMS compute device opens when you explicitly check it or start work that needs it. Loading a track can start work if automatic separation or vocal detection is enabled.

On integrated adapters such as Vega 8, deck separation, vocal detection and stem-assisted key analysis take turns on the GPU. Recordbox releases the compiled network and its compute resources after each checkpoint and uses smaller attention tiles and fewer queued audio buffers during separation. Task Manager’s Performance tab includes GPU memory used by the desktop and other applications. The standard Windows ZIP also offers CPU separation. The CUDA 12 ZIP targets NVIDIA hardware.

## Build and verify

In the usual Windows x64 MSVC build environment, fetch the two pinned Microsoft NuGet packages and build the optional feature.

```powershell
python packaging/directml_runtime.py app/target/release
cargo build --release --locked --manifest-path app/Cargo.toml --features directml
.\app\target\release\recordbox.exe --stems-device
```

The release packager includes the x64 DLLs and their license notices. It checks both NuGet archives and each distributed runtime file against SHA-256 digests. The network uses ONNX opset 13 and FP32 with fixed-size inputs. The DirectML session requires the selected hardware provider for network execution. Recordbox computes the audio transforms and combines overlapping segments on the CPU. [Microsoft DirectML requirements](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html#requirements)
