"""Benchmark complete DDS construction; --no-cache disables decoded caches on BOTH sides."""
import argparse
import hashlib
import importlib
import json
import pathlib
import statistics
import subprocess
import sys
import time


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida',type=pathlib.Path,required=True);p.add_argument('--store',type=pathlib.Path,required=True)
    p.add_argument('--exe',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True)
    p.add_argument('--index',type=int,default=0);p.add_argument('--trials',type=int,default=7)
    p.add_argument('--no-cache',action='store_true',help='disable decoded chunk reuse in BOTH implementations')
    a=p.parse_args()
    if not 1<=a.trials<=31:p.error('trials must be 1..31')
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):p.error('report must be outside repository')
    sys.path.insert(0,str(a.leonida.resolve()/'tools'));base=importlib.import_module('ttpf_mips')
    class BenchmarkPool(base.Pool):
        profile=False
        def read_range(self,start,size,cache=None,**kwargs):
            started=time.perf_counter()
            value=super().read_range(start,size,cache={} if a.no_cache else cache,**kwargs)
            if self.profile:reference_stages['read_decode_seconds']+=time.perf_counter()-started
            return value
    reference_stages=dict(read_decode_seconds=0.,detile_seconds=0.,dds_seconds=0.)
    pool=BenchmarkPool(str(a.store))
    try:
        source_stamp=a.store.stat()
        analysis=base.analyze(pool,a.index,allow_bcpack=False)
        expected=hashlib.sha256(base.texture_dds(pool,analysis)).hexdigest()
        native=[];reference=[];native_stats=[];native_stages=[];native_input_bytes=[]
        def rust():
            command=[str(a.exe),'bench-export',str(a.store),str(a.index),'1']
            if a.no_cache:command+=['--no-cache']
            r=json.loads(subprocess.check_output(command))
            if r['sha256']!=[expected]:raise ValueError('DDS bytes disagree')
            if a.no_cache and any(s['cache_hits'] or s['cache_bytes'] or s['peak_cache_bytes'] or s['evictions'] for s in r['stats']):raise ValueError('native cache was used in no-cache benchmark')
            native_stats.extend(r['stats']);native_stages.extend(r['stages'])
            native_input_bytes.extend(r['input_bytes'])
            native.extend(r['seconds']);return r['bytes']
        def python():
            t=time.perf_counter();data=base.texture_dds(pool,analysis);reference.append(time.perf_counter()-t)
            if hashlib.sha256(data).hexdigest()!=expected:raise ValueError('baseline output changed')
        size=0
        for i in range(a.trials):
            if i%2:size=rust();python()
            else:python();size=rust()
        # One separate profiling pass; wrappers do not affect the timed comparison.
        originals={name:getattr(base,name) for name in ('detile','detile_volume','_visible_rows','dds_bytes')}
        def wrap(fn,key):
            def measured(*args,**kwargs):
                started=time.perf_counter();value=fn(*args,**kwargs);reference_stages[key]+=time.perf_counter()-started;return value
            return measured
        try:
            pool.profile=True
            for name,fn in originals.items():setattr(base,name,wrap(fn,'dds_seconds' if name=='dds_bytes' else 'detile_seconds'))
            if hashlib.sha256(base.texture_dds(pool,analysis)).hexdigest()!=expected:raise ValueError('profile changed output')
        finally:
            pool.profile=False
            for name,fn in originals.items():setattr(base,name,fn)
        result=dict(baseline_commit=subprocess.check_output(['git','-C',str(a.leonida),'rev-parse','HEAD'],text=True).strip(),
                    source_path=str(a.store.resolve()),index=a.index,
                    source_size=source_stamp.st_size,source_mtime_ns=source_stamp.st_mtime_ns,
                    benchmark_script_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                    executable_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest(),
                    shape=analysis['descriptor'],dds_bytes=size,equal=True,trials=a.trials,
                    rust_seconds=native,leonida_seconds=reference,speedup=statistics.median(reference)/statistics.median(native),
                    decoded_cache_enabled=not a.no_cache,rust_stats=native_stats,rust_stages=native_stages,leonida_profile=reference_stages,
                    native_input_bytes=native_input_bytes,
                    scope='complete DDS construction including native source freshness checks, read/decode, detile, all mips/faces/slices; warm OS cache; catalog lookup/startup/file writes/hash excluded; baseline no-cache mode uses a fresh decode dictionary per read_range without changing source files')
        # Descriptor/asset identifiers remain in this local report only.
        after=a.store.stat()
        if (source_stamp.st_size,source_stamp.st_mtime_ns)!=(after.st_size,after.st_mtime_ns):raise ValueError('source changed during comparison')
        a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2),encoding='utf-8')
        print(json.dumps({k:v for k,v in result.items() if k!='shape'},indent=2))
    finally:pool.close()


if __name__=='__main__':main()
