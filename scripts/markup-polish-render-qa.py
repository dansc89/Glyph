#!/usr/bin/env python3
"""Independent Poppler check of generated styled/transformed native markups.

--expected is a JSON list of independently predicted records: kind
(Rectangle/Ellipse/Line/Arrow), color (three 0..1 floats), width (PDF points),
and normalized geometry (rect [x,y,w,h] or endpoints [x1,y1,x2,y2]).
Only generated QA fixtures are authorized inputs. Existing original page streams
and foreign annotations must remain unchanged. No relaxed record-only mode.
"""
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import shutil
import subprocess
from typing import Any

MARKERS = {'Rectangle': '/GlyphRectangle', 'Ellipse': '/GlyphEllipse',
           'Line': '/GlyphLine', 'Arrow': '/GlyphArrow'}


def require(value, message):
    if not value:
        raise AssertionError(message)


def resolve(value: Any) -> Any:
    return value.get_object() if hasattr(value, 'get_object') else value


def canonical(value: Any) -> Any:
    value = resolve(value)
    if isinstance(value, dict):
        result = {str(k): canonical(v) for k, v in value.items()
                  if str(k) not in ('/Length', '/Filter', '/DecodeParms', '/P')}
        if hasattr(value, 'get_data'):
            get_data = getattr(value, 'get_data')
            result['decoded_stream_sha256'] = hashlib.sha256(get_data()).hexdigest()
        return result
    if isinstance(value, (list, tuple)):
        return [canonical(v) for v in value]
    if isinstance(value, (int, float)):
        return float(value)
    return str(value)


def inspect(path, reader_type):
    reader = reader_type(io.BytesIO(path.read_bytes()), strict=True)
    streams, foreign, owned = [], [], []
    for index, page in enumerate(reader.pages):
        streams.append(page.get_contents().get_data())
        for reference in page.get('/Annots', []):
            annotation = resolve(reference)
            kinds = [kind for kind, marker in MARKERS.items() if marker in annotation]
            if kinds:
                require(len(kinds) == 1, 'conflicting ownership markers')
                owned.append((index, kinds[0], annotation))
            else:
                foreign.append((index, canonical(annotation)))
    return reader, streams, foreign, owned


def main():
    from PIL import Image, ImageChops, ImageDraw
    from pypdf import PdfReader
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('original', 'edited', 'expected', 'out'):
        parser.add_argument('--'+name, type=Path, required=True)
    args = parser.parse_args()
    require(shutil.which('pdftoppm'), 'missing Poppler pdftoppm')
    out = args.out.resolve()
    require(not out.exists() or not any(out.iterdir()), 'refusing nonempty output directory')
    out.mkdir(parents=True, exist_ok=True)
    old, old_streams, old_foreign, old_owned = inspect(args.original, PdfReader)
    new, new_streams, new_foreign, owned = inspect(args.edited, PdfReader)
    expected = json.loads(args.expected.read_text())
    require(not old_owned, 'original fixture must have no owned annotations')
    require(old_streams == new_streams and old_foreign == new_foreign,
            'original streams or foreign annotations changed')
    require(len(owned) == len(expected) and expected, 'unexpected annotation count')
    require(len(old.pages) == len(new.pages), 'page count changed')
    checks = ['decoded page streams and foreign annotations preserved']
    unmatched = list(owned)
    matched = []
    for record in expected:
        kind = record['kind']
        target = record['rect'] if kind in ('Rectangle', 'Ellipse') else record['endpoints']
        key = '/GlyphNormalizedRect' if kind in ('Rectangle', 'Ellipse') else '/GlyphNormalizedEndpoints'
        candidates = [(i, row) for i, row in enumerate(unmatched)
                      if row[0] == 0 and row[1] == kind and len(row[2].get(key, [])) == 4
                      and all(math.isfinite(float(v)) and abs(float(v)-t) <= .006
                              for v, t in zip(row[2][key], target))]
        require(len(candidates) == 1, 'missing/ambiguous independently predicted '+kind+' geometry')
        i, row = candidates[0]
        annotation = row[2]
        color = [float(v) for v in annotation.get('/C', [])]
        require(len(color) == 3 and all(abs(v-t) < .002 for v, t in zip(color, record['color'])),
                'wrong persisted '+kind+' color')
        width = float(resolve(annotation.get('/BS', {})).get('/W', -1))
        require(math.isfinite(width) and abs(width-record['width']) < .01,
                'wrong persisted '+kind+' stroke width')
        matched.append((kind, annotation, color, width))
        unmatched.pop(i)
        checks.append(kind+' predicted geometry and persisted standard color/weight')
    require(not unmatched, 'unexpected owned markup')
    for name, path in [('original', args.original), ('edited', args.edited)]:
        subprocess.run(['pdftoppm', '-f', '1', '-singlefile', '-cropbox', '-scale-to',
                        '1200', '-png', str(path), str(out/name)],
                       check=True, capture_output=True, timeout=30)
    base = Image.open(out/'original.png').convert('RGB')
    image = Image.open(out/'edited.png').convert('RGB')
    require(base.size == image.size, 'independent dimensions changed')
    crop = tuple(float(v) for v in new.pages[0].cropbox)
    width, height = crop[2]-crop[0], crop[3]-crop[1]
    rotation = int(new.pages[0].get('/Rotate', 0)) % 360
    require(rotation in (0, 90, 180, 270), 'unsupported rotation')
    require(tuple(float(v) for v in old.pages[0].cropbox) == crop
            and int(old.pages[0].get('/Rotate', 0)) % 360 == rotation, 'page basis changed')

    def pixel(x, y):
        u, v = (x-crop[0])/width, (y-crop[1])/height
        a, b = ((u, 1-v), (v, u), (1-u, v), (1-v, 1-u))[rotation//90]
        return a*image.width, b*image.height

    def color_near(point, color, radius):
        x, y = point
        region = image.crop((max(0, int(x)-radius), max(0, int(y)-radius),
                             min(image.width, int(x)+radius+1), min(image.height, int(y)+radius+1)))
        flattened = getattr(region, 'get_flattened_data', None)
        pixels = flattened() if flattened else region.getdata()
        rgb = [round(c*255) for c in color]
        return any(isinstance(p, tuple) and len(p) >= 3
                   and all(abs(p[i]-rgb[i]) <= 28 for i in range(3)) for p in pixels)

    mask = Image.new('L', image.size, 255)
    draw = ImageDraw.Draw(mask)
    for kind, annotation, color, stroke in matched:
        rect = [float(v) for v in annotation['/Rect']]
        corners = [pixel(x, y) for x in (rect[0], rect[2]) for y in (rect[1], rect[3])]
        xs, ys = zip(*corners)
        draw.rectangle((math.floor(min(xs))-3, math.floor(min(ys))-3,
                        math.ceil(max(xs))+3, math.ceil(max(ys))+3), fill=0)
        scale = max(image.width/width, image.height/height)
        radius = max(3, math.ceil(stroke*scale))
        if kind in ('Line', 'Arrow'):
            x1, y1, x2, y2 = [float(v) for v in annotation['/L']]
            point = pixel((x1+x2)/2, (y1+y2)/2)
            if kind == 'Arrow':
                length = math.hypot(x2-x1, y2-y1)
                size = min(10, length*.4)
                ux, uy = (x2-x1)/length, (y2-y1)/length
                wings = [(x2-size*ux-size*.5*uy, y2-size*uy+size*.5*ux),
                         (x2-size*ux+size*.5*uy, y2-size*uy-size*.5*ux)]
                require(all(color_near(pixel(x, y), color, radius) for x, y in wings),
                        'independently rendered arrow wings missing')
                checks.append('independent Arrow wings at directed endpoint')
        else:
            point = pixel((rect[0]+rect[2])/2, rect[3]-min(stroke/2, (rect[3]-rect[1])/4))
        require(color_near(point, color, radius), 'independently rendered '+kind+' colored stroke missing')
        checks.append('independent '+kind+' colored vector stroke')
    outside = ImageChops.multiply(ImageChops.difference(base, image), mask.convert('RGB'))
    require(outside.getbbox() is None, 'original rendering changed outside owned markup bounds')
    checks.append('original rendering pixel-identical outside owned markup bounds')
    result = {'passed': True, 'renderer': 'Poppler', 'checks': checks,
              'original_sha256': hashlib.sha256(args.original.read_bytes()).hexdigest(),
              'edited_sha256': hashlib.sha256(args.edited.read_bytes()).hexdigest(), 'rotation': rotation}
    (out/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
