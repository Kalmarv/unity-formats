//! The real-data inputs this crate's tests read, each with what it is and where it comes from
//! (`primase_testdata::inputs!`; README.md here lists them). A test whose input is not configured skips with a line
//! naming its key. An input declared `gate: true` is one a complete run needs: with `[run] require = 'gate'` in the
//! `GAME_DATA` file, a missing one fails the test instead of skipping it. Integration tests reach this module with
//! `mod inputs;`, unit tests with `#[cfg(test)] #[path = "../tests/inputs/mod.rs"] mod inputs;` in the crate root, and
//! tests/real_data_inputs.rs checks that every query the crate's sources write is declared here. Declare an input
//! here when a test first asks for it.

primase_testdata::inputs! {
    games {
        Skatebird = "skatebird",
        Peak = "peak",
        Sledding = "sledding",
        Bigwalk = "bigwalk",
        Gungeon = "gungeon",
        Ror2 = "ror2",
        Schedule1 = "schedule1",
        General = "general",
    }
    installs {
        Skatebird {
            about: "SkateBIRD's install folder (`SkateBIRD_Data`: a Unity 2019.4.20f1 player)",
            source: "the user's own install",
            gate: true,
        },
        Peak {
            about: "PEAK's install folder (a Unity 6000.3.15f1 player: loose files and level AssetBundles)",
            source: "the user's own install",
            gate: true,
        },
        Sledding {
            about: "Sledding Game's install folder (a Unity 6000.3.12f1 player with Addressables bundles)",
            source: "the user's own install",
            gate: true,
        },
        Bigwalk {
            about: "Big Walk's install folder (a Unity 6000.3.17f1 player in one `data.unity3d`)",
            source: "the user's own install",
            gate: true,
        },
        Gungeon {
            about: "Enter the Gungeon's install folder (`EtG_Data`: a Unity 2017.4.27f1 player of SerializedFile version 17, with UnityFS format 6 bundles in `StreamingAssets/Assets`)",
            source: "the user's own install",
            gate: true,
        },
        Ror2 {
            about: "Risk of Rain 2's install folder (`Risk of Rain 2_Data`: a Unity 2021.3.45f2 player with Addressables bundles in `StreamingAssets/aa`)",
            source: "the user's own install",
            gate: true,
        },
        Schedule1 {
            about: "Schedule I's install folder (`Schedule I_Data`: a Unity 2022.3.62f2 player of loose files)",
            source: "the user's own install",
            gate: true,
        },
    }
    keys {}
    tools {
        UnityTpk = "unity_tpk" {
            about: "a type tree package (TPK, uncompressed or LZ4) with the class layouts and the common strings of the Unity versions read",
            source: "input data: class layouts for the players' Unity versions",
            gate: true,
        },
    }
    oracle_sets {
        Corpus = "corpus" {
            about: "other tools' public files, kept beside the oracles so the tests read them as those tools read them",
            source: "the tools' own repositories and CI, at the commit each entry names",
        },
        UnityPy = "unitypy" {
            about: "UnityPy's readings of a Unity player (the object-table census these tests use; tests/oracles/unitypy_census.py makes it) and of its bundles (tests/oracles/unitypy_bundles.py)",
            source: "independent: UnityPy (https://github.com/K0lb3/UnityPy), run by the scripts in tests/oracles",
        },
    }
    declared {
        Query::Oracle(OracleSet::Corpus, Game::General, "unitypy-samples") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's test bundles `atlas_test`, `banner_1`, `char_118_yuki.ab` and `xinzexi_2_n_tex` (UnityFS format 6, LZ4HC, Unity 5.6.7 to 2018.4; SerializedFile version 17), read by tests/public_samples.rs"#,
            source: r#"UnityPy's repository (https://github.com/K0lb3/UnityPy) at 417d5952, `tests/samples` (Git LFS)"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::General, "samples.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of those bundles: format, header flags, every node's path, size and SHA-256 (JSON; tests/oracles/unitypy_samples.py makes it, tests/public_samples.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_samples.py"#,
        },
        Query::Oracle(OracleSet::Corpus, Game::General, "assetripper-tpk") => {
            status: Live,
            gate: true,
            about: r#"AssetRipper's type tree package in its four compressions (`uncompressed.tpk`, `lz4.tpk`, `lzma.tpk`, `brotli.tpk`), read by tests/public_samples.rs"#,
            source: r#"AssetRipper/Tpk's CI artifacts of run 41278032151 (commit 94549d10, 2026-11-01)"#,
        },
        Query::Install(Game::Skatebird, Some("SkateBIRD_Data")) => {
            status: Live,
            gate: true,
            about: r#"SkateBIRD's data folder (`SkateBIRD_Data`, a Unity 2019.4.20f1 player: SerializedFile version 21 containers), parsed by tests/serialized_census.rs; the SkateBIRD tests of primase read it too ([worlds/skatebird/tests/markers.rs](https://github.com/Kalmarv/primase/tree/main/worlds/skatebird/tests/markers.rs))"#,
            source: r#"the user's own install"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Skatebird, "census.json") => {
            status: Live,
            gate: true,
            about: r#"an independent UnityPy object-table census of that folder (JSON; its schema is the `Census` struct of tests/serialized_census.rs)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py"#,
        },
        Query::Install(Game::Peak, Some("PEAK_Data")) => {
            status: Live,
            gate: true,
            about: r#"PEAK's data folder (`PEAK_Data`: the loose player files and `StreamingAssets/maps`, the level AssetBundles), whose level 207 tests/bundles.rs reads with its bundle"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Sledding, Some("Sledding Game_Data")) => {
            status: Live,
            gate: true,
            about: r#"Sledding Game's data folder (`Sledding Game_Data`: nine loose serialized files, two of them in `Resources`, and the Addressables bundles in `StreamingAssets/aa`), read by tests/bundles.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Peak, Some("PEAK_Data/StreamingAssets/maps")) => {
            status: Live,
            gate: true,
            about: r#"PEAK's AssetBundles (`PEAK_Data/StreamingAssets/maps`: seven `levels*.assetbundle` of 208 level scenes and the `maps` manifest bundle), read by tests/bundles.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Sledding, Some("Sledding Game_Data/StreamingAssets/aa/StandaloneWindows64")) => {
            status: Live,
            gate: true,
            about: r#"Sledding Game's 50 Addressables bundles (`StreamingAssets/aa/StandaloneWindows64`), read by tests/bundles.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Bigwalk, Some("Big Walk_Data")) => {
            status: Live,
            gate: true,
            about: r#"Big Walk's data folder (`Big Walk_Data`, whose `data.unity3d` is the player's own files in one bundle), read by tests/bundles.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Gungeon, Some("EtG_Data")) => {
            status: Live,
            gate: true,
            about: r#"Enter the Gungeon's data folder (`EtG_Data`, a Unity 2017.4.27f1 player: five SerializedFile version 17 containers), parsed by tests/serialized_census.rs, opened by tests/player_settings.rs and, with its bundles, by tests/clips.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Gungeon, "census.json") => {
            status: Live,
            gate: true,
            about: r#"an independent UnityPy object-table census of that folder (JSON; its schema is the `Census` struct of tests/serialized_census.rs)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py"#,
        },
        Query::Install(Game::Gungeon, Some("EtG_Data/StreamingAssets/Assets")) => {
            status: Live,
            gate: true,
            about: r#"Enter the Gungeon's 30 AssetBundles (`EtG_Data/StreamingAssets/Assets` and its `dungeons` folder: UnityFS format 6, SerializedFile version 17 with stored type trees), read by tests/bundles.rs and tests/clips.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Gungeon, "bundles.jsonl") => {
            status: Live,
            gate: true,
            about: r#"an independent census of Enter the Gungeon's bundles (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Peak, "bundles.jsonl") => {
            status: Live,
            gate: true,
            about: r#"an independent census of PEAK's bundles (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Sledding, "bundles.jsonl") => {
            status: Live,
            gate: true,
            about: r#"an independent census of Sledding Game's bundles (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Bigwalk, "bundles.jsonl") => {
            status: Live,
            gate: true,
            about: r#"an independent census of Big Walk's `data.unity3d` (JSON lines; tests/oracles/unitypy_bundles.py makes it, tests/bundles.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) and the script's own bundle reader"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Skatebird, "player-settings.json") => {
            status: Live,
            gate: true,
            about: r#"the colour space SkateBIRD's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Sledding, "player-settings.json") => {
            status: Live,
            gate: true,
            about: r#"the colour space Sledding Game's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Peak, "player-settings.json") => {
            status: Live,
            gate: true,
            about: r#"the colour space PEAK's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Bigwalk, "player-settings.json") => {
            status: Live,
            gate: true,
            about: r#"the colour space Big Walk's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Gungeon, "player-settings.json") => {
            status: Live,
            gate: true,
            about: r#"the colour space Enter the Gungeon's PlayerSettings states (`m_ActiveColorSpace`, JSON; tests/oracles/unitypy_player_settings.py makes it, tests/player_settings.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_player_settings.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Peak, "urp.json") => {
            status: Live,
            gate: true,
            about: r#"PEAK's Universal Render Pipeline objects: its pipeline asset's renderers and settings, the renderers' fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Sledding, "urp.json") => {
            status: Live,
            gate: true,
            about: r#"Sledding Game's Universal Render Pipeline objects: its pipeline assets' renderers and settings, the renderer's fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Bigwalk, "urp.json") => {
            status: Live,
            gate: true,
            about: r#"Big Walk's Universal Render Pipeline objects, read from its `data.unity3d`: its pipeline assets' renderers and settings, the renderers' fields and features, and every camera's URP data in its scene files (JSON; tests/oracles/unitypy_urp.py makes it, tests/urp.rs compares)"#,
            source: r#"independent: UnityPy (https://github.com/K0lb3/UnityPy) for the objects and scripts, decoded by tests/oracles/unitypy_urp.py"#,
        },
        Query::Install(Game::Ror2, Some("Risk of Rain 2_Data")) => {
            status: Live,
            gate: true,
            about: r#"Risk of Rain 2's data folder (`Risk of Rain 2_Data`, a Unity 2021.3.45f2 player), whose textures and legacy clips tests/textures.rs and tests/clips.rs read with its bundles"#,
            source: r#"the user's own install"#,
        },
        Query::Install(Game::Ror2, Some("Risk of Rain 2_Data/StreamingAssets/aa")) => {
            status: Live,
            gate: true,
            about: r#"Risk of Rain 2's Addressables bundles (`Risk of Rain 2_Data/StreamingAssets/aa`, where its crunched DXT1 and DXT5 textures are), read by tests/textures.rs and tests/clips.rs"#,
            source: r#"the user's own install"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Ror2, "textures.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of every Texture2D of Risk of Rain 2's player and bundles: name, format, size, mips, pixel byte length and, crunched, the SHA-256 of its first level as RGBA (JSON; tests/oracles/unitypy_textures.py makes it, tests/textures.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy) and its texture2ddecoder crunch decoder, run by tests/oracles/unitypy_textures.py"#,
        },
        Query::Install(Game::Schedule1, Some("Schedule I_Data")) => {
            status: Live,
            gate: true,
            about: r#"Schedule I's data folder (`Schedule I_Data`, a Unity 2022.3.62f2 player of loose files, with crunched DXT5 textures and weighted legacy clip keys), whose textures and legacy clips tests/textures.rs and tests/clips.rs read"#,
            source: r#"the user's own install"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Schedule1, "textures.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of every Texture2D of Schedule I's player: name, format, size, mips, pixel byte length and, crunched, the SHA-256 of its first level as RGBA (JSON; tests/oracles/unitypy_textures.py makes it, tests/textures.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy) and its texture2ddecoder crunch decoder, run by tests/oracles/unitypy_textures.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Gungeon, "clips.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of every legacy AnimationClip of Enter the Gungeon's player and bundles: name, wrap mode, events, and each curve's kind, path, attribute, class, key count and the SHA-256 of its keys (JSON; tests/oracles/unitypy_clips.py makes it, tests/clips.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_clips.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Ror2, "clips.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of every legacy AnimationClip of Risk of Rain 2's player and bundles: name, wrap mode, events, and each curve's kind, path, attribute, class, key count and the SHA-256 of its keys (JSON; tests/oracles/unitypy_clips.py makes it, tests/clips.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_clips.py"#,
        },
        Query::Oracle(OracleSet::UnityPy, Game::Schedule1, "clips.json") => {
            status: Live,
            gate: true,
            about: r#"UnityPy's reading of every legacy AnimationClip of Schedule I's player: name, wrap mode, events, and each curve's kind, path, attribute, class, key count and the SHA-256 of its keys (JSON; tests/oracles/unitypy_clips.py makes it, tests/clips.rs compares)"#,
            source: r#"independent: UnityPy 1.25.4 (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_clips.py"#,
        },
    }
    debug {}
}
