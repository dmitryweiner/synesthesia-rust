#!/usr/bin/env python3
"""Xrun soak with the sound on: runs the interface in a pty (so keys can be
pressed on a schedule), next to a terminal window running the picture probe
at 8 fps (so the terminal and X are loaded as in real use), and reads the
xrun count of the `pw-cat` node from PipeWire's own ERR counter. The player
keeps no xrun count of its own. GRAPHICS.md, "Soak".

  scripts/soak.py [--secs 130] [--viz panel|spectrum|full] [--keys ldldr...]
                  [--scout true|false] [--scout-threads N] [--no-probe]
                  [--null] [--bin PATH]

--null is the soak on a machine without PipeWire (macOS): no sound, no probe
window; the app plays into its null sink, which models a device buffer and
counts underruns — the render thread falling behind real time by more than
the buffer. It answers "does the audio thread keep up here", not what the
device's xrun count would be. --bin runs another build (a control).

Each key in --keys is pressed 10 s after the previous one. Plays real sound
through the speakers for the whole run. Build first:
  cargo build --release && cargo build --release --example field_probe -p syn-tui
"""
import argparse, fcntl, json, os, pty, re, select, struct, subprocess, tempfile, termios, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT))['target_directory']

ap = argparse.ArgumentParser()
ap.add_argument('--secs', type=int, default=130)
ap.add_argument('--viz', default='panel')
ap.add_argument('--keys', default='')
ap.add_argument('--scout', default='true')
ap.add_argument('--scout-threads', type=int, default=0)
ap.add_argument('--no-probe', action='store_true')
ap.add_argument('--null', action='store_true')
ap.add_argument('--bin', default=f'{TARGET}/release/synesthesia')
a = ap.parse_args()
if a.null:
    a.no_probe = True


def pw_cat_errors():
    """ERR of the pw-cat node, from the second (settled) pw-top sample."""
    if a.null:
        return None
    out = subprocess.run(['pw-top', '-b', '-n', '2'], capture_output=True, text=True, timeout=10).stdout
    lines = out.strip().splitlines()
    for line in lines[len(lines) // 2:]:
        if line.rstrip().endswith('pw-cat'):
            return int(line.split()[8])
    return None


conf = tempfile.mkdtemp(prefix='soak-config-')
os.makedirs(f'{conf}/synesthesia')
with open(f'{conf}/synesthesia/config.toml', 'w') as f:
    f.write(f'viz = "{a.viz}"\nscout = {a.scout}\nscout_threads = {a.scout_threads}\n')

if not a.no_probe:
    subprocess.Popen(['xfce4-terminal', '--disable-server', '--geometry=120x40', '-x',
                      f'{TARGET}/release/examples/field_probe', '--secs', str(a.secs + 5),
                      '--out', os.path.join(conf, 'probe.json')],
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

pid, fd = pty.fork()
if pid == 0:
    os.execve(a.bin, ['synesthesia'] + (['--no-sound'] if a.null else []),
              dict(os.environ, XDG_CONFIG_HOME=conf, XDG_DATA_HOME=conf, COLORTERM='truecolor'))
# A pty with no size makes ratatui draw nothing at all.
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
os.set_blocking(fd, False)

t0 = time.time()
pressed, start_err, mid_err = 0, None, None
out = bytearray()


def drain(seconds):
    """Reads the pty for `seconds`. Without select the app blocks writing a
    frame into a full pty (macOS keeps only a few KB) and falls behind."""
    end = time.time() + seconds
    while True:
        left = end - time.time()
        if left <= 0:
            return
        if select.select([fd], [], [], left)[0]:
            try:
                out.extend(os.read(fd, 1 << 20)[-4096:])
                del out[:-8192]
            except (BlockingIOError, OSError):
                return


while time.time() - t0 < a.secs:
    drain(0.02)
    el = time.time() - t0
    if start_err is None and el > 6:
        start_err = pw_cat_errors()
    if mid_err is None and el > a.secs / 2:
        mid_err = pw_cat_errors()
    if pressed < len(a.keys) and el > 10 * (pressed + 1):
        os.write(fd, a.keys[pressed].encode())
        pressed += 1
end_err = pw_cat_errors()
os.write(fd, b'q')
drain(3)
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
if a.null:
    m = re.search(rb'null sink: (\d+) underruns \(longest gap ([\d.]+) ms\)', bytes(out))
    found = f'{int(m[1])} underruns, longest gap {m[2].decode()} ms' if m else 'no underrun line (did it exit?)'
    print(f'{a.secs} s, viz {a.viz}, scout {a.scout} ({a.scout_threads or "auto"} threads), '
          f'{pressed} presses, null sink: {found}')
    raise SystemExit
print(f'{a.secs} s, viz {a.viz}, scout {a.scout} ({a.scout_threads or "auto"} threads), '
      f'{pressed} presses: pw-cat ERR {start_err} at 6 s, {mid_err} at half, {end_err} at the end')
