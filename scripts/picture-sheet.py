#!/usr/bin/env python3
"""Every preset's picture on one PNG, to look at without a terminal: renders
each preset with `synesthesia render --picture` (its own sound drives it)
and tiles the PPMs, scaled up, three to a row. Stdlib only (zlib for PNG).

  scripts/picture-sheet.py OUT.png [--secs 20] [--size 120x60] [--scale 3]
"""
import argparse, json, os, struct, subprocess, tempfile, zlib
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT))['target_directory']
BIN = f'{TARGET}/release/synesthesia'

ap = argparse.ArgumentParser()
ap.add_argument('out')
ap.add_argument('--secs', default='20')
ap.add_argument('--size', default='120x60')
ap.add_argument('--scale', type=int, default=3)
a = ap.parse_args()

count = len(subprocess.check_output([BIN, 'render', '--list'], text=True).splitlines())
tmp = tempfile.mkdtemp(prefix='picture-sheet-')


def render(i):
    path = f'{tmp}/p{i}.ppm'
    subprocess.run([BIN, 'render', '--preset', str(i), '--secs', a.secs, '--size', a.size, '--picture', path],
                   check=True, stdout=subprocess.DEVNULL)
    magic, size, _maxval, pixels = open(path, 'rb').read().split(b'\n', 3)
    w, h = map(int, size.split())
    return w, h, pixels


with ThreadPoolExecutor(max_workers=os.cpu_count()) as pool:
    images = list(pool.map(render, range(count)))

cols, gap, k = 3, 4, a.scale
w, h = images[0][0], images[0][1]
W = cols * (w * k + gap)
H = (count + cols - 1) // cols * (h * k + gap)
canvas = bytearray([30, 30, 30] * W * H)
for n, (iw, ih, px) in enumerate(images):
    ox, oy = (n % cols) * (w * k + gap), (n // cols) * (h * k + gap)
    for y in range(ih * k):
        for x in range(iw * k):
            s = ((y // k) * iw + x // k) * 3
            t = ((oy + y) * W + ox + x) * 3
            canvas[t:t + 3] = px[s:s + 3]

raw = b''.join(b'\0' + bytes(canvas[y * W * 3:(y + 1) * W * 3]) for y in range(H))


def chunk(tag, data):
    return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)


with open(a.out, 'wb') as f:
    f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b''))
print(f'{a.out}: {count} presets, {W}x{H}')
