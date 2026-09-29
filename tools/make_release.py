"""
Release build: cargo build --release, the Inno Setup installer
(dist\\Disk_Flashlight_<version>_Setup.exe) and the portable archive
(dist\\Disk_Flashlight_<version>_portable.zip).

Runs from any folder: python tools/make_release.py [--no-tests]
No external dependencies. The output of cargo and ISCC is shown as is, so that
the progress of the build is visible.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
ISS_FILE = ROOT / "tools" / "setup.iss"
DIST_DIR = ROOT / "dist"
EXE = ROOT / "target" / "release" / "disk_flashlight.exe"
ICON = ROOT / "target" / "app.ico"

# Files the installer takes from the repository (see [Files] in setup.iss).
BUNDLED_FILES = ["LICENSE", "README.md", "README.ru.md"]
# Folder inside the portable archive, so that extracting "here" does not drop the exe among other files.
PORTABLE_DIR = "Disk Flashlight"
# Must match settings::PORTABLE_FILE in src/settings.rs.
PORTABLE_SETTINGS = "disk_flashlight.ron"

ISCC_PATHS = [
    Path(R"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
    Path(R"C:\Program Files\Inno Setup 6\ISCC.exe"),
    Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Inno Setup 6" / "ISCC.exe",
]


class ReleaseError(Exception):
    """Build failure with a message ready to be shown."""


def human_size(num_bytes: int) -> str:
    num = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB"):
        if num < 1024:
            return f"{num:.1f} {unit}" if unit != "B" else f"{int(num)} {unit}"
        num /= 1024
    return f"{num:.1f} TB"


def fmt_cmd(command: list[str]) -> str:
    return " ".join(f'"{c}"' if " " in c else c for c in command)


def run_command(command: list[str], title: str) -> None:
    """Run a command, showing its output as is; stop the build if it fails."""
    print(f"$ {fmt_cmd(command)}\n")
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT)
    elapsed = time.monotonic() - started
    if result.returncode != 0:
        raise ReleaseError(f"{title}: the command exited with code {result.returncode} (after {elapsed:.0f} s)")
    print(f"\n{title}: done in {elapsed:.0f} s")


class Steps:
    """Print step headers and the time the previous step took."""

    def __init__(self, total: int):
        self.total = total
        self.number = 0
        self.started: float | None = None

    def _close(self) -> None:
        if self.started is not None:
            print(f"--- step took {time.monotonic() - self.started:.1f} s")

    def next(self, title: str) -> None:
        self._close()
        self.number += 1
        print(f"\n=== [{self.number}/{self.total}] {title} ===")
        self.started = time.monotonic()

    def finish(self) -> None:
        self._close()
        self.started = None


def extract_version(path: Path) -> str:
    """Version from the [package] section of Cargo.toml."""
    content = path.read_text(encoding="utf-8")
    package = re.search(r"^\[package\](.*?)(?=^\[|\Z)", content, re.S | re.M)
    match = package and re.search(r'^version\s*=\s*"([^"]+)"', package.group(1), re.M)
    if not match:
        raise ReleaseError(f"package version not found in {path}")
    return match.group(1)


def windows_version(version: str) -> str:
    """0.1.0 -> 0.1.0.0: VersionInfoVersion needs four numbers."""
    numbers = [int(n) for n in re.findall(r"\d+", version.split("-")[0])][:4]
    return ".".join(str(n) for n in numbers + [0] * (4 - len(numbers)))


def find_cargo() -> str:
    found = shutil.which("cargo")
    if found:
        return found
    # rustup installed with --no-modify-path leaves cargo off PATH in new shells.
    fallback = Path.home() / ".cargo" / "bin" / "cargo.exe"
    if fallback.is_file():
        return str(fallback)
    raise ReleaseError("cargo not found: install Rust (rustup) or add ~/.cargo/bin to PATH")


def find_iscc() -> Path:
    for path in ISCC_PATHS:
        if path.is_file():
            return path
    found = shutil.which("ISCC")
    if found:
        return Path(found)
    raise ReleaseError("ISCC.exe not found: install Inno Setup 6 or add ISCC to PATH")


def check_exe_not_running() -> None:
    """A running exe is locked by Windows, and cargo could not overwrite it."""
    if not EXE.is_file():
        return
    try:
        with EXE.open("r+b"):
            pass
    except PermissionError:
        raise ReleaseError(f"{EXE.relative_to(ROOT)} is running: close Disk Flashlight and build again")


def check_prerequisites() -> tuple[str, Path]:
    missing = [name for name in BUNDLED_FILES if not (ROOT / name).is_file()]
    if missing:
        raise ReleaseError("files for the installer not found: " + ", ".join(missing))
    cargo = find_cargo()
    iscc = find_iscc()
    print(f"  cargo:                 {cargo}")
    print(f"  Inno Setup:            {iscc}")
    check_exe_not_running()
    return cargo, iscc


def make_portable_zip(version: str) -> Path:
    """Archive with the exe and an empty settings file in the PORTABLE_DIR folder (the documentation is on GitHub).

    The settings file next to the exe (settings::PORTABLE_FILE) makes the copy keep its settings there
    instead of in %APPDATA%."""
    archive = DIST_DIR / f"Disk_Flashlight_{version}_portable.zip"
    # Earlier builds made a portable exe, which is no longer released: remove it so that it is not uploaded.
    stale = DIST_DIR / f"Disk_Flashlight_{version}_portable.exe"
    stale.unlink(missing_ok=True)
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.write(EXE, f"{PORTABLE_DIR}/{EXE.name}")
        z.writestr(f"{PORTABLE_DIR}/{PORTABLE_SETTINGS}", "")
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description="Build a Disk Flashlight release")
    parser.add_argument("--no-tests", action="store_true", help="do not run cargo test")
    args = parser.parse_args()

    # Line buffering: with output redirected to a file, step headers still come before the tools' output.
    sys.stdout.reconfigure(line_buffering=True)
    total_started = time.monotonic()
    steps = Steps(4 if args.no_tests else 5)
    try:
        steps.next("Checks")
        version = extract_version(CARGO_TOML)
        print(f"  version:               {version} (from {CARGO_TOML.name})")
        cargo, iscc = check_prerequisites()

        if not args.no_tests:
            steps.next("Tests")
            run_command([cargo, "test"], "cargo test")

        steps.next("Release build")
        run_command([cargo, "build", "--release"], "cargo build")
        if not EXE.is_file():
            raise ReleaseError(f"cargo finished, but {EXE} was not found")

        steps.next("Installer icon")
        run_command([str(EXE), "--export-icon", str(ICON)], "icon export")
        if not ICON.is_file():
            raise ReleaseError(f"icon not created: {ICON}")

        steps.next("Inno Setup installer")
        run_command(
            [
                str(iscc),
                f"/DMyAppVersion={version}",
                f"/DVersionInfoVersion={windows_version(version)}",
                f"/DAppIcon={ICON}",
                str(ISS_FILE),
            ],
            "ISCC",
        )
        installer = DIST_DIR / f"Disk_Flashlight_{version}_Setup.exe"
        if not installer.is_file():
            raise ReleaseError(f"ISCC finished, but the installer was not found: {installer}")
        # Names without spaces: GitHub replaces spaces in release file names with dots.
        portable = make_portable_zip(version)
        print(f"Portable version: {portable.relative_to(ROOT)}")
        steps.finish()

    except ReleaseError as e:
        print(f"\nERROR: build stopped: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nERROR: build interrupted by the user", file=sys.stderr)
        return 1

    minutes, seconds = divmod(int(time.monotonic() - total_started), 60)
    print(f"\n=== Release {version} built in {minutes} min {seconds} s ===")
    print(f"  installer:   {installer}  ({human_size(installer.stat().st_size)})")
    print(f"  portable:    {portable}  ({human_size(portable.stat().st_size)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
