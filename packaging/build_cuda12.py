"""Build the optional Pascal-compatible Recordbox executable with CUDA Toolkit 12.9."""
import argparse
from contextlib import contextmanager
import filecmp
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "app" / "Cargo.toml"
KERNEL_BUILD_SHA = "aab4cf3bf981980e3a907692138122d9a5e50c293de7e4813443f23132efb49e"


def pascal_kernel_copy(source, destination):
    # The upstream CUDA crate builds both the floating-point graph and unrelated,
    # statically linked quantized/MoE kernels. Only the latter require Tensor Cores.
    script = (source / "build.rs").read_bytes()
    if hashlib.sha256(script).hexdigest() != KERNEL_BUILD_SHA:
        raise ValueError("Review CUDA architecture targets for this Candle kernel revision")
    shutil.copytree(source, destination)
    old = b"moe_builder.build_lib("
    if script.count(old) != 1:
        raise ValueError("Missing Candle static kernel builder")
    script = script.replace(old, b"moe_builder.compute_cap(80).build_lib(")
    if os.name == "nt":
        # rustc resolves -lmoe as moe.lib for the MSVC target.
        script = script.replace(b'out_dir.join("libmoe.a")', b'out_dir.join("moe.lib")')
    (destination / "build.rs").write_bytes(script)
    # A scalar half-precision atomic add first appeared on SM 7.0. HTDemucs
    # uses FP32 reductions. Keep all its ordinary SM 6.1 reduction kernels.
    reduce = destination / "src" / "reduce.cu"
    source_code = reduce.read_bytes()
    half_sum = b"SUM_OP(__half, sum_f16)"
    if source_code.count(half_sum) != 1:
        raise ValueError("Review Candle's half-precision reduction for Pascal")
    reduce.write_bytes(source_code.replace(half_sum, b"#if __CUDA_ARCH__ >= 700\n" + half_sum + b"\n#endif"))


def same_sources(expected, existing):
    if not existing.is_dir() or existing.is_symlink():
        return False
    for entry in expected.rglob("*"):
        if entry.is_file():
            other = existing / entry.relative_to(expected)
            if other.is_symlink() or not other.is_file() or not filecmp.cmp(entry, other, shallow=False):
                return False
    return {p.relative_to(expected) for p in expected.rglob("*") if p.is_file()} == {
        p.relative_to(existing) for p in existing.rglob("*") if p.is_file()
    }


@contextmanager
def kernel_override(source, stable):
    # Cargo CI caches may restore a partial directory. Build from a complete
    # private copy when this happens, without replacing files another build uses.
    stable.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=stable.parent) as tmp:
        prepared = Path(tmp) / "candle-kernels"
        pascal_kernel_copy(source, prepared)
        if not stable.exists():
            try:
                prepared.rename(stable)
            except OSError:
                if not stable.is_dir():
                    raise
            else:
                yield stable
                return
        yield stable if same_sources(prepared, stable) else prepared


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "dist" / "cuda12")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "app" / "target" / "cuda12")
    args = parser.parse_args()
    if sys.platform not in ("linux", "win32"):
        parser.error("Use a Windows or Linux x64 build host")
    nvcc = subprocess.check_output(["nvcc", "--version"], text=True)
    if not re.search(r"release 12\.9,", nvcc):
        parser.error("Install CUDA Toolkit 12.9 and put its nvcc first on PATH")
    rust = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.split()[1] for line in rust.splitlines() if line.startswith("host:"))
    if host not in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
        parser.error(f"Unsupported build host {host}")
    args.target_dir = args.target_dir.resolve()
    args.target_dir.mkdir(parents=True, exist_ok=True)
    flags = ["--manifest-path", str(MANIFEST), "--locked", "--features", "cuda12"]
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", *flags, "--format-version", "1", "--filter-platform", host,
    ]))
    kernels, = [p for p in metadata["packages"] if p["name"] == "candle-kernels"]
    # A Cargo path override keeps the registry checkout and the signed-off lockfile intact.
    key = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()[:16]
    # Keep source out of the Cargo target tree pruned by rust-cache.
    stable = ROOT / "dist" / ".cuda-sources" / f"candle-kernels-{kernels['version']}-{key}"
    build_env = dict(os.environ, CUDA_COMPUTE_CAP="61")
    with kernel_override(Path(kernels["manifest_path"]).parent, stable) as patched:
        config = "paths = [" + json.dumps(patched.as_posix()) + "]"
        subprocess.run(["cargo", "--config", config, "build", *flags, "--release",
                        "--bin", "recordbox", "--target-dir", str(args.target_dir)],
                       check=True, env=build_env)
    suffix = ".exe" if os.name == "nt" else ""
    args.output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(args.target_dir / "release" / ("recordbox" + suffix),
                 args.output / ("recordbox-cuda12" + suffix))


if __name__ == "__main__":
    main()
