"""Getting viewable pixels out of a catalog, cheapest source first.

Three tiers:

  thumb    Cache/Thumbnails/<variant_pk:08d>.cot   450 px, a plain JPEG, no dependencies.
           Rendered by Capture One, so it shows the crop, the B&W conversion and any
           applied style - which is what you want when culling.

  preview  Cache/Previews/<ZIMAGE.ZSIDECARPATH>.cop   1920-2560 px, JPEG-XL.
           Stored as LINEAR, CAMERA-SPACE RGB, before white balance. Decoding it
           without applying ZWHITEBALANCE gives a heavy green cast, because a Fuji
           sensor's green channel is roughly twice as sensitive as its red. It also
           predates the film curve, so it is flatter than Capture One's own render.

  original the RAW/JPEG file itself. RAF embeds a full JPEG at a known offset; TIFF-based
           DNG/CR2 keep a preview in an IFD. Neither reflects any Capture One edit.
"""

from __future__ import annotations

import struct
from pathlib import Path

TIERS = ("thumb", "preview", "original")


class ImageError(Exception):
    pass


def _pillow():
    try:
        from PIL import Image
    except ImportError as exc:  # pragma: no cover - Pillow is present in this environment
        raise ImageError("Pillow is required: python3 -m pip install pillow") from exc
    return Image


# --------------------------------------------------------------------------
# thumb
# --------------------------------------------------------------------------

def load_thumb(path: Path):
    Image = _pillow()
    image = Image.open(path)
    image.load()
    return image.convert("RGB")


# --------------------------------------------------------------------------
# preview (JPEG-XL)
# --------------------------------------------------------------------------

def have_jxl() -> bool:
    try:
        import imagecodecs  # noqa: F401
        return True
    except ImportError:
        try:
            import pillow_jxl  # noqa: F401
            return True
        except ImportError:
            return False


def load_preview(path: Path, white_balance: str | None = None, highlight_percentile: float = 99.5):
    """Decode a .cop and turn it into something displayable.

    ``white_balance`` is ZVARIANTLAYER.ZWHITEBALANCE, 'R;G;B' multipliers with G = 1.
    Without it the result has a strong green cast.
    """
    Image = _pillow()
    try:
        import imagecodecs
        import numpy as np
    except ImportError as exc:
        raise ImageError(
            "decoding .cop previews needs imagecodecs and numpy:\n"
            "  python3 -m pip install imagecodecs numpy\n"
            "Until then use --tier thumb (450 px, no dependencies)."
        ) from exc

    frames = imagecodecs.jpegxl_decode(path.read_bytes())
    # Capture One writes two frames; the last one is the full-resolution image.
    data = np.asarray(frames[-1] if frames.ndim == 4 else frames, dtype=np.float32)

    if white_balance:
        try:
            multipliers = np.array([float(x) for x in white_balance.split(";")][:3], dtype=np.float32)
            if multipliers.size == 3:
                data = data * multipliers
        except ValueError:
            pass

    # The preview has no film curve applied, so normalise on a high percentile
    # rather than assuming 1.0 is white.
    ceiling = float(np.percentile(data, highlight_percentile))
    if ceiling > 1e-6:
        data = data / ceiling
    data = np.clip(data, 0.0, 1.0)

    # linear -> sRGB
    srgb = np.where(data <= 0.0031308, data * 12.92, 1.055 * np.power(data, 1 / 2.4) - 0.055)
    return Image.fromarray((np.clip(srgb, 0, 1) * 255).astype("uint8"), "RGB")


# --------------------------------------------------------------------------
# original
# --------------------------------------------------------------------------

def _raf_embedded_jpeg(path: Path) -> bytes | None:
    """Fuji RAF stores the offset and length of a full JPEG as two big-endian ints at 84."""
    with path.open("rb") as handle:
        header = handle.read(96)
        if not header.startswith(b"FUJIFILM"):
            return None
        offset, length = struct.unpack(">II", header[84:92])
        if not (0 < length < 64 * 2**20):
            return None
        handle.seek(offset)
        payload = handle.read(length)
    return payload if payload[:3] == b"\xff\xd8\xff" else None


_TIFF_PREVIEW_TAGS = {
    0x0201: "start",   # JPEGInterchangeFormat
    0x0202: "length",  # JPEGInterchangeFormatLength
    0x0111: "strips",  # StripOffsets
    0x0117: "counts",  # StripByteCounts
}
_TYPE_SIZE = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 9: 4, 10: 8, 11: 4, 12: 8}


def _tiff_previews(path: Path) -> list[tuple[int, int]]:
    """Walk TIFF/DNG/CR2 IFDs and collect (offset, length) of embedded JPEGs."""
    data = path.open("rb").read(1024 * 1024)
    if data[:2] not in (b"II", b"MM"):
        return []
    endian = "<" if data[:2] == b"II" else ">"
    (first,) = struct.unpack(endian + "I", data[4:8])

    found: list[tuple[int, int]] = []
    seen: set[int] = set()
    queue = [first]
    while queue:
        offset = queue.pop()
        if offset in seen or offset <= 0 or offset + 2 > len(data):
            continue
        seen.add(offset)
        (count,) = struct.unpack(endian + "H", data[offset:offset + 2])
        entries = {}
        for i in range(count):
            base = offset + 2 + i * 12
            if base + 12 > len(data):
                break
            tag, kind, number = struct.unpack(endian + "HHI", data[base:base + 8])
            payload = data[base + 8:base + 12]
            size = _TYPE_SIZE.get(kind, 0) * number
            if size > 4:
                (pointer,) = struct.unpack(endian + "I", payload)
                if pointer + 4 <= len(data):
                    (value,) = struct.unpack(endian + "I", data[pointer:pointer + 4])
                else:
                    continue
            else:
                (value,) = struct.unpack(endian + "I", payload) if kind == 4 else (
                    struct.unpack(endian + "H", payload[:2])[0],)
            if tag in _TIFF_PREVIEW_TAGS:
                entries[_TIFF_PREVIEW_TAGS[tag]] = value
            elif tag == 0x014A:  # SubIFDs
                queue.append(value)
        start = entries.get("start") or entries.get("strips")
        length = entries.get("length") or entries.get("counts")
        if start and length and length > 4096:
            found.append((start, length))
        end = offset + 2 + count * 12
        if end + 4 <= len(data):
            (nxt,) = struct.unpack(endian + "I", data[end:end + 4])
            if nxt:
                queue.append(nxt)
    return found


def load_original(path: Path):
    Image = _pillow()
    suffix = path.suffix.lower()
    if suffix in (".jpg", ".jpeg", ".png", ".tif", ".tiff", ".psd", ".webp"):
        image = Image.open(path)
        image.load()
        return image.convert("RGB")

    if suffix == ".raf":
        payload = _raf_embedded_jpeg(path)
        if payload:
            import io
            image = Image.open(io.BytesIO(payload))
            image.load()
            return image.convert("RGB")

    if suffix in (".dng", ".cr2", ".nef", ".arw"):
        import io
        best = None
        with path.open("rb") as handle:
            for start, length in sorted(_tiff_previews(path), key=lambda t: -t[1]):
                handle.seek(start)
                payload = handle.read(length)
                if payload[:3] != b"\xff\xd8\xff":
                    continue
                try:
                    image = Image.open(io.BytesIO(payload))
                    image.load()
                except Exception:  # noqa: BLE001 - a bad IFD entry is not fatal
                    continue
                if best is None or image.size[0] * image.size[1] > best.size[0] * best.size[1]:
                    best = image
                break
        if best is not None:
            return best.convert("RGB")

    raise ImageError(f"no decoder for {path.name} - use --tier thumb or --tier preview")


# --------------------------------------------------------------------------
# tier resolution
# --------------------------------------------------------------------------

def white_balance_for(con, variant_pk: int) -> str | None:
    """ZWHITEBALANCE from the variant's own layers, preferring the default layer."""
    row = con.execute(
        "SELECT l.Z_ENT, l.ZWHITEBALANCE FROM ZVARIANTLAYER l JOIN ZVARIANT v "
        "ON l.Z_PK IN (v.ZDEFAULTLAYER, v.ZCOMBINEDSETTINGS) "
        "WHERE v.Z_PK = ? AND l.ZWHITEBALANCE IS NOT NULL AND l.ZWHITEBALANCE <> '' "
        "ORDER BY CASE l.Z_ENT WHEN 18 THEN 0 ELSE 1 END LIMIT 1", (variant_pk,)).fetchone()
    return row[1] if row else None


def _resolve_preview(stored: str | None) -> Path | None:
    """The index stores the derived path; confirm it here, tolerating a case mismatch.

    ZSIDECARPATH is sometimes lower-cased relative to the real filename on disk.
    """
    if not stored:
        return None
    path = Path(stored)
    if path.exists():
        return path
    parent = path.parent
    if parent.is_dir():
        wanted = path.name.lower()
        for entry in parent.iterdir():
            if entry.name.lower() == wanted:
                return entry
    return None


def load(row, con=None, tier: str = "thumb", fallback: bool = True) -> tuple[object, str]:
    """Load a variant's pixels at the requested tier. Returns (image, tier_used)."""
    order = [tier]
    if fallback:
        order += [t for t in ("thumb", "preview", "original") if t != tier]

    errors = []
    for candidate in order:
        try:
            if candidate == "thumb":
                path = Path(row["thumb_path"]) if row["thumb_path"] else None
                if path and path.exists():
                    return load_thumb(path), "thumb"
            elif candidate == "preview":
                path = _resolve_preview(row["preview_path"])
                if path:
                    wb = white_balance_for(con, row["variant_pk"]) if con is not None else None
                    return load_preview(path, wb), "preview"
            else:
                path = Path(row["path_wsl"]) if row["path_wsl"] else None
                if path and path.exists():
                    return load_original(path), "original"
        except ImageError as exc:
            errors.append(f"{candidate}: {exc}")
        except Exception as exc:  # noqa: BLE001 - a corrupt cache file must not stop a batch
            errors.append(f"{candidate}: {type(exc).__name__}: {exc}")

    raise ImageError(f"no usable image for variant {row['variant_pk']} ({row['filename']})"
                     + ("; " + " | ".join(errors) if errors else ""))


def fit(image, long_edge: int):
    if not long_edge or max(image.size) <= long_edge:
        return image
    Image = _pillow()
    ratio = long_edge / max(image.size)
    size = (max(1, round(image.size[0] * ratio)), max(1, round(image.size[1] * ratio)))
    return image.resize(size, Image.Resampling.LANCZOS)
