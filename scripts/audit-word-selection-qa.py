#!/usr/bin/env python3
"""Reproduce double-click word selection using only the generated QA PDF."""
import argparse, csv, importlib.util, io, json, subprocess, time
from pathlib import Path
from PIL import Image


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location('owned', Path(__file__).with_name('editing-qa.py'))
    assert spec and spec.loader
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    display = helper.PrivateXvfb()
    app = None
    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, check=True, text=True, capture_output=True, timeout=15).stdout
    try:
        deadline = time.monotonic() + 10
        while True:
            try:
                run('xdotool', 'getdisplaygeometry'); break
            except subprocess.CalledProcessError:
                assert time.monotonic() < deadline
                time.sleep(.03)
        with (a.out / 'app.log').open('w') as log:
            fixture_spec = importlib.util.spec_from_file_location('fixture', Path(__file__).with_name('gui-qa.py'))
            assert fixture_spec and fixture_spec.loader
            fixture = importlib.util.module_from_spec(fixture_spec)
            fixture_spec.loader.exec_module(fixture)
            pdf = (a.out / 'word-selection.pdf').resolve()
            fixture.make_colored_pdf(pdf)
            app = subprocess.Popen([str(a.binary.resolve()), str(pdf)], env=display.env, stdout=log, stderr=log)
            image = a.out / 'loaded.png'
            deadline = time.monotonic() + 20
            word = None
            while time.monotonic() < deadline:
                assert app.poll() is None
                run('ffmpeg', '-v', 'error', '-y', '-f', 'x11grab', '-video_size', '1600x1000', '-draw_mouse', '0', '-i', display.env['DISPLAY'], '-frames:v', '1', str(image))
                words = csv.DictReader(io.StringIO(run('tesseract', str(image), 'stdout', '--psm', '11', 'tsv')), delimiter='\t')
                word = next((w for w in words if w['text'].upper() == 'CENTERLINE' and int(w['left']) > 340), None)
                if word: break
            assert word, 'generated word not displayed'
            # Text extraction is asynchronous; its integration was already validated
            # by the separate native drag/clipboard check.
            time.sleep(2)
            window = run('xdotool', 'search', '--onlyvisible', '--pid', str(app.pid)).strip().splitlines()[-1]
            run('xdotool', 'windowfocus', window)
            subprocess.run(['xclip', '-selection', 'clipboard', '-i'], input='audit-selection-sentinel', env=display.env, check=True, text=True, timeout=3)
            x = int(word['left']) + int(word['width']) // 2
            y = int(word['top']) + int(word['height']) // 2
            run('xdotool', 'mousemove', str(x), str(y), 'click', '--repeat', '2', '--delay', '100', '1')
            time.sleep(.2)
            run('xdotool', 'key', 'ctrl+c')
            time.sleep(.1)
            copied = run('xclip', '-selection', 'clipboard', '-o')
            run('ffmpeg', '-v', 'error', '-y', '-f', 'x11grab', '-video_size', '1600x1000', '-draw_mouse', '0', '-i', display.env['DISPLAY'], '-frames:v', '1', str(a.out / 'double-click.png'))
            report = {'expected_word': 'CENTERLINE', 'copied_text': copied,
                      'double_click_selects_word': copied == 'CENTERLINE',
                      'scope': 'generated PDF; actual native input and clipboard on owned Xvfb'}
            (a.out / 'result.json').write_text(json.dumps(report, indent=2))
            print(json.dumps(report, indent=2))
            assert report['double_click_selects_word'], f'Expected whole word, copied {copied!r}'
    finally:
        try:
            if app is not None and app.poll() is None:
                app.terminate()
                try: app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill(); app.wait(timeout=5)
        finally:
            display.close()


if __name__ == '__main__':
    main()
