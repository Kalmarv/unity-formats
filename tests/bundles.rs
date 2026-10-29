//! Every UnityFS bundle of PEAK (`installs.peak`), Sledding Game (`installs.sledding`) and Big Walk
//! (`installs.bigwalk`), read with the type tree package `tools.unity_tpk`.
//!
//! `every_bundle_matches_the_independent_reader`: each bundle against an independent reader's census
//! (`oracles.unitypy`: tests/oracles/unitypy_bundles.py, UnityPy over the script's own bundle reader): every node's
//! offset, size and SHA-256, every serialized node's header, types, stored type trees (a digest of each), object table
//! and externals, equal; the counts of what the installs hold are asserted.
//!
//! `every_object_decodes_by_its_stored_tree`: every object of every serialized node decoded by the type tree its file
//! stores for its type, to exactly its size; the objects that are refused, by reason; every streamed resource an object
//! names inside the stream node of its bundle; the stored trees of the classes the package knows equal to its trees.

#![allow(clippy::unwrap_used)]

mod census;
mod inputs;

use std::collections::BTreeMap;
use std::path::PathBuf;

use inputs::{Game, OracleSet, Tool, install_file, oracle, tool};
use serde_json::Value as Json;
use unity_formats::TypeTreePackage;

/// The bundles of an install, in a stable order.
fn bundles(game: Game) -> Option<Vec<PathBuf>> {
    let (dir, wanted): (PathBuf, fn(&str) -> bool) = match game {
        Game::Peak => (install_file(game, "PEAK_Data/StreamingAssets/maps")?, |n| {
            n == "maps" || n.ends_with(".assetbundle")
        }),
        Game::Sledding => (
            install_file(game, "Sledding Game_Data/StreamingAssets/aa/StandaloneWindows64")?,
            |n| n.ends_with(".bundle"),
        ),
        Game::Bigwalk => (install_file(game, "Big Walk_Data")?, |n| n == "data.unity3d"),
        Game::Skatebird => return None,
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| wanted(&n.to_string_lossy())))
        .collect();
    found.sort();
    Some(found)
}

/// The census lines of every bundle, several bundles at a time.
fn censuses(paths: &[PathBuf], package: &TypeTreePackage, decode: bool) -> Vec<Json> {
    let threads = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .clamp(1, 4);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::Mutex::new(BTreeMap::new());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(path) = paths.get(i) else { break };
                    let lines = census::bundle(path, package, decode);
                    done.lock().unwrap().insert(i, lines);
                }
            });
        }
    });
    done.into_inner().unwrap().into_values().flatten().collect()
}

fn read_lines(path: &std::path::Path) -> Vec<Json> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| l.starts_with('{'))
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// What a game's bundles hold, counted from census lines.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    bundles: u64,
    nodes: u64,
    serialized: u64,
    streams: u64,
    blocks_stored: u64,
    blocks_lz4hc: u64,
    types: u64,
    types_with_trees: u64,
    objects: u64,
    node_bytes: u64,
}

fn counts(lines: &[Json]) -> Counts {
    let mut c = Counts::default();
    for l in lines {
        if let Some(blocks) = l.get("blocks") {
            c.bundles += 1;
            c.blocks_stored += blocks["0"].as_u64().unwrap_or(0);
            c.blocks_lz4hc += blocks["3"].as_u64().unwrap_or(0);
            for other in blocks.as_object().unwrap().keys() {
                assert!(other == "0" || other == "3", "a block compression {other}");
            }
            assert_eq!(l["header"]["flags"], 0x243, "{l}");
            assert_eq!(l["header"]["format_version"], 8, "{l}");
        } else if l.get("node").is_some() {
            c.nodes += 1;
            c.node_bytes += l["size"].as_u64().unwrap();
            if let Some(s) = l.get("serialized") {
                c.serialized += 1;
                let types = s["types"].as_array().unwrap();
                c.types += types.len() as u64;
                c.types_with_trees += types.iter().filter(|t| !t["tree"].is_null()).count() as u64;
                c.objects += s["objects"]["count"].as_u64().unwrap();
            } else {
                c.streams += 1;
            }
        }
    }
    c
}

/// What each install holds (counted from the census lines; the independent reader's census has the same counts).
fn expected(game: Game) -> Counts {
    match game {
        Game::Peak => Counts {
            bundles: 8,
            nodes: 441,
            serialized: 417,
            streams: 24,
            blocks_stored: 13_475,
            blocks_lz4hc: 283_256,
            types: 83_645,
            types_with_trees: 83_643,
            objects: 136_474_698,
            node_bytes: 38_892_592_969,
        },
        Game::Sledding => Counts {
            bundles: 50,
            nodes: 66,
            serialized: 50,
            streams: 16,
            blocks_stored: 0,
            blocks_lz4hc: 1_698,
            types: 224,
            types_with_trees: 224,
            objects: 1_427,
            node_bytes: 218_376_459,
        },
        Game::Bigwalk => Counts {
            bundles: 1,
            nodes: 19,
            serialized: 12,
            streams: 7,
            blocks_stored: 459,
            blocks_lz4hc: 43_139,
            types: 1_036,
            types_with_trees: 0,
            objects: 659_824,
            node_bytes: 5_714_422_612,
        },
        Game::Skatebird => unreachable!("SkateBIRD has no bundle"),
    }
}

/// The census lines with what the oracle does not read taken out.
fn comparable(mut line: Json) -> Json {
    if let Some(s) = line.get_mut("serialized").and_then(Json::as_object_mut) {
        s.remove("decode");
    }
    line
}

fn key(line: &Json) -> String {
    match line.get("node") {
        Some(n) => format!("{}/{}", line["bundle"].as_str().unwrap(), n.as_str().unwrap()),
        None => line["bundle"].as_str().unwrap().to_owned(),
    }
}

const GAMES: [Game; 3] = [Game::Peak, Game::Sledding, Game::Bigwalk];

#[test]
fn every_bundle_matches_the_independent_reader() {
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let package = TypeTreePackage::parse(&std::fs::read(tpk).unwrap()).unwrap();
    for game in GAMES {
        let (Some(paths), Some(oracle)) = (bundles(game), oracle(OracleSet::UnityPy, game, "bundles.jsonl"))
        else {
            continue;
        };
        let ours: Vec<Json> = censuses(&paths, &package, false)
            .into_iter()
            .map(comparable)
            .collect();
        let theirs = read_lines(&oracle);
        assert_eq!(ours.len(), theirs.len(), "{game:?}: census lines");
        let mut ours_by: BTreeMap<String, &Json> = BTreeMap::new();
        for l in &ours {
            assert!(l.get("refused").is_none(), "{game:?}: {l}");
            ours_by.insert(key(l), l);
        }
        for t in &theirs {
            let o = ours_by
                .get(&key(t))
                .unwrap_or_else(|| panic!("{game:?}: {}: not read", key(t)));
            assert_eq!(*o, t, "{game:?}: {}", key(t));
        }
        assert_eq!(counts(&ours), expected(game), "{game:?}");
        eprintln!("{game:?}: {:?}, equal to the independent reader's", counts(&ours));
    }
}

/// What decoding every object found, summed over a game's serialized nodes.
#[derive(Debug, Default, PartialEq, Eq)]
struct Decoded {
    objects: u64,
    decoded: u64,
    covered: u64,
    no_tree: u64,
    refusals: BTreeMap<String, u64>,
    streams_resolved: u64,
    stream_bytes: u64,
    trees_same_as_package: u64,
    trees_differ_from_package: u64,
    trees_not_in_package: u64,
}

fn decoded(lines: &[Json]) -> Decoded {
    let mut d = Decoded::default();
    for l in lines {
        let Some(s) = l.get("serialized") else { continue };
        let x = &s["decode"];
        d.objects += s["objects"]["count"].as_u64().unwrap();
        d.decoded += x["decoded"].as_u64().unwrap();
        d.covered += x["covered"].as_u64().unwrap();
        d.no_tree += x["no_tree"].as_u64().unwrap();
        for (reason, n) in x["refusals"].as_object().unwrap() {
            *d.refusals.entry(reason.clone()).or_default() += n.as_u64().unwrap();
        }
        d.streams_resolved += x["streams_resolved"].as_u64().unwrap();
        d.stream_bytes += x["stream_bytes"].as_u64().unwrap();
        d.trees_same_as_package += x["trees_same_as_package"].as_u64().unwrap();
        d.trees_differ_from_package += x["trees_differ_from_package"].as_array().unwrap().len() as u64;
        d.trees_not_in_package += x["trees_not_in_package"].as_u64().unwrap();
    }
    d
}

/// The one reason an object of these installs is refused: a MonoBehaviour with a `[SerializeReference]` field, whose
/// registry of managed references the type tree reader does not decode.
const MANAGED: &str =
    "class 114: not supported: data: managed reference (needs the file's reference type trees)";

fn expected_decoded(game: Game) -> Decoded {
    let refused = |n: u64| BTreeMap::from([(MANAGED.to_owned(), n)]);
    match game {
        Game::Peak => Decoded {
            objects: 136_474_698,
            decoded: 136_275_632,
            covered: 136_275_632,
            no_tree: 2,
            refusals: refused(199_064),
            streams_resolved: 0,
            stream_bytes: 0,
            trees_same_as_package: 8_367,
            trees_differ_from_package: 0,
            trees_not_in_package: 0,
        },
        Game::Sledding => Decoded {
            objects: 1_427,
            decoded: 1_333,
            covered: 1_333,
            no_tree: 0,
            refusals: refused(94),
            streams_resolved: 317,
            stream_bytes: 206_661_032,
            trees_same_as_package: 173,
            trees_differ_from_package: 0,
            trees_not_in_package: 0,
        },
        Game::Bigwalk => Decoded {
            objects: 659_824,
            no_tree: 659_824,
            ..Decoded::default()
        },
        Game::Skatebird => unreachable!("SkateBIRD has no bundle"),
    }
}

#[test]
fn every_object_decodes_by_its_stored_tree() {
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let package = TypeTreePackage::parse(&std::fs::read(tpk).unwrap()).unwrap();
    for game in GAMES {
        let Some(paths) = bundles(game) else { continue };
        let lines = censuses(&paths, &package, true);
        let found = decoded(&lines);
        eprintln!("{game:?}: {found:?}");
        assert_eq!(found, expected_decoded(game), "{game:?}");
    }
}

/// Players over bundles: Big Walk's `data.unity3d` is its player (twelve files named like a folder's, the build's scenes
/// in them, an object of each decodes); Sledding Game's bundles read beside its nine loose files (seven serialized files of the data folder and the two builtin ones in `Resources`), pointing into each
/// other, with the pixels of their textures in their own stream nodes; one PEAK level reads with the files its pointers
/// name and not its bundle's other levels.
#[test]
fn players_read_bundled_files_like_loose_ones() {
    use unity_formats::player::{BundleSpec, NodeSelection, ObjRef, Player};
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let package = || TypeTreePackage::parse(&std::fs::read(&tpk).unwrap()).unwrap();

    if let Some(data) = install_file(Game::Bigwalk, "Big Walk_Data") {
        let bundle = data.join("data.unity3d");
        let spec = BundleSpec {
            path: &bundle,
            nodes: NodeSelection::All,
        };
        let player = Player::open_with_bundles(&data, package(), &[spec]).unwrap();
        // the bundle's twelve files, and the loose `Resources/unity default resources` beside it
        assert_eq!(player.files.len(), 13);
        assert!(player.files["unity default resources"].bundle().is_none());
        assert_eq!(player.bundles.len(), 1);
        let scenes = player.scenes().unwrap();
        assert_eq!(
            scenes.iter().map(|s| s.file.as_str()).collect::<Vec<_>>(),
            ["level0", "level1", "level2", "level3"]
        );
        for (name, f) in player.files.iter().filter(|(_, f)| f.bundle().is_some()) {
            // not PlayerSettings (class 129), whose layout in the package is short of the data by 4 bytes
            let o = f.objects.iter().find(|o| o.class_id != 129).unwrap();
            let r = ObjRef {
                file: name.clone(),
                path_id: o.path_id,
            };
            assert!(player.class_name(&r).is_some(), "{name}/{}", o.path_id);
            player.decode(&r).unwrap();
        }
    }

    if let Some(data) = install_file(Game::Sledding, "Sledding Game_Data") {
        let dir = data.join("StreamingAssets/aa/StandaloneWindows64");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "bundle"))
            .collect();
        paths.sort();
        let specs: Vec<BundleSpec> = paths
            .iter()
            .map(|path| BundleSpec {
                path,
                nodes: NodeSelection::All,
            })
            .collect();
        let player = Player::open_with_bundles(&data, package(), &specs).unwrap();
        let bundled = player.files.values().filter(|f| f.bundle().is_some()).count();
        assert_eq!(
            (bundled, player.files.len() - bundled, player.bundles.len()),
            (50, 9, 50)
        );
        let mut textures = 0;
        for (name, f) in player.files.iter().filter(|(_, f)| f.bundle().is_some()) {
            for o in &f.objects {
                let r = ObjRef {
                    file: name.clone(),
                    path_id: o.path_id,
                };
                if player.class_name(&r) == Some("Texture2D") {
                    let t = unity_formats::texture::UnityTexture::read(&player, &r).unwrap();
                    assert_eq!(
                        t.data.len() as u64,
                        player
                            .field(&r, "m_CompleteImageSize")
                            .unwrap()
                            .and_then(|v| v.as_i64())
                            .unwrap() as u64,
                        "{name}/{}",
                        o.path_id
                    );
                    textures += 1;
                }
            }
        }
        assert_eq!(
            textures, 302,
            "the 302 Texture2D objects of the Addressables bundles"
        );
    }

    if let Some(data) = install_file(Game::Peak, "PEAK_Data") {
        let bundle = data.join("StreamingAssets/maps/levels192-207.assetbundle");
        let spec = BundleSpec {
            path: &bundle,
            nodes: NodeSelection::Closure(&["BuildPlayer-Level_207"]),
        };
        let player = Player::open_with_bundles(&data, package(), &[spec]).unwrap();
        let mut bundled: Vec<&str> = player
            .files
            .iter()
            .filter(|(_, f)| f.bundle().is_some())
            .map(|(n, _)| n.as_str())
            .collect();
        bundled.sort_unstable();
        assert_eq!(
            bundled,
            [
                "buildplayer-level_193.sharedassets",
                "buildplayer-level_197.sharedassets",
                "buildplayer-level_200.sharedassets",
                "buildplayer-level_207",
                "buildplayer-level_207.sharedassets"
            ]
        );
        // the builtin files the level names are the player's loose ones
        assert!(
            player.files.contains_key("unity default resources")
                && player.files.contains_key("unity_builtin_extra")
        );
    }
}

/// The AnimatorControllers in the files of one PEAK level, and in PEAK's loose files beside them, read whole, among them
/// the four with an empty layer (`Ambience`'s first, `Scout`'s seventh, one pair in the loose `resources.assets` and one
/// in level 193's shared assets): a state machine without states, whose entry node's one transition stores `0xFFFFFFFF`
/// and so has no destination.
///
/// Expected values: UnityPy over a bundle reader of its own (`tests/oracles/unitypy_bundles.py`'s) counts the class 91
/// objects of each file and reads `m_Destination` of every selector transition with the file's stored type trees. The
/// closure of `BuildPlayer-Level_207` holds the shared assets of levels 193 (73 controllers), 200 (6) and 197 (none); 192's
/// (4) is not in it. The player also holds PEAK's loose files, 102 more (`resources.assets` 43, `sharedassets0.assets`
/// 1, `sharedassets1.assets` 6, `sharedassets3.assets` 27, `sharedassets4.assets` 25), so 181 in all. Of the 181
/// controllers' selector transitions exactly four store `0xFFFFFFFF`, each the only transition of an entry node of a
/// machine without states. An independent census of the compiled controllers of PEAK, Sledding Game, Big Walk and
/// SkateBIRD (699 controllers, 1,671 selector transitions) finds 16 of them, in `Ambience` and `Scout` of each of PEAK's
/// seven level bundles and of its loose `resources.assets`, and nowhere else.
#[test]
fn peak_controllers_read_including_the_layers_with_nothing_in_them() {
    use std::collections::BTreeMap;
    use unity_formats::animator::UnityAnimatorController;
    use unity_formats::listing::list;
    use unity_formats::player::{BundleSpec, NodeSelection, Player};
    let Some(tpk) = tool(Tool::UnityTpk) else { return };
    let Some(data) = install_file(Game::Peak, "PEAK_Data") else {
        return;
    };
    let package = TypeTreePackage::parse(&std::fs::read(tpk).unwrap()).unwrap();
    let bundle = data.join("StreamingAssets/maps/levels192-207.assetbundle");
    let spec = BundleSpec {
        path: &bundle,
        nodes: NodeSelection::Closure(&["BuildPlayer-Level_207"]),
    };
    let player = Player::open_with_bundles(&data, package, &[spec]).unwrap();
    let controllers = list(&player, Some(&["AnimatorController"])).unwrap();
    let mut per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &controllers.entries {
        *per_file.entry(e.object.file.as_str()).or_default() += 1;
    }
    let bundled: usize = per_file
        .iter()
        .filter(|(file, _)| file.starts_with("buildplayer-"))
        .map(|(_, n)| n)
        .sum();
    assert_eq!(
        per_file,
        BTreeMap::from([
            ("buildplayer-level_193.sharedassets", 73),
            ("buildplayer-level_200.sharedassets", 6),
            ("resources.assets", 43),
            ("sharedassets0.assets", 1),
            ("sharedassets1.assets", 6),
            ("sharedassets3.assets", 27),
            ("sharedassets4.assets", 25),
        ])
    );
    assert_eq!((bundled, controllers.entries.len()), (79, 181));
    let mut to_nowhere = Vec::new();
    for e in &controllers.entries {
        let c = UnityAnimatorController::read(&player, &e.object).unwrap();
        for (m, machine) in c.state_machines.iter().enumerate() {
            for selector in &machine.selectors {
                for t in selector.transitions.iter().filter(|t| t.destination.is_none()) {
                    assert!(selector.is_entry && machine.states.is_empty() && t.conditions.is_empty());
                    to_nowhere.push(format!(
                        "{}/{} {:?} machine {m}",
                        e.object.file, e.object.path_id, c.name
                    ));
                }
            }
        }
    }
    to_nowhere.sort();
    assert_eq!(
        to_nowhere,
        [
            "buildplayer-level_193.sharedassets/7070 \"Ambience\" machine 0",
            "buildplayer-level_193.sharedassets/7071 \"Scout\" machine 6",
            "resources.assets/2439 \"Ambience\" machine 0",
            "resources.assets/2441 \"Scout\" machine 6",
        ]
    );
}
