"""Fetch the Windows x64 DirectML runtime and notices from pinned Microsoft NuGet packages."""
import argparse
import hashlib
from pathlib import Path
from tempfile import TemporaryDirectory
from urllib.request import urlopen
from zipfile import ZipFile
import shutil


# The per-file hashes are also used when checking the final release ZIP on Linux.
PACKAGES = {
    ("microsoft.ml.onnxruntime.directml", "1.24.4", "57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681"): {
        "runtimes/win-x64/native/onnxruntime.dll": ("onnxruntime.dll", "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40"),
        "runtimes/win-x64/native/onnxruntime_providers_shared.dll": ("onnxruntime_providers_shared.dll", "265c8daf29637cb259cac8be9f08f2cd45f3883f0f0e4949cbfddd5b4cbec3b6"),
        "LICENSE": ("onnxruntime-LICENSE.txt", "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674"),
        "ThirdPartyNotices.txt": ("onnxruntime-ThirdPartyNotices.txt", "fb0af774b4d7cffc5b9d046f2aaeade2f37df2f80abf8033c95dfffcc77a8866"),
    },
    ("microsoft.ai.directml", "1.15.4", "4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9"): {
        "bin/x64-win/DirectML.dll": ("directml.dll", "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1"),
        "LICENSE.txt": ("DirectML-LICENSE.txt", "a05138e3a085ff60a44881eedfa58dccb03ecc1d7b1f6ae888418e8c2fec4b8d"),
        "LICENSE-CODE.txt": ("DirectML-LICENSE-CODE.txt", "903df5512f7d02609fed0c780a9b704f5a3eeb6e4d84ebe42a29845c81899a3c"),
        "ThirdPartyNotices.txt": ("DirectML-ThirdPartyNotices.txt", "2c95795c13ff48a58b6ed916f37901c23d964b5d9d601af422f17ad2172e7950"),
    },
}
FILES = dict(entry for members in PACKAGES.values() for entry in members.values())
LIBRARIES = tuple(name for name in FILES if name.endswith(".dll"))


def digest(path):
    with path.open("rb") as data:
        return hashlib.file_digest(data, "sha256").hexdigest()


def verify(root, packaged=False):
    for name, expected in FILES.items():
        path = root / ("runtime-notices" if packaged and name not in LIBRARIES else "") / name
        if not path.is_file() or path.is_symlink() or digest(path) != expected:
            raise ValueError(f"Missing or incorrect Microsoft runtime file: {name}")


def fetch(output):
    output.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=output) as temporary:
        staging = Path(temporary)
        for (package, version, expected), members in PACKAGES.items():
            file = staging / f"{package}.{version}.nupkg"
            url = f"https://api.nuget.org/v3-flatcontainer/{package}/{version}/{file.name}"
            with urlopen(url, timeout=90) as source, file.open("wb") as dest:
                shutil.copyfileobj(source, dest)
            if digest(file) != expected:
                raise ValueError(f"Unexpected SHA-256 for {file.name}")
            with ZipFile(file) as archive:
                for member, (name, _) in members.items():
                    (staging / name).write_bytes(archive.read(member))
        verify(staging)
        for name in FILES:
            (staging / name).replace(output / name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    fetch(parser.parse_args().directory)
