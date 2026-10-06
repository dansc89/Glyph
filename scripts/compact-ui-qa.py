#!/usr/bin/env python3
"""Native compact-chrome regression; owned Xvfb, generated PDFs, no installed app.

Usage: uv run --with pillow python scripts/compact-ui-qa.py --binary ... --out ...
A before screenshot can be supplied to measure viewport gain at identical size.
All interaction is routed only to the display allocated by PrivateXvfb.
"""
import argparse
import importlib.util
import json
import subprocess
import time
from pathlib import Path

from PIL import Image, ImageOps

BORDER = (79, 88, 104)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def longest_run(values):
    best = (0, 0)
    start = None
    for index, value in enumerate(values + [False]):
        if value and start is None:
            start = index
        elif not value and start is not None:
            if index - start > best[1] - best[0]:
                best = (start, index)
            start = None
    return best


def image_pixels(image):
    getter = getattr(image, 'get_flattened_data', None) or image.getdata
    return getter()


def geometry(path, width, height):
    """Find the actual outlined canvas, not white page pixels or text estimates."""
    image = Image.open(path).convert('RGB').crop((0, 0, width, height))
    pixels = image.load()
    assert pixels is not None, 'Screenshot pixels unavailable'
    ranked = sorted(((sum(pixels[x, y] == BORDER for y in range(50, height - 50)), x)
                     for x in range(100, width - 5)), reverse=True)
    columns = []
    for count, x in ranked:
        if count < height // 3:
            break
        if all(abs(x - other) > 4 for other in columns):
            columns.append(x)
        if len(columns) == 3:
            break
    if len(columns) != 3:
        raise AssertionError(f'Expected sidebar and two canvas borders: {ranked[:10]}')
    sidebar, left, right = sorted(columns)
    rows = []
    for y in range(40, height - 2):
        start, end = longest_run([pixels[x, y] == BORDER for x in range(width)])
        # Rounded canvas corners inset the first/last horizontal border by 6px.
        # Full-width header/footer lines are deliberately excluded.
        if left + 2 <= start <= left + 10 and right - 10 <= end <= right:
            rows.append(y)
    if len(rows) < 2:
        raise AssertionError(f'Canvas outline not found between {left}, {right}: {rows}')
    top, bottom = min(rows), max(rows)
    assert bottom - top > height // 3
    return {'sidebar_right_px': sidebar + 1, 'canvas': [left, top, right + 1, bottom + 1],
            'canvas_area_px': (right + 1 - left) * (bottom + 1 - top),
            'canvas_fraction': (right + 1 - left) * (bottom + 1 - top) / (width * height)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--exercise-window-reposition', action='store_true',
                        help='Exercise capture recovery from an off-screen owned test window')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    fixture = load('compact_fixture', Path(__file__).with_name('gui-qa.py'))
    isolation = load('compact_isolation', Path(__file__).with_name('editing-qa.py'))
    pdf = out / 'compact-generated.pdf'
    fixture.make_colored_pdf(pdf)
    original = pdf.read_bytes()
    display = isolation.PrivateXvfb()
    # Use a deterministic fallback palette, not the user's current desktop theme.
    display.env['XDG_STATE_HOME'] = str(out / 'isolated-state')
    checks, images, measurements, apps = [], [], {}, []
    window = None
    viewport = [960, 640]

    def check(name, value):
        checks.append({'name': name, 'passed': bool(value)})
        assert value, name

    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, text=True, check=True,
                              capture_output=True, timeout=20).stdout

    def key(keys):
        run('xdotool', 'key', '--clearmodifiers', keys)

    def shot(name):
        # Winit can issue a delayed configure/move after xdotool resizes a
        # window on unmanaged Xvfb. Confirm size AND origin before capturing.
        expected = {'X': 0, 'Y': 0, 'WIDTH': viewport[0], 'HEIGHT': viewport[1]}
        deadline, stable = time.monotonic() + 10, 0
        while time.monotonic() < deadline:
            observed = dict(line.split('=', 1) for line in
                            run('xdotool', 'getwindowgeometry', '--shell', window).splitlines())
            if all(int(observed[k]) == v for k, v in expected.items()):
                stable += 1
                if stable >= 3:
                    break
            else:
                stable = 0
                if any(int(observed[k]) != expected[k] for k in ('WIDTH', 'HEIGHT')):
                    run('xdotool', 'windowsize', '--sync', window,
                        str(viewport[0]), str(viewport[1]))
                run('xdotool', 'windowmove', '--sync', window, '0', '0')
            time.sleep(.05)
        else:
            raise AssertionError(f'Owned window did not settle at {expected}: {observed}')
        path = out / (name + '.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size',
            '1600x1000', '-draw_mouse', '0', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(path))
        images.append(str(path))
        return path

    def ocr(path, width=960, height=640, crop_box=None):
        crop = Image.open(path).crop(crop_box or (0, 0, width, height))
        processed = out / (path.stem + '-ocr.png')
        ImageOps.autocontrast(crop.convert('L')).resize((crop.width * 2, crop.height * 2)).save(processed)
        return run('tesseract', str(processed), 'stdout', '--psm', '11').lower()

    def verify_page_fit(path, width, height, measured):
        image = Image.open(path).convert('RGB')
        left = measured['sidebar_right_px'] + 1
        # Scan the whole document pane, not a canvas-clipped crop; white paper
        # must have a margin on ALL four sides after Fit page.
        crop = image.crop((left, 0, width, height))
        mask = Image.new('L', crop.size)
        mask.putdata([255 if min(p) > 245 else 0 for p in image_pixels(crop)])
        bbox = mask.getbbox()
        assert bbox is not None, 'No actual rendered paper geometry'
        paper = [bbox[0] + left, bbox[1], bbox[2] + left, bbox[3]]
        canvas = measured['canvas']
        measured['rendered_page'] = paper
        check(f'actual rendered page fits the {width}x{height} canvas with margins',
              paper[2] - paper[0] > 100 and paper[3] - paper[1] > 100
              and paper[0] > canvas[0] + 8 and paper[1] > canvas[1] + 8
              and paper[2] < canvas[2] - 8 and paper[3] < canvas[3] - 8)

    def launch(path=None):
        viewport[:] = [960, 640]
        log = open(out / f'app-{len(apps)}.log', 'w')
        app = subprocess.Popen([str(args.binary.resolve())] + ([str(path)] if path else []),
                               env=display.env, stdout=log, stderr=log)
        apps.append((app, log))
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            assert app.poll() is None, 'app exited before window'
            try:
                window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()[-1]
                run('xdotool', 'windowfocus', window)
                run('xdotool', 'windowsize', '--sync', window, '960', '640')
                run('xdotool', 'windowmove', window, '0', '0')
                return app, window
            except (subprocess.CalledProcessError, IndexError):
                time.sleep(.05)
        raise AssertionError('no owned native window')

    def ready(name, expected_page=None):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            path = shot(name)
            text = ocr(path)
            if expected_page == 2:
                # A prefetched page reports 'ready — cached', not 'Rendered'.
                # Require actual green fixture pixels on the canvas, not status alone.
                crop = Image.open(path).convert('RGB').crop((300, 100, 950, 600))
                green = sum(p[1] > 120 and p[0] < 100 and p[2] < 100
                            for p in image_pixels(crop))
                if 'page 2' in text and green > 5000:
                    return path
            elif 'rendered page' in text:
                return path
            time.sleep(.1)
        raise AssertionError('generated PDF never reached rendered status')

    try:
        app, window = launch(pdf)
        if args.exercise_window_reposition:
            run('xdotool', 'windowmove', '--sync', window, '0', '-280')
        image = ready('01-compact-minimum')
        measurements['minimum'] = geometry(image, 960, 640)
        g = measurements['minimum']
        verify_page_fit(image, 960, 640, g)
        check('minimum sidebar is at most 260px', g['sidebar_right_px'] <= 260)
        check('minimum canvas starts above 110px', g['canvas'][1] <= 110)
        check('minimum canvas receives over 55 percent of window area', g['canvas_fraction'] > .55)
        if args.baseline:
            before = geometry(args.baseline, 960, 640)
            measurements['before'] = before
            gain = g['canvas_area_px'] / before['canvas_area_px'] - 1
            measurements['canvas_area_gain_percent'] = gain * 100
            check('minimum viewport area increased at least 35 percent', gain >= .35)

        key('ctrl+g')
        run('xdotool', 'key', '--clearmodifiers', 'ctrl+a')
        run('xdotool', 'type', '--clearmodifiers', '2')
        key('Return')
        path = ready('02-direct-page-entry', expected_page=2)
        check('Ctrl+G still navigates to page 2', 'page 2' in ocr(path))
        key('ctrl+f')
        run('xdotool', 'type', '--clearmodifiers', 'FocusProbe734')
        time.sleep(.2)
        path = shot('03-search-focus')
        sidebar_text = ocr(path, crop_box=(0, 30, g['sidebar_right_px'], 300))
        check('Ctrl+F focuses search input and receives a unique typed sentinel',
              'focusprobe734' in sidebar_text and 'matches' in sidebar_text)
        key('Escape')
        key('ctrl+1')
        time.sleep(.3)

        viewport[:] = [1600, 1000]
        run('xdotool', 'windowsize', '--sync', window, '1600', '1000')
        time.sleep(.4)
        image = shot('04-compact-large')
        measurements['large'] = geometry(image, 1600, 1000)
        verify_page_fit(image, 1600, 1000, measurements['large'])
        check('large sidebar remains compact', measurements['large']['sidebar_right_px'] <= 280)
        check('large canvas starts above 110px', measurements['large']['canvas'][1] <= 110)
        check('large viewer exceeds 70 percent of window area', measurements['large']['canvas_fraction'] > .70)
        app.terminate()
        app.wait(timeout=5)
        app, window = launch()
        time.sleep(.6)
        path = shot('05-compact-empty')
        text = ocr(path)
        check('empty window remains discoverable', 'drop a pdf' in text or 'open a pdf' in text)
        check('generated input unchanged after navigation-only checks', pdf.read_bytes() == original)
        check('owned application stays alive', app.poll() is None)
    finally:
        for app, log in apps:
            if app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait()
            log.close()
        display.close()
        receipt = {'checks': checks, 'measurements': measurements, 'screenshots': images,
                   'generated_fixtures_only': True, 'owned_display_only': True}
        (out / 'checks.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
