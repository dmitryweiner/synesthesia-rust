#!/usr/bin/env python3
"""What a screen costs outside this process: opens a command in its own
xfce4-terminal window at 120x40 and reads the CPU of that terminal, of Xorg
and of xfwm4 from /proc, next to the command's own. GRAPHICS.md, the probe.

  scripts/term-cost.py idle app app:off app:panel app:full FPS:COLOR:BITS:BREATHE ...
  e.g. scripts/term-cost.py idle app:off app:panel 8:true:8:1 8:256:8:0

`app` runs the interface on the user's own config; `app:<viz>` runs it on a
throwaway config with the picture set to off, panel or full. The `viz`
column is the picture thread alone. With SHOT=dir set, each run leaves a
screenshot of its window there.

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

def threads_named(pids, name):
    out = []
    for pid in pids:
        try:
            for tid in os.listdir(f'/proc/{pid}/task'):
                if open(f'/proc/{pid}/task/{tid}/comm').read().strip() == name:
                    out.append(f'{pid}/task/{tid}')
        except Exception:
            pass
    return out

def run(label, cmd, env=None):
    term = subprocess.Popen(['xfce4-terminal', '--disable-server', '--geometry=120x40', '--hide-menubar', '--hide-toolbar', '-x'] + cmd,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
    time.sleep(SKIP)
    kids = descendants(term.pid)
    pids = {'term': [term.pid], 'Xorg': [XORG], 'xfwm4': [XFWM], 'app': kids, 'viz': threads_named(kids, 'picture')}
    a = {k: sum(ticks(p) or 0 for p in v) for k, v in pids.items()}
    t0 = time.time()
    if os.environ.get('SHOT'):
        time.sleep(4)
        name = label.replace(' ', '_').replace(':', '-').replace('=', '')
        subprocess.run(['xfce4-screenshooter', '-w', '-s', os.path.join(os.environ['SHOT'], name + '.png')],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(max(0, SECS - SKIP - TAIL - (time.time() - t0)))
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
        run('app', ['timeout', str(SECS), APP, '--no-sound', '--no-scout'])
    elif c.startswith('app:'):
        viz = c.split(':', 1)[1]
        conf = tempfile.mkdtemp(prefix='term-cost-config-')
        os.makedirs(f'{conf}/synesthesia')
        open(f'{conf}/synesthesia/config.toml', 'w').write(f'viz = "{viz}"\n')
        run(c, ['timeout', str(SECS), APP, '--no-sound', '--no-scout'], env={**os.environ, 'XDG_CONFIG_HOME': conf})
    else:
        fps, color, bits, breathe = c.split(':')
        out = f'{SCR}/probe-{c.replace(":", "_")}.json'
        run(c, [PROBE, '--secs', str(SECS - 1), '--fps', fps, '--color', color, '--bits', bits, '--breathe', breathe, '--out', out])
        time.sleep(0.5)
        try: print('   probe', open(out).read().strip(), flush=True)
        except Exception as e: print('   no probe output', e)
