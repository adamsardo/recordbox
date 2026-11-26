# NVIDIA CUDA 12 STEMS preview

Choose the **Windows x64 CUDA 12** or **Linux x64 CUDA 12** archive from the release. Each contains the full application. The standard Windows and Linux archives run STEMS on the CPU. macOS uses Metal when available.

## On your computer

1. Check the adapter and driver with `nvidia-smi --query-gpu=name,driver_version --format=csv,noheader`. This preview targets NVIDIA compute capability **6.1** and newer architectures supported by CUDA 12.9. The Quadro P2000 is a Pascal target. The GeForce GTX 750 is below the build's minimum. The model also needs enough free graphics memory for one segment.
2. Install a driver that supports CUDA 12.9 PTX and the **CUDA Toolkit 12.9 Update 1** runtime libraries, including cuBLAS, cuRAND and the CUDA runtime. The toolkit installer supplies these libraries. On Windows, its `bin` directory must be on `PATH`. On Linux, register its runtime library directory with the loader or add it to `LD_LIBRARY_PATH`. The standard archive works on systems without these dependencies. [NVIDIA installer](https://developer.nvidia.com/cuda-12-9-1-download-archive) · [NVIDIA driver compatibility](https://docs.nvidia.com/cuda/archive/12.9.1/cuda-toolkit-release-notes/index.html) · [NVIDIA GPU capabilities](https://developer.nvidia.com/cuda/gpus/legacy)
3. Extract the CUDA archive into its own folder. From that folder, run the following in PowerShell, or use `./recordbox --stems-device` on Linux.

   ```powershell
   .\recordbox.exe --stems-device
   ```

   A successful probe prints the GPU name, compute capability and total VRAM after a small FP32 convolution and matrix multiplication. The same information appears in **Preferences → Extensions → STEMS**. If initialization fails after startup, the app reports the reason and selects the CPU. If the OS reports a missing NVIDIA DLL or shared library before startup, install the runtime above or launch the standard archive.

4. Choose **Prioritize speed** in **Preferences → Extensions → STEMS** and download **HTDemucs**. To test one track interactively, turn off **Separate tracks when they are loaded**, load the track, then click a stem part. A previous separation can be served from cache. To request CPU separation explicitly, start the application with `--stems-cpu`.

## Compare one song

This command decodes the named file, runs the same HTDemucs checkpoint on the selected GPU and CPU, and compares all four stereo outputs. It bypasses the saved stem cache and prints elapsed seconds, real-time factors and each stem's relative RMS difference. A missing GPU, invalid output or a difference above 3% produces a nonzero exit code. It can take several minutes.

```powershell
.\recordbox.exe --compare-stems "C:\Music\example.wav"
```

On Linux use `./recordbox --compare-stems "/path/to/example.wav"`. To watch device memory in another terminal, run `nvidia-smi --query-gpu=name,memory.used,utilization.gpu --format=csv -l 1`. Other programs sharing the GPU contribute to these readings. For an out-of-memory error, close other GPU jobs or use the standard CPU build.

## Build from source

On Linux x64 or in a Windows **x64 Native Tools Command Prompt for VS 2022**, install the normal Recordbox build prerequisites and CUDA Toolkit 12.9.1. With `nvcc` on `PATH`, run the following from the repository root.

```sh
python packaging/build_cuda12.py
```

The executable is written to `dist/cuda12`. The build makes a private copy of Candle's locked kernel crate. Its floating-point kernels target SM 6.1. Its separate Tensor Core kernels for quantized models target SM 8.0. HTDemucs uses FP32 and processes attention in blocks of 256 queries to limit peak temporary memory. Release verification checks the embedded Pascal PTX and native CUDA imports. Playback and beat-grid analysis use their existing paths. [Standard build prerequisites](BUILD-AND-PACKAGING.md)
