#!/usr/bin/env python3
"""Strict generated/private-display native move/resize/endpoint persistence QA.

Requires Python Pillow/pypdf, Xvfb, xdotool, ffmpeg and a fresh native executable.
This geometry gate does not claim that property widgets were exercised.
"""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import shutil
import subprocess
import time
from typing import Any


def load(name, path):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError('missing helper '+str(path))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def main():
    from PIL import Image, ImageChops
    from pypdf import PdfReader
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--properties-x', type=int, help='Verified toolbar properties-button X in the 1200x800 owned window; enables strict RGB/width native tests')
    parser.add_argument('--properties-y', type=int, default=51, help='Parent-verified toolbar button Y')
    args = parser.parse_args()
    binary, out = args.binary.resolve(), args.out.resolve()
    require(binary.is_file() and all(shutil.which(c) for c in ('Xvfb', 'xdotool', 'ffmpeg')),
            'missing binary/display dependencies')
    require(not out.exists() or not any(out.iterdir()), 'refusing nonempty output')
    out.mkdir(parents=True, exist_ok=True)
    here = Path(__file__).resolve().parent
    lines = load('geometry_line_helpers', here/'line-arrow-qa.py')
    fixture = load('geometry_fixture_helpers', here/'gui-qa.py')
    isolation = load('geometry_isolation_helpers', here/'editing-qa.py')
    pdf = out/'generated-editable-shapes.pdf'
    lines.make_fixture(pdf, fixture)
    original = pdf.read_bytes()
    (out/'original.pdf').write_bytes(original)
    baseline_reader = PdfReader(io.BytesIO(original), strict=True)
    baseline_streams = [page.get_contents().get_data() for page in baseline_reader.pages]
    baseline_foreign = [lines.canonical(ref.get_object()) for ref in baseline_reader.pages[0]['/Annots']]
    markers = {'Rectangle': '/GlyphRectangle', 'Ellipse': '/GlyphEllipse',
               'Line': '/GlyphLine', 'Arrow': '/GlyphArrow'}
    initial: list[dict[str, Any]] = [dict(kind='Rectangle', rect=[.20, .15, .25, .20]),
               dict(kind='Ellipse', rect=[.70, .15, .16, .20]),
               dict(kind='Line', endpoints=[.20, .50, .48, .50]),
               dict(kind='Arrow', endpoints=[.24, .66, .52, .78])]
    expected: list[dict[str, Any]] = [dict(kind='Rectangle', rect=[.30, .25, .35, .25]),
                dict(kind='Ellipse', rect=[.67, .23, .21, .25]),
                dict(kind='Line', endpoints=[.28, .55, .66, .53]),
                dict(kind='Arrow', endpoints=[.28, .63, .76, .73])]
    for row in initial+expected:
        row.update(color=[1, 0, 0], width=2)
    (out/'expected.json').write_text(json.dumps(expected, indent=2)+'\n')
    checks, apps = [], []
    display = isolation.PrivateXvfb()
    app: Any = None
    window: Any = None
    box: Any = None
    region = (210, 76, 1193, 765)

    def check(name, condition=True):
        checks.append({'name': name, 'passed': bool(condition)})
        require(condition, name)

    def run(*cmd):
        display.ensure_alive()
        if app is not None:
            require(app.poll() is None, 'owned application exited')
        return subprocess.run(cmd, env=display.env, check=True, capture_output=True,
                              text=True, timeout=15).stdout

    def shot(name):
        path = out/(name+'.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-draw_mouse', '0',
            '-video_size', '1600x1000', '-i', display.env['DISPLAY'], '-frames:v', '1',
            '-threads', '1', str(path))
        return Image.open(path).convert('RGB')

    def stable(name):
        deadline, previous, repeats = time.monotonic()+15, None, 0
        while time.monotonic() < deadline:
            image = shot(name)
            canvas = image.crop(region)
            loaded = sum(90 < p[0] < 190 and p[1] < 100 and p[2] > 180
                         for p in lines.image_pixels(canvas)) > 500
            repeats = repeats+1 if previous is not None and ImageChops.difference(previous, canvas).getbbox() is None else 0
            previous = canvas
            if loaded and repeats >= 2:
                return image
            time.sleep(.1)
        raise AssertionError(name+': no loaded settled canvas')

    def key(value):
        run('xdotool', 'windowfocus', window)
        run('xdotool', 'key', '--clearmodifiers', value)

    def point(x, y):
        return round(box[0]+(box[2]-box[0])*x), round(box[1]+(box[3]-box[1])*y)

    def click(position):
        run('xdotool', 'windowfocus', window)
        run('xdotool', 'mousemove', '--sync', str(position[0]), str(position[1]), 'click', '1')
        run('xdotool', 'mousemove', '190', '780')

    def drag(a, b, name):
        start, end = point(*a), point(*b)
        require(all(box[0] < p[0] < box[2] and box[1] < p[1] < box[3] for p in (start, end)), 'drag outside page')
        run('xdotool', 'windowfocus', window)
        run('xdotool', 'mousemove', '--sync', str(start[0]), str(start[1]), 'mousedown', '1')
        time.sleep(.15)
        run('xdotool', 'mousemove', '--sync', str(end[0]), str(end[1]))
        time.sleep(.15)
        run('xdotool', 'mouseup', '1', 'mousemove', '190', '780')
        stable(name)

    def validate(records):
        reader = PdfReader(io.BytesIO(pdf.read_bytes()), strict=True)
        require([page.get_contents().get_data() for page in reader.pages] == baseline_streams, 'page streams changed')
        annotations = [ref.get_object() for ref in reader.pages[0].get('/Annots', [])]
        foreign = [lines.canonical(a) for a in annotations if not any(marker in a for marker in markers.values())]
        require(foreign == baseline_foreign, 'foreign Line/Link changed')
        owned = [a for a in annotations if any(marker in a for marker in markers.values())]
        require(len(owned) == len(records), 'owned count changed')
        for row in records:
            candidates = [a for a in owned if markers[row['kind']] in a]
            require(len(candidates) == 1, 'missing/duplicate '+row['kind'])
            annotation = candidates[0]
            field = '/GlyphNormalizedRect' if 'rect' in row else '/GlyphNormalizedEndpoints'
            target = row.get('rect', row.get('endpoints'))
            actual = annotation.get(field, [])
            require(len(actual) == 4 and all(math.isfinite(float(v)) and abs(float(v)-t) < .006 for v, t in zip(actual, target)),
                    row['kind']+': saved geometry does not match native drag prediction')
            require(len(annotation['/C']) == 3 and all(math.isfinite(float(v)) and abs(float(v)-t) < .002 for v, t in zip(annotation['/C'], row['color'])), 'unexpected style')
            require(abs(float(annotation['/BS']['/W'])-row['width']) < .01, 'unexpected stroke weight')
        return True

    def save(records, name):
        prior = pdf.read_bytes()
        key('ctrl+s')
        deadline = time.monotonic()+12
        while time.monotonic() < deadline:
            if pdf.read_bytes() != prior:
                try:
                    validate(records)
                    break
                except Exception:
                    pass
            display.ensure_alive()
            require(app.poll() is None, 'application exited during Save')
            time.sleep(.1)
        else:
            validate(records)
            raise AssertionError(name+': no changed-byte commit')
        stable(name)
        check(name+': independently reopened expected geometry and preserved original streams/foreign annotations', validate(records))
        check(name+': exact previous-byte backup', any(p.read_bytes() == prior for p in out.glob('.glyph-backup-*.pdf')))

    def launch():
        nonlocal app, window, box
        require(run('xdotool', 'getdisplaygeometry').strip().split() == ['1600', '1000'], 'bad owned display')
        log = (out/f'app-{len(apps)}.log').open('w')
        app = subprocess.Popen([str(binary), str(pdf)], env=display.env, stdout=log, stderr=log)
        apps.append((app, log))
        deadline = time.monotonic()+10
        while time.monotonic() < deadline:
            try:
                windows = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()
                require(len(windows) == 1, 'expected one PID-owned window')
                window = windows[0]
                run('xdotool', 'windowmove', '--sync', window, '0', '0')
                run('xdotool', 'windowsize', '--sync', window, '1200', '800')
                key('ctrl+1')
                run('xdotool', 'mousemove', '190', '780')
                break
            except subprocess.CalledProcessError:
                time.sleep(.05)
        else:
            raise AssertionError('native window absent')
        image = stable('loaded-'+str(len(apps)))
        canvas = image.crop(region)
        pixels = [255 if min(p) > 245 else 0 for p in lines.image_pixels(canvas)]
        mask = Image.new('L', canvas.size)
        mask.putdata(pixels)
        bounds = mask.getbbox()
        if bounds is None or pixels.count(255) <= 3000:
            raise AssertionError('missing rendered paper')
        box = bounds[0]+region[0], bounds[1]+region[1], bounds[2]+region[0], bounds[3]+region[1]
        require(abs((box[2]-box[0])/(box[3]-box[1])-612/792) < .015, 'clipped paper geometry')
        click(point(.96, .5))
        stable('focused-'+str(len(apps)))

    success = False
    try:
        launch()
        key('a'); click(point(.2, .6)); stable('cancel-fence-first-endpoint')
        key('ctrl+l'); time.sleep(.4)
        image = shot('cancel-fence-picker-open')
        image.crop((300, 250, 900, 550)).resize((1800, 900)).save(out/'cancel-fence-picker-ocr.png')
        image.close()
        picker_text = run('tesseract', str(out/'cancel-fence-picker-ocr.png'), 'stdout', '--psm', '6')
        require('go to sheet' in ' '.join(picker_text.lower().split()), 'cancel fence must actually open Go to Sheet: '+picker_text[:200])
        key('Escape'); click(point(.4, .6)); stable('cancel-fence-fresh-first-endpoint')
        key('ctrl+s'); time.sleep(.6); stable('cancel-fence-save')
        check('picker takes input ownership: one click after Escape cannot finish pre-picker Arrow or mutate saved PDF', pdf.read_bytes() == original)
        key('Escape')
        key('r'); drag((.2, .15), (.45, .35), 'created-rectangle')
        key('e'); drag((.7, .15), (.86, .35), 'created-ellipse')
        for shortcut, a, b, name in [('l', (.2, .5), (.48, .5), 'line'), ('a', (.24, .66), (.52, .78), 'arrow')]:
            key(shortcut); click(point(*a)); stable(name+'-first'); click(point(*b)); stable('created-'+name)
        check('unsaved creation leaves source bytes unchanged', pdf.read_bytes() == original)
        save(initial, 'initial-save')
        prior = pdf.read_bytes()
        key('v'); click(point(.325, .15)); stable('selected-rectangle')
        drag((.325, .15), (.425, .25), 'moved-rectangle')
        drag((.55, .45), (.65, .50), 'resized-rectangle')
        click(point(.78, .15)); stable('selected-ellipse')
        drag((.78, .15), (.75, .23), 'moved-ellipse')
        drag((.83, .43), (.88, .48), 'resized-ellipse')
        click(point(.34, .5)); stable('selected-line')
        drag((.34, .5), (.42, .55), 'moved-line')
        drag((.56, .55), (.66, .53), 'edited-line-endpoint')
        click(point(.38, .72)); stable('selected-arrow')
        drag((.38, .72), (.42, .69), 'moved-arrow')
        drag((.56, .75), (.76, .73), 'edited-arrow-endpoint')
        check('unsaved transformations leave committed source untouched', pdf.read_bytes() == prior)
        save(expected, 'transformed-save')
        before_endpoint = [dict(row) for row in expected]
        before_endpoint[-1] = dict(kind='Arrow', endpoints=[.28, .63, .56, .75], color=[1, 0, 0], width=2)
        key('ctrl+z'); stable('undo-endpoint'); save(before_endpoint, 'undo-save')
        key('ctrl+shift+z'); stable('redo-endpoint'); save(expected, 'redo-save')
        if args.properties_x is not None:
            # Button position is parent-verified on this exact toolbar;
            # numeric fields are located from actual rendered popup labels.
            click(point(.52, .68)); stable('reselected-arrow-for-properties')
            click((args.properties_x, args.properties_y)); time.sleep(.4)
            def text_boxes(name):
                shot(name).close()
                data = run('tesseract', str(out/(name+'.png')), 'stdout', '--psm', '11', 'tsv')
                result = []
                for line in data.splitlines()[1:]:
                    columns = line.split('\t')
                    if len(columns) == 12 and columns[11].strip():
                        result.append((columns[11].strip(), *(int(v) for v in columns[6:10])))
                return result
            for label, value in [('Red', '0'), ('Green', '1'), ('Blue', '0'), ('Width', '4')]:
                candidates = [row for row in text_boxes('properties-before-'+label.lower())
                              if row[0].lower().startswith(label.lower())]
                require(len(candidates) == 1, 'native properties label missing/ambiguous: '+label)
                _, x, y, w, h = candidates[0]
                tx = x+w+27 if label != 'Width' else x+78
                run('xdotool', 'mousemove', str(tx), str(y+h//2), 'click', '--repeat', '2', '--delay', '100', '1')
                time.sleep(.15); key('ctrl+a')
                run('xdotool', 'type', '--clearmodifiers', '--delay', '40', value)
                key('Return'); time.sleep(.15)
            buttons = [row for row in text_boxes('properties-ready-apply') if row[0].lower() == 'apply']
            require(len(buttons) == 1, 'native Apply button missing/ambiguous')
            _, x, y, w, h = buttons[0]; click((x+w//2, y+h//2)); stable('properties-applied')
            styled = [dict(row) for row in expected]
            styled[-1] = dict(styled[-1], color=[0.,1.,0.], width=4.)
            save(styled, 'styled-arrow-save')
            key('ctrl+z'); stable('undo-style'); save(expected, 'undo-style-save')
            key('ctrl+shift+z'); stable('redo-style'); save(styled, 'redo-style-save')
            expected = styled
            (out/'expected.json').write_text(json.dumps(expected, indent=2)+'\n')
            check('native properties RGB/point width apply, one Undo and Redo across saved checkpoints', True)
        app.terminate(); app.wait(timeout=5); app = None
        launch(); check('transformed geometry survives native reopen', validate(expected))
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
        report = {'passed': success, 'native_success': success, 'record_only': False,
                  'property_widgets_exercised': success and args.properties_x is not None, 'binary': str(binary),
                  'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                  'fixture': str(pdf), 'checks': checks}
        (out/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
