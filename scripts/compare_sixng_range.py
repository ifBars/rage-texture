"""Cache-disabled virtual-read comparison against unmodified SixNG core.

This measures a reader primitive, not full texture extraction. Both timed
regions exclude opening/parsing the store, process startup and hashing.
"""
import argparse
import hashlib
import json
import pathlib
import statistics
import subprocess


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--store', type=pathlib.Path, required=True)
    p.add_argument('--exe', type=pathlib.Path, required=True)
    p.add_argument('--reference', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--samples', type=int, default=6)
    p.add_argument('--trials', type=int, default=7)
    a = p.parse_args()
    if not 1 <= a.samples <= 64 or not 1 <= a.trials <= 31:
        p.error('samples must be 1..64 and trials 1..31')
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('report must be outside repository')

    def call(command):
        return json.loads(subprocess.check_output([str(x) for x in command]))

    binaries = [a.exe, a.reference]
    fingerprints = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    meta = call([a.exe, 'inspect', a.store])
    group = [c for c in meta['chunks'] if c['codec'] in (0, 8)]
    n = min(a.samples, len(group))
    if not n:
        p.error('no lossless chunks in store')
    result = dict(native_sha256=fingerprints[0], reference_sha256=fingerprints[1],
                  reference_commit='702435cd56fd09de4f456ddc94d60886de7365ee',
                  chunk_count=len(meta['chunks']), decoded_cache_enabled=False,
                  scope='virtual read/decode/copy; native source guards included; '
                        'warm OS cache; startup/catalog/hash/output writes excluded', cases=[])
    for i in range(n):
        chunk = group[i * len(group) // n]
        for length in sorted({min(4096, chunk['decoded_bytes']), chunk['decoded_bytes']}):
            native, reference, expected = [], [], None
            for trial in range(a.trials):
                for implementation in ([0, 1] if trial % 2 == 0 else [1, 0]):
                    command = ([a.exe, 'range-hash', a.store] if implementation == 0 else
                               [a.reference, 'range', a.store])
                    value = call(command + [chunk['virtual_offset'], length])
                    if value.get('skipped', 0):
                        raise ValueError('reference skipped bytes')
                    if implementation == 0:
                        stats = value['stats']
                        if any(stats[k] for k in ('cache_hits', 'cache_bytes', 'peak_cache_bytes', 'evictions')):
                            raise ValueError('native cache used')
                    digest = (value['bytes'], value['sha256'])
                    if expected is not None and digest != expected:
                        raise ValueError('byte mismatch')
                    expected = digest
                    (native if implementation == 0 else reference).append(value['seconds'])
            result['cases'].append(dict(codec=chunk['codec'], length=length,
                                        virtual_offset=chunk['virtual_offset'],
                                        native_seconds=native, sixng_seconds=reference,
                                        speedup=statistics.median(reference)/statistics.median(native),
                                        equal=True))
    if fingerprints != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('binary changed during comparison')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps(dict(chunk_count=result['chunk_count'], equal=True,
                         cases=[dict(length=c['length'], speedup=c['speedup'])
                                for c in result['cases']]), indent=2))


if __name__ == '__main__':
    main()
