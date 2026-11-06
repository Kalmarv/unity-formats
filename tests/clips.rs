//! Every legacy AnimationClip of Enter the Gungeon, Risk of Rain 2 and Schedule I (`installs.<game>`, their loose files
//! and bundles), read with the type tree package `tools.unity_tpk`, against UnityPy's reading of them
//! (`oracles.unitypy.<game>/clips.json`, tests/oracles/unitypy_clips.py): each clip's name, wrap mode and events, and its
//! tracks one for one with UnityPy's curves: the CRC-32 of the path (and of a float curve's attribute), the Transform
//! attribute or the component's class, and every key (time, value, slopes and the weights its `weightedMode` sets) by a
//! SHA-256 of the same bytes. Each Mecanim clip with object (PPtr) curves has UnityPy's object bindings and the objects
//! their values index, and every value is an index of one. The counts of what the players hold are asserted, the
//! Mecanim clips' with them.

#![allow(clippy::unwrap_used)]

mod inputs;
mod players;

use std::collections::BTreeMap;

use inputs::{Game, OracleSet, oracle};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use unity_formats::clip::{ClipData, Curve, Storage, path_hash};
use unity_formats::player::ObjRef;

/// The AnimationClip class.
const ANIMATION_CLIP: i32 = 74;

/// The SHA-256 the oracle takes of a curve's keys: per component, per key, f32 time, value, in and out slope, the in
/// and out weight (0 when unweighted) and a byte of the weighted sides (1 in, 2 out).
fn keys_sha(curves: &[Curve]) -> String {
    let mut h = Sha256::new();
    for c in curves {
        for k in &c.keys {
            for x in [
                k.time,
                k.value,
                k.in_slope,
                k.out_slope,
                k.in_weight.unwrap_or(0.0),
                k.out_weight.unwrap_or(0.0),
            ] {
                h.update(x.to_le_bytes());
            }
            h.update([u8::from(k.in_weight.is_some()) | (u8::from(k.out_weight.is_some()) << 1)]);
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What a player's clips hold.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    legacy: u64,
    looping: u64,
    ping_pong: u64,
    events: u64,
    /// Tracks by the oracle's kind.
    tracks: BTreeMap<String, u64>,
    keys: u64,
    weighted_curves: u64,
    stepped_curves: u64,
    /// Mecanim clips read, and refused by the reason.
    mecanim: u64,
    mecanim_refused: BTreeMap<String, u64>,
    /// Mecanim clips with PPtr curves, those tracks, their keys and the objects the clips' PPtr curves index.
    pptr_clips: u64,
    pptr_tracks: u64,
    pptr_keys: u64,
    pptr_targets: u64,
}

fn expected(game: Game) -> Counts {
    match game {
        Game::Gungeon => Counts {
            legacy: 27,
            events: 14,
            tracks: BTreeMap::from([
                ("float".into(), 7),
                ("position".into(), 158),
                ("rotation".into(), 10),
            ]),
            keys: 3150,
            stepped_curves: 3,
            ..Counts::default()
        },
        Game::Ror2 => Counts {
            legacy: 25,
            looping: 10,
            events: 1,
            tracks: BTreeMap::from([
                ("float".into(), 31),
                ("position".into(), 95),
                ("rotation".into(), 111),
                ("scale".into(), 72),
            ]),
            keys: 26_400,
            stepped_curves: 47,
            mecanim: 4898,
            pptr_clips: 5,
            pptr_tracks: 20,
            pptr_keys: 26,
            pptr_targets: 28,
            ..Counts::default()
        },
        Game::Schedule1 => Counts {
            legacy: 187,
            looping: 17,
            ping_pong: 1,
            events: 30,
            tracks: BTreeMap::from([
                ("euler".into(), 7),
                ("float".into(), 294),
                ("position".into(), 155),
                ("rotation".into(), 180),
                ("scale".into(), 113),
            ]),
            keys: 99_698,
            weighted_curves: 24,
            stepped_curves: 86,
            mecanim: 279,
            ..Counts::default()
        },
        _ => unreachable!("not a clip test player"),
    }
}

/// The Transform attribute of an oracle curve kind.
fn attribute(kind: &str) -> u32 {
    match kind {
        "position" => 1,
        "rotation" => 2,
        "scale" => 3,
        "euler" => 4,
        other => panic!("curve kind {other}"),
    }
}

#[test]
fn every_legacy_clip_reads_as_unitypy_reads_it() {
    let mut read = Vec::new();
    for game in [Game::Gungeon, Game::Ror2, Game::Schedule1] {
        let Some(want) = oracle(OracleSet::UnityPy, game, "clips.json") else {
            continue;
        };
        let Some(player) = players::open(game) else {
            continue;
        };
        let oracle: Json = serde_json::from_slice(&std::fs::read(want).unwrap()).unwrap();
        let want = oracle["files"].as_object().unwrap();
        let mecanim = oracle["mecanim"].as_object().unwrap();
        let mut counts = Counts::default();
        for (file, f) in &player.files {
            for o in f.objects.iter().filter(|o| o.class_id == ANIMATION_CLIP) {
                let r = ObjRef {
                    file: file.clone(),
                    path_id: o.path_id,
                };
                let at = format!("{game:?}: {file}/{}", o.path_id);
                let theirs = want.get(file).and_then(|f| f.get(o.path_id.to_string()));
                let clip = match ClipData::read(&player, &r) {
                    Ok(clip) => clip,
                    Err(e) if theirs.is_none() => {
                        // a Mecanim clip: what refuses it (`<kind>: <clip>: <why>`), without the clip's name
                        let e = e.to_string();
                        let why = e.splitn(3, ": ").last().unwrap_or(&e).to_owned();
                        *counts.mecanim_refused.entry(why).or_default() += 1;
                        continue;
                    }
                    Err(e) => panic!("{at}: {e}"),
                };
                let Some(theirs) = theirs else {
                    assert!(!clip.legacy, "{at}: a legacy clip the oracle does not list");
                    counts.mecanim += 1;
                    let pptr: Vec<_> = clip.tracks.iter().filter(|t| t.binding.is_pptr).collect();
                    let theirs = mecanim.get(file).and_then(|f| f.get(o.path_id.to_string()));
                    let Some(theirs) = theirs else {
                        assert!(pptr.is_empty(), "{at}: PPtr curves the oracle does not list");
                        continue;
                    };
                    let bindings: Vec<(u32, u32, i64)> = pptr
                        .iter()
                        .map(|t| (t.binding.path, t.binding.attribute, t.binding.type_id))
                        .collect();
                    let their_bindings: Vec<(u32, u32, i64)> = theirs["bindings"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|b| {
                            (
                                b[0].as_u64().unwrap() as u32,
                                b[1].as_u64().unwrap() as u32,
                                b[2].as_i64().unwrap(),
                            )
                        })
                        .collect();
                    assert_eq!(bindings, their_bindings, "{at}: PPtr bindings");
                    // the objects, by path id (0 for a null reference)
                    let targets: Vec<i64> = clip
                        .pptr_targets
                        .iter()
                        .map(|t| t.as_ref().map_or(0, |t| t.path_id))
                        .collect();
                    let their_targets: Vec<i64> = theirs["targets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|t| t[1].as_i64().unwrap())
                        .collect();
                    assert_eq!(targets, their_targets, "{at}: PPtr curve objects");
                    counts.pptr_clips += 1;
                    counts.pptr_tracks += pptr.len() as u64;
                    counts.pptr_keys += pptr.iter().map(|t| t.curves[0].keys.len() as u64).sum::<u64>();
                    counts.pptr_targets += targets.len() as u64;
                    continue;
                };
                assert!(clip.legacy, "{at}");
                let wrap = theirs["wrap_mode"].as_i64().unwrap();
                assert_eq!(
                    (
                        clip.name.as_str(),
                        clip.looping,
                        clip.ping_pong,
                        clip.events as u64
                    ),
                    (
                        theirs["name"].as_str().unwrap(),
                        wrap == 2,
                        wrap == 4,
                        theirs["events"].as_u64().unwrap()
                    ),
                    "{at}"
                );
                let curves = theirs["curves"].as_array().unwrap();
                assert_eq!(clip.tracks.len(), curves.len(), "{at}: tracks");
                let mut last = 0.0f32;
                for (i, (track, c)) in clip.tracks.iter().zip(curves).enumerate() {
                    let kind = c["kind"].as_str().unwrap();
                    let b = &track.binding;
                    let (attr, type_id) = if kind == "float" {
                        (
                            path_hash(c["attribute"].as_str().unwrap()),
                            c["class_id"].as_i64().unwrap(),
                        )
                    } else {
                        (attribute(kind), 4)
                    };
                    assert_eq!(
                        (b.path, b.attribute, b.type_id, b.is_pptr),
                        (path_hash(c["path"].as_str().unwrap()), attr, type_id, false),
                        "{at}: track {i} ({kind} of {})",
                        c["path"]
                    );
                    assert!(track.curves.iter().all(|c| c.storage == Storage::Keyframes));
                    let n = c["keys"].as_u64().unwrap();
                    assert!(
                        track.curves.iter().all(|c| c.keys.len() as u64 == n),
                        "{at}: track {i}: keys"
                    );
                    assert_eq!(
                        keys_sha(&track.curves),
                        c["sha256"].as_str().unwrap(),
                        "{at}: track {i} ({kind} of {}): the keys",
                        c["path"]
                    );
                    for curve in &track.curves {
                        if let Some(k) = curve.keys.last() {
                            last = last.max(k.time);
                        }
                        counts.keys += curve.keys.len() as u64;
                        counts.weighted_curves += u64::from(curve.is_weighted());
                        counts.stepped_curves += u64::from(curve.has_steps());
                    }
                    *counts.tracks.entry(kind.to_owned()).or_default() += 1;
                }
                assert_eq!(clip.duration, last, "{at}: the duration is the last key's time");
                counts.legacy += 1;
                counts.looping += u64::from(clip.looping);
                counts.ping_pong += u64::from(clip.ping_pong);
                counts.events += clip.events as u64;
            }
        }
        let listed: u64 = want.values().map(|f| f.as_object().unwrap().len() as u64).sum();
        assert_eq!(
            counts.legacy, listed,
            "{game:?}: legacy clips read and in the oracle"
        );
        let listed: u64 = mecanim
            .values()
            .map(|f| f.as_object().unwrap().len() as u64)
            .sum();
        assert_eq!(
            counts.pptr_clips, listed,
            "{game:?}: Mecanim clips with PPtr curves"
        );
        eprintln!("{game:?}: {counts:?}");
        read.push((game, counts));
    }
    for (game, counts) in read {
        assert_eq!(counts, expected(game), "{game:?}");
    }
}
