"""Read bounded evidence from configured local game roots."""
import gzip
import hashlib
import json
import mmap
import os
from pathlib import Path
import re
import sys
from typing import NamedTuple
import zlib

MIB = 1024 * 1024
MAX_DECODED = 8 * MIB
MAX_COMPRESSED = 8 * MIB
MAX_WINDOW = 64 * MIB
HEADERS = re.compile(rb"\x78[\x01\x5e\x9c\xda]")
WORD = frozenset(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
TEXTURE_HASH = re.compile(r"[0-9a-fA-F]{16}")
MAX_INDEX_BYTES = 2 * 1024 * MIB
MAX_INDEX_LINE = 128 * 1024


class Source(NamedTuple):
    game: str
    root: Path
    texture_users: Path | None


def sources(config):
    result = {}
    entries = json.loads(config.read_text(encoding="utf-8-sig"))["sources"]
    if not isinstance(entries, list) or not 1 <= len(entries) <= 16:
        raise ValueError("Configure 1 to 16 sources.")
    for entry in entries:
        key = entry["id"]
        if not re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", key) or key in result:
            raise ValueError("Source IDs must be unique ASCII identifiers.")
        game = entry["game"]
        if not isinstance(game, str) or not 1 <= len(game.strip()) <= 120:
            raise ValueError("Each source needs a game or build label of up to 120 characters.")
        root = Path(entry["root"])
        if not root.is_absolute():
            root = config.parent / root
        root = root.resolve(strict=True)
        if not root.is_dir():
            raise ValueError("Each source root must be a directory.")
        texture_users = entry.get("textureUsersIndex")
        if texture_users is not None:
            texture_users = (config.parent / texture_users).absolute()
        result[key] = Source(game, root, texture_users)
    return result


def confined_file(root, relative):
    part = Path(relative)
    if part.anchor or ".." in part.parts or ":" in relative:
        raise ValueError("Use a path relative to the configured source.")
    entry = (root / part).resolve(strict=True)
    if not entry.is_relative_to(root) or not entry.is_file():
        raise ValueError("File lies outside the configured source or is not a regular file.")
    return entry


def source_directories(root, onerror):
    for directory, dirs, files in os.walk(root, followlinks=False, onerror=onerror):
        base = Path(directory)
        count = len(dirs)
        # A resolved root lets us skip symlinks and Windows junctions, including cycles.
        dirs[:] = [name for name in dirs if (base / name).resolve() == base / name]
        yield base, files, count - len(dirs)


def file_matches(root, query, after, limit):
    import bisect

    matches = []
    files_seen = 0
    errors = []
    skipped_directories = 0
    for base, files, skipped in source_directories(root, lambda e: errors.append(e.errno)):
        skipped_directories += skipped
        for file in files:
            files_seen += 1
            if query.casefold() not in file.casefold():
                continue
            p = base / file
            relative = p.relative_to(root).as_posix()
            if relative > after and p.resolve().is_relative_to(root) and p.is_file():
                bisect.insort(matches, relative)
                if len(matches) > limit + 1:
                    matches.pop()
    return {"files": matches[:limit], "filesSeen": files_seen,
            "walkErrors": len(errors), "skippedDirectories": skipped_directories,
            "nextAfter": matches[limit - 1] if len(matches) > limit else None}


def patterns(terms):
    for term in dict.fromkeys(terms):
        if not (2 <= len(term) <= 64 and term.isascii() and term.isprintable()
                and term == term.strip()):
            raise ValueError("Use 2–64 printable ASCII characters per term, with no surrounding spaces.")
        for encoding in ("ascii", "utf-16le"):
            yield term, encoding, term.lower().encode(encoding)


def context(data, start, end, encoding):
    step = 2 if encoding == "utf-16le" else 1

    def printable(i):
        return (0 <= i and i + step <= len(data) and 32 <= data[i] < 127
                and (step == 1 or data[i + 1] == 0))

    lo, hi = start, end
    while start - lo < 48 * step and printable(lo - step):
        lo -= step
    while hi - end < 48 * step and printable(hi):
        hi += step
    return data[lo:hi].decode(encoding, errors="replace")


def matches(data, compiled, start, end, limit):
    # Merge lazily so a result cap does not allocate all hits in a dense file.
    import heapq

    folded = data.lower() if start == 0 and end == len(data) and isinstance(data, bytes) else data[start:end].lower()

    def hits(term, encoding, needle):
        step = 2 if encoding == "utf-16le" else 1

        def word_at(pos):
            return (0 <= pos and pos + step <= len(data) and data[pos] in WORD
                    and (step == 1 or data[pos + 1] == 0))

        cursor = 0
        while (found := folded.find(needle, cursor)) != -1:
            lo, hi = start + found, start + found + len(needle)
            if not word_at(lo - step) and not word_at(hi):
                yield lo, -hi, term, encoding
            cursor = found + 1

    ordered = heapq.merge(*(hits(*item) for item in compiled))
    results = []
    next_offset = None
    previous = -1
    for lo, negative_end, term, encoding in ordered:
        if lo == previous:
            continue
        previous = lo
        if len(results) == limit:
            next_offset = lo
            break
        results.append({"term": term, "encoding": encoding, "byteOffset": lo,
                        "context": context(data, lo, -negative_end, encoding)})
    return results, next_offset


def data_start(data):
    if len(data) < 64 or data[:4] not in (b"9FPR", b"RPF9"):
        raise ValueError("Expected an RPF9 archive.")
    start = int.from_bytes(data[4:8], "little")
    if not 64 <= start <= len(data):
        raise ValueError("Invalid RPF9 data-area offset.")
    return start


def inflate(data, offset, budget=MAX_DECODED):
    decoder = zlib.decompressobj()
    content = bytearray()
    cursor = offset
    end = min(len(data), offset + MAX_COMPRESSED)
    try:
        while cursor < end:
            packet = data[cursor:min(end, cursor + 65536)]
            remaining = budget - len(content)
            content.extend(decoder.decompress(packet, remaining + 1))
            if len(content) > budget:
                return None, 0, "decoded-limit"
            cursor += len(packet)
            if decoder.eof:
                return bytes(content), cursor - offset - len(decoder.unused_data), None
    except zlib.error:
        return None, 0, "invalid-zlib"
    return None, 0, "input-limit-or-truncated"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def archive_scan(data, compiled, start, stop, limit, locator):
    cursor = start
    valid = 0
    rejected = {}
    results = []
    decoded = 0
    candidates = 0
    while cursor < stop and len(results) < limit and candidates < 25000 and decoded < 256 * MIB:
        header = HEADERS.search(data, cursor, min(stop + 1, len(data)))
        if not header or header.start() >= stop:
            cursor = stop
            break
        pos = header.start()
        candidates += 1
        # Observed zlib streams start on 16-byte boundaries in these archives.
        cursor = pos + 1
        if pos % 16:
            continue
        payload, consumed, reason = inflate(data, pos)
        if payload is None:
            rejected[reason] = rejected.get(reason, 0) + 1
            continue
        valid += 1
        decoded += len(payload)
        cursor = pos + consumed
        found, more = matches(payload, compiled, 0, len(payload), 12)
        if found:
            results.append({"kind": "zlib-text", "locator": {**locator, "offset": pos,
                             "sha256": sha(payload)}, "compressedBytes": consumed,
                            "decodedBytes": len(payload), "matches": found,
                            "matchesTruncated": more is not None})
    return results, cursor, {"candidateHeaders": candidates, "verifiedStreams": valid,
                             "verifiedDecodedBytes": decoded, "rejectedCandidates": rejected}


def replay(index, loc):
    args = {"source": loc["source"], "path": loc["path"]}
    file = confined_file(index[args["source"]].root, args["path"])
    previous = file.stat()
    with file.open("rb") as fh, mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as data:
        stamp = os.fstat(fh.fileno())
        if not data_start(data) <= loc["offset"] < len(data) or loc["offset"] % 16:
            raise ValueError("Locator does not point to an aligned RPF9 data offset.")
        payload, size, reason = inflate(data, loc["offset"])
        check_stamp(file, previous, fh, stamp)
    if reason or sha(payload) != loc["sha256"]:
        raise ValueError("Stream no longer matches the evidence locator.")
    return payload, size


def check_stamp(file, previous, fh, opened):
    after = file.stat()
    current = os.fstat(fh.fileno())
    # Windows path and handle ctime have different meanings on some Python versions.
    def identity(stat):
        return stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns

    if (identity(previous) != identity(after) or identity(current) != identity(opened)
            or identity(after) != identity(opened) or previous.st_ctime_ns != after.st_ctime_ns
            or current.st_ctime_ns != opened.st_ctime_ns):
        raise ValueError("Source changed during the read. Repeat the search.")


def loose_payload(index, loc, max_bytes=MAX_DECODED):
    file = confined_file(index[loc["source"]].root, loc["path"])
    previous = file.stat()
    with file.open("rb") as fh:
        opened = os.fstat(fh.fileno())
        if opened.st_size > max_bytes:
            raise ValueError(f"Loose file exceeds {max_bytes // MIB} MiB.")
        data = fh.read(max_bytes + 1)
        check_stamp(file, previous, fh, opened)
    if len(data) != opened.st_size:
        raise ValueError("Incomplete loose-file read.")
    return data


def index_rows(stream, *, decode=True):
    total = 0
    line_number = 0
    while line := stream.readline(MAX_INDEX_LINE + 1):
        total += len(line)
        line_number += 1
        if len(line) > MAX_INDEX_LINE or total > MAX_INDEX_BYTES:
            raise ValueError("SixNG texture-users index exceeds the supported read budget.")
        row = json.loads(line) if decode else line
        if decode and not isinstance(row, dict):
            raise ValueError(f"Expected a JSON object at index line {line_number}.")
        yield line_number, row


def used_by(source, args):
    path = source.texture_users
    if path is None:
        raise ValueError("No textureUsersIndex configured for this source. Build it with SixNG texture-users.")
    if not path.is_file():
        raise ValueError("The configured SixNG texture-users index is missing.")
    target = args["textureHash"].lower().removeprefix("0x")
    if not TEXTURE_HASH.fullmatch(target) or target == "0" * 16:
        raise ValueError("Use a nonzero 16-digit texture hash from a material or texture catalog.")
    previous = path.stat()
    found = {}
    material_offset = args.get("offset", 0)
    model_offset = args.get("modelOffset", 0)
    model_limit = args.get("modelLimit", 5)
    total_users = 0
    with path.open("rb") as file:
        opened = os.fstat(file.fileno())
        if opened.st_size > MAX_INDEX_BYTES:
            raise ValueError("SixNG texture-users index exceeds the supported read budget.")
        digest = hashlib.file_digest(file, "sha256").hexdigest()
        if args.get("indexSha256", digest) != digest:
            raise ValueError("The SixNG index changed. Restart the lookup from the first page.")
        file.seek(0)
        with gzip.GzipFile(fileobj=file) as stream:
            rows = index_rows(stream)
            _, header = next(rows, (0, {}))
            material_count = header.get("materials")
            model_count = header.get("models")
            if (header.get("texture_users") != 1 or type(material_count) is not int
                    or type(model_count) is not int or min(material_count, model_count) < 0):
                raise ValueError("Expected a version-1 SixNG texture-users index with row counts.")
            for mid in range(material_count):
                line, row = next(rows, (0, {}))
                definition = row.get("m")
                if (not isinstance(definition, list) or len(definition) != 2
                        or not isinstance(definition[0], str) or not isinstance(definition[1], list)):
                    raise ValueError(f"Invalid SixNG material row {mid}.")
                slots = definition[1]
                matched = False
                for slot in slots:
                    if (not isinstance(slot, list) or len(slot) != 2 or not isinstance(slot[0], str)
                            or not isinstance(slot[1], str) or not TEXTURE_HASH.fullmatch(slot[1])):
                        raise ValueError(f"Invalid texture slot at index line {line}.")
                    if slot[1].lower() == target:
                        matched = True
                if matched:
                    found[mid] = 0
            for _ in range(model_count):
                line, row = next(rows, (0, {}))
                ids = row.get("mats")
                if (not isinstance(row.get("id"), str) or not isinstance(row.get("name"), str)
                        or not isinstance(ids, list)
                        or any(type(mid) is not int or not 0 <= mid < material_count for mid in ids)):
                    raise ValueError(f"Invalid SixNG model row at index line {line}.")
                used = found.keys() & set(ids)
                total_users += bool(used)
                for mid in used:
                    found[mid] += 1
            if next(rows, None) is not None:
                raise ValueError("SixNG texture-users index has extra rows beyond its declared counts.")
        end = material_offset + args.get("limit", 10)
        selected = sorted(found, key=lambda mid: (-found[mid], mid))[material_offset:end]
        page = material_details(file, material_count, selected, found, target, model_offset, model_limit)
        check_stamp(path, previous, file, opened)
    for material in page:
        next_model = model_offset + len(material["models"])
        material["nextModelOffset"] = next_model if next_model < material["modelCount"] else None
    return {"source": args["source"], "textureHash": target, "indexSha256": digest,
            "evidence": "sixng-texture-users-v1", "indexedMaterials": material_count,
            "indexedModels": model_count, "matchedMaterials": len(found), "matchedModels": total_users,
            "materials": page, "nextOffset": end if end < len(found) else None}


def material_details(file, material_count, selected, counts, target, model_offset, model_limit):
    if not selected:
        return []
    details = {}
    remaining = dict.fromkeys(selected, model_offset)
    wanted = {mid for mid in selected if counts[mid] > model_offset}
    file.seek(0)
    with gzip.GzipFile(fileobj=file) as stream:
        for line, raw in index_rows(stream, decode=False):
            mid = line - 2
            if mid in remaining:
                shader, slots = json.loads(raw)["m"]
                matched = [name[:128] for name, h in slots if h.lower() == target]
                details[mid] = {
                    "materialId": mid, "indexLine": line, "shader": shader[:128],
                    "slots": [{"parameter": name[:128], "textureHash": h.lower(),
                               "matchesTarget": h.lower() == target} for name, h in slots[:64]],
                    "slotCount": len(slots), "matchedParameterCount": len(matched),
                    "matchedParameters": matched[:64], "modelCount": counts[mid], "models": [],
                }
            if mid < material_count:
                continue
            if not wanted:
                break
            model = json.loads(raw)
            for mid in wanted & set(model["mats"]):
                if remaining[mid]:
                    remaining[mid] -= 1
                    continue
                models = details[mid]["models"]
                models.append({"id": model["id"][:512], "name": model["name"][:256], "indexLine": line})
                if len(models) == min(model_limit, counts[mid] - model_offset):
                    wanted.remove(mid)
    return [details[mid] for mid in selected]


def dispatch(index, operation, args):
    if operation == "list_sources":
        return {"sources": [{"id": key, "game": value.game, "textureUsersConfigured": value.texture_users is not None}
                            for key, value in index.items()]}
    if operation in {"inspect_texture", "find_texture", "inspect_game_texture"}:
        # Isolated Python omits the script directory from its module search path.
        sys.path.insert(0, str(Path(__file__).resolve().parent))
        from texture_image import inspect_png
        metadata = {}
        if operation == "inspect_texture":
            payload = loose_payload(index, args, 32 * MIB)
        else:
            try:
                from native_texture import find, decode
            except ImportError as error:
                raise ValueError(f"Install the rage-texture Python binding: {error}") from None
            root = index[args["source"]].root
            if operation == "find_texture":
                return find(root, args, confined_file, source_directories)
            payload, metadata = decode(confined_file(root, args["path"]), args)
        digest = sha(payload)
        if args.get("sha256", digest) != digest:
            raise ValueError("Texture digest changed. Recheck the exported file.")
        return {"source": args["source"], "path": args["path"], "sha256": digest,
                **metadata, **inspect_png(payload, args)}
    if operation == "used_by":
        return used_by(index[args["source"]], args)
    if operation == "compare_evidence":
        left, _ = replay(index, args["left"])
        right_loc = args["right"]
        right = replay(index, right_loc)[0] if "sha256" in right_loc else loose_payload(index, right_loc)
        return {"relation": "identical-decoded-payload" if left == right else "different-decoded-payload",
                "leftBytes": len(left), "rightBytes": len(right),
                "left": args["left"], "right": right_loc, "rightPayloadSha256": sha(right)}
    if operation == "inspect_stream":
        payload, size = replay(index, args["locator"])
        found, more = matches(payload, list(patterns(args["terms"])), 0, len(payload), 100)
        return {"locator": args["locator"], "decodedBytes": len(payload), "compressedBytes": size,
                "matches": found, "matchesTruncated": more is not None}

    source = args["source"]
    root = index[source].root
    if operation == "find_files":
        return file_matches(root, args["query"], args.get("after", ""), args.get("limit", 25))
    if operation != "search_evidence":
        raise ValueError("Unknown research operation.")

    file = confined_file(root, args["path"])
    locator = {"source": source, "path": file.relative_to(root).as_posix()}
    compiled = list(patterns(args["terms"]))
    mode = args["mode"]
    previous = file.stat()
    with file.open("rb") as fh:
        stamp = os.fstat(fh.fileno())
        if not stamp.st_size:
            if mode == "rpf9":
                raise ValueError("Expected an RPF9 archive.")
            check_stamp(file, previous, fh, stamp)
            return {**locator, "mode": mode, "fileBytes": 0, "results": [], "nextOffset": None}
        with mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as data:
            start = args.get("offset", 0)
            if mode == "rpf9":
                start = max(start, data_start(data))
            start = min(start, len(data))
            stop = min(len(data), start + args.get("bytes", 16 * MIB))
            limit = args.get("limit", 25)
            if mode == "rpf9":
                found, cursor, coverage = archive_scan(data, compiled, start, stop, limit, locator)
            elif mode == "raw":
                # Look ahead far enough to retain hits that cross adjacent windows.
                results, more = matches(data, compiled, start, min(len(data), stop + 129), limit)
                found = [r for r in results if r["byteOffset"] < stop]
                cursor = min(stop, more) if more is not None else stop
                coverage = {"rawBytes": cursor - start}
            else:
                raise ValueError("Unknown search mode.")
        check_stamp(file, previous, fh, stamp)
    return {**locator, "mode": mode, "fileBytes": stamp.st_size, "startOffset": start,
            "endOffset": cursor, "nextOffset": cursor if cursor < stamp.st_size else None,
            "coverage": coverage, "results": found}


if __name__ == "__main__":
    try:
        config, operation, args = sys.argv[1:]
        print(json.dumps(dispatch(sources(Path(config).resolve()), operation, json.loads(args))))
    except (ValueError, KeyError, OSError, EOFError, zlib.error) as error:
        message = (error.strerror or str(error)) if isinstance(error, OSError) else str(error)
        sys.exit(message or "Could not read the configured source.")
