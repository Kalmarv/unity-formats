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
        UnityPy = "unitypy" {
            about: "UnityPy's readings of a Unity player (the object-table census these tests use; tests/oracles/unitypy_census.py makes it) and of its bundles (tests/oracles/unitypy_bundles.py)",
            source: "independent: UnityPy (https://github.com/K0lb3/UnityPy), run by the scripts in tests/oracles",
        },
    }
    declared {
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
    }
    debug {}
}
