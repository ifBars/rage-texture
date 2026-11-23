"""Compare equivalent virtual reads against an external, unmodified Leonida checkout.

Inputs and reports stay local. No game data or baseline source is redistributed.
Python 3 standard library; Leonida may optionally use its own installed dependencies.
"""
import argparse
import hashlib
import importlib
import json
import pathlib
import platform
import statistics
import subprocess
import sys
import time


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--leonida', type=pathlib.Path, required=True)
    ap.add_argument('--store', type=pathlib.Path, required=True)
    ap.add_argument('--exe', type=pathlib.Path, required=True)
    ap.add_argument('--out', type=pathlib.Path, required=True)
    ap.add_argument('--codec', type=int, choices=[0, 1, 8], default=8)
    ap.add_argument('--chunks', type=int, default=64)
    ap.add_argument('--range-bytes', type=int, default=4096, help='0 reads full chunks')
    ap.add_argument('--repeats', type=int, default=100)
    ap.add_argument('--trials', type=int, default=7)
    ap.add_argument('--batch', action='store_true', help='Rust source-checked batch vs baseline sequential requests')
    args = ap.parse_args()
    if not (1 <= args.chunks <= 4096 and 1 <= args.repeats <= 10000 and 1 <= args.trials <= 31 and args.range_bytes >= 0):
        ap.error('invalid workload bounds')
    if args.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        ap.error('reports must be outside the repository')
    sys.path.insert(0, str(args.leonida.resolve() / 'tools'))
    baseline = importlib.import_module('ttpf_mips')
    metadata = json.loads(subprocess.check_output([str(args.exe), 'inspect', str(args.store)]))
    eligible = [c for c in metadata['chunks'] if c['codec'] == args.codec]
    count = min(args.chunks, len(eligible))
    if not count:
        raise ValueError('no chunks matching codec')
    chosen = [eligible[i * len(eligible) // count] for i in range(count)]
    ranges = [(c['virtual_offset'], min(c['decoded_bytes'], args.range_bytes) if args.range_bytes else c['decoded_bytes']) for c in chosen]
    budget = sum(c['decoded_bytes'] for c in chosen)
    plan = dict(ranges=ranges, cache_bytes=budget, repeats=args.repeats, trials=args.trials, allow_fitted_bc1=args.codec == 1,batch=args.batch)
    args.out.mkdir(parents=True, exist_ok=True)
    plan_path = args.out / 'plan.json'
    plan_path.write_text(json.dumps(plan), encoding='utf-8')
    pool = baseline.Pool(str(args.store))
    try:
        cache = {}
        def read(start, size):
            return pool.read_range(start, size, cache=cache, bcpack_mode='provisional' if args.codec == 1 else 'strict')
        expected = [hashlib.sha256(read(a, n)).hexdigest() for a, n in ranges]
        # Alternate implementations between trials; both use warm OS page cache.
        # Both receive an empty decoded cache for cold, then reuse it for hot.
        py_cold, py_hot, rs_cold, rs_hot = [], [], [], []
        rust_results = []
        one_trial = dict(plan, trials=1)
        plan_path.write_text(json.dumps(one_trial), encoding='utf-8')
        def rust_trial():
            result = json.loads(subprocess.check_output([str(args.exe), 'run', str(args.store), str(plan_path)]))
            if result['sha256'] != expected:
                raise ValueError('decoded byte hashes disagree; no performance gate may pass')
            rust_results.append(result)
            rs_cold.extend(result['cold_seconds']); rs_hot.extend(result['hot_seconds'])
        def python_trial():
            cache.clear()
            started = time.perf_counter()
            for a, n in ranges:
                read(a, n)
            py_cold.append(time.perf_counter() - started)
            started = time.perf_counter()
            for _ in range(args.repeats):
                for a, n in ranges:
                    read(a, n)
            py_hot.append(time.perf_counter() - started)
        for i in range(args.trials):
            if i % 2:
                rust_trial(); python_trial()
            else:
                python_trial(); rust_trial()
        def git_sha(folder):
            return subprocess.check_output(['git', '-C', str(folder), 'rev-parse', 'HEAD'], text=True).strip()
        result = dict(
            baseline='Leonida tools/ttpf_mips.py Pool.read_range', baseline_commit=git_sha(args.leonida),
            executable_sha256=hashlib.sha256(args.exe.read_bytes()).hexdigest(),
            platform=platform.platform(), python=sys.version, codec=args.codec,
            rust_batch=args.batch,
            request_count=len(ranges), range_bytes=args.range_bytes, repeats=args.repeats, trials=args.trials,
            decoded_cache_budget_bytes=budget, equal_sha256=True,
            python_cold_seconds=py_cold, rust_cold_seconds=rs_cold,
            python_hot_seconds=py_hot, rust_hot_seconds=rs_hot,
            cold_speedup=statistics.median(py_cold)/statistics.median(rs_cold),
            hot_speedup=statistics.median(py_hot)/statistics.median(rs_hot),
            rust_stats=[r['stats'] for r in rust_results],
            scope='chunk-backed virtual byte reads; warm OS cache; hot decoded workset fits in both caches; no detiling/export',
        )
        (args.out / 'comparison.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
        print(json.dumps(result, indent=2))
    finally:
        pool.close()


if __name__ == '__main__':
    main()
