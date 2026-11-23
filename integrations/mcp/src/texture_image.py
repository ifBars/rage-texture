"""Inspect a bounded region of a previously exported PNG texture."""
import base64
import io
import os
from pathlib import Path
import shutil
import subprocess


def recognize(image, left, top):
    executable = os.environ.get("RAGE_RESEARCH_TESSERACT") or shutil.which("tesseract")
    if not executable and os.name == "nt":
        executable = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Tesseract-OCR/tesseract.exe"
    if not executable or not Path(executable).is_file():
        raise ValueError("OCR requires Tesseract with English data. Set RAGE_RESEARCH_TESSERACT to its executable.")
    # Enlarge small lettering while bounding both compute and the OCR input.
    scale = min(2, 2048 / max(image.size))
    size = tuple(max(1, round(value * scale)) for value in image.size)
    from PIL import Image
    encoded = io.BytesIO()
    image.resize(size, Image.Resampling.LANCZOS).convert("L").save(encoded, "PNG")
    try:
        result = subprocess.run([str(executable), "stdin", "stdout", "-l", "eng", "--psm", "11", "tsv"],
                                input=encoded.getvalue(), capture_output=True, timeout=25)
    except subprocess.TimeoutExpired:
        raise ValueError("OCR exceeded 25 seconds. Try a smaller crop.") from None
    if result.returncode:
        raise ValueError("Tesseract failed. Check its English language data.")
    words = []
    for row in result.stdout.decode("utf-8").splitlines()[1:]:
        cells = row.split("\t", 11)
        if len(cells) != 12 or cells[0] != "5" or not cells[11].strip():
            continue
        if len(words) == 512:
            return {"words": words, "truncated": True}
        x, y, w, h = map(int, cells[6:10])
        sx, sy = image.width / size[0], image.height / size[1]
        words.append({"text": cells[11], "confidence": float(cells[10]),
                      "box": [left + round(x*sx), top + round(y*sy),
                              left + round((x+w)*sx), top + round((y+h)*sy)]})
    return {"words": words, "truncated": False}


def inspect_png(payload, args):
    try:
        from PIL import Image
    except ImportError:
        raise ValueError("Texture inspection requires Pillow in the configured Python interpreter.") from None
    with Image.open(io.BytesIO(payload), formats=["PNG"]) as image:
        width, height = image.size
        if width * height > 16_777_216 or getattr(image, "is_animated", False):
            raise ValueError("Use a single PNG image of at most 16 megapixels.")
        crop = args.get("crop", {"x": 0, "y": 0, "width": width, "height": height})
        x, y = crop["x"], crop["y"]
        right, bottom = x + crop["width"], y + crop["height"]
        if not (0 <= x < right <= width and 0 <= y < bottom <= height):
            raise ValueError("Crop must fit within the original texture.")
        rgba = image.crop((x, y, right, bottom)).convert("RGBA")
        region = Image.new("RGB", rgba.size, "white")
        region.paste(rgba, mask=rgba.getchannel("A"))
    result = {"width": width, "height": height, "crop": crop}
    if args.get("ocr"):
        result["ocr"] = recognize(region, x, y)
    region.thumbnail((768, 768), Image.Resampling.LANCZOS)
    preview = io.BytesIO()
    region.save(preview, "JPEG", quality=90)
    result["previewBase64"] = base64.b64encode(preview.getvalue()).decode("ascii")
    return result
