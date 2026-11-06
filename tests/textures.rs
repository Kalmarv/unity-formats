//! Every Texture2D of Risk of Rain 2 (`installs.ror2`: its loose files and its Addressables bundles) and Schedule I
//! (`installs.schedule1`), read with the type tree package `tools.unity_tpk`, against UnityPy's reading of them
//! (`oracles.unitypy.<game>/textures.json`, tests/oracles/unitypy_textures.py): every texture's name, format, size, mip
//! count and the length of its pixel bytes equal; its levels split (or, crunched, unpack) to exactly those bytes; and
//! each crunched texture's first level, its DXT blocks decoded to RGBA, the image UnityPy decodes from the crunch file.
//! The counts of what the players hold, by format, are asserted.

#![allow(clippy::unwrap_used)]

mod inputs;
mod players;

use std::collections::BTreeMap;

use inputs::{Game, OracleSet, oracle};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use unity_formats::player::ObjRef;
use unity_formats::texture::{TextureFormat, UnityTexture};

/// The Texture2D class.
const TEXTURE_2D: i32 = 28;

/// RGBA bytes of a DXT1 or DXT5 level, rows as stored.
fn rgba(format: TextureFormat, blocks: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut pixels = vec![0u32; w * h];
    match format {
        TextureFormat::Dxt1 => texture2ddecoder::decode_bc1(blocks, w, h, &mut pixels).unwrap(),
        TextureFormat::Dxt5 => texture2ddecoder::decode_bc3(blocks, w, h, &mut pixels).unwrap(),
        other => panic!("no RGBA decoding of {other:?}"),
    }
    // texture2ddecoder packs a pixel as 0xAARRGGBB
    pixels
        .iter()
        .flat_map(|&p| {
            let [b, g, r, a] = p.to_le_bytes();
            [r, g, b, a]
        })
        .collect()
}

/// What a player's textures hold: per TextureFormat id, the textures and their pixel bytes; and the crunched textures
/// whose first level decodes to UnityPy's image.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    by_format: BTreeMap<i64, (u64, u64)>,
    crunched_images_equal: u64,
}

fn expected(game: Game) -> Counts {
    match game {
        Game::Ror2 => Counts {
            by_format: BTreeMap::from([
                (1, (4264, 57_621_966)),
                (2, (34, 143_970)),
                (3, (187, 200_776_644)),
                (4, (520, 426_400_472)),
                (5, (5, 8_388_636)),
                (9, (1, 32_768)),
                (10, (1766, 608_294_384)),
                (12, (3925, 1_879_761_712)),
                (25, (79, 227_839_328)),
                (26, (2, 43_728)),
                (27, (1, 21_872)),
                (28, (2, 111_431)),
                (29, (30, 2_631_534)),
            ]),
            crunched_images_equal: 32,
        },
        Game::Schedule1 => Counts {
            by_format: BTreeMap::from([
                (1, (79, 36_101_819)),
                (2, (34, 143_970)),
                (3, (21, 39_922_092)),
                (4, (63, 86_407_768)),
                (5, (1, 262_144)),
                (10, (899, 1_621_605_440)),
                (12, (1921, 3_403_778_976)),
                (19, (6, 36_874_920)),
                (20, (4, 262_144)),
                (24, (2, 2_796_256)),
                (25, (72, 155_823_792)),
                (28, (2, 14_087)),
            ]),
            crunched_images_equal: 2,
        },
        _ => unreachable!("not a texture test player"),
    }
}

#[test]
fn every_texture_reads_as_unitypy_reads_it() {
    for game in [Game::Ror2, Game::Schedule1] {
        let Some(want) = oracle(OracleSet::UnityPy, game, "textures.json") else {
            continue;
        };
        let Some(player) = players::open(game) else {
            continue;
        };
        let want: Json = serde_json::from_slice(&std::fs::read(want).unwrap()).unwrap();
        let want = want["files"].as_object().unwrap();
        let mut counts = Counts::default();
        let mut seen = 0usize;
        for (file, f) in &player.files {
            for o in f.objects.iter().filter(|o| o.class_id == TEXTURE_2D) {
                let r = ObjRef {
                    file: file.clone(),
                    path_id: o.path_id,
                };
                let at = format!("{game:?}: {file}/{}", o.path_id);
                let theirs = want
                    .get(file)
                    .and_then(|f| f.get(o.path_id.to_string()))
                    .unwrap_or_else(|| panic!("{at}: not in the oracle"));
                seen += 1;
                let t = UnityTexture::read(&player, &r).unwrap_or_else(|e| panic!("{at}: {e}"));
                assert_eq!(
                    (
                        t.name.as_str(),
                        t.format,
                        u64::from(t.width),
                        u64::from(t.height),
                        u64::from(t.mips),
                        t.data.len() as u64,
                    ),
                    (
                        theirs["name"].as_str().unwrap(),
                        theirs["format"].as_i64().unwrap(),
                        theirs["width"].as_u64().unwrap(),
                        theirs["height"].as_u64().unwrap(),
                        theirs["mips"].as_u64().unwrap(),
                        theirs["bytes"].as_u64().unwrap(),
                    ),
                    "{at}"
                );
                let (format, levels) = t.levels().unwrap_or_else(|e| panic!("{at}: {e}"));
                let entry = counts.by_format.entry(t.format).or_default();
                entry.0 += 1;
                entry.1 += t.data.len() as u64;
                if let Some(sha) = theirs.get("rgba_sha256").and_then(Json::as_str) {
                    let image = rgba(format, &levels[0], t.width, t.height);
                    let ours: String = Sha256::digest(&image)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    assert_eq!(ours, sha, "{at} ({}): the first level's RGBA", t.name);
                    counts.crunched_images_equal += 1;
                }
            }
        }
        let theirs: usize = want.values().map(|f| f.as_object().unwrap().len()).sum();
        assert_eq!(seen, theirs, "{game:?}: textures read and in the oracle");
        eprintln!("{game:?}: {counts:?}");
        assert_eq!(counts, expected(game), "{game:?}");
    }
}
