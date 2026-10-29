//! The colour properties a material's shader declares, with their flags (`program::colour_properties`), against an
//! independent UnityPy reading of the same shaders (tests/oracles/unitypy_shader_properties.py; its output is stated
//! here). In a linear colour space player Unity hands an `[HDR]` colour's stored value to the programs as it is and
//! converts any other colour from gamma to linear, so the flag decides what a stored colour is.

#![allow(clippy::unwrap_used)]

mod inputs;

use std::path::PathBuf;

use inputs::{Game, Tool, install_file, tool};
use unity_formats::TypeTreePackage;
use unity_formats::player::{ObjRef, Player};
use unity_formats::program::{PROPERTY_HDR, colour_properties};
use unity_formats::value::Value;

/// A player's data folder, `None` when the game is not configured.
fn data_folder(game: Game) -> Option<PathBuf> {
    match game {
        Game::Skatebird => install_file(game, "SkateBIRD_Data"),
        Game::Sledding => install_file(game, "Sledding Game_Data"),
        Game::Peak => install_file(game, "PEAK_Data"),
        Game::Bigwalk => None,
    }
}

/// The Material named `name` in `file` of `player`.
fn material(player: &Player, file: &str, name: &str) -> ObjRef {
    let found: Vec<ObjRef> = player
        .file(file)
        .unwrap()
        .objects
        .iter()
        .filter(|o| o.class_id == 21)
        .map(|o| ObjRef {
            file: file.to_owned(),
            path_id: o.path_id,
        })
        .filter(|r| {
            player
                .field(r, "m_Name")
                .unwrap()
                .as_ref()
                .and_then(Value::as_str)
                == Some(name)
        })
        .collect();
    assert_eq!(found.len(), 1, "{file}: Materials named {name}: {found:?}");
    found.into_iter().next().unwrap()
}

/// A game, a file of its player, a Material in it, and its shader's colour properties with their flags.
type Case = (Game, &'static str, &'static str, &'static [(&'static str, u32)]);

/// Each material's shader's colour properties and flags, as UnityPy reads them.
#[test]
fn each_colour_property_has_the_flags_unitypy_reads() {
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let tpk = std::fs::read(tpk).unwrap();
    let cases: [Case; 3] = [
        // W/Peak_Standard: every colour [HDR]
        (
            Game::Peak,
            "resources.assets",
            "M_Rock_Crystal",
            &[
                ("_Tint", 16),
                ("_BaseColor", 16),
                ("_Color1", 16),
                ("_Color21", 16),
                ("_Color3", 16),
            ],
        ),
        // Universal Render Pipeline/Lit: _BaseColor [MainColor] (256), _EmissionColor [HDR], _Color [HideInInspector]
        (
            Game::Sledding,
            "sharedassets0.assets",
            "Skin_Bear_Black",
            &[
                ("_BaseColor", 256),
                ("_SpecColor", 0),
                ("_EmissionColor", 16),
                ("_Color", 1),
            ],
        ),
        // the built-in Standard shader declares its emission colour without [HDR]
        (
            Game::Skatebird,
            "sharedassets0.assets",
            "gold",
            &[("_Color", 0), ("_EmissionColor", 0)],
        ),
    ];
    for (game, file, name, expected) in cases {
        let Some(data) = data_folder(game) else { continue };
        let player = Player::open(&data, TypeTreePackage::parse(&tpk).unwrap()).unwrap();
        let m = material(&player, file, name);
        let shader = player
            .resolve(file, &player.field(&m, "m_Shader").unwrap().unwrap())
            .unwrap()
            .unwrap();
        let read = colour_properties(&player, &shader).unwrap();
        let got: Vec<(&str, u32)> = read.iter().map(|p| (p.name.as_str(), p.flags)).collect();
        assert_eq!(got, expected, "{game:?} {name}");
        for p in &read {
            assert_eq!(p.hdr(), p.flags & PROPERTY_HDR != 0);
        }
        eprintln!("{game:?} {name}: {got:?}");
    }
}
