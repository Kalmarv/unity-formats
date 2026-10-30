//! The Universal Render Pipeline readers (`Player::urp_settings`, `pipeline::read_urp_renderer`,
//! `read_urp_feature_active`, `read_urp_decal_feature`, `read_urp_full_screen_feature`, `read_urp_camera`) on the
//! URP players, against an independent UnityPy reading of the same objects (tests/oracles/unitypy_urp.py): every
//! pipeline asset's renderers, each renderer's features followed from them, and every camera's URP data in the
//! players' scene files, each read to exactly its bytes. An asset of a player whose Unity version identifies no verified
//! layout is refused by name (Big Walk's).

#![allow(clippy::unwrap_used)]

mod inputs;

use std::collections::BTreeMap;
use std::path::PathBuf;

use inputs::{Game, OracleSet, Tool, install_file, oracle, tool};
use serde_json::Value as Json;
use unity_formats::TypeTreePackage;
use unity_formats::pipeline::{
    URP_CAMERA_DATA, URP_DECAL_FEATURE, URP_FULL_SCREEN_FEATURE, read_urp_camera, read_urp_decal_feature,
    read_urp_feature_active, read_urp_full_screen_feature, read_urp_renderer,
};
use unity_formats::player::{BundleSpec, NodeSelection, ObjRef, PPtr, Player};

/// The URP players read.
const PLAYERS: [Game; 3] = [Game::Peak, Game::Sledding, Game::Bigwalk];

/// A player's data folder, `None` when the game is not configured.
fn data_folder(game: Game) -> Option<PathBuf> {
    match game {
        Game::Peak => install_file(game, "PEAK_Data"),
        Game::Sledding => install_file(game, "Sledding Game_Data"),
        Game::Bigwalk => install_file(game, "Big Walk_Data"),
        Game::Skatebird => None,
    }
}

/// The oracle's key for an object: `file/path id`.
fn key(r: &ObjRef) -> String {
    format!("{}/{}", r.file, r.path_id)
}

/// A reader's value as JSON, each object reference written as the oracle writes it (`[file id, path id]`).
fn json(value: impl serde::Serialize) -> Json {
    fn refs(v: Json) -> Json {
        match v {
            Json::Object(m) if m.len() == 2 && m.contains_key("file_id") && m.contains_key("path_id") => {
                Json::Array(vec![m["file_id"].clone(), m["path_id"].clone()])
            }
            Json::Object(m) => Json::Object(m.into_iter().map(|(k, v)| (k, refs(v))).collect()),
            Json::Array(a) => Json::Array(a.into_iter().map(refs).collect()),
            v => v,
        }
    }
    refs(serde_json::to_value(value).unwrap())
}

/// The oracle's reading of an object with `drop` removed, after checking that it read the verified layout.
fn expected(oracle: &Json, what: &str, drop: &[&str]) -> Json {
    let mut v = oracle.clone();
    assert!(
        v.get("layout").is_none(),
        "{what}: the oracle did not read it: {v}"
    );
    for k in drop {
        v.as_object_mut().unwrap().remove(*k);
    }
    v
}

/// An object resolved from a reference its holder stores.
fn follow(player: &Player, holder: &ObjRef, pptr: PPtr) -> ObjRef {
    player.resolve_pptr(&holder.file, pptr).unwrap().unwrap()
}

#[test]
fn urp_objects_read_as_unitypy_reads_them() {
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let tpk = std::fs::read(tpk).unwrap();
    for game in PLAYERS {
        let (Some(data), Some(oracle_file)) =
            (data_folder(game), oracle(OracleSet::UnityPy, game, "urp.json"))
        else {
            continue;
        };
        let want: Json = serde_json::from_slice(&std::fs::read(&oracle_file).unwrap()).unwrap();
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

        // the assets the oracle found, and the renderers and features followed from them here
        let mut renderers = BTreeMap::new();
        for (k, asset) in want["assets"].as_object().unwrap() {
            let (file, pid) = k.rsplit_once('/').unwrap();
            let r = ObjRef {
                file: file.into(),
                path_id: pid.parse().unwrap(),
            };
            // the layout is identified by the player's Unity version: Big Walk's is not one it is verified on, and its
            // renderers are the ones the oracle read (its fields to the renderer list are 17.3's)
            let listed: Vec<PPtr> = match player.urp_settings(&r) {
                Ok(settings) => {
                    let got = json(&settings);
                    for field in [
                        "renderers",
                        "default_renderer",
                        "require_depth_texture",
                        "require_opaque_texture",
                        "msaa",
                    ] {
                        assert_eq!(got[field], asset[field], "{game:?} {k}: {field}");
                    }
                    settings.renderers
                }
                Err(e) if matches!(game, Game::Bigwalk) => {
                    let why = e.to_string();
                    assert!(
                        why.contains("of a Unity 6000.3.17f1 player"),
                        "{game:?} {k}: {why}"
                    );
                    asset["renderers"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| PPtr {
                            file_id: p[0].as_i64().unwrap() as i32,
                            path_id: p[1].as_i64().unwrap(),
                        })
                        .collect()
                }
                Err(e) => panic!("{game:?} {k}: {e}"),
            };
            for p in &listed {
                let renderer = follow(&player, &r, *p);
                renderers.insert(key(&renderer), renderer);
            }
        }
        let want_renderers = want["renderers"].as_object().unwrap();
        assert_eq!(
            renderers.keys().collect::<Vec<_>>(),
            want_renderers.keys().collect::<Vec<_>>(),
            "{game:?}: the renderers"
        );
        let mut features = BTreeMap::new();
        for (k, r) in &renderers {
            let oracle = &want_renderers[k];
            assert_eq!(
                player.script_class(r).unwrap().as_deref(),
                oracle["class"].as_str(),
                "{game:?} {k}"
            );
            let renderer = read_urp_renderer(&player.behaviour(r).unwrap().1).unwrap();
            assert_eq!(
                json(&renderer),
                expected(oracle, k, &["class", "asset_version"]),
                "{game:?} {k}"
            );
            assert_eq!(oracle["asset_version"], 3, "{game:?} {k}");
            for p in &renderer.features {
                let feature = follow(&player, r, *p);
                features.insert(key(&feature), feature);
            }
        }
        let want_features = want["features"].as_object().unwrap();
        assert_eq!(
            features.keys().collect::<Vec<_>>(),
            want_features.keys().collect::<Vec<_>>(),
            "{game:?}: the features"
        );
        for (k, f) in &features {
            let oracle = &want_features[k];
            let class = player.script_class(f).unwrap().unwrap();
            assert_eq!(Some(class.as_str()), oracle["class"].as_str(), "{game:?} {k}");
            let bytes = player.behaviour(f).unwrap().1;
            assert_eq!(
                Json::Bool(read_urp_feature_active(&bytes).unwrap()),
                oracle["active"],
                "{game:?} {k}"
            );
            match class.as_str() {
                URP_DECAL_FEATURE => {
                    let mut got = json(read_urp_decal_feature(&bytes).unwrap());
                    got.as_object_mut().unwrap().remove("active");
                    assert_eq!(got, expected(&oracle["decal"], k, &[]), "{game:?} {k}");
                }
                URP_FULL_SCREEN_FEATURE => {
                    let mut got = json(read_urp_full_screen_feature(&bytes).unwrap());
                    got.as_object_mut().unwrap().remove("active");
                    assert_eq!(
                        got,
                        expected(&oracle["full_screen"], k, &["version"]),
                        "{game:?} {k}"
                    );
                    assert_eq!(oracle["full_screen"]["version"], 1, "{game:?} {k}");
                }
                _ => {}
            }
        }

        // every camera's URP data in the scene files, found here by its script class
        let mut cameras = BTreeMap::new();
        for scene in player.scenes().unwrap() {
            let file = player.file(&scene.file).unwrap();
            for o in &file.objects {
                let r = ObjRef {
                    file: scene.file.clone(),
                    path_id: o.path_id,
                };
                if player.class_name(&r) != Some("MonoBehaviour")
                    || player.script_class(&r).ok().flatten().as_deref() != Some(URP_CAMERA_DATA)
                {
                    continue;
                }
                let camera = read_urp_camera(&player.behaviour(&r).unwrap().1).unwrap();
                cameras.insert(key(&r), json(camera));
            }
        }
        let want_cameras = want["cameras"].as_object().unwrap();
        assert_eq!(
            cameras.keys().collect::<Vec<_>>(),
            want_cameras.keys().collect::<Vec<_>>(),
            "{game:?}: the cameras"
        );
        for (k, got) in &cameras {
            let oracle = &want_cameras[k];
            assert_eq!(*got, expected(oracle, k, &["version"]), "{game:?} {k}");
            assert_eq!(oracle["version"], 2, "{game:?} {k}");
        }
        eprintln!(
            "{game:?}: {} assets, {} renderers, {} features, {} cameras",
            want["assets"].as_object().unwrap().len(),
            renderers.len(),
            features.len(),
            cameras.len()
        );
    }
}
