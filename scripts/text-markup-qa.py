#!/usr/bin/env python3
"""Strict generated/private-display native text editing and persistence QA.

Font field coordinates must be verified on the actual dialog. No record-only mode.
Expected CLI flow is adapted to the actual producer controls before acceptance.
"""
import argparse
import hashlib
import io
import json
import math
import re
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
    parser.add_argument('--font-x', type=int, required=True, help='Verified font-size input X')
    parser.add_argument('--font-y', type=int, required=True, help='Verified font-size input Y')
    parser.add_argument('--placement', choices=('click','drag'), default='click')
    parser.add_argument('--commit-button', default='Apply')
    parser.add_argument('--editor-title', default='Text')
    parser.add_argument('--edit-x', type=int, help='Verified edit-text button X; absent means double-click selected text')
    parser.add_argument('--edit-y', type=int, default=51)
    parser.add_argument('--ownership-marker', default='/GlyphText')
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
    # Imported FreeText shares the imported Line's AP, an intentional preservation adversary.
    from pypdf import PdfWriter
    from pypdf.generic import DictionaryObject, NameObject, TextStringObject, ArrayObject, FloatObject, NumberObject
    writer = PdfWriter(clone_from=PdfReader(str(pdf)))
    imported_line = writer.pages[0]['/Annots'][0].get_object()
    writer.add_annotation(0, DictionaryObject({
        NameObject('/Type'):NameObject('/Annot'), NameObject('/Subtype'):NameObject('/FreeText'),
        NameObject('/NM'):TextStringObject('foreign-text'),NameObject('/Contents'):TextStringObject('IMPORTED NOTE KEEP'),
        NameObject('/Rect'):ArrayObject([FloatObject(v) for v in [60,100,130,120]]),
        NameObject('/F'):NumberObject(4),NameObject('/AP'):imported_line['/AP'],
        NameObject('/DA'):TextStringObject('/Courier 12 Tf 0 0 1 rg')}))
    with pdf.open('wb') as f: writer.write(f)
    original = pdf.read_bytes()
    (out/'original.pdf').write_bytes(original)
    baseline_reader = PdfReader(io.BytesIO(original), strict=True)
    baseline_streams = [page.get_contents().get_data() for page in baseline_reader.pages]
    baseline_foreign = [lines.canonical(ref.get_object()) for ref in baseline_reader.pages[0]['/Annots']]
    original_text = 'VISIBLE TEXT CHECK\nL R A (door) \\ roof'
    edited_text = 'VISIBLE TEXT CHECK\nLRA EDIT (A) \\ roof'
    checks, apps = [], []
    display = isolation.PrivateXvfb()
    # No file picker may escape the owned display through the user's session portal.
    display.env['DBUS_SESSION_BUS_ADDRESS']='unix:path='+str(out/'nonexistent-private-bus')
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
        annots = [ref.get_object() for ref in reader.pages[0].get('/Annots', [])]
        foreign = [lines.canonical(a) for a in annots if args.ownership_marker not in a]
        require(foreign == baseline_foreign, 'foreign Line/Link semantics/AP changed')
        owned = [a for a in annots if args.ownership_marker in a]
        require(len(owned) == len(records), 'unexpected FreeText count')
        for annotation, row in zip(owned, records):
            require(str(annotation.get('/Contents', '')) == row['text'], 'saved text differs from native typed draft')
            da = str(annotation.get('/DA', ''))
            values = re.findall(r'([-+0-9.]+)\s+Tf\b', da)
            unit = float(reader.pages[0].get('/UserUnit', 1))
            require(values and abs(float(values[-1])*unit-row['font_size']) < .05, 'saved physical font size does not match typed value')
            appearance = annotation['/AP']['/N'].get_object()
            require(appearance.get('/Subtype') == '/Form' and appearance.get_data().strip(), 'real vector appearance missing')
            rect = [float(v) for v in annotation['/Rect']]
            require(len(rect)==4 and all(math.isfinite(v) for v in rect) and rect[0]<rect[2] and rect[1]<rect[3], 'invalid saved text bounds')
            if 'pdf_rect' in row:
                require(all(abs(a-b)<3 for a,b in zip(rect,row['pdf_rect'])), 'text move geometry is not independently predicted')
        return True

    def record(text, size, rect=None):
        row = {'text':text, 'font_size':size, 'page':0, 'ocr_sentinel':'VISIBLE TEXT CHECK'}
        if rect is not None: row['pdf_rect']=rect
        return row

    def boxes(name):
        shot(name).close()
        data = run('tesseract', str(out/(name+'.png')), 'stdout', '--psm','11','tsv')
        result=[]
        for line in data.splitlines()[1:]:
            columns=line.split('\t')
            if len(columns)==12 and columns[11].strip():
                result.append((columns[11].strip(), *(int(v) for v in columns[6:10])))
        return result

    def editor_open(name):
        time.sleep(.3)
        words=boxes(name)
        require(any(args.editor_title.lower() in w[0].lower() for w in words), 'actual text editor title not visible')
        require(any('font' in w[0].lower() for w in words), 'actual font-size control not visible')

    def place(name):
        key('t')
        if args.placement=='drag': drag((.18,.20),(.75,.45),name+'-placement')
        else: click(point(.18,.20))
        editor_open(name+'-editor')

    def type_draft(text,size,name,commit=True):
        # Producer must hand native keyboard focus to text on opening.
        key('ctrl+a')
        for index, line in enumerate(text.split('\n')):
            if index: key('Return')
            run('xdotool','type','--clearmodifiers','--delay','20',line)
        run('xdotool','mousemove',str(args.font_x),str(args.font_y),'click','--repeat','2','--delay','100','1')
        time.sleep(.1); key('ctrl+a')
        run('xdotool','type','--clearmodifiers','--delay','40',str(size))
        key('Return');time.sleep(.15)
        if commit:
            buttons=[row for row in boxes(name+'-before-apply') if row[0].lower()==args.commit_button.lower()]
            require(len(buttons)==1, 'missing/ambiguous text commit control')
            _,x,y,w,h=buttons[0];click((x+w//2,y+h//2));stable(name+'-applied')
        else:
            stable(name+'-uncommitted-draft')

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
        check(name+': independently reopened expected text/font/geometry and preserved original streams/foreign annotations', validate(records))
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
        place('close-guard-draft')
        type_draft('KEEP DRAFT SAFE',18,'close-guard-draft',commit=False)
        click((95,14));time.sleep(.2)
        candidates=[r for r in boxes('document-close-action') if r[0].lower()=='close']
        require(len(candidates)==1,'actual Close document action missing/ambiguous')
        _,x,y,w,h=candidates[0];click((x+w//2,y+h//2));time.sleep(.4)
        require(app.poll() is None,'Close document/window lost the active text draft')
        editor_open('close-guard-retained')
        check('actual menu Close retains pending draft and current source',pdf.read_bytes()==original)
        key('Escape');stable('close-guard-cancelled');key('ctrl+s');time.sleep(.4)
        check('cancelling close-guarded draft restores clean checkpoint',pdf.read_bytes()==original)
        place('save-as-cancel-draft')
        type_draft('SAVE AS COPY ONLY',18,'save-as-cancel-draft',commit=False)
        key('ctrl+shift+s');time.sleep(.8);stable('save-as-picker-open')
        # rfd can fall back to a real GTK chooser on the owned Xvfb even without a portal.
        # Cancel that actual picker before testing subsequent ordinary Save.
        cancels=sorted((r for r in boxes('save-as-picker-controls') if r[0].lower()=='cancel'),key=lambda r:r[2])
        require(len(cancels)>=1 and cancels[0][2]<400,'actual owned Save As Cancel control absent')
        _,x,y,w,h=cancels[0];click((x+w//2,y+h//2));time.sleep(.5);stable('save-as-picker-cancelled')
        check('Ctrl+Shift+S and actual picker cancellation never overwrite the original source',pdf.read_bytes()==original)
        # Commit-then-picker contract: retained text is dirty but never discarded.
        key('ctrl+s');time.sleep(.7)
        save_as_row=[record('SAVE AS COPY ONLY',18)]
        check('cancelled Save As retains text for subsequent ordinary Save',validate(save_as_row))
        key('ctrl+z');stable('undo-cancelled-save-as-draft');save([],'restore-save-as-checkpoint')
        # The retained checkpoint can be reserialized by Undo, so refresh the exact byte baseline.
        original=pdf.read_bytes()
        place('cancelled-text');key('Escape');key('ctrl+s');time.sleep(.4)
        check('cancelled text draft never mutates source',pdf.read_bytes()==original)
        place('new-text');type_draft(original_text,18,'new-text')
        check('new text remains staged until Save',pdf.read_bytes()==original)
        initial=[record(original_text,18)]
        save(initial,'created-text-save')
        shutil.copy2(pdf,out/'created-text.pdf')
        key('t');click(point(.3,.25));editor_open('noop-reedit')
        type_draft(original_text,18,'noop-reedit',commit=False)
        before_noop=pdf.read_bytes();before_backups={p.name for p in out.glob('.glyph-backup-*.pdf')}
        key('ctrl+s');stable('noop-finish-and-save');time.sleep(.6)
        check('identical text/font commit produces no source write or false backup',pdf.read_bytes()==before_noop and {p.name for p in out.glob('.glyph-backup-*.pdf')}==before_backups)
        validate(initial)
        key('t');click(point(.3,.25));editor_open('invalid-reedit')
        type_draft(original_text,144,'invalid-reedit',commit=False)
        before_invalid=pdf.read_bytes();key('ctrl+s');stable('invalid-save-rejected')
        editor_open('invalid-draft-retained')
        check('non-fitting font/text remains editable and leaves checkpoint intact',pdf.read_bytes()==before_invalid)
        key('Escape');stable('invalid-reedit-cancelled');key('ctrl+s');time.sleep(.5)
        check('cancelled invalid re-edit preserves existing text/source',pdf.read_bytes()==before_invalid)
        validate(initial)
        key('v');click(point(.3,.25));stable('selected-text')
        if args.edit_x is not None:click((args.edit_x,args.edit_y))
        else:
            key('t');click(point(.3,.25))
        editor_open('editing-text')
        previous=pdf.read_bytes();type_draft(edited_text,24,'edited-text',commit=False)
        check('editing text/font remains staged until Save',pdf.read_bytes()==previous)
        edited=[record(edited_text,24)];save(edited,'edited-text-save')
        key('ctrl+z');stable('undo-text-and-font');save(initial,'undo-text-save')
        key('ctrl+shift+z');stable('redo-text-and-font');save(edited,'redo-text-save')
        check('text and font edit share one undoable transaction')
        annotation=next(a.get_object() for a in PdfReader(str(pdf)).pages[0]['/Annots'] if args.ownership_marker in a.get_object())
        rect=[float(v) for v in annotation['/Rect']]
        key('v');click(point(.3,.25));stable('reselected-text')
        drag((.3,.25),(.4,.35),'moved-text')
        moved_rect=[rect[0]+61.2,rect[1]-79.2,rect[2]+61.2,rect[3]-79.2]
        moved=[record(edited_text,24,moved_rect)]
        save(moved,'moved-text-save');shutil.copy2(pdf,out/'moved-text.pdf')
        drag((.78,.55),(.86,.62),'resized-text')
        resized_rect=[moved_rect[0],moved_rect[1]-.07*792,moved_rect[2]+.08*612,moved_rect[3]]
        moved=[record(edited_text,24,resized_rect)]
        save(moved,'resized-text-save');shutil.copy2(pdf,out/'resized-text.pdf')
        check('corner resize retains exact text and physical font size')
        key('Delete');stable('deleted-text');save([],'deleted-text-save')
        key('ctrl+z');stable('undo-delete-text');save(moved,'restored-text-save')
        check('text move/delete participate in shared history')
        app.terminate();app.wait(timeout=5);app=None
        launch();check('actual native reopen retains text/font/geometry',validate(moved))
        (out/'expected.json').write_text(json.dumps(moved,indent=2)+'\n')
        shot('final-generated-text').close()
        success=True
    finally:
        for process,log in apps:
            if process.poll() is None:
                process.terminate()
                try:process.wait(timeout=5)
                except subprocess.TimeoutExpired:process.kill();process.wait()
            log.close()
        display.close()
        report={'passed':success,'native_success':success,'record_only':False,'font_widget_exercised':success,
                'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
                'fixture':str(pdf),'checks':checks}
        (out/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()
