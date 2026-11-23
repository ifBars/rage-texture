"""Measure the real in-process Python bridge against unchanged Leonida output."""
import argparse
import hashlib
import importlib
import json
import pathlib
import statistics
import sys
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida', type=pathlib.Path, required=True)
    p.add_argument('--store', type=pathlib.Path, required=True)
    p.add_argument('--index', type=int, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--trials', type=int, default=7)
    p.add_argument('--mips', type=int)
    p.add_argument('--history', type=pathlib.Path)
    p.add_argument('--allow-provisional', action='store_true')
    p.add_argument('--baseline-per-export-cache', action='store_true',
                   help="comparison control: allow Leonida's normal within-export reuse; native cache stays disabled")
    a = p.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if a.out.resolve().is_relative_to(root):
        p.error('raw report must remain outside repository')
    if not 1 <= a.trials <= 31:
        p.error('trials must be 1..31')
    sys.path.insert(0, str(a.leonida.resolve()/'tools'))
    sys.path.insert(0, str(root/'integrations/leonida'))
    import rage_texture._native as native_module
    from native_dds import NativeDdsBackend
    baseline = importlib.import_module('ttpf_mips')

    class UncachedPool(baseline.Pool):
        def read_range(self, start, size, cache=None, **kwargs):
            return super().read_range(start, size, cache={}, **kwargs)

    module_path = pathlib.Path(native_module.__file__)
    fingerprint = hashlib.sha256(module_path.read_bytes()).hexdigest()
    source_stamp = a.store.stat()
    pool = (baseline.Pool if a.baseline_per_export_cache else UncachedPool)(str(a.store))
    try:
        analysis = baseline.analyze(pool, a.index, allow_bcpack=True)
        options = dict(mips=a.mips, allow_provisional=a.allow_provisional,
                       history=str(a.history) if a.history else None)
        expected = baseline.texture_dds(pool, analysis, **options)
        native_times, baseline_times, work = [], [], []
        with NativeDdsBackend(a.store, history=a.history,
                              allow_provisional=a.allow_provisional) as backend:
            history_digest = backend.store.history_digest
            def rust():
                backend.store.reset()
                started = time.perf_counter()
                result = backend.export(pool, analysis, mips=a.mips)
                elapsed = time.perf_counter()-started
                if result.data != expected:
                    raise ValueError('DDS output differs')
                stats = backend.store.stats()
                if any(getattr(stats,k) for k in ('cache_hits','cache_bytes','peak_cache_bytes','evictions')):
                    raise ValueError('decoded cache was used')
                native_times.append(elapsed)
                work.append(dict(input_bytes=result.input_bytes, source_bytes=stats.source_bytes,
                                 decodes=stats.decodes))

            def python():
                started = time.perf_counter()
                result = baseline.texture_dds(pool, analysis, **options)
                elapsed = time.perf_counter()-started
                if result != expected:
                    raise ValueError('baseline output changed')
                baseline_times.append(elapsed)

            for trial in range(a.trials):
                if trial % 2:rust();python()
                else:python();rust()
        if hashlib.sha256(module_path.read_bytes()).hexdigest() != fingerprint:
            raise ValueError('native extension changed during comparison')
        after = a.store.stat()
        if (source_stamp.st_size, source_stamp.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
            raise ValueError('source changed during comparison')
        report = dict(native_extension_sha256=fingerprint, trials=a.trials, equal=True,
                      benchmark_script_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                      baseline_source_sha256=hashlib.sha256((a.leonida/'tools/ttpf_mips.py').read_bytes()).hexdigest(),
                      source_path=str(a.store.resolve()), index=a.index,
                      source_size=source_stamp.st_size, source_mtime_ns=source_stamp.st_mtime_ns,
                      history_digest=history_digest,
                      requested_mips=a.mips, allow_provisional=a.allow_provisional,
                      decoded_cache_enabled=a.baseline_per_export_cache,
                      native_decoded_cache_enabled=False,
                      persistent_decoded_cache_enabled=False,
                      baseline_cache_scope='per-export' if a.baseline_per_export_cache else 'no reuse between range reads',
                      native_seconds=native_times,
                      baseline_seconds=baseline_times, work=work, dds_bytes=len(expected),
                      speedup=statistics.median(baseline_times)/statistics.median(native_times),
                      scope='real Python adapter, descriptor checks, Python/Rust conversion, source guards, complete DDS construction; warm OS cache; startup/catalog/hash/file writes excluded')
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
        print(json.dumps(dict(equal=True, speedup=report['speedup'],
                             native_ms=statistics.median(native_times)*1000,
                             baseline_ms=statistics.median(baseline_times)*1000),indent=2))
    finally:
        pool.close()


if __name__ == '__main__':
    main()
