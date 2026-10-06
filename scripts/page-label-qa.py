#!/usr/bin/env python3
"""Assert page-label editing on an owned display and a generated PDF only."""
import argparse
import csv
import importlib.util
import io
import json
import re
import subprocess
import time
from pathlib import Path
from PIL import Image, ImageChops, ImageOps


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    fixture = load('glyph_label_fixture', Path(__file__).with_name('gui-qa.py'))
    isolation = load('glyph_label_isolation', Path(__file__).with_name('editing-qa.py'))
    pdf = out / 'page-label-fixture.pdf'
    fixture.make_colored_pdf(pdf)
    original = pdf.read_bytes()
    checks = []
    apps = []
    display = isolation.PrivateXvfb()

    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, text=True, check=True, capture_output=True).stdout

    def check(name, condition):
        checks.append({'name': name, 'passed': bool(condition)})
        assert condition, name

    def shot(name):
        path = out / (name + '.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1600x1000', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(path))
        return path

    def words(name, region):
        path = shot(name)
        left, top, right, bottom = region
        processed = out / (name + '-ocr.png')
        image = ImageOps.invert(Image.open(path).crop(region).convert('L'))
        image.resize((image.width * 4, image.height * 4), Image.Resampling.LANCZOS).save(processed)
        rows = csv.DictReader(io.StringIO(run('tesseract', str(processed), 'stdout', '--psm', '11', 'tsv')), delimiter='\t')
        return [(re.sub(r'[^a-z0-9]', '', row['text'].lower()), left + int(row['left']) // 4 + int(row['width']) // 8, top + int(row['top']) // 4 + int(row['height']) // 8) for row in rows if row.get('text', '').strip()]

    def token(prefix, name, region):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            found = [row for row in words(name, region) if row[0].startswith(prefix)]
            if found:
                return found[0][1:]
            time.sleep(.05)
        raise AssertionError(f'{prefix} not visible; inspect {out / (name + ".png")}')

    def key(keys):
        run('xdotool', 'key', '--clearmodifiers', keys)

    def launch():
        display.ensure_alive()
        log = open(out / f'app-{len(apps)}.log', 'w')
        app = subprocess.Popen([str(args.binary.resolve()), str(pdf)], env=display.env, stdout=log, stderr=log)
        apps.append((app, log))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            display.ensure_alive()
            try:
                window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()[-1]
                run('xdotool', 'windowfocus', window)
                run('xdotool', 'windowmove', window, '0', '0')
                run('xdotool', 'windowsize', '--sync', window, '1200', '800')
                time.sleep(.5)
                return app
            except (subprocess.CalledProcessError, IndexError):
                if app.poll() is not None:
                    raise AssertionError('native app exited during startup')
                time.sleep(.05)
        raise AssertionError('no owned native window')

    sidebar = (0, 180, 340, 760)
    try:
        app = launch()
        # Use the actual menu and dialog; no hidden app automation or OS picker.
        run('xdotool', 'mousemove', '110', '22', 'click', '1')
        x, y = token('rename', '01-label-menu', (20, 20, 420, 500))
        run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
        token('label', '02-label-dialog', (350, 170, 1100, 650))
        key('ctrl+a')
        run('xdotool', 'type', '--clearmodifiers', '--delay', '20', 'A101')
        key('Return')
        token('a101', '03-renamed-page', sidebar)
        check('native page-label rename updates thumbnail caption', True)
        check('label edit leaves source bytes unchanged before Save', pdf.read_bytes() == original)
        key('ctrl+z')
        time.sleep(.2)
        check('native Undo removes the new label', not any(row[0].startswith('a101') for row in words('04-label-undo', sidebar)))
        check('Undo leaves source bytes unchanged', pdf.read_bytes() == original)
        key('ctrl+shift+z')
        token('a101', '05-label-redo', sidebar)
        check('native Redo restores page label', True)
        run('xdotool', 'mousemove', '350', '760')
        time.sleep(.2)
        before = Image.open(shot('06-before-save')).convert('RGB').crop((440, 210, 1160, 600))
        key('ctrl+s')
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and pdf.read_bytes() == original:
            display.ensure_alive()
            time.sleep(.05)
        check('Save commits the label PDF', pdf.read_bytes() != original)
        token('a101', '07-saved-label', sidebar)
        run('xdotool', 'mousemove', '350', '760')
        time.sleep(.3)
        after = Image.open(shot('08-after-save')).convert('RGB').crop((440, 210, 1160, 600))
        check('label Save preserves displayed drawing pixels', ImageChops.difference(before, after).getbbox() is None)
        check('Save retains displaced original backup', any(path.read_bytes() == original for path in out.glob('.glyph-backup-*.pdf')))
        app.terminate()
        app.wait(timeout=5)
        launch()
        token('a101', '09-reopened-label', sidebar)
        check('embedded page label survives native reopen', True)
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
        (out / 'result.json').write_text(json.dumps({'checks': checks}, indent=2) + '\n')
    print(json.dumps({'checks': checks}, indent=2))


if __name__ == '__main__':
    main()
