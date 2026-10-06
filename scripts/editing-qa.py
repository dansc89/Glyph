#!/usr/bin/env python3
"""Real native edit workflow on private Xvfb; only generated PDFs are changed."""
import argparse, csv, importlib.util, io, json, os, re, select, subprocess, time
from pathlib import Path
from PIL import Image, ImageChops, ImageOps


class PrivateXvfb:
    """Own a server and accept only the display it allocates via -displayfd."""

    def __init__(self, timeout=10):
        self.server = None
        self.env = dict(os.environ, DISPLAY='', WAYLAND_DISPLAY='',
                        WINIT_UNIX_BACKEND='x11', XDG_SESSION_TYPE='x11',
                        DBUS_SESSION_BUS_ADDRESS='unix:path=/nonexistent/glyph-isolated-qa-bus')
        read_fd, write_fd = os.pipe()
        try:
            self.server = subprocess.Popen(
                ['Xvfb', '-displayfd', str(write_fd), '-screen', '0',
                 '1600x1000x24', '-nolisten', 'tcp'],
                pass_fds=(write_fd,), env=self.env,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            os.close(write_fd)
            write_fd = None
            deadline = time.monotonic() + timeout
            allocation = b''
            while b'\n' not in allocation:
                self.ensure_alive()
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise RuntimeError('owned Xvfb did not allocate a display before timeout')
                if select.select([read_fd], [], [], min(remaining, .05))[0]:
                    chunk = os.read(read_fd, 64)
                    if not chunk:
                        raise RuntimeError('owned Xvfb closed its display allocation pipe')
                    allocation += chunk
                    if len(allocation) > 32:
                        raise RuntimeError('invalid Xvfb display allocation')
            if not re.fullmatch(rb'[0-9]+\n', allocation):
                raise RuntimeError('invalid Xvfb display allocation')
            self.ensure_alive()
            self.env['DISPLAY'] = ':' + allocation[:-1].decode('ascii')
        except BaseException:
            self.close()
            raise
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)

    def ensure_alive(self):
        if self.server is None or self.server.poll() is not None:
            raise RuntimeError('owned Xvfb is not running; refusing display interaction')

    def close(self):
        if self.server is not None:
            if self.server.poll() is None:
                self.server.terminate()
            try:
                self.server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.server.kill()
                self.server.wait()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--out',type=Path,required=True);parser.add_argument('--rename-only',action='store_true');parser.add_argument('--document-menu-only',action='store_true')
    args=parser.parse_args();out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    spec=importlib.util.spec_from_file_location('fixture',Path(__file__).with_name('gui-qa.py'))
    assert spec is not None and spec.loader is not None
    fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)
    pdf=out/'editable-fixture.pdf';fixture.make_colored_pdf(pdf)
    objects=re.findall(rb"\d+ 0 obj\n(.*?)\nendobj",pdf.read_bytes(),re.S)
    root=len(objects)+1;first=root+1;last=first+2
    objects[0]=objects[0].replace(b"<<",f"<< /Outlines {root} 0 R ".encode(),1)
    objects.append(f"<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count 3 >>".encode())
    for i in range(3):
        previous=f" /Prev {first+i-1} 0 R" if i else "";following=f" /Next {first+i+1} 0 R" if i<2 else ""
        objects.append(f"<< /Title (Sheet {i+1}) /Parent {root} 0 R /Dest [{i+3} 0 R /Fit]{previous}{following} >>".encode())
    data=b"%PDF-1.4\n";offsets=[]
    for index,obj in enumerate(objects,1):offsets.append(len(data));data+=f"{index} 0 obj\n".encode()+obj+b"\nendobj\n"
    xref=len(data);data+=f"xref\n0 {len(objects)+1}\n0000000000 65535 f \n".encode()
    for offset in offsets:data+=f"{offset:010d} 00000 n \n".encode()
    data+=f"trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode();pdf.write_bytes(data);original=data
    checks=[];apps=[];display=PrivateXvfb();env=display.env
    def run(*cmd):
        display.ensure_alive()
        return subprocess.run(cmd,env=env,text=True,check=True,capture_output=True).stdout
    def check(name,condition):
        checks.append({'name':name,'passed':bool(condition)});assert condition,name
    def screenshot(name):
        path=out/(name+'.png');run('ffmpeg','-y','-loglevel','error','-f','x11grab','-video_size','1600x1000','-i',env['DISPLAY'],'-frames:v','1','-threads','1',str(path));return path
    def words(path,region=None,invert=False):
        processed=path.with_name(path.stem+'-ocr.png');image=Image.open(path).convert('L')
        left,top=0,0
        if region is not None:
            left,top=region[:2];image=image.crop(region)
        image=ImageOps.invert(image) if invert else ImageOps.autocontrast(image)
        image.resize((image.width*4,image.height*4),Image.Resampling.LANCZOS).save(processed)
        rows=csv.DictReader(io.StringIO(run('tesseract',str(processed),'stdout','--psm','11','tsv')),delimiter='\t')
        result=[r for r in rows if r.get('text','').strip()]
        for r in result:
            for k in ('left','top','width','height'):r[k]=str(int(r[k])//4)
            r['left']=str(int(r['left'])+left);r['top']=str(int(r['top'])+top)
        return result
    def token(word,name,sidebar=False,last=False):
        deadline=time.monotonic()+8;found=[];path=out/(name+'.png')
        while time.monotonic()<deadline:
            path=screenshot(name);items=words(path,region=(0,180,350,480),invert=True) if word=='rename' else words(path)
            found=[r for r in items if re.sub(r'[^a-z0-9]','',r['text'].lower()).startswith(word.lower()) and (not sidebar or (int(r['left'])<350 and (word not in ('sheet','floorplan') or int(r['top'])>180)))]
            if found:
                r=found[-1] if last else found[0];return (int(r['left'])+int(r['width'])//2,int(r['top'])+int(r['height'])//2)
            time.sleep(.1)
        raise AssertionError(f'{word} not visible; inspect {path}')
    def click_word(word,name,sidebar=False,last=False,button='1'):
        x,y=token(word,name,sidebar,last);run('xdotool','mousemove','--sync',str(x),str(y),'click',button)
    def key(keys):run('xdotool','key','--clearmodifiers',keys)
    def launch():
        display.ensure_alive()
        log=open(out/f'app-{len(apps)}.log','w');app=subprocess.Popen([str(args.binary.resolve()),str(pdf)],env=env,stdout=log,stderr=log);apps.append((app,log))
        deadline=time.monotonic()+10
        while time.monotonic()<deadline:
            try:
                window=run('xdotool','search','--onlyvisible','--name','^Glyph$').strip().splitlines()[-1];run('xdotool','windowfocus',window);return
            except (subprocess.CalledProcessError,IndexError):
                if app.poll() is not None:raise AssertionError('app exited during startup')
                time.sleep(.05)
        raise AssertionError('native window did not appear')
    try:
        launch()
        click_word('document','00-document-menu');token('save','00-document-actions');key('Escape')
        check('Document menu exposes save actions',True)
        if args.document_menu_only:
            print(json.dumps({'checks':checks},indent=2));return
        run('xdotool','mousemove','211','236','click','1')
        token('sheet','02-original-bookmark',sidebar=True)
        click_word('sheet','03-bookmark-context',sidebar=True,button='3')
        click_word('rename','04-rename-menu')
        key('ctrl+a');run('xdotool','type','--clearmodifiers','--delay','20','A101 Floorplan');key('Return')
        token('floorplan','05-renamed-bookmark',sidebar=True)
        check('rename does not modify source before Save',pdf.read_bytes()==original)
        if args.rename_only:
            print(json.dumps({'checks':checks},indent=2));return
        key('ctrl+z');token('sheet','06-undo',sidebar=True);check('undo does not modify source',pdf.read_bytes()==original)
        key('ctrl+shift+z');token('floorplan','07-redo',sidebar=True)
        run('xdotool','mousemove','350','760');time.sleep(.2);before=Image.open(screenshot('08-before-save')).convert('RGB').crop((440,210,1160,600))
        key('ctrl+s');deadline=time.monotonic()+8
        while time.monotonic()<deadline and pdf.read_bytes()==original:time.sleep(.05)
        check('Save commits the generated PDF',pdf.read_bytes()!=original);token('floorplan','09-saved-bookmark',sidebar=True)
        run('xdotool','mousemove','350','760');time.sleep(.3);after=Image.open(screenshot('10-after-save')).convert('RGB').crop((440,210,1160,600))
        check('bookmark Save preserves rendered page pixels',ImageChops.difference(before,after).getbbox() is None)
        saved=pdf.read_bytes();key('ctrl+z');token('sheet','11-unsaved-undo',sidebar=True)
        key('ctrl+w');click_word('cancel','12-unsaved-cancel');token('sheet','13-kept-document',sidebar=True)
        check('Cancel close keeps saved PDF unchanged',pdf.read_bytes()==saved)
        key('ctrl+w');click_word('discard','14-discard-close')
        closed=screenshot('15-document-closed');empty=out/'empty-sidebar-ocr.png'
        ImageOps.invert(Image.open(closed).crop((10,102,318,178)).convert('L')).resize((1232,304)).save(empty)
        empty_text=run('tesseract',str(empty),'stdout','--psm','6').lower()
        check('Discard closes to the empty viewer', 'no pdf loaded' in empty_text)
        check('Discard leaves saved PDF unchanged',pdf.read_bytes()==saved)
        apps[-1][0].terminate();apps[-1][0].wait(timeout=5);launch();run('xdotool','mousemove','211','236','click','1');token('floorplan','17-persisted-title',sidebar=True)
        check('renamed bookmark survives native reopen',True)
    finally:
        for app,log in apps:
            if app.poll() is None:
                app.terminate()
                try:app.wait(timeout=5)
                except subprocess.TimeoutExpired:app.kill();app.wait()
            log.close()
        display.close()
        (out/'result.json').write_text(json.dumps({'checks':checks},indent=2)+'\n')
    print(json.dumps({'checks':checks},indent=2))

if __name__=='__main__':main()
