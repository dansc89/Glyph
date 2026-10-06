#!/usr/bin/env python3
"""Exercise the native viewer on a private X display, never on the user desktop."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
import re

from PIL import Image, ImageChops


def pixels(image):
    """Pillow 10/11 compatibility without deprecated calls on Pillow 12."""
    method = getattr(image, "get_flattened_data", None) or image.getdata
    return method()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location("fixture", Path(__file__).with_name("gui-qa.py"))
    assert spec is not None and spec.loader is not None
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    pdf = args.out / "performance-qa.pdf"
    fixture.make_colored_pdf(pdf)
    # Add an internal hotspot covering the purple header, targeting page three.
    objects=re.findall(rb"\d+ 0 obj\n(.*?)\nendobj", pdf.read_bytes(),re.S)
    objects[2]=objects[2].replace(b"/Contents",b"/Annots [14 0 R] /Contents")
    objects.append(b"<< /Type /Annot /Subtype /Link /Rect [72 600 540 695] /Border [0 0 0] /Dest [5 0 R /Fit] >>")
    data=b"%PDF-1.4\n";offsets=[]
    for index,obj in enumerate(objects,1):
        offsets.append(len(data));data+=f"{index} 0 obj\n".encode()+obj+b"\nendobj\n"
    xref=len(data)
    data+=f"xref\n0 {len(objects)+1}\n0000000000 65535 f \n".encode()
    for offset in offsets:data+=f"{offset:010d} 00000 n \n".encode()
    data+=f"trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    pdf.write_bytes(data)
    env = os.environ.copy()
    isolation_spec = importlib.util.spec_from_file_location("isolation", Path(__file__).with_name("editing-qa.py"))
    assert isolation_spec is not None and isolation_spec.loader is not None
    isolation = importlib.util.module_from_spec(isolation_spec)
    isolation_spec.loader.exec_module(isolation)
    owned = None
    env.update(WAYLAND_DISPLAY="", WINIT_UNIX_BACKEND="x11", XDG_SESSION_TYPE="x11")
    app = None
    results = []
    app_log = (args.out / "glyph.log").open("w")


    def run(*command):
        assert owned is not None
        owned.ensure_alive()
        return subprocess.run(command, env=env, check=True, capture_output=True, text=True).stdout

    def screenshot(name):
        path = args.out / f"{name}.png"
        run("ffmpeg", "-v", "error", "-y", "-f", "x11grab", "-video_size", "1600x1000", "-i", env["DISPLAY"], "-frames:v", "1", str(path))
        return path

    def wait_page(name, color):
        start = time.monotonic()
        deadline = start + 15
        assert app is not None
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError("viewer exited unexpectedly")
            path = screenshot(name)
            # Only the PDF viewport, not the highlighted sidebar, contributes.
            image = Image.open(path).convert("RGB").crop((340, 120, 1425, 870))
            matches = sum(all(abs(v - target) <= 35 for v, target in zip(pixel, color)) for pixel in pixels(image))
            if matches > 1000:
                results.append({"action": name, "passed": True, "matching_pixels": matches, "observed_seconds_including_capture": time.monotonic() - start})
                return
            time.sleep(.1)
        raise AssertionError(f"{name}: expected PDF page color {color} not displayed")

    def stable_view(name):
        deadline = time.monotonic() + 15
        previous = None
        stable_frames = 0
        while time.monotonic() < deadline:
            path = screenshot(name)
            current = Image.open(path).convert("RGB").crop((340, 240, 1425, 870))
            if previous is not None:
                changed = sum(any(v > 20 for v in px) for px in pixels(ImageChops.difference(previous, current)))
                stable_frames = stable_frames + 1 if changed < 100 else 0
                if stable_frames >= 2:
                    return current
            previous = current
        raise AssertionError(f"{name}: viewport never settled")

    try:
        owned = isolation.PrivateXvfb()
        env = owned.env
        assert owned is not None
        owned.ensure_alive()
        app = subprocess.Popen([str(args.binary.resolve()), str(pdf.resolve())], env=env, stdout=app_log, stderr=subprocess.STDOUT)
        wait_page("01-initial-fit-page", (140, 51, 242))
        window=run("xdotool","search","--name","^Glyph$").splitlines()[0]
        run("xdotool","windowfocus","--sync",window)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            thumbnail_image = Image.open(screenshot("00-thumbnail-sidebar")).convert("RGB")
            coords = [(x,y) for y in range(260,850,2) for x in range(20,310,2) if all(abs(v-t)<30 for v,t in zip(thumbnail_image.getpixel((x,y)),(26,89,230)))]
            if len(coords) > 50: break
        assert len(coords) > 50, "third page thumbnail was not rendered"
        run("xdotool", "mousemove", str(sum(p[0] for p in coords)//len(coords)), str(sum(p[1] for p in coords)//len(coords)), "click", "1")
        wait_page("00-thumbnail-click-page-three", (26,89,230))
        run("xdotool", "key", "Home")
        wait_page("00-thumbnail-back-first-page", (140,51,242))
        results.append({"action":"thumbnail-render-and-navigation", "passed":True})
        window=run("xdotool","search","--name","^Glyph$").splitlines()[0]
        run("xdotool","windowfocus","--sync",window)
        def header_bounds(path):
            image = Image.open(path).convert("RGB")
            points = [(x, y) for y in range(180, 870, 3) for x in range(340, 1425, 3)
                      if all(abs(v - t) < 25 for v, t in zip(image.getpixel((x, y)), (140, 51, 242)))]
            assert points, "PDF header is not visible"
            return min(x for x, _ in points), max(x for x, _ in points), min(y for _, y in points)
        fitted = header_bounds(args.out / "01-initial-fit-page.png")
        run("xdotool", "key", "ctrl+2")
        wait_page("01b-fit-width", (140, 51, 242))
        wide = header_bounds(args.out / "01b-fit-width.png")
        assert wide[1] - wide[0] > (fitted[1] - fitted[0]) * 1.2, "Ctrl+2 did not fit page width"
        assert wide[2] < 350, "fit-width did not keep top of drawing visible"
        results.append({"action": "fit-width-shortcut-top-aligned", "passed": True})
        run("xdotool", "windowsize", "--sync", window, "1100", "800")
        wait_page("01d-fit-width-small-window", (140, 51, 242))
        narrow = header_bounds(args.out / "01d-fit-width-small-window.png")
        assert narrow[1] - narrow[0] < (wide[1] - wide[0]) * .85, "fit width did not follow window resize"
        results.append({"action": "persistent-fit-width-resize", "passed": True})
        run("xdotool", "windowsize", "--sync", window, "1440", "920")
        wait_page("01e-fit-width-restored-window", (140, 51, 242))
        run("xdotool", "key", "ctrl+1")
        wait_page("01c-refit-page", (140, 51, 242))
        refitted = header_bounds(args.out / "01c-refit-page.png")
        assert abs((refitted[1] - refitted[0]) - (fitted[1] - fitted[0])) <= 6, "Ctrl+1 did not restore fit-page"
        results.append({"action": "fit-page-shortcut", "passed": True})
        # Direct page entry replaces the current number without needing Ctrl+A.
        run("xdotool", "key", "ctrl+g")
        run("xdotool", "type", "3")
        run("xdotool", "key", "Return")
        wait_page("01f-direct-page-three", (26, 89, 230))
        run("xdotool", "key", "alt+Left")
        wait_page("01g-direct-page-history", (140, 51, 242))
        results.append({"action": "direct-page-entry-history", "passed": True})
        run("xdotool", "mousemove", "900", "500", "click", "--repeat", "2", "--delay", "100", "4")
        run("xdotool", "mousedown", "2", "mousemove", "945", "530", "mouseup", "2")
        run("xdotool", "mousemove", "900", "500")
        wait_page("01h-source-zoomed-panned", (140, 51, 242))
        source_view = stable_view("01h-source-zoomed-panned")
        run("xdotool", "key", "Right")
        wait_page("01i-history-destination", (26, 166, 77))
        run("xdotool", "mousemove", "900", "500", "click", "--repeat", "3", "--delay", "100", "4")
        stable_view("01i-destination-zoomed")
        run("xdotool", "key", "alt+Left")
        wait_page("01j-history-restored-view", (140, 51, 242))
        restored_view = stable_view("01j-history-restored-view")
        changed = sum(any(v > 20 for v in px) for px in pixels(ImageChops.difference(source_view, restored_view)))
        assert changed < 100, f"history did not restore viewport: {changed} changed pixels"
        results.append({"action": "history-restores-zoom-and-pan", "passed": True, "changed_pixels": changed})
        import csv, io
        words = list(csv.DictReader(io.StringIO(run("tesseract", str(args.out / "01j-history-restored-view.png"), "stdout", "--psm", "11", "tsv")), delimiter="\t"))
        word = next(w for w in words if w["text"].upper() == "CENTERLINE" and int(w["left"]) > 340)
        left,top,width,height = (int(word[k]) for k in ("left","top","width","height"))
        run("xdotool","mousemove",str(left+2),str(top+height//2),"mousedown","1","mousemove","--sync",str(left+width-2),str(top+height//2),"mouseup","1")
        screenshot("01k-selected-pdf-text")
        run("xdotool","key","ctrl+c")
        copied = subprocess.run(["xclip","-o","-selection","clipboard"],env=env,check=True,capture_output=True,text=True,timeout=3).stdout
        assert copied == "CENTERLINE", f"PDF selection copied wrong text: {copied!r}"
        results.append({"action":"select-and-copy-pdf-text","passed":True,"copied_text":copied})
        run("xdotool", "key", "ctrl+1", "Right")
        wait_page("02-next-page", (26, 166, 77))
        run("xdotool", "key", "Right")
        wait_page("03-third-page", (26, 89, 230))
        run("xdotool", "key", "alt+Left")
        wait_page("04-back-cached", (26, 166, 77))
        run("xdotool", "key", "alt+Right")
        wait_page("05-forward-cached", (26, 89, 230))
        # Repeated navigation should never leave an obsolete rendering on screen.
        for _ in range(10):
            run("xdotool", "key", "Home", "End")
        wait_page("06-rapid-navigation", (26, 89, 230))
        run("xdotool", "key", "Home")
        wait_page("07-link-source", (140,51,242))
        image=Image.open(args.out/"07-link-source.png").convert("RGB")
        coords=[(x,y) for y in range(120,870,3) for x in range(340,1425,3) if all(abs(v-t)<35 for v,t in zip(image.getpixel((x,y)),(140,51,242)))]
        x=sum(p[0] for p in coords)//len(coords);y=sum(p[1] for p in coords)//len(coords)
        time.sleep(.3)
        run("xdotool","mousemove",str(x),str(y),"click","1")
        wait_page("08-click-internal-link",(26,89,230))
        run("xdotool", "key", "ctrl+f")
        run("xdotool", "type", "CENTERLINE")
        run("xdotool", "key", "Return")
        time.sleep(.8)
        search_image=screenshot("09-search-results")
        text=run("tesseract",str(search_image),"stdout","--psm","11")
        assert re.search(r"3\s*matches",text), f"search results not visible: {text}"
        results.append({"action":"search-three-pages","passed":True})
        # The keyboard must remain in the search box instead of navigating pages.
        run("xdotool", "key", "ctrl+f")
        time.sleep(.1)
        run("xdotool", "key", "Left")
        wait_page("10-search-keyboard-focus",(26,89,230))
        # High zoom must refine only the viewport and remain correct after panning.
        run("xdotool","mousemove","880","490","click","--repeat","24","--delay","20","4")
        deadline=time.monotonic()+15
        while True:
            zoom_image=screenshot("11-high-zoom-tile")
            # OCR only the status row; low-contrast desktop palettes are valid.
            status = Image.open(zoom_image).convert("RGB").crop((340, 890, 1425, 918))
            from PIL import ImageOps
            status = ImageOps.autocontrast(status.convert("L")).resize((3255, 84))
            status_path = args.out / "11-status-ocr.png"
            status.save(status_path)
            text=run("tesseract",str(status_path),"stdout","--psm","7")
            if "viewporttile" in re.sub(r"[^a-z]","",text.lower()): break
            if time.monotonic()>deadline: raise AssertionError(f"high-res tile not displayed: {text}")
            time.sleep(.1)
        results.append({"action":"high-zoom-viewport-tile","passed":True})
        run("xdotool","mousemove","880","490","mousedown","2","mousemove","930","510","mouseup","2")
        time.sleep(.5)
        pan_image=screenshot("12-pan-high-zoom")
        before=Image.open(zoom_image).crop((342,119,1426,869));after=Image.open(pan_image).crop((342,119,1426,869))
        assert ImageChops.difference(before,after).getbbox() is not None, "middle-button pan did not change viewport"
        results.append({"action":"middle-button-pan","passed":True})
        (args.out / "results.json").write_text(json.dumps({"passed": True, "checks": results, "note": "Search uses OCR assertions; pan uses viewport image comparison; visual geometry review is additional. Timings include PNG capture and are not latency benchmarks."}, indent=2))
        print(json.dumps({"passed": True, "checks": results, "artifacts": str(args.out.resolve())}, indent=2))
    finally:
        for process in (app,):
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        app_log.close()
        if owned is not None:
            owned.close()


if __name__ == "__main__":
    main()
