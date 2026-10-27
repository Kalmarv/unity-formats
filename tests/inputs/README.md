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
| `scratch.root` | - | where real-data tests write their outputs (the OS temporary folder when unset) | - |

#### Oracle sets (`<oracles.root>/<set>/<game>/`)

| set | about | source |
| --- | --- | --- |
| `oracles.unitypy` | UnityPy's readings of a Unity player (the object-table census these tests use; tests/oracles/unitypy_census.py makes it) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py |

#### Declared files

| key | status | gate | about | source |
| --- | --- | --- | --- | --- |
| `installs.skatebird/SkateBIRD_Data` | live | yes | SkateBIRD's data folder (`SkateBIRD_Data`, a Unity 2019.4.20f1 player: SerializedFile version 21 containers), parsed by tests/serialized_census.rs; the SkateBIRD tests of primase read it too ([worlds/skatebird/tests/markers.rs](https://github.com/Kalmarv/primase/tree/main/worlds/skatebird/tests/markers.rs)) | the user's own install |
| `oracles.unitypy.skatebird/census.json` | live | yes | an independent UnityPy object-table census of that folder (JSON; its schema is the `Census` struct of tests/serialized_census.rs) | independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py |

<!-- end of real-data inputs -->
