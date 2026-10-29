# Changelog

## 0.2.1 (2026-10-29)

- `Player::colour_space` reads the colour space a player's programs and textures work in, `player::ColourSpace` (`Gamma` or `Linear`), from its PlayerSettings' `m_ActiveColorSpace`; `None` for a player with no PlayerSettings. It reads that one field: for the Unity 6 players read (Sledding Game, PEAK, Big Walk) the package's PlayerSettings layout is 4 bytes short of the stored object, as UnityPy's is, so the whole object does not decode. SkateBIRD's, Sledding Game's, PEAK's and Big Walk's players are all `Linear`, as UnityPy reads them (`tests/player_settings.rs`).

## 0.2.0 (2026-10-29)

- **Breaking:** `animator::SelectorTransition::destination` is an `Option<u32>`: `None` for a stored destination of `0xFFFFFFFF` (`animator::NO_DESTINATION`), which names no state or selector. PEAK's `Ambience` and `Scout` controllers have a layer with no states whose entry node's one transition stores it, and the controller check refused them (`read_level` of a PEAK level stopped there). The check takes `None` on an entry node's transition and refuses it on an exit node's.

- **Breaking:** `SerializedFile::bytes` returns `Cow<[u8]>` (a file read from a bundle reads an object's bytes from the bundle when asked).
- UnityFS bundles: `bundle::Bundle` reads format 8 bundles (stored, LZ4 and LZ4HC blocks) and their nodes by range; `Player::open_with_bundles` reads a bundle's serialized files beside the loose ones (`BundleSpec`, `NodeSelection`), `Player::read_stream` reads streamed resources from the player's folder or the file's bundle.
- SerializedFiles that store their type trees are read: `SerializedType::tree`, `SerializedFile::parse_with` (common strings from the package for the file's Unity version, `TypeTreePackage::common_strings`), `SerializedFile::from_bundle`, `Player::script_fields`. A file's own tree decodes its objects (and names its classes) before the package's.
- A streamed AnimationClip whose well-formed frames name a curve past the stated curve count is `Error::Unsupported` (was `Error::Invalid`), so a world that skips clips it does not read keeps going.

## 0.1.0 (2026-10-27)

- First release: Unity players' SerializedFiles, scenes, meshes, textures, terrains, shader programs and Animator data.
