#!/usr/bin/env python3
"""Strict native Go to Sheet QA. Generates its own PDF; never saves or edits it.

Parent-owned runtime:
  uv run --with pillow --with pypdf python scripts/sheet-picker-qa.py \
    --binary /absolute/path/to/glyph --out /absolute/path/to/new-empty-directory
Requires Xvfb, xdotool, ffmpeg, tesseract and the sibling editing-qa.py.
Record-only produces evidence and failure checks, NEVER a strict-pass result.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

# Loading a sibling helper must not write artifacts outside --out.
sys.dont_write_bytecode = True

from PIL import Image, ImageChops, ImageOps
from pypdf import PdfReader, PdfWriter
from pypdf.annotations import Link
from pypdf.generic import (
    ArrayObject, DecodedStreamObject, DictionaryObject, FloatObject,
    NameObject, NumberObject, TextStringObject,
)

LABELS = ('A-101', 'DUP', 'DUP', 'Z-404')
COLORS = ((220, 45, 55), (35, 165, 65), (35, 80, 220), (220, 135, 30))
SENTINEL = 'glyphqaunknown827493'


def image_pixels(image):
    """Same ABI-safe enumeration as markup-qa, including old Pillow wheels."""
    getter = getattr(image, 'get_flattened_data', None) or image.getdata
    return getter()


def make_fixture():
    """Return independently parsed PDF bytes, compatible with pypdf 4.0.2+."""
    writer = PdfWriter()
    font = writer._add_object(DictionaryObject({
        NameObject('/Type'): NameObject('/Font'),
        NameObject('/Subtype'): NameObject('/Type1'),
        NameObject('/BaseFont'): NameObject('/Helvetica'),
    }))
    nums = ArrayObject()
    for index, (label, color) in enumerate(zip(LABELS, COLORS)):
        page = writer.add_blank_page(width=612, height=792)
        page[NameObject('/Resources')] = DictionaryObject({
            NameObject('/Font'): DictionaryObject({NameObject('/F1'): font}),
        })
        rgb = ' '.join(f'{v / 255:.6f}' for v in color)
        stream = DecodedStreamObject()
        stream.set_data((
            'q 1 1 1 rg 0 0 612 792 re f Q\n'
            f'q {rgb} rg 60 130 492 540 re f Q\n'
            f'BT /F1 22 Tf 65 720 Td (PHYSICAL PAGE {index + 1} - {label}) Tj ET\n'
            f'BT /F1 16 Tf 65 52 Td (SHEET {label} - PAGE {index + 1}) Tj ET\n'
            'q 0.1 0.1 0.1 RG 2 w 40 40 532 712 re S Q\n'
        ).encode('ascii'))
        page[NameObject('/Contents')] = writer._add_object(stream)
        # Prefix-only labels avoid API differences in set_page_label across pypdf.
        nums.extend([NumberObject(index), DictionaryObject({
            NameObject('/P'): TextStringObject(label),
        })])
    writer._root_object[NameObject('/PageLabels')] = DictionaryObject({
        NameObject('/Nums'): nums,
    })
    writer.add_outline_item('Foundation Plan', 0)
    writer.add_outline_item('Roof Detail', 3)
    foreign_line = DictionaryObject({
        NameObject('/Type'): NameObject('/Annot'),
        NameObject('/Subtype'): NameObject('/Line'),
        NameObject('/Rect'): ArrayObject([FloatObject(v) for v in (70, 70, 220, 100)]),
        NameObject('/L'): ArrayObject([FloatObject(v) for v in (70, 80, 220, 90)]),
        NameObject('/C'): ArrayObject([FloatObject(0), FloatObject(0), FloatObject(0)]),
        NameObject('/F'): NumberObject(4),
        NameObject('/Contents'): TextStringObject('foreign line, not Glyph owned'),
    })
    writer.add_annotation(0, foreign_line)
    # Public Link normalizes destinations on both hosted 4.0.2 and modern pypdf.
    writer.add_annotation(0, Link(rect=(250, 70, 400, 105), target_page_index=3))
    buffer = io.BytesIO()
    writer.write(buffer)
    data = buffer.getvalue()
    verify_fixture(data)
    return data


def verify_fixture(data):
    reader = PdfReader(io.BytesIO(data))
    if len(reader.pages) != 4 or tuple(reader.page_labels) != LABELS:
        raise RuntimeError('generated page count/labels did not round-trip')
    outlines = {item['/Title']: reader.get_destination_page_number(item)
                for item in reader.outline if isinstance(item, dict)}
    if outlines != {'Foundation Plan': 0, 'Roof Detail': 3}:
        raise RuntimeError('generated bookmarks did not round-trip')
    subtypes = [str(ref.get_object()['/Subtype'])
                for ref in reader.pages[0].get('/Annots', [])]
    if sorted(subtypes) != ['/Line', '/Link']:
        raise RuntimeError('foreign annotations did not round-trip')
    for page, color in zip(reader.pages, COLORS):
        marker = ' '.join(f'{v / 255:.6f}' for v in color).encode('ascii')
        if marker not in page.get_contents().get_data():
            raise RuntimeError('distinct generated color stream missing')
    return {'pages': 4, 'labels': list(reader.page_labels),
            'bookmarks': outlines, 'foreign_annotations': subtypes}


def load_isolation():
    path = Path(__file__).with_name('editing-qa.py')
    spec = importlib.util.spec_from_file_location('sheet_picker_isolation', path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f'cannot load owned-display helper: {path}')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.PrivateXvfb


def normalized(text):
    return re.sub(r'[^a-z0-9]', '', text.lower())


class NativeProbe:
    def __init__(self, binary, out, checks):
        self.binary, self.out, self.checks = binary, out, checks
        self.display = None
        self.app = None
        self.log = None
        self.window = None
        self.sequence = 0

    def alive(self):
        if self.display is None:
            raise RuntimeError('no owned display')
        self.display.ensure_alive()
        if self.app is not None and self.app.poll() is not None:
            raise RuntimeError(f'owned native app exited: {self.app.returncode}')

    def run(self, *command):
        self.alive()
        return subprocess.run(command, env=self.display.env, check=True,
                              capture_output=True, text=True, timeout=15).stdout

    def start(self, pdf):
        self.display = load_isolation()()
        self.alive()
        self.log = (self.out / 'native-app.log').open('w')
        self.app = subprocess.Popen([str(self.binary), str(pdf)], env=self.display.env,
                                    stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            self.alive()
            try:
                windows = self.run('xdotool', 'search', '--onlyvisible', '--pid',
                                   str(self.app.pid)).strip().splitlines()
                if windows:
                    self.window = windows[-1]
                    self.run('xdotool', 'windowfocus', '--sync', self.window)
                    self.run('xdotool', 'windowmove', self.window, '0', '0')
                    self.run('xdotool', 'windowsize', '--sync', self.window, '1200', '800')
                    self.run('xdotool', 'mousemove', '1590', '990')
                    return
            except subprocess.CalledProcessError:
                pass
            time.sleep(.08)
        raise RuntimeError('no visible PID-owned native window before deadline')

    def key(self, keys):
        self.run('xdotool', 'key', '--window', self.window, '--clearmodifiers', keys)
        time.sleep(.10)

    def type(self, text):
        self.run('xdotool', 'type', '--window', self.window, '--clearmodifiers',
                 '--delay', '35', '--', text)
        time.sleep(.12)

    def shot(self, name):
        path = self.out / (name + '.png')
        self.run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab',
                 '-draw_mouse', '0', '-video_size', '1600x1000',
                 '-i', self.display.env['DISPLAY'], '-frames:v', '1',
                 '-threads', '1', str(path))
        self.alive()
        return path

    def ocr(self, shot, region, suffix):
        path = self.out / (shot.stem + '-' + suffix + '-ocr.png')
        with Image.open(shot) as source:
            crop = ImageOps.autocontrast(source.crop(region).convert('L'))
            crop.resize((crop.width * 4, crop.height * 4),
                        Image.Resampling.LANCZOS).save(path)
        text = self.run('tesseract', str(path), 'stdout', '--psm', '6')
        (self.out / (path.stem + '.txt')).write_text(text)
        return text

    def check(self, name, condition, **evidence):
        self.checks.append({'name': name, 'passed': bool(condition), **evidence})
        if not condition:
            raise AssertionError(name)

    def colors(self, shot):
        # Exclude sidebar thumbnail colors, toolbar, footer and outside surface.
        with Image.open(shot) as image:
            pixels = list(image_pixels(image.convert('RGB').crop((350, 90, 1190, 755))))
        return [sum(all(abs(pixel[k] - color[k]) <= 12 for k in range(3))
                    for pixel in pixels) for color in COLORS]

    def page(self, number, name):
        deadline = time.monotonic() + 15
        previous = None
        stable = 0
        counts = []
        while time.monotonic() < deadline:
            shot = self.shot(name)
            counts = self.colors(shot)
            with Image.open(shot) as image:
                surface = image.convert('RGB').crop((350, 90, 1190, 755))
                digest = hashlib.sha256(surface.tobytes()).hexdigest()
            correct = counts[number - 1] > 12000 and all(
                n < 80 for i, n in enumerate(counts) if i != number - 1)
            stable = stable + 1 if correct and digest == previous else 0
            previous = digest
            if stable >= 2:
                footer = self.ocr(shot, (0, 760, 1180, 800), 'footer')
                chrome = self.ocr(shot, (0, 0, 1195, 90), 'chrome')
                page_text = self.ocr(shot, (350, 90, 1190, 755), 'page')
                # Actual page footer label+number, independent of sidebar matches.
                numeric = bool(re.search(rf'page\s*{number}\b', footer, re.I))
                sheet_footer = normalized(f'SHEET {LABELS[number - 1]} PAGE {number}')
                label = sheet_footer in normalized(page_text)
                self.check(f'{name}: actual settled page {number} pixels', True,
                           screenshot=shot.name, color_counts=counts)
                self.check(f'{name}: visible native footer physical page number', numeric,
                           footer_ocr=footer, chrome_ocr=chrome)
                self.check(f'{name}: actual sheet footer custom label and number', label,
                           page_ocr=page_text)
                return shot
            time.sleep(.12)
        self.check(f'{name}: actual settled page {number} pixels', False,
                   color_counts=counts, screenshot=name + '.png')

    def modal(self, name, query=None, no_match=False):
        deadline = time.monotonic() + 8
        text = ''
        while time.monotonic() < deadline:
            shot = self.shot(name)
            # Central modal only: don't let sidebar/bookmark text prove input.
            text = self.ocr(shot, (320, 130, 1040, 680), 'modal')
            title = 'gotosheet' in normalized(text)
            typed = query is None or normalized(query) in normalized(text)
            empty = not no_match or bool(re.search(
                r'no\s+(?:matching\s+(?:sheets|results)|matches|results|sheets)', text, re.I))
            if title and typed and empty:
                self.check(f'{name}: opaque native dialog and visible query', True,
                           screenshot=shot.name, query=query, modal_ocr=text)
                if no_match:
                    self.check(f'{name}: visible no-match text', True)
                return shot
            time.sleep(.1)
        self.check(f'{name}: native dialog/query/no-match evidence', False,
                   query=query, modal_ocr=text, screenshot=name + '.png')

    def open_query(self, query, name):
        self.key('ctrl+l')
        self.modal(name + '-open')
        self.key('ctrl+a')
        self.type(query)
        return self.modal(name + '-query', query)

    def closed(self, name):
        shot = self.shot(name)
        text = self.ocr(shot, (320, 130, 1040, 680), 'closed')
        self.check(f'{name}: Escape/confirmation closes native dialog',
                   'gotosheet' not in normalized(text), screenshot=shot.name)

    def close(self):
        errors = []
        try:
            if self.app is not None:
                if self.app.poll() is None:
                    self.app.terminate()
                try:
                    self.app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.app.kill()
                    self.app.wait(timeout=5)
        except Exception as exc:
            errors.append(f'app cleanup: {exc}')
        finally:
            if self.log is not None:
                self.log.close()
            if self.display is not None:
                try:
                    self.display.close()
                except Exception as exc:
                    errors.append(f'Xvfb cleanup: {exc}')
        return {'app_exited': self.app is None or self.app.poll() is not None,
                'xvfb_exited': self.display is None or self.display.server.poll() is not None,
                'errors': errors}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--record-only', action='store_true',
                        help='Record failures without claiming strict pass')
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error('--binary must be an existing executable')
    for command in ('Xvfb', 'xdotool', 'ffmpeg', 'tesseract'):
        if shutil.which(command) is None:
            parser.error(f'missing prerequisite: {command}')
    out = args.out.resolve()
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        parser.error('--out must be a new or empty generated-only directory')
    out.mkdir(parents=True, exist_ok=True)
    checks = []
    probe = NativeProbe(binary, out, checks)
    report = {'mode': 'record-only' if args.record_only else 'strict',
              'strict_pass': False, 'native_exercised': False, 'checks': checks,
              'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    pdf = out / 'sheet-picker-fixture.pdf'
    original = None
    error = None
    try:
        original = make_fixture()
        pdf.write_bytes(original)
        report['fixture'] = verify_fixture(original)
        report['source_sha256_before'] = hashlib.sha256(original).hexdigest()
        probe.start(pdf)
        report['native_exercised'] = True
        probe.key('ctrl+1')
        probe.page(1, '00-start-page1')
        probe.open_query('roof', '01-bookmark')
        probe.key('Return')
        probe.closed('01-bookmark-closed')
        probe.page(4, '01-bookmark-page4')
        probe.check('bookmark navigation leaves source SHA unchanged', pdf.read_bytes() == original)
        probe.open_query('2', '02-number')
        probe.key('Return')
        probe.page(2, '02-number-page2')
        probe.open_query('DUP', '03-duplicate')
        probe.key('Down')
        probe.key('Up')
        probe.key('Down')
        probe.key('Return')
        baseline = probe.page(3, '03-duplicate-page3')
        # Matching text includes E/L/R; PageDown must belong to the modal, not viewer.
        probe.open_query('Roof Detail', '04-modal-ownership')
        probe.key('Page_Down')
        probe.modal('04-modal-after-page-key', 'Roof Detail')
        probe.key('Escape')
        probe.closed('04-escape')
        after = probe.page(3, '04-escape-unchanged-page3')
        with Image.open(baseline) as a, Image.open(after) as b:
            # Toolbar selected-tool paint is independent of unchanged file bytes.
            diff = ImageChops.difference(a.convert('RGB').crop((500, 30, 1190, 85)),
                                        b.convert('RGB').crop((500, 30, 1190, 85)))
            changed = sum(max(p) > 20 for p in image_pixels(diff))
            probe.check('modal E/L/R typing does not change selected tool paint',
                        changed < 100, changed_toolbar_pixels=changed)
            canvas = ImageChops.difference(a.convert('RGB').crop((350, 100, 1190, 750)),
                                          b.convert('RGB').crop((350, 100, 1190, 750)))
            probe.check('modal page-global key and Escape do not move canvas',
                        canvas.getbbox() is None)
        # No preparatory canvas click: this specifically proves released keyboard focus.
        probe.key('ctrl+g')
        probe.key('ctrl+a')
        probe.type('1')
        probe.key('Return')
        probe.page(1, '05-focus-release-page1')
        probe.open_query('roof', '06-replacement')
        probe.key('ctrl+a')
        probe.type(SENTINEL)
        probe.modal('06-no-match', SENTINEL, no_match=True)
        probe.key('Return')
        probe.modal('06-no-match-enter-stays-open', SENTINEL, no_match=True)
        # Center overlay may obscure page colors: do not infer navigation from them.
        # Prove no navigation by physical pixels AFTER keyboard Escape dismisses it.
        probe.key('Escape')
        probe.closed('06-no-match-escape')
        probe.page(1, '06-no-match-unchanged-page1')
        probe.check('native navigation never edits or Saves source bytes', pdf.read_bytes() == original)
        verify_fixture(pdf.read_bytes())
        probe.check('foreign Line and Link, labels and bookmarks preserved', True)
    except Exception as exc:
        error = f'{type(exc).__name__}: {exc}'
        report['error'] = error
        checks.append({'name': 'workflow completed', 'passed': False, 'error': error})
    finally:
        report['cleanup'] = probe.close()
        if original is not None and pdf.exists():
            current = pdf.read_bytes()
            report['source_sha256_after'] = hashlib.sha256(current).hexdigest()
            checks.append({'name': 'final exact source bytes unchanged', 'passed': current == original})
        cleanup_ok = (report['cleanup']['app_exited'] and report['cleanup']['xvfb_exited']
                      and not report['cleanup']['errors'])
        checks.append({'name': 'owned app and Xvfb exited', 'passed': cleanup_ok})
        report['strict_pass'] = (not args.record_only and error is None
                                 and report['native_exercised'] and bool(checks)
                                 and all(check['passed'] for check in checks))
        (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
    return 0 if report['strict_pass'] or args.record_only else 1


if __name__ == '__main__':
    raise SystemExit(main())
