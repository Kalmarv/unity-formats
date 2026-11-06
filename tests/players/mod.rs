//! The players the object tests open whole, shared by tests/textures.rs and tests/clips.rs: each game's data folder with
//! the UnityFS bundles it loads, as the oracle scripts read them (`<Game>_Data` and its `Resources`, and every bundle
//! under its bundle folder).
#![allow(dead_code, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use unity_formats::TypeTreePackage;
use unity_formats::player::{BundleSpec, NodeSelection, Player};

use crate::inputs::{Game, Tool, install_file, tool};

/// The UnityFS bundles under `dir` (recursively), in a stable order.
pub fn bundles(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if std::fs::File::open(&p).is_ok_and(|mut f| {
                let mut magic = [0u8; 8];
                std::io::Read::read_exact(&mut f, &mut magic).is_ok() && &magic == b"UnityFS\0"
            }) {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

/// A game's data folder and the folder of its bundles; `None` when the game is not configured.
pub fn folders(game: Game) -> Option<(PathBuf, Option<PathBuf>)> {
    match game {
        Game::Gungeon => Some((
            install_file(game, "EtG_Data")?,
            Some(install_file(game, "EtG_Data/StreamingAssets/Assets")?),
        )),
        Game::Ror2 => Some((
            install_file(game, "Risk of Rain 2_Data")?,
            Some(install_file(game, "Risk of Rain 2_Data/StreamingAssets/aa")?),
        )),
        Game::Schedule1 => Some((install_file(game, "Schedule I_Data")?, None)),
        _ => unreachable!("{game:?} is not opened whole"),
    }
}

/// The player of `game`, read with the type tree package `tools.unity_tpk`; `None` when either is not configured.
pub fn open(game: Game) -> Option<Player> {
    let tpk = std::fs::read(tool(Tool::UnityTpk)?).unwrap();
    let (data, bundle_dir) = folders(game)?;
    let paths = bundle_dir.as_deref().map(bundles).unwrap_or_default();
    let specs: Vec<BundleSpec> = paths
        .iter()
        .map(|path| BundleSpec {
            path,
            nodes: NodeSelection::All,
        })
        .collect();
    Some(
        Player::open_with_bundles(&data, TypeTreePackage::parse(&tpk).unwrap(), &specs)
            .unwrap_or_else(|e| panic!("{game:?}: {e}")),
    )
}
