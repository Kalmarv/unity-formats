# Changelog

## 0.2.0 (2026-10-28)

- **Breaking:** `SerializedFile::bytes` returns `Cow<[u8]>` (a file read from a bundle reads an object's bytes from the bundle when asked).
- UnityFS bundles: `bundle::Bundle` reads format 8 bundles (stored, LZ4 and LZ4HC blocks) and their nodes by range; `Player::open_with_bundles` reads a bundle's serialized files beside the loose ones (`BundleSpec`, `NodeSelection`), `Player::read_stream` reads streamed resources from the player's folder or the file's bundle.
- SerializedFiles that store their type trees are read: `SerializedType::tree`, `SerializedFile::parse_with` (common strings from the package for the file's Unity version, `TypeTreePackage::common_strings`), `SerializedFile::from_bundle`, `Player::script_fields`. A file's own tree decodes its objects (and names its classes) before the package's.
- A streamed AnimationClip whose well-formed frames name a curve past the stated curve count is `Error::Unsupported` (was `Error::Invalid`), so a world that skips clips it does not read keeps going.

## 0.1.0 (2026-10-27)

- First release: Unity players' SerializedFiles, scenes, meshes, textures, terrains, shader programs and Animator data.
