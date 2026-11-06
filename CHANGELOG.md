# Changelog

## 0.4.0 (2026-11-06)

- **Breaking:** `bundle::FORMAT_VERSION` is `bundle::FORMAT_VERSIONS` (6 to 8). UnityFS bundles of formats 6 and 7, LZMA
  compressed block tables and blocks (Unity's form: the five property bytes and the raw stream), and the block table at
  the end of the file (header flag `0x80`, the blocks right after the header) are read as UnityPy and AssetStudio read
  them; before, each was refused. A format 6 header is not aligned to 16, format 7 and 8 headers are. LZHAM is still
  refused. Bundles built in the tests in every combination read the same bytes, and UnityPy's own format 6 test
  bundles (Unity 5.6.7 to 2018.4) read every node to the bytes UnityPy reads.
- Type tree packages compressed with LZMA or Brotli are read (`TypeTreePackage::parse`, compressions 2 and 3); before,
  they were refused. AssetRipper's LZMA and Brotli packages (its Tpk CI's artifacts of 2026-11-01) decode to the same
  1,538,989 bytes as its uncompressed one.
- `ShaderPrograms::variant_parameters` gives the parameters a variant is compiled with wherever the player keeps them: the variant's own (before 2021.2), the pass's common ones merged with its parameter blob entry (2021.2 on), else the pass's.
- The `program_keys` example lists every D3D11 variant of a player's Shaders as a line of JSON that `locum-cli index --programs` reads: the XXH3-128 key of its DXBC container, its name (`<shader> <subshader>.<pass> <stage> [<keywords>]`), and its constant buffers, textures and texture samplers in the shape of locum's shader reflection, so a frame captured from the game names the programs, constants and textures Unity strips from the bytecode. 90 of the 93 programs of a SkateBIRD (Unity 2019.4) title-screen capture and 26 of the 28 of a Hollow Knight (Unity 6000.0) one match a listed key; the rest are the Steam overlay's and a ComputeShader, which the example does not read.
- The `mesh_keys` example lists the XXH3-128 keys of every Mesh's vertex streams, whole vertex data and index buffer in a player, as `key<TAB>bytes<TAB>name` lines that `locum-cli index --keys` reads, so the buffers of a frame captured from the game are named by the meshes they came from. A stream's stride is the sum of its channels' sizes by Unity 2019 on's vertex formats and each stream starts 16-byte aligned; every one of SkateBIRD's (Unity 2019.4) 7,491 and Hollow Knight's (Unity 6000.0) 3,941 meshes lays out to its stored bytes so, and a mesh that does not is reported and keyed whole. A SkateBIRD title-screen capture (D3D11) has 183 of its 209 buffers named this way, and Hollow Knight's D3D12 captures name the built-in `Plane`'s vertex and index buffers.

## 0.3.1 (2026-10-30)

- `pipeline::read_urp_global_settings` and `Player::urp_global_settings` read Universal RP's global settings (`UnityUrpGlobalSettings`: the object GraphicsSettings' `m_CurrentRenderPipelineGlobalSettings` names, `m_AssetVersion` 10) to the last byte, with every `[SerializeReference]` graphics settings entry of its managed-reference registry in the settings list's order (`UnityGraphicsSettings`, each field in declaration order). 18 classes of Universal RP 17.3, Core RP 17.3 and GPU Driven RP 17.3 have a verified layout (`pipeline::verified_graphics_settings`); another class, another class version, an entry the list and the registry do not both hold or non-empty probe volume scene data is refused by name. `UnityUrpGlobalSettings::renderer_resources` gives the Universal Renderer's runtime shaders (`UnityUrpRendererResources`: CopyDepth and DBufferClear among them), `render_compatibility_mode` the stored Compatibility Mode. PEAK's and Sledding Game's settings read to their last byte with the values of an independent reading of the same bytes; Big Walk's hold a class of the game's own and are refused (`tests/urp.rs`).
- `Player::urp_compatibility_code`: whether the player's Universal RP is compiled with `URP_COMPATIBILITY_MODE`, without which it ignores a stored Compatibility Mode and always draws through the render graph, read from whether its code holds the log line the pipeline's constructor writes only under the define (`player::URP_COMPATIBILITY_LOG`) in the Mono assembly or the IL2CPP metadata. Sledding Game's does, PEAK's and Big Walk's do not.

## 0.3.0 (2026-10-30)

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
