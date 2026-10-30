# Changelog

## Unreleased

- **Breaking:** `pipeline::read_urp_settings` takes the asset's layout (`pipeline::UrpAssetLayout`), which `UrpAssetLayout::of` identifies from the player's Unity version before any field is read, and reads every field of it to the asset's last byte; `Player::urp_settings` reads an asset of a player so. The layout is Universal RP 17.3's, verified on the Unity 6000.3.12f1 (Sledding Game) and 6000.3.15f1 (PEAK) players, whose every asset reads to exactly its bytes; a player of another version is refused by name. Big Walk's (6000.3.17f1) assets differ after `m_ReflectionProbeBlending`, where the reader used to read a value no bool holds.
- **Breaking:** `pipeline::UnityUrpSettings` has the asset's renderers (`renderers`, `m_RendererDataList`), the default renderer's index, whether it makes the depth and opaque textures and its MSAA sample count; a default renderer index outside the list and an MSAA count other than 1, 2, 4 or 8 are refused.
- `pipeline::read_urp_renderer` reads a `UniversalRendererData` (`UnityUrpRenderer`: its features and feature map, layer masks for the prepass, opaque and transparent draws, default stencil state, rendering, depth priming and depth copy modes, depth formats), `read_urp_feature_active` a renderer feature's `m_Active`, `read_urp_decal_feature` and `read_urp_full_screen_feature` the Decal and Full Screen Pass features' settings, and `read_urp_camera` a camera's `UniversalAdditionalCameraData` (`UnityUrpCamera`: its texture options, stack, renderer index, post-processing, anti-aliasing and TAA settings). The layouts are Universal RP 17.3's (renderer asset version 3, Full Screen Pass version 1, camera version 2); another version, an enum value outside the class's or a length the layout does not account for is refused. PEAK's, Sledding Game's and Big Walk's renderers, Decal and Full Screen Pass features and every camera in their scene files read as UnityPy reads them (`tests/urp.rs`, `tests/oracles/unitypy_urp.py`).
- `player::PPtr` is an object reference as stored and `Player::resolve_pptr` follows one (a script's fields store references as bytes); `Player::resolve` reads a decoded `PPtr` and follows it the same way. `Player::script_class` names a MonoBehaviour's script class.
- **Breaking:** `scene::CameraComponent` has its GameObject (`game_object`), whose MonoBehaviours hold a URP camera's data.

## 0.2.2 (2026-10-29)

- `program::colour_properties` reads the colour properties a shader declares (`m_ParsedForm.m_PropInfo.m_Props`, type 0) with their `m_Flags` (`ColourProperty`; `ColourProperty::hdr`, `PROPERTY_HDR`: `ShaderPropertyFlags.HDR`, 16). A linear colour space player hands an `[HDR]` colour's stored value to its programs as it is and converts any other colour from gamma to linear, so the flag decides what a stored colour is. PEAK's `W/Peak_Standard` declares every colour `[HDR]`; URP Lit `_BaseColor` `[MainColor]` (256) and `_EmissionColor` `[HDR]`; built-in Standard none, as UnityPy reads them (`tests/shader_properties.rs`, `tests/oracles/unitypy_shader_properties.py`).

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
