#!/usr/bin/env python3
"""Fail-closed verification for Silent Disco Linux release bundles."""

from __future__ import annotations

import argparse
import configparser
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile

EXPECTED_DEB_DEPENDENCIES = {
    "libasound2",
    "libgtk-3-0",
    "libwebkit2gtk-4.1-0",
}
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
PRODUCTION_FORBIDDEN_BINARIES = {
    "block45_runtime_probe",
    "performance_probe",
}


def fail(message: str) -> None:
    raise AssertionError(message)


def single_match(root: Path, pattern: str, label: str) -> Path:
    matches = sorted(root.glob(pattern))
    if len(matches) != 1:
        fail(f"expected exactly one {label} matching {root / pattern}; found {matches}")
    return matches[0]


def read_desktop_entry(path: Path) -> dict[str, str]:
    parser = configparser.ConfigParser(interpolation=None, strict=True)
    parser.optionxform = str
    with path.open(encoding="utf-8") as handle:
        parser.read_file(handle)
    if parser.sections() != ["Desktop Entry"]:
        fail(f"{path}: expected exactly one [Desktop Entry] section")
    return dict(parser["Desktop Entry"])


def validate_desktop_entry(path: Path, *, product_name: str, main_binary: str) -> None:
    entry = read_desktop_entry(path)
    expected = {
        "Name": product_name,
        "Exec": main_binary,
        "Icon": main_binary,
        "StartupWMClass": main_binary,
        "Terminal": "false",
        "Type": "Application",
    }
    for key, value in expected.items():
        if entry.get(key) != value:
            fail(f"{path}: {key}={entry.get(key)!r}; expected {value!r}")
    categories = {value for value in entry.get("Categories", "").split(";") if value}
    if not {"AudioVideo", "Audio", "Music"}.issubset(categories):
        fail(f"{path}: expected AudioVideo/Audio/Music categories, got {sorted(categories)}")


def png_dimensions(path: Path) -> tuple[int, int, int, int]:
    with path.open("rb") as handle:
        header = handle.read(33)
    if len(header) < 33 or header[:8] != PNG_SIGNATURE or header[12:16] != b"IHDR":
        fail(f"{path}: not a valid PNG with an IHDR header")
    width, height, bit_depth, color_type = struct.unpack(">IIBB", header[16:26])
    return width, height, bit_depth, color_type


def validate_icon(root: Path, *, main_binary: str) -> None:
    icon = root / "usr/share/icons/hicolor/512x512/apps" / f"{main_binary}.png"
    if not icon.is_file():
        fail(f"missing installed 512x512 application icon: {icon}")
    width, height, bit_depth, color_type = png_dimensions(icon)
    if (width, height, bit_depth, color_type) != (512, 512, 8, 6):
        fail(
            f"{icon}: expected 512x512 8-bit RGBA PNG; "
            f"got {width}x{height}, bit depth {bit_depth}, color type {color_type}"
        )


def validate_release_binaries(root: Path, *, main_binary: str) -> None:
    bin_dir = root / "usr/bin"
    if not bin_dir.is_dir():
        fail(f"missing release binary directory: {bin_dir}")
    names = {entry.name for entry in bin_dir.iterdir() if entry.is_file()}
    if main_binary not in names:
        fail(f"{bin_dir}: missing main binary {main_binary!r}; found {sorted(names)}")
    forbidden = names & PRODUCTION_FORBIDDEN_BINARIES
    if forbidden:
        fail(f"{bin_dir}: Block 45 probe binary leaked into production bundle: {sorted(forbidden)}")
    extras = names - {main_binary}
    if extras:
        fail(f"{bin_dir}: unexpected production executables: {sorted(extras)}")
    binary = bin_dir / main_binary
    if not os.access(binary, os.X_OK):
        fail(f"{binary}: main binary is not executable")


def validate_payload_root(root: Path, *, product_name: str, main_binary: str) -> None:
    desktop = root / "usr/share/applications" / f"{product_name}.desktop"
    if not desktop.is_file():
        fail(f"missing installed desktop entry: {desktop}")
    validate_desktop_entry(desktop, product_name=product_name, main_binary=main_binary)
    validate_icon(root, main_binary=main_binary)
    validate_release_binaries(root, main_binary=main_binary)


def extract_appimage(appimage: Path, destination: Path) -> Path:
    appimage.chmod(appimage.stat().st_mode | 0o111)
    result = subprocess.run(
        [str(appimage), "--appimage-extract"],
        cwd=destination,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        fail(f"failed to extract {appimage}: {result.stderr.strip()}")
    root = destination / "squashfs-root"
    if not root.is_dir():
        fail(f"{appimage}: extraction succeeded but squashfs-root is missing")
    return root


def deb_field(deb: Path, field: str) -> str:
    return subprocess.check_output(
        ["dpkg-deb", "-f", str(deb), field], text=True
    ).strip()


def dependency_names(raw: str) -> set[str]:
    names: set[str] = set()
    for dependency in raw.split(","):
        token = dependency.strip().split(" ", 1)[0]
        if token:
            names.add(token)
    return names


def validate_deb_metadata(deb: Path, *, version: str) -> None:
    if deb_field(deb, "Package") != "silent-disco":
        fail(f"{deb}: unexpected Debian package name {deb_field(deb, 'Package')!r}")
    if deb_field(deb, "Version") != version:
        fail(f"{deb}: version does not match tauri.conf.json {version!r}")
    if deb_field(deb, "Architecture") != "amd64":
        fail(f"{deb}: Block 46 baseline expects amd64 package output")
    dependencies = dependency_names(deb_field(deb, "Depends"))
    missing = EXPECTED_DEB_DEPENDENCIES - dependencies
    if missing:
        fail(f"{deb}: missing direct runtime dependencies {sorted(missing)}; got {sorted(dependencies)}")


def cargo_default_run(cargo_manifest: Path) -> str:
    for raw_line in cargo_manifest.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line.startswith("default-run") and "=" in line:
            value = line.split("=", 1)[1].strip()
            if len(value) >= 2 and value[0] == value[-1] == '"':
                return value[1:-1]
    fail(f"{cargo_manifest}: missing simple quoted package default-run")


def validate_configuration(tauri_config: Path, cargo_manifest: Path) -> tuple[str, str, str]:
    config = json.loads(tauri_config.read_text(encoding="utf-8"))
    product_name = config["productName"]
    version = config["version"]
    main_binary = cargo_default_run(cargo_manifest)
    targets = config["bundle"]["targets"]
    if targets != ["appimage", "deb"]:
        fail(
            "initial Linux release formats must be exactly AppImage and .deb; "
            f"configured targets are {targets!r}"
        )
    configured_dependencies = set(
        config.get("bundle", {})
        .get("linux", {})
        .get("deb", {})
        .get("depends", [])
    )
    missing = EXPECTED_DEB_DEPENDENCIES - configured_dependencies
    if missing:
        fail(
            "tauri.conf.json must explicitly declare direct Linux runtime dependencies; "
            f"missing {sorted(missing)}"
        )
    return product_name, version, main_binary


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle-dir", required=True, type=Path)
    parser.add_argument("--tauri-config", required=True, type=Path)
    parser.add_argument("--cargo-manifest", required=True, type=Path)
    args = parser.parse_args()

    bundle_dir = args.bundle_dir.resolve()
    product_name, version, main_binary = validate_configuration(
        args.tauri_config.resolve(), args.cargo_manifest.resolve()
    )
    appimage = single_match(bundle_dir / "appimage", "*.AppImage", "AppImage")
    deb = single_match(bundle_dir / "deb", "*.deb", "Debian package")
    validate_deb_metadata(deb, version=version)

    with tempfile.TemporaryDirectory(prefix="silent-disco-appimage-") as temp_dir:
        app_root = extract_appimage(appimage, Path(temp_dir))
        validate_payload_root(
            app_root, product_name=product_name, main_binary=main_binary
        )

    with tempfile.TemporaryDirectory(prefix="silent-disco-deb-") as temp_dir:
        deb_root = Path(temp_dir) / "root"
        deb_root.mkdir()
        subprocess.run(
            ["dpkg-deb", "-x", str(deb), str(deb_root)],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        validate_payload_root(
            deb_root, product_name=product_name, main_binary=main_binary
        )

    print(
        "Linux bundle verification passed: AppImage + .deb only; desktop entry/icon valid; "
        "no Block 45 probes shipped; required runtime dependencies declared."
    )


if __name__ == "__main__":
    main()
