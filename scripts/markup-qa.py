#!/usr/bin/env python3
"""Assert native rectangle drawing, history, deletion and persistence on an owned display and a generated PDF only."""
import argparse
import importlib.util
import json
import subprocess
import time
from pathlib import Path
from PIL import Image


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def image_pixels(image):
    getter = getattr(image, "get_flattened_data", None) or image.getdata
    return getter()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--imported-reals', action='store_true', help='Generated raw-PDF fixture with valid integral-real MediaBox values')
    parser.add_argument('--shape', choices=['rectangle', 'ellipse'], default='rectangle')
    parser.add_argument('--read-only-retry', action='store_true', help='With repeat-save, prove rejected read-only Save retains edits and retry succeeds')
    parser.add_argument('--repeat-save', action='store_true', help='Exercise mixed shapes across repeated saves and saved-checkpoint Undo')
    parser.add_argument('--save-on-release', action='store_true', help='Probe immediate Save on the gesture release frame')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    fixture = load('glyph_label_fixture', Path(__file__).with_name('gui-qa.py'))
    isolation = load('glyph_label_isolation', Path(__file__).with_name('editing-qa.py'))
    pdf = out / 'markup-fixture.pdf'
    shape = args.shape
    shape_key = 'r' if shape == 'rectangle' else 'e'
    fixture.make_colored_pdf(pdf)
    if args.imported_reals:
        # Rebuild this generated fixture's xref; never mutate user PDFs or merely
        # replace longer tokens without fixing offsets.
        import re
        objects=re.findall(rb'(\d+) 0 obj\n(.*?)\nendobj\n',pdf.read_bytes(),re.S)
        data=b'%PDF-1.4\n%\xe2\xe3\xcf\xd3\n';offsets=[]
        for number,body in objects:
            body=body.replace(b'/MediaBox [0 0 612 792]',b'/MediaBox [0.0 0.0 612.0 792.0]')
            offsets.append(len(data));data+=number+b' 0 obj\n'+body+b'\nendobj\n'
        xref=len(data)
        data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()
        for offset in offsets:data+=f'{offset:010d} 00000 n \n'.encode()
        data+=f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
        pdf.write_bytes(data)
    original = pdf.read_bytes()
    checks = []
    apps = []
    display = isolation.PrivateXvfb()

    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd, env=display.env, text=True, check=True, capture_output=True).stdout

    def check(name, condition):
        name = name.replace('rectangle', shape).replace('Rectangle', shape.title())
        checks.append({'name': name, 'passed': bool(condition)})
        assert condition, name

    def shot(name):
        path = out / (name + '.png')
        run('ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1600x1000', '-i', display.env['DISPLAY'], '-frames:v', '1', '-threads', '1', str(path))
        return path

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
                run('xdotool','mousemove','1590','990')
                return app
            except (subprocess.CalledProcessError, IndexError):
                if app.poll() is not None:
                    raise AssertionError('native app exited during startup')
                time.sleep(.05)
        raise AssertionError('no owned native window')

    region=(350,220,1190,750)
    def red(name,area=region):
        image=Image.open(shot(name)).convert('RGB').crop(area)
        return sum(1 for p in image_pixels(image) if isinstance(p,tuple) and p[0]>180 and p[1]<75 and p[2]<90)
    def wait_red(name,present,area=region,threshold=60):
        deadline=time.monotonic()+10
        n=0
        while time.monotonic()<deadline:
            n=red(name,area)
            if (n>threshold if present else n<5):return n
            time.sleep(.05)
        raise AssertionError(f'{name}: expected red={present}, got {n}; inspect screenshot')
    def point(box,x,y):return (round(box[0]+(box[2]-box[0])*x),round(box[1]+(box[3]-box[1])*y))
    def drag(a,b):
        run('xdotool','mousemove',str(a[0]),str(a[1]),'mousedown','1')
        for i in range(1,8):
            run('xdotool','mousemove',str(round(a[0]+(b[0]-a[0])*i/7)),str(round(a[1]+(b[1]-a[1])*i/7)))
            time.sleep(.035)
        if args.save_on_release:
            run('xdotool','mouseup','1','key','--delay','0','ctrl+s')
        else:
            run('xdotool','mouseup','1')
        run('xdotool','mousemove','350','760')
    try:
        app=launch();key('ctrl+1')
        # A visible window is not a rendered document. Await actual fixture pixels
        # rather than racing first-page/PDFium startup with a fixed sleep.
        deadline=time.monotonic()+15
        bbox=None
        while time.monotonic()<deadline:
            display.ensure_alive()
            assert app.poll() is None, 'native app exited before first page render'
            image=Image.open(shot('00-baseline')).convert('RGB')
            crop=image.crop(region)
            pixels=[255 if isinstance(p,tuple) and min(p)>245 else 0 for p in image_pixels(crop)]
            if pixels.count(255)>10000:
                mask=Image.new('L',crop.size);mask.putdata(pixels);bbox=mask.getbbox()
                if bbox is not None:break
            time.sleep(.1)
        assert bbox,'No rendered white fixture page before deadline; inspect 00-baseline.png and app log'
        box=(bbox[0]+region[0],bbox[1]+region[1],bbox[2]+region[0],bbox[3]+region[1])
        check('baseline drawing contains no red rectangle',red('00-baseline-red')<5)
        # Focus the canvas, choose R, wait for the retained editing session.
        x,y=point(box,.9,.9);run('xdotool','mousemove',str(x),str(y),'click','1');key(shape_key);time.sleep(.5)
        drag(point(box,.2,.2),point(box,.7,.65))
        check('native Rectangle drag renders real PDF annotation',wait_red('01-rectangle',True)>60)
        if shape == 'ellipse':
            image = Image.open(shot('01-curved-appearance')).convert('RGB')
            a,b = point(box,.19,.19),point(box,.25,.25)
            corner = image.crop((a[0],a[1],b[0],b[1]))
            check('ellipse is curved, not a rectangular appearance',sum(1 for p in image_pixels(corner) if p[0]>180 and p[1]<75 and p[2]<90)<5)
        if args.save_on_release:
            deadline=time.monotonic()+5
            while time.monotonic()<deadline and pdf.read_bytes()==original:
                display.ensure_alive();time.sleep(.05)
            check('Save immediately after release commits the completed gesture',pdf.read_bytes()!=original)
            app.terminate();app.wait(timeout=5);app=launch();key('ctrl+1')
            check('immediate Save annotation survives native reopen',wait_red('01-immediate-save-reopened',True)>60)
            return
        check('unsaved rectangle leaves source bytes untouched',pdf.read_bytes()==original)
        if shape == 'ellipse':
            # Inside the annotation's bounding box, but outside the actual oval.
            # A missed selection must not authorize deleting the visible ellipse.
            key('v');time.sleep(.2)
            x,y=point(box,.21,.21)
            run('xdotool','mousemove',str(x),str(y),'click','1');key('Delete')
            check('ellipse empty bounding-box corner cannot select/delete the oval',wait_red('01-corner-delete-safe',True)>60)
            check('missed ellipse selection does not write the source',pdf.read_bytes()==original)
        key('ctrl+z');check('native Undo removes rectangle pixels',wait_red('02-undo',False)<5)
        key('ctrl+shift+z');check('native Redo restores rectangle pixels',wait_red('03-redo',True)>60)
        key('v');time.sleep(.2);x,y=point(box,.4,.4);run('xdotool','mousemove',str(x),str(y),'click','1');key('Delete')
        check('Select markup then Delete removes annotation',wait_red('04-delete',False)<5)
        key('ctrl+z');check('Undo restores deleted rectangle',wait_red('05-delete-undo',True)>60)
        # A selection on page 1 must not permit deleting its invisible annotation from page 2.
        key('v');x,y=point(box,.4,.4);run('xdotool','mousemove',str(x),str(y),'click','1')
        run('xdotool','mousemove','120','188','click','1')
        deadline=time.monotonic()+10;green=0
        while time.monotonic()<deadline:
            image=Image.open(shot('05-other-page')).convert('RGB').crop(region)
            green=sum(1 for p in image_pixels(image) if isinstance(p,tuple) and p[0]<80 and p[1]>100 and p[2]<120)
            if green>1000:break
            time.sleep(.05)
        check('navigation displays the second green drawing',green>1000)
        key('Delete');time.sleep(.3)
        run('xdotool','mousemove','50','188','click','1')
        check('Delete on another page does not remove the invisible rectangle',wait_red('05-other-page-delete-safe',True)>60)
        check('history and deletion do not write original PDF',pdf.read_bytes()==original)
        key('ctrl+s');deadline=time.monotonic()+10
        while time.monotonic()<deadline and pdf.read_bytes()==original:display.ensure_alive();time.sleep(.05)
        check('Save commits rectangle to PDF',pdf.read_bytes()!=original)
        check('Save retains original PDF backup',any(p.read_bytes()==original for p in out.glob('.glyph-backup-*.pdf')))
        check('saved PDF retains native rectangle appearance',wait_red('06-saved',True)>60)
        if args.repeat_save:
            first_saved=pdf.read_bytes();key('ctrl+s');time.sleep(.3)
            check('clean repeated Save does not rewrite file',pdf.read_bytes()==first_saved)
            area_a,area_b=point(box,.77,.72),point(box,.95,.92)
            area=(*area_a,*area_b)
            check('second markup area initially contains no red',red('06-second-baseline',area)<5)
            key('e' if shape=='rectangle' else 'r');time.sleep(.3)
            drag(point(box,.8,.75),point(box,.92,.88))
            check('mixed second shape renders on the same page',wait_red('06-second-shape',True,area,10)>10)
            check('new shape after Save still leaves saved source untouched',pdf.read_bytes()==first_saved)
            if args.read_only_retry:
                old_mode=pdf.stat().st_mode
                try:
                    pdf.chmod(0o444);key('ctrl+s')
                    deadline=time.monotonic()+10;error=''
                    while time.monotonic()<deadline:
                        screenshot=shot('06-read-only-save-error')
                        ocr=out/'06-read-only-error-footer.png'
                        Image.open(screenshot).crop((0,730,1200,800)).resize((2400,140)).save(ocr)
                        error=run('tesseract',str(ocr),'stdout','--psm','6').lower()
                        if 'read' in error and 'only' in error:break
                        time.sleep(.05)
                    check('read-only Save reports protective refusal without crashing',app.poll() is None and 'read' in error and 'only' in error)
                    check('rejected Save leaves original saved file untouched',pdf.read_bytes()==first_saved)
                    check('rejected Save retains unsaved second shape',wait_red('06-rejected-retained',True,area,10)>10)
                finally:
                    pdf.chmod(old_mode)
            key('ctrl+s');deadline=time.monotonic()+10
            while time.monotonic()<deadline and pdf.read_bytes()==first_saved:display.ensure_alive();time.sleep(.05)
            check('second Save commits mixed shape state',pdf.read_bytes()!=first_saved)
            wait_red('06-second-saved',True,area,10)
            second_saved=pdf.read_bytes()
            (out/"mixed-shapes-proof.pdf").write_bytes(second_saved)
            check('second Save retains previous annotated version as backup',any(p.read_bytes()==first_saved for p in out.glob('.glyph-backup-*.pdf')))
            key('ctrl+z')
            check('Undo crosses saved mixed-shape checkpoint',wait_red('06-second-undo',False,area)<5)
            check('Undo after Save does not itself rewrite source',pdf.read_bytes()==second_saved)
            key('ctrl+s');deadline=time.monotonic()+10
            while time.monotonic()<deadline and pdf.read_bytes()==second_saved:display.ensure_alive();time.sleep(.05)
            check('Save after Undo commits restored first shape state',pdf.read_bytes()!=second_saved)
            check('first shape remains visible after repeated saves',wait_red('06-repeated-saved',True)>60)
        app.terminate();app.wait(timeout=5);app=launch();key('ctrl+1');check('real annotation survives native reopen',wait_red('07-reopened',True)>60)
        # Reopened Glyph ownership survives parsing and enables deletion.
        key('v');time.sleep(.5);x,y=point(box,.4,.4);run('xdotool','mousemove',str(x),str(y),'click','1');key('Delete')
        check('reopened rectangle remains selectable and deletable',wait_red('08-reopened-delete',False)<5)
        # Cancel an in-progress rectangle without touching history or source.
        key(shape_key);time.sleep(.2);a=point(box,.15,.15);b=point(box,.8,.8)
        run('xdotool','mousemove',str(a[0]),str(a[1]),'mousedown','1','mousemove',str(b[0]),str(b[1]));key('Escape');run('xdotool','mouseup','1','mousemove','350','760')
        check('Escape cancels a live drawing gesture',wait_red('09-cancelled',False)<5)
    finally:
        for app, log in apps:
            if app.poll() is None:
                app.terminate()
                try:app.wait(timeout=5)
                except subprocess.TimeoutExpired:app.kill();app.wait()
            log.close()
        display.close()
        (out/'result.json').write_text(json.dumps({'checks':checks},indent=2)+'\n')
    print(json.dumps({'checks':checks},indent=2))

if __name__=='__main__':main()
