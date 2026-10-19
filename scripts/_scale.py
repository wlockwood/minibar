import json, subprocess, tempfile, os, time
exe = os.path.abspath('data/brains/obj/bin/visigoth-simulator.exe')
base = json.load(open('examples/train-thugs.json'))
base['opponents'] = ['hold']
base['max_time'] = 20.0
base['layout_jitter'] = 0.0
print("units/side  total units  wall s  per tick ms  per unit-tick us")
for n in (10, 25, 50, 100, 200):
    c = json.loads(json.dumps(base))
    c['scenarios'] = [{"name": "scale", "learner": [{"unit": "armpw", "count": n}], "enemy": [{"unit": "armpw", "count": n}], "separation": 500}]
    d = tempfile.mkdtemp(); p = os.path.join(d, 'c.json'); json.dump(c, open(p, 'w'))
    env = dict(os.environ, RAYON_NUM_THREADS='1')
    t = time.time()
    out = subprocess.run([exe, '--catalog', 'examples/t1-catalog.json', '--evaluate', 'data/brains/obj/champion.json', '--train-config', p, '--eval-seeds', '1', '--eval-seed-base', '5'], capture_output=True, text=True, encoding='utf-8', env=env)
    wall = time.time() - t
    e = json.loads(out.stdout)
    sim = e['total']['duration_sum']          # simulated seconds over both battles (learner on each side)
    ticks = sim * 30
    print("%6d  %10d  %7.2f  %10.2f  %12.2f" % (n, 2 * n, wall, 1000 * wall / max(1, ticks), 1e6 * wall / max(1, ticks) / (2 * n)))
