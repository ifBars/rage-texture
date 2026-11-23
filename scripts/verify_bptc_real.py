"""Bounded real-chunk comparison for external provisional BC6H/BC7 history.

Compares the native Rust history decoder with an unmodified Leonida checkout on
real BCPack chunks that are fully covered by a supplied preliminary history
JSON. A table is treated as **provisional** even when every consumed slot has
present data: present values are not confirmed values. The report records the
history file hash and a caller-supplied provenance string, and never promotes
slots to confirmed.

The run is explicitly bounded: at most N chunks per codec per store (default
32), a short prefix trace per chunk, and a decoded-size cap. Reports and any
scratch data stay outside the repository. No game bytes are printed.

Example (local paths only):

  python -I scripts/verify_bptc_real.py --leonida PATH --exe target/debug/rage-texture.exe \
      --store STORE.ttpf --history provisional.json --history-provenance sixng-fitted \
      --out REPORT.json
"""
import argparse
import hashlib
import importlib
import json
import pathlib
import subprocess
import sys
import time

REPO = pathlib.Path(__file__).resolve().parents[1]


def die(parser, message):
    parser.error(message)


def require_outside_repo(parser, path):
    path = path.resolve()
    if path.is_relative_to(REPO):
        die(parser, "reports must stay outside the repository")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_path(path):
    digest = hashlib.sha256()
    if path.is_dir():
        for item in sorted(path.rglob("*.json")):
            digest.update(item.name.encode("utf-8"))
            digest.update(b"\0")
            digest.update(item.read_bytes())
            digest.update(b"\0")
    else:
        with path.open("rb") as source:
            for block in iter(lambda: source.read(1 << 20), b""):
                digest.update(block)
    return digest.hexdigest()


def load_leonida(path):
    tools = path.resolve() / "tools"
    if not (tools / "bcpack.py").is_file():
        raise SystemExit(f"not a Leonida checkout: {path}")
    sys.path.insert(0, str(tools))
    return (
        importlib.import_module("bcpack"),
        importlib.import_module("bcpack_history_calib"),
        importlib.import_module("bcpack_tables_bptc"),
        importlib.import_module("ttpf_mips"),
    )


def section(document, codec):
    marker = "endpointHistory" if codec == 6 else "historyEndpoints"
    name = "bc6h" if codec == 6 else "bc7"
    if not isinstance(document, dict):
        raise ValueError("history document must be a JSON object")
    if name in document:
        value = document[name]
    elif marker in document:
        value = document
    else:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{codec} history section must be an object")
    return value


def rows(value, count, default=0):
    out = [default] * count
    if isinstance(value, list):
        for index, item in enumerate(value[:count]):
            out[index] = item
    elif isinstance(value, dict):
        for key, item in value.items():
            index = int(key)
            if 0 <= index < count:
                out[index] = item
    elif value is not None:
        raise ValueError("history rows must be a list or an index-keyed object")
    return out


def history_coverage(document):
    bc6 = section(document, 6)
    bc7 = section(document, 7)
    return {
        "bc6h_endpoint_class": rows(bc6.get("endpointClass"), 256, 0),
        "bc6h_index_class": rows(bc6.get("indexClass"), 256, 0),
        "bc7_endpoint_bits": [
            rows(row, 24, 0)
            for row in rows(bc7.get("historyEndpointBits"), 256, [])
        ],
        "bc7_index_bits": [
            rows(row, 32, 0)
            for row in rows(bc7.get("historyIndexBits"), 256, [])
        ],
    }


def covered(codec, coverage, endpoint_need, index_need):
    if codec == 6:
        endpoint_class = coverage["bc6h_endpoint_class"]
        index_class = coverage["bc6h_index_class"]
        return all(endpoint_class[entry] for entry in endpoint_need) and all(
            index_class[entry] for entry in index_need
        )
    endpoint_bits = coverage["bc7_endpoint_bits"]
    index_bits = coverage["bc7_index_bits"]
    for entry, widths in endpoint_need.items():
        for slot, width in enumerate(widths):
            if width and endpoint_bits[entry][slot] < width:
                return False
    for entry, widths in index_need.items():
        for slot, width in enumerate(widths):
            if width and index_bits[entry][slot] < width:
                return False
    return True


def build_tracer(calib, bcpack, bptc):
    deint0, deint1 = bcpack._DEINT
    static = bptc._bc7_static()
    rank_of = static["rank_of"]
    config = static["config"]

    def pixel_delta(lanes, first, pixel):
        row, column = divmod(pixel, 4)
        symbol = lanes[11 + row][first] if column < 2 else lanes[11 + row][first + 1]
        if row < 3:
            value = deint1[symbol] if column % 2 == 0 else deint0[symbol]
        else:
            value = deint0[symbol] if column % 2 == 0 else deint1[symbol]
        return bcpack._unzig(value)

    def trace(lanes, codec, nblocks):
        l0, l1, l15 = lanes[0], lanes[1], lanes[15]
        endpoint_need = {}
        index_need = {}
        for block in range(nblocks):
            first, second = 2 * block, 2 * block + 1
            symbol = l15[first]
            if codec == 6:
                if symbol > 13:
                    continue
                mode = symbol + 1
                _trans, base, deltas, regions = bcpack._BC6H_INFO[bcpack._BC6H_CODE[mode]]
                index_bits = 3 if regions == 2 else 4
                secondary = False
                endpoint_slots = [0] * 12
                for channel in range(3):
                    if regions == 2:
                        widths = (base, deltas[channel], deltas[channel], deltas[channel])
                    else:
                        widths = (base, deltas[channel], 0, 0)
                    for field, width in enumerate(widths):
                        endpoint_slots[channel * 4 + field] = width
            else:
                rank = (
                    rank_of[l15[first] * 16 + l15[second]]
                    if l15[first] < 32 and l15[second] < 16
                    else -1
                )
                if rank < 0 or config[rank][0] > 7:
                    continue
                mode = int(config[rank][0])
                subsets = bcpack._B7_NS[mode]
                index_bits = bcpack._B7_IB[mode]
                secondary = mode in (4, 5)
                colour = bcpack._B7_CB[mode]
                alpha = bcpack._B7_AB[mode]
                pbit = bcpack._B7_PK[mode]
                endpoint_slots = [0] * 24
                for subset in range(subsets):
                    for endpoint in range(2):
                        for channel, width in enumerate((colour, colour, colour, alpha)):
                            if not width:
                                continue
                            bits = width + (1 if pbit and channel == 0 else 0)
                            slot = subset * 8 + endpoint * 4 + channel
                            endpoint_slots[slot] = max(endpoint_slots[slot], bits)
            index_distance = bcpack._inter8(l1[first], l1[second])
            endpoint_distance = (
                index_distance
                + bcpack._unzig(bcpack._inter8(l0[first], l0[second]))
            ) & 255
            if endpoint_distance > block:
                entry = 256 + block - endpoint_distance
                row = endpoint_need.setdefault(entry, [0] * len(endpoint_slots))
                if len(row) != len(endpoint_slots):
                    raise ValueError("mixed endpoint slot layout for one history entry")
                for slot, width in enumerate(endpoint_slots):
                    if width:
                        row[slot] = max(row[slot], width)
            if index_distance > block:
                entry = 256 + block - index_distance
                row = index_need.setdefault(entry, [0] * 32)
                for pixel in range(16):
                    row[pixel] = max(row[pixel], index_bits)
                if secondary:
                    for pixel in range(16):
                        row[16 + pixel] = max(row[16 + pixel], 2)
        return endpoint_need, index_need

    return trace


def native_hash(exe, store, virtual, size, history):
    command = [
        str(exe),
        "range-hash",
        str(store),
        str(virtual),
        str(size),
        "--allow-provisional",
        "--history",
        str(history),
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        return None, result.stderr.strip()[:400]
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None, "native output is not JSON"
    quality = payload.get("quality") or []
    dependencies = 0
    found_history = False
    for entry in quality:
        history_quality = entry.get("HistoryBc") if isinstance(entry, dict) else None
        if isinstance(history_quality, dict):
            found_history = True
            dependencies += int(history_quality.get("provisional_dependencies") or 0)
    stats = payload.get("stats") or {}
    cache = any(
        int(stats.get(key) or 0)
        for key in ("cache_hits", "cache_bytes", "peak_cache_bytes", "evictions")
    )
    return {
        "sha256": payload.get("sha256"),
        "bytes": payload.get("bytes"),
        "provisional_dependencies": dependencies,
        "history_quality": found_history,
        "cache_used": cache,
        "stats": stats,
    }, None


def git_revision(path):
    try:
        return subprocess.check_output(
            ["git", "-C", str(path), "rev-parse", "HEAD"], text=True
        ).strip()
    except Exception:
        return None


def parse_codecs(text):
    out = []
    for token in text.split(","):
        token = token.strip().lower()
        if not token:
            continue
        codec = {"6": 6, "bc6": 6, "bc6h": 6, "7": 7, "bc7": 7}.get(token)
        if codec is None:
            raise SystemExit(f"unsupported codec {token!r}")
        if codec not in out:
            out.append(codec)
    if not out:
        raise SystemExit("--codec must name at least one codec")
    return out


def new_counts():
    return {
        "scanned": 0,
        "prefilter_candidates": 0,
        "skipped_prefilter": 0,
        "skipped_large": 0,
        "both_refused": 0,
        "python_refused": 0,
        "native_refused": 0,
        "acceptance_mismatch": 0,
        "equal": 0,
        "mismatch": 0,
        "label_presence_mismatch": 0,
        "cache_used": 0,
        "errors": 0,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--leonida", type=pathlib.Path, required=True)
    parser.add_argument("--exe", type=pathlib.Path, required=True)
    parser.add_argument("--store", type=pathlib.Path, action="append", default=[])
    parser.add_argument("--root", type=pathlib.Path)
    parser.add_argument("--history", type=pathlib.Path, required=True)
    parser.add_argument("--history-provenance")
    parser.add_argument("--codec", default="6,7")
    parser.add_argument("--chunks-per-codec", type=int, default=32)
    parser.add_argument("--prefix-blocks", type=int, default=256)
    parser.add_argument("--max-decoded-bytes", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--inventory-only", action="store_true", help="exit 0 even when no byte-equal success is observed")
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args(argv)
    if not 1 <= args.chunks_per_codec <= 256:
        die(parser, "--chunks-per-codec must be 1..256")
    if not 1 <= args.prefix_blocks <= 256:
        die(parser, "--prefix-blocks must be 1..256")
    if args.max_decoded_bytes < 16:
        die(parser, "--max-decoded-bytes must be at least 16")
    stores = list(args.store)
    if args.root:
        stores.extend(sorted(args.root.rglob("*.ttpf")))
    if not stores:
        die(parser, "give at least one --store or --root")
    if not args.exe.is_file():
        die(parser, "--exe does not exist")
    if not args.history.exists():
        die(parser, "--history does not exist")
    out = require_outside_repo(parser, args.out)
    codecs = parse_codecs(args.codec)
    bcpack, calib, bptc, ttpfs = load_leonida(args.leonida)
    trace = build_tracer(calib, bcpack, bptc)
    document = json.loads(args.history.read_bytes())
    coverage = history_coverage(document)
    history_sha = sha256_path(args.history)
    embedded = document.get("_provenance") if isinstance(document, dict) else None

    report = {
        "tool": "verify_bptc_real",
        "scope": "real external preliminary history; not a confirmed oracle",
        "provenance": {
            "history_sha256": history_sha,
            "history_label": args.history_provenance,
            "history_embedded_provenance": embedded,
            "history_source_revision": (
                embedded.get("source_revision") if isinstance(embedded, dict) else None
            ),
            "history_target_revision": (
                embedded.get("target_revision") if isinstance(embedded, dict) else None
            ),
            "history_is_provisional": True,
            "confirmed_slots_claimed": False,
            "baseline_leonida_revision": git_revision(args.leonida),
            "native_sha256": sha256_path(args.exe),
            "stores": [str(path) for path in stores],
        },
        "bounded": {
            "chunks_per_codec": args.chunks_per_codec,
            "prefix_blocks": args.prefix_blocks,
            "max_decoded_bytes": args.max_decoded_bytes,
            "incomplete": False,
        },
        "by_store": {},
        "total": new_counts(),
        "errors": [],
        "coverage_observed": False,
    }
    started = time.perf_counter()
    for path in stores:
        pool = ttpfs.Pool(str(path))
        available = {
            codec: sum(1 for chunk in pool.chunks if chunk[4] == codec)
            for codec in codecs
        }
        picked = {codec: 0 for codec in codecs}
        store_report = {
            "available": available,
            "scanned": {codec: 0 for codec in codecs},
            "by_codec": {codec: new_counts() for codec in codecs},
            "incomplete": False,
        }
        try:
            with path.open("rb") as source:
                for virtual, size, start, packed, codec, _pad in pool.chunks:
                    if codec not in picked or picked[codec] >= args.chunks_per_codec:
                        continue
                    picked[codec] += 1
                    counts = store_report["by_codec"][codec]
                    counts["scanned"] += 1
                    store_report["scanned"][codec] += 1
                    source.seek(start)
                    raw = source.read(packed)
                    if size > args.max_decoded_bytes:
                        counts["skipped_large"] += 1
                        continue
                    blocks = size // 16
                    prefix = min(blocks, args.prefix_blocks)
                    try:
                        lanes = calib.decode_lanes_prefix(
                            raw, codec, size, min(2 * prefix, 2 * blocks)
                        )
                        endpoint_need, index_need = trace(lanes, codec, prefix)
                    except Exception as exc:
                        counts["errors"] += 1
                        report["errors"].append(
                            {
                                "store": path.name,
                                "codec": codec,
                                "virtual": virtual,
                                "stage": "trace",
                                "error": f"{type(exc).__name__}: {exc}"[:300],
                            }
                        )
                        continue
                    if not covered(codec, coverage, endpoint_need, index_need):
                        counts["skipped_prefilter"] += 1
                        continue
                    counts["prefilter_candidates"] += 1
                    try:
                        expected, labels = bcpack.decode_with_info(
                            raw,
                            codec,
                            size,
                            allow_provisional=True,
                            history=str(args.history),
                        )
                        python_error = None
                    except Exception as exc:
                        expected = None
                        labels = set()
                        python_error = f"{type(exc).__name__}: {exc}"[:300]
                    actual, native_error = native_hash(
                        args.exe, path, virtual, size, args.history
                    )
                    if expected is None:
                        counts["python_refused"] += 1
                        if actual is None:
                            counts["both_refused"] += 1
                        else:
                            counts["acceptance_mismatch"] += 1
                            report["errors"].append(
                                {
                                    "store": path.name,
                                    "codec": codec,
                                    "virtual": virtual,
                                    "stage": "acceptance",
                                    "python": python_error,
                                    "native": "accepted",
                                }
                            )
                        continue
                    if actual is None:
                        counts["native_refused"] += 1
                        counts["acceptance_mismatch"] += 1
                        report["errors"].append(
                            {
                                "store": path.name,
                                "codec": codec,
                                "virtual": virtual,
                                "stage": "acceptance",
                                "python": "accepted",
                                "native": native_error,
                            }
                        )
                        continue
                    if actual["cache_used"]:
                        counts["cache_used"] += 1
                        report["errors"].append(
                            {
                                "store": path.name,
                                "codec": codec,
                                "virtual": virtual,
                                "stage": "cache",
                                "error": "native range-hash reported decoded-cache use",
                            }
                        )
                    expected_sha = sha256_bytes(expected)
                    if (
                        actual["sha256"] != expected_sha
                        or actual["bytes"] != len(expected)
                    ):
                        counts["mismatch"] += 1
                        report["errors"].append(
                            {
                                "store": path.name,
                                "codec": codec,
                                "virtual": virtual,
                                "stage": "bytes",
                                "python_sha256": expected_sha,
                                "native_sha256": actual["sha256"],
                                "python_bytes": len(expected),
                                "native_bytes": actual["bytes"],
                            }
                        )
                        continue
                    if bool(labels) != (actual["provisional_dependencies"] > 0):
                        counts["label_presence_mismatch"] += 1
                        report["errors"].append(
                            {
                                "store": path.name,
                                "codec": codec,
                                "virtual": virtual,
                                "stage": "labels",
                                "python_labels": len(labels),
                                "native_dependencies": actual[
                                    "provisional_dependencies"
                                ],
                            }
                        )
                        continue
                    counts["equal"] += 1
        finally:
            pool.close()
        store_report["incomplete"] = any(
            picked[codec] < available[codec] for codec in codecs
        )
        report["by_store"][str(path.name)] = store_report
        report["bounded"]["incomplete"] |= store_report["incomplete"]
        for codec, counts in store_report["by_codec"].items():
            for key, value in counts.items():
                report["total"][key] += value
        print(
            f"{path.name}: scanned={sum(store_report['scanned'].values())} "
            f"equal={sum(c['equal'] for c in store_report['by_codec'].values())} "
            f"refused={sum(c['both_refused'] for c in store_report['by_codec'].values())} "
            f"mismatch={sum(c['mismatch'] for c in store_report['by_codec'].values())} "
            f"incomplete={store_report['incomplete']}",
            flush=True,
        )
    report["elapsed_seconds"] = round(time.perf_counter() - started, 3)
    report["coverage_observed"] = report["total"]["equal"] > 0
    report["base_checks_ok"] = (
        report["total"]["mismatch"] == 0
        and report["total"]["label_presence_mismatch"] == 0
        and report["total"]["acceptance_mismatch"] == 0
        and report["total"]["cache_used"] == 0
        and report["total"]["errors"] == 0
    )
    report["success"] = report["base_checks_ok"] and report["coverage_observed"]
    report["ok"] = report["base_checks_ok"] and (
        args.inventory_only or report["coverage_observed"]
    )
    report["status"] = (
        "success"
        if report["success"]
        else ("inventory-only" if report["ok"] else "failed-no-success")
    )
    out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print("TOTAL", json.dumps(report["total"]), flush=True)
    print(
        f"provisional={report['provenance']['history_is_provisional']} "
        f"coverage_observed={report['coverage_observed']} "
        f"status={report['status']} incomplete={report['bounded']['incomplete']}",
        flush=True,
    )
    print("wrote", out)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
