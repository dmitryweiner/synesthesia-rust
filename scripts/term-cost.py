#!/usr/bin/env python3
"""What a screen costs outside this process: opens a command in its own
xfce4-terminal window at 120x40 and reads the CPU of that terminal, of Xorg
and of xfwm4 from /proc, next to the command's own. GRAPHICS.md, the probe.

  scripts/term-cost.py idle app FPS:COLOR:BITS:BREATHE ...
  e.g. scripts/term-cost.py idle app 8:true:8:1 8:256:8:0

Build first: cargo build --release && cargo build --release --example field_probe -p syn-tui
"""
import os, subprocess, time, json, sys, tempfile
HZ = os.sysconf('SC_CLK_TCK')
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TARGET = json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT))['target_directory']
PROBE = TARGET + '/release/examples/field_probe'
APP = TARGET + '/release/synesthesia'
SCR = tempfile.mkdtemp(prefix='term-cost-')
SECS = 24; SKIP = 4; TAIL = 2

def ticks(pid):
    try:
        f = open(f'/proc/{pid}/stat').read().rsplit(')', 1)[1].split()
        return int(f[11]) + int(f[12])
    except Exception:
        return None

def pidof(name):
    return int(subprocess.check_output(['pgrep', '-x', name]).split()[0])

def descendants(pid):
    out = []
    for p in os.listdir('/proc'):
        if not p.isdigit(): continue
        try:
            pp = int(open(f'/proc/{p}/stat').read().rsplit(')', 1)[1].split()[1])
        except Exception:
            continue
        if pp == pid: out.append(int(p))
    res = list(out)
    for c in out: res += descendants(c)
    return res

XORG, XFWM = pidof('Xorg'), pidof('xfwm4')

def run(label, cmd):
    term = subprocess.Popen(['xfce4-terminal', '--disable-server', '--geometry=120x40', '--hide-menubar', '--hide-toolbar', '-x'] + cmd,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(SKIP)
    kids = descendants(term.pid)
    pids = {'term': [term.pid], 'Xorg': [XORG], 'xfwm4': [XFWM], 'app': kids}
    a = {k: sum(ticks(p) or 0 for p in v) for k, v in pids.items()}
    t0 = time.time()
    time.sleep(SECS - SKIP - TAIL)
    b = {k: sum(ticks(p) or 0 for p in v) for k, v in pids.items()}
    dt = time.time() - t0
    term.wait(timeout=30)
    cpu = {k: round(100 * (b[k] - a[k]) / HZ / dt, 1) for k in a}
    print(label, json.dumps(cpu), flush=True)
    return cpu

configs = sys.argv[1:]
for c in configs:
    if c == 'idle':
        run('idle (sleep)', ['sleep', str(SECS)])
    elif c == 'app':
        run('app ui_fps=8', ['timeout', str(SECS), APP, '--no-sound', '--no-scout'])
    else:
        fps, color, bits, breathe = c.split(':')
        out = f'{SCR}/probe-{c.replace(":", "_")}.json'
        run(c, [PROBE, '--secs', str(SECS - 1), '--fps', fps, '--color', color, '--bits', bits, '--breathe', breathe, '--out', out])
        time.sleep(0.5)
        try: print('   probe', open(out).read().strip(), flush=True)
        except Exception as e: print('   no probe output', e)
