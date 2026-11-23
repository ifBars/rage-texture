"""Run named complete DDS workloads with decoded caching disabled on both sides."""
import argparse
import hashlib
import json
import pathlib
import re
import statistics
import subprocess
import sys


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida', type=pathlib.Path, required=True)
    p.add_argument('--exe', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--trials', type=int, default=7)
    p.add_argument('--case', nargs=3, action='append', metavar=('NAME', 'STORE', 'INDEX'), required=True)
    a = p.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if a.out.resolve().is_relative_to(root):
        p.error('reports must be outside repository')
    if not 1 <= a.trials <= 31 or len(a.case) > 64:
        p.error('invalid trial/case count')
    names = [x[0] for x in a.case]
    if len(set(names)) != len(names) or any(not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', n) for n in names):
        p.error('case names must be unique filename-safe identifiers')
    fingerprint = hashlib.sha256(a.exe.read_bytes()).hexdigest()
    a.out.mkdir(parents=True, exist_ok=True)
    rows = []
    for name, store, index in a.case:
        report = a.out / (name + '.json')
        subprocess.run([sys.executable, str(root / 'scripts/compare_export.py'),
                        '--leonida', str(a.leonida), '--exe', str(a.exe),
                        '--store', store, '--index', index, '--out', str(report),
                        '--trials', str(a.trials), '--no-cache'],
                       stdout=subprocess.DEVNULL, check=True)
        r = json.loads(report.read_text(encoding='utf-8'))
        if r['executable_sha256'] != fingerprint:
            raise ValueError('binary changed during comparison')
        shape = {k: r['shape'][k] for k in ('width', 'height', 'depth', 'mips', 'format', 'dimension')}
        rows.append(dict(name=name, shape=shape, native_ms=statistics.median(r['rust_seconds'])*1000,
                         baseline_ms=statistics.median(r['leonida_seconds'])*1000,
                         speedup=r['speedup'], equal=r['equal'], dds_bytes=r['dds_bytes'],
                         input_bytes=r['native_input_bytes'],
                         physical_bytes=[s['source_bytes'] for s in r['rust_stats']],
                         decodes=[s['decodes'] for s in r['rust_stats']]))
        print(json.dumps(rows[-1]), flush=True)
    if hashlib.sha256(a.exe.read_bytes()).hexdigest() != fingerprint:
        raise ValueError('binary changed during comparison')
    (a.out / 'summary.json').write_text(json.dumps(dict(executable_sha256=fingerprint,
          trials=a.trials, decoded_cache_enabled=False, cases=rows), indent=2), encoding='utf-8')


if __name__ == '__main__':
    main()
