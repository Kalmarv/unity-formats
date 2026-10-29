//! The colour space each player renders in (`Player::colour_space`), against an independent UnityPy reading of its
//! PlayerSettings (tests/oracles/unitypy_player_settings.py).

#![allow(clippy::unwrap_used)]

mod inputs;

use std::path::PathBuf;

use inputs::{Game, OracleSet, Tool, install_file, oracle, tool};
use unity_formats::TypeTreePackage;
use unity_formats::player::{BundleSpec, ColourSpace, NodeSelection, Player};

/// The players read.
const PLAYERS: [Game; 4] = [Game::Skatebird, Game::Sledding, Game::Peak, Game::Bigwalk];

/// A player's data folder, `None` when the game is not configured.
fn data_folder(game: Game) -> Option<PathBuf> {
    match game {
        Game::Skatebird => install_file(game, "SkateBIRD_Data"),
        Game::Sledding => install_file(game, "Sledding Game_Data"),
        Game::Peak => install_file(game, "PEAK_Data"),
        Game::Bigwalk => install_file(game, "Big Walk_Data"),
    }
}

#[test]
fn the_colour_space_is_the_one_unitypy_reads() {
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let tpk = std::fs::read(tpk).unwrap();
    for game in PLAYERS {
        let (Some(data), Some(expected)) = (
            data_folder(game),
            oracle(OracleSet::UnityPy, game, "player-settings.json"),
        ) else {
            continue;
        };
        let package = TypeTreePackage::parse(&tpk).unwrap();
        // a player whose files are one bundle (Big Walk's `data.unity3d`) is read through it
        let bundle = data.join("data.unity3d");
        let player = if bundle.exists() {
            let spec = BundleSpec {
                path: &bundle,
                nodes: NodeSelection::All,
            };
            Player::open_with_bundles(&data, package, &[spec]).unwrap()
        } else {
            Player::open(&data, package).unwrap()
        };
        let expected: serde_json::Value = serde_json::from_slice(&std::fs::read(&expected).unwrap()).unwrap();
        let expected = match expected["m_ActiveColorSpace"].as_i64() {
            Some(0) => ColourSpace::Gamma,
            Some(1) => ColourSpace::Linear,
            other => panic!("{game:?}: the oracle's m_ActiveColorSpace is {other:?}"),
        };
        assert_eq!(player.colour_space().unwrap(), Some(expected), "{game:?}");
        eprintln!("{game:?}: {expected:?}");
    }
}
