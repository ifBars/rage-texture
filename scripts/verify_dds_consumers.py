"""Independent consumer QA for generated DDS files with Microsoft DirectXTex.

Two checks are kept separate:

* Container: ``texdiag info`` parses the file and reports format, dimensions,
  mips, array size, cube/volume dimension. Every field must match the manifest.
* Pixels: ``texconv`` decodes to R8G8B8A8_UNORM in a DX10 DDS; this script then
  reads that lossless container and compares every subresource against the
  manifest's expected RGBA bytes or SHA-256. The expected data never comes from
  DirectXTex itself, so a pass is a cross-decoder agreement, not a self-check.

Unsupported formats, missing subresources, failed conversions and missing
oracles are failures, never passes. No game data is written to the repository.

Manifest schema (all paths relative to the manifest directory unless absolute)::

    {
      "samples": [
        {
          "file": "sample.dds",
          "kind": "2d" | "array" | "cube" | "volume",
          "dxgi": 28,
          "width": 16, "height": 16, "depth": 1,
          "mips": 4,
          "subresources": [
            {
              "mip": 0, "slice": 0,
              "sha256": "<64 hex>",
              "reference": "refs/sample_m0_s0.rgba",
              "channels": "rgba",
              "tolerance": 0
            }
          ]
        }
      ]
    }

``slice`` is the array item, cube face or volume Z. ``reference`` is raw
tightly packed RGBA8 (``width * height * 4`` bytes for that subresource).
``tolerance`` is the maximum absolute per-channel difference for reference
comparisons (default 0). A SHA-256-only oracle is always exact.

Use ``--self-test`` to generate a bounded synthetic set (2D, array, cube,
volume and a truncated negative control), then run the same audit.
"""

import argparse
import hashlib
import json
import pathlib
import struct
import subprocess
import sys
import tempfile

FORMAT_NAMES = {
    10: "R16G16B16A16_FLOAT",
    11: "R16G16B16A16_UNORM",
    28: "R8G8B8A8_UNORM",
    29: "R8G8B8A8_UNORM_SRGB",
    54: "R16_FLOAT",
    61: "R8_UNORM",
    65: "A8_UNORM",
    71: "BC1_UNORM",
    72: "BC1_UNORM_SRGB",
    74: "BC2_UNORM",
    77: "BC3_UNORM",
    78: "BC3_UNORM_SRGB",
    80: "BC4_UNORM",
    83: "BC5_UNORM",
    87: "B8G8R8A8_UNORM",
    91: "B8G8R8A8_UNORM_SRGB",
    95: "BC6H_UF16",
    96: "BC6H_SF16",
    98: "BC7_UNORM",
    99: "BC7_UNORM_SRGB",
}
REAL_FORMATS = set(FORMAT_NAMES)
BLOCK_FORMATS = frozenset(
    set(range(70, 85)) | {94, 95, 96, 97, 98, 99}
)
SRGB_FORMATS = frozenset({29, 72, 75, 78, 91, 93, 99})
# Formats whose stored precision is above 8 bits per channel. Converting to
# RGBA8 can differ by one LSB between independent converters.
QUANTIZED_FORMATS = (
    set(range(2, 9))
    | set(range(10, 19))
    | {24, 25, 26}
    | set(range(34, 39))
    | set(range(41, 44))
    | set(range(54, 60))
    | {95, 96}
)
TOLERATED_FORMATS = BLOCK_FORMATS | QUANTIZED_FORMATS


def stored_channels(dxgi):
    """The channels the DDS format actually stores.

    A comparison mask must include all of them; it may add presentation-only
    channels, but it may not hide a stored channel.
    """
    if dxgi in (6, 7, 8, 26, 95, 96):
        return "rgb"
    if dxgi in (16, 17, 18, 34, 35, 36, 37, 38, 49, 50, 51, 52, 82, 83, 84):
        return "rg"
    if dxgi in (41, 42, 43, 54, 56, 57, 58, 59, 61, 62, 63, 64, 79, 80, 81):
        return "r"
    if dxgi == 65:
        return "a"
    if dxgi in (88, 93):
        return "rgb"
    return "rgba"


def fail(message):
    raise ValueError(message)


def run(tool, *args):
    proc = subprocess.run(
        [str(tool), *map(str, args)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        errors="replace",
    )
    return proc.returncode, proc.stdout


def parse_texdiag(text):
    fields = {}
    for line in text.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if key:
            fields[key] = value.strip()
    return fields


def metadata_for(entry):
    kind = entry["kind"]
    dxgi = int(entry["dxgi"])
    name = FORMAT_NAMES.get(dxgi)
    if name is None:
        fail(f"DXGI {dxgi} has no Microsoft format name in this checker")
    expect = {
        "width": str(entry["width"]),
        "height": str(entry["height"]),
        "mipLevels": str(entry["mips"]),
        "format": name,
    }
    if kind == "volume":
        expect.update(depth=str(entry["depth"]), arraySize="1", dimension="3D")
    elif kind == "cube":
        if int(entry["depth"]) != 6:
            fail("a cube sample must have depth 6")
        expect.update(depth="1", arraySize="6", dimension="Cube")
    elif kind == "array":
        expect.update(depth="1", arraySize=str(entry["depth"]), dimension="2DArray")
    elif kind == "2d":
        expect.update(depth="1", arraySize="1", dimension="2D")
    else:
        fail(f"unknown kind {kind!r}")
    return expect


def expected_offsets(entry):
    """Yield (mip, slice, width, height, offset, size) in a DX10 RGBA8 DDS."""
    kind = entry["kind"]
    width, height, depth = int(entry["width"]), int(entry["height"]), int(entry["depth"])
    mips = int(entry["mips"])
    out = []
    offset = 0
    if kind == "volume":
        for mip in range(mips):
            w, h = max(1, width >> mip), max(1, height >> mip)
            d = max(1, depth >> mip)
            size = w * h * 4
            for z in range(d):
                out.append((mip, z, w, h, offset + z * size, size))
            offset += size * d
        return out
    layers = 6 if kind == "cube" else depth
    for layer in range(layers):
        for mip in range(mips):
            w, h = max(1, width >> mip), max(1, height >> mip)
            size = w * h * 4
            out.append((mip, layer, w, h, offset, size))
            offset += size
    return out


def read_dx10_layout(data, what):
    if len(data) < 148 or data[:4] != b"DDS ":
        fail(f"{what}: not a DX10 DDS")
    if data[84:88] != b"DX10":
        fail(f"{what}: missing DX10 header")
    height, width = struct.unpack_from("<II", data, 12)
    depth, mips = struct.unpack_from("<II", data, 24)
    dxgi, dimension, misc, array_size = struct.unpack_from("<4I", data, 128)
    return {
        "width": width,
        "height": height,
        "depth": depth if depth else 1,
        "mips": mips if mips else 1,
        "dxgi": dxgi,
        "dimension": dimension,
        "misc": misc,
        "array_size": array_size,
        "payload": data[148:],
    }


def audit_sample(entry, base, texdiag, texconv, work, limit):
    errors = []
    checks = 0
    decoded = 0
    exact = quantized = masked_exact = masked_quantized = 0
    sample_path = (base / entry["file"]).resolve()
    if not sample_path.is_file():
        return {"file": entry["file"], "checks": 0, "decoded": 0,
                "errors": [f"missing sample {sample_path}"]}

    code, output = run(texdiag, "info", "-nologo", sample_path)
    if code != 0:
        return {"file": entry["file"], "checks": 0, "decoded": 0,
                "errors": [f"texdiag failed: {output.strip()[:400]}"]}
    fields = parse_texdiag(output)
    for key, want in metadata_for(entry).items():
        checks += 1
        got = fields.get(key)
        if key == "dimension" and got:
            # texdiag prints 3D on the same line as the alpha mode.
            got = got.split()[0]
        if got != want:
            errors.append(f"texdiag {key}={got!r}, expected {want!r}")

    out_dir = work / pathlib.Path(entry["file"]).stem
    out_dir.mkdir(parents=True, exist_ok=True)
    srgb = int(entry["dxgi"]) in SRGB_FORMATS
    out_fmt = "R8G8B8A8_UNORM_SRGB" if srgb else "R8G8B8A8_UNORM"
    code, output = run(
        texconv, "-nologo", "-y", "-dx10", "-m", entry["mips"],
        "-f", out_fmt, "-ft", "dds", "-o", out_dir, sample_path,
    )
    if code != 0:
        errors.append(f"texconv failed: {output.strip()[:400]}")
        return {"file": entry["file"], "checks": checks, "decoded": 0,
                "errors": errors[:limit]}

    converted = out_dir / (sample_path.stem + ".dds")
    if not converted.is_file():
        errors.append(f"texconv produced no {converted.name}")
        return {"file": entry["file"], "checks": checks, "decoded": 0,
                "errors": errors[:limit]}
    try:
        layout = read_dx10_layout(converted.read_bytes(), converted.name)
    except ValueError as exc:
        errors.append(str(exc))
        return {"file": entry["file"], "checks": checks, "decoded": 0,
                "errors": errors[:limit]}

    # The conversion must preserve the manifest geometry, or offsets are moot.
    expected_geometry = {
        "width": int(entry["width"]),
        "height": int(entry["height"]),
        "mips": int(entry["mips"]),
        "depth": int(entry["depth"]) if entry["kind"] == "volume" else 1,
        "dimension": 4 if entry["kind"] == "volume" else 3,
        "misc": 4 if entry["kind"] == "cube" else 0,
        "array_size": {
            "volume": 1,
            "cube": 1,
            "array": int(entry["depth"]),
            "2d": 1,
        }[entry["kind"]],
    }
    for key, want in expected_geometry.items():
        checks += 1
        if layout[key] != want:
            errors.append(f"converted {key}={layout[key]}, expected {want}")
    if layout["dxgi"] != (29 if srgb else 28):
        errors.append(
            f"converted DXGI {layout['dxgi']}, expected {'sRGB' if srgb else 'UNORM'} RGBA8"
        )

    offsets = { (m, s): (w, h, off, size) for m, s, w, h, off, size in expected_offsets(entry) }
    expected_payload = sum(size for _, _, _, _, _, size in expected_offsets(entry))
    checks += 1
    if len(layout["payload"]) != expected_payload:
        errors.append(
            f"converted payload is {len(layout['payload'])} bytes, "
            f"expected {expected_payload}"
        )
    seen = set()
    for sub in entry.get("subresources", []):
        mip, sl = int(sub["mip"]), int(sub["slice"])
        key = (mip, sl)
        checks += 1
        if key in seen:
            errors.append(f"duplicate subresource mip {mip} slice {sl}")
            continue
        seen.add(key)
        if key not in offsets:
            errors.append(f"subresource mip {mip} slice {sl} is outside the shape")
            continue
        w, h, off, size = offsets[key]
        if "width" in sub and int(sub["width"]) != w:
            errors.append(f"mip {mip} slice {sl}: width {w}, expected {sub['width']}")
        if "height" in sub and int(sub["height"]) != h:
            errors.append(f"mip {mip} slice {sl}: height {h}, expected {sub['height']}")
        if off < 0 or off + size > len(layout["payload"]):
            errors.append(f"mip {mip} slice {sl}: conversion payload is short")
            continue
        pixels = layout["payload"][off:off + size]
        decoded += 1
        oracle = False
        tolerance = int(sub.get("tolerance", 0))
        dxgi = int(entry["dxgi"])
        if tolerance not in (0, 1):
            errors.append(f"mip {mip} slice {sl}: invalid tolerance {tolerance}")
            continue
        if tolerance == 1 and dxgi not in TOLERATED_FORMATS:
            errors.append(
                f"mip {mip} slice {sl}: tolerance 1 is not allowed for DXGI {dxgi}"
            )
            continue
        channels = sub.get("channels", "rgba")
        if not channels or any(c not in "rgba" for c in channels):
            errors.append(f"mip {mip} slice {sl}: invalid channels {channels!r}")
            continue
        required = set(stored_channels(dxgi))
        missing = required - set(channels)
        if missing:
            errors.append(
                f"mip {mip} slice {sl}: channels {channels!r} omit stored "
                f"channel(s) {''.join(sorted(missing))!r}"
            )
            continue
        masked = set(channels) != set("rgba")
        channel_offsets = [i for i, c in enumerate("rgba") if c in channels]
        has_reference = "reference" in sub
        if has_reference:
            oracle = True
            ref_path = (base / sub["reference"]).resolve()
            try:
                expected = ref_path.read_bytes()
            except OSError as exc:
                errors.append(f"mip {mip} slice {sl}: reference unreadable ({exc})")
                continue
            if len(expected) != size:
                errors.append(
                    f"mip {mip} slice {sl}: reference is {len(expected)} bytes, expected {size}"
                )
                continue
            mv_expected = memoryview(expected)
            mv_pixels = memoryview(pixels)
            if tolerance:
                worst = 0
                for off in channel_offsets:
                    for a, b in zip(mv_expected[off::4], mv_pixels[off::4]):
                        delta = abs(a - b)
                        if delta > worst:
                            worst = delta
                            if worst > tolerance:
                                break
                    if worst > tolerance:
                        break
                if worst > tolerance:
                    errors.append(
                        f"mip {mip} slice {sl}: max channel delta {worst} > {tolerance}"
                    )
            elif any(
                bytes(mv_expected[off::4]) != bytes(mv_pixels[off::4])
                for off in channel_offsets
            ):
                where = "" if not masked else f" on channels {channels!r}"
                errors.append(
                    f"mip {mip} slice {sl}: reference pixels differ{where}"
                )
        if "sha256" in sub:
            if masked:
                errors.append(
                    f"mip {mip} slice {sl}: a byte hash cannot check channels {channels!r}"
                )
            elif not has_reference or tolerance == 0:
                oracle = True
                got = hashlib.sha256(pixels).hexdigest()
                if got != sub["sha256"]:
                    errors.append(
                        f"mip {mip} slice {sl}: pixel SHA-256 differs ({got})"
                    )
        if not oracle:
            errors.append(f"mip {mip} slice {sl}: no reference oracle")
        if masked and tolerance:
            masked_quantized += 1
        elif masked:
            masked_exact += 1
        elif tolerance:
            quantized += 1
        else:
            exact += 1

    expected_count = len(expected_offsets(entry))
    checks += 1
    if len(seen) != expected_count:
        errors.append(
            f"manifest covers {len(seen)} subresources, shape has {expected_count}"
        )
    return {"file": entry["file"], "checks": checks, "decoded": decoded,
            "exact": exact, "quantized": quantized,
            "masked_exact": masked_exact, "masked_quantized": masked_quantized,
            "errors": errors[:limit]}


def audit(manifest, samples, texdiag, texconv, work, limit):
    entries = manifest.get("samples")
    if not isinstance(entries, list) or not entries:
        fail("manifest has no samples")
    results = [audit_sample(e, samples, texdiag, texconv, work, limit) for e in entries]
    errors = sum(len(r["errors"]) for r in results)
    return {
        "scope": (
            "Independent DirectXTex consumer QA. Container metadata comes from "
            "texdiag; pixels come from texconv's RGBA8 decode compared with "
            "manifest references or SHA-256. Missing oracles are failures."
        ),
        "samples": len(results),
        "checks": sum(r["checks"] for r in results),
        "decoded_subresources": sum(r["decoded"] for r in results),
        "exact_subresources": sum(r.get("exact", 0) for r in results),
        "quantized_subresources": sum(r.get("quantized", 0) for r in results),
        "masked_exact_subresources": sum(r.get("masked_exact", 0) for r in results),
        "masked_quantized_subresources": sum(
            r.get("masked_quantized", 0) for r in results
        ),
        "presentation_subresources": sum(
            r.get("masked_exact", 0) + r.get("masked_quantized", 0) for r in results
        ),
        "equal": sum(1 for r in results if not r["errors"]),
        "errors": errors,
        "results": results,
    }


# --- bounded synthetic self-test ------------------------------------------------


def write_dds(path, width, height, depth, mips, kind, payload, dxgi=28, block=None):
    flags = 0x1 | 0x2 | 0x4 | 0x1000 | (0x80000 if block else 0x8)
    flags |= 0x20000 if mips > 1 else 0
    if kind == "volume":
        flags |= 0x800000
    caps = 0x1000 | 0x8 | (0x400000 if mips > 1 else 0)
    caps2 = 0
    dimension, misc, array_size = 3, 0, 1
    if kind == "cube":
        caps2 = 0xFE00
        misc = 4
        array_size = 1
    elif kind == "array":
        array_size = depth
    elif kind == "volume":
        caps2 = 0x200000
        dimension = 4
    header = bytearray(128)
    header[:4] = b"DDS "
    struct.pack_into("<I", header, 4, 124)
    struct.pack_into("<I", header, 8, flags)
    struct.pack_into("<I", header, 12, height)
    struct.pack_into("<I", header, 16, width)
    if block:
        pitch = ((width + 3) // 4) * ((height + 3) // 4) * block
    else:
        pitch = width * 4
    struct.pack_into("<I", header, 20, pitch)
    struct.pack_into("<I", header, 24, depth if kind == "volume" else 0)
    struct.pack_into("<I", header, 28, mips)
    struct.pack_into("<I", header, 76, 32)
    struct.pack_into("<I", header, 80, 4)
    header[84:88] = b"DX10"
    struct.pack_into("<I", header, 108, caps)
    struct.pack_into("<I", header, 112, caps2)
    dx10 = struct.pack("<5I", dxgi, dimension, misc, array_size, 0)
    path.write_bytes(bytes(header) + dx10 + payload)


def rgba_pattern(width, height, mip, slice):
    out = bytearray()
    for y in range(height):
        for x in range(width):
            out += bytes(
                (
                    (x * 7 + y * 5 + mip * 13 + slice * 29) & 0xFF,
                    (x * 3 + y * 11 + mip * 7) & 0xFF,
                    (x + y + mip + slice) & 0xFF,
                    255 - ((x + y + mip + slice) & 0x3F),
                )
            )
    return bytes(out)


def build_self_test(root):
    root.mkdir(parents=True, exist_ok=True)
    refs = root / "refs"
    refs.mkdir(exist_ok=True)
    samples = []
    specs = [
        ("selftest_2d", "2d", 16, 16, 1, 4),
        ("selftest_array", "array", 16, 8, 3, 2),
        ("selftest_cube", "cube", 8, 8, 6, 2),
        ("selftest_volume", "volume", 8, 8, 8, 1),
    ]
    for name, kind, width, height, depth, mips in specs:
        payload = bytearray()
        subs = []
        if kind == "volume":
            for mip in range(mips):
                mw, mh = max(1, width >> mip), max(1, height >> mip)
                md = max(1, depth >> mip)
                for z in range(md):
                    px = rgba_pattern(mw, mh, mip, z)
                    payload += px
                    ref = refs / f"{name}_m{mip}_s{z}.rgba"
                    ref.write_bytes(px)
                    subs.append({"mip": mip, "slice": z, "width": mw,
                                 "height": mh, "reference": str(ref.relative_to(root)),
                                 "sha256": hashlib.sha256(px).hexdigest()})
        else:
            layers = 6 if kind == "cube" else depth
            for layer in range(layers):
                for mip in range(mips):
                    mw, mh = max(1, width >> mip), max(1, height >> mip)
                    px = rgba_pattern(mw, mh, mip, layer)
                    payload += px
                    ref = refs / f"{name}_m{mip}_s{layer}.rgba"
                    ref.write_bytes(px)
                    subs.append({"mip": mip, "slice": layer, "width": mw,
                                 "height": mh, "reference": str(ref.relative_to(root)),
                                 "sha256": hashlib.sha256(px).hexdigest()})
        path = root / f"{name}.dds"
        write_dds(path, width, height, depth, mips, kind, bytes(payload))
        samples.append({"file": path.name, "kind": kind, "dxgi": 28,
                        "width": width, "height": height, "depth": depth,
                        "mips": mips, "subresources": subs})
    # Synthetic codec cases that independent decoders must agree on:
    # BC1 3-colour block with transparent index 3, and BC3 low-ordered
    # endpoints (BC3 is always 4-colour, unlike BC1).
    bc1_payload = struct.pack("<HH", 0, 0xFFFF) + b"\xff\xff\xff\xff"
    bc1_ref = bytes(4 * 4 * 4)
    (refs / "selftest_bc1_transparent.rgba").write_bytes(bc1_ref)
    write_dds(root / "selftest_bc1_transparent.dds", 4, 4, 1, 1, "2d",
              bc1_payload, dxgi=71, block=8)
    samples.append({
        "file": "selftest_bc1_transparent.dds", "kind": "2d", "dxgi": 71,
        "width": 4, "height": 4, "depth": 1, "mips": 1,
        "subresources": [{
            "mip": 0, "slice": 0, "reference": "refs/selftest_bc1_transparent.rgba",
            "sha256": hashlib.sha256(bc1_ref).hexdigest(), "tolerance": 0,
        }],
    })
    bc3_payload = (
        struct.pack("<BB", 255, 255) + bytes(6)
        + struct.pack("<HH", 0, 0xFFFF) + b"\xaa\xaa\xaa\xaa"
    )
    bc3_ref = bytes((85, 85, 85, 255)) * 16
    (refs / "selftest_bc3_four_colour.rgba").write_bytes(bc3_ref)
    write_dds(root / "selftest_bc3_four_colour.dds", 4, 4, 1, 1, "2d",
              bc3_payload, dxgi=77, block=16)
    samples.append({
        "file": "selftest_bc3_four_colour.dds", "kind": "2d", "dxgi": 77,
        "width": 4, "height": 4, "depth": 1, "mips": 1,
        "subresources": [{
            "mip": 0, "slice": 0, "reference": "refs/selftest_bc3_four_colour.rgba",
            "sha256": hashlib.sha256(bc3_ref).hexdigest(), "tolerance": 0,
        }],
    })
    # Negative control: truncation must not pass the container check.
    full = (root / "selftest_2d.dds").read_bytes()
    (root / "selftest_truncated.dds").write_bytes(full[:-16])
    return {"samples": samples}


# --- bounded representative samples from a read-only store ---------------------


def generate_samples(store, catalog, root, max_samples, max_dimension, max_subresources):
    """Export one bounded sample per observed (DXGI, dimension) pair.

    The store is opened read-only by the catalog CLI with --no-cache. DDS comes
    from the SDK's full-chain writer; expected pixels come from the SDK's own
    PNG previews, so DirectXTex is still the independent decoder in the audit.
    """
    from PIL import Image

    code, output = run(catalog, "list", store)
    if code != 0:
        fail(f"catalog list failed: {output.strip()[:300]}")
    textures = json.loads(output)
    chosen = {}
    for t in textures:
        dxgi, dimension = int(t["dxgi"]), int(t["dimension"])
        kind = {1: "array" if int(t["depth"]) > 1 else "2d",
                2: "cube", 3: "volume"}[dimension]
        key = (dxgi, kind)
        width, height, depth, mips = (
            int(t["width"]),
            int(t["height"]),
            int(t["depth"]),
            int(t["mips"]),
        )
        if width > max_dimension or height > max_dimension:
            continue
        slices = depth if kind in ("array", "cube", "volume") else 1
        if slices * mips > max_subresources:
            continue
        group = chosen.setdefault(key, [])
        if len(group) < 8:
            group.append(t)
        if len(chosen) >= max_samples and len(group) == 1:
            break

    root.mkdir(parents=True, exist_ok=True)
    refs = root / "refs"
    refs.mkdir(exist_ok=True)
    samples = []
    for (dxgi, kind), group in sorted(chosen.items()):
        for t in group:
            index = int(t["index"])
            name = f"fmt{dxgi}_{kind}_{index}"
            dds_path = root / f"{name}.dds"
            code, output = run(catalog, "export", store, index, dds_path, "--no-cache")
            if code != 0:
                print(f"skip {name}: DDS export failed ({output.strip()[:160]})")
                continue
            slices = int(t["depth"]) if kind in ("array", "cube", "volume") else 1
            subs = []
            complete = True
            for mip in range(int(t["mips"])):
                for sl in range(slices):
                    png = root / f"{name}_m{mip}_s{sl}.png"
                    code, output = run(
                        catalog, "export", store, index, png,
                        "--mip", mip, "--slice", sl, "--no-cache",
                    )
                    if code != 0:
                        complete = False
                        break
                    with Image.open(png) as image:
                        rgba = image.convert("RGBA").tobytes()
                    png.unlink()
                    ref = refs / f"{name}_m{mip}_s{sl}.rgba"
                    ref.write_bytes(rgba)
                    channels = stored_channels(dxgi)
                    sub = {
                        "mip": mip,
                        "slice": sl,
                        "reference": str(ref.relative_to(root)),
                        "channels": channels,
                        "tolerance": 1 if dxgi in BLOCK_FORMATS or dxgi in QUANTIZED_FORMATS else 0,
                    }
                    if channels == "rgba":
                        sub["sha256"] = hashlib.sha256(rgba).hexdigest()
                    subs.append(sub)
                if not complete:
                    break
            if not complete:
                dds_path.unlink(missing_ok=True)
                print(f"skip {name}: a subresource preview is not available")
                continue
            samples.append({
                "file": dds_path.name,
                "kind": kind,
                "dxgi": dxgi,
                "width": int(t["width"]),
                "height": int(t["height"]),
                "depth": int(t["depth"]),
                "mips": int(t["mips"]),
                "source_index": index,
                "subresources": subs,
            })
            break
        if len(samples) >= max_samples:
            break
    if not samples:
        fail("no representative samples could be generated")
    return {"samples": samples}


def negative_controls(base, manifest, texdiag, texconv, work, limit):
    """Prove the audit rejects wrong geometry, a wrong oracle and truncation."""
    entry = json.loads(json.dumps(manifest["samples"][0]))
    source = base / entry["file"]
    good = source.read_bytes()
    cases = []

    wrong_geometry = bytearray(good)
    struct.pack_into("<I", wrong_geometry, 16, int(entry["width"]) + 1)
    (base / "neg_geometry.dds").write_bytes(bytes(wrong_geometry))
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_geometry.dds"
    cases.append(("wrong_geometry", e))

    wrong_array = bytearray(good)
    struct.pack_into("<I", wrong_array, 140, 2)
    (base / "neg_array.dds").write_bytes(bytes(wrong_array))
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_array.dds"
    cases.append(("wrong_array_size", e))

    (base / "neg_truncated.dds").write_bytes(good[:-16])
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_truncated.dds"
    cases.append(("truncated", e))

    (base / "neg_oracle.dds").write_bytes(good)
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_oracle.dds"
    for sub in e["subresources"]:
        sub.pop("reference", None)
        sub["sha256"] = "0" * 64
    cases.append(("wrong_oracle", e))

    (base / "neg_mask.dds").write_bytes(good)
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_mask.dds"
    for sub in e["subresources"]:
        sub["channels"] = "rgb"
    cases.append(("incomplete_channel_mask", e))

    (base / "neg_tolerance.dds").write_bytes(good)
    e = json.loads(json.dumps(entry))
    e["file"] = "neg_tolerance.dds"
    for sub in e["subresources"]:
        sub["tolerance"] = 2
    cases.append(("excessive_tolerance", e))

    results = []
    for name, case in cases:
        report = audit({"samples": [case]}, base, texdiag, texconv, work, limit)
        results.append({"case": name, "errors": report["errors"],
                        "rejected": report["errors"] > 0})
    return results


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--samples", type=pathlib.Path, required=True)
    p.add_argument("--manifest", type=pathlib.Path)
    p.add_argument("--texdiag", type=pathlib.Path, required=True)
    p.add_argument("--texconv", type=pathlib.Path, required=True)
    p.add_argument("--out", type=pathlib.Path, required=True)
    p.add_argument("--require-all-formats", action="store_true")
    p.add_argument("--limit", type=int, default=40)
    p.add_argument("--self-test", action="store_true")
    p.add_argument("--generate", action="store_true",
                   help="build bounded samples from --store with the SDK CLI")
    p.add_argument("--store", type=pathlib.Path)
    p.add_argument("--catalog", type=pathlib.Path)
    p.add_argument("--max-samples", type=int, default=24)
    p.add_argument("--max-dimension", type=int, default=512)
    p.add_argument("--max-subresources", type=int, default=600)
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error("reports must stay outside the repository")
    if not 1 <= a.limit <= 1000:
        p.error("limit must be 1..1000")
    if a.self_test and a.generate:
        p.error("--self-test and --generate are exclusive")
    if a.self_test:
        manifest = build_self_test(a.samples.resolve())
        # Exercise the negative control before the positive audit.
        code, output = run(a.texdiag, "info", "-nologo", a.samples / "selftest_truncated.dds")
        if code == 0:
            fail("texdiag accepted the truncated negative control")
        print(f"negative control rejected: {output.strip()[:100]}")
    elif a.generate:
        if not a.store or not a.catalog or not a.manifest:
            p.error("--generate needs --store, --catalog and --manifest")
        if a.samples.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
            p.error("generated samples and references must stay outside the repository")
        manifest = generate_samples(
            a.store.resolve(), a.catalog.resolve(), a.samples.resolve(),
            a.max_samples, a.max_dimension, a.max_subresources,
        )
        a.manifest.write_text(json.dumps(manifest, indent=1))
        if not manifest["samples"]:
            fail("no samples generated")
    else:
        if not a.manifest:
            p.error("--manifest is required without --self-test")
        manifest = json.loads(a.manifest.read_text())
    base = a.samples.resolve()
    if a.require_all_formats:
        seen = {int(e["dxgi"]) for e in manifest.get("samples", [])}
        missing = REAL_FORMATS - seen
        if missing:
            fail(f"missing required formats: {sorted(missing)}")
    if not a.texdiag.is_file() or not a.texconv.is_file():
        p.error("texdiag/texconv executable is missing")
    with tempfile.TemporaryDirectory(prefix="dds-consumer-") as tmp:
        report = audit(manifest, base, a.texdiag, a.texconv, pathlib.Path(tmp), a.limit)
    if "oracle_extension_sha256" in manifest:
        report["oracle_extension_sha256"] = manifest["oracle_extension_sha256"]
    if "oracle_extension_mtime" in manifest:
        report["oracle_extension_mtime"] = manifest["oracle_extension_mtime"]
    if a.self_test:
        with tempfile.TemporaryDirectory(prefix="dds-consumer-neg-") as tmp:
            negatives = negative_controls(
                base, manifest, a.texdiag, a.texconv, pathlib.Path(tmp), a.limit
            )
        report["negative_controls"] = negatives
        if not all(n["rejected"] for n in negatives):
            fail("a negative control was accepted")
    a.out.write_text(json.dumps(report, indent=1))
    print(json.dumps({k: report[k] for k in
                      ("samples", "checks", "decoded_subresources",
                       "exact_subresources", "quantized_subresources",
                       "masked_exact_subresources", "masked_quantized_subresources",
                       "equal", "errors")}))
    if a.self_test:
        print(json.dumps({"negative_controls": report["negative_controls"]}))
    return 0 if report["errors"] == 0 else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ValueError as exc:
        print(f"verify_dds_consumers: {exc}", file=sys.stderr)
        raise SystemExit(1)


