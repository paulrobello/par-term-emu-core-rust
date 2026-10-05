#!/usr/bin/env python3
"""Package the standalone par-mux release binaries (deployment.yml build-mux-binaries).

Stages a built ``par-mux`` executable together with ``LICENSE`` and a generated
short install note, archives it as ``par-mux-v<version>-<target>.tar.gz`` (Unix
targets) or ``.zip`` (Windows targets), and writes a ``sha256sum -c``-compatible
per-target checksum file. The github-release job concatenates the per-target
checksum files into the release-level ``par-mux-v<version>-SHA256SUMS.txt``.

Pure python3 stdlib so it runs identically on every CI host. Tar members get
normalized mtimes/ownership for reproducible archives; the executable bit is
preserved (tar stores the on-disk mode; zip entries carry it in their external
attributes so Unix unzip tools restore it).

With ``--self-test`` it builds a fake binary in a temp tree and verifies
archive layout, exec-bit preservation, and checksum integrity — same shape as
the mux/FFI docs gates.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

BIN_NAME = "par-mux"
LICENSE_NAME = "LICENSE"
INSTALL_NOTE_NAME = "INSTALL.md"

WINDOWS_TARGET_SUFFIX = "-windows-msvc"


def archive_name(version: str, target: str) -> str:
    ext = "zip" if target.endswith(WINDOWS_TARGET_SUFFIX) else "tar.gz"
    return f"{BIN_NAME}-v{version}-{target}.{ext}"


def checksum_name(version: str, target: str) -> str:
    return f"{BIN_NAME}-v{version}-{target}.sha256"


def install_note(version: str, target: str, windows: bool) -> str:
    name = f"{BIN_NAME}.exe" if windows else BIN_NAME
    if windows:
        lines = [
            f"par-mux v{version} ({target}) — terminal multiplexer daemon and client.",
            "",
            "Install:",
            f"1. Extract the archive; it contains {name}, LICENSE, and this note.",
            "2. Create a directory for the executable, e.g. %LOCALAPPDATA%\\Programs\\par-mux,",
            f"   and copy {name} into it.",
            "3. Add that directory to your PATH (System Properties > Environment Variables),",
            "   then open a new terminal and run: par-mux --version",
            "",
            "Verify the download (in a terminal with the archive and checksum file):",
            "  certutil -hashfile <archive> SHA256",
        ]
    else:
        lines = [
            f"par-mux v{version} ({target}) — terminal multiplexer daemon and client.",
            "",
            "Install:",
            f"1. Extract the archive: tar -xzf {BIN_NAME}-v{version}-{target}.tar.gz",
            "2. Copy the executable onto your PATH:",
            f"   install -m 755 {BIN_NAME}-v{version}-{target}/{name} ~/.local/bin/{name}",
            "3. Ensure ~/.local/bin is on PATH, then run: par-mux --version",
            "",
            "Verify the download against the release checksum file:",
            f"  sha256sum -c par-mux-v{version}-SHA256SUMS.txt",
        ]
    lines += [
        "",
        "The executable is standalone: no Python and no par-term installation required.",
        "It includes the `attach` client. Docs: https://github.com/paulrobello/par-term-emu-core-rust",
    ]
    return "\n".join(lines) + "\n"


def package(
    bin_path: Path,
    version: str,
    target: str,
    output_dir: Path,
    license_path: Path,
) -> list[Path]:
    """Stage and archive one binary. Returns the created archive + checksum paths."""
    if not bin_path.is_file():
        raise SystemExit(f"ERROR: binary not found: {bin_path}")
    if not license_path.is_file():
        raise SystemExit(f"ERROR: LICENSE not found: {license_path}")

    windows = target.endswith(WINDOWS_TARGET_SUFFIX)
    top = f"{BIN_NAME}-v{version}-{target}"
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mux-pkg-") as tmp:
        stage = Path(tmp) / top
        stage.mkdir()
        staged_bin = stage / (f"{BIN_NAME}.exe" if windows else BIN_NAME)
        staged_bin.write_bytes(bin_path.read_bytes())
        staged_bin.chmod(0o755)
        (stage / LICENSE_NAME).write_bytes(license_path.read_bytes())
        (stage / INSTALL_NOTE_NAME).write_text(
            install_note(version, target, windows), encoding="utf-8"
        )

        members = [staged_bin, stage / LICENSE_NAME, stage / INSTALL_NOTE_NAME]
        modes = [0o755, 0o644, 0o644]
        archive = output_dir / archive_name(version, target)
        if windows:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
                for member, mode in zip(members, modes):
                    info = zipfile.ZipInfo(
                        f"{top}/{member.name}", date_time=(1980, 1, 1, 0, 0, 0)
                    )
                    info.external_attr = mode << 16
                    info.compress_type = zipfile.ZIP_DEFLATED
                    zf.writestr(info, member.read_bytes())
        else:

            def tar_info(member: Path, mode: int) -> tarfile.TarInfo:
                info = tarfile.TarInfo(f"{top}/{member.name}")
                info.size = member.stat().st_size
                info.mode = mode
                info.mtime = 0
                info.uid = 0
                info.gid = 0
                info.uname = ""
                info.gname = ""
                info.type = tarfile.REGTYPE
                return info

            with tarfile.open(archive, "w:gz") as tf:
                for member, mode in zip(members, modes):
                    tf.addfile(tar_info(member, mode), member.open("rb"))

    checksum = output_dir / checksum_name(version, target)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum.write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(f"created {archive}")
    print(f"created {checksum}")
    return [archive, checksum]


def self_test() -> int:
    failures: list[str] = []

    def check(cond: bool, what: str) -> None:
        if not cond:
            failures.append(what)

    with tempfile.TemporaryDirectory(prefix="mux-pkg-test-") as tmp:
        root = Path(tmp)
        fake_bin = root / "par-mux-fake"
        fake_bin.write_bytes(b"#!/bin/sh\necho fake-par-mux\n")
        fake_bin.chmod(0o755)
        fake_license = root / "LICENSE"
        fake_license.write_text("MIT\n")
        out = root / "out"
        version = "9.9.9"

        for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
            windows = target.endswith(WINDOWS_TARGET_SUFFIX)
            created = package(fake_bin, version, target, out, fake_license)
            check(len(created) == 2, f"{target}: expected 2 artifacts")
            archive = out / archive_name(version, target)
            check(archive.is_file(), f"{target}: archive missing")

            top = f"{BIN_NAME}-v{version}-{target}"
            bin_member = f"{top}/{BIN_NAME}.exe" if windows else f"{top}/{BIN_NAME}"
            if windows:
                with zipfile.ZipFile(archive) as zf:
                    names = zf.namelist()
                    check(bin_member in names, f"{target}: {bin_member} not in zip")
                    check(f"{top}/LICENSE" in names, f"{target}: LICENSE not in zip")
                    check(
                        f"{top}/{INSTALL_NOTE_NAME}" in names,
                        f"{target}: INSTALL.md not in zip",
                    )
                    check(
                        zf.getinfo(bin_member).external_attr >> 16 == 0o755,
                        f"{target}: zip exec bit not 0755",
                    )
            else:
                with tarfile.open(archive) as tf:
                    infos = {ti.name: ti for ti in tf.getmembers()}
                    check(bin_member in infos, f"{target}: {bin_member} not in tar")
                    check(f"{top}/LICENSE" in infos, f"{target}: LICENSE not in tar")
                    check(
                        f"{top}/{INSTALL_NOTE_NAME}" in infos,
                        f"{target}: INSTALL.md not in tar",
                    )
                    check(
                        infos[bin_member].mode == 0o755,
                        f"{target}: tar exec mode not 0755",
                    )
                    check(
                        all(ti.mtime == 0 for ti in infos.values()),
                        f"{target}: mtimes not normalized",
                    )

            checksum = out / checksum_name(version, target)
            text = checksum.read_text(encoding="utf-8")
            expected = (
                f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n"
            )
            check(text == expected, f"{target}: checksum file mismatch")

        # A missing binary must fail loudly.
        try:
            package(
                root / "nope", version, "x86_64-unknown-linux-gnu", out, fake_license
            )
            failures.append("missing binary: package() did not exit")
        except SystemExit:
            pass

    if failures:
        for f in failures:
            print(f"SELF-TEST FAIL: {f}", file=sys.stderr)
        print(f"self-test: {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print("self-test: all packaging checks passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Package the standalone par-mux release binaries."
    )
    parser.add_argument("--bin", type=Path, help="path to the built par-mux executable")
    parser.add_argument("--version", help="release version, e.g. 0.58.1")
    parser.add_argument(
        "--target", help="Rust target triple, e.g. aarch64-apple-darwin"
    )
    parser.add_argument("--output-dir", type=Path, default=Path("mux-release"))
    parser.add_argument(
        "--license",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "LICENSE",
        help="license file to ship inside the archive",
    )
    parser.add_argument(
        "--self-test", action="store_true", help="run the built-in checks"
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()
    if not (args.bin and args.version and args.target):
        parser.error("--bin, --version and --target are required without --self-test")
    package(args.bin, args.version, args.target, args.output_dir, args.license)
    return 0


if __name__ == "__main__":
    sys.exit(main())
