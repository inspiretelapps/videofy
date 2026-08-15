#!/usr/bin/env python3
"""Copy Homebrew ffmpeg/ffprobe plus their dylibs into src-tauri/resources/ffbin.

Finder-launched apps do not see Homebrew's PATH, so the .app has to carry
its own binaries. Output is gitignored and rebuilt from the local Cellar.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "src-tauri" / "resources" / "ffbin"
STAMP = DEST / ".stamp"


def which(name: str) -> Path | None:
    found = shutil.which(name)
    return Path(found).resolve() if found else None


def otool_install_names(path: Path) -> list[str]:
    out = subprocess.check_output(["otool", "-L", str(path)], text=True)
    names: list[str] = []
    for line in out.splitlines()[1:]:
        raw = line.strip().split(" (", 1)[0].strip()
        if raw:
            names.append(raw)
    return names


def otool_rpaths(path: Path) -> list[Path]:
    out = subprocess.check_output(["otool", "-l", str(path)], text=True)
    rpaths: list[Path] = []
    lines = out.splitlines()
    for i, line in enumerate(lines):
        if "LC_RPATH" not in line:
            continue
        for follow in lines[i + 1 : i + 6]:
            follow = follow.strip()
            if follow.startswith("path "):
                raw = follow.split("path ", 1)[1].split(" (", 1)[0].strip()
                if raw.startswith("@loader_path"):
                    rpaths.append((path.parent / raw.replace("@loader_path", ".")).resolve())
                elif raw.startswith("@executable_path"):
                    continue
                else:
                    rpaths.append(Path(raw))
                break
    return rpaths


def resolve_dep(name: str, loader: Path) -> Path | None:
    if name.startswith("/opt/homebrew/") or name.startswith("/usr/local/"):
        path = Path(name)
        return path.resolve() if path.exists() else None
    if name.startswith("@loader_path/"):
        path = (loader.parent / name[len("@loader_path/") :]).resolve()
        return path if path.exists() else None
    if name.startswith("@rpath/"):
        rest = name[len("@rpath/") :]
        for rpath in otool_rpaths(loader):
            path = (rpath / rest).resolve()
            if path.exists():
                return path
            # Homebrew often stores versioned files next to a shorter symlink.
            sibling = rpath / Path(rest).name
            if sibling.exists():
                return sibling.resolve()
    return None


def collect(binaries: list[Path]) -> dict[str, Path]:
    """Map destination basename -> resolved source file."""
    mapping: dict[str, Path] = {}
    queue: list[Path] = list(binaries)
    seen_files: set[Path] = set()

    while queue:
        current = queue.pop()
        if current in seen_files or not current.exists():
            continue
        seen_files.add(current)
        for name in otool_install_names(current):
            src = resolve_dep(name, current)
            if src is None:
                continue
            dest_name = Path(name).name
            existing = mapping.get(dest_name)
            if existing and existing != src:
                raise SystemExit(
                    f"basename collision for {dest_name}: {existing} vs {src}"
                )
            mapping[dest_name] = src
            if src not in seen_files:
                queue.append(src)
    return mapping


def rewrite(path: Path, bundled_names: set[str]) -> None:
    os.chmod(path, 0o755)
    subprocess.run(
        ["codesign", "--remove-signature", str(path)],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if path.suffix == ".dylib" or ".dylib" in path.name:
        subprocess.run(
            ["install_name_tool", "-id", f"@loader_path/{path.name}", str(path)],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    for name in otool_install_names(path):
        dest_name = Path(name).name
        if dest_name not in bundled_names:
            continue
        new = f"@loader_path/{dest_name}"
        if name == new:
            continue
        subprocess.run(
            ["install_name_tool", "-change", name, new, str(path)],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    subprocess.run(
        ["install_name_tool", "-add_rpath", "@loader_path", str(path)],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    subprocess.check_call(
        ["codesign", "--force", "--sign", "-", str(path)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def stamp_value(ffmpeg: Path, ffprobe: Path) -> str:
    def token(path: Path) -> str:
        st = path.stat()
        return f"{path}:{st.st_mtime_ns}:{st.st_size}"

    return f"{token(ffmpeg)}\n{token(ffprobe)}\n"


def main() -> int:
    ffmpeg = which("ffmpeg")
    ffprobe = which("ffprobe")
    if ffmpeg is None or ffprobe is None:
        print("ffmpeg/ffprobe not found on PATH", file=sys.stderr)
        return 1

    wanted = stamp_value(ffmpeg, ffprobe)
    if STAMP.is_file() and STAMP.read_text() == wanted and (DEST / "ffmpeg").is_file():
        print(f"ffmpeg bundle up to date at {DEST}")
        return 0

    mapping = collect([ffmpeg, ffprobe])
    if DEST.exists():
        shutil.rmtree(DEST)
    DEST.mkdir(parents=True)

    shutil.copy2(ffmpeg, DEST / "ffmpeg")
    shutil.copy2(ffprobe, DEST / "ffprobe")
    for dest_name, src in mapping.items():
        shutil.copy2(src, DEST / dest_name)

    bundled_names = set(mapping)
    for dest_name in ["ffmpeg", "ffprobe", *mapping]:
        rewrite(DEST / dest_name, bundled_names)

    version = subprocess.check_output(
        [str(DEST / "ffmpeg"), "-version"],
        text=True,
    ).splitlines()[0]
    probe = subprocess.check_output(
        [str(DEST / "ffprobe"), "-version"],
        text=True,
    ).splitlines()[0]
    STAMP.write_text(wanted)
    print(f"bundled {len(bundled_names) + 2} files into {DEST}")
    print(version)
    print(probe)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
