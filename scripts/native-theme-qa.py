#!/usr/bin/env python3
"""Check the release UI's palette on a private display without changing desktop config."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib

from PIL import Image


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(DISPLAY=':80', WAYLAND_DISPLAY='', WINIT_UNIX_BACKEND='x11', XDG_SESSION_TYPE='x11')
    results = []
    app = None
    with (args.out / 'xvfb.log').open('w') as xvfb_log, (args.out / 'glyph.log').open('w') as app_log:
        server = subprocess.Popen(['Xvfb', ':80', '-screen', '0', '1600x1000x24', '-nolisten', 'tcp'], env=env, stdout=xvfb_log, stderr=subprocess.STDOUT)
        try:
            deadline = time.monotonic() + 10
            while subprocess.run(['xdotool', 'getdisplaygeometry'], env=env, capture_output=True).returncode:
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('isolated X server failed')
                time.sleep(.05)

            def wait_surface(name, hex_color):
                expected = tuple(bytes.fromhex(hex_color.lstrip('#')))
                deadline = time.monotonic() + 10
                path = args.out / f'{name}.png'
                pixel = None
                assert app is not None
                while time.monotonic() < deadline:
                    if app.poll() is not None:
                        raise RuntimeError('Glyph exited unexpectedly')
                    subprocess.run(['ffmpeg', '-v', 'error', '-y', '-f', 'x11grab', '-video_size', '1600x1000', '-i', ':80', '-frames:v', '1', str(path)], env=env, check=True, capture_output=True)
                    pixel = Image.open(path).convert('RGB').getpixel((400, 5))
                    if pixel == expected:
                        results.append({'action': name, 'passed': True, 'surface_rgb': pixel})
                        return
                    time.sleep(.1)
                raise AssertionError(f'{name}: expected {expected}, observed {pixel}')

            state = Path(os.environ.get('XDG_STATE_HOME') or Path.home() / '.local/state')
            active = state / 'omarchy/current/theme/colors.toml'
            active_palette = tomllib.loads(active.read_text())
            app = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=app_log, stderr=subprocess.STDOUT)
            wait_surface('01-installed-omarchy-theme', active_palette['background'])
            app.terminate()
            app.wait(timeout=10)
            app = None
            with tempfile.TemporaryDirectory(prefix='glyph-theme-qa-') as temporary:
                path = Path(temporary) / 'omarchy/current/theme/colors.toml'
                path.parent.mkdir(parents=True)
                dark = 'mode = "dark"\nbackground = "#101820"\nforeground = "#eef0f2"\naccent = "#ab8030"\n'
                light = 'mode = "light"\nbackground = "#faf8f0"\nforeground = "#182020"\naccent = "#805000"\n'
                path.write_text(dark)
                test_env = env.copy()
                test_env['XDG_STATE_HOME'] = temporary
                app = subprocess.Popen([str(args.binary.resolve())], env=test_env, stdout=app_log, stderr=subprocess.STDOUT)
                wait_surface('02-private-dark-theme', '#101820')
                path.write_text(light)
                wait_surface('03-live-switch-light-theme', '#faf8f0')
                path.write_text('malformed theme')
                wait_surface('04-malformed-fallback', '#07090c')
                path.write_text(dark)
                wait_surface('05-live-theme-recovery', '#101820')
        finally:
            for process in (app, server):
                if process is not None and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=10)
    report = {'passed': True, 'checks': results, 'artifacts': str(args.out.resolve())}
    (args.out / 'results.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
