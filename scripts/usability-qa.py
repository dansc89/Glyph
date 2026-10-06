#!/usr/bin/env python3
"""Exploratory native screenshots on an owned X server; generated fixtures only."""
import argparse, importlib.util, json, shutil, subprocess, time, re
from pathlib import Path
from PIL import Image, ImageOps

def load(name, path):
    spec=importlib.util.spec_from_file_location(name,path)
    assert spec is not None and spec.loader is not None
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--binary',type=Path,required=True);parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args();out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    isolation=load('glyph_editing_qa',Path(__file__).with_name('editing-qa.py'))
    fixture=load('glyph_fixture',Path(__file__).with_name('gui-qa.py'))
    normal=out/'normal.pdf';fixture.make_colored_pdf(normal)
    long=out/('architectural-drawing-set-'+'long-project-name-'*8+'revision-C.pdf');shutil.copyfile(normal,long)
    # Generated fixture with a deliberately long embedded sheet label.
    objects=re.findall(rb"\d+ 0 obj\n(.*?)\nendobj", normal.read_bytes(), re.S)
    label=('A101 architectural sheet name '*8).encode('ascii')
    objects[0]=objects[0].replace(b'<<',b'<< /PageLabels << /Nums [0 << /P ('+label+b') >>] >> ',1)
    data=b'%PDF-1.4\n';offsets=[]
    for index,obj in enumerate(objects,1):
        offsets.append(len(data));data+=f'{index} 0 obj\n'.encode()+obj+b'\nendobj\n'
    xref=len(data);data+=f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode()
    for offset in offsets:data+=f'{offset:010d} 00000 n \n'.encode()
    data+=f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    long.write_bytes(data)
    display=isolation.PrivateXvfb();apps=[];records=[]
    def run(*cmd):
        display.ensure_alive();return subprocess.run(cmd,env=display.env,text=True,check=True,capture_output=True).stdout
    def shot(name):
        path=out/(name+'.png');run('ffmpeg','-y','-loglevel','error','-f','x11grab','-video_size','1600x1000','-i',display.env['DISPLAY'],'-frames:v','1','-threads','1',str(path))
        crop=Image.open(path).crop((0,0,960,640));processed=out/(name+'-ocr.png');ImageOps.autocontrast(crop.convert('L')).resize((1920,1280)).save(processed)
        text=run('tesseract',str(processed),'stdout','--psm','11');records.append({'scenario':name,'screenshot':str(path),'ocr':text});return path
    def start(path=None):
        display.ensure_alive();log=open(out/(f'app-{len(apps)}.log'),'w');app=subprocess.Popen([str(args.binary.resolve())]+([str(path)] if path else []),env=display.env,stdout=log,stderr=log);apps.append((app,log))
        deadline=time.monotonic()+10
        while time.monotonic()<deadline:
            display.ensure_alive()
            try:
                wid=run('xdotool','search','--onlyvisible','--pid',str(app.pid)).strip().splitlines()[-1];run('xdotool','windowfocus',wid);run('xdotool','windowsize','--sync',wid,'960','640');run('xdotool','windowmove',wid,'0','0');time.sleep(1);return app
            except (subprocess.CalledProcessError,IndexError):
                if app.poll() is not None:raise RuntimeError('app exited during launch')
                time.sleep(.05)
        raise RuntimeError('no native window')
    try:
        app=start(normal);shot('01-minimum-window-toolbar');run('xdotool','mousemove','110','22','click','1');time.sleep(.2);shot('02-document-menu');run('xdotool','key','Escape','ctrl+f');time.sleep(.2);shot('03-search-focus');app.terminate();app.wait(timeout=5)
        app=start(long);shot('04-long-filename');run('xdotool','mousemove','327','360','mousedown','1');time.sleep(.15);run('xdotool','mousemove','--sync','420','360');time.sleep(.15);run('xdotool','mouseup','1');time.sleep(.3);shot('07-wide-sidebar');app.terminate();app.wait(timeout=5)
        app=start();shot('05-empty-window');run('xdotool','mousemove','110','22','click','1');time.sleep(.2);shot('06-empty-document-menu');app.terminate();app.wait(timeout=5)
    finally:
        for app,log in apps:
            if app.poll() is None:
                app.terminate()
                try:app.wait(timeout=5)
                except subprocess.TimeoutExpired:app.kill();app.wait()
            log.close()
        display.close();(out/'observations.json').write_text(json.dumps(records,indent=2)+'\n')
    print(json.dumps(records,indent=2))
if __name__=='__main__':main()
