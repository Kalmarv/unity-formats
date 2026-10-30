# Real-data inputs

What this crate's real-data tests read, as tests/inputs/mod.rs declares them: the keys of the configuration file
`GAME_DATA` names (its layout is that of [primase-testdata](https://github.com/Kalmarv/primase-testdata), which the
tests use), the oracle sets and the files of each, and the debug keys `GAME_DATA_DEBUG` sets. A test whose input is not
configured skips with a line naming its key; the "gate" column marks the inputs that fail instead of skipping when the
file says `[run] require = 'gate'`. The tables below are generated from the declarations (the `real_data_inputs` test
fails when they differ); [the README](../../README.md#running-the-tests) says how to set the inputs up.

<!-- real-data inputs: made by primase_testdata::doc::tables from tests/inputs/mod.rs -->

#### Configuration keys

| key | gate | about | source |
| --- | --- | --- | --- |
| `installs.skatebird` | yes | SkateBIRD's install folder (`SkateBIRD_Data`: a Unity 2019.4.20f1 player) | the user's own install |
| `installs.peak` | yes | PEAK's install folder (a Unity 6000.3.15f1 player: loose files and level AssetBundles) | the user's own install |
| `installs.sledding` | yes | Sledding Game's install folder (a Unity 6000.3.12f1 player with Addressables bundles) | the user's own install |
| `installs.bigwalk` | yes | Big Walk's install folder (a Unity 6000.3.17f1 player in one `data.unity3d`) | the user's own install |
| `tools.unity_tpk` | yes | a type tree package (TPK, uncompressed or LZ4) with the class layouts and the common strings of the Unity versions read | input data: class layouts for the players' Unity versions |
| `scratch.root` | - | where real-data tests write their outputs (the OS temporary folder when unset) | - |

#### Oracle sets (`<oracles.root>/<set>/<game>/`)

| set | about | source |
| --- | --- | --- |
| `oracles.unitypy` | UnityPy's readings of a Unity player (the object-table census these tests use; tests/oracles/unitypy_census.py makes it) and of its bundles (tests/oracles/unitypy_bundles.py) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by the scripts in tests/oracles |

#### Declared files

| key | status | gate | about | source |
| --- | --- | --- | --- | --- |
| `installs.skatebird/SkateBIRD_Data` | live | yes | SkateBIRD's data folder (`SkateBIRD_Data`, a Unity 2019.4.20f1 player: SerializedFile version 21 containers), parsed by tests/serialized_census.rs; the SkateBIRD tests of primase read it too ([worlds/skatebird/tests/markers.rs](https://github.com/Kalmarv/primase/tree/main/worlds/skatebird/tests/markers.rs)) | the user's own install |
| `oracles.unitypy.skatebird/census.json` | live | yes | an independent UnityPy object-table census of that folder (JSON; its schema is the `Census` struct of tests/serialized_census.rs) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py |
| `installs.peak/PEAK_Data` | live | yes | PEAK's data folder (`PEAK_Data`: the loose player files and `StreamingAssets/maps`, the level AssetBundles), whose level 207 tests/bundles.rs reads with its bundle | the user's own install |
| `installs.sledding/Sledding Game_Data` | live | yes | Sledding Game's data folder (`Sledding Game_Data`: nine loose serialized files, two of them in `Resources`, and the Addressables bundles in `StreamingAssets/aa`), read by tests/bundles.rs | the user's own install |
| `installs.peak/PEAK_Data/StreamingAssets/maps` | live | yes | PEAK's AssetBundles (`PEAK_Data/StreamingAssets/maps`: seven `levels*.assetbundle` of 208 level scenes and the `maps` manifest bundle), read by tests/bundles.rs | the user's own install |
| `installs.sledding/Sledding Game_Data/StreamingAssets/aa/StandaloneWindows64` | live | yes | Sledding Game's 50 Addressables bundles (`StreamingAssets/aa/StandaloneWindows64`), read by tests/bundles.rs | the user's own install |
| `installs.bigwalk/Big Walk_Data` | live | yes | Big Walk's data folder (`Big Walk_Data`, whose `data.unity3d` is the player's own files in one bundle), read by tests/bundles.rs | the user's own install |
| `oracles.unitypy.peak/bundles.jsonl` | live | yes | an independent census of PEAK's bundles (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader |
| `oracles.unitypy.sledding/bundles.jsonl` | live | yes | an independent census of Sledding Game's bundles (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader |
| `oracles.unitypy.bigwalk/bundles.jsonl` | live | yes | an independent census of Big Walk's `data.unity3d` (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader |
| `oracles.unitypy.skatebird/player-settings.json` | live | yes | the colour space SkateBIRD's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py |
| `oracles.unitypy.sledding/player-settings.json` | live | yes | the colour space Sledding Game's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py |
| `oracles.unitypy.peak/player-settings.json` | live | yes | the colour space PEAK's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py |
| `oracles.unitypy.bigwalk/player-settings.json` | live | yes | the colour space Big Walk's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py |
| `oracles.unitypy.peak/urp.json` | live | yes | PEAK's Universal Render Pipeline objects: its pipeline asset's renderers and settings, the renderers' fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py |
| `oracles.unitypy.sledding/urp.json` | live | yes | Sledding Game's Universal Render Pipeline objects: its pipeline assets' renderers and settings, the renderer's fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py |
| `oracles.unitypy.bigwalk/urp.json` | live | yes | Big Walk's Universal Render Pipeline objects, read from its `data.unity3d`: its pipeline assets' renderers and settings, the renderers' fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares) | independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py |

<!-- end of real-data inputs -->
