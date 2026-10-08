#!/usr/bin/env python3
"""Fail-closed native two-click L/A QA; exclusively generated files/private Xvfb.

Run with a freshly built native binary (not the SourceRender Glyph 1.5 binary):
  python scripts/line-arrow-qa.py --binary /absolute/path/to/glyph --out /tmp/line-arrow-proof
Prerequisites: Python with Pillow/pypdf, Xvfb, xdotool, ffmpeg; working PDFium
runtime for the supplied binary. --out must be absent or empty. No record-only
mode: a missing native tool, render, metadata, backup or persistence fails.
"""
import argparse
import hashlib
import io
import json
import math
import shutil
import subprocess
import time
from pathlib import Path


def load(name, path):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, path)
    if not spec or not spec.loader:
        raise RuntimeError(f'cannot load helper: {path}')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def require(condition, message):
    # Unlike assert, all safety/proof checks remain enabled under python -O.
    if not condition:
        raise AssertionError(message)


def image_pixels(image):
    flattened = getattr(image, 'get_flattened_data', None)
    return flattened() if flattened else image.getdata()


def canonical(value):
    """Semantic dictionaries + decoded AP streams, independent of object IDs."""
    value = value.get_object() if hasattr(value, 'get_object') else value
    if isinstance(value, dict):
        result = {str(k): canonical(v) for k, v in value.items()
                  if str(k) not in ('/Length', '/Filter', '/DecodeParms', '/P')}
        if hasattr(value, 'get_data'):
            result['decoded_stream_sha256'] = hashlib.sha256(value.get_data()).hexdigest()
        return result
    if isinstance(value, (list, tuple)):
        return [canonical(v) for v in value]
    if isinstance(value, (int, float)):
        return float(value)
    return str(value)


def make_fixture(pdf, fixture, rotation=0, cropped=False):
    from pypdf import PdfReader, PdfWriter
    from pypdf.generic import (ArrayObject, DecodedStreamObject, DictionaryObject,
                               FloatObject, NameObject, NumberObject, TextStringObject)
    fixture.make_colored_pdf(pdf)
    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(pdf.read_bytes())))
    nums = lambda values: ArrayObject([FloatObject(v) for v in values])
    appearance = DecodedStreamObject()
    appearance.set_data(b'q 0 0.3 0.8 RG 2 w 0 0 m 70 20 l S Q\n')
    appearance.update({NameObject('/Type'): NameObject('/XObject'),
                       NameObject('/Subtype'): NameObject('/Form'),
                       NameObject('/BBox'): nums([0, 0, 70, 20])})
    line = DictionaryObject({NameObject('/Type'): NameObject('/Annot'),
        NameObject('/Subtype'): NameObject('/Line'), NameObject('/Rect'): nums([60, 60, 130, 80]),
        NameObject('/L'): nums([60, 60, 130, 80]), NameObject('/F'): NumberObject(4),
        NameObject('/NM'): TextStringObject('foreign-line'),
        NameObject('/LE'): ArrayObject([NameObject('/None'), NameObject('/None')]),
        NameObject('/AP'): DictionaryObject({NameObject('/N'): writer._add_object(appearance)})})
    link = DictionaryObject({NameObject('/Type'): NameObject('/Annot'),
        NameObject('/Subtype'): NameObject('/Link'), NameObject('/Rect'): nums([450, 60, 530, 80]),
        NameObject('/NM'): TextStringObject('foreign-link'), NameObject('/Border'): nums([0, 0, 0]),
        NameObject('/A'): DictionaryObject({NameObject('/S'): NameObject('/URI'),
            NameObject('/URI'): TextStringObject('https://example.invalid/fixture-only')})})
    if cropped:
        writer.pages[0].cropbox.lower_left = (30, 40)
        writer.pages[0].cropbox.upper_right = (582, 752)
    if rotation:
        writer.pages[0].rotate(rotation)
    writer.add_annotation(0, line)
    writer.add_annotation(0, link)
    with pdf.open('wb') as stream:
        writer.write(stream)


def snapshot(pdf):
    from pypdf import PdfReader
    reader = PdfReader(io.BytesIO(pdf.read_bytes()), strict=True)
    content = [p.get_contents().get_data() for p in reader.pages]
    foreign = {}
    owned = []
    for page_index, page in enumerate(reader.pages):
        for ref in page.get('/Annots', []):
            annot = ref.get_object()
            name = str(annot.get('/NM', ''))
            if name.startswith('foreign-'):
                require(name not in foreign, 'duplicate foreign annotation')
                foreign[name] = canonical(annot)
            elif '/GlyphLine' in annot or '/GlyphArrow' in annot:
                owned.append((page_index, annot))
            else:
                raise AssertionError('unexpected annotation in generated fixture')
    return content, foreign, owned


def validate(pdf, baseline, expected):
    content, foreign, owned = snapshot(pdf)
    require(content == baseline[0], 'original decoded page contents changed')
    require(foreign == baseline[1], 'foreign Line/Link semantic or appearance changed')
    require(len(owned) == len(expected), f'owned annotation count {len(owned)} != {len(expected)}')
    unmatched = list(owned)
    for kind, endpoints in expected:
        candidates = [(i, page, a) for i, (page, a) in enumerate(unmatched)
                      if a.get('/' + kind) == 1 and '/L' in a
                      and len(a['/L']) == 4
                      and all(math.isfinite(float(v)) for v in a['/L'])
                      and all(abs(float(v) - target) <= 3 for v, target in zip(a['/L'], endpoints))]
        require(len(candidates) == 1, f'missing/ambiguous {kind} with ordered endpoints {endpoints}')
        i, page, annot = candidates[0]
        require(page == 0 and annot.get('/Subtype') == '/Line', 'native annotation is not page-one /Line')
        require(annot.get('/' + ('GlyphArrow' if kind == 'GlyphLine' else 'GlyphLine'), 0) == 0,
                'conflicting native ownership markers')
        endings = [str(v) for v in annot.get('/LE', [])]
        require(endings == ['/None', '/OpenArrow' if kind == 'GlyphArrow' else '/None'],
                f'incorrect line endings: {endings}')
        require('/AP' in annot and '/N' in annot['/AP'], 'native appearance stream missing')
        unmatched.pop(i)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--rotation', type=int, choices=(0, 90, 180, 270), default=0)
    parser.add_argument('--crop', action='store_true', help='Use a nonzero cropped page origin')
    args = parser.parse_args()
    binary = args.binary.resolve()
    require(binary.is_file(), 'binary must be an existing native executable')
    for command in ('Xvfb', 'xdotool', 'ffmpeg'):
        require(shutil.which(command), f'missing dependency: {command}')
    from PIL import Image, ImageChops
    import pypdf  # fail before starting a display if unavailable
    out = args.out.resolve()
    require(not out.exists() or not any(out.iterdir()), '--out must be absent or empty; refusing overwrite')
    out.mkdir(parents=True, exist_ok=True)
    helpers = Path(__file__).resolve().parent
    fixture = load('line_qa_fixture', helpers / 'gui-qa.py')
    isolation = load('line_qa_isolation', helpers / 'editing-qa.py')
    pdf = out / 'line-arrow-fixture.pdf'
    make_fixture(pdf, fixture, args.rotation, args.crop)
    reader = pypdf.PdfReader(io.BytesIO(pdf.read_bytes()), strict=True)
    crop_box = tuple(float(v) for v in reader.pages[0].cropbox)
    page_width, page_height = crop_box[2]-crop_box[0], crop_box[3]-crop_box[1]
    page_ratio = (page_height/page_width if args.rotation in (90, 270)
                  else page_width/page_height)
    original = pdf.read_bytes()
    baseline = snapshot(pdf)
    require(len(baseline[1]) == 2 and not baseline[2], 'fixture schema invalid')
    checks, apps = [], []
    display = isolation.PrivateXvfb()
    app, window, box = None, None, None
    # Interior of the controlled 1200x800 native canvas, excluding toolbar/sidebar.
    # Portrait-only cropping would silently clip fitted landscape pages.
    region = (210, 76, 1193, 765)

    def check(name, condition=True):
        checks.append({'name': name, 'passed': bool(condition)})
        require(condition, name)

    def run(*cmd):
        display.ensure_alive()
        if app is not None:
            require(app.poll() is None, 'owned native process exited')
        return subprocess.run(cmd, env=display.env, text=True, check=True,
                              capture_output=True, timeout=15).stdout

    def shot(name):
        path = out / (name + '.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size',
            '1600x1000', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(path))
        return Image.open(path).convert('RGB')

    def stable(name, red_area=None, present=None):
        # Await actual colored fixture content and three unchanged canvas frames;
        # a visible window, white placeholder or animated loading spinner is not ready.
        deadline = time.monotonic() + 15
        previous, repeats = None, 0
        while time.monotonic() < deadline:
            image = shot(name)
            crop = image.crop(region)
            loaded = sum(1 for p in image_pixels(crop)
                         if p[0] > 90 and p[0] < 190 and p[1] < 100 and p[2] > 180) > 500
            repeats = repeats + 1 if previous is not None and ImageChops.difference(previous, crop).getbbox() is None else 0
            previous = crop
            red_ok = True
            if red_area is not None:
                n = sum(1 for p in image_pixels(image.crop(red_area))
                        if p[0] > 180 and p[1] < 75 and p[2] < 90)
                red_ok = n > 8 if present else n < 5
            if loaded and repeats >= 2 and red_ok:
                return image
            time.sleep(.1)
        raise AssertionError(f'{name}: loaded stable canvas/expected markup did not appear')

    def key(value):
        run('xdotool', 'windowfocus', window)
        run('xdotool', 'key', '--clearmodifiers', value)

    def point(x, y):
        return (round(box[0] + (box[2] - box[0]) * x),
                round(box[1] + (box[3] - box[1]) * y))

    def click(p):
        require(box[0] < p[0] < box[2] and box[1] < p[1] < box[3], 'click outside paper')
        run('xdotool', 'windowfocus', window)
        run('xdotool', 'mousemove', '--sync', str(p[0]), str(p[1]), 'click', '1')
        run('xdotool', 'mousemove', '190', '780')

    def launch():
        nonlocal app, window, box
        geometry = run('xdotool', 'getdisplaygeometry').strip().split()
        require(geometry == ['1600', '1000'], 'owned display is not ready at its declared size')
        log = (out / f'app-{len(apps)}.log').open('w')
        app = subprocess.Popen([str(binary), str(pdf)], env=display.env, stdout=log, stderr=log)
        apps.append((app, log))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            require(app.poll() is None, 'native startup failed')
            try:
                windows = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()
                require(len(windows) == 1, 'expected exactly one PID-owned window')
                window = windows[0]
                run('xdotool', 'windowmove', '--sync', window, '0', '0')
                run('xdotool', 'windowsize', '--sync', window, '1200', '800')
                key('ctrl+1')
                run('xdotool', 'mousemove', '190', '780')
                break
            except subprocess.CalledProcessError:
                time.sleep(.05)
        else:
            raise AssertionError('no PID-owned native window')
        image = stable('loaded-' + str(len(apps)))
        crop = image.crop(region)
        pixels = [255 if min(p) > 245 else 0 for p in image_pixels(crop)]
        mask = Image.new('L', crop.size)
        mask.putdata(pixels)
        bounds = mask.getbbox()
        require(bounds and pixels.count(255) > 3000, 'no rendered paper bounds')
        box = (bounds[0] + region[0], bounds[1] + region[1], bounds[2] + region[0], bounds[3] + region[1])
        require(abs((box[2]-box[0]) / (box[3]-box[1]) - page_ratio) < .015,
                'paper bounds clipped or inconsistent with known MediaBox')
        click(point(.95, .5))
        stable('focused-' + str(len(apps)))

    def stop():
        nonlocal app
        app.terminate()
        app.wait(timeout=5)
        app = None

    def draw(kind, a, b, name):
        key('l' if kind == 'GlyphLine' else 'a')
        stable(name + '-tool')
        pa, pb = point(*a), point(*b)
        click(pa)
        stable(name + '-first-click')
        click(pb)
        area = (min(pa[0],pb[0])-5, min(pa[1],pb[1])-5,
                max(pa[0],pb[0])+6, max(pa[1],pb[1])+6)
        stable(name, area, True)
        def to_pdf(p):
            u = (p[0]-box[0])/(box[2]-box[0])
            v = (p[1]-box[1])/(box[3]-box[1])
            if args.rotation == 0:
                x, y = u, 1-v
            elif args.rotation == 90:
                x, y = v, u
            elif args.rotation == 180:
                x, y = 1-u, v
            else:
                x, y = 1-v, 1-u
            return crop_box[0]+x*page_width, crop_box[1]+y*page_height
        return (kind, (*to_pdf(pa), *to_pdf(pb))), area

    def save(expected, name):
        prior = pdf.read_bytes()
        key('ctrl+s')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if pdf.read_bytes() != prior:
                try:
                    validate(pdf, baseline, expected)
                    break
                except Exception:
                    pass
            display.ensure_alive()
            time.sleep(.1)
        else:
            validate(pdf, baseline, expected)
            raise AssertionError(name + ': Save did not commit changed bytes')
        stable(name)
        validate(pdf, baseline, expected)
        check(name + ': contents, foreign annotations, native endpoints and appearances preserved')
        check(name + ': backup equals actual prior bytes',
              any(p.read_bytes() == prior for p in out.glob('.glyph-backup-*.pdf')))

    success = False
    try:
        launch()
        expected = []
        for kind, a, b, name in [
            ('GlyphLine', (.18,.30), (.65,.30), 'horizontal'),
            ('GlyphLine', (.75,.34), (.75,.60), 'vertical'),
            ('GlyphLine', (.65,.65), (.25,.43), 'reverse-sloped'),
            ('GlyphArrow', (.20,.72), (.65,.80), 'arrow')]:
            item, area = draw(kind, a, b, name)
            expected.append(item)
        check('unsaved two-click annotations do not write source', pdf.read_bytes() == original)
        key('ctrl+z'); stable('undo-arrow', area, False)
        key('ctrl+shift+z'); stable('redo-arrow', area, True)
        # Empty corner of sloped line bounds must not select the line.
        key('v'); click(point(.27,.63)); key('Delete')
        stable('empty-bbox-delete-safe', area, True)
        save(expected, 'first-save')
        first = pdf.read_bytes()
        key('ctrl+s'); stable('clean-save')
        check('clean Save leaves bytes unchanged', pdf.read_bytes() == first)
        stop(); launch(); validate(pdf, baseline, expected)
        check('all native markup survives reopen')
        key('v'); click(point(.425,.76)); key('Delete')
        stable('reopened-delete', area, False)
        save(expected[:-1], 'delete-save')
        key('ctrl+z'); stable('undo-deletion', area, True)
        save(expected, 'undo-delete-save')
        key('ctrl+shift+z'); stable('redo-deletion', area, False)
        save(expected[:-1], 'redo-delete-save')
        # One click followed by Escape is not a completed annotation.
        key('l'); click(point(.85,.72)); key('Escape')
        stable('cancelled-first-click')
        before = pdf.read_bytes()
        key('ctrl+s'); stable('cancelled-save')
        check('Escape cancels pending line without dirtying source', pdf.read_bytes() == before)
        validate(pdf, baseline, expected[:-1])
        stop(); launch(); validate(pdf, baseline, expected[:-1])
        check('repeated saved state survives second native reopen')
        shot('final-generated-sample')
        success = True
    finally:
        for process, log in apps:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()
            log.close()
        display.close()
        result = {'passed': success, 'native_success': success, 'record_only': False,
                  'binary': str(binary), 'fixture': str(pdf),
                  'rotation': args.rotation, 'cropped': args.crop, 'checks': checks}
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
