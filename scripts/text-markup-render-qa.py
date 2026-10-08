#!/usr/bin/env python3
"""Strict independent FreeText PDF + Poppler/OCR verifier (no application claims).

Run: uv run --with pillow --with pypdf scripts/text-markup-render-qa.py \
  --original before.pdf --edited after.pdf --expected expected.json --out proof

Expected JSON (--expected file path or inline JSON) is a nonempty list. Each entry:
  {"page": 0, "text": "QUARTZTEST (literal) \\ slash", "sentinel": "QUARTZTEST",
   "font_size": 18, "rect": [60, 180, 440, 240], "color": [0.1, 0.2, 0.7]}
page is zero-based physical page; text is exact decoded /Contents, including
literal parentheses/backslashes. sentinel is a unique ASCII alphanumeric word
(at least 6 chars) present in text, and must be independently OCR-readable.
font_size is physical points including UserUnit and AP/text transform scaling.
rect (optional) is PDF default user-space annotation Rect, not rotated UI pixels.
color (optional) is RGB text fill; /DA, AP text fill and actual ink are checked.
Optional font_tolerance defaults to 0.15 physical pt (maximum 0.5).
Ownership is presence of a non-null/truthy dictionary key --ownership-marker
(default /GlyphText), not magic values or invented additional producer metadata.
All edited owned annotations must exactly match this manifest. Original owned
bounds also form the outside-pixel exclusion region (moves/deletes supported).

--out must not exist or must be empty. Exit 0 means every strict gate passed;
exit 1 means rejected/error, recorded in result.json. --selftest --out must use
an absent dist/text-render-harness-selftest-* directory. No record-only mode.
Tools: pdftoppm + tesseract (PATH or --poppler/--tesseract); cached Poppler search
is used if unavailable on PATH. Every subprocess has a 45 second timeout.
See the authoring report for scope, conservatism and remaining native QA.
"""
import argparse
import hashlib
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw
from pypdf import PdfReader, PdfWriter
from pypdf.generic import (ArrayObject, BooleanObject, ContentStream,
                          DecodedStreamObject, DictionaryObject, FloatObject,
                          NameObject, NullObject, NumberObject, TextStringObject)

DPI = 144
MAX_PIXELS = 40_000_000
MAX_PAGES = 100
CORE14 = {'Courier', 'Courier-Bold', 'Courier-Oblique', 'Courier-BoldOblique',
          'Helvetica', 'Helvetica-Bold', 'Helvetica-Oblique', 'Helvetica-BoldOblique',
          'Times-Roman', 'Times-Bold', 'Times-Italic', 'Times-BoldItalic',
          'Symbol', 'ZapfDingbats'}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def obj(value):
    return value.get_object() if hasattr(value, 'get_object') else value


def numbers(value, count, label):
    require(value is not None and len(value) == count, f'{label}: need {count} numbers')
    result = [float(v) for v in value]
    require(all(math.isfinite(v) and abs(v) <= 1e7 for v in result), f'{label}: nonfinite/unbounded')
    return result


def rect(value, label):
    r = numbers(value, 4, label)
    require(r[2] > r[0] and r[3] > r[1], f'{label}: empty/inverted')
    return r


def canonical(value, active=None, root=False):
    """Compare decoded semantics, never xref IDs; detect cycles by recursion path."""
    active = set() if active is None else active
    value = obj(value)
    if isinstance(value, (dict, list, tuple)):
        ident = id(value)
        if ident in active:
            return {'cycle': type(value).__name__}
        active.add(ident)
        try:
            if isinstance(value, dict):
                ignored = {'/Length', '/Filter', '/DecodeParms'} if hasattr(value, 'get_data') else set()
                if root:
                    ignored.add('/P')  # page backlink is separately checked via page membership
                out = {str(k): canonical(v, active) for k, v in value.items() if str(k) not in ignored}
                if hasattr(value, 'get_data'):
                    out['decoded_sha256'] = hashlib.sha256(value.get_data()).hexdigest()
                return out
            return [canonical(v, active) for v in value]
        finally:
            active.remove(ident)
    if isinstance(value, bytes):
        return {'bytes': value.hex()}
    if value is None:
        return None
    if isinstance(value, BooleanObject):
        return bool(value.value)
    if isinstance(value, (int, float)):
        return float(value)
    return str(value)


def owned(a, marker):
    if marker not in a:
        return False
    value = obj(a[marker])
    if isinstance(value, BooleanObject):
        return bool(value.value)
    return value is not None and not isinstance(value, NullObject) and bool(value)


def basis(page):
    media = rect(page.mediabox, 'MediaBox')
    crop = rect(page.cropbox, 'CropBox')
    require(all(media[i] <= crop[i] for i in (0, 1)) and
            all(crop[i] <= media[i] for i in (2, 3)), 'crop outside media')
    rotation = float(page.get('/Rotate', 0))
    require(math.isfinite(rotation) and rotation in (0, 90, 180, 270), 'unsupported Rotate')
    unit = float(page.get('/UserUnit', 1))
    require(math.isfinite(unit) and 0 < unit <= 100, 'invalid/unbounded UserUnit')
    require((crop[2]-crop[0]) * (crop[3]-crop[1]) * (DPI/72*unit)**2 <= MAX_PIXELS,
            'page raster budget exceeded')
    return {'media': media, 'crop': crop, 'rotation': int(rotation), 'unit': unit,
            'boxes': {k: canonical(page.get(k)) for k in ('/BleedBox', '/TrimBox', '/ArtBox')}}


def streams(page):
    raw = obj(page.get('/Contents'))
    if raw is None:
        return []
    return [obj(x).get_data() for x in (raw if isinstance(raw, list) else [raw])]


def annotations(page):
    return [obj(a) for a in page.get('/Annots', [])]


def within(r, container, label):
    require(r[0] >= container[0] and r[1] >= container[1] and
            r[2] <= container[2] and r[3] <= container[3], f'{label}: outside bounds')


def matrix(values, label):
    m = numbers(values, 6, label)
    require(abs(m[0]*m[3]-m[1]*m[2]) > 1e-10, f'{label}: singular')
    return m


def mul(a, b):
    return [a[0]*b[0]+a[2]*b[1], a[1]*b[0]+a[3]*b[1],
            a[0]*b[2]+a[2]*b[3], a[1]*b[2]+a[3]*b[3],
            a[0]*b[4]+a[2]*b[5]+a[4], a[1]*b[4]+a[3]*b[5]+a[5]]


def point(m, x, y):
    return m[0]*x+m[2]*y+m[4], m[1]*x+m[3]*y+m[5]


def sfnt_tables(data):
    require(len(data) >= 12 and data[:4] in (b'\x00\x01\x00\x00', b'OTTO', b'true'), 'invalid SFNT font signature')
    count = struct.unpack_from('>H', data, 4)[0]
    require(0 < count <= 256 and 12+16*count <= len(data), 'invalid font table directory')
    tables = {}
    for i in range(count):
        tag, checksum, offset, length = struct.unpack_from('>4sIII', data, 12+16*i)
        require(length > 0 and offset+length <= len(data), 'font table outside embedded program')
        tables[tag] = data[offset:offset+length]
    require(all(tag in tables for tag in (b'head', b'hhea', b'hmtx', b'maxp', b'cmap')),
            'embedded SFNT lacks required font tables')
    require(b'glyf' in tables or b'CFF ' in tables or b'CFF2' in tables, 'embedded font lacks glyph outlines')
    return tables


def font_valid(font):
    font = obj(font)
    require(font.get('/Type') == '/Font', 'resource is not a Font')
    subtype = font.get('/Subtype')
    base = str(font.get('/BaseFont', '')).lstrip('/')
    descriptor_hint = obj(font.get('/FontDescriptor'))
    has_program = isinstance(descriptor_hint, dict) and any(k in descriptor_hint for k in ('/FontFile', '/FontFile2', '/FontFile3'))
    if subtype == '/Type1' and base in CORE14 and not has_program:
        return base
    require(subtype in ('/Type0', '/TrueType', '/Type1'), 'unsupported font subtype')
    target = font
    if subtype == '/Type0':
        descendants = obj(font.get('/DescendantFonts'))
        require(descendants and len(descendants) == 1, 'Type0 needs one descendant')
        target = obj(descendants[0])
        require(target.get('/Subtype') in ('/CIDFontType0', '/CIDFontType2'), 'invalid CID font')
        require(font.get('/Encoding') is not None, 'Type0 missing Encoding')
    descriptor = obj(target.get('/FontDescriptor'))
    require(isinstance(descriptor, dict), 'non-core font missing descriptor')
    key = next((k for k in ('/FontFile', '/FontFile2', '/FontFile3') if k in descriptor), None)
    embedded = obj(descriptor[key]) if key else None
    require(hasattr(embedded, 'get_data') and len(embedded.get_data()) >= 64,
            'non-core font missing/empty embedded program')
    data = embedded.get_data()
    if key == '/FontFile2' or embedded.get('/Subtype') == '/OpenType':
        sfnt_tables(data)
    elif key == '/FontFile':
        require(data.startswith((b'%!PS', b'\x80\x01')), 'invalid Type1 program signature')
    else:
        require(embedded.get('/Subtype') in ('/Type1C', '/CIDFontType0C') and
                data[0] in (1, 2) and 4 <= data[2] <= len(data), 'invalid embedded CFF font')
    return base


def rgb(operands, operator):
    if operator == b'rg':
        c = numbers(operands, 3, 'RGB fill')
    elif operator == b'g':
        g = numbers(operands, 1, 'gray fill')[0]
        c = [g]*3
    elif operator == b'k':
        a, b, c0, k = numbers(operands, 4, 'CMYK fill')
        c = [1-min(1, v+k) for v in (a, b, c0)]
    else:
        raise ValueError('unsupported text fill color space')
    require(all(0 <= v <= 1 for v in c), 'fill color out of range')
    return c


def appearance(a, expected, reader, unit):
    require(a.get('/Subtype') == '/FreeText', 'owned annotation is not FreeText')
    require(str(a.get('/Contents', '')) == expected['text'], 'Contents mismatch')
    require(not int(a.get('/F', 0)) & (1 | 2 | 32), 'annotation invisible/hidden/NoView')
    r = rect(a.get('/Rect'), 'annotation Rect')
    if 'rect' in expected:
        require(all(abs(x-y) <= 0.01 for x, y in zip(r, expected['rect'])), 'expected Rect mismatch')
    ap = obj(a.get('/AP'))
    require(isinstance(ap, dict) and '/N' in ap, 'missing normal appearance')
    normal = obj(ap['/N'])
    if not hasattr(normal, 'get_data'):
        require(isinstance(normal, dict) and a.get('/AS') in normal, 'unresolved appearance state')
        normal = obj(normal[a['/AS']])
    require(hasattr(normal, 'get_data') and normal.get('/Subtype') == '/Form', 'normal AP not Form stream')
    require(len(normal.get_data()) > 0, 'empty normal appearance')
    bbox = rect(normal.get('/BBox'), 'AP BBox')
    am = matrix(normal.get('/Matrix', [1, 0, 0, 1, 0, 0]), 'AP Matrix')
    corners = [point(am, x, y) for x in (bbox[0], bbox[2]) for y in (bbox[1], bbox[3])]
    ax, ay = min(v[0] for v in corners), min(v[1] for v in corners)
    bx, by = max(v[0] for v in corners), max(v[1] for v in corners)
    require(bx > ax and by > ay, 'degenerate transformed AP')
    sx, sy = (r[2]-r[0])/(bx-ax), (r[3]-r[1])/(by-ay)
    fit = [sx, 0, 0, sy, r[0]-sx*ax, r[1]-sy*ay]
    resources = obj(normal.get('/Resources'))
    require(isinstance(resources, dict), 'AP missing Resources')
    fonts = obj(resources.get('/Font'))
    require(isinstance(fonts, dict) and fonts, 'AP missing font resources')
    fonts_used, sizes, colors = [], [], []
    ident = [1, 0, 0, 1, 0, 0]
    ctm, tm, fill, selected, size, render_mode = ident[:], ident[:], [0, 0, 0], None, None, 0
    stack, in_text, shows = [], False, 0
    for args, op in ContentStream(normal, reader).operations:
        if op == b'q':
            stack.append((ctm[:], fill[:], selected, size, render_mode))
        elif op == b'Q':
            require(stack, 'unbalanced AP Q')
            ctm, fill, selected, size, render_mode = stack.pop()
        elif op == b'cm':
            ctm = mul(ctm, matrix(args, 'AP cm'))
        elif op == b'BT':
            require(not in_text, 'nested BT')
            in_text, tm = True, ident[:]
        elif op == b'ET':
            require(in_text, 'ET outside text')
            in_text = False
        elif op == b'Tf':
            require(len(args) == 2 and args[0] in fonts, 'unresolved AP Tf font')
            selected = args[0]
            size = float(args[1])
            require(math.isfinite(size) and size > 0, 'invalid AP fontsize')
            fonts_used.append(font_valid(fonts[selected]))
        elif op == b'Tm':
            tm = matrix(args, 'text matrix')
        elif op == b'Tr':
            render_mode = int(args[0])
        elif op in (b'rg', b'g', b'k'):
            fill = rgb(args, op)
        elif op in (b'cs', b'sc', b'scn', b'Do', b'gs'):
            raise ValueError('unsupported AP color/XObject/ExtGState operator (fail closed)')
        elif op in (b'Tj', b'TJ', b"'", b'"'):
            require(in_text and selected is not None and size is not None, 'text show missing BT/font')
            require(render_mode == 0, 'text must use visible fill rendering mode 0')
            values = args[0] if op == b'TJ' else [args[-1]]
            require(any(isinstance(v, (str, bytes)) and len(v) for v in values), 'empty text show')
            transform = mul(mul(fit, am), mul(ctm, tm))
            physical = size * math.hypot(transform[2], transform[3]) * unit
            require(math.isfinite(physical) and abs(physical-expected['font_size']) <= expected.get('font_tolerance', 0.15),
                    f'physical fontsize {physical:g} != {expected["font_size"]:g}')
            sizes.append(physical)
            colors.append(fill[:])
            shows += 1
    require(not stack and not in_text and shows, 'AP missing text or unbalanced state')
    da = a.get('/DA')
    require(isinstance(da, (str, bytes)) and len(da), 'missing default appearance')
    ds = DecodedStreamObject()
    ds.set_data(da.encode('latin1') if isinstance(da, str) else da)
    da_size, da_color = None, [0, 0, 0]
    for args, op in ContentStream(ds, reader).operations:
        if op == b'Tf':
            require(len(args) == 2 and args[0] in fonts, 'DA font unresolved in AP Resources')
            font_valid(fonts[args[0]])
            da_size = float(args[1]) * unit
        elif op in (b'rg', b'g', b'k'):
            da_color = rgb(args, op)
    require(da_size is not None and math.isfinite(da_size) and
            abs(da_size-expected['font_size']) <= expected.get('font_tolerance', 0.15), 'DA physical fontsize mismatch')
    if 'color' in expected:
        require(all(max(abs(a-b) for a, b in zip(c, expected['color'])) <= 0.015 for c in colors+[da_color]),
                'text color mismatch in AP/DA')
    return {'rect': r, 'physical_sizes': sizes, 'fonts': fonts_used, 'text_colors': colors}


def tool(name, explicit=None):
    if explicit:
        p = Path(explicit).resolve()
        require(p.is_file() and os.access(p, os.X_OK), f'invalid tool: {p}')
        return str(p)
    found = shutil.which(name)
    if found:
        return found
    if name == 'pdftoppm':
        for root in (Path.home()/'.cache', Path.home()/'.local/share'):
            for p in root.glob(f'**/{name}'):
                if p.is_file() and os.access(p, os.X_OK):
                    return str(p)
    raise ValueError(f'missing required tool: {name}')


def run(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=45)
    require(result.returncode == 0, f'tool failed {command[0]}: {result.stderr[-3000:]}')
    return result.stdout


def render(poppler, pdf, page, prefix, crop=None, dpi=DPI):
    cmd = [poppler, '-f', str(page+1), '-l', str(page+1), '-singlefile', '-cropbox',
           '-r', str(dpi), '-png']
    if crop:
        x, y, w, h = crop
        cmd += ['-x', str(x), '-y', str(y), '-W', str(w), '-H', str(h)]
    run(cmd+[str(pdf), str(prefix)])
    image = Image.open(str(prefix)+'.png').convert('RGB')
    require(image.width*image.height <= MAX_PIXELS, 'render exceeds pixel budget')
    return image


def pixel_rect(r, b, width, height):
    c = b['crop']
    w, h = c[2]-c[0], c[3]-c[1]
    def convert(x, y):
        u, v = (x-c[0])/w, (c[3]-y)/h
        return {0: (u, v), 90: (1-v, u), 180: (1-u, 1-v), 270: (v, 1-u)}[b['rotation']]
    pts = [convert(x, y) for x in (r[0], r[2]) for y in (r[1], r[3])]
    return (max(0, math.floor(min(x for x, y in pts)*width)),
            max(0, math.floor(min(y for x, y in pts)*height)),
            min(width, math.ceil(max(x for x, y in pts)*width)),
            min(height, math.ceil(max(y for x, y in pts)*height)))


def ocr(tesseract, image_path, sentinel, want):
    # OCR only pixels emitted by Poppler's crop operation. Try raster rotations,
    # not PDF text extraction, to support rotated pages/AP text orientations.
    source = Image.open(image_path).convert('RGB')
    attempts = []
    for angle in (0, 90, 180, 270):
        path = Path(image_path)
        if angle:
            path = path.with_name(path.stem+f'-ocr-rotate-{angle}.png')
            source.rotate(angle, expand=True).save(path)
        text = run([tesseract, str(path), 'stdout', '--psm', '6', '-l', 'eng'])
        attempts.append({'raster_rotation': angle, 'text': text})
        if sentinel in re.findall(r'[A-Za-z0-9]+', text):
            require(want, f'original crop already contains OCR sentinel {sentinel!r}')
            return {'matched_rotation': angle, 'text': text, 'attempts': attempts}
    require(not want, f'OCR sentinel {sentinel!r} missing in all raster orientations: {attempts!r}')
    return {'matched_rotation': None, 'attempts': attempts}


def manifest(path):
    raw = str(path)
    values = json.loads(raw if raw.lstrip().startswith('[') else Path(path).read_text())
    require(isinstance(values, list) and 0 < len(values) <= 1000, 'expected must be nonempty bounded JSON list')
    seen = set()
    for e in values:
        require(isinstance(e, dict), 'expected entry must be object')
        require(set(e) <= {'page', 'text', 'sentinel', 'font_size', 'rect', 'color', 'font_tolerance'}, 'unknown expected field')
        require(isinstance(e.get('page'), int) and not isinstance(e['page'], bool) and e['page'] >= 0, 'invalid page')
        require(isinstance(e.get('text'), str) and e['text'], 'missing expected text')
        s = e.get('sentinel')
        require(isinstance(s, str) and re.fullmatch(r'[A-Za-z0-9]{6,}', s) and s in e['text'], 'invalid/missing ASCII sentinel')
        require(s not in seen, 'sentinels must be unique')
        seen.add(s)
        size = float(e.get('font_size', 0))
        require(math.isfinite(size) and 0 < size <= 1000, 'invalid font_size')
        tolerance = float(e.get('font_tolerance', 0.15))
        require(math.isfinite(tolerance) and 0 <= tolerance <= 0.5, 'invalid font_tolerance')
        if 'rect' in e:
            rect(e['rect'], 'expected rect')
        if 'color' in e:
            require(all(0 <= c <= 1 for c in numbers(e['color'], 3, 'expected color')), 'invalid color')
    for e in values:
        require(sum(e['sentinel'] in x['text'] for x in values) == 1, 'sentinel not unique among expected texts')
    return values


def outside_unchanged(original_image, edited_image, bounds, page_basis, page_index):
    require(original_image.size == edited_image.size, 'raster dimensions changed')
    w, h = original_image.size
    delta = ImageChops.difference(original_image, edited_image)
    mask = Image.new('L', (w, h), 255)
    draw = ImageDraw.Draw(mask)
    for r in bounds:
        x0, y0, x1, y1 = pixel_rect(r, page_basis, w, h)
        # Only pixel cells intersecting Rect; no extra AA/UI padding relaxation.
        draw.rectangle((x0, y0, x1-1, y1-1), fill=0)
    require(ImageChops.multiply(delta, Image.merge('RGB', (mask, mask, mask))).getbbox() is None,
            f'page {page_index}: raster changed outside owned bounds')


def verify(original, edited, expected_path, out, marker, poppler, tesseract):
    require(marker.startswith('/') and len(marker) > 1, 'ownership marker must be PDF name')
    expected = manifest(expected_path)
    before, after = PdfReader(original, strict=True), PdfReader(edited, strict=True)
    require(not before.is_encrypted and not after.is_encrypted, 'encrypted input unsupported')
    require(0 < len(before.pages) == len(after.pages) <= MAX_PAGES, 'page count changed or over budget')
    require(all(e['page'] < len(after.pages) for e in expected), 'expected page out of range')
    result = {'status': 'pass', 'kind': 'strict-independent-PDF-Poppler-OCR',
              'original_sha256': hashlib.sha256(Path(original).read_bytes()).hexdigest(),
              'edited_sha256': hashlib.sha256(Path(edited).read_bytes()).hexdigest(),
              'ownership_marker': marker, 'dpi': DPI, 'poppler': poppler, 'tesseract': tesseract,
              'pages': [], 'annotations': []}
    for i, (p, q) in enumerate(zip(before.pages, after.pages)):
        b = basis(p)
        require(b == basis(q), f'page {i}: basis changed')
        require(streams(p) == streams(q), f'page {i}: decoded original streams/count changed')
        require(canonical(p.get('/Resources')) == canonical(q.get('/Resources')), f'page {i}: original resources changed')
        pa, qa = annotations(p), annotations(q)
        foreign_before = [canonical(a, root=True) for a in pa if not owned(a, marker)]
        foreign_after = [canonical(a, root=True) for a in qa if not owned(a, marker)]
        require(foreign_before == foreign_after, f'page {i}: foreign annotations changed')
        old_owned = [a for a in pa if owned(a, marker)]
        new_owned = [a for a in qa if owned(a, marker)]
        entries = [e for e in expected if e['page'] == i]
        require(len(new_owned) == len(entries), f'page {i}: owned annotation count mismatch')
        used, details = set(), []
        for e in entries:
            candidates = [(j, a) for j, a in enumerate(new_owned) if str(a.get('/Contents', '')) == e['text'] and j not in used]
            require(len(candidates) == 1, f'page {i}: missing/ambiguous expected Contents')
            j, a = candidates[0]
            used.add(j)
            d = appearance(a, e, after, b['unit'])
            within(d['rect'], b['crop'], 'owned Rect')
            d.update({'page': i, 'sentinel': e['sentinel'], 'expected_text': e['text']})
            details.append((e, d))
        all_bounds = [rect(a.get('/Rect'), 'original owned Rect') for a in old_owned] + [d['rect'] for e, d in details]
        for r in all_bounds:
            within(r, b['crop'], 'owned bounds')
        for j, (_, d) in enumerate(details):
            r = d['rect']
            for _, other in details[j+1:]:
                t = other['rect']
                require(min(r[2], t[2]) <= max(r[0], t[0]) or min(r[3], t[3]) <= max(r[1], t[1]),
                        'overlapping owned annotations prevent independent OCR attribution')
        original_image = render(poppler, original, i, out/f'page-{i}-original')
        # Poppler versions may ignore /UserUnit. Normalize the actual raster to
        # physical 144 dpi based on observed dimensions, never silently pretend
        # unscaled pixels are physical-point evidence.
        unscaled = [(b['crop'][2]-b['crop'][0])*DPI/72, (b['crop'][3]-b['crop'][1])*DPI/72]
        if b['rotation'] in (90, 270):
            unscaled.reverse()
        physical_size = [v*b['unit'] for v in unscaled]
        near = lambda actual, wanted: all(abs(a-z) <= 1.01 for a, z in zip(actual, wanted))
        page_dpi = DPI
        correction = False
        if not near(original_image.size, physical_size):
            require(b['unit'] != 1 and near(original_image.size, unscaled), 'unexpected Poppler physical raster dimensions')
            page_dpi = DPI*b['unit']
            correction = True
            original_image = render(poppler, original, i, out/f'page-{i}-original', dpi=page_dpi)
        require(near(original_image.size, physical_size), 'UserUnit physical raster normalization failed')
        edited_image = render(poppler, edited, i, out/f'page-{i}-edited', dpi=page_dpi)
        require(original_image.size == edited_image.size, 'raster dimensions changed')
        w, h = original_image.size
        outside_unchanged(original_image, edited_image, all_bounds, b, i)
        for j, (e, d) in enumerate(details):
            box = pixel_rect(d['rect'], b, w, h)
            require(box[2] > box[0] and box[3] > box[1], 'empty annotation crop')
            crop_spec = (box[0], box[1], box[2]-box[0], box[3]-box[1])
            ep, op = out/f'page-{i}-text-{j}-edited-crop', out/f'page-{i}-text-{j}-original-crop'
            ec = render(poppler, edited, i, ep, crop_spec, dpi=page_dpi)
            oc = render(poppler, original, i, op, crop_spec, dpi=page_dpi)
            require(ec.size == oc.size == (crop_spec[2], crop_spec[3]), 'Poppler crop dimensions mismatch')
            diff = ImageChops.difference(ec, oc)
            pixels = getattr(diff, 'get_flattened_data', diff.getdata)()
            changed = sum(max(pixel) >= 24 for pixel in pixels)
            require(changed >= max(20, int(ec.width*ec.height*0.0005)), 'missing actual visible ink/delta')
            if 'color' in e:
                target = [round(v*255) for v in e['color']]
                data = getattr(ec, 'get_flattened_data', ec.getdata)()
                colored = sum(max(abs(v-t) for v, t in zip(pixel, target)) <= 35 for pixel in data)
                require(colored >= 12, 'expected RGB ink not present in actual raster')
                d['color_ink_pixels'] = colored
            d['ocr'] = ocr(tesseract, str(ep)+'.png', e['sentinel'], True)
            # The sentinel may already exist in an original owned text being restyled.
            same_old = any(str(a.get('/Contents', '')) == e['text'] for a in old_owned)
            if not same_old:
                ocr(tesseract, str(op)+'.png', e['sentinel'], False)
            d['changed_ink_pixels'] = changed
            d['pixel_crop'] = list(box)
            result['annotations'].append(d)
        result['pages'].append({'page': i, 'basis': b, 'raster_size': [w, h],
                                'poppler_render_dpi': page_dpi, 'userunit_dpi_correction': correction,
                                'decoded_stream_count': len(streams(p)), 'foreign_count': len(foreign_before),
                                'outside_bounds_pixel_identical': True})
    return result


def fixture_embedded_font(writer, font_path):
    """Build a real Identity-H Type0 fixture from a system TTF, with Unicode CIDs.
    Minimal format-4 cmap reader keeps the harness dependency set Pillow+pypdf.
    """
    data = Path(font_path).read_bytes()
    tables = sfnt_tables(data)
    cmap = tables[b'cmap']
    sub = None
    for i in range(struct.unpack_from('>H', cmap, 2)[0]):
        platform, encoding, offset = struct.unpack_from('>HHI', cmap, 4+8*i)
        if platform in (0, 3) and struct.unpack_from('>H', cmap, offset)[0] == 4:
            sub = cmap[offset:]
            break
    require(sub is not None, 'selftest TTF lacks format4 cmap')
    n = struct.unpack_from('>H', sub, 6)[0]//2
    end_at, start_at, delta_at, range_at = 14, 16+2*n, 16+4*n, 16+6*n
    gid_map = bytearray(256)
    for code in range(128):
        for j in range(n):
            end = struct.unpack_from('>H', sub, end_at+2*j)[0]
            start = struct.unpack_from('>H', sub, start_at+2*j)[0]
            if start <= code <= end:
                delta = struct.unpack_from('>h', sub, delta_at+2*j)[0]
                offset = struct.unpack_from('>H', sub, range_at+2*j)[0]
                gid = ((code+delta) & 65535) if not offset else struct.unpack_from(
                    '>H', sub, range_at+2*j+offset+2*(code-start))[0]
                if offset and gid:
                    gid = (gid+delta) & 65535
                struct.pack_into('>H', gid_map, 2*code, gid)
                break
    def stream(raw):
        value = DecodedStreamObject()
        value.set_data(raw)
        return writer._add_object(value)
    nums = lambda values: ArrayObject([FloatObject(v) for v in values])
    descriptor = DictionaryObject({NameObject('/Type'): NameObject('/FontDescriptor'),
        NameObject('/FontName'): NameObject('/FixtureEmbedded'), NameObject('/Flags'): NumberObject(32),
        NameObject('/FontBBox'): nums([-1000, -1000, 2500, 2500]), NameObject('/ItalicAngle'): NumberObject(0),
        NameObject('/Ascent'): NumberObject(1000), NameObject('/Descent'): NumberObject(-300),
        NameObject('/CapHeight'): NumberObject(700), NameObject('/StemV'): NumberObject(80),
        NameObject('/FontFile2'): stream(data)})
    descendant = DictionaryObject({NameObject('/Type'): NameObject('/Font'), NameObject('/Subtype'): NameObject('/CIDFontType2'),
        NameObject('/BaseFont'): NameObject('/FixtureEmbedded'), NameObject('/FontDescriptor'): writer._add_object(descriptor),
        NameObject('/CIDSystemInfo'): DictionaryObject({NameObject('/Registry'): TextStringObject('Adobe'),
            NameObject('/Ordering'): TextStringObject('Identity'), NameObject('/Supplement'): NumberObject(0)}),
        NameObject('/CIDToGIDMap'): stream(bytes(gid_map)), NameObject('/DW'): NumberObject(600)})
    return DictionaryObject({NameObject('/Type'): NameObject('/Font'), NameObject('/Subtype'): NameObject('/Type0'),
        NameObject('/BaseFont'): NameObject('/FixtureEmbedded'), NameObject('/Encoding'): NameObject('/Identity-H'),
        NameObject('/DescendantFonts'): ArrayObject([writer._add_object(descendant)])})


def fixture(path, edited=False, defect=None, rotation=0, unit=1, font_path=None):
    """Only generated public fixture PDFs; never application-produced evidence."""
    nums = lambda x: ArrayObject([FloatObject(v) for v in x])
    w = PdfWriter()
    p = w.add_blank_page(width=600, height=400)
    p[NameObject('/Rotate')] = NumberObject(rotation)
    p[NameObject('/UserUnit')] = FloatObject(unit)
    p[NameObject('/CropBox')] = nums([20, 20, 580, 380])
    page_stream = DecodedStreamObject()
    page_stream.set_data(b'q 0.85 0.85 0.85 rg 30 30 25 25 re f Q\n')
    if defect == 'page-stream':
        page_stream.set_data(b'q 1 0 0 rg 30 30 25 25 re f Q\n')
    p[NameObject('/Contents')] = w._add_object(page_stream)
    p[NameObject('/Resources')] = DictionaryObject()
    foreign = DictionaryObject({NameObject('/Type'): NameObject('/Annot'), NameObject('/Subtype'): NameObject('/Link'),
        NameObject('/Rect'): nums([30, 30, 55, 55]), NameObject('/Border'): nums([0, 0, 0]),
        NameObject('/Contents'): TextStringObject('corrupted' if defect == 'foreign' else 'foreign link preserved'),
        NameObject('/A'): DictionaryObject({NameObject('/S'): NameObject('/URI'), NameObject('/URI'): TextStringObject('https://example.invalid')})})
    p[NameObject('/Annots')] = ArrayObject([w._add_object(foreign)])
    text = 'QUARTZTEST (literal) \\ slash'
    physical = 18*unit
    expected = [{'page': 0, 'text': text, 'sentinel': 'QUARTZTEST', 'font_size': physical,
                 'rect': [60, 180, 500, 250], 'color': [0.1, 0.2, 0.7]}]
    if edited:
        font = DictionaryObject({NameObject('/Type'): NameObject('/Font'), NameObject('/Subtype'): NameObject('/Type1'),
                                 NameObject('/BaseFont'): NameObject('/Helvetica')})
        if font_path:
            font = fixture_embedded_font(w, font_path)
        if defect == 'fake-embedded-font':
            fake = DecodedStreamObject()
            fake.set_data(b'not a real font program'*10)
            font[NameObject('/FontDescriptor')] = DictionaryObject({NameObject('/FontFile2'): w._add_object(fake)})
        resources = DictionaryObject({NameObject('/Font'): DictionaryObject({NameObject('/F0'): w._add_object(font)})})
        ap = DecodedStreamObject()
        literal = text.replace('\\', '\\\\').replace('(', '\\(').replace(')', '\\)')
        shown = '<'+text.encode('utf-16-be').hex()+'>' if font_path else '('+literal+')'
        stream = f'q 0.1 0.2 0.7 rg BT /F0 18 Tf 1 0 0 1 10 30 Tm {shown} Tj ET Q\n'.encode()
        if defect == 'empty-text':
            stream = b'q 0.1 0.2 0.7 rg BT /F0 18 Tf 1 0 0 1 10 30 Tm () Tj ET Q\n'
        elif defect == 'wrong-render-text':
            stream = b'q 0.1 0.2 0.7 rg BT /F0 18 Tf 1 0 0 1 10 30 Tm (COUNTERFEIT) Tj ET Q\n'
        elif defect == 'invisible-text':
            stream = stream.replace(b'BT ', b'BT 3 Tr ')
        elif defect == 'no-visible-ink':
            stream = b'q 0 0 0 0 re W n '+stream+b' Q\n'
        elif defect == 'wrong-color':
            stream = stream.replace(b'0.1 0.2 0.7 rg', b'0.9 0.1 0.1 rg')
        elif defect == 'scaled-font':
            stream = stream.replace(b'1 0 0 1 10 30 Tm', b'1 0 0 0.5 10 30 Tm')
        ap.set_data(stream)
        ap.update({NameObject('/Type'): NameObject('/XObject'), NameObject('/Subtype'): NameObject('/Form'),
                   NameObject('/BBox'): nums([0, 0, 440, 70]), NameObject('/Resources'): resources})
        a = DictionaryObject({NameObject('/Type'): NameObject('/Annot'), NameObject('/Subtype'): NameObject('/FreeText'),
            NameObject('/GlyphText'): BooleanObject(True), NameObject('/Contents'): TextStringObject(text),
            NameObject('/Rect'): nums([60, 180, 500, 250]), NameObject('/F'): NumberObject(4),
            NameObject('/DA'): TextStringObject('/F0 18 Tf 0.1 0.2 0.7 rg'),
            NameObject('/AP'): DictionaryObject({NameObject('/N'): w._add_object(ap)})})
        if defect == 'missing-ap':
            del a['/AP']
        if defect == 'missing-font':
            del resources['/Font']
        if defect == 'wrong-font-size':
            ap.set_data(stream.replace(b'18 Tf', b'12 Tf'))
        if defect == 'missing-contents':
            del a['/Contents']
        if defect == 'bad-matrix':
            ap[NameObject('/Matrix')] = nums([0, 0, 0, 0, 0, 0])
        p['/Annots'].append(w._add_object(a))
    with path.open('wb') as f:
        w.write(f)
    return expected


def selftest(out, poppler, tesseract):
    require(out.name.startswith('text-render-harness-selftest-') and out.parent.name == 'dist',
            'selftests must be under dist/text-render-harness-selftest-*')
    rows = []
    # Exercise the exact production pixel mask with generated image fixtures,
    # including a one-pixel corruption immediately outside the owned rectangle.
    base = Image.new('RGB', (100, 100), 'white')
    inside = base.copy()
    inside.putpixel((20, 20), (0, 0, 0))
    test_basis = {'crop': [0, 0, 100, 100], 'rotation': 0}
    outside_unchanged(base, inside, [[20, 20, 80, 80]], test_basis, 0)
    rows.append({'case': 'pixel-mask-owned-boundary-positive', 'status': 'pass'})
    outside = inside.copy()
    outside.putpixel((19, 20), (254, 255, 255))
    base.save(out/'pixel-mask-original.png')
    inside.save(out/'pixel-mask-inside.png')
    outside.save(out/'pixel-mask-corrupt-outside.png')
    try:
        outside_unchanged(base, outside, [[20, 20, 80, 80]], test_basis, 0)
    except ValueError as e:
        require('raster changed outside owned bounds' in str(e), 'wrong pixel-mask rejection')
        rows.append({'case': 'pixel-mask-one-pixel-outside-negative', 'status': 'correctly-rejected', 'reason': str(e)})
    else:
        raise ValueError('selftest false acceptance: outside pixel')
    # Rotation OCR uses Poppler crop followed by explicit raster unrotation.
    # Positive basic fixture proves literal escaping, crop, real core14 font, color and OCR.
    before, after, exp = out/'original.pdf', out/'edited.pdf', out/'expected.json'
    fixture(before)
    expected = fixture(after, True)
    exp.write_text(json.dumps(expected, indent=2))
    proof = out/'positive'
    proof.mkdir()
    result = verify(before, after, exp, proof, '/GlyphText', poppler, tesseract)
    (proof/'result.json').write_text(json.dumps(result, indent=2))
    rows.append({'case': 'positive-core14-literal-escaping-cropped', 'status': 'pass'})
    for rotation, unit in ((90, 1), (180, 1), (270, 1), (0, 2)):
        label = f'positive-rotate-{rotation}-unit-{unit}'
        sub = out/label
        sub.mkdir()
        fixture(sub/'original.pdf', rotation=rotation, unit=unit)
        values = fixture(sub/'edited.pdf', True, rotation=rotation, unit=unit)
        (sub/'expected.json').write_text(json.dumps(values))
        proof_dir = sub/'proof'
        proof_dir.mkdir()
        got = verify(sub/'original.pdf', sub/'edited.pdf', sub/'expected.json', proof_dir,
                     '/GlyphText', poppler, tesseract)
        (proof_dir/'result.json').write_text(json.dumps(got, indent=2))
        rows.append({'case': label, 'status': 'pass'})
    font_candidates = [p for p in Path('/usr/share/fonts').glob('**/*.ttf')
                       if p.name in ('AdwaitaSans-Regular.ttf', 'DejaVuSans.ttf', 'LiberationSans-Regular.ttf')]
    require(font_candidates, 'selftest requires a system sans TTF for real embedded Type0 coverage')
    sub = out/'positive-embedded-type0'
    sub.mkdir()
    fixture(sub/'original.pdf')
    values = fixture(sub/'edited.pdf', True, font_path=font_candidates[0])
    (sub/'expected.json').write_text(json.dumps(values))
    proof_dir = sub/'proof'
    proof_dir.mkdir()
    got = verify(sub/'original.pdf', sub/'edited.pdf', sub/'expected.json', proof_dir,
                 '/GlyphText', poppler, tesseract)
    (proof_dir/'result.json').write_text(json.dumps(got, indent=2))
    rows.append({'case': 'positive-embedded-type0', 'status': 'pass', 'fixture_font': str(font_candidates[0])})
    rejection_reasons = {
        'missing-ap': 'missing normal appearance', 'empty-text': 'empty text show',
        'wrong-render-text': 'missing in all raster orientations', 'foreign': 'foreign annotations changed',
        'page-stream': 'decoded original streams/count changed', 'invisible-text': 'visible fill rendering mode',
        'missing-font': 'missing font resources', 'wrong-font-size': 'physical fontsize',
        'missing-contents': 'missing/ambiguous expected Contents', 'bad-matrix': 'singular',
        'no-visible-ink': 'missing actual visible ink/delta', 'wrong-color': 'text color mismatch',
        'scaled-font': 'physical fontsize', 'fake-embedded-font': 'invalid SFNT font signature'}
    for defect in rejection_reasons:
        bad = out/f'{defect}.pdf'
        fixture(bad, True, defect)
        sub = out/defect
        sub.mkdir()
        try:
            verify(before, bad, exp, sub, '/GlyphText', poppler, tesseract)
        except ValueError as e:
            require(rejection_reasons[defect] in str(e), f'selftest wrong rejection for {defect}: {e}')
            rows.append({'case': defect, 'status': 'correctly-rejected', 'reason': str(e)})
        else:
            raise ValueError(f'selftest false acceptance: {defect}')
    return {'status': 'pass', 'kind': 'generated-harness-selftests-only-not-application-verification', 'cases': rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--original', type=Path)
    parser.add_argument('--edited', type=Path)
    parser.add_argument('--expected', help='JSON-list file path or inline JSON list')
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--ownership-marker', default='/GlyphText')
    parser.add_argument('--poppler', help='absolute pdftoppm executable override')
    parser.add_argument('--tesseract', help='absolute tesseract executable override')
    parser.add_argument('--selftest', action='store_true')
    args = parser.parse_args()
    if not args.selftest and not all((args.original, args.edited, args.expected)):
        parser.error('--original --edited --expected are required unless --selftest')
    out = args.out.resolve()
    if args.selftest and not (out.name.startswith('text-render-harness-selftest-') and out.parent.name == 'dist'):
        parser.error('--selftest output must be dist/text-render-harness-selftest-*')
    if out.exists() and (not out.is_dir() or any(out.iterdir())):
        parser.error('--out must be absent or empty (will not overwrite evidence)')
    out.mkdir(parents=True, exist_ok=True)
    try:
        poppler, tesseract = tool('pdftoppm', args.poppler), tool('tesseract', args.tesseract)
        result = selftest(out, poppler, tesseract) if args.selftest else verify(
            args.original, args.edited, args.expected, out, args.ownership_marker, poppler, tesseract)
        code = 0
    except Exception as e:
        result = {'status': 'fail', 'error': str(e), 'error_type': type(e).__name__}
        code = 1
    (out/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps(result, indent=2))
    return code


if __name__ == '__main__':
    sys.exit(main())
