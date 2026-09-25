"""
Сборка выпуска: cargo build --release + установщик Inno Setup (dist\\Disk_Flashlight <версия> Setup.exe).

Запускается из любой папки: python tools/make_release.py [--no-tests]
Внешних зависимостей нет. Вывод cargo и ISCC показывается как есть, чтобы был виден ход сборки.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
ISS_FILE = ROOT / "tools" / "setup.iss"
DIST_DIR = ROOT / "dist"
EXE = ROOT / "target" / "release" / "disk_flashlight.exe"
ICON = ROOT / "target" / "app.ico"

# Файлы, которые установщик берёт из репозитория (см. [Files] в setup.iss).
BUNDLED_FILES = ["LICENSE", "README.md", "README.ru.md"]

ISCC_PATHS = [
    Path(R"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
    Path(R"C:\Program Files\Inno Setup 6\ISCC.exe"),
    Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "Inno Setup 6" / "ISCC.exe",
]


class ReleaseError(Exception):
    """Ошибка сборки с готовым для показа сообщением."""


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
    """Запускает команду, показывая её вывод как есть; при ошибке прерывает сборку."""
    print(f"$ {fmt_cmd(command)}\n")
    started = time.monotonic()
    result = subprocess.run(command, cwd=ROOT)
    elapsed = time.monotonic() - started
    if result.returncode != 0:
        raise ReleaseError(f"{title}: команда завершилась с кодом {result.returncode} (за {elapsed:.0f} с)")
    print(f"\n{title}: готово за {elapsed:.0f} с")


class Steps:
    """Печатает заголовки шагов и время, ушедшее на предыдущий."""

    def __init__(self, total: int):
        self.total = total
        self.number = 0
        self.started: float | None = None

    def _close(self) -> None:
        if self.started is not None:
            print(f"--- шаг занял {time.monotonic() - self.started:.1f} с")

    def next(self, title: str) -> None:
        self._close()
        self.number += 1
        print(f"\n=== [{self.number}/{self.total}] {title} ===")
        self.started = time.monotonic()

    def finish(self) -> None:
        self._close()
        self.started = None


def extract_version(path: Path) -> str:
    """Версия из секции [package] в Cargo.toml."""
    content = path.read_text(encoding="utf-8")
    package = re.search(r"^\[package\](.*?)(?=^\[|\Z)", content, re.S | re.M)
    match = package and re.search(r'^version\s*=\s*"([^"]+)"', package.group(1), re.M)
    if not match:
        raise ReleaseError(f"версия пакета не найдена в {path}")
    return match.group(1)


def windows_version(version: str) -> str:
    """0.1.0 -> 0.1.0.0: VersionInfoVersion требует четыре числа."""
    numbers = [int(n) for n in re.findall(r"\d+", version.split("-")[0])][:4]
    return ".".join(str(n) for n in numbers + [0] * (4 - len(numbers)))


def find_cargo() -> str:
    found = shutil.which("cargo")
    if found:
        return found
    # rustup ставился с --no-modify-path: в новых оболочках cargo может не быть в PATH.
    fallback = Path.home() / ".cargo" / "bin" / "cargo.exe"
    if fallback.is_file():
        return str(fallback)
    raise ReleaseError("не найден cargo: установите Rust (rustup) или добавьте ~/.cargo/bin в PATH")


def find_iscc() -> Path:
    for path in ISCC_PATHS:
        if path.is_file():
            return path
    found = shutil.which("ISCC")
    if found:
        return Path(found)
    raise ReleaseError("не найден ISCC.exe: установите Inno Setup 6 или добавьте ISCC в PATH")


def check_exe_not_running() -> None:
    """Запущенный exe заблокирован Windows, и cargo не сможет его перезаписать."""
    if not EXE.is_file():
        return
    try:
        with EXE.open("r+b"):
            pass
    except PermissionError:
        raise ReleaseError(f"{EXE.relative_to(ROOT)} запущен: закройте Disk Flashlight и повторите сборку")


def check_prerequisites() -> tuple[str, Path]:
    missing = [name for name in BUNDLED_FILES if not (ROOT / name).is_file()]
    if missing:
        raise ReleaseError("не найдены файлы для установщика: " + ", ".join(missing))
    cargo = find_cargo()
    iscc = find_iscc()
    print(f"  cargo:                 {cargo}")
    print(f"  Inno Setup:            {iscc}")
    check_exe_not_running()
    return cargo, iscc


def main() -> int:
    parser = argparse.ArgumentParser(description="Сборка выпуска Disk Flashlight")
    parser.add_argument("--no-tests", action="store_true", help="не запускать cargo test")
    args = parser.parse_args()

    # Построчная буферизация: при перенаправлении в файл заголовки шагов идут перед выводом сборщиков.
    sys.stdout.reconfigure(line_buffering=True)
    total_started = time.monotonic()
    steps = Steps(4 if args.no_tests else 5)
    try:
        steps.next("Проверка")
        version = extract_version(CARGO_TOML)
        print(f"  версия:                {version} (из {CARGO_TOML.name})")
        cargo, iscc = check_prerequisites()

        if not args.no_tests:
            steps.next("Тесты")
            run_command([cargo, "test"], "cargo test")

        steps.next("Сборка release")
        run_command([cargo, "build", "--release"], "cargo build")
        if not EXE.is_file():
            raise ReleaseError(f"cargo завершился, но {EXE} не найден")

        steps.next("Иконка установщика")
        run_command([str(EXE), "--export-icon", str(ICON)], "экспорт иконки")
        if not ICON.is_file():
            raise ReleaseError(f"иконка не создана: {ICON}")

        steps.next("Установщик Inno Setup")
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
        installer = DIST_DIR / f"Disk_Flashlight {version} Setup.exe"
        if not installer.is_file():
            raise ReleaseError(f"ISCC завершился, но установщик не найден: {installer}")
        steps.finish()

    except ReleaseError as e:
        print(f"\nОШИБКА: сборка прервана: {e}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nОШИБКА: сборка прервана пользователем", file=sys.stderr)
        return 1

    minutes, seconds = divmod(int(time.monotonic() - total_started), 60)
    print(f"\n=== Выпуск {version} собран за {minutes} мин {seconds} с ===")
    print(f"  программа:   {EXE}  ({human_size(EXE.stat().st_size)})")
    print(f"  установщик:  {installer}  ({human_size(installer.stat().st_size)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
