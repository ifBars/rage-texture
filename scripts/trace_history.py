"""Trace BCPack BC6H/BC7 initial-history dependencies on real pools.

Read-only research helper. It decodes a bounded prefix of each sampled chunk
(the prefix is enough for every direct built-in reference: those can only
occur in blocks 0..255), then reports which initial-history slots and widths
the stream consumes. It can also check whether a supplied history JSON has
present data for every consumed slot.

With ``--history`` the report deliberately calls that table **provisional**:
present data is not the same as confirmed data, and this tool never labels
slots as confirmed. No game bytes are written to the report; keep ``--out``
outside the repository.

Usage (paths are local; nothing is uploaded):

  python -I scripts/trace_history.py --leonida PATH --store STORE.ttpf \
      --history tables.json --out REPORT.json

The default cap is 32 chunks per codec per store. ``incomplete`` in the
report is true whenever a store had more chunks than the cap.
"""
import argparse
import hashlib
import importlib
import json
import pathlib
import sys
import time

REPO = pathlib.Path(__file__).resolve().parents[1]
CODEC_NAMES = {6: "bc6h", 7: "bc7"}


def die(parser, message):
    parser.error(message)


def require_outside_repo(parser, path):
    path = path.resolve()
    if path.is_relative_to(REPO):
        die(parser, "reports must stay outside the repository")
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def sha256_file(path):
    digest = hashlib.sha256()
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
        raise ValueError(f"{CODEC_NAMES[codec]} history section must be an object")
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
    """Return raw presence tables without promoting anything to confirmed."""
    bc6 = section(document, 6)
    ep_class = rows(bc6.get("endpointClass"), 256, 0)
    ix_class = rows(bc6.get("indexClass"), 256, 0)
    bc7 = section(document, 7)
    ep_bits = [rows(row, 24, 0) for row in rows(bc7.get("historyEndpointBits"), 256, [])]
    ix_bits = [rows(row, 32, 0) for row in rows(bc7.get("historyIndexBits"), 256, [])]
    ep_conf = [
        rows(row, 24, 0)
        for row in rows(bc7.get("historyEndpointConfirmedBits"), 256, [])
    ]
    ix_conf = [
        rows(row, 32, 0)
        for row in rows(bc7.get("historyIndexConfirmedBits"), 256, [])
    ]
    return {
        "bc6h_endpoint_class": ep_class,
        "bc6h_index_class": ix_class,
        "bc7_endpoint_bits": ep_bits,
        "bc7_index_bits": ix_bits,
        "bc7_endpoint_confirmed": ep_conf,
        "bc7_index_confirmed": ix_conf,
    }


def covered(codec, coverage, endpoint_need, index_need):
    """True when every consumed byte/bit has present (not necessarily confirmed) data."""
    if not coverage:
        return False
    if codec == 6:
        classes = coverage["bc6h_endpoint_class"]
        for entry in endpoint_need:
            if classes[entry] == 0:
                return False
        classes = coverage["bc6h_index_class"]
        for entry in index_need:
            if classes[entry] == 0:
                return False
        return True
    bits = coverage["bc7_endpoint_bits"]
    for entry, widths in endpoint_need.items():
        for slot, width in enumerate(widths):
            if width and bits[entry][slot] < width:
                return False
    bits = coverage["bc7_index_bits"]
    for entry, widths in index_need.items():
        for slot, width in enumerate(widths):
            if width and bits[entry][slot] < width:
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

    def add_anchor(masks, entry, index_set, pixel, bits, lanes, first):
        delta = pixel_delta(lanes, first, pixel)
        shift = 4 - bits
        limit = 1 << (bits - 1)
        allowed = 0
        for state in range(16):
            value = ((state >> shift) + delta) & ((1 << bits) - 1)
            if value < limit:
                allowed |= 1 << state
        key = (entry, index_set, pixel)
        masks[key] = masks.get(key, 0xFFFF) & allowed

    def trace(lanes, codec, nblocks):
        l0, l1, l15 = lanes[0], lanes[1], lanes[15]
        endpoint_need = {}
        index_need = {}
        masks = {}
        endpoint_refs = index_refs = 0
        pbit_relations = 0
        for block in range(nblocks):
            first, second = 2 * block, 2 * block + 1
            symbol = l15[first]
            if codec == 6:
                if symbol > 13:
                    continue
                mode = symbol + 1
                _trans, base, deltas, regions = bcpack._BC6H_INFO[bcpack._BC6H_CODE[mode]]
                index_bits = 3 if regions == 2 else 4
                anchors = [0]
                if regions == 2:
                    partition = min(l15[second], 31)
                    anchors.append(int(bptc.BC7_ANCHOR2[partition]))
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
                mode, partition, _rotation, _selection = config[rank]
                mode, partition = int(mode), int(partition)
                subsets = bcpack._B7_NS[mode]
                index_bits = bcpack._B7_IB[mode]
                secondary = mode in (4, 5)
                if secondary:
                    anchors = [0]
                elif subsets == 2:
                    anchors = [0, int(bptc.BC7_ANCHOR2[partition])]
                elif subsets == 3:
                    anchors = [
                        0,
                        int(bptc.BC7_ANCHOR3A[partition]),
                        int(bptc.BC7_ANCHOR3B[partition]),
                    ]
                else:
                    anchors = [0]
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
                if pbit == 2:
                    pbit_relations += 1
            index_distance = bcpack._inter8(l1[first], l1[second])
            endpoint_distance = (
                index_distance
                + bcpack._unzig(bcpack._inter8(l0[first], l0[second]))
            ) & 255
            if endpoint_distance > block:
                endpoint_refs += 1
                entry = 256 + block - endpoint_distance
                row = endpoint_need.setdefault(entry, [0] * len(endpoint_slots))
                if len(row) != len(endpoint_slots):
                    raise ValueError("mixed endpoint slot layout for one history entry")
                for slot, width in enumerate(endpoint_slots):
                    if width:
                        row[slot] = max(row[slot], width)
            if index_distance > block:
                index_refs += 1
                entry = 256 + block - index_distance
                row = index_need.setdefault(entry, [0] * 32)
                for pixel in range(16):
                    row[pixel] = max(row[pixel], index_bits)
                if secondary:
                    for pixel in range(16):
                        row[16 + pixel] = max(row[16 + pixel], 2)
                for pixel in anchors:
                    add_anchor(masks, entry, 0, pixel, index_bits, lanes, first)
                if secondary:
                    add_anchor(masks, entry, 1, 0, 2, lanes, first)
        endpoint_bits = sum(
            sum(width for width in slots if width)
            for slots in endpoint_need.values()
        )
        index_bits = 0
        resolved_bits = 0
        for entry, widths in index_need.items():
            for index_set in range(2):
                for pixel in range(16):
                    width = widths[index_set * 16 + pixel]
                    if not width:
                        continue
                    index_bits += width
                    mask = masks.get((entry, index_set, pixel), 0xFFFF)
                    values = {
                        state >> (4 - width)
                        for state in range(16)
                        if (mask >> state) & 1
                    }
                    if len(values) == 1:
                        resolved_bits += width
        fully_resolved = (
            endpoint_bits == 0
            and index_bits > 0
            and resolved_bits == index_bits
        )
        return {
            "self_contained": endpoint_refs == 0 and index_refs == 0,
            "endpoint_refs": endpoint_refs,
            "index_refs": index_refs,
            "endpoint_entries": len(endpoint_need),
            "index_entries": len(index_need),
            "endpoint_bits": endpoint_bits,
            "index_bits": index_bits,
            "index_anchor_resolved_bits": resolved_bits,
            "endpoint_anchor_resolved_bits": 0,
            "pbit_parity_relations": pbit_relations,
            "fully_resolved": fully_resolved,
            "endpoint_need": endpoint_need,
            "index_need": index_need,
        }

    return trace


def parse_codecs(text):
    out = []
    for token in text.split(","):
        token = token.strip().lower()
        if not token:
            continue
        codec = {"6": 6, "bc6": 6, "bc6h": 6, "7": 7, "bc7": 7}.get(token)
        if codec is None:
            raise ValueError(f"unsupported codec {token!r}; use 6/bc6h or 7/bc7")
        if codec not in out:
            out.append(codec)
    if not out:
        raise ValueError("--codec must name at least one codec")
    return out


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--leonida", type=pathlib.Path, required=True)
    parser.add_argument("--store", type=pathlib.Path, action="append", default=[])
    parser.add_argument("--root", type=pathlib.Path)
    parser.add_argument("--codec", default="6,7")
    parser.add_argument("--chunks-per-codec", type=int, default=32)
    parser.add_argument("--prefix-blocks", type=int, default=256)
    parser.add_argument("--history", type=pathlib.Path)
    parser.add_argument("--history-provenance")
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args(argv)
    if not 1 <= args.chunks_per_codec <= 4096:
        die(parser, "--chunks-per-codec must be 1..4096")
    if not 1 <= args.prefix_blocks <= 256:
        die(parser, "--prefix-blocks must be 1..256; direct references stop at 256")
    stores = list(args.store)
    if args.root:
        stores.extend(sorted(args.root.rglob("*.ttpf")))
    if not stores:
        die(parser, "give at least one --store or --root")
    out = require_outside_repo(parser, args.out)
    codecs = parse_codecs(args.codec)
    bcpack, calib, bptc, ttpfs = load_leonida(args.leonida)
    trace = build_tracer(calib, bcpack, bptc)

    coverage = None
    history_info = None
    if args.history:
        raw_history = args.history.read_bytes()
        document = json.loads(raw_history)
        coverage = history_coverage(document)
        embedded = document.get("_provenance") if isinstance(document, dict) else None
        history_info = {
            "sha256": hashlib.sha256(raw_history).hexdigest(),
            "label": args.history_provenance,
            "embedded_provenance": embedded,
            "source_revision": (
                embedded.get("source_revision") if isinstance(embedded, dict) else None
            ),
            "target_revision": (
                embedded.get("target_revision") if isinstance(embedded, dict) else None
            ),
            "provisional": True,
            "confirmed_slots_claimed": False,
        }

    total = {
        "stores": 0,
        "chunks": 0,
        "packed_bytes": 0,
        "prefix_blocks": 0,
        "self_contained": 0,
        "endpoint_refs": 0,
        "index_refs": 0,
        "endpoint_bits": 0,
        "index_bits": 0,
        "index_anchor_resolved_bits": 0,
        "endpoint_anchor_resolved_bits": 0,
        "pbit_parity_relations": 0,
        "fully_resolved": 0,
        "prefilter_covered_chunks": None if coverage is None else 0,
        "prefilter_uncovered_chunks": None if coverage is None else 0,
        "lane_errors": 0,
        "incomplete_stores": 0,
    }
    by_store = {}
    started = time.perf_counter()
    for path in stores:
        pool = ttpfs.Pool(str(path))
        available = {
            codec: sum(1 for chunk in pool.chunks if chunk[4] == codec)
            for codec in codecs
        }
        picked = {codec: 0 for codec in codecs}
        stats = {
            "available": available,
            "scanned": {codec: 0 for codec in codecs},
            "packed_bytes": 0,
            "prefix_blocks": 0,
            "self_contained": 0,
            "endpoint_refs": 0,
            "index_refs": 0,
            "endpoint_bits": 0,
            "index_bits": 0,
            "index_anchor_resolved_bits": 0,
            "endpoint_anchor_resolved_bits": 0,
            "pbit_parity_relations": 0,
            "fully_resolved": 0,
            "prefilter_covered_chunks": None if coverage is None else 0,
            "prefilter_uncovered_chunks": None if coverage is None else 0,
            "lane_errors": 0,
        }
        try:
            with path.open("rb") as source:
                for virtual, size, start, packed, codec, _pad in pool.chunks:
                    if codec not in picked or picked[codec] >= args.chunks_per_codec:
                        continue
                    picked[codec] += 1
                    stats["scanned"][codec] += 1
                    source.seek(start)
                    raw = source.read(packed)
                    blocks = size // 16
                    want = min(blocks, args.prefix_blocks)
                    try:
                        lanes = calib.decode_lanes_prefix(
                            raw, codec, size, min(2 * want, 2 * blocks)
                        )
                    except Exception:
                        stats["lane_errors"] += 1
                        continue
                    info = trace(lanes, codec, want)
                    stats["packed_bytes"] += packed
                    stats["prefix_blocks"] += want
                    for key in (
                        "self_contained",
                        "endpoint_refs",
                        "index_refs",
                        "endpoint_bits",
                        "index_bits",
                        "index_anchor_resolved_bits",
                        "endpoint_anchor_resolved_bits",
                        "pbit_parity_relations",
                        "fully_resolved",
                    ):
                        if key == "self_contained":
                            stats[key] += int(info[key])
                        else:
                            stats[key] += info[key]
                    if coverage is not None:
                        if covered(
                            codec,
                            coverage,
                            info["endpoint_need"],
                            info["index_need"],
                        ):
                            stats["prefilter_covered_chunks"] += 1
                        else:
                            stats["prefilter_uncovered_chunks"] += 1
        finally:
            pool.close()
        stats["incomplete"] = any(
            picked[codec] < available[codec] for codec in codecs
        )
        by_store[str(path)] = stats
        for key in total:
            if key in ("stores", "incomplete_stores"):
                continue
            if key == "chunks":
                total["chunks"] += sum(stats["scanned"].values())
                continue
            value = stats[key]
            if isinstance(value, dict):
                total[key] += sum(value.values())
            elif value is not None:
                total[key] += value
        total["stores"] += 1
        total["incomplete_stores"] += int(stats["incomplete"])
        print(
            f"{path.name}: chunks={sum(stats['scanned'].values())} "
            f"packed={stats['packed_bytes']} blocks={stats['prefix_blocks']} "
            f"self={stats['self_contained']} ep_bits={stats['endpoint_bits']} "
            f"ix_bits={stats['index_bits']} ix_resolved={stats['index_anchor_resolved_bits']} "
            f"prefilter={stats['prefilter_covered_chunks']} incomplete={stats['incomplete']}",
            flush=True,
        )
    total["elapsed_seconds"] = round(time.perf_counter() - started, 3)
    report = {
        "tool": "trace_history",
        "bounded": {
            "chunks_per_codec": args.chunks_per_codec,
            "prefix_blocks": args.prefix_blocks,
            "incomplete": total["incomplete_stores"] > 0,
        },
        "total": total,
        "by_store": by_store,
        "history": history_info,
        "policy": {
            "confirmed_recovery": False,
            "note": (
                "Present history data is reported as provisional. The prefilter only "
                "checks that every consumed slot has present data; only a decode "
                "and byte comparison can establish success. Anchor and p-bit "
                "constraints only narrow index bits; they cannot confirm "
                "endpoint values or full history entries."
            ),
        },
    }
    out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print("TOTAL", json.dumps(total), flush=True)
    print("wrote", out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
