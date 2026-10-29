"""PlayerSettings of a Unity player with UnityPy: the colour space the player renders in.

Writes the `m_ActiveColorSpace` of the player's PlayerSettings (in `globalgamemanagers`, or in `data.unity3d` for a
player whose files are one bundle) as JSON. This is the independent-reader oracle for unity-formats'
`Player::colour_space`: tests/player_settings.rs reads the same player and compares.

UnityPy reads a stripped player with its own class layouts; where a layout reads short at the end of PlayerSettings it
is read again unchecked (the colour space is near its start) and the JSON says so in `unchecked`.

Usage:
    pip install UnityPy
    python tests/oracles/unitypy_player_settings.py <Game>_Data <oracles root>/unitypy/<game>/player-settings.json

`<oracles root>` is the `[oracles] root` of the GAME_DATA file (tests/inputs/README.md).
"""

from __future__ import annotations

import json
import os
import sys

import UnityPy


def player_settings(data_dir: str) -> dict:
    """The PlayerSettings fields this oracle reports, and how they were read."""
    loose = os.path.join(data_dir, "globalgamemanagers")
    source = loose if os.path.exists(loose) else os.path.join(data_dir, "data.unity3d")
    env = UnityPy.load(source)
    found = []
    for obj in env.objects:
        if obj.type.name != "PlayerSettings":
            continue
        try:
            tree, unchecked = obj.read_typetree(), None
        except ValueError as error:
            tree, unchecked = obj.read_typetree(check_read=False), str(error)
        found.append(
            {
                "file": obj.assets_file.name,
                "m_ActiveColorSpace": tree["m_ActiveColorSpace"],
                "unchecked": unchecked,
            }
        )
    if len(found) != 1:
        raise SystemExit(f"{source}: {len(found)} PlayerSettings, one expected")
    return {"source": os.path.basename(source), "unitypy": UnityPy.__version__, **found[0]}


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    data_dir, out = sys.argv[1], sys.argv[2]
    result = player_settings(data_dir)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(result, f, indent=1)
        f.write("\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
