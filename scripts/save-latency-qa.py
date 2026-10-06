#!/usr/bin/env python3
"""Synthetic 100-page native Save/reopen stress; never uses user PDFs/displays.

Run with pypdf and Pillow (e.g. uv run --with pypdf --with pillow python ...).
Save times include scripted polling, not input-to-display or production latency.
"""
import argparse
import importlib.util
import json
import random
import hashlib
import subprocess
import time
from collections import Counter
from pathlib import Path

from PIL import Image
from pypdf import PdfReader, PdfWriter
from pypdf.annotations import Link
from pypdf.generic import ArrayObject, NameObject, NumberObject, DictionaryObject, DecodedStreamObject


def load_isolation():
    spec = importlib.util.spec_from_file_location('glyph_stress_isolation', Path(__file__).with_name('editing-qa.py'))
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def owned(a):
    return '/GlyphRectangle' in a or '/GlyphEllipse' in a


def records(reader):
    return [(i, str(a['/Subtype']), tuple(float(v) for v in a['/Rect']))
            for i, page in enumerate(reader.pages)
            for ref in page.get('/Annots', [])
            if owned(a := ref.get_object())]


def appearances(reader):
    return Counter((i, str(a["/Subtype"]), tuple(float(v) for v in a["/Rect"]), a["/AP"]["/N"].get_object().get_data())
                   for i, page in enumerate(reader.pages) for ref in page.get("/Annots", [])
                   if owned(a := ref.get_object()))


def preserved(reader):
    pages = []
    for page in reader.pages:
        foreign = []
        for ref in page.get('/Annots', []):
            a = ref.get_object()
            if not owned(a):
                dest = a.get('/Dest', [])
                foreign.append((str(a['/Subtype']), tuple(float(v) for v in a.get('/Rect', [])),
                                tuple((v.idnum, v.generation) if hasattr(v, 'idnum') else str(v) for v in dest)))
        pages.append((tuple(page.mediabox), tuple(page.cropbox), page.rotation,
                      page.extract_text(), page.get_contents().get_data(), foreign))
    return pages


def image_pixels(image):
    getter = getattr(image, "get_flattened_data", None) or image.getdata
    return getter()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--seed-pdf', type=Path, required=True, help='Previously generated mixed Glyph fixture, not a user PDF')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--image-heavy', action='store_true', help='Add a distinct uncompressed 512x512 RGB image on every generated page (~75 MB)')
    parser.add_argument('--cycles', type=int, default=3)
    args = parser.parse_args()
    assert 1 <= args.cycles <= 12
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    seed = PdfReader(args.seed_pdf)
    assert sorted(r[1] for r in records(seed) if r[0] == 0) == ['/Circle', '/Square'], 'seed must contain two genuine saved Glyph types'
    writer = PdfWriter()
    for i in range(100):
        # Every owned annotation must have its own PDF object identity/page.
        writer.reset_translation(seed)
        page = writer.add_page(seed.pages[0])
        # Preserve the original geometry: changing crop/rotation without updating
        # Glyph's normalized ownership metadata correctly triggers a protective
        # mapping rejection. Separate backend coverage creates all four rotations
        # through the real annotation API instead of relocating saved annotations.
        page[NameObject('/Rotate')] = NumberObject(seed.pages[0].rotation)
        page.pop('/CropBox', None)
        if args.image_heavy:
            image = DecodedStreamObject()
            image.set_data(random.Random(i).randbytes(512 * 512 * 3))
            image.update({NameObject('/Type'): NameObject('/XObject'), NameObject('/Subtype'): NameObject('/Image'),
                          NameObject('/Width'): NumberObject(512), NameObject('/Height'): NumberObject(512),
                          NameObject('/ColorSpace'): NameObject('/DeviceRGB'), NameObject('/BitsPerComponent'): NumberObject(8)})
            resources = page['/Resources'].get_object()
            resources[NameObject('/XObject')] = DictionaryObject({NameObject('/StressImage'): writer._add_object(image)})
            content = DecodedStreamObject()
            content.set_data(page.get_contents().get_data() + b'\nq 24 0 0 24 10 10 cm /StressImage Do Q\n')
            page[NameObject('/Contents')] = writer._add_object(content)
    writer._pages.get_object()[NameObject('/CropBox')] = ArrayObject([NumberObject(v) for v in seed.pages[0].cropbox])
    # Use pypdf's public builder: older add_annotation expects its structured
    # internal-destination representation, not a raw PDF /Dest array.
    link = Link(rect=(30, 30, 90, 50), target_page_index=0)
    writer.add_annotation(99, link)
    writer.add_outline_item('Stress sheet 001', 0)
    pdf = out / 'synthetic-100-page.pdf'
    with pdf.open('wb') as f:
        writer.write(f)
    original = pdf.read_bytes()
    (out / 'original.pdf').write_bytes(original)
    baseline = PdfReader(pdf)
    assert len(baseline.pages) == 100 and len(records(baseline)) == 200
    expected = preserved(baseline)
    expected_appearances = appearances(baseline)
    def image_hashes(reader):
        return [hashlib.sha256(ref.get_object().get_data()).hexdigest()
                for p in reader.pages for ref in p['/Resources'].get_object().get('/XObject', {}).values()]
    expected_images = image_hashes(baseline)
    initial_bytes = len(original)
    checks, timings, apps = [], [], []
    display = load_isolation().PrivateXvfb()

    def check(name, ok):
        checks.append({'name': name, 'passed': bool(ok)})
        assert ok, name

    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, check=True, capture_output=True, text=True).stdout

    def key(value):
        run('xdotool', 'key', '--clearmodifiers', value)

    def shot(name):
        path = out / (name + '.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1600x1000',
            '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(path))
        return Image.open(path).convert('RGB')

    def launch():
        display.ensure_alive()
        log = (out / f'app-{len(apps)}.log').open('w')
        readiness_deadline = time.monotonic() + 10
        while True:
            try:
                run('xdotool', 'getdisplaygeometry'); break
            except subprocess.CalledProcessError:
                assert time.monotonic() < readiness_deadline
                time.sleep(.03)
        app = subprocess.Popen([str(args.binary.resolve()), str(pdf)], env=display.env, stdout=log, stderr=log)
        apps.append((app, log))
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            assert app.poll() is None, 'native app exited during startup'
            try:
                window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()[-1]
                run('xdotool', 'windowfocus', window)
                run('xdotool', 'windowmove', window, '0', '0')
                run('xdotool', 'windowsize', '--sync', window, '1200', '800')
                break
            except (subprocess.CalledProcessError, IndexError):
                time.sleep(.1)
        else:
            raise AssertionError('No owned native window')
        key('ctrl+1')
        region = (350, 220, 1190, 750)
        while time.monotonic() < deadline:
            image = shot('00-loaded').crop(region)
            values = [255 if isinstance(p, tuple) and min(p) > 245 else 0 for p in image_pixels(image)]
            if values.count(255) > 10000:
                mask = Image.new('L', image.size); mask.putdata(values)
                b = mask.getbbox(); assert b
                return app, (b[0] + region[0], b[1] + region[1], b[2] + region[0], b[3] + region[1])
            time.sleep(.1)
        raise AssertionError('Generated 100-page document did not render')

    def point(box, x, y):
        return round(box[0] + (box[2]-box[0])*x), round(box[1] + (box[3]-box[1])*y)

    try:
        app, box = launch()
        check('native 100-page 200-markup document loads', app.poll() is None)
        for cycle, tool in enumerate(('e' if i % 2 == 0 else 'r' for i in range(args.cycles))):
            before = pdf.read_bytes()
            a, b = point(box, .82, .08 + (cycle % 3)*.12), point(box, .94, .16 + (cycle % 3)*.12)
            key(tool); time.sleep(.3)
            run('xdotool', 'mousemove', str(a[0]), str(a[1]), 'mousedown', '1')
            for step in range(1, 8):
                run('xdotool', 'mousemove', str(round(a[0]+(b[0]-a[0])*step/7)), str(round(a[1]+(b[1]-a[1])*step/7)))
                time.sleep(.035)
            run('xdotool', 'mouseup', '1', 'mousemove', '350', '760')
            deadline = time.monotonic() + 20
            red = 0
            while time.monotonic() < deadline:
                crop = shot(f'{cycle+1:02d}-unsaved').crop((*a, *b))
                red = sum(1 for p in image_pixels(crop) if isinstance(p, tuple) and p[0] > 180 and p[1] < 75 and p[2] < 90)
                if red > 10: break
                assert app.poll() is None, 'native app exited during edit'
                time.sleep(.1)
            check(f'cycle {cycle+1}: native unsaved markup appears', red > 10)
            check(f'cycle {cycle+1}: unsaved edit leaves source unchanged', pdf.read_bytes() == before)
            start = time.monotonic(); key('ctrl+s')
            deadline = start + 20
            while pdf.read_bytes() == before and time.monotonic() < deadline:
                display.ensure_alive(); assert app.poll() is None; time.sleep(.025)
            check(f'cycle {cycle+1}: native Save commits', pdf.read_bytes() != before)
            timings.append({'cycle': cycle+1, 'observed_save_seconds': time.monotonic()-start, 'bytes': pdf.stat().st_size})
            current = PdfReader(pdf)
            check(f'cycle {cycle+1}: reopened owned count exact', len(records(current)) == 201+cycle)
            check(f'cycle {cycle+1}: embedded image bytes preserved', image_hashes(current) == expected_images)
            check(f'cycle {cycle+1}: original text/streams/boxes/rotation/link preserved', preserved(current) == expected)
            check(f'cycle {cycle+1}: original 200 markup geometries and appearance streams preserved', not (expected_appearances - appearances(current)))
            check(f'cycle {cycle+1}: backup retains preceding exact PDF', any(p.read_bytes() == before for p in out.glob('.glyph-backup-*.pdf')))
        app.terminate(); app.wait(timeout=5)
        app, box = launch()
        check(f'final {200 + args.cycles}-markup PDF reopens in native viewer', app.poll() is None and len(records(PdfReader(pdf))) == 200 + args.cycles)
        image = shot('04-reopened-proof')
        for cycle in range(min(3, args.cycles)):
            a, b = point(box, .82, .08 + (cycle % 3)*.12), point(box, .94, .16 + (cycle % 3)*.12)
            red = sum(1 for p in image_pixels(image.crop((*a, *b)))
                      if isinstance(p, tuple) and p[0] > 180 and p[1] < 75 and p[2] < 90)
            check(f'reopen: saved new markup {cycle+1} renders', red > 10)
    finally:
        for app, log in apps:
            if app.poll() is None:
                app.terminate()
                try: app.wait(timeout=5)
                except subprocess.TimeoutExpired: app.kill(); app.wait()
            log.close()
        display.close()
        report = {'scope': 'synthetic native 100-page/200-seeded-markup workload; not a production corpus or comparative latency benchmark',
                  'checks': checks, 'timings': timings, 'fixture': str(pdf), 'initial_bytes': initial_bytes, 'image_heavy': args.image_heavy, 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest()}
        (out / 'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()

