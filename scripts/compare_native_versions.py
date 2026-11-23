"""Alternate two native checkpoints on identical uncached DDS requests."""
import argparse
import hashlib
import json
import pathlib
import statistics
import subprocess


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--before', type=pathlib.Path, required=True)
    p.add_argument('--after', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--case', nargs=3, action='append', required=True,
                   metavar=('NAME','STORE','INDEX'))
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('reports must stay outside repository')
    if len(a.case) > 64:p.error('too many cases')
    binaries = [a.before,a.after]
    stamps = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    report = dict(before_sha256=stamps[0],after_sha256=stamps[1],
                  decoded_cache_enabled=False,cases=[],
                  scope='complete native DDS construction; three alternating runs of seven extractions; source guards included; warm OS cache; startup/catalog/hash/file writes excluded')
    for name, store, index in a.case:
        times = [[],[]]
        expected = None
        source = pathlib.Path(store).stat()
        for run in range(3):
            for side in ((0,1) if run % 2 == 0 else (1,0)):
                result = json.loads(subprocess.check_output([
                    str(binaries[side]),'bench-export',store,index,'7','--no-cache']))
                for i, seconds in enumerate(result['seconds']):
                    digest = (result['bytes'],result['sha256'][i])
                    if expected is not None and expected != digest:
                        raise ValueError('output differs between checkpoints')
                    expected = digest
                    if any(result['stats'][i][k] for k in
                           ('cache_hits','cache_bytes','peak_cache_bytes','evictions')):
                        raise ValueError('decoded cache was used')
                    times[side].append(seconds)
        stat = pathlib.Path(store).stat()
        if (source.st_size,source.st_mtime_ns) != (stat.st_size,stat.st_mtime_ns):
            raise ValueError('source changed during comparison')
        report['cases'].append(dict(name=name,equal=True,before_seconds=times[0],
            after_seconds=times[1],after_over_before=statistics.median(times[1])/statistics.median(times[0])))
    if stamps != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('binary changed during comparison')
    a.out.parent.mkdir(parents=True,exist_ok=True)
    a.out.write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps([dict(name=x['name'],after_over_before=x['after_over_before'],equal=True)
                      for x in report['cases']],indent=2))


if __name__ == '__main__':
    main()
