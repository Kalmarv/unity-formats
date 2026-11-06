# unity-formats

[![crates.io: unity-formats](https://img.shields.io/crates/v/unity-formats?label=unity-formats)](https://crates.io/crates/unity-formats) [![docs.rs](https://img.shields.io/docsrs/unity-formats)](https://docs.rs/unity-formats) [![CI](https://img.shields.io/github/actions/workflow/status/Kalmarv/unity-formats/ci.yml?branch=main&label=CI)](https://github.com/Kalmarv/unity-formats/actions/workflows/ci.yml)

A Rust library that reads the data files of built Unity games. It reads SerializedFiles, UnityFS bundles and streamed `.resS` data, and decodes the objects inside them into typed Rust values. A player build strips its type trees, so to read one you supply a type tree package (a TPK file) for the game's Unity version.

## Features

- Reads SerializedFile containers of versions 21 and 22
- Reads UnityFS bundles of format 8 with stored, LZ4 or LZ4HC blocks, including a player's `data.unity3d`
- Decodes objects through a file's stored type trees or through a TPK type tree package (uncompressed or LZ4)
- Follows object references across a player's files and reads streamed `.resS` and `.resource` data, loose or bundled
- Reads scenes: GameObjects, Transforms composed to world matrices, renderers, LOD groups, colliders, lights, cameras, terrains, ambient and fog settings
- Reads meshes, including vertex streams, compressed meshes, skins and blend shapes
- Splits `Texture2D` data into mip levels for 11 pixel formats, and reads materials and a Shader's render state
- Reads compiled shader program variants with their constant buffers and texture slots, and extracts DXBC
- Reads terrain data, Avatars, AnimatorControllers and Mecanim AnimationClips
- Reads Universal Render Pipeline 17.3 objects: pipeline assets, renderers, Decal and Full Screen Pass features, camera data and global settings
- Tested against SkateBIRD (Unity 2019.4.20f1), Sledding Game (6000.3.12f1), PEAK (6000.3.15f1) and Big Walk (6000.3.17f1)
- Returns values as the file stores them, in Unity's coordinate frame, without converting them
- Refuses unsupported or malformed input with an `Error` that names what it found, and does not panic on it
- Never writes files, and reads only the folder and bundles it is given

## Getting Started

### Requirements

- Rust 1.99 or later (the repository pins 1.99.0 in `rust-toolchain.toml`)
- A type tree package (`.tpk`) covering the game's Unity version, which you supply; the crate ships none, and refuses LZMA- or Brotli-compressed packages

### Installation

The crate is published on [crates.io](https://crates.io/crates/unity-formats):

```sh
cargo add unity-formats
```

### Usage

This program lists a player's scenes, reads three of its meshes and, for a Universal Render Pipeline game, prints its default renderer:

```rust,no_run
use std::path::Path;
use unity_formats::pipeline::read_urp_renderer;
use unity_formats::{TypeTreePackage, listing::list, mesh::UnityMeshData, player::Player};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // a type tree package for the game's Unity version
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

The repository has these example programs:

```sh
# the program above, also reading any bundles given
cargo run --example inspect_player -- <Game_Data folder> <typetrees.tpk> [<bundle>...]
# a JSON census of bundles: header, nodes, types and every object decoded
cargo run --release --example bundle_census -- <typetrees.tpk> <bundle>...
# XXH3-128 keys of every mesh's vertex and index data, for `locum-cli index --keys`
cargo run --release --example mesh_keys -- <Game_Data folder> <typetrees.tpk> > keys.tsv
# every D3D11 shader variant with its DXBC key, for `locum-cli index --programs`
cargo run --release --example program_keys -- <Game_Data folder> <typetrees.tpk> > programs.jsonl
```

What each reader refuses is listed in [docs/limitations.md](docs/limitations.md), and the API docs are on [docs.rs](https://docs.rs/unity-formats).

`cargo test --workspace` runs the unit tests and the tests on synthetic bytes. The tests that read real games are skipped unless `GAME_DATA` names a configuration file of your own installs; [docs/testing.md](docs/testing.md) describes how to set that up. [CONTRIBUTING.md](CONTRIBUTING.md) covers the build checks and how to report a file the library cannot read. The asset-conversion library [primase](https://github.com/Kalmarv/primase) is built on this crate. Licensed under GPL-3.0-or-later; see [LICENSE](LICENSE).
