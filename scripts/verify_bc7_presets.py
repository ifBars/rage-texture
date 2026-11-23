"""Reconcile every affected BC7 chunk against a separately pinned reference.

The manifest and reports contain local paths/hashes and must stay outside Git.
This verifies fitted compatibility, not the original initial-history values.
"""
import argparse
import hashlib
import json
import pathlib
import time

import rage_texture._native as native
from rage_texture import Store, UnsupportedError


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stamp(path):
    stat = path.stat()
    return (stat.st_size, stat.st_mtime_ns)


def verify(manifest, reference, before):
    expected = {}
    for group in manifest:
        for chunk in group["chunks"]:
            key = (group["path"], chunk["index"])
            if key in expected or chunk["len"] <= 0:
                raise ValueError("duplicate chunk or empty range")
            expected[key] = chunk
    if not expected:
        raise ValueError("empty manifest")
    refs = {}
    for row in reference:
        key = (row["path"], row["index"])
        if key in refs or key not in expected or row["skipped"] != 0:
            raise ValueError("duplicate/unexpected reference or skipped blocks")
        if row["bytes"] != expected[key]["len"]:
            raise ValueError("reference range length differs")
        refs[key] = row
    if refs.keys() != expected.keys():
        raise ValueError("incomplete reference")
    records = []
    sources = []
    totals = dict(chunks=0, bytes=0, unfitted_blocks=0, invalid_blocks=0,
                  changed_chunks=0, confirmed_refusals=0, read_seconds=0.0)
    for group in manifest:
        path = pathlib.Path(group["path"])
        snapshot = stamp(path)
        with Store(path, cache_bytes=0, allow_fitted=True) as store:
            with Store(path, cache_bytes=0, decode_policy="confirmed") as strict:
                for chunk in group["chunks"]:
                    started = time.perf_counter()
                    result = store.read_range_result(chunk["start"], chunk["len"])
                    data = result.data
                    seconds = time.perf_counter() - started
                    sha = hashlib.sha256(data).hexdigest()
                    ref = refs[(group["path"], chunk["index"])]
                    wanted = chunk["old_sha256"] if before else ref["sha256"]
                    if len(data) != chunk["len"] or sha != wanted:
                        raise ValueError(f"chunk output differs: {path.name}:{chunk['index']}")
                    quality = result.quality
                    if not quality or any(q.codec != 7 or q.is_lossless or q.is_confirmed
                                          or q.invalid_blocks != 0 for q in quality):
                        raise ValueError("BC7 fitted quality was misclassified")
                    unfitted = sum(q.unfitted_blocks for q in quality)
                    if not before and unfitted:
                        raise ValueError("current BC7 table still has unfitted blocks")
                    try:
                        strict.read_range_result(chunk["start"], chunk["len"])
                    except UnsupportedError as exc:
                        if not any(term in str(exc).lower() for term in
                                   ("history", "inferred-only (unconfirmed)")):
                            raise
                        totals["confirmed_refusals"] += 1
                    else:
                        raise ValueError("fitted presets were accepted as confirmed history")
                    totals["chunks"] += 1
                    totals["bytes"] += len(data)
                    totals["unfitted_blocks"] += unfitted
                    totals["changed_chunks"] += sha != chunk["old_sha256"]
                    totals["read_seconds"] += seconds
                    records.append(dict(path=str(path), index=chunk["index"],
                                        sha256=sha, bytes=len(data), unfitted_blocks=unfitted))
                for candidate in (store, strict):
                    stats = candidate.stats()
                    if stats.cache_hits or stats.cache_bytes or stats.peak_cache_bytes:
                        raise ValueError("decoded cache was used")
        if stamp(path) != snapshot:
            raise ValueError("source changed during verification")
        sources.append(dict(path=str(path), size=snapshot[0], mtime_ns=snapshot[1]))
        print(f"{len(sources)}/{len(manifest)} stores; {totals['chunks']} chunks", flush=True)
    return dict(passed=True, stage="before" if before else "current", totals=totals,
                cache_bytes=0, sources=sources, chunks=records,
                native_sha256=digest(pathlib.Path(native.__file__)),
                script_sha256=digest(pathlib.Path(__file__)),
                limits="Fitted compatibility only; strict built-in history refuses every chunk. "
                       "Read timings are single-pass operational data, not a speed benchmark.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--reference", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--before", action="store_true")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    if args.out.resolve().is_relative_to(root):
        parser.error("reports must stay outside the repository")
    manifest = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
    reference = [json.loads(line) for line in args.reference.read_text(encoding="utf-8-sig").splitlines()]
    report = verify(manifest, reference, args.before)
    report.update(manifest_sha256=digest(args.manifest), reference_sha256=digest(args.reference))
    args.out.write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(json.dumps(report["totals"]))


if __name__ == "__main__":
    main()
