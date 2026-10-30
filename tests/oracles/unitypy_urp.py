"""The Universal Render Pipeline objects of a Unity player with UnityPy: its pipeline assets, their renderers, the
renderers' features and the cameras' URP data.

Writes JSON: for every quality level's pipeline asset (QualitySettings' `customRenderPipeline`, else GraphicsSettings'
`m_CustomRenderPipeline`) its renderer list, default renderer index, depth and opaque texture settings and MSAA; for
every renderer (`UniversalRendererData`) its features, feature map, masks, stencil state and modes; for every feature
its class, `m_Active` and, for the Decal and Full Screen Pass features, their settings; and for every
`UniversalAdditionalCameraData` in the player's scene files (`level<n>`, loose or in `data.unity3d` for a player whose
files are one bundle) its fields. This is the independent-reader oracle for
unity-formats' `pipeline` readers: tests/urp.rs reads the same player and compares.

A build stores no type tree for a script's fields, so UnityPy hands them over as bytes and this script decodes them in
the declaration order of the Universal RP 17.3 classes (each bool padded to 4 bytes, a reference 12 bytes, a list its
count and elements), checking that each object's bytes are used up exactly; an object whose length does not fit is
reported with `"layout": "unread"` and its length.

Usage:
    pip install UnityPy
    python tests/oracles/unitypy_urp.py <Game>_Data <oracles root>/unitypy/<game>/urp.json

`<oracles root>` is the `[oracles] root` of the GAME_DATA file (tests/inputs/README.md).
"""

from __future__ import annotations

import json
import os
import re
import struct
import sys

import UnityPy


class Cursor:
    """Little-endian reads over a script's bytes."""

    def __init__(self, data: bytes):
        self.data = data
        self.pos = 0

    def take(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise ValueError(f"{n} bytes wanted at {self.pos} of {len(self.data)}")
        b = self.data[self.pos : self.pos + n]
        self.pos += n
        return b

    def i32(self) -> int:
        return struct.unpack("<i", self.take(4))[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.take(4))[0]

    def i64(self) -> int:
        return struct.unpack("<q", self.take(8))[0]

    def f32(self) -> float:
        return struct.unpack("<f", self.take(4))[0]

    def flag(self) -> bool:
        v = self.i32()
        if v not in (0, 1):
            raise ValueError(f"bool {v} at {self.pos - 4}")
        return bool(v)

    def pptr(self) -> list:
        return [self.i32(), self.i64()]

    def left(self) -> int:
        return len(self.data) - self.pos


def script_bytes(obj) -> tuple[bytes, tuple[int, int]]:
    """A MonoBehaviour's script fields and its m_Script reference (the base fields of a Unity 6 MonoBehaviour: the
    GameObject reference, m_Enabled padded, m_Script, m_Name)."""
    raw = obj.get_raw_data()
    sfid, spid = struct.unpack_from("<iq", raw, 16)
    n = struct.unpack_from("<i", raw, 28)[0]
    at = (32 + n + 3) & ~3
    return raw[at:], (sfid, spid)


class Files:
    """The player's serialized files, loaded once each, and the class names of the scripts they reference."""

    def __init__(self, data_dir: str):
        self.dir = data_dir
        self.loaded: dict[str, object] = {}
        # a player whose files are one bundle (Big Walk's `data.unity3d`) is read through it
        self.bundled = not os.path.exists(os.path.join(data_dir, "globalgamemanagers"))
        if self.bundled:
            bundle = next(iter(UnityPy.load(os.path.join(data_dir, "data.unity3d")).files.values()))
            for name, f in bundle.files.items():
                if hasattr(f, "objects"):
                    self.loaded[name.lower()] = f

    def scenes(self) -> list[str]:
        """The player's scene files (`level<n>`), in build order."""
        names = self.loaded if self.bundled else os.listdir(self.dir)
        return sorted((f for f in names if re.fullmatch(r"level\d+", f)), key=lambda f: int(f[5:]))

    def file(self, name: str):
        name = os.path.basename(name).lower()
        if name not in self.loaded:
            path = next(
                os.path.join(self.dir, f) for f in os.listdir(self.dir) if f.lower() == name
            )
            self.loaded[name] = next(iter(UnityPy.load(path).files.values()))
        return self.loaded[name]

    def target(self, sf, fid: int):
        if fid == 0:
            return sf
        return self.file(sf.externals[fid - 1].name)

    def name_of(self, sf) -> str:
        for k, v in self.loaded.items():
            if v is sf:
                return k
        raise KeyError("file not loaded")

    def script_class(self, sf, ref: tuple[int, int]) -> str:
        s = self.target(sf, ref[0]).objects[ref[1]].read()
        return f"{s.m_Namespace}.{s.m_ClassName}" if s.m_Namespace else s.m_ClassName


def urp_asset(b: bytes) -> dict:
    """The asset fields before m_OpaqueDownsampling (asset version 13)."""
    c = Cursor(b)
    out = {"version": c.i32()}
    c.i32()  # k_AssetPreviousVersion
    c.i32()  # m_RendererType
    c.pptr()  # m_RendererData
    out["renderers"] = [c.pptr() for _ in range(c.i32())]
    out["default_renderer"] = c.i32()
    out["require_depth_texture"] = c.flag()
    out["require_opaque_texture"] = c.flag()
    c.i32()  # m_OpaqueDownsampling
    c.flag()  # m_SupportsTerrainHoles
    c.flag()  # m_SupportsHDR
    c.i32()  # m_HDRColorBufferPrecision
    out["msaa"] = c.i32()
    return out


# `StencilStateData`'s fields, in declaration order (a dict comprehension reads them in this order)
STENCIL = ("override_stencil", "reference", "compare", "pass", "fail", "z_fail")


def renderer(b: bytes) -> dict:
    c = Cursor(b)
    c.take(12 * 2 + 12 * 7)  # debugShaders, probeVolumeResources
    out = {"features": [c.pptr() for _ in range(c.i32())]}
    out["feature_map"] = [c.i64() for _ in range(c.i32())]
    out["native_render_pass"] = c.flag()
    if c.left() == 96:
        out["xr_system_data"] = c.pptr()
    elif c.left() != 84:
        return {"layout": "unread", "length": len(b)}
    out["post_process_data"] = c.pptr()
    out["asset_version"] = c.i32()
    out["prepass_layer_mask"] = c.u32()
    out["opaque_layer_mask"] = c.u32()
    out["transparent_layer_mask"] = c.u32()
    out["stencil"] = {k: (c.flag() if k == "override_stencil" else c.i32()) for k in STENCIL}
    out["shadow_transparent_receive"] = c.flag()
    out["rendering_mode"] = c.i32()
    out["depth_priming_mode"] = c.i32()
    out["copy_depth_mode"] = c.i32()
    out["depth_attachment_format"] = c.i32()
    out["depth_texture_format"] = c.i32()
    out["accurate_gbuffer_normals"] = c.flag()
    out["intermediate_texture_mode"] = c.i32()
    assert c.left() == 0
    return out


def feature(cls: str, b: bytes) -> dict:
    c = Cursor(b)
    out = {"class": cls, "active": c.flag()}
    if cls == "UnityEngine.Rendering.Universal.DecalRendererFeature":
        if len(b) != 24:
            return {**out, "layout": "unread", "length": len(b)}
        out["decal"] = {
            "technique": c.i32(),
            "max_draw_distance": c.f32(),
            "decal_layers": c.flag(),
            "dbuffer_surface_data": c.i32(),
            "screen_space_normal_blend": c.i32(),
        }
    elif cls == "UnityEngine.Rendering.Universal.FullScreenPassRendererFeature":
        if len(b) != 40:
            return {**out, "layout": "unread", "length": len(b)}
        out["full_screen"] = {
            "injection_point": c.i32(),
            "fetch_color_buffer": c.flag(),
            "requirements": c.i32(),
            "pass_material": c.pptr(),
            "pass_index": c.i32(),
            "bind_depth_stencil_attachment": c.flag(),
            "version": c.i32(),
        }
    return out


def camera(b: bytes) -> dict:
    c = Cursor(b)
    out = {
        "render_shadows": c.flag(),
        "depth_texture_option": c.i32(),
        "opaque_texture_option": c.i32(),
        "camera_type": c.i32(),
        "stack": [c.pptr() for _ in range(c.i32())],
        "renderer_index": c.i32(),
        "volume_layer_mask": c.u32(),
        "volume_trigger": c.pptr(),
        "volume_update_mode": c.i32(),
        "render_post_processing": c.flag(),
        "antialiasing": c.i32(),
        "antialiasing_quality": c.i32(),
        "stop_nan": c.flag(),
        "dithering": c.flag(),
        "clear_depth": c.flag(),
        "allow_xr_rendering": c.flag(),
        "allow_hdr_output": c.flag(),
        "use_screen_coord_override": c.flag(),
        "screen_size_override": [c.f32() for _ in range(4)],
        "screen_coord_scale_bias": [c.f32() for _ in range(4)],
        "requires_depth_texture": c.flag(),
        "requires_color_texture": c.flag(),
        "taa": {
            "quality": c.i32(),
            "frame_influence": c.f32(),
            "jitter_scale": c.f32(),
            "mip_bias": c.f32(),
            "variance_clamp_scale": c.f32(),
            "contrast_adaptive_sharpening": c.f32(),
        },
        "version": c.i32(),
    }
    if c.left() != 0:
        return {"layout": "unread", "length": len(b)}
    return out


def main(data_dir: str, out_path: str) -> None:
    files = Files(data_dir)
    ggm = files.file("globalgamemanagers")
    graphics = next(o for o in ggm.objects.values() if o.type.name == "GraphicsSettings").read_typetree()
    quality = next(o for o in ggm.objects.values() if o.type.name == "QualitySettings").read_typetree()
    default = graphics["m_CustomRenderPipeline"]
    levels = [
        q["customRenderPipeline"] if q["customRenderPipeline"]["m_PathID"] else default
        for q in quality["m_QualitySettings"]
    ] or [default]

    assets, renderers, features = {}, {}, {}
    for level in levels:
        if not level["m_PathID"]:
            continue
        sf = files.target(ggm, level["m_FileID"])
        key = f"{files.name_of(sf)}/{level['m_PathID']}"
        if key in assets:
            continue
        b, script = script_bytes(sf.objects[level["m_PathID"]])
        if files.script_class(sf, script) != "UnityEngine.Rendering.Universal.UniversalRenderPipelineAsset":
            continue
        a = urp_asset(b)
        assets[key] = a
        for fid, pid in a["renderers"]:
            rf = files.target(sf, fid)
            rkey = f"{files.name_of(rf)}/{pid}"
            rb, rscript = script_bytes(rf.objects[pid])
            r = renderer(rb)
            r["class"] = files.script_class(rf, rscript)
            renderers[rkey] = r
            for ffid, fpid in r.get("features", []):
                ff = files.target(rf, ffid)
                fb, fscript = script_bytes(ff.objects[fpid])
                features[f"{files.name_of(ff)}/{fpid}"] = feature(files.script_class(ff, fscript), fb)

    cameras = {}
    for scene in files.scenes():
        sf = files.file(scene)
        for o in sf.objects.values():
            if o.type.name != "MonoBehaviour":
                continue
            b, script = script_bytes(o)
            try:
                cls = files.script_class(sf, script)
            except (KeyError, IndexError):
                continue
            if cls == "UnityEngine.Rendering.Universal.UniversalAdditionalCameraData":
                cameras[f"{scene}/{o.path_id}"] = camera(b)
        # a loose scene's objects are not needed once its cameras are read
        if not files.bundled:
            files.loaded.pop(scene, None)

    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(
            {"assets": assets, "renderers": renderers, "features": features, "cameras": cameras},
            f,
            indent=1,
            sort_keys=True,
        )


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
