"""Complete RGBA preview comparison with disabled decoded caches on both sides.

Both helpers exclude opening/catalog parsing, hashing, process startup and
file writes. SixNG reads the full allocation; the SDK reads the requested
subresource. Both fulfill the same mip/slice request. OS cache is warm.
"""
import argparse
import hashlib
import json
import pathlib
import statistics
import subprocess


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--native', type=pathlib.Path, required=True)
    p.add_argument('--reference', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--trials', type=int, default=7)
    p.add_argument('--case', nargs=5, action='append', required=True,
                   metavar=('NAME', 'STORE', 'INDEX', 'MIP', 'SLICE'))
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('reports must stay outside repository')
    if not 1 <= a.trials <= 31 or len(a.case) > 256:
        p.error('invalid trial/case count')
    binaries = [a.native, a.reference]
    fingerprints = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    report = dict(native_sha256=fingerprints[0], reference_sha256=fingerprints[1],
                  reference_commit='7881042c294f3eeea1e8153e7bcafd84f9d787b9',
                  decoded_cache_enabled=False, scope=__doc__, cases=[], errors=[])

    def call(args):
        r = subprocess.run([str(x) for x in args], capture_output=True, text=True)
        if r.returncode:
            raise ValueError(r.stderr.strip()[-1000:])
        return json.loads(r.stdout)

    for name, store, index, mip, sl in a.case:
        source = pathlib.Path(store)
        stamp = (source.stat().st_size, source.stat().st_mtime_ns)
        try:
            first = call([a.native, store, index, mip, sl])
            if first['asset_key'] is None:
                raise ValueError('SixNG lookup requires an asset key')
            commands = ([a.native, store, index, mip, sl],
                        [a.reference, 'image', store, f"{first['asset_key']:016x}", mip, sl])
            times = [[], []]
            for trial in range(a.trials):
                for side in ((0, 1) if trial % 2 == 0 else (1, 0)):
                    result = call(commands[side])
                    if side == 0:
                        if any(result['stats'][k] for k in
                               ('cache_hits', 'cache_bytes', 'peak_cache_bytes', 'evictions')):
                            raise ValueError('native decoded cache was used')
                    elif result['skipped']:
                        raise ValueError('SixNG skipped source data')
                    if (result['bytes'], result['sha256']) != (first['bytes'], first['sha256']):
                        raise ValueError('RGBA bytes disagree')
                    times[side].append(result['seconds'])
            report['cases'].append(dict(name=name, equal=True, bytes=first['bytes'],
                native_seconds=times[0], sixng_seconds=times[1],
                speedup=statistics.median(times[1])/statistics.median(times[0])))
        except (ValueError, KeyError) as e:
            report['errors'].append(dict(name=name, error=str(e)))
        if stamp != (source.stat().st_size, source.stat().st_mtime_ns):
            raise ValueError('source changed during comparison')
    if fingerprints != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('binary changed during comparison')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(dict(cases=[dict(name=c['name'], speedup=c['speedup'], equal=True)
                                for c in report['cases']], errors=report['errors']), indent=2))
    return bool(report['errors']) or not report['cases']


if __name__ == '__main__':
    raise SystemExit(main())
