#!/usr/bin/env python3
"""Exercise cold heavy-page loading on an owned X server and generated PDF."""
import argparse
import hashlib
import importlib.util
import json
import subprocess
import time
from pathlib import Path
from PIL import Image, ImageOps


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--navigate', action='store_true', help='Navigate from a light first page to uncached heavy page five')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location('isolation', Path(__file__).with_name('editing-qa.py'))
    assert spec is not None and spec.loader is not None
    isolation = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(isolation)
    pdf = out / 'generated-heavy-page.pdf'
    stream = bytearray(b'0.15 0.15 0.15 rg\n')
    for i in range(500000):
        stream.extend(f'{36 + i * 37 % 540} {80 + i * 23 % 580} 1 1 re f\n'.encode())
    stream.extend(b'0.1 0.65 0.3 rg 72 690 468 40 re f\n')
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>',
               b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>',
               b'<< /Length ' + str(len(stream)).encode() + b' >>\nstream\n' + stream + b'endstream']
    if args.navigate:
        light = b'0.1 0.3 0.8 rg 72 690 468 40 re f\n'
        objects = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R 7 0 R] /Count 5 >>']
        for index in range(5):
            content = 9 if index == 4 else 8
            objects.append(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents {content} 0 R >>'.encode())
        objects.extend([b'<< /Length ' + str(len(light)).encode() + b' >>\nstream\n' + light + b'endstream',
                        b'<< /Length ' + str(len(stream)).encode() + b' >>\nstream\n' + stream + b'endstream'])
    data, offsets = bytearray(b'%PDF-1.4\n'), []
    for i, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f'{i} 0 obj\n'.encode() + obj + b'\nendobj\n')
    xref = len(data)
    data.extend(f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        data.extend(f'{offset:010d} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    pdf.write_bytes(data)
    original = hashlib.sha256(data).hexdigest()
    display = isolation.PrivateXvfb()
    display.env['XDG_STATE_HOME'] = str(out / 'isolated-state')
    app = None
    checks = []
    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, capture_output=True, text=True, check=True, timeout=25).stdout
    def check(name, passed):
        checks.append({'name': name, 'passed': bool(passed)})
        (out / 'checks.json').write_text(json.dumps({'checks': checks}, indent=2))
        assert passed, name
    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                run('xdotool', 'getdisplaygeometry')
                break
            except subprocess.CalledProcessError:
                assert time.monotonic() < deadline, 'owned display did not become ready'
                time.sleep(.03)
        with (out / 'app.log').open('w') as log:
            app = subprocess.Popen([str(args.binary.resolve()), str(pdf)], env=display.env, stdout=log, stderr=log)
            deadline = time.monotonic() + 15
            while True:
                assert app.poll() is None, 'native app exited'
                try:
                    window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).splitlines()[-1]
                    break
                except (subprocess.CalledProcessError, IndexError):
                    assert time.monotonic() < deadline, 'no owned window'
                    time.sleep(.03)
            run('xdotool', 'windowmove', '--sync', window, '0', '0')
            if args.navigate:
                deadline = time.monotonic() + 20
                while True:
                    assert time.monotonic() < deadline, 'light first page did not render'
                    first = out / 'light-first-page.png'
                    run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1600x1000', '-draw_mouse', '0', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(first))
                    crop = Image.open(first).convert('RGB').crop((350, 50, 1400, 850))
                    getter = getattr(crop, 'get_flattened_data', None) or crop.getdata
                    if sum(isinstance(p, tuple) and p[0] < 60 and p[1] < 110 and p[2] > 160 for p in getter()) > 1500:
                        break
                run('xdotool', 'windowfocus', window)
                run('xdotool', 'key', '--clearmodifiers', 'ctrl+g', 'ctrl+a')
                run('xdotool', 'type', '5')
                run('xdotool', 'key', 'Return')
            deadline = time.monotonic() + 40
            captured = []
            ready = False
            while time.monotonic() < deadline:
                assert app.poll() is None, 'native app exited during rendering'
                image = out / f'frame-{len(captured):02}.png'
                run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1600x1000',
                    '-draw_mouse', '0', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(image))
                captured.append(image)
                crop = Image.open(image).convert('RGB').crop((350, 50, 1400, 850))
                getter = getattr(crop, 'get_flattened_data', None) or crop.getdata
                green = sum(isinstance(p, tuple) and p[0] < 60 and p[1] > 130 and p[2] < 110 for p in getter())
                if green > 1500:
                    ready = True
                    break
            check('heavy generated page finishes rendering; app remains alive', ready)
            found, wrong = None, []
            for image in captured:
                crop = Image.open(image).crop((350, 50, 1400, 850))
                ocr_image = out / 'ocr.png'
                ImageOps.autocontrast(crop.convert('L')).resize((1575, 1200)).save(ocr_image)
                text = run('tesseract', str(ocr_image), 'stdout', '--psm', '11').lower()
                if 'drop a pdf' in text:
                    wrong.append(image.name)
                expected_title = 'loading page 5 of 5' if args.navigate else 'loading page 1 of 1'
                if expected_title in ' '.join(text.split()):
                    found = image
            check('cold heavy-page render shows explicit Loading page feedback', found is not None)
            check('open document never shows Drop a PDF in captured loading frames', not wrong)
            check('loading/rendering does not modify generated PDF', hashlib.sha256(pdf.read_bytes()).hexdigest() == original)
            if found:
                Image.open(found).save(out / 'loading-preview.png')
            print(json.dumps({'checks': checks, 'captured_frames': len(captured), 'loading_screenshot': str(found)}, indent=2))
    finally:
        if app is not None and app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=5)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(timeout=5)
        display.close()


if __name__ == '__main__':
    main()
