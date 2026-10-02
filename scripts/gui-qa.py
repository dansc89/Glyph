#!/usr/bin/env python3
"""Glyph GUI smoke/UX harness.

Runs the real Glyph binary under Xvfb, opens a generated multi-page PDF,
performs basic user interactions, and captures screenshots for review.

Dependencies: Xvfb, xdotool, ffmpeg.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

SCREEN_W = 1600
SCREEN_H = 1000
DISPLAY = ":77"


def require(cmd: str) -> str:
    path = shutil.which(cmd)
    if not path:
        raise SystemExit(f"missing required command: {cmd}")
    return path


def make_colored_pdf(out: Path) -> None:
    """Create a simple 3-page PDF with distinct page colors/shapes."""
    objs: list[bytes] = []

    def add(obj: str | bytes) -> None:
        objs.append(obj.encode("latin1") if isinstance(obj, str) else obj)

    add("<< /Type /Catalog /Pages 2 0 R >>")
    add("<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>")
    for i in range(3):
        add(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            f"/Contents {6 + i} 0 R >>"
        )

    # Strong shapes instead of relying on font availability in PDFium builds.
    colors = [
        ("0.55 0.20 0.95", "72 600 468 95", "PAGE 1: purple header"),
        ("0.10 0.65 0.30", "72 430 468 130", "PAGE 2: green middle band"),
        ("0.10 0.35 0.90", "72 170 468 180", "PAGE 3: blue lower block"),
    ]
    for color, rect, label in colors:
        # Include a gray border and a distinctive filled rectangle.
        stream = (
            "q 0.92 0.92 0.92 rg 36 36 540 720 re f Q\n"
            "q 0.10 0.10 0.10 RG 2 w 36 36 540 720 re S Q\n"
            f"q {color} rg {rect} re f Q\n"
            "q 0.20 0.20 0.20 rg 86 86 440 42 re f Q\n"
            f"% {label}\n"
        ).encode("latin1")
        add(b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream")

    buf = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n"
    offsets: list[int] = []
    for idx, obj in enumerate(objs, 1):
        offsets.append(len(buf))
        buf += f"{idx} 0 obj\n".encode() + obj + b"\nendobj\n"
    xref = len(buf)
    buf += f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode()
    for off in offsets:
        buf += f"{off:010d} 00000 n \n".encode()
    buf += f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    out.write_bytes(buf)


def run(cmd: list[str], *, env: dict[str, str] | None = None, cwd: str | None = None, **kwargs) -> subprocess.CompletedProcess:
    print("$", " ".join(cmd), flush=True)
    return subprocess.run(cmd, env=env, cwd=cwd, check=True, text=True, **kwargs)


def capture(ffmpeg: str, name: str, out_dir: Path) -> Path:
    dest = out_dir / f"{name}.png"
    run(
        [
            ffmpeg,
            "-y",
            "-f",
            "x11grab",
            "-video_size",
            f"{SCREEN_W}x{SCREEN_H}",
            "-i",
            DISPLAY,
            "-frames:v",
            "1",
            str(dest),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return dest


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="target/release/glyph", help="Glyph binary to test")
    parser.add_argument("--out", default="/srv/apps/_agent-artifacts/glyph-gui-qa/latest")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()

    xvfb = require("Xvfb")
    xdotool = require("xdotool")
    ffmpeg = shutil.which("ffmpeg") or "/home/ubuntu/.hermes/tools/ffmpeg-9.0.1-linux-arm64/bin/ffmpeg"
    if not Path(ffmpeg).exists():
        raise SystemExit("missing required command: ffmpeg")

    repo = Path(__file__).resolve().parents[1]
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    pdf = out_dir / "glyph-qa-3page.pdf"
    make_colored_pdf(pdf)

    if not args.skip_build:
        run(["cargo", "build", "--locked", "--release"], cwd=str(repo))

    binary = Path(args.binary)
    if not binary.is_absolute():
        binary = repo / binary
    if not binary.exists():
        raise SystemExit(f"Glyph binary not found: {binary}")

    env = os.environ.copy()
    env["DISPLAY"] = DISPLAY
    env.setdefault("RUST_BACKTRACE", "1")

    procs: list[subprocess.Popen] = []
    try:
        xvfb_log = (out_dir / "xvfb.log").open("wb")
        xproc = subprocess.Popen([xvfb, DISPLAY, "-screen", "0", f"{SCREEN_W}x{SCREEN_H}x24"], stdout=xvfb_log, stderr=subprocess.STDOUT)
        procs.append(xproc)
        time.sleep(1.0)
        if xproc.poll() is not None:
            raise RuntimeError(f"Xvfb exited early with {xproc.returncode}")

        app_log = (out_dir / "glyph.log").open("wb")
        app = subprocess.Popen([str(binary), str(pdf)], env=env, stdout=app_log, stderr=subprocess.STDOUT)
        procs.append(app)
        time.sleep(5.0)
        if app.poll() is not None:
            raise RuntimeError(f"Glyph exited early with {app.returncode}; see {out_dir / 'glyph.log'}")

        geom = subprocess.run(
            [xdotool, "search", "--name", "Glyph", "getwindowgeometry", "--shell"],
            env=env,
            text=True,
            capture_output=True,
            check=True,
        )
        (out_dir / "window-geometry.txt").write_text(geom.stdout)

        shots: list[Path] = []
        shots.append(capture(ffmpeg, "01-open-page-1", out_dir))

        # Coordinates target the fixed 1440x920 default window inside a 1600x1000 Xvfb screen.
        run([xdotool, "mousemove", "72", "472", "click", "1"], env=env)
        time.sleep(1.0)
        shots.append(capture(ffmpeg, "02-click-page-2", out_dir))

        run([xdotool, "mousemove", "72", "512", "click", "1"], env=env)
        time.sleep(1.0)
        shots.append(capture(ffmpeg, "03-click-page-3", out_dir))

        # Cursor-anchored zoom in the document canvas.
        run([xdotool, "mousemove", "800", "500", "click", "4", "click", "4", "click", "4"], env=env)
        time.sleep(1.0)
        shots.append(capture(ffmpeg, "04-zoom-at-cursor", out_dir))

        # Drag pan on the canvas.
        run([xdotool, "mousemove", "800", "500", "mousedown", "1", "mousemove_relative", "--sync", "160", "70", "mouseup", "1"], env=env)
        time.sleep(1.0)
        shots.append(capture(ffmpeg, "05-drag-pan", out_dir))

        summary = out_dir / "summary.txt"
        summary.write_text(
            "Glyph GUI QA completed.\n"
            f"Binary: {binary}\n"
            f"PDF: {pdf}\n"
            f"Geometry:\n{geom.stdout}\n"
            "Screenshots:\n" + "\n".join(str(p) for p in shots) + "\n"
        )
        print(summary.read_text())
        return 0
    finally:
        for proc in reversed(procs):
            if proc.poll() is None:
                proc.terminate()
        time.sleep(1.0)
        for proc in reversed(procs):
            if proc.poll() is None:
                proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
