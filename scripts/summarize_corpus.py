"""Reconcile a complete native corpus audit with the independent SixNG run.

This checks coverage, ordered decoded-byte digests and zero-cache evidence.
Fitted output remains fitted; a compatibility pass is not original-pixel proof.
"""
import argparse
import collections
import json
import ntpath
import pathlib


def source_key(path):
    value = str(path).replace("\\", "/").rstrip("/")
    # Preserve distinct case-sensitive POSIX paths in reports from Linux.
    return value.casefold() if ntpath.splitdrive(value)[0] else value


def read_native(path):
    header = final = active = None
    stores = {}
    codecs = collections.Counter()
    formats = collections.Counter()
    layouts = collections.Counter()
    unfitted_by_codec = collections.Counter()
    unfitted = invalid = 0
    with pathlib.Path(path).open(encoding="utf-8") as stream:
        for number, line in enumerate(stream, 1):
            try:
                row = json.loads(line)
            except ValueError as exc:
                raise ValueError(f"incomplete/invalid JSONL at line {number}") from exc
            kind = row.get("kind")
            if final is not None:
                raise ValueError("events after run_done")
            if kind == "run":
                if header is not None or row.get("phase") != "both" or row.get("cache_budget") != 0:
                    raise ValueError("expected one complete both-phase, zero-cache run")
                header = row
            elif kind == "store_start":
                if header is None or active is not None:
                    raise ValueError("unexpected store_start")
                key = source_key(row["path"])
                if key in stores:
                    raise ValueError("duplicate store")
                active = dict(start=row, key=key, chunks=0, textures=0,
                              unfitted=0, invalid=0, codecs=collections.Counter())
            elif kind in ("chunk", "texture"):
                if active is None or row["store"] != active["start"]["store"]:
                    raise ValueError("record outside its store")
                counter = "chunks" if kind == "chunk" else "textures"
                index = row["chunk"] if kind == "chunk" else row["texture"]
                if index != active[counter]:
                    raise ValueError(f"missing, duplicate or unordered {kind} index")
                active[counter] += 1
                if row.get("status") != "pass":
                    raise ValueError(f"{kind} failure at line {number}")
                if kind == "chunk":
                    codecs[str(row["codec"])] += 1
                    active["codecs"][str(row["codec"])] += 1
                    quality = row.get("quality", {})
                    active["unfitted"] += quality.get("unfitted_blocks", 0)
                    unfitted_by_codec[str(row["codec"])] += quality.get("unfitted_blocks", 0)
                    active["invalid"] += quality.get("invalid_blocks", 0)
                else:
                    formats[str(row["dxgi"])] += 1
                    layouts[row["layout"]] += 1
                    if row.get("consumer") != "pass":
                        raise ValueError("DDS consumer failure")
            elif kind == "store_done":
                if active is None or row["store"] != active["start"]["store"]:
                    raise ValueError("unexpected store_done")
                for counter in ("chunks", "textures"):
                    expected = active["start"][counter]
                    counts = row[counter]
                    if active[counter] != expected or any(counts[k] != expected for k in ("expected", "attempted", "pass")) or counts["fail"]:
                        raise ValueError(f"incomplete {counter} coverage")
                stats = row["stats"]
                if (row.get("status") != "pass" or row.get("source_changed") or
                    not row.get("cache_policy_ok") or row.get("consumer_failures") or
                    any(stats[k] for k in ("cache_hits", "cache_bytes", "peak_cache_bytes"))):
                    raise ValueError("store/source/cache validation failed")
                aggregate = row["chunk_aggregate"]
                dds = row["dds_aggregate"]
                if (aggregate["chunks"] != active["chunks"] or aggregate["failed_indices"] or
                    dds["textures"] != active["textures"] or dds["failed_textures"] or
                    dds["consumer_failed_textures"]):
                    raise ValueError("incomplete aggregate")
                row["observed_unfitted_blocks"] = active["unfitted"]
                row["observed_invalid_blocks"] = active["invalid"]
                row["observed_codecs"] = dict(active["codecs"])
                stores[active["key"]] = row
                unfitted += active["unfitted"]
                invalid += active["invalid"]
                active = None
            elif kind == "store_failed":
                raise ValueError("store failed")
            elif kind == "run_done":
                final = row
    if header is None or final is None or active is not None:
        raise ValueError("audit is incomplete: missing run/store completion")
    if not stores or len(stores) != header["stores_total"]:
        raise ValueError("missing stores")
    return dict(header=header, final=final, stores=stores, codecs=dict(codecs),
                formats=dict(formats), layouts=dict(layouts), unfitted_blocks=unfitted,
                unfitted_by_codec=dict(unfitted_by_codec), invalid_blocks=invalid)


def compare(native, reference):
    errors = []
    baseline = {}
    for row in reference["stores"]:
        key = source_key(row["path"])
        if key in baseline:
            raise ValueError("duplicate reference store")
        baseline[key] = row
    if set(baseline) != set(native["stores"]):
        errors.append("store sets differ")
    if (reference["stores_total"] != len(baseline) or reference["stores_done"] != len(baseline) or
        reference.get("stores_skipped", 0) or reference["stores_failed"] or
        reference["cache_budget_bytes"] != 0):
        errors.append("reference coverage/cache evidence failed")
    equal = 0
    for key, row in native["stores"].items():
        other = baseline.get(key)
        if other is None:
            continue
        aggregate = row["chunk_aggregate"]
        checks = (
            other["status"] == "pass", not other["source_changed"],
            other["chunks_expected"] == other["chunks_attempted"] == other["chunks_pass"] == row["chunks"]["expected"],
            other["chunks_fail"] == 0, not other["failure_indices"],
            other["aggregate_sha256"] == aggregate["sha256"],
            other["successful_bytes"] == aggregate["bytes"],
            other["unfitted_blocks"] == row["observed_unfitted_blocks"],
            other["invalid_blocks"] == row["observed_invalid_blocks"],
            {str(int(flag) >> 2): count for flag, count in other["codec_counts"].items()} == row["observed_codecs"],
        )
        if all(checks):
            equal += 1
        else:
            errors.append(f"reference disagreement: {key}")
    return dict(
        compatibility_passed=not errors,
        scope="Complete chunk-byte parity and native DDS container coverage; fitted history is not original-pixel proof.",
        stores=len(native["stores"]), matching_store_digests=equal,
        chunks=sum(row["chunks"]["pass"] for row in native["stores"].values()),
        decoded_chunk_bytes=sum(row["chunk_aggregate"]["bytes"] for row in native["stores"].values()),
        textures=sum(row["textures"]["pass"] for row in native["stores"].values()),
        dds_bytes=sum(row["dds_aggregate"]["bytes"] for row in native["stores"].values()),
        codecs=native["codecs"], formats=native["formats"], layouts=native["layouts"],
        unfitted_blocks=native["unfitted_blocks"], invalid_blocks=native["invalid_blocks"],
        unfitted_blocks_by_codec=native["unfitted_by_codec"],
        decoded_cache_enabled=False, errors=errors,
        native_audit_source_sha256=native["header"].get("audit_source_sha256"),
        sixng_revision=reference.get("sixng_rev"),
        sixng_reference_exe_sha256=reference.get("exe_sha256"),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=pathlib.Path, required=True)
    parser.add_argument("--sixng", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        parser.error("raw reports must remain outside the repository")
    try:
        report = compare(read_native(args.native), json.loads(args.sixng.read_text(encoding="utf-8")))
    except (ValueError, KeyError) as exc:
        parser.exit(1, f"corpus reconciliation failed: {exc}\n")
    with args.out.open("x", encoding="utf-8", newline="\n") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps(report))
    return not report["compatibility_passed"]


if __name__ == "__main__":
    raise SystemExit(main())
