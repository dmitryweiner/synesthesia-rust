#!/usr/bin/env python3
"""Xrun soak with the sound on: runs the interface in a pty (so keys can be
pressed on a schedule), next to a terminal window running the picture probe
at 8 fps (so the terminal and X are loaded as in real use), and reads the
xrun count of the `pw-cat` node from PipeWire's own ERR counter. The player
keeps no xrun count of its own. GRAPHICS.md, "Soak".

  scripts/soak.py [--secs 130] [--viz panel|spectrum|full] [--keys ldldr...]
                  [--scout true|false] [--scout-threads N] [--no-probe]

Each key in --keys is pressed 10 s after the previous one. Plays real sound
through the speakers for the whole run. Build first:
  cargo build --release && cargo build --release --example field_probe -p syn-tui
"""
import argparse, fcntl, json, os, pty, struct, subprocess, tempfile, termios, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT))['target_directory']

ap = argparse.ArgumentParser()
ap.add_argument('--secs', type=int, default=130)
ap.add_argument('--viz', default='panel')
ap.add_argument('--keys', default='')
ap.add_argument('--scout', default='true')
ap.add_argument('--scout-threads', type=int, default=0)
ap.add_argument('--no-probe', action='store_true')
a = ap.parse_args()


def pw_cat_errors():
    """ERR of the pw-cat node, from the second (settled) pw-top sample."""
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
    os.execve(f'{TARGET}/release/synesthesia', ['synesthesia'],
              dict(os.environ, XDG_CONFIG_HOME=conf, COLORTERM='truecolor'))
# A pty with no size makes ratatui draw nothing at all.
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 0, 0))
os.set_blocking(fd, False)

t0 = time.time()
pressed, start_err, mid_err = 0, None, None
while time.time() - t0 < a.secs:
    try:
        os.read(fd, 1 << 20)
    except BlockingIOError:
        pass
    el = time.time() - t0
    if start_err is None and el > 6:
        start_err = pw_cat_errors()
    if mid_err is None and el > a.secs / 2:
        mid_err = pw_cat_errors()
    if pressed < len(a.keys) and el > 10 * (pressed + 1):
        os.write(fd, a.keys[pressed].encode())
        pressed += 1
    time.sleep(0.02)
end_err = pw_cat_errors()
os.write(fd, b'q')
time.sleep(1.5)
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
print(f'{a.secs} s, viz {a.viz}, scout {a.scout} ({a.scout_threads or "auto"} threads), '
      f'{pressed} presses: pw-cat ERR {start_err} at 6 s, {mid_err} at half, {end_err} at the end')
