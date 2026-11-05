# Running the tests

```sh
cargo test --workspace
```

This runs the unit tests and the tests on synthetic bytes. The tests that read real games are skipped, with a line naming what they need, when `GAME_DATA` is not set. To run them, set the `GAME_DATA` environment variable to a TOML file that names your own copies of the games, in the layout of [primase-testdata](https://github.com/Kalmarv/primase-testdata):

```toml
[installs]
skatebird = 'D:/games/SkateBIRD'        # the folder that holds SkateBIRD_Data
[oracles]
root = 'D:/oracles'                     # <root>/unitypy/skatebird/census.json
[run]
require = 'gate'                        # optional: a missing input fails instead of skipping
```

[tests/inputs/README.md](../tests/inputs/README.md) lists every input the tests can read (generated from `tests/inputs/mod.rs`).

The real-data tests compare this crate's reads with independent readings made by [UnityPy](https://github.com/K0lb3/UnityPy). The scripts in `tests/oracles` produce those readings from your own installs (`pip install UnityPy`, plus `lz4` for the bundle census):

| Test | Compares | Oracle |
|---|---|---|
| `tests/serialized_census.rs` | every SerializedFile's object table in SkateBIRD's data folder | `python tests/oracles/unitypy_census.py <SkateBIRD_Data> <root>/unitypy/skatebird/census.json` |
| `tests/bundles.rs` | every UnityFS bundle of PEAK, Sledding Game and Big Walk: node bytes, headers, types, stored trees, object tables, externals | `python tests/oracles/unitypy_bundles.py <bundle>... > <root>/unitypy/<game>/bundles.jsonl` |
| `tests/player_settings.rs` | `Player::colour_space` | `python tests/oracles/unitypy_player_settings.py <Game>_Data <root>/unitypy/<game>/player-settings.json` |
| `tests/shader_properties.rs` | `program::colour_properties` | `python tests/oracles/unitypy_shader_properties.py <Game>_Data <file> <material>...` |
| `tests/urp.rs` | the Universal Render Pipeline readers | `python tests/oracles/unitypy_urp.py <Game>_Data <root>/unitypy/<game>/urp.json` |

`tests/bundles.rs` also decodes every object of the bundles by the type tree its file stores; the census takes minutes, and the decoding about ten, with 4 threads. `cargo run --release --example bundle_census -- <type tree package> <bundle>...` prints the same census for one bundle.

## Other checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The code blocks in `README.md` are compiled as doc tests (`cargo test -p unity-formats --doc`).
