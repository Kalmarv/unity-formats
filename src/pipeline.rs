//! The Universal Render Pipeline's objects, decoded from the bytes of their MonoBehaviours (a build has no type tree for
//! a script's fields): a pipeline asset's renderers and its lighting, shadow and texture settings
//! ([`read_urp_settings`]), a renderer's features, layer masks and modes ([`read_urp_renderer`]), a renderer feature's
//! `m_Active` ([`read_urp_feature_active`]) and the settings of the Decal and Full Screen Pass features
//! ([`read_urp_decal_feature`], [`read_urp_full_screen_feature`]), a camera's URP data ([`read_urp_camera`]), and the
//! pipeline's global settings with the graphics settings they hold: its runtime shaders and its render graph settings
//! ([`read_urp_global_settings`]).
//!
//! The field order is each class's declaration order in the Universal RP package. A field is a 4-byte value, or a
//! bool padded to 4 bytes; an object reference is 12 bytes (a [`PPtr`]: file id and a 64-bit path id) and an array is
//! its count and elements. A value outside what its field can hold is refused as a sign of an unverified layout.
//!
//! - The asset's layout ([`UrpAssetLayout`]) is Universal RP 17.3's field order, read to the asset's last byte, and is
//!   identified by the Unity version of the player before any field is read: it is verified on the players of Unity
//!   6000.3.12f1 (Sledding Game) and 6000.3.15f1 (PEAK), each of whose assets reads to exactly its bytes, with the
//!   renderers and settings an independent reading makes (tests/urp.rs) and the settings the keywords of the shader
//!   variants the player stores agree with (as
//!   [this test of primase](https://github.com/Kalmarv/primase/tree/main/worlds/unity/tests/resolve_shaders.rs)
//!   compares). A player of another Unity version is refused by name: Big Walk's (6000.3.17f1) assets differ after
//!   `m_ReflectionProbeBlending`. It holds for `k_AssetVersion` 13 only; another version is refused.
//! - The renderer, feature and camera layouts are those of Universal RP 17.3 (the package of Unity 6000.3), checked
//!   against PEAK's, Sledding Game's and Big Walk's players: every renderer, every Decal and Full Screen Pass feature
//!   and every camera of their scenes reads to exactly its bytes, with the fields an independent reading makes
//!   (tests/urp.rs). A length the layout does not account for is refused, and so is a renderer `m_AssetVersion`, a
//!   Full Screen Pass `m_Version` or a camera `m_Version` other than the one read.
//! - The global settings' layout (`m_AssetVersion` 10) and its graphics settings classes' are Universal RP 17.3's, Core
//!   RP 17.3's and GPU Driven RP 17.3's, checked against PEAK's and Sledding Game's players: each object reads to exactly
//!   its bytes, every entry of its managed-reference registry included (tests/urp.rs). A class whose layout is not
//!   verified is refused by name (Big Walk's settings hold one of the game's own), as is another asset or class version.

use crate::player::PPtr;
use crate::reader::Reader;
use crate::version::UnityVersion;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::{Result, invalid};

/// The settings of a Universal Render Pipeline asset as stored (`UniversalRenderPipelineAsset`, asset version 13; this
/// module's docs state the verified layout): its renderers and the one cameras use by default, whether it makes the
/// depth and opaque textures, its MSAA sample count, and the lighting and shadow settings that decide which of a
/// pass's keywords the pipeline sets: the light modes as `LightRenderingMode` numbers them (0 disabled, 1 per pixel,
/// 2 per vertex) and the shadow cascade count (1 to 4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpSettings {
    /// The renderers (`m_RendererDataList`), each a `ScriptableRendererData` asset, in the order a camera's renderer
    /// index counts them.
    pub renderers: Vec<PPtr>,
    /// The index in [`UnityUrpSettings::renderers`] of the renderer a camera with renderer index -1 draws with
    /// (`m_DefaultRendererIndex`).
    pub default_renderer: i64,
    /// Whether the pipeline makes the camera depth texture (`m_RequireDepthTexture`), for a camera whose depth texture
    /// option is the pipeline's setting.
    pub require_depth_texture: bool,
    /// Whether the pipeline makes the camera opaque texture (`m_RequireOpaqueTexture`), likewise.
    pub require_opaque_texture: bool,
    /// The MSAA sample count (`m_MSAA`: 1, 2, 4 or 8).
    pub msaa: i64,
    /// The main light's rendering mode (`m_MainLightRenderingMode`).
    pub main_light_mode: i64,
    /// Whether the main light casts shadows.
    pub main_light_shadows: bool,
    /// The additional lights' rendering mode (`m_AdditionalLightsRenderingMode`).
    pub additional_lights_mode: i64,
    /// Whether additional lights cast shadows.
    pub additional_light_shadows: bool,
    /// Whether reflection probes blend.
    pub reflection_probe_blending: bool,
    /// Whether reflection probes use box projection.
    pub reflection_probe_box_projection: bool,
    /// Whether reflection probes are packed into an atlas.
    pub reflection_probe_atlas: bool,
    /// `m_ShadowDistance`.
    pub shadow_distance: f32,
    /// `m_ShadowCascadeCount`.
    pub shadow_cascades: u8,
    /// Whether soft shadows are supported.
    pub soft_shadows: bool,
    /// Whether mixed lighting is supported.
    pub mixed_lighting: bool,
    /// Whether light cookies are supported.
    pub light_cookies: bool,
    /// Whether light layers are supported.
    pub light_layers: bool,
    /// Whether LOD cross-fading is enabled.
    pub lod_cross_fade: bool,
    /// `ShEvalMode`: 0 auto, 1 per vertex, 2 mixed, 3 per pixel.
    pub sh_eval_mode: i64,
    /// `LightProbeSystem`: 0 legacy light probes, 1 probe volumes.
    pub light_probe_system: i64,
}

/// The asset version the layout below is verified for.
const ASSET_VERSION: i32 = 13;

/// A serialized layout of `UniversalRenderPipelineAsset` whose every field is verified (this module's docs say on
/// which players).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UrpAssetLayout {
    /// Universal RP 17.3's, as Unity 6000.3.12f1 and 6000.3.15f1 players store it.
    Urp17_3,
}

/// The Unity versions whose players' assets are verified to have [`UrpAssetLayout::Urp17_3`].
const URP_17_3_PLAYERS: [&str; 2] = ["6000.3.12f1", "6000.3.15f1"];

impl UrpAssetLayout {
    /// The layout of the assets a player of Unity `unity` stores (its serialized files' version); a version on whose
    /// players no layout is verified is refused, naming it.
    pub fn of(unity: UnityVersion) -> Result<Self> {
        for v in URP_17_3_PLAYERS {
            if UnityVersion::parse(v)? == unity {
                return Ok(UrpAssetLayout::Urp17_3);
            }
        }
        Err(invalid(format!(
            "a Universal Render Pipeline asset of a Unity {unity} player: its layout is verified only on the players \
             of Unity {} (the package's fields are not recorded in the asset, and other players store others)",
            URP_17_3_PLAYERS.join(" and ")
        )))
    }
}

/// The settings of the asset whose script fields are `bytes` (the bytes after the MonoBehaviour's base fields), stored
/// in `layout` ([`UrpAssetLayout::of`] the player's Unity version; [`crate::player::Player::urp_settings`] reads an
/// asset of a player so). Every field is read, to the last byte.
pub fn read_urp_settings(bytes: &[u8], layout: UrpAssetLayout) -> Result<UnityUrpSettings> {
    match layout {
        UrpAssetLayout::Urp17_3 => {}
    }
    let mut r = Reader::new(bytes, false);
    let version = r.i32()?;
    if version != ASSET_VERSION {
        return Err(invalid(format!(
            "Universal Render Pipeline asset version {version}: only version {ASSET_VERSION} has a verified layout"
        )));
    }
    r.i32()?; // k_AssetPreviousVersion
    r.i32()?; // m_RendererType
    r.take(12)?; // m_RendererData (a reference)
    let renderers = pptrs(&mut r, "m_RendererDataList")?;
    let default_renderer = r.i32()?;
    let require_depth_texture = flag(&mut r)?;
    let require_opaque_texture = flag(&mut r)?;
    r.i32()?; // m_OpaqueDownsampling
    r.take(8)?; // m_SupportsTerrainHoles, m_SupportsHDR
    r.i32()?; // m_HDRColorBufferPrecision
    let msaa = r.i32()?;
    r.f32()?; // m_RenderScale
    r.i32()?; // m_UpscalingFilter
    r.take(4)?; // m_FsrOverrideSharpness
    r.f32()?; // m_FsrSharpness
    let lod_cross_fade = flag(&mut r)?;
    r.i32()?; // m_LODCrossFadeDitheringType
    let sh_eval_mode = r.i32()?;
    let light_probe_system = r.i32()?;
    r.i32()?; // m_ProbeVolumeMemoryBudget
    r.i32()?; // m_ProbeVolumeBlendingMemoryBudget
    r.take(16)?; // the four probe volume streaming and scenario flags
    r.i32()?; // m_ProbeVolumeSHBands
    let main_light_mode = r.i32()?;
    let main_light_shadows = flag(&mut r)?;
    r.i32()?; // m_MainLightShadowmapResolution
    let additional_lights_mode = r.i32()?;
    r.i32()?; // m_AdditionalLightsPerObjectLimit
    let additional_light_shadows = flag(&mut r)?;
    r.take(16)?; // the additional lights' shadowmap resolution and its three tiers
    let reflection_probe_blending = flag(&mut r)?;
    let reflection_probe_box_projection = flag(&mut r)?;
    let reflection_probe_atlas = flag(&mut r)?;
    let shadow_distance = r.f32()?;
    let shadow_cascades = r.i32()?;
    r.take(4 + 8 + 12 + 4 + 4 + 4)?; // the cascade splits and border, the depth and normal bias
    let soft_shadows = flag(&mut r)?;
    r.take(4)?; // m_ConservativeEnclosingSphere
    r.i32()?; // m_NumIterationsEnclosingSphere
    r.i32()?; // m_SoftShadowQuality
    r.i32()?; // m_AdditionalLightsCookieResolution
    r.i32()?; // m_AdditionalLightsCookieFormat
    r.take(8)?; // m_UseSRPBatcher, m_SupportsDynamicBatching
    let mixed_lighting = flag(&mut r)?;
    let light_cookies = flag(&mut r)?;
    let light_layers = flag(&mut r)?;
    r.i32()?; // m_DebugLevel
    r.i32()?; // m_StoreActionsOptimization
    flag(&mut r)?; // m_UseAdaptivePerformance
    r.i32()?; // m_ColorGradingMode
    r.i32()?; // m_ColorGradingLutSize
    for _ in 0..4 {
        // m_AllowPostProcessAlphaOutput, m_UseFastSRGBLinearConversion and the two lens flare flags
        flag(&mut r)?;
    }
    r.i32()?; // m_GPUResidentDrawerMode
    r.f32()?; // m_SmallMeshScreenPercentage
    flag(&mut r)?; // m_GPUResidentDrawerEnableOcclusionCullingInCameras
    r.i32()?; // m_ShadowType
    flag(&mut r)?; // m_LocalShadowsSupported
    r.take(12)?; // m_LocalShadowsAtlasResolution, m_MaxPixelLights, m_ShadowAtlasResolution
    r.i32()?; // m_VolumeFrameworkUpdateMode
    pptr(&mut r)?; // m_VolumeProfile
    // apvScenesData: two SerializedDictionary, each its keys' list and its values' list (scene GUIDs to bounds, and
    // to whether the scene has probe volumes)
    strings(&mut r, "apvScenesData.obsoleteSceneBounds.m_Keys")?;
    let n = list_len(&mut r, "apvScenesData.obsoleteSceneBounds.m_Values", 24)?;
    for _ in 0..n {
        r.take(24)?; // a Bounds: its centre and extents
    }
    strings(&mut r, "apvScenesData.obsoleteHasProbeVolumes.m_Keys")?;
    let n = list_len(&mut r, "apvScenesData.obsoleteHasProbeVolumes.m_Values", 1)?;
    for _ in 0..n {
        r.bool()?;
    }
    r.align(4)?;
    r.i32()?; // m_ShaderVariantLogLevel
    r.i32()?; // m_ShadowCascades
    r.take(24)?; // m_Textures: its two textures
    used_up(&r, "UniversalRenderPipelineAsset")?;
    // a layout that does not hold reads values outside what the fields can be
    let in_range = |what: &str, ok: bool| -> Result<()> {
        ok.then_some(()).ok_or_else(|| {
            invalid(format!(
                "Universal Render Pipeline asset: {what} out of range (an unverified layout)"
            ))
        })
    };
    in_range("m_ShadowCascadeCount", (1..=4).contains(&shadow_cascades))?;
    in_range(
        "m_ShadowDistance",
        shadow_distance.is_finite() && shadow_distance >= 0.0,
    )?;
    in_range("m_MainLightRenderingMode", (0..=2).contains(&main_light_mode))?;
    in_range(
        "m_AdditionalLightsRenderingMode",
        (0..=2).contains(&additional_lights_mode),
    )?;
    in_range("m_ShEvalMode", (0..=3).contains(&sh_eval_mode))?;
    in_range("m_MSAA", [1, 2, 4, 8].contains(&msaa))?;
    in_range(
        "m_DefaultRendererIndex",
        usize::try_from(default_renderer).is_ok_and(|i| i < renderers.len()),
    )?;
    Ok(UnityUrpSettings {
        renderers,
        default_renderer: i64::from(default_renderer),
        require_depth_texture,
        require_opaque_texture,
        msaa: i64::from(msaa),
        main_light_mode: i64::from(main_light_mode),
        main_light_shadows,
        additional_lights_mode: i64::from(additional_lights_mode),
        additional_light_shadows,
        reflection_probe_blending,
        reflection_probe_box_projection,
        reflection_probe_atlas,
        shadow_distance,
        shadow_cascades: u8::try_from(shadow_cascades).map_err(|_| {
            invalid(format!(
                "Universal Render Pipeline asset: m_ShadowCascadeCount {shadow_cascades} out of range"
            ))
        })?,
        soft_shadows,
        mixed_lighting,
        light_cookies,
        light_layers,
        lod_cross_fade,
        sh_eval_mode: i64::from(sh_eval_mode),
        light_probe_system: i64::from(light_probe_system),
    })
}

/// A bool, padded to 4 bytes.
fn flag(r: &mut Reader) -> Result<bool> {
    let b = r.bool()?;
    r.align(4)?;
    Ok(b)
}

/// A list of strings, each its byte count and its UTF-8 bytes padded to 4 bytes.
fn strings(r: &mut Reader, what: &str) -> Result<Vec<String>> {
    let n = list_len(r, what, 4)?;
    (0..n)
        .map(|_| {
            let len = list_len(r, what, 1)?;
            let s = std::str::from_utf8(r.take(len)?)
                .map_err(|_| invalid(format!("{what}: a string that is not UTF-8")))?
                .to_owned();
            r.align(4)?;
            Ok(s)
        })
        .collect()
}

/// An object reference: its file id and path id.
fn pptr(r: &mut Reader) -> Result<PPtr> {
    Ok(PPtr {
        file_id: r.i32()?,
        path_id: r.i64()?,
    })
}

/// A list's count, refused when its `size`-byte elements would run past the bytes left (`what` names the list).
fn list_len(r: &mut Reader, what: &str, size: usize) -> Result<usize> {
    let n = r.u32()?;
    usize::try_from(n)
        .ok()
        .filter(|&n| n.checked_mul(size).is_some_and(|len| len <= r.remaining()))
        .ok_or_else(|| {
            invalid(format!(
                "{what}: {n} entries of {size} bytes, {} bytes left",
                r.remaining()
            ))
        })
}

/// A list of object references.
fn pptrs(r: &mut Reader, what: &str) -> Result<Vec<PPtr>> {
    let n = list_len(r, what, 12)?;
    (0..n).map(|_| pptr(r)).collect()
}

/// A 4-byte enum value, refused unless it is one of `values` (`what` names the class and field).
fn choice(r: &mut Reader, what: &str, values: &[i32]) -> Result<i64> {
    let v = r.i32()?;
    if values.contains(&v) {
        Ok(i64::from(v))
    } else {
        Err(invalid(format!(
            "{what} {v} (the values: {values:?}; an unverified layout)"
        )))
    }
}

/// The values `0..n` of an enum.
fn first(n: i32) -> Vec<i32> {
    (0..n).collect()
}

/// Refused unless the bytes were used up: a stored object longer than the layout is another layout.
fn used_up(r: &Reader, what: &str) -> Result<()> {
    match r.remaining() {
        0 => Ok(()),
        n => Err(invalid(format!(
            "{what}: {n} bytes past the verified layout's {} (another layout)",
            r.pos
        ))),
    }
}

/// The `m_AssetVersion` of `UniversalRendererData` the renderer layout is verified for.
const RENDERER_ASSET_VERSION: i32 = 3;

/// The bytes a `UniversalRendererData` stores after `m_UseNativeRenderPass` in the verified layout: `xrSystemData`,
/// `postProcessData`, the asset version, the three layer masks, the stencil state and the eight modes. A Universal RP
/// compiled without XR support (`ENABLE_VR && ENABLE_XR_MODULE` unset) stores no `xrSystemData` (84 bytes); no player
/// read so far is one, so that layout is refused.
const RENDERER_TAIL: usize = 96;

/// The renderer's default stencil state (`m_DefaultStencilState`, a `StencilStateData`): Unity's `CompareFunction`
/// (0 disabled, 1 never, 2 less, 3 equal, 4 less or equal, 5 greater, 6 not equal, 7 greater or equal, 8 always) and
/// `StencilOp` (0 keep, 1 zero, 2 replace, 3 increment saturate, 4 decrement saturate, 5 invert, 6 increment wrap,
/// 7 decrement wrap) numbers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnityStencilState {
    /// Whether the renderer's opaque and transparent draws use this state (`overrideStencilState`).
    pub override_stencil: bool,
    /// `stencilReference`.
    pub reference: i64,
    /// `stencilCompareFunction`.
    pub compare: i64,
    /// `passOperation`.
    pub pass: i64,
    /// `failOperation`.
    pub fail: i64,
    /// `zFailOperation`.
    pub z_fail: i64,
}

/// A Universal Render Pipeline renderer as stored (`UniversalRendererData`, asset version 3, Universal RP 17.3; this
/// module's docs say how the layout is verified). Its modes are numbered as the package's enums are:
///
/// - `RenderingMode`: 0 forward, 1 deferred, 2 Forward+, 3 Deferred+.
/// - `DepthPrimingMode`: 0 disabled, 1 auto, 2 forced.
/// - `CopyDepthMode`: 0 after opaques, 1 after transparents, 2 force prepass.
/// - `DepthFormat`: 0 default, 90 16-bit, 91 24-bit, 93 32-bit, 151, 92 and 94 the same with 8 stencil bits.
/// - `IntermediateTextureMode`: 0 auto, 1 always.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpRenderer {
    /// The renderer features (`m_RendererFeatures`), each a `ScriptableRendererFeature` asset, in the order the
    /// renderer asks them for passes.
    pub features: Vec<PPtr>,
    /// `m_RendererFeatureMap` as stored: the editor's ids for the features. A player does not read it, and a stored
    /// map need not match the features (Big Walk's second renderer has no features and one entry).
    pub feature_map: Vec<i64>,
    /// `m_UseNativeRenderPass`.
    pub native_render_pass: bool,
    /// `xrSystemData`.
    pub xr_system_data: PPtr,
    /// `postProcessData`.
    pub post_process_data: PPtr,
    /// The layers a depth prepass draws (`m_PrepassLayerMask`, one bit per layer).
    pub prepass_layer_mask: u32,
    /// The layers the opaque draws draw (`m_OpaqueLayerMask`).
    pub opaque_layer_mask: u32,
    /// The layers the transparent draws draw (`m_TransparentLayerMask`).
    pub transparent_layer_mask: u32,
    /// `m_DefaultStencilState`.
    pub stencil: UnityStencilState,
    /// `m_ShadowTransparentReceive`.
    pub shadow_transparent_receive: bool,
    /// `m_RenderingMode`.
    pub rendering_mode: i64,
    /// `m_DepthPrimingMode`.
    pub depth_priming_mode: i64,
    /// `m_CopyDepthMode`.
    pub copy_depth_mode: i64,
    /// `m_DepthAttachmentFormat`.
    pub depth_attachment_format: i64,
    /// `m_DepthTextureFormat`.
    pub depth_texture_format: i64,
    /// `m_AccurateGbufferNormals`.
    pub accurate_gbuffer_normals: bool,
    /// `m_IntermediateTextureMode`.
    pub intermediate_texture_mode: i64,
}

/// The renderer whose script fields are `bytes` (the bytes after the MonoBehaviour's base fields).
pub fn read_urp_renderer(bytes: &[u8]) -> Result<UnityUrpRenderer> {
    const WHAT: &str = "Universal Render Pipeline renderer";
    let mut r = Reader::new(bytes, false);
    // `debugShaders` (two shaders) and `probeVolumeResources` (seven objects)
    r.take(12 * 9)?;
    let features = pptrs(&mut r, "m_RendererFeatures")?;
    let n = list_len(&mut r, "m_RendererFeatureMap", 8)?;
    let feature_map = (0..n).map(|_| r.i64()).collect::<Result<Vec<_>>>()?;
    let native_render_pass = flag(&mut r)?;
    if r.remaining() != RENDERER_TAIL {
        return Err(invalid(format!(
            "{WHAT}: {} bytes after m_UseNativeRenderPass, the verified layout stores {RENDERER_TAIL} (84 is the layout \
             without xrSystemData, which is not verified)",
            r.remaining()
        )));
    }
    let xr_system_data = pptr(&mut r)?;
    let post_process_data = pptr(&mut r)?;
    let version = r.i32()?;
    if version != RENDERER_ASSET_VERSION {
        return Err(invalid(format!(
            "{WHAT} asset version {version}: only version {RENDERER_ASSET_VERSION} has a verified layout"
        )));
    }
    let prepass_layer_mask = r.u32()?;
    let opaque_layer_mask = r.u32()?;
    let transparent_layer_mask = r.u32()?;
    let stencil = UnityStencilState {
        override_stencil: flag(&mut r)?,
        reference: i64::from(r.i32()?),
        compare: choice(&mut r, "stencilCompareFunction", &first(9))?,
        pass: choice(&mut r, "passOperation", &first(8))?,
        fail: choice(&mut r, "failOperation", &first(8))?,
        z_fail: choice(&mut r, "zFailOperation", &first(8))?,
    };
    let depth_formats = [0, 90, 91, 92, 93, 94, 151];
    let renderer = UnityUrpRenderer {
        features,
        feature_map,
        native_render_pass,
        xr_system_data,
        post_process_data,
        prepass_layer_mask,
        opaque_layer_mask,
        transparent_layer_mask,
        stencil,
        shadow_transparent_receive: flag(&mut r)?,
        rendering_mode: choice(&mut r, "m_RenderingMode", &first(4))?,
        depth_priming_mode: choice(&mut r, "m_DepthPrimingMode", &first(3))?,
        copy_depth_mode: choice(&mut r, "m_CopyDepthMode", &first(3))?,
        depth_attachment_format: choice(&mut r, "m_DepthAttachmentFormat", &depth_formats)?,
        depth_texture_format: choice(&mut r, "m_DepthTextureFormat", &depth_formats)?,
        accurate_gbuffer_normals: flag(&mut r)?,
        intermediate_texture_mode: choice(&mut r, "m_IntermediateTextureMode", &first(2))?,
    };
    used_up(&r, WHAT)?;
    Ok(renderer)
}

/// Whether the renderer feature whose script fields are `bytes` is on (`ScriptableRendererFeature.m_Active`, the
/// first field of every feature class). A renderer asks only its active features for passes.
pub fn read_urp_feature_active(bytes: &[u8]) -> Result<bool> {
    flag(&mut Reader::new(bytes, false))
}

/// The script class of Universal RP's Decal renderer feature.
pub const URP_DECAL_FEATURE: &str = "UnityEngine.Rendering.Universal.DecalRendererFeature";

/// The script class of Universal RP's Full Screen Pass renderer feature.
pub const URP_FULL_SCREEN_FEATURE: &str = "UnityEngine.Rendering.Universal.FullScreenPassRendererFeature";

/// A Decal renderer feature as stored ([`URP_DECAL_FEATURE`]: `m_Active` and its `DecalSettings`, Universal RP
/// 17.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpDecalFeature {
    /// `m_Active`.
    pub active: bool,
    /// `technique`, a `DecalTechniqueOption`: 0 automatic, 1 DBuffer, 2 screen space.
    pub technique: i64,
    /// `maxDrawDistance`.
    pub max_draw_distance: f32,
    /// `decalLayers`.
    pub decal_layers: bool,
    /// `dBufferSettings.surfaceData`, a `DecalSurfaceData`: 0 albedo, 1 albedo and normal, 2 albedo, normal,
    /// metallic, occlusion and smoothness.
    pub dbuffer_surface_data: i64,
    /// `screenSpaceSettings.normalBlend`, a `DecalNormalBlend`: 0 low, 1 medium, 2 high.
    pub screen_space_normal_blend: i64,
}

/// The Decal feature whose script fields are `bytes`.
pub fn read_urp_decal_feature(bytes: &[u8]) -> Result<UnityUrpDecalFeature> {
    let mut r = Reader::new(bytes, false);
    let feature = UnityUrpDecalFeature {
        active: flag(&mut r)?,
        technique: choice(&mut r, "DecalSettings.technique", &first(3))?,
        max_draw_distance: r.f32()?,
        decal_layers: flag(&mut r)?,
        dbuffer_surface_data: choice(&mut r, "DBufferSettings.surfaceData", &first(3))?,
        screen_space_normal_blend: choice(&mut r, "DecalScreenSpaceSettings.normalBlend", &first(3))?,
    };
    used_up(&r, "Decal renderer feature")?;
    Ok(feature)
}

/// The `m_Version` of a Full Screen Pass feature the layout is verified for (`AddFetchColorBufferCheckbox`).
const FULL_SCREEN_VERSION: i32 = 1;

/// A Full Screen Pass renderer feature as stored ([`URP_FULL_SCREEN_FEATURE`], `m_Version` 1, Universal RP 17.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpFullScreenFeature {
    /// `m_Active`.
    pub active: bool,
    /// When its pass runs (`injectionPoint`, a `RenderPassEvent`): 450 before transparents, 550 before
    /// post-processing, 600 after post-processing.
    pub injection_point: i64,
    /// Whether the pass copies the camera colour for its material to read (`fetchColorBuffer`).
    pub fetch_color_buffer: bool,
    /// The textures the pass asks the renderer for (`requirements`, `ScriptableRenderPassInput` flags: 1 depth,
    /// 2 normals, 4 colour, 8 motion vectors; the editor's "everything" stores -1, every bit set).
    pub requirements: i64,
    /// `passMaterial`.
    pub pass_material: PPtr,
    /// `passIndex`.
    pub pass_index: i64,
    /// `bindDepthStencilAttachment`.
    pub bind_depth_stencil_attachment: bool,
}

/// The Full Screen Pass feature whose script fields are `bytes`.
pub fn read_urp_full_screen_feature(bytes: &[u8]) -> Result<UnityUrpFullScreenFeature> {
    const WHAT: &str = "Full Screen Pass renderer feature";
    let mut r = Reader::new(bytes, false);
    let active = flag(&mut r)?;
    let injection_point = choice(&mut r, "injectionPoint", &[450, 550, 600])?;
    let fetch_color_buffer = flag(&mut r)?;
    let requirements = i64::from(r.i32()?);
    let pass_material = pptr(&mut r)?;
    let pass_index = i64::from(r.i32()?);
    let bind_depth_stencil_attachment = flag(&mut r)?;
    let version = r.i32()?;
    if version != FULL_SCREEN_VERSION {
        return Err(invalid(format!(
            "{WHAT} m_Version {version}: only version {FULL_SCREEN_VERSION} is verified"
        )));
    }
    used_up(&r, WHAT)?;
    Ok(UnityUrpFullScreenFeature {
        active,
        injection_point,
        fetch_color_buffer,
        requirements,
        pass_material,
        pass_index,
        bind_depth_stencil_attachment,
    })
}

/// The script class of the URP data Universal RP keeps beside a Camera.
pub const URP_CAMERA_DATA: &str = "UnityEngine.Rendering.Universal.UniversalAdditionalCameraData";

/// The `m_Version` of a camera's URP data the layout is verified for (`DepthAndOpaqueTextureOptions`).
const CAMERA_VERSION: i32 = 2;

/// A camera's temporal anti-aliasing settings (`TemporalAA.Settings`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityTaaSettings {
    /// `m_Quality`, a `TemporalAAQuality`: 0 very low to 4 very high.
    pub quality: i64,
    /// `m_FrameInfluence`.
    pub frame_influence: f32,
    /// `m_JitterScale`.
    pub jitter_scale: f32,
    /// `m_MipBias`.
    pub mip_bias: f32,
    /// `m_VarianceClampScale`.
    pub variance_clamp_scale: f32,
    /// `m_ContrastAdaptiveSharpening`.
    pub contrast_adaptive_sharpening: f32,
}

/// A camera's URP data as stored ([`URP_CAMERA_DATA`], `m_Version` 2, Universal RP 17.3). The options are
/// `CameraOverrideOption`s: 0 off, 1 on, 2 the pipeline asset's setting.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpCamera {
    /// `m_RenderShadows`.
    pub render_shadows: bool,
    /// Whether the camera makes the depth texture (`m_RequiresDepthTextureOption`).
    pub depth_texture_option: i64,
    /// Whether the camera makes the opaque texture (`m_RequiresOpaqueTextureOption`).
    pub opaque_texture_option: i64,
    /// `m_CameraType`, a `CameraRenderType`: 0 base, 1 overlay.
    pub camera_type: i64,
    /// The overlay cameras stacked on a base camera (`m_Cameras`).
    pub stack: Vec<PPtr>,
    /// The index of the pipeline asset's renderer the camera draws with (`m_RendererIndex`); -1 for the asset's
    /// default renderer.
    pub renderer_index: i64,
    /// `m_VolumeLayerMask`.
    pub volume_layer_mask: u32,
    /// `m_VolumeTrigger`.
    pub volume_trigger: PPtr,
    /// `m_VolumeFrameworkUpdateModeOption`: 0 every frame, 1 by script, 2 the pipeline asset's setting.
    pub volume_update_mode: i64,
    /// `m_RenderPostProcessing`.
    pub render_post_processing: bool,
    /// `m_Antialiasing`, an `AntialiasingMode`: 0 none, 1 FXAA, 2 SMAA, 3 temporal.
    pub antialiasing: i64,
    /// `m_AntialiasingQuality`: 0 low, 1 medium, 2 high.
    pub antialiasing_quality: i64,
    /// `m_StopNaN`.
    pub stop_nan: bool,
    /// `m_Dithering`.
    pub dithering: bool,
    /// `m_ClearDepth`.
    pub clear_depth: bool,
    /// `m_AllowXRRendering`.
    pub allow_xr_rendering: bool,
    /// `m_AllowHDROutput`.
    pub allow_hdr_output: bool,
    /// `m_UseScreenCoordOverride`.
    pub use_screen_coord_override: bool,
    /// `m_ScreenSizeOverride`.
    pub screen_size_override: [f32; 4],
    /// `m_ScreenCoordScaleBias`.
    pub screen_coord_scale_bias: [f32; 4],
    /// `m_RequiresDepthTexture`, the field the depth texture option replaced.
    pub requires_depth_texture: bool,
    /// `m_RequiresColorTexture`, the field the opaque texture option replaced.
    pub requires_color_texture: bool,
    /// `m_TaaSettings`.
    pub taa: UnityTaaSettings,
}

/// The camera URP data whose script fields are `bytes`.
pub fn read_urp_camera(bytes: &[u8]) -> Result<UnityUrpCamera> {
    const WHAT: &str = "Universal Render Pipeline camera data";
    let mut r = Reader::new(bytes, false);
    let vec4 = |r: &mut Reader| -> Result<[f32; 4]> { Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]) };
    let camera = UnityUrpCamera {
        render_shadows: flag(&mut r)?,
        depth_texture_option: choice(&mut r, "m_RequiresDepthTextureOption", &first(3))?,
        opaque_texture_option: choice(&mut r, "m_RequiresOpaqueTextureOption", &first(3))?,
        camera_type: choice(&mut r, "m_CameraType", &first(2))?,
        stack: pptrs(&mut r, "m_Cameras")?,
        renderer_index: i64::from(r.i32()?),
        volume_layer_mask: r.u32()?,
        volume_trigger: pptr(&mut r)?,
        volume_update_mode: choice(&mut r, "m_VolumeFrameworkUpdateModeOption", &first(3))?,
        render_post_processing: flag(&mut r)?,
        antialiasing: choice(&mut r, "m_Antialiasing", &first(4))?,
        antialiasing_quality: choice(&mut r, "m_AntialiasingQuality", &first(3))?,
        stop_nan: flag(&mut r)?,
        dithering: flag(&mut r)?,
        clear_depth: flag(&mut r)?,
        allow_xr_rendering: flag(&mut r)?,
        allow_hdr_output: flag(&mut r)?,
        use_screen_coord_override: flag(&mut r)?,
        screen_size_override: vec4(&mut r)?,
        screen_coord_scale_bias: vec4(&mut r)?,
        requires_depth_texture: flag(&mut r)?,
        requires_color_texture: flag(&mut r)?,
        taa: UnityTaaSettings {
            quality: choice(&mut r, "TemporalAA.Settings.m_Quality", &first(5))?,
            frame_influence: r.f32()?,
            jitter_scale: r.f32()?,
            mip_bias: r.f32()?,
            variance_clamp_scale: r.f32()?,
            contrast_adaptive_sharpening: r.f32()?,
        },
    };
    let version = r.i32()?;
    if version != CAMERA_VERSION {
        return Err(invalid(format!(
            "{WHAT} m_Version {version}: only version {CAMERA_VERSION} is verified"
        )));
    }
    used_up(&r, WHAT)?;
    Ok(camera)
}

/// The script class of Universal RP's global settings (the object GraphicsSettings'
/// `m_CurrentRenderPipelineGlobalSettings` names; [`crate::player::Player::urp_global_settings`]).
pub const URP_GLOBAL_SETTINGS: &str = "UnityEngine.Rendering.Universal.UniversalRenderPipelineGlobalSettings";

/// The `m_AssetVersion` of the global settings the layout is verified for.
const GLOBAL_SETTINGS_VERSION: i32 = 10;

/// The version of the managed-reference registry a script's `[SerializeReference]` fields are stored in.
const REFERENCES_VERSION: i32 = 2;

/// Universal RP's global settings as stored (`UniversalRenderPipelineGlobalSettings`, `m_AssetVersion` 10, Universal
/// RP 17.3): the rendering layers and the graphics settings the pipeline reads its runtime resources and options from.
/// The settings are `[SerializeReference]` entries, each stored once in the object's managed-reference registry with
/// its class; every one is read to its last byte, so a class whose layout is not verified is refused by name
/// ([`verified_graphics_settings`] lists the verified ones).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityUrpGlobalSettings {
    /// `m_RenderingLayerNames`.
    pub rendering_layer_names: Vec<String>,
    /// `m_ValidRenderingLayers`.
    pub valid_rendering_layers: u32,
    /// The graphics settings (`m_Settings.m_RuntimeSettings.m_List`), in the list's order.
    pub settings: Vec<UnityGraphicsSettings>,
}

/// One graphics settings entry (an `IRenderPipelineGraphicsSettings`) as stored.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityGraphicsSettings {
    /// Its managed-reference id.
    pub rid: i64,
    /// Its class, `namespace.class` as the registry names it (a nested class `Outer/Inner`).
    pub class: String,
    /// The assembly the class is in.
    pub assembly: String,
    /// Its serialized fields, in declaration order, each by its name in the source.
    pub fields: Vec<(&'static str, UnityField)>,
}

/// A field of a graphics settings entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityField {
    /// A 4-byte integer or enum value.
    Int(i64),
    /// A bool, padded to 4 bytes.
    Bool(bool),
    /// An object reference.
    Ref(PPtr),
    /// A list of object references.
    Refs(Vec<PPtr>),
    /// A list of colours (`Color[]`), RGBA.
    Colours(Vec<[f32; 4]>),
}

/// How a field of a graphics settings class is stored.
#[derive(Clone, Copy, Debug)]
enum FieldKind {
    /// A 4-byte value that must be this one: a class's `m_Version`, whose other values are other layouts.
    Version(i32),
    Int,
    Bool,
    Ref,
    Refs,
    Colours,
}

use FieldKind as F;

/// The fields of `ShaderStrippingSetting`.
const SHADER_STRIPPING_FIELDS: &[(&str, FieldKind)] = &[
    ("m_Version", F::Version(0)),
    ("m_ExportShaderVariants", F::Bool),
    ("m_ShaderVariantLogLevel", F::Int),
    ("m_StripRuntimeDebugShaders", F::Bool),
];

/// The graphics settings classes whose layout is verified, each with its serialized fields in declaration order
/// (Universal RP 17.3's, Core RP 17.3's and GPU Driven RP 17.3's classes as Unity 6000.3's players store them): each
/// reads to the byte where the registry's next entry starts on PEAK's and Sledding Game's players.
const GRAPHICS_SETTINGS: [(&str, &[(&str, FieldKind)]); 18] = [
    (
        "UnityEngine.Rendering.Universal.UniversalRenderPipelineRuntimeTerrainShaders",
        &[
            ("m_Version", F::Version(0)),
            ("m_TerrainDetailLit", F::Ref),
            ("m_TerrainDetailGrassBillboard", F::Ref),
            ("m_TerrainDetailGrass", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.URPReflectionProbeSettings",
        &[
            ("version", F::Version(1)),
            ("useReflectionProbeRotation", F::Bool),
        ],
    ),
    (
        "UnityEngine.Rendering.VrsRenderPipelineRuntimeResources",
        &[
            ("m_TextureComputeShader", F::Ref),
            ("m_VisualizationShader", F::Ref),
            ("m_VisualizationLookupTable", F::Colours),
            ("m_ConversionLookupTable", F::Colours),
        ],
    ),
    (
        "UnityEngine.Rendering.LightmapSamplingSettings",
        &[
            ("m_Version", F::Version(1)),
            ("m_UseBicubicLightmapSampling", F::Bool),
        ],
    ),
    (
        URP_RENDER_GRAPH_SETTINGS,
        &[
            ("m_Version", F::Version(0)),
            ("m_EnableRenderCompatibilityMode", F::Bool),
        ],
    ),
    (
        URP_RENDERER_RESOURCES,
        &[
            ("m_Version", F::Version(0)),
            ("m_CopyDepthPS", F::Ref),
            ("m_CameraMotionVector", F::Ref),
            ("m_StencilDeferredPS", F::Ref),
            ("m_ClusterDeferred", F::Ref),
            ("m_StencilDitherMaskSeedPS", F::Ref),
            ("m_DBufferClear", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.Universal.URPDefaultVolumeProfileSettings",
        &[("m_Version", F::Version(0)), ("m_VolumeProfile", F::Ref)],
    ),
    (
        "UnityEngine.Rendering.Universal.UniversalRenderPipelineRuntimeShaders",
        &[
            ("m_Version", F::Version(0)),
            ("m_FallbackErrorShader", F::Ref),
            ("m_BlitHDROverlay", F::Ref),
            ("m_CoreBlitPS", F::Ref),
            ("m_CoreBlitColorAndDepthPS", F::Ref),
            ("m_SamplingPS", F::Ref),
            ("m_TerrainDetailLit", F::Ref),
            ("m_TerrainDetailGrassBillboard", F::Ref),
            ("m_TerrainDetailGrass", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.Universal.UniversalRenderPipelineRuntimeXRResources",
        &[
            ("m_xrOcclusionMeshPS", F::Ref),
            ("m_xrMirrorViewPS", F::Ref),
            ("m_xrMotionVector", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.Universal.UniversalRenderPipelineDebugShaders",
        &[
            ("m_DebugReplacementPS", F::Ref),
            ("m_HdrDebugViewPS", F::Ref),
            ("m_ProbeVolumeSamplingDebugComputeShader", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.Universal.UniversalRenderPipelineRuntimeTextures",
        &[
            ("m_Version", F::Version(1)),
            ("m_BlueNoise64LTex", F::Ref),
            ("m_BayerMatrixTex", F::Ref),
            ("m_DebugFontTex", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.RenderGraphGlobalSettings",
        &[
            ("m_version", F::Version(0)),
            ("m_EnableCompilationCaching", F::Bool),
            ("m_EnableValidityChecks", F::Bool),
        ],
    ),
    (
        "UnityEngine.Rendering.RenderGraphModule.Util.RenderGraphUtilsResources",
        &[("m_Version", F::Version(0)), ("m_CoreCopyPS", F::Ref)],
    ),
    (
        "UnityEngine.Rendering.ShaderStrippingSetting",
        SHADER_STRIPPING_FIELDS,
    ),
    (
        "UnityEngine.Rendering.GPUResidentDrawerResources",
        &[
            ("m_Version", F::Version(0)),
            ("m_InstanceDataBufferCopyKernels", F::Ref),
            ("m_InstanceDataBufferUploadKernels", F::Ref),
            ("m_TransformUpdaterKernels", F::Ref),
            ("m_WindDataUpdaterKernels", F::Ref),
            ("m_OccluderDepthPyramidKernels", F::Ref),
            ("m_InstanceOcclusionCullingKernels", F::Ref),
            ("m_OcclusionCullingDebugKernels", F::Ref),
            ("m_DebugOcclusionTestPS", F::Ref),
            ("m_DebugOccluderPS", F::Ref),
        ],
    ),
    (
        "UnityEngine.Rendering.STP/RuntimeResources",
        &[("m_setupCS", F::Ref), ("m_preTaaCS", F::Ref), ("m_taaCS", F::Ref)],
    ),
    (
        "UnityEngine.Rendering.Universal.ScreenSpaceAmbientOcclusionPersistentResources",
        &[("m_Shader", F::Ref), ("m_Version", F::Version(0))],
    ),
    (
        "UnityEngine.Rendering.Universal.ScreenSpaceAmbientOcclusionDynamicResources",
        &[("m_BlueNoise256Textures", F::Refs), ("m_Version", F::Version(0))],
    ),
];

/// The graphics settings classes whose layout is verified ([`read_urp_global_settings`] refuses the others by name).
pub fn verified_graphics_settings() -> impl Iterator<Item = &'static str> {
    GRAPHICS_SETTINGS.iter().map(|(c, _)| *c)
}

/// The class of Universal RP's render graph settings.
pub const URP_RENDER_GRAPH_SETTINGS: &str = "UnityEngine.Rendering.Universal.RenderGraphSettings";

/// The class of the Universal Renderer's runtime resources.
pub const URP_RENDERER_RESOURCES: &str = "UnityEngine.Rendering.Universal.UniversalRendererResources";

/// The Universal Renderer's runtime shaders (`UniversalRendererResources`), each a `Shader` reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct UnityUrpRendererResources {
    /// `m_CopyDepthPS`: the copy of the camera depth (`CopyDepthPass`).
    pub copy_depth: PPtr,
    /// `m_CameraMotionVector`.
    pub camera_motion_vector: PPtr,
    /// `m_StencilDeferredPS`.
    pub stencil_deferred: PPtr,
    /// `m_ClusterDeferred`.
    pub cluster_deferred: PPtr,
    /// `m_StencilDitherMaskSeedPS`.
    pub stencil_dither_mask_seed: PPtr,
    /// `m_DBufferClear`: the DBuffer's clear (`DBufferRenderPass`).
    pub dbuffer_clear: PPtr,
}

impl UnityGraphicsSettings {
    /// The field named `name`, where the class has one.
    pub fn field(&self, name: &str) -> Option<&UnityField> {
        self.fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
}

impl UnityUrpGlobalSettings {
    /// The entry of class `class`, where the list has one (a class is listed once).
    pub fn entry(&self, class: &str) -> Option<&UnityGraphicsSettings> {
        self.settings.iter().find(|s| s.class == class)
    }

    /// The Universal Renderer's runtime shaders, where the settings list them.
    pub fn renderer_resources(&self) -> Option<UnityUrpRendererResources> {
        let e = self.entry(URP_RENDERER_RESOURCES)?;
        let r = |name: &str| match e.field(name) {
            Some(UnityField::Ref(p)) => Some(*p),
            _ => None,
        };
        Some(UnityUrpRendererResources {
            copy_depth: r("m_CopyDepthPS")?,
            camera_motion_vector: r("m_CameraMotionVector")?,
            stencil_deferred: r("m_StencilDeferredPS")?,
            cluster_deferred: r("m_ClusterDeferred")?,
            stencil_dither_mask_seed: r("m_StencilDitherMaskSeedPS")?,
            dbuffer_clear: r("m_DBufferClear")?,
        })
    }

    /// The stored `m_EnableRenderCompatibilityMode` of the render graph settings, where the settings list them: whether
    /// the project asks Universal RP to draw without the render graph. A build honours it only where its Universal RP
    /// is compiled with `URP_COMPATIBILITY_MODE` ([`crate::player::Player::urp_compatibility_code`]); without, the
    /// pipeline always draws through the render graph.
    pub fn render_compatibility_mode(&self) -> Option<bool> {
        match self
            .entry(URP_RENDER_GRAPH_SETTINGS)?
            .field("m_EnableRenderCompatibilityMode")
        {
            Some(UnityField::Bool(b)) => Some(*b),
            _ => None,
        }
    }
}

/// One string: its byte count and its UTF-8 bytes padded to 4 bytes.
fn string(r: &mut Reader, what: &str) -> Result<String> {
    let len = list_len(r, what, 1)?;
    let s = std::str::from_utf8(r.take(len)?)
        .map_err(|_| invalid(format!("{what}: a string that is not UTF-8")))?
        .to_owned();
    r.align(4)?;
    Ok(s)
}

/// The fields `fields` of an object of `class`, in order.
fn settings_fields(
    r: &mut Reader,
    class: &str,
    fields: &[(&'static str, FieldKind)],
) -> Result<Vec<(&'static str, UnityField)>> {
    fields
        .iter()
        .map(|&(name, kind)| {
            let what = format!("{class}.{name}");
            let value = match kind {
                FieldKind::Version(want) => {
                    let v = r.i32()?;
                    if v != want {
                        return Err(invalid(format!(
                            "{what} {v}: only version {want} has a verified layout"
                        )));
                    }
                    UnityField::Int(i64::from(v))
                }
                FieldKind::Int => UnityField::Int(i64::from(r.i32()?)),
                FieldKind::Bool => UnityField::Bool(flag(r)?),
                FieldKind::Ref => UnityField::Ref(pptr(r)?),
                FieldKind::Refs => UnityField::Refs(pptrs(r, &what)?),
                FieldKind::Colours => {
                    let n = list_len(r, &what, 16)?;
                    UnityField::Colours(
                        (0..n)
                            .map(|_| Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]))
                            .collect::<Result<_>>()?,
                    )
                }
            };
            Ok((name, value))
        })
        .collect()
}

/// An empty list, refused when it holds anything: its elements' layout is not verified (no player read stores any).
fn empty_list(r: &mut Reader, what: &str) -> Result<()> {
    match r.u32()? {
        0 => Ok(()),
        n => Err(invalid(format!(
            "{what}: {n} entries, whose layout is not verified (only empty lists are read)"
        ))),
    }
}

/// The global settings whose script fields are `bytes` ([`URP_GLOBAL_SETTINGS`]), read to the last byte with every
/// graphics settings entry of the registry: a class not in [`verified_graphics_settings`] is refused by name, as is an entry
/// the list and the registry do not both hold.
pub fn read_urp_global_settings(bytes: &[u8]) -> Result<UnityUrpGlobalSettings> {
    const WHAT: &str = "Universal Render Pipeline global settings";
    let mut r = Reader::new(bytes, false);
    // the migration copies of the stripping settings and the obsolete flags, kept by the class and read past
    settings_fields(&mut r, "m_ShaderStrippingSetting", SHADER_STRIPPING_FIELDS)?;
    settings_fields(
        &mut r,
        "m_URPShaderStrippingSetting",
        &[
            ("m_Version", F::Version(0)),
            ("m_StripUnusedPostProcessingVariants", F::Bool),
            ("m_StripUnusedVariants", F::Bool),
            ("m_StripScreenCoordOverrideVariants", F::Bool),
        ],
    )?;
    settings_fields(
        &mut r,
        WHAT,
        &[
            ("m_ShaderVariantLogLevel", F::Int),
            ("m_ExportShaderVariants", F::Bool),
            ("m_StripDebugVariants", F::Bool),
            ("m_StripUnusedPostProcessingVariants", F::Bool),
            ("m_StripUnusedVariants", F::Bool),
            ("m_StripScreenCoordOverrideVariants", F::Bool),
            ("supportRuntimeDebugDisplay", F::Bool),
            ("m_EnableRenderGraph", F::Bool),
        ],
    )?;
    // m_Settings.m_RuntimeSettings.m_List: the entries' managed-reference ids
    let n = list_len(&mut r, "m_Settings.m_RuntimeSettings.m_List", 8)?;
    let list: Vec<i64> = (0..n).map(|_| r.i64()).collect::<Result<_>>()?;
    let version = r.i32()?;
    if version != GLOBAL_SETTINGS_VERSION {
        return Err(invalid(format!(
            "{WHAT} m_AssetVersion {version}: only version {GLOBAL_SETTINGS_VERSION} has a verified layout"
        )));
    }
    pptr(&mut r)?; // m_ObsoleteDefaultVolumeProfile
    let rendering_layer_names = strings(&mut r, "m_RenderingLayerNames")?;
    let valid_rendering_layers = r.u32()?;
    for i in 0..8 {
        string(&mut r, &format!("lightLayerName{i}"))?;
    }
    // apvScenesData: two serialized dictionaries' key and value lists
    for list in [
        "obsoleteSceneBounds keys",
        "obsoleteSceneBounds values",
        "obsoleteHasProbeVolumes keys",
        "obsoleteHasProbeVolumes values",
    ] {
        empty_list(&mut r, &format!("apvScenesData.{list}"))?;
    }
    // the managed-reference registry: each entry's id, class and fields
    let registry = r.i32()?;
    if registry != REFERENCES_VERSION {
        return Err(invalid(format!(
            "{WHAT}: managed-reference registry version {registry} (read: {REFERENCES_VERSION})"
        )));
    }
    let n = list_len(&mut r, "the managed-reference registry", 20)?;
    let mut entries: BTreeMap<i64, UnityGraphicsSettings> = BTreeMap::new();
    for _ in 0..n {
        let rid = r.i64()?;
        let class = string(&mut r, "a managed reference's class")?;
        let namespace = string(&mut r, "a managed reference's namespace")?;
        let assembly = string(&mut r, "a managed reference's assembly")?;
        let class = match namespace.as_str() {
            "" => class,
            ns => format!("{ns}.{class}"),
        };
        let (_, layout) = GRAPHICS_SETTINGS.iter().find(|(c, _)| *c == class).ok_or_else(|| {
            invalid(format!(
                "{WHAT}: a graphics settings entry of class {class} ({assembly}), whose layout is not verified"
            ))
        })?;
        let fields = settings_fields(&mut r, &class, layout)?;
        let entry = UnityGraphicsSettings {
            rid,
            class,
            assembly,
            fields,
        };
        if let Some(first) = entries.insert(rid, entry) {
            return Err(invalid(format!(
                "{WHAT}: managed reference {rid} stored twice ({})",
                first.class
            )));
        }
    }
    used_up(&r, WHAT)?;
    let settings = list
        .iter()
        .map(|rid| {
            entries.remove(rid).ok_or_else(|| {
                invalid(format!(
                    "{WHAT}: the settings list names managed reference {rid}, which the registry does not hold"
                ))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(e) = entries.values().next() {
        return Err(invalid(format!(
            "{WHAT}: managed reference {} ({}) that the settings list does not name",
            e.rid, e.class
        )));
    }
    let mut classes = std::collections::BTreeSet::new();
    for s in &settings {
        if !classes.insert(s.class.as_str()) {
            return Err(invalid(format!("{WHAT}: the class {} listed twice", s.class)));
        }
    }
    Ok(UnityUrpGlobalSettings {
        rendering_layer_names,
        valid_rendering_layers,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic asset in the layout: each field at the position the real bytes of a player's asset put it.
    fn asset(cascades: i32, default_renderer: i32) -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        let int = |b: &mut Vec<u8>, v: i32| b.extend(v.to_le_bytes());
        let float = |b: &mut Vec<u8>, v: f32| b.extend(v.to_le_bytes());
        let bool4 = |b: &mut Vec<u8>, v: bool| b.extend(i32::from(v).to_le_bytes());
        int(&mut b, 13); // k_AssetVersion
        int(&mut b, 13);
        int(&mut b, 1); // renderer type
        b.extend([0; 12]); // m_RendererData
        int(&mut b, 1); // one renderer
        int(&mut b, 0);
        b.extend(6168i64.to_le_bytes());
        int(&mut b, default_renderer);
        bool4(&mut b, true); // depth texture
        bool4(&mut b, false); // opaque texture
        int(&mut b, 1);
        bool4(&mut b, false); // terrain holes
        bool4(&mut b, true); // HDR
        int(&mut b, 0);
        int(&mut b, 4); // MSAA
        float(&mut b, 1.0);
        int(&mut b, 0); // upscaling filter
        bool4(&mut b, false);
        float(&mut b, 0.92);
        bool4(&mut b, true); // LOD cross fade
        int(&mut b, 1);
        int(&mut b, 2); // SH evaluation: mixed
        int(&mut b, 0); // legacy light probes
        int(&mut b, 1024);
        int(&mut b, 256);
        for _ in 0..4 {
            bool4(&mut b, false);
        }
        int(&mut b, 1); // SH bands
        int(&mut b, 1); // main light: per pixel
        bool4(&mut b, true); // main light shadows
        int(&mut b, 4096);
        int(&mut b, 1); // additional lights: per pixel
        int(&mut b, 8);
        bool4(&mut b, true); // additional light shadows
        for v in [1024, 256, 512, 1024] {
            int(&mut b, v);
        }
        bool4(&mut b, true); // reflection probe blending
        bool4(&mut b, false); // box projection
        bool4(&mut b, true); // atlas
        float(&mut b, 400.0);
        int(&mut b, cascades);
        float(&mut b, 0.25);
        for v in [0.1, 0.3, 0.123, 0.29, 0.54, 0.11, 0.1, 0.5] {
            float(&mut b, v);
        }
        bool4(&mut b, true); // soft shadows
        bool4(&mut b, true); // conservative sphere
        int(&mut b, 64);
        int(&mut b, 3);
        int(&mut b, 2048);
        int(&mut b, 3);
        bool4(&mut b, true);
        bool4(&mut b, false);
        bool4(&mut b, true); // mixed lighting
        bool4(&mut b, false); // light cookies
        bool4(&mut b, true); // light layers
        // the fields after them, which no setting is read from: debug level to the volume framework's update mode
        int(&mut b, 0);
        int(&mut b, 0);
        bool4(&mut b, true);
        int(&mut b, 0);
        int(&mut b, 32);
        for _ in 0..4 {
            bool4(&mut b, true);
        }
        int(&mut b, 0);
        float(&mut b, 0.0);
        bool4(&mut b, false);
        int(&mut b, 1);
        bool4(&mut b, false);
        for v in [256, 0, 256, 2] {
            int(&mut b, v);
        }
        b.extend([0; 12]); // m_VolumeProfile
        // apvScenesData: one scene's bounds, one scene's flag (a 5-byte key, padded)
        int(&mut b, 1);
        int(&mut b, 5);
        b.extend(b"scene\0\0\0");
        int(&mut b, 1);
        b.extend([0; 24]);
        int(&mut b, 1);
        int(&mut b, 5);
        b.extend(b"scene\0\0\0");
        int(&mut b, 1);
        b.extend([1, 0, 0, 0]);
        int(&mut b, 0); // m_ShaderVariantLogLevel
        int(&mut b, 1); // m_ShadowCascades
        b.extend([0; 24]); // m_Textures
        b
    }

    /// The fields the keywords depend on are read from where the layout puts them, bools padded to 4 bytes and the
    /// renderer list's length moving what follows it.
    #[test]
    fn a_urp_assets_settings_are_read_by_the_verified_layout() {
        let read = |b: &[u8]| read_urp_settings(b, UrpAssetLayout::Urp17_3);
        let s = read(&asset(4, 0)).unwrap();
        let renderer = PPtr {
            file_id: 0,
            path_id: 6168,
        };
        assert_eq!((s.renderers.as_slice(), s.default_renderer), (&[renderer][..], 0));
        assert!(s.require_depth_texture && !s.require_opaque_texture);
        assert_eq!(s.msaa, 4);
        assert_eq!((s.shadow_cascades, s.shadow_distance), (4, 400.0));
        assert!(s.main_light_shadows && s.soft_shadows && s.additional_light_shadows);
        assert!(
            s.reflection_probe_blending && !s.reflection_probe_box_projection && s.reflection_probe_atlas
        );
        assert!(s.mixed_lighting && !s.light_cookies && s.light_layers && s.lod_cross_fade);
        assert_eq!(
            (s.main_light_mode, s.additional_lights_mode, s.sh_eval_mode),
            (1, 1, 2)
        );
        // another version, and values no field can hold: a fifth cascade, a default renderer past the list
        let mut other = asset(4, 0);
        other[0] = 12;
        assert!(read(&other).is_err());
        assert!(read(&asset(7, 0)).is_err());
        let past = read(&asset(4, 1)).unwrap_err().to_string();
        assert!(past.contains("m_DefaultRendererIndex"), "{past}");
        // the asset is read to its last byte: one short, or one more, is another layout
        let whole = asset(4, 0);
        assert!(read(&whole[..whole.len() - 4]).is_err());
        let mut longer = whole.clone();
        longer.extend([0; 4]);
        let longer = read(&longer).unwrap_err().to_string();
        assert!(longer.contains("past the verified layout"), "{longer}");
    }

    /// The layout is the player's Unity version's, known before any field is read: the versions it is verified on,
    /// and no other.
    #[test]
    fn a_urp_assets_layout_is_identified_by_the_players_unity_version() {
        let of = |v: &str| UrpAssetLayout::of(UnityVersion::parse(v).unwrap());
        assert_eq!(of("6000.3.12f1").unwrap(), UrpAssetLayout::Urp17_3);
        assert_eq!(of("6000.3.15f1").unwrap(), UrpAssetLayout::Urp17_3);
        let refused = of("6000.3.17f1").unwrap_err().to_string();
        assert!(refused.contains("Unity 6000.3.17f1 player"), "{refused}");
        assert!(of("6000.6.4f1").is_err());
    }

    fn int(b: &mut Vec<u8>, v: i32) {
        b.extend(v.to_le_bytes());
    }

    fn reference(b: &mut Vec<u8>, path_id: i64) {
        int(b, 0);
        b.extend(path_id.to_le_bytes());
    }

    /// A synthetic renderer in the layout: two features, the masks, the stencil state and the modes after them.
    fn renderer(version: i32, mode: i32) -> Vec<u8> {
        let mut b = vec![0; 12 * 9]; // debugShaders, probeVolumeResources
        int(&mut b, 2);
        reference(&mut b, 6001);
        reference(&mut b, 6002);
        int(&mut b, 2);
        b.extend(7i64.to_le_bytes());
        b.extend((-8i64).to_le_bytes());
        int(&mut b, 1); // native render pass
        reference(&mut b, 0); // xrSystemData
        reference(&mut b, 6170); // postProcessData
        int(&mut b, version);
        for mask in [0x8a30_9437u32, 0x8a31_d437, u32::MAX] {
            b.extend(mask.to_le_bytes());
        }
        for v in [1, 3, 8, 2, 0, 0] {
            int(&mut b, v); // stencil: overridden, reference 3, always, replace
        }
        int(&mut b, 1); // shadow transparent receive
        int(&mut b, mode);
        int(&mut b, 0); // depth priming disabled
        int(&mut b, 1); // copy depth after transparents
        int(&mut b, 92);
        int(&mut b, 0);
        int(&mut b, 0);
        int(&mut b, 1); // intermediate texture always
        b
    }

    /// A renderer's fields are read from where the layout puts them, the feature list's length moving what follows;
    /// a feature map is read as stored, whatever its length; another asset version, a mode no field can hold and a
    /// length the layout does not account for (extra bytes, or the layout without `xrSystemData`) are refused.
    #[test]
    fn a_urp_renderer_is_read_by_the_verified_layout() {
        let r = read_urp_renderer(&renderer(3, 2)).unwrap();
        let ids: Vec<i64> = r.features.iter().map(|f| f.path_id).collect();
        assert_eq!((ids, r.feature_map.clone()), (vec![6001, 6002], vec![7, -8]));
        assert!(r.native_render_pass && r.shadow_transparent_receive && !r.accurate_gbuffer_normals);
        assert_eq!((r.xr_system_data.path_id, r.post_process_data.path_id), (0, 6170));
        assert_eq!(
            (
                r.prepass_layer_mask,
                r.opaque_layer_mask,
                r.transparent_layer_mask
            ),
            (0x8a30_9437, 0x8a31_d437, u32::MAX)
        );
        assert_eq!(
            r.stencil,
            UnityStencilState {
                override_stencil: true,
                reference: 3,
                compare: 8,
                pass: 2,
                fail: 0,
                z_fail: 0
            }
        );
        assert_eq!(
            (r.rendering_mode, r.depth_priming_mode, r.copy_depth_mode),
            (2, 0, 1)
        );
        assert_eq!(
            (
                r.depth_attachment_format,
                r.depth_texture_format,
                r.intermediate_texture_mode
            ),
            (92, 0, 1)
        );
        let refusal = |b: &[u8]| read_urp_renderer(b).unwrap_err().to_string();
        assert!(refusal(&renderer(2, 2)).contains("asset version 2"));
        assert!(refusal(&renderer(3, 4)).contains("m_RenderingMode 4"));
        let mut longer = renderer(3, 2);
        longer.extend([0; 4]);
        assert!(refusal(&longer).contains("100 bytes after m_UseNativeRenderPass"));
        // the layout without xrSystemData: the same renderer 12 bytes shorter
        let mut no_xr = renderer(3, 2);
        no_xr.drain(160..172);
        assert!(refusal(&no_xr).contains("84 bytes after"), "{}", refusal(&no_xr));
        // a map of one entry for the two features: the bytes after it move back by one entry, as they are read
        let mut map = renderer(3, 2);
        map[136] = 1; // the map's count: one entry
        map.drain(148..156);
        let short = read_urp_renderer(&map).unwrap();
        assert_eq!((short.features.len(), short.feature_map), (2, vec![7]));
        assert_eq!(short.post_process_data.path_id, 6170);
        assert!(read_urp_renderer(&renderer(3, 2)[..150]).is_err());
    }

    /// A feature's `m_Active` is its first field; the Decal and Full Screen Pass features' settings follow it, and a
    /// value outside a field's enum, another version or a length past the layout is refused.
    #[test]
    fn urp_features_are_read_by_the_verified_layout() {
        assert!(read_urp_feature_active(&[1, 0, 0, 0, 9, 9]).unwrap());
        assert!(!read_urp_feature_active(&[0, 0, 0, 0]).unwrap());
        assert!(read_urp_feature_active(&[2, 0, 0, 0]).is_err());
        let decal = |technique: i32| {
            let mut b = Vec::new();
            int(&mut b, 1);
            int(&mut b, technique);
            b.extend(500f32.to_le_bytes());
            int(&mut b, 0);
            int(&mut b, 2);
            int(&mut b, 0);
            b
        };
        assert_eq!(
            read_urp_decal_feature(&decal(1)).unwrap(),
            UnityUrpDecalFeature {
                active: true,
                technique: 1,
                max_draw_distance: 500.0,
                decal_layers: false,
                dbuffer_surface_data: 2,
                screen_space_normal_blend: 0
            }
        );
        assert!(read_urp_decal_feature(&decal(3)).is_err());
        let mut longer = decal(1);
        longer.extend([0; 4]);
        assert!(read_urp_decal_feature(&longer).is_err());
        let full_screen = |injection: i32, version: i32| {
            let mut b = Vec::new();
            int(&mut b, 1);
            int(&mut b, injection);
            int(&mut b, 1);
            int(&mut b, -1); // every input
            reference(&mut b, 7);
            int(&mut b, 0);
            int(&mut b, 0);
            int(&mut b, version);
            b
        };
        let f = read_urp_full_screen_feature(&full_screen(600, 1)).unwrap();
        assert!(f.active && f.fetch_color_buffer && !f.bind_depth_stencil_attachment);
        assert_eq!(
            (
                f.injection_point,
                f.requirements,
                f.pass_material.path_id,
                f.pass_index
            ),
            (600, -1, 7, 0)
        );
        let refusal = |b: &[u8]| read_urp_full_screen_feature(b).unwrap_err().to_string();
        assert!(refusal(&full_screen(300, 1)).contains("injectionPoint 300"));
        assert!(refusal(&full_screen(600, 0)).contains("m_Version 0"));
    }

    /// A synthetic camera's URP data in the layout, with `stack` overlay cameras.
    fn camera(stack: i32, version: i32) -> Vec<u8> {
        let mut b = Vec::new();
        int(&mut b, 1); // shadows
        int(&mut b, 2);
        int(&mut b, 2);
        int(&mut b, 0); // base
        int(&mut b, stack);
        for i in 0..stack {
            reference(&mut b, i64::from(i) + 100);
        }
        int(&mut b, -1); // the default renderer
        b.extend(0x8000_0000u32.to_le_bytes());
        reference(&mut b, 0);
        int(&mut b, 2);
        int(&mut b, 1); // post-processing
        int(&mut b, 3); // temporal anti-aliasing
        int(&mut b, 2);
        for v in [0, 1, 1, 1, 1, 0] {
            int(&mut b, v);
        }
        for v in [0f32, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0] {
            b.extend(v.to_le_bytes());
        }
        int(&mut b, 0);
        int(&mut b, 0);
        int(&mut b, 3); // TAA quality high
        for v in [0.1f32, 1.0, 0.0, 0.9, 0.0] {
            b.extend(v.to_le_bytes());
        }
        int(&mut b, version);
        b
    }

    /// A camera's fields are read from where the layout puts them, the stack's length moving what follows; another
    /// version and a length past the layout are refused.
    #[test]
    fn a_urp_camera_is_read_by_the_verified_layout() {
        assert_eq!(camera(0, 2).len(), 148);
        let c = read_urp_camera(&camera(2, 2)).unwrap();
        let stack: Vec<i64> = c.stack.iter().map(|p| p.path_id).collect();
        assert_eq!(stack, vec![100, 101]);
        assert_eq!(
            (
                c.renderer_index,
                c.depth_texture_option,
                c.antialiasing,
                c.taa.quality
            ),
            (-1, 2, 3, 3)
        );
        assert!(c.render_shadows && c.render_post_processing && c.clear_depth && !c.stop_nan && c.dithering);
        assert_eq!(
            (c.volume_layer_mask, c.screen_coord_scale_bias),
            (0x8000_0000, [1.0, 1.0, 0.0, 0.0])
        );
        assert_eq!(c.taa.variance_clamp_scale, 0.9);
        assert!(
            read_urp_camera(&camera(0, 1))
                .unwrap_err()
                .to_string()
                .contains("m_Version 1")
        );
        let mut longer = camera(0, 2);
        longer.extend([0; 4]);
        assert!(read_urp_camera(&longer).is_err());
    }
}
