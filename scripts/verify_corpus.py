"""Check full decoded chunks against external Leonida, without exporting game bytes."""
import argparse
import hashlib
import importlib
import json
import pathlib
import subprocess
import sys
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida', type=pathlib.Path, required=True)
    p.add_argument('--stores', type=pathlib.Path, required=True)
    p.add_argument('--exe', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--samples', type=int, default=16)
    args = p.parse_args()
    if not 1 <= args.samples <= 4096:
        p.error('samples must be 1..4096')
    if args.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('reports must be outside repository')
    sys.path.insert(0, str(args.leonida.resolve() / 'tools'))
    baseline = importlib.import_module('ttpf_mips')
    result = dict(stores=0, full_chunks=0, decoded_bytes=0, errors=[], unsupported_chunks=0)
    for path in sorted(args.stores.rglob('*.ttpf')):
        try:
            meta = json.loads(subprocess.check_output([str(args.exe), 'inspect', str(path)]))
            eligible = [c for c in meta['chunks'] if c['codec'] in (0, 8)]
            result['unsupported_chunks'] += sum(c['codec'] not in (0, 8) for c in meta['chunks'])
            chosen = []
            for codec in (0, 8):
                group = [c for c in eligible if c['codec'] == codec]
                n = min(args.samples, len(group))
                chosen.extend(group[i * len(group) // n] for i in range(n))
            if not chosen:
                raise ValueError('no lossless chunks available')
            pool = baseline.Pool(str(path))
            try:
                if len(pool.chunks) != len(meta['chunks']):
                    raise ValueError('chunk count mismatch')
                for a, b in zip(pool.chunks, meta['chunks']):
                    if (a[0], a[1], a[2], a[3], a[4]) != (b['virtual_offset'], b['decoded_bytes'], b['file_offset'], b['packed_bytes'], b['codec']):
                        raise ValueError('chunk metadata mismatch')
                expected = [hashlib.sha256(pool.read_range(c['virtual_offset'], c['decoded_bytes'])).hexdigest() for c in chosen]
            finally:
                pool.close()
            plan = dict(ranges=[(c['virtual_offset'], c['decoded_bytes']) for c in chosen], cache_bytes=0, repeats=1, trials=1, allow_fitted_bc1=False)
            with tempfile.TemporaryDirectory(prefix='rage-verify-') as temp:
                plan_path = pathlib.Path(temp) / 'plan.json'
                plan_path.write_text(json.dumps(plan), encoding='utf-8')
                actual = json.loads(subprocess.check_output([str(args.exe), 'run', str(path), str(plan_path)]))
            if actual['sha256'] != expected:
                raise ValueError('full-chunk byte hashes disagree')
            result['stores'] += 1
            result['full_chunks'] += len(chosen)
            result['decoded_bytes'] += sum(c['decoded_bytes'] for c in chosen)
        except Exception as error:
            result['errors'].append(dict(store=path.name, error=str(error)))
    if not result['stores']:
        result['errors'].append(dict(error='no stores checked'))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps(result, indent=2))
    return bool(result['errors'])


if __name__ == '__main__':
    sys.exit(main())
