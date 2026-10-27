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
    }
    installs {
        Skatebird {
            about: "SkateBIRD's install folder (`SkateBIRD_Data`: a Unity 2019.4.20f1 player)",
            source: "the user's own install",
            gate: true,
        },
    }
    keys {}
    tools {}
    oracle_sets {
        UnityPy = "unitypy" {
            about: "UnityPy's readings of a Unity player (the object-table census these tests use; tests/oracles/unitypy_census.py makes it)",
            source: "independent: UnityPy (https://github.com/K0lb3/UnityPy), run by tests/oracles/unitypy_census.py",
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
    }
    debug {}
}
