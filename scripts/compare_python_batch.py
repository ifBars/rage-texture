"""Compare full sampled DDS batches including file creation, writes and fsync.

Uses installed native SDK vs unmodified Leonida. Both decoded caches are
disabled. Opening/catalog/analysis and verification hashing are excluded.
Fresh temporary outputs on the same volume, alternating order, warm OS cache.
This measures the SDK with an identical Python output writer on both sides,
not the CLI's atomic-publication workflow or the entire corpus.
"""
import argparse
import hashlib
import importlib
import json
import os
import pathlib
import statistics
import sys
import tempfile
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida', type=pathlib.Path, required=True)
    p.add_argument('--reference-report', type=pathlib.Path, action='append', required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--cases', type=int, default=64)
    p.add_argument('--trials', type=int, default=3)
    a = p.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if a.out.resolve().is_relative_to(root):p.error('reports must stay outside repository')
    if not 1 <= a.cases <= 512 or not 1 <= a.trials <= 15:p.error('invalid limits')
    sys.path.insert(0, str(a.leonida.resolve()/'tools'))
    baseline = importlib.import_module('ttpf_mips')
    import rage_texture._native as module
    from rage_texture import Store

    class UncachedPool(baseline.Pool):
        def read_range(self, start, size, cache=None, **kw):
            return super().read_range(start, size, cache={}, **kw)

    cases = []
    for source in a.reference_report:
        cases.extend(json.loads(source.read_text(encoding='utf-8'))['cases'])
    n = min(a.cases, len(cases))
    if not n:p.error('no reference cases')
    cases = [cases[i*len(cases)//n] for i in range(n)]
    pools, stores, catalogs, analysis = {}, {}, {}, {}
    fingerprint = hashlib.sha256(pathlib.Path(module.__file__).read_bytes()).hexdigest()
    try:
        for case in cases:
            path = case['store']
            stat = pathlib.Path(path).stat()
            if (stat.st_size, stat.st_mtime_ns) != (case['source_size'], case['source_mtime_ns']):
                raise ValueError('reference source changed')
            if path not in pools:
                pools[path] = UncachedPool(path)
                stores[path] = Store(path, cache_bytes=0)
                catalogs[path] = stores[path].textures()
            analysis[(path, case['index'])] = baseline.analyze(pools[path], case['index'], allow_bcpack=False)
        a.out.parent.mkdir(parents=True, exist_ok=True)
        timings = [[], []]
        total_bytes = None
        for trial in range(a.trials):
            for side in ((0, 1) if trial % 2 == 0 else (1, 0)):
                for store in stores.values():store.reset()
                with tempfile.TemporaryDirectory(prefix='rage-batch-', dir=a.out.parent) as folder:
                    outputs = [pathlib.Path(folder)/f'{i:06d}.dds' for i in range(n)]
                    written = 0
                    started = time.perf_counter()
                    for case, dest in zip(cases, outputs):
                        path, index = case['store'], case['index']
                        data = (stores[path].texture_dds(catalogs[path][index]).data if side == 0 else
                                baseline.texture_dds(pools[path], analysis[(path, index)]))
                        with dest.open('xb') as f:
                            f.write(data)
                            f.flush()
                            os.fsync(f.fileno())
                        written += len(data)
                    timings[side].append(time.perf_counter()-started)
                    for case, dest in zip(cases, outputs):
                        if hashlib.sha256(dest.read_bytes()).hexdigest() != case['dds_sha256']:
                            raise ValueError('batch DDS output differs')
                    if total_bytes is not None and written != total_bytes:
                        raise ValueError('output size changed')
                    total_bytes = written
                for store in stores.values():
                    if any(getattr(store.stats(), k) for k in
                           ('cache_hits', 'cache_bytes', 'peak_cache_bytes', 'evictions')):
                        raise ValueError('native decoded cache was used')
        for case in cases:
            stat = pathlib.Path(case['store']).stat()
            if (stat.st_size, stat.st_mtime_ns) != (case['source_size'], case['source_mtime_ns']):
                raise ValueError('source changed during comparison')
        if hashlib.sha256(pathlib.Path(module.__file__).read_bytes()).hexdigest() != fingerprint:
            raise ValueError('extension changed during comparison')
        report = dict(cases=n, stores=len(stores), trials=a.trials, equal=True,
                      decoded_cache_enabled=False, bytes=total_bytes,
                      native_seconds=timings[0], leonida_seconds=timings[1],
                      speedup=statistics.median(timings[1])/statistics.median(timings[0]),
                      native_extension_sha256=fingerprint, scope=__doc__)
        a.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
        print(json.dumps(report, indent=2))
    finally:
        for store in stores.values():store.close()
        for pool in pools.values():pool.close()


if __name__ == '__main__':
    main()
