# Contributing

## Build and test

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The toolchain is pinned in `rust-toolchain.toml`. The unit tests and the tests on synthetic bytes need no game files. The test that reads real games (`tests/serialized_census.rs`) skips with a line naming its input unless the `GAME_DATA` environment variable names a configuration file; [docs/testing.md](docs/testing.md) says how to set that up, and [tests/inputs/README.md](tests/inputs/README.md) lists the inputs.

The code blocks in README.md are compiled as doc tests, and `examples/inspect_player.rs` is a runnable version of the README example. When you add a real-data input, declare it in `tests/inputs/mod.rs` and update the generated tables in `tests/inputs/README.md` (the `real_data_inputs` test prints the expected tables when they differ).

## Releasing

1. Raise the version (a patch, 0.x.Y, for fixes and data or manifest additions; a minor, 0.X.0, for new API or any
   breaking change before 1.0; then `cargo check`, so `Cargo.lock` follows) and put the change's CHANGELOG lines under a
   `## <version> (<date>)` heading, in one commit.
2. Push it to main. Once every check has passed, release-plz publishes each crate whose version crates.io does not have
   yet (trusted publishing: no token is stored anywhere), tags the commit `v<version>` and creates the GitHub Release
   with that CHANGELOG section as its notes.

A commit that raises no version only runs CI. Never release by hand.

A new crate's first version can't come from trusted publishing: the token holder publishes it by hand, then adds its
trusted-publishing config (workflow `ci.yml`).

## Reporting a file this library cannot read

Open an issue with the "unreadable file" template. It asks for:

- the commit or version of this crate, and your platform;
- the game and its Unity version (the string in the file's header, e.g. `2022.3.14f1`, which `SerializedFile::parse` reports as `unity_version`), and whether the file is a level, `*.assets`, `globalgamemanagers` or a streamed `.resS`;
- the file's name and size, and which call refused it: the code you ran, or `cargo run --example inspect_player`;
- the exact error message, which says what the reader found.

Do not attach game files, extracted assets, type tree packages made from a game, or anything else taken from a game: they are not ours to redistribute, and the report does not need them. Describe the file instead. If a few bytes decide it, quote them as hex (the first 48 bytes of a SerializedFile are its header).

## Changing the code

- Keep public items documented, and keep refusals naming what was found and where.
- Library code (outside tests) denies `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::cast_possible_truncation` (`src/lib.rs`): read with `get`, `checked_*`/`saturating_*` and `try_from`, and report a failure as an `Error` naming the offset.
- Reads of untrusted lengths and indices are checked against the bytes that exist before anything is allocated for them; add a test with synthetic bytes next to behaviour you change.
- A module's docs say what it refuses; lifting a refusal changes those docs and [docs/limitations.md](docs/limitations.md) too.

## File access

Library code works on bytes it is given. Only `src/player.rs` (opening a data folder), `src/resource.rs` (reading a `.resS` file) and `src/bundle.rs` (opening a bundle and reading blocks from it) touch the file system, and they opt out of the `disallowed_methods` and `disallowed_types` lints in `clippy.toml` with a reason; the rest of the crate denies them. Library code reads no environment variables and starts no process.
