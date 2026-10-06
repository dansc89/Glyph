#!/usr/bin/env python3
"""Isolated native Save feedback + independent persistence QA on a private copy.

Run: uv run --with pillow --with pypdf python scripts/save-feedback-qa.py
     --binary target/release/glyph --out dist/save-feedback-new
Prerequisite generated fixture (never a user drawing):
  uv run --with pillow --with pypdf python scripts/markup-qa.py --binary target/release/glyph --shape ellipse --repeat-save --read-only-retry --imported-reals --out dist/markup-qa
  uv run --with pillow --with pypdf python scripts/save-latency-qa.py --binary target/release/glyph --seed-pdf dist/markup-qa/mixed-shapes-proof.pdf --heavy --out dist/save-latency-heavy
Requires Xvfb, xdotool, ffmpeg (x11grab), tesseract. No native run at import.
Record-only relaxes ONLY the Saving-label gate, never persistence checks.
"""
import argparse
import hashlib
import filecmp
import importlib.util
import json
import re
import shutil
import subprocess
import time
from collections import Counter
from pathlib import Path

from PIL import Image, ImageOps
from pypdf import PdfReader


def load_module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    if spec is None or spec.loader is None:
        raise RuntimeError(f'Cannot import {filename}')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def stop(child):
    if child is not None:
        if child.poll() is None:
            child.terminate()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True, help='New, nonexistent output directory')
    parser.add_argument('--require-saving-label', action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument('--record-only', action='store_true', help='Record label evidence without gating on Saving PDF; persistence still required')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    fixture = Path(__file__).resolve().parents[1] / 'dist/save-latency-heavy/synthetic-100-page.pdf'
    fixture = fixture.resolve(strict=True)
    out = args.out.resolve()
    # Refuse reuse: no original fixture, existing output, or user PDF can be overwritten.
    out.mkdir(parents=True, exist_ok=False)
    pdf = out / 'private.pdf'
    shutil.copyfile(fixture, pdf)
    shutil.copyfile(pdf, out / 'pre-save.pdf')
    before_sha = sha(pdf)
    report = {'fixture': str(fixture), 'private_pdf': str(pdf), 'binary': str(binary),
              'binary_sha256': sha(binary), 'source_sha256': before_sha,
              'require_saving_label': args.require_saving_label and not args.record_only,
              'record_only': args.record_only, 'checks': [], 'frames': [],
              'seen_saving': False, 'seen_saved_refresh': False,
              'scope': 'Synthetic native private-display feedback evidence; labels do not prove persistence.'}
    display = app = capture = None
    app_log = capture_log = None
    latency = load_module('glyph_feedback_latency', 'save-latency-qa.py')

    def check(name, condition):
        report['checks'].append({'name': name, 'passed': bool(condition)})
        if not condition:
            raise AssertionError(name)

    def run(*command, timeout=15):
        display.ensure_alive()
        if app is not None and app.poll() is not None:
            raise RuntimeError('Owned native app exited')
        return subprocess.run(list(command), env=display.env, capture_output=True,
                              text=True, check=True, timeout=timeout).stdout

    def key(value):
        run('xdotool', 'key', '--clearmodifiers', value)

    def shot(name):
        path = out / f'{name}.png'
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-draw_mouse', '0',
            '-video_size', '1200x800', '-i', display.env['DISPLAY'],
            '-frames:v', '1', '-threads', '1', str(path))
        with Image.open(path) as image:
            return image.convert('RGB')

    def ocr(image, path):
        # Dark footer becomes black text on white; contrast/upscale before OCR.
        gray = ImageOps.autocontrast(image.convert('L'))
        if gray.getpixel((0, 0)) < 128:
            gray = ImageOps.invert(gray)
        gray.resize((gray.width * 3, gray.height * 3), Image.Resampling.LANCZOS).save(path)
        return run('tesseract', str(path), 'stdout', '--psm', '6', timeout=15).strip()

    def normalized(text):
        return re.sub(r'[^a-z0-9]+', ' ', text.lower()).strip()

    def red(image):
        return sum(1 for r, g, b in latency.image_pixels(image) if r > 180 and g < 75 and b < 90)

    def image_hashes(reader):
        return [hashlib.sha256(ref.get_object().get_data()).hexdigest()
                for page in reader.pages
                for ref in page['/Resources'].get_object().get('/XObject', {}).values()]

    try:
        baseline = PdfReader(pdf)
        expected_records = Counter(latency.records(baseline))
        expected_preserved = latency.preserved(baseline)
        expected_appearances = latency.appearances(baseline)
        expected_images = image_hashes(baseline)
        report['initial_owned_count'] = sum(expected_records.values())
        report['initial_bytes'] = pdf.stat().st_size
        check('100-page heavy generated fixture', len(baseline.pages) == 100 and sum(expected_records.values()) >= 206)
        display = load_module('glyph_feedback_isolation', 'editing-qa.py').PrivateXvfb()
        deadline = time.monotonic() + 10
        while True:
            try:
                run('xdotool', 'getdisplaygeometry')
                break
            except subprocess.CalledProcessError:
                if time.monotonic() >= deadline:
                    raise RuntimeError('Private display not ready')
                time.sleep(.05)
        app_log = (out / 'native.log').open('w')
        app = subprocess.Popen([str(binary), str(pdf)], env=display.env, stdout=app_log, stderr=app_log)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()[-1]
                run('xdotool', 'windowmove', window, '0', '0')
                run('xdotool', 'windowsize', '--sync', window, '1200', '800')
                run('xdotool', 'windowfocus', window)
                break
            except (subprocess.CalledProcessError, IndexError):
                time.sleep(.1)
        else:
            raise RuntimeError('No owned native window')
        report['display'] = display.env['DISPLAY']
        report['window'] = window
        key('ctrl+1')
        region = (350, 220, 1190, 750)
        box = None
        while time.monotonic() < deadline:
            image = shot('loaded')
            crop = image.crop(region)
            values = [255 if min(pixel) > 245 else 0 for pixel in latency.image_pixels(crop)]
            if values.count(255) > 10000:
                mask = Image.new('L', crop.size)
                mask.putdata(values)
                bounds = mask.getbbox()
                if bounds:
                    box = (bounds[0]+region[0], bounds[1]+region[1], bounds[2]+region[0], bounds[3]+region[1])
                    break
            time.sleep(.1)
        check('Actual white PDF bounds detected', box is not None)
        report['page_screen_bounds'] = box
        key('e')  # This triggers asynchronous markup loading on baseline binaries too.
        # Demand observed idle footer repeatedly, not just a fixed startup sleep.
        deadline = time.monotonic() + 30
        idle = 0
        idle_evidence = []
        while time.monotonic() < deadline:
            image = shot('initial-idle')
            text = ocr(image.crop((0, 748, 1200, 800)), out / 'initial-idle-ocr.png')
            norm = normalized(text)
            busy = any(s in norm for s in ('loading', 'editing document', 'saving', 'refresh'))
            actual_idle = bool(norm) and not busy and any(s in norm for s in ('ellipse', 'markup', 'page', 'ready', 'saved'))
            idle = idle + 1 if actual_idle else 0
            idle_evidence.append(text)
            if idle >= 3:
                break
            time.sleep(.2)
        report['initial_idle_footer_text'] = idle_evidence
        check('Asynchronous initial markup load reaches observed idle', idle >= 3)
        def point(x, y):
            return (round(box[0]+(box[2]-box[0])*x), round(box[1]+(box[3]-box[1])*y))
        chosen = None
        for coords in ((.08, .70, .19, .80), (.24, .70, .35, .80), (.08, .54, .19, .64)):
            a, b = point(*coords[:2]), point(*coords[2:])
            rectangle = (a[0]-3, a[1]-3, b[0]+4, b[1]+4)
            if red(image.crop(rectangle)) == 0:
                chosen = (a, b, rectangle)
                break
        check('Candidate native ellipse region is initially red-free', chosen is not None)
        a, b, rectangle = chosen
        report['ellipse_screen_rect'] = rectangle
        run('xdotool', 'mousemove', '--sync', str(a[0]), str(a[1]))
        time.sleep(.08)
        run('xdotool', 'mousedown', '1')
        time.sleep(.08)
        try:
            for step in range(1, 9):
                run('xdotool', 'mousemove', str(round(a[0]+(b[0]-a[0])*step/8)), str(round(a[1]+(b[1]-a[1])*step/8)))
                time.sleep(.045)
        finally:
            run('xdotool', 'mouseup', '1')
        run('xdotool', 'mousemove', '350', '760')
        deadline = time.monotonic() + 20
        red_count = 0
        while time.monotonic() < deadline:
            red_count = red(shot('unsaved-ellipse').crop(rectangle))
            if red_count > 10:
                break
            time.sleep(.1)
        report['unsaved_red_pixels'] = red_count
        check('New native ellipse visibly appears', red_count > 10)
        check('Unsaved ellipse leaves private PDF SHA unchanged', sha(pdf) == before_sha)
        deadline = time.monotonic() + 30
        edit_idle = 0
        report['pre_save_footer_text'] = []
        while time.monotonic() < deadline:
            image = shot('pre-save-idle')
            text = ocr(image.crop((0, 748, 1200, 800)), out / 'pre-save-idle-ocr.png')
            norm = normalized(text)
            busy = any(s in norm for s in ('loading', 'editing document', 'saving', 'refresh', 'wait for'))
            edit_idle = edit_idle + 1 if norm and not busy else 0
            report['pre_save_footer_text'].append(text)
            if edit_idle >= 2:
                break
            time.sleep(.2)
        check('Native edit reaches observed idle before Save', edit_idle >= 2)
        frame_dir = out / 'footer-frames'
        frame_dir.mkdir()
        capture_log = (out / 'capture.log').open('w')
        # Continuous capture; no OCR or whole-PDF polling competes with Save.
        display.ensure_alive()
        capture = subprocess.Popen(['ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab',
            '-draw_mouse', '0', '-framerate', '30', '-video_size', '1200x800',
            '-i', display.env['DISPLAY'], '-t', '4', '-vf', 'crop=1200:52:0:748',
            '-threads', '1', str(frame_dir / 'frame-%05d.png')],
            env=display.env, stdout=capture_log, stderr=capture_log)
        try:
            deadline = time.monotonic() + 8
            first = frame_dir / 'frame-00001.png'
            while time.monotonic() < deadline:
                display.ensure_alive()
                if first.exists():
                    try:
                        with Image.open(first) as frame:
                            frame.load()
                        break
                    except (OSError, ValueError):
                        pass
                if capture.poll() is not None:
                    raise RuntimeError('Footer capture exited before first complete frame')
                time.sleep(.01)
            else:
                raise RuntimeError('No complete pre-Save footer frame')
            report['save_trigger_monotonic'] = time.monotonic()
            key('ctrl+s')
            capture.wait(timeout=12)
            check('Continuous footer capture completed', capture.returncode == 0)
        finally:
            stop(capture)
            capture = None
            capture_log.close()
            capture_log = None
        frames = sorted(frame_dir.glob('frame-*.png'))
        check('Continuous footer has multiple frames', len(frames) >= 20)
        # All frame OCR is deferred until capture has fully stopped.
        for index, path in enumerate(frames):
            with Image.open(path) as frame:
                text = ocr(frame, path.with_name(path.stem + '-ocr.png'))
            norm = normalized(text)
            saving = bool(re.search(r'\bsaving\s+pdf\b', norm))
            saved_refresh = 'saved' in norm and 'refresh' in norm
            report['frames'].append({'frame': path.name, 'relative_seconds': index/30,
                                     'text': text, 'saving_pdf': saving, 'saved_refresh': saved_refresh})
            report['seen_saving'] |= saving
            report['seen_saved_refresh'] |= saved_refresh
        # Independently reopen committed PDF; never use feedback as proof of commit.
        check('Save changes private PDF bytes', sha(pdf) != before_sha)
        current = PdfReader(pdf)
        current_records = Counter(latency.records(current))
        check('Reopened owned annotation count exactly +1', sum(current_records.values()) == sum(expected_records.values()) + 1)
        added = current_records - expected_records
        check('Exactly one new page-one ellipse and all old records retained',
              not (expected_records-current_records) and sum(added.values()) == 1
              and all(item[0] == 0 and item[1] == '/Circle' for item in added))
        check('Original text/streams/boxes/rotation/foreign annotations preserved', latency.preserved(current) == expected_preserved)
        check('Original image stream hashes preserved', image_hashes(current) == expected_images)
        check('Original owned appearance streams preserved', not (expected_appearances-latency.appearances(current)))
        check('Exact pre-Save backup exists', any(filecmp.cmp(path, out / 'pre-save.pdf', shallow=False) for path in out.glob('.glyph-backup-*.pdf')))
        check('Original heavy fixture remains unchanged', sha(fixture) == before_sha)
        report['saved_sha256'] = sha(pdf)
        report['final_owned_count'] = sum(current_records.values())
        report['saving_frame_count'] = sum(frame['saving_pdf'] for frame in report['frames'])
        report['saved_refresh_note'] = 'Informational only: brief native refresh feedback is not a required gate.'
        if report['require_saving_label']:
            check('Actual Saving PDF label captured', report['seen_saving'])
        report['passed'] = True
    except BaseException as error:
        report['passed'] = False
        report['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        # Each cleanup runs even if a previous cleanup fails.
        try:
            stop(capture)
        finally:
            try:
                if capture_log is not None:
                    capture_log.close()
                stop(app)
            finally:
                try:
                    if app_log is not None:
                        app_log.close()
                finally:
                    try:
                        if display is not None:
                            display.close()
                    finally:
                        (out / 'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'frames'}, indent=2))


if __name__ == '__main__':
    main()
