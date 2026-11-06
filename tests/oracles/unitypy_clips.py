"""Every legacy AnimationClip of a Unity player with UnityPy: its curves, by kind, each with the SHA-256 of its keys;
and the object curves of its Mecanim clips.

Writes JSON: for each SerializedFile (keyed by its lower-cased name, as unity-formats' Player names it: a loose file's
name or a bundle node's path) and each AnimationClip in it whose `m_Legacy` is set (keyed by path id): name, wrap mode,
event count and its curves in the order unity-formats' `ClipData` gives its tracks (`m_RotationCurves`,
`m_EulerCurves`, `m_PositionCurves`, `m_ScaleCurves`, `m_FloatCurves`, each in stored order). A curve is its kind, its
path (and for a float curve its attribute and class id), its Euler rotation order, the number of keys and the SHA-256
of its keys: for each component in order (x, y, z, w; one for a float), for each key, little-endian f32 time, value,
inSlope, outSlope, then the in weight when `weightedMode` has bit 1 (else 0), the out weight when it has bit 2 (else 0),
and one byte of those two bits. Under `mecanim`, likewise keyed, each Mecanim clip with an object (`isPPtrCurve`)
binding: its name, the objects `m_ClipBindingConstant.pptrCurveMapping` lists (file id and path id) and its object
bindings (path hash, attribute, class id), in binding order. tests/clips.rs reads the same player and compares.

Usage:
    pip install UnityPy
    python tests/oracles/unitypy_clips.py <Game>_Data [<bundle folder>] > <oracles root>/unitypy/<game>/clips.json

The player's loose SerializedFiles are the data folder's and its `Resources`' (as `Player::open` reads them); every file
under `<bundle folder>` (recursively) that is a UnityFS bundle is read too. `<oracles root>` is the `[oracles] root` of
the GAME_DATA file (tests/inputs/README.md).
"""

from __future__ import annotations

import hashlib
import json
import os
import struct
import sys

import UnityPy
from UnityPy.files import BundleFile, SerializedFile

KINDS = (
    ("m_RotationCurves", "rotation", "xyzw"),
    ("m_EulerCurves", "euler", "xyz"),
    ("m_PositionCurves", "position", "xyz"),
    ("m_ScaleCurves", "scale", "xyz"),
    ("m_FloatCurves", "float", None),
)


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


def keys_sha(keys: list[dict], components: str | None) -> str:
    h = hashlib.sha256()
    for c in components or [None]:
        pick = (lambda v: v) if c is None else (lambda v, c=c: v[c])
        for k in keys:
            mode = int(k.get("weightedMode", 0))
            in_w = pick(k["inWeight"]) if mode & 1 else 0.0
            out_w = pick(k["outWeight"]) if mode & 2 else 0.0
            h.update(
                struct.pack(
                    "<6fB",
                    k["time"],
                    pick(k["value"]),
                    pick(k["inSlope"]),
                    pick(k["outSlope"]),
                    in_w,
                    out_w,
                    mode & 3,
                )
            )
    return h.hexdigest()


def main() -> None:
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    data_dir = sys.argv[1]
    bundle_dir = sys.argv[2] if len(sys.argv) == 3 else None
    files: dict[str, dict] = {}
    mecanim: dict[str, dict] = {}
    for path in candidates(data_dir, bundle_dir):
        if path.endswith((".resS", ".resource")):
            continue
        try:
            env = UnityPy.load(path)
        except Exception:
            continue
        for name, sf in serialized_files(env):
            for path_id, obj in sf.objects.items():
                if obj.type.name != "AnimationClip":
                    continue
                d = obj.read_typetree()
                if not d.get("m_Legacy"):
                    bc = d["m_ClipBindingConstant"]
                    pptr = [b for b in bc["genericBindings"] if b.get("isPPtrCurve")]
                    if pptr:
                        mecanim.setdefault(name, {})[str(path_id)] = {
                            "name": d["m_Name"],
                            "targets": [[p["m_FileID"], p["m_PathID"]] for p in bc["pptrCurveMapping"]],
                            "bindings": [[b["path"], b["attribute"], b["typeID"]] for b in pptr],
                        }
                    continue
                curves = []
                for field, kind, components in KINDS:
                    for cv in d.get(field) or []:
                        keys = cv["curve"]["m_Curve"]
                        entry = {"kind": kind, "path": cv["path"]}
                        if kind == "float":
                            entry["attribute"] = cv["attribute"]
                            entry["class_id"] = cv["classID"]
                        entry["rotation_order"] = cv["curve"].get("m_RotationOrder", 4)
                        entry["keys"] = len(keys)
                        entry["sha256"] = keys_sha(keys, components)
                        curves.append(entry)
                files.setdefault(name, {})[str(path_id)] = {
                    "name": d["m_Name"],
                    "wrap_mode": d.get("m_WrapMode", 0),
                    "events": len(d.get("m_Events") or []),
                    "compressed": bool(d.get("m_Compressed")),
                    "compressed_rotation_curves": len(d.get("m_CompressedRotationCurves") or []),
                    "pptr_curves": len(d.get("m_PPtrCurves") or []),
                    "curves": curves,
                }
    json.dump(
        {"reader": {"library": "UnityPy", "version": UnityPy.__version__}, "files": files, "mecanim": mecanim},
        sys.stdout,
        indent=1,
    )


if __name__ == "__main__":
    main()
