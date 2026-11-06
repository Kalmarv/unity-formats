"""Every Texture2D of a Unity player with UnityPy: its format, size, levels and pixel bytes, and for a crunched texture
the SHA-256 of its first level decoded to RGBA.

Writes JSON: for each SerializedFile (keyed by its lower-cased name, as unity-formats' Player names it: a loose file's
name or a bundle node's path) and each Texture2D in it (keyed by path id): name, TextureFormat id, width, height, mip
count, the length of its pixel bytes (in the object, or the size of its streamed resource: UnityPy resolves a `.resS`
only in an environment of the whole folder) and, for TextureFormat 28 and 29, the
SHA-256 of UnityPy's decoding of the first level (texture2ddecoder's crunch decoder) as RGBA bytes, rows bottom first as
stored. This is the independent-reader oracle for unity-formats' textures: tests/textures.rs reads the same player and
compares.

Usage:
    pip install UnityPy
    python tests/oracles/unitypy_textures.py <Game>_Data [<bundle folder>] > <oracles root>/unitypy/<game>/textures.json

The player's loose SerializedFiles are the data folder's and its `Resources`' (as `Player::open` reads them); every file
under `<bundle folder>` (recursively) that is a UnityFS bundle is read too. `<oracles root>` is the `[oracles] root` of
the GAME_DATA file (tests/inputs/README.md).
"""

from __future__ import annotations

import hashlib
import json
import os
import sys

import UnityPy
from PIL import Image
from UnityPy.files import BundleFile, SerializedFile

CRUNCHED = {28, 29}


def candidates(data_dir: str, bundle_dir: str | None) -> list[str]:
    out = []
    for d in (data_dir, os.path.join(data_dir, "Resources")):
        if os.path.isdir(d):
            out += sorted(os.path.join(d, n) for n in os.listdir(d) if os.path.isfile(os.path.join(d, n)))
    if bundle_dir:
        for root, _, files in os.walk(bundle_dir):
            for n in sorted(files):
                p = os.path.join(root, n)
                with open(p, "rb") as f:
                    if f.read(8) == b"UnityFS\0":
                        out.append(p)
    return out


def serialized_files(env) -> list[tuple[str, SerializedFile]]:
    found = []

    def walk(f):
        for name, x in getattr(f, "files", {}).items():
            if isinstance(x, SerializedFile):
                found.append((os.path.basename(name).lower(), x))
            elif isinstance(x, BundleFile):
                walk(x)

    for name, f in env.files.items():
        if isinstance(f, SerializedFile):
            found.append((os.path.basename(name).lower(), f))
        else:
            walk(f)
    return found


def main() -> None:
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    data_dir = sys.argv[1]
    bundle_dir = sys.argv[2] if len(sys.argv) == 3 else None
    files: dict[str, dict] = {}
    for path in candidates(data_dir, bundle_dir):
        if path.endswith((".resS", ".resource")):
            continue
        try:
            env = UnityPy.load(path)
        except Exception:
            continue
        for name, sf in serialized_files(env):
            for path_id, obj in sf.objects.items():
                if obj.type.name != "Texture2D":
                    continue
                tex = obj.read()
                fmt = int(tex.m_TextureFormat)
                entry = {
                    "name": tex.m_Name,
                    "format": fmt,
                    "width": tex.m_Width,
                    "height": tex.m_Height,
                    "mips": tex.m_MipCount,
                    "bytes": len(tex.image_data) or (tex.m_StreamData.size if tex.m_StreamData else 0),
                }
                if fmt in CRUNCHED:
                    image = tex.image.convert("RGBA").transpose(Image.FLIP_TOP_BOTTOM)
                    entry["rgba_sha256"] = hashlib.sha256(image.tobytes()).hexdigest()
                files.setdefault(name, {})[str(path_id)] = entry
    json.dump({"reader": {"library": "UnityPy", "version": UnityPy.__version__}, "files": files}, sys.stdout, indent=1)


if __name__ == "__main__":
    main()
