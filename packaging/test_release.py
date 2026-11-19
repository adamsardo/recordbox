"""Synthetic native archives exercise release verification without running code."""
import copy
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import zlib

import build_archive as package
import prepare_release as release


def executable(platform, architecture):
    if platform == "macos":
        return struct.pack("<IIIIIIII", 0xFEEDFACF, 0x100000C if architecture == "arm64" else 0x1000007,
                           0, 2, 1, 72, 0, 0) + struct.pack("<II", 0x19, 72) + bytes(64)
    if platform == "windows":
        dos = bytearray(64); dos[:2] = b"MZ"; struct.pack_into("<I", dos, 60, 64)
        return bytes(dos) + b"PE\0\0" + struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 112, 2) + b"\x0b\x02" + bytes(110 + 40)
    return (b"\x7fELF\x02\x01\x01" + bytes(9) + struct.pack("<HHIQQQIHHHHHH", 3, 62, 1, 0x400000,
            64, 0, 0, 64, 56, 1, 0, 0, 0) + struct.pack("<IIQQQQQQ", 1, 5, 0, 0x400000, 0, 120, 120, 4096))


def png():
    def chunk(kind, payload):
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\xff\0\0\xff")) + chunk(b"IEND", b"")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="recordbox-release-tests-")
        self.addCleanup(temporary.cleanup); self.root = Path(temporary.name)
        self.source = self.root / "source"; (self.source / "app/assets/fonts").mkdir(parents=True)
        (self.source / "packaging").mkdir(); (self.source / "docs/releases").mkdir(parents=True)
        (self.source / "VERSION").write_text("0.5.0\n")
        (self.source / "app/Cargo.toml").write_text('[package]\nname="recordbox"\nversion="0.5.0"\n')
        (self.source / "app/Cargo.lock").write_text('version=4\n[[package]]\nname="recordbox"\nversion="0.5.0"\n')
        (self.source / "LICENSE").write_text("MIT fixture license\n")
        (self.source / "app/assets/fonts/README.txt").write_text("Inter font notice\n")
        shutil.copyfile(Path(package.__file__).parent / "Inter-OFL.txt", self.source / "packaging/Inter-OFL.txt")
        (self.source / "docs/releases/0.5.0.md").write_text("# Recordbox 0.5.0\n\nTested native archives.\n")
        self.commit = "a" * 40; self.input = self.root / "input"; self.output = self.root / "staged"
        self.packages = self.input / "packages"; self.smoke = self.input / "smoke"
        self.packages.mkdir(parents=True); self.smoke.mkdir()
        self.names = {}
        for platform, architecture in release.TARGETS:
            binary = self.root / "binary"; binary.write_bytes(executable(platform, architecture)); binary.chmod(0o755)
            info = package.BinaryInfo(architecture, {"/usr/lib/libSystem.B.dylib": "/usr/lib/libSystem.B.dylib"}, "11.0") if platform == "macos" else package.BinaryInfo(architecture)
            with patch.object(package, "inspect_binary", return_value=info):
                result = package.build_archive(binary, self.source, self.root / f"built-{platform}-{architecture}",
                                               platform, architecture, self.commit)
            archive = Path(result["archive"]); self.names[platform, architecture] = archive.name
            shutil.copyfile(archive, self.packages / archive.name)
            shutil.copyfile(str(archive) + ".sha256", self.packages / (archive.name + ".sha256"))
            directory = self.smoke / f"smoke-{platform}-{architecture}-{self.commit}"; directory.mkdir()
            report = {"status": "passed", "synthetic_data_only": True, "archive": archive.name,
                      "archive_sha256": result["sha256"], "source_commit": self.commit,
                      "target": result["manifest"]["target"], "process_returncode": 0,
                      "profile_created": True, "database_created": True, "screenshot": {"width": 1, "height": 1}}
            (directory / "smoke.json").write_text(json.dumps(report)); (directory / "synthetic-window.png").write_bytes(png())
            (directory / "application.log").write_text("synthetic native CI fixture\n")
        self.log = f"commit {self.commit}\nAuthor: Example <example@example.com>\nDate: 2026-11-19\n\nRelease fixtures\n"

    def stage(self, tag="v0.5.0"):
        with patch.object(release, "get_commit_log", return_value=self.log):
            return release.prepare_release(self.input, self.output, self.source, self.commit, tag)

    def report_path(self, platform="macos", architecture="arm64"):
        return self.smoke / f"smoke-{platform}-{architecture}-{self.commit}" / "smoke.json"

    def refresh_checksum(self, platform="macos", architecture="arm64"):
        name = self.names[platform, architecture]; checksum = package.sha256(self.packages / name)
        (self.packages / (name + ".sha256")).write_text(f"{checksum}  {name}\n")
        path = self.report_path(platform, architecture); report = json.loads(path.read_text())
        report["archive_sha256"] = checksum; path.write_text(json.dumps(report))

    def change_zip(self, change, platform="macos", architecture="arm64"):
        path = self.packages / self.names[platform, architecture]
        with zipfile.ZipFile(path) as source:
            entries = [(copy.copy(member), source.read(member)) for member in source.infolist()]
        change(entries)
        with zipfile.ZipFile(path, "w") as target:
            for member, data in entries: target.writestr(member, data)
        self.refresh_checksum(platform, architecture)

    def change_manifest(self, change, platform="macos", architecture="arm64"):
        def mutate(entries):
            for i, (member, data) in enumerate(entries):
                if member.filename.endswith("/runtime-manifest.json"):
                    manifest = json.loads(data); change(manifest)
                    entries[i] = (member, json.dumps(manifest).encode())
        self.change_zip(mutate, platform, architecture)

    def replace_product_file(self, suffix, data):
        def mutate(entries):
            name = next(member.filename for member, _ in entries if member.filename.endswith(suffix))
            for i, (member, old) in enumerate(entries):
                if member.filename == name: entries[i] = (member, data)
                elif member.filename.endswith("/runtime-manifest.json"):
                    manifest = json.loads(old)
                    row = next(r for r in manifest["files"] if name.endswith("/" + r["path"]))
                    row.update(size=len(data), sha256=hashlib.sha256(data).hexdigest())
                    entries[i] = (member, json.dumps(manifest).encode())
        self.change_zip(mutate)

    def test_valid_four_platform_release_has_exact_assets_and_source_marker(self):
        self.stage()
        assets = self.output / "assets"
        self.assertEqual({p.name for p in assets.iterdir()}, set(self.names.values()) | {"COMMIT-LOG.txt", "SHA256SUMS"})
        sums = (assets / "SHA256SUMS").read_text().splitlines(); self.assertEqual(len(sums), 5)
        for line in sums:
            digest, name = line.split("  "); self.assertEqual(digest, package.sha256(assets / name))
        body = (self.output / "body.md").read_text()
        self.assertIn(f"<!-- recordbox-release-source: {self.commit} -->", body)
        self.assertIn("Author: Example", body); self.assertIn("0.5.0", body)

    def test_tag_mismatch_or_existing_output_is_refused(self):
        with self.assertRaisesRegex(package.PackageError, "Tag"): self.stage("v0.5.1")
        self.output.mkdir(); (self.output / "keep").write_text("existing")
        with self.assertRaisesRegex(package.PackageError, "already exists"): self.stage()
        self.assertEqual((self.output / "keep").read_text(), "existing")

    def test_missing_extra_and_corrupted_package_inputs_are_refused(self):
        sidecar = self.packages / (next(iter(self.names.values())) + ".sha256")
        original = sidecar.read_bytes(); sidecar.write_text("wrong checksum\n")
        with self.assertRaisesRegex(package.PackageError, "checksum"): self.stage()
        sidecar.write_bytes(original); extra = self.packages / "other.zip"; extra.write_bytes(b"unexpected")
        with self.assertRaisesRegex(package.PackageError, "package files"): self.stage()
        extra.unlink(); sidecar.unlink()
        with self.assertRaisesRegex(package.PackageError, "package files"): self.stage()
        self.assertFalse(self.output.exists())

    def test_failed_mixed_or_incomplete_smoke_never_stages(self):
        path = self.report_path(); original = json.loads(path.read_text())
        for key, value in (("status", "failed"), ("source_commit", "b" * 40), ("archive_sha256", "0" * 64),
                           ("process_returncode", False), ("profile_created", False), ("database_created", False),
                           ("screenshot", {"width": 2, "height": 1}),
                           ("windows_opengl_test_driver", {"temporary_override": True})):
            with self.subTest(key=key):
                report = dict(original); report[key] = value; path.write_text(json.dumps(report))
                with self.assertRaises(package.PackageError): self.stage()
                self.assertFalse(self.output.exists())
        path.write_text(json.dumps(original))
        (path.parent / "synthetic-window.png").write_bytes(png()[:-2])
        with self.assertRaises(package.PackageError): self.stage()

    def test_forged_manifest_source_version_and_file_list_are_refused(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        changes = [lambda m: m.update(version="0.4.0"), lambda m: m["source"].update(commit="b" * 40),
                   lambda m: m["source"].update(cargo_lock_sha256="0" * 64),
                   lambda m: m["files"].append(dict(m["files"][0])),
                   lambda m: m["files"][0].update(mode="0777")]
        for change in changes:
            path.write_bytes(original); self.refresh_checksum(); self.change_manifest(change)
            with self.assertRaises(package.PackageError): self.stage()
            self.assertFalse(self.output.exists())

    def test_windows_crlf_checkout_hashes_are_allowed_only_for_windows(self):
        def crlf(m):
            for name, key in (("app/Cargo.toml", "cargo_manifest_sha256"), ("app/Cargo.lock", "cargo_lock_sha256")):
                m["source"][key] = hashlib.sha256((self.source / name).read_bytes().replace(b"\n", b"\r\n")).hexdigest()
        self.change_manifest(crlf, "windows", "x86_64"); self.stage(); shutil.rmtree(self.output)
        self.change_manifest(crlf)
        with self.assertRaisesRegex(package.PackageError, "Cargo digest"): self.stage()

    def test_wrong_machine_still_fails_with_self_consistent_manifest_hashes(self):
        self.replace_product_file("/Contents/MacOS/recordbox", executable("macos", "x86_64"))
        with self.assertRaisesRegex(package.PackageError, "Mach-O"): self.stage()

    def test_changed_license_and_plist_are_refused_even_if_manifest_rehashed(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        self.replace_product_file("/Inter-OFL.txt", b"abbreviated license")
        with self.assertRaisesRegex(package.PackageError, "license notice"): self.stage()
        path.write_bytes(original); self.refresh_checksum()
        plist = {"CFBundleVersion": "0.4.0", "CFBundleShortVersionString": "0.4.0", "CFBundleExecutable": "recordbox"}
        self.replace_product_file("/Contents/Info.plist", plistlib.dumps(plist))
        with self.assertRaisesRegex(package.PackageError, "plist version"): self.stage()

    def test_traversal_and_special_file_members_fail_before_extraction(self):
        path = self.packages / self.names["macos", "arm64"]; original = path.read_bytes()
        for name, mode in (("Recordbox.app/../escape", stat.S_IFREG | 0o644),
                           ("Recordbox.app/device", stat.S_IFCHR | 0o644),
                           ("Recordbox.app/sticky", stat.S_IFREG | 0o4755)):
            path.write_bytes(original); self.refresh_checksum()
            member = zipfile.ZipInfo(name); member.create_system = 3; member.external_attr = mode << 16
            self.change_zip(lambda entries: entries.append((member, b"unsafe")))
            with self.assertRaises(package.PackageError): self.stage()
            self.assertFalse((self.root / "escape").exists()); self.assertFalse(self.output.exists())

    def test_bad_native_headers_are_rejected_without_execution(self):
        binary = self.root / "bad"
        for platform, architecture in release.TARGETS:
            binary.write_bytes(executable(platform, architecture)[:20])
            with self.subTest(platform=platform), self.assertRaises(package.PackageError):
                release.verify_machine(binary, platform, architecture)


if __name__ == "__main__":
    unittest.main()
