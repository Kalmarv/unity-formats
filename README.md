# unity-formats

A Rust library that reads the data files of Unity games: the SerializedFiles a built player keeps its scenes and assets in, and the meshes, textures, materials, shader programs, terrains and animation data inside them.

It reads SerializedFile containers (versions 21 and 22) and UnityFS bundles (AssetBundles, Addressables content, a player's `data.unity3d`), decodes their objects through the type trees a file stores or, for a player build that strips them, through a type tree package (a TPK file you supply), and follows pointers across a player's files and streamed `.resS` data, loose or bundled. On top of that it reads scenes (GameObjects, Transforms composed to world matrices, renderers, lights, cameras, colliders, terrains, ambient and fog settings), meshes (vertex streams, compressed meshes, skins, blend shapes), `Texture2D` pixels and mip chains, materials, a Shader's render state and compiled program variants, terrains, Avatars, AnimatorControllers and AnimationClips, and the Universal Render Pipeline's objects: a pipeline asset's renderers and settings, a renderer's features, layer masks and modes, the Decal and Full Screen Pass features' settings, a camera's URP data, and the pipeline's global settings with their graphics settings (its runtime shaders, whether it draws through the render graph). It was written against Unity 2019.4 and Unity 6 players: SkateBIRD (2019.4.20f1), Sledding Game (6000.3.12f1), PEAK (6000.3.15f1) and Big Walk (6000.3.17f1).

## Example

```rust,no_run
use std::path::Path;
use unity_formats::pipeline::read_urp_renderer;
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
    // the Universal Render Pipeline asset every quality level draws with, and the renderer its cameras use by default
    if let Some(asset) = player.urp_asset(None)? {
        let settings = player.urp_settings(&asset)?;
        let default = settings.renderers.get(usize::try_from(settings.default_renderer)?);
        if let Some(r) = default.map(|p| player.resolve_pptr(&asset.file, *p)).transpose()?.flatten() {
            let renderer = read_urp_renderer(&player.behaviour(&r)?.1)?;
            println!("rendering mode {}, {} features", renderer.rendering_mode, renderer.features.len());
        }
    }
    Ok(())
}
```

`cargo run --example inspect_player -- <Game_Data folder> <typetrees.tpk>` is a runnable version of it. The API docs are on [docs.rs](https://docs.rs/unity-formats) (or `cargo doc --open`).

## What it guarantees

- Reads are bounds-checked, and a file it does not understand is refused with an `Error` that says what it found, for example `SerializedFile version 19 (read: 21..=22)`, `header file size 9758532, file has 30 bytes` or `<n> bytes wanted at offset <p>, <m> left`. Only SerializedFile versions checked against real files are read; anything else is `Error::Unsupported`, never a guess.
- Library code denies, by lint, panicking indexing, unchecked arithmetic and truncating casts, so a malformed file is an `Error` and not a panic. The test suite checks truncated headers and unsupported versions on synthetic bytes; it does not fuzz.
- What a reader returns is what the file stores: values keep their stored units and Unity's coordinate frame (no handedness change, resampling or colour-space change), and pixels stay as stored. Where a module substitutes a default for a field the file may omit, its docs say so.
- The library never writes to disk. It reads only the files in the folder it is given (a player's data folder and its `Resources` folder), the bundles it is told to open and the `.resS` files its objects name there (or the stream nodes of the bundle they are in); a streamed resource's path is reduced to a file name in that folder, or to a node of the bundle. `unsafe` code is denied.

## What it does not do

- It reads: nothing here writes SerializedFiles, bundles or any other Unity format.
- It reads UnityFS bundles of format 8 with the header flags `0x243` (a block table stored with the directory, LZ4 or LZ4HC) whose blocks are stored or LZ4/LZ4HC; bundles with LZMA or LZHAM blocks, other format versions and Addressables catalogs are refused or not read. SerializedFiles of other container versions than 21 and 22 are refused.
- It ships no type tree package: LZMA- and Brotli-compressed packages are refused (re-pack them uncompressed or LZ4).
- A MonoBehaviour's script fields are returned as bytes for a file without type trees; a file that stores its trees (a bundle's) decodes them with `Player::script_fields`. `[SerializeReference]` managed references are refused.
- Textures: eleven formats are named (Alpha8, ARGB4444, RGB24, RGBA32, DXT1, DXT5, BC4, BC5, BC6H, BC7, R8); crunched formats are refused, and no format is decoded to RGBA, only split into mip levels.
- Meshes with compressed skin weights, legacy AnimationClips and clips with object-reference curves are refused.
- A streamed AnimationClip whose frames name a curve past its stated curve count (some of PEAK's 6000.3.15f1 clips) is refused as `Error::Unsupported`, not guessed at.
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

The real-data tests are `tests/serialized_census.rs`, which parses every SerializedFile in SkateBIRD's data folder and compares each file's object table (path ids, class ids, offsets, sizes) with a census made by [UnityPy](https://github.com/K0lb3/UnityPy) (make it from your own install with `pip install UnityPy` and `python tests/oracles/unitypy_census.py <SkateBIRD_Data folder> <oracles root>/unitypy/skatebird/census.json`), and `tests/bundles.rs`, which reads every UnityFS bundle of PEAK, Sledding Game and Big Walk, compares each node's bytes and each serialized node's header, types, stored type trees, object table and externals with a census made by `python tests/oracles/unitypy_bundles.py <bundles> > <oracles root>/unitypy/<game>/bundles.jsonl` (UnityPy and `lz4`), and decodes every object by the tree its file stores. The bundle census takes minutes and the decoding about ten, with 4 threads; `cargo run --release --example bundle_census -- <type tree package> <bundle>...` prints the same census for one bundle. [tests/inputs/README.md](tests/inputs/README.md) lists every input the tests can read (generated from `tests/inputs/mod.rs`).

CONTRIBUTING.md says how to build, and how to report a file this library cannot read.

The asset-conversion library [primase](https://github.com/Kalmarv/primase) is built on this crate.

## Check the examples

From the repository root, `cargo test -p unity-formats --doc` checks the crate documentation, and
`cargo build -p unity-formats --examples` builds its example programs. Examples that read paths need the caller's files; the repository ships no game assets.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
