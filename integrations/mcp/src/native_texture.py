"""Look up native TTPF catalog keys and decode selected records in memory."""
from collections import Counter

from rage_texture import Error, Store


def describe(texture):
    return {"textureHash": f"{texture.asset_key:016x}", "catalogIndex": texture.index,
            "width": texture.width, "height": texture.height, "dxgi": texture.dxgi,
            "mips": texture.mips, "depth": texture.depth}


def find(root, args, confined_file, source_directories):
    key = int(args["textureHash"], 16)
    paths = []
    def fail(error):
        raise error
    for folder, files, _ in source_directories(root, fail):
        for name in files:
            if name.lower().endswith(".ttpf"):
                paths.append((folder / name).relative_to(root).as_posix())
    candidates = sorted(p for p in paths if p > args.get("after", ""))
    page = candidates[:args.get("storeLimit", 16)]
    matches, errors = [], []
    for path in page:
        try:
            with Store(confined_file(root, path), cache_bytes=0) as store:
                for texture in store.textures():
                    if texture.asset_key == key:
                        matches.append({"path": path, **describe(texture)})
        except (Error, OSError, ValueError) as error:
            errors.append({"path": path, "error": str(error)[:256]})
    return {"source": args["source"], "matches": matches, "errors": errors,
            "storesAttempted": len(page), "totalStores": len(paths),
            "nextAfter": page[-1] if len(candidates) > len(page) else None}


def decode(file, args):
    key = int(args["textureHash"], 16)
    policy = args.get("decodePolicy", "lossless")
    try:
        with Store(file, cache_bytes=0, allow_fitted=(policy == "fitted")) as store:
            records = [t for t in store.textures() if t.asset_key == key
                       and ("catalogIndex" not in args or t.index == args["catalogIndex"])]
            if not records:
                raise ValueError("No matching key and catalog record in this store.")
            if len(records) > 1:
                raise ValueError("Duplicate key. Select a catalogIndex from find_texture.")
            texture = records[0]
            mip, layer = args.get("mip", 0), args.get("slice", 0)
            if mip >= texture.mips:
                raise ValueError("Mip is outside this texture.")
            if max(1, texture.width >> mip) * max(1, texture.height >> mip) > 16_777_216:
                raise ValueError("Mip exceeds 16 megapixels. Choose a smaller mip.")
            export = store.texture_png(texture, mip=mip, slice=layer)
            quality = export.quality
            return export.data, {**describe(texture), "mip": mip, "slice": layer,
                                 "decodePolicy": policy, "quality": dict(Counter(q.kind for q in quality)),
                                 "unfittedBlocks": sum(q.unfitted_blocks for q in quality),
                                 "invalidBlocks": sum(q.invalid_blocks or 0 for q in quality)}
    except Error as error:
        raise ValueError(f"rage-texture: {error}") from None
