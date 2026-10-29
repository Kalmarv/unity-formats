"""Colour properties of the shaders of named Materials, with UnityPy: each property's name and `m_Flags`.

Prints, for each named Material in one serialized file of a player, its shader's name and the colour properties
(`m_ParsedForm.m_PropInfo.m_Props`, `m_Type` 0) with their `m_Flags` (Unity's `ShaderPropertyFlags`; 16 is `HDR`), as
JSON lines. This is the independent-reader oracle for unity-formats' `program::colour_properties`:
tests/shader_properties.rs reads the same materials and states these values.

Usage:
    pip install UnityPy
    python tests/oracles/unitypy_shader_properties.py <Game>_Data <file> <material name>...
"""

from __future__ import annotations

import json
import os
import sys

import UnityPy


def main() -> None:
    data, file, names = sys.argv[1], sys.argv[2], set(sys.argv[3:])
    env = UnityPy.load(os.path.join(data, file))
    for obj in env.objects:
        if obj.type.name != "Material":
            continue
        material = obj.read()
        if material.m_Name not in names:
            continue
        shader = material.m_Shader.read().object_reader.read_typetree()
        form = shader["m_ParsedForm"]
        colours = [
            {"name": p["m_Name"], "flags": p["m_Flags"]} for p in form["m_PropInfo"]["m_Props"] if p["m_Type"] == 0
        ]
        print(json.dumps({"material": material.m_Name, "shader": form["m_Name"], "colours": colours}))


if __name__ == "__main__":
    main()
