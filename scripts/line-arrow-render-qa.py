#!/usr/bin/env python3
"""Independent Poppler appearance/preservation check of generated native L/A QA."""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import shutil
import subprocess


def require(value, message):
    if not value:
        raise AssertionError(message)


def annotations(reader):
    return [ref.get_object() for ref in reader.pages[0].get('/Annots', [])
            if '/GlyphLine' in ref.get_object() or '/GlyphArrow' in ref.get_object()]


def main():
    from PIL import Image, ImageChops, ImageDraw
    from pypdf import PdfReader
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-out', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    native, out = args.native_out.resolve(), args.out.resolve()
    report = json.loads((native/'result.json').read_text())
    require(report.get('passed') is True and report.get('record_only') is False,
            'requires successful strict generated native QA')
    require(shutil.which('pdftoppm'), 'missing Poppler pdftoppm')
    require(not out.exists() or not any(out.iterdir()), 'refusing nonempty output directory')
    out.mkdir(parents=True, exist_ok=True)
    fixtures = []
    original = None
    for path in sorted(native.glob('.glyph-backup-*.pdf')):
        reader = PdfReader(io.BytesIO(path.read_bytes()), strict=True)
        owned = annotations(reader)
        if not owned:
            original = path
        if len(owned) == 4 and sum('/GlyphArrow' in a for a in owned) == 1:
            fixtures.append(path)
    if original is None or not fixtures:
        raise AssertionError('missing original and saved four-markup backups')
    saved = fixtures[-1]
    for name, path in [('original', original), ('saved', saved)]:
        subprocess.run(['pdftoppm', '-f', '1', '-singlefile', '-cropbox',
                        '-scale-to', '1200', '-png', str(path), str(out/name)],
                       check=True, capture_output=True, timeout=30)
    base = Image.open(out/'original.png').convert('RGB')
    image = Image.open(out/'saved.png').convert('RGB')
    require(base.size == image.size, 'independent page dimensions changed')
    reader = PdfReader(io.BytesIO(saved.read_bytes()), strict=True)
    page = reader.pages[0]
    crop = tuple(float(v) for v in page.cropbox)
    width, height = crop[2]-crop[0], crop[3]-crop[1]
    rotation = int(page.get('/Rotate', 0)) % 360
    require(rotation in (0, 90, 180, 270), 'unsupported fixture rotation')

    def pixel(x, y):
        u, v = (x-crop[0])/width, (y-crop[1])/height
        if rotation == 0:
            a, b = u, 1-v
        elif rotation == 90:
            a, b = v, u
        elif rotation == 180:
            a, b = 1-u, v
        else:
            a, b = 1-v, 1-u
        return a*image.width, b*image.height

    def red_near(point, radius=3):
        x, y = point
        region = image.crop((max(0, int(x)-radius), max(0, int(y)-radius),
                             min(image.width, int(x)+radius+1),
                             min(image.height, int(y)+radius+1)))
        for value in region.getdata():
            if isinstance(value, tuple) and len(value) >= 3:
                r, g, b = value[:3]
                if r > 180 and g < 75 and b < 90:
                    return True
        return False

    outside = Image.new('L', image.size, 255)
    draw = ImageDraw.Draw(outside)
    checks = []
    for index, annot in enumerate(annotations(reader)):
        rect = [float(v) for v in annot['/Rect']]
        corners = [pixel(x, y) for x in (rect[0], rect[2]) for y in (rect[1], rect[3])]
        xs, ys = zip(*corners)
        draw.rectangle((math.floor(min(xs))-3, math.floor(min(ys))-3,
                        math.ceil(max(xs))+3, math.ceil(max(ys))+3), fill=0)
        x1, y1, x2, y2 = [float(v) for v in annot['/L']]
        require(red_near(pixel((x1+x2)/2, (y1+y2)/2)),
                f'annotation {index}: independently rendered stem missing')
        checks.append('independent vector stem '+str(index))
        if '/GlyphArrow' in annot:
            dx, dy = x2-x1, y2-y1
            length = math.hypot(dx, dy)
            size = min(10, length*.4)
            ux, uy = dx/length, dy/length
            wings = [(x2-size*ux-size*.5*uy, y2-size*uy+size*.5*ux),
                     (x2-size*ux+size*.5*uy, y2-size*uy-size*.5*ux)]
            for x, y in wings:
                require(red_near(pixel(x, y), radius=2), 'independent open-arrow wing missing')
            checks.append('independent open-arrow wings at the second endpoint')
    masked = ImageChops.multiply(ImageChops.difference(base, image), outside.convert('RGB'))
    require(masked.getbbox() is None, 'independent rendering changed outside owned annotation bounds')
    checks.append('original rendering pixel-identical outside owned annotation bounds')
    result = {'passed': True, 'renderer': 'Poppler', 'rotation': rotation,
              'source': str(saved), 'source_sha256': hashlib.sha256(saved.read_bytes()).hexdigest(),
              'checks': checks}
    (out/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
