"""Differential BCPack history acceptance and byte checks against Leonida."""
import argparse
import hashlib
import importlib
import json
import pathlib
import subprocess
import sys


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida', type=pathlib.Path, required=True)
    p.add_argument('--store', type=pathlib.Path, required=True, action='append')
    p.add_argument('--exe', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--per-codec', type=int, default=8)
    p.add_argument('--history', type=pathlib.Path)
    p.add_argument('--allow-provisional', action='store_true')
    a = p.parse_args()
    if not 1 <= a.per_codec <= 4096:p.error('per-codec must be 1..4096')
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('raw report must remain outside repository')
    sys.path.insert(0,str(a.leonida.resolve()/'tools'))
    reference = importlib.import_module('bcpack')
    pools = importlib.import_module('ttpf_mips')
    fingerprint = hashlib.sha256(a.exe.read_bytes()).hexdigest()
    report = dict(equal=0,both_refused=0,unknown_placeholder_refused=0,
                  errors=[],by_codec={},executable_sha256=fingerprint,
                  allow_provisional=a.allow_provisional)
    for path in a.store:
        stamp = (path.stat().st_size,path.stat().st_mtime_ns)
        pool = pools.Pool(str(path))
        try:
            with path.open('rb') as source:
                for codec in (1,3,4,5,6,7):
                    chunks = [c for c in pool.chunks if c[4]==codec]
                    n = min(len(chunks),a.per_codec)
                    counts = report['by_codec'].setdefault(str(codec),dict(equal=0,both_refused=0,unknown_placeholder_refused=0))
                    for i in range(n):
                        start,size,offset,packed_size,_ = chunks[i*len(chunks)//n][:5]
                        source.seek(offset);packed = source.read(packed_size)
                        expected = None;labels=set();reason=None
                        try:
                            expected,labels = reference.decode_with_info(packed,codec,size,
                                  allow_provisional=a.allow_provisional,
                                  history=str(a.history) if a.history else None)
                        except reference.BcpackProvisionalError as e:reason=str(e)
                        except Exception as e:
                            report['errors'].append(dict(codec=codec,sample=i,error='reference structural failure: '+str(e)));continue
                        command=[str(a.exe),'range-hash',str(path),str(start),str(size),
                                 '--allow-provisional' if a.allow_provisional else '--confirmed']
                        if a.history:command.extend(['--history',str(a.history)])
                        actual=subprocess.run(command,text=True,capture_output=True)
                        if expected is None:
                            if actual.returncode:
                                report['both_refused']+=1;counts['both_refused']+=1
                            else:report['errors'].append(dict(codec=codec,sample=i,error='native accepted a strict reference refusal',reference=reason))
                            continue
                        if actual.returncode:
                            if a.allow_provisional and any('unknown' in label for label in labels):
                                report['unknown_placeholder_refused']+=1;counts['unknown_placeholder_refused']+=1
                            else:report['errors'].append(dict(codec=codec,sample=i,error='native rejected reference success',native=actual.stderr.strip()))
                            continue
                        decoded=json.loads(actual.stdout)
                        if decoded['bytes'] != len(expected) or decoded['sha256'] != hashlib.sha256(expected).hexdigest():
                            report['errors'].append(dict(codec=codec,sample=i,error='decoded byte mismatch'));continue
                        stats=decoded['stats']
                        if any(stats[k] for k in ('cache_hits','cache_bytes','peak_cache_bytes','evictions')):
                            raise ValueError('decoded cache was used')
                        quality=decoded['quality']
                        if len(quality)!=1 or 'HistoryBc' not in quality[0]:raise ValueError('missing history provenance')
                        if not labels and quality[0]['HistoryBc']['provisional_dependencies']:
                            raise ValueError('unexpected provisional dependency')
                        if labels and not quality[0]['HistoryBc']['provisional_dependencies']:
                            raise ValueError('provisional reference output was labelled confirmed')
                        report['equal']+=1;counts['equal']+=1
        finally:pool.close()
        if stamp != (path.stat().st_size,path.stat().st_mtime_ns):raise ValueError('source changed during validation')
    if fingerprint != hashlib.sha256(a.exe.read_bytes()).hexdigest():raise ValueError('native executable changed during validation')
    a.out.parent.mkdir(parents=True,exist_ok=True)
    a.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report,indent=2))
    return bool(report['errors']) or not report['equal']


if __name__ == '__main__':
    raise SystemExit(main())
