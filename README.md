# unity-formats

A Rust library that reads the data files of Unity games: the SerializedFiles a built player keeps its scenes and assets in, and the meshes, textures, materials, shader programs, terrains and animation data inside them.

It reads SerializedFile containers (versions 21 and 22), decodes their objects through a type tree package (a TPK file you supply, because a player build strips the type trees), and follows pointers across a player's files and streamed `.resS` data. On top of that it reads scenes (GameObjects, Transforms composed to world matrices, renderers, lights, cameras, colliders, terrains, ambient and fog settings), meshes (vertex streams, compressed meshes, skins, blend shapes), `Texture2D` pixels and mip chains, materials, a Shader's render state and compiled program variants, terrains, Avatars, AnimatorControllers and AnimationClips, and the lighting settings of a Universal Render Pipeline asset. It was written against Unity 2019.4 and Unity 6 players: SkateBIRD (2019.4.20f1), Sledding Game (6000.3.12f1) and PEAK.

## Example

```rust,no_run
use std::path::Path;
use unity_formats::{TypeTreePackage, listing::list, mesh::UnityMeshData, player::Player};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // a type tree package for the game's Unity version, e.g. the one AssetRipper publishes
    let package = TypeTreePackage::parse(&std::fs::read("typetrees.tpk")?)?;
    // the game's `<Game>_Data` folder
    let player = Player::open(Path::new("Game_Data"), package)?;
    for scene in player.scenes()? {
        println!("{}: {}", scene.file, scene.name);
    }
    for entry in list(&player, Some(&["Mesh"]))?.entries.iter().take(3) {
        let mesh = UnityMeshData::read(&player, &entry.object)?;
        println!("{}: {} vertices", mesh.name, mesh.vertex_count);
    }
    Ok(())
}
```

`cargo run --example inspect_player -- <Game_Data folder> <typetrees.tpk>` is a runnable version of it. The API docs are on [docs.rs](https://docs.rs/unity-formats) (or `cargo doc --open`).

## What it guarantees

- Reads are bounds-checked, and a file it does not understand is refused with an `Error` that says what it found, for example `SerializedFile version 19 (read: 21..=22)`, `header file size 9758532, file has 30 bytes` or `<n> bytes wanted at offset <p>, <m> left`. Only SerializedFile versions checked against real files are read; anything else is `Error::Unsupported`, never a guess.
- Library code denies, by lint, panicking indexing, unchecked arithmetic and truncating casts, so a malformed file is an `Error` and not a panic. The test suite checks truncated headers and unsupported versions on synthetic bytes; it does not fuzz.
- What a reader returns is what the file stores: values keep their stored units and Unity's coordinate frame (no handedness change, resampling or colour-space change), and pixels stay as stored. Where a module substitutes a default for a field the file may omit, its docs say so.
- The library never writes to disk. It reads only the files in the folder it is given (a player's data folder and its `Resources` folder) and the `.resS` files its objects name there; a streamed resource's path is reduced to a file name in that folder. `unsafe` code is denied.

## What it does not do

- It reads: nothing here writes SerializedFiles, bundles or any other Unity format.
- It does not read AssetBundles (UnityFS), Addressables catalogs or SerializedFiles of other container versions than 21 and 22, nor files with embedded type trees.
- It ships no type tree package: LZMA- and Brotli-compressed packages are refused (re-pack them uncompressed or LZ4).
- A MonoBehaviour's script fields are returned as bytes, and `[SerializeReference]` managed references are refused.
- Textures: eleven formats are named (Alpha8, ARGB4444, RGB24, RGBA32, DXT1, DXT5, BC4, BC5, BC6H, BC7, R8); crunched formats are refused, and no format is decoded to RGBA, only split into mip levels.
- Meshes with compressed skin weights, legacy AnimationClips and clips with object-reference curves are refused.
- It does not run, render or simulate anything: animation data, shaders and materials are read as stored, and what they mean is the caller's.
- It contains no game files, and no tests need you to share any.

## Running the tests

```sh
cargo test --workspace
```

runs the unit tests and the tests on synthetic bytes. The tests that read real games skip with a line naming what they need when `GAME_DATA` is not set. To run them, point the `GAME_DATA` environment variable at a TOML file naming your own copies, in the layout of [primase-testdata](https://github.com/Kalmarv/primase-testdata):

```toml
[installs]
skatebird = 'D:/games/SkateBIRD'        # the folder that holds SkateBIRD_Data
[oracles]
root = 'D:/oracles'                     # <root>/unitypy/skatebird/census.json
[run]
require = 'gate'                        # optional: a missing input fails instead of skipping
```

The one real-data test, `tests/serialized_census.rs`, parses every SerializedFile in SkateBIRD's data folder and compares each file's object table (path ids, class ids, offsets, sizes) with a census made by [UnityPy](https://github.com/K0lb3/UnityPy). Make the census from your own install with `pip install UnityPy` and `python tests/oracles/unitypy_census.py <SkateBIRD_Data folder> <oracles root>/unitypy/skatebird/census.json`. [tests/inputs/README.md](tests/inputs/README.md) lists every input the tests can read (generated from `tests/inputs/mod.rs`).

CONTRIBUTING.md says how to build, and how to report a file this library cannot read.

The asset-conversion library [primase](https://github.com/Kalmarv/primase) is built on this crate.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
