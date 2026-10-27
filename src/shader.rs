//! A Shader's render state as stored (`m_ParsedForm`): its subshaders, their passes and the passes' render-state
//! values, with the `BlendMode` values they use. [`crate::material::ShaderData`] decodes them from a
//! Shader object; [`procedural`] says which shaders build their geometry from component data.

/// Whether a shader builds its geometry from component data, drawing a placeholder mesh: Shapes (a widely used
/// package's line, disc, triangle and sphere shaders).
pub fn procedural(shader: &str) -> bool {
    shader.starts_with("Shapes/")
}

/// Unity's `BlendMode` values (`UnityEngine.Rendering.BlendMode`) a pass's blend factors use.
pub mod blend_mode {
    /// `Zero`.
    pub const ZERO: u8 = 0;
    /// `One`.
    pub const ONE: u8 = 1;
    /// `DstColor`.
    pub const DST_COLOR: u8 = 2;
    /// `SrcColor`.
    pub const SRC_COLOR: u8 = 3;
    /// `SrcAlpha`.
    pub const SRC_ALPHA: u8 = 5;
    /// `OneMinusSrcColor`.
    pub const ONE_MINUS_SRC_COLOR: u8 = 6;
    /// `OneMinusSrcAlpha`.
    pub const ONE_MINUS_SRC_ALPHA: u8 = 10;
    /// The last value, `OneMinusSrcAlpha`.
    pub const LAST: u8 = 10;
}

/// A render-state value of a shader pass: the value the shader declares, and the material property it is bound to
/// (`Blend [_SrcBlend] ...`), which then gives the value.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ShaderValue {
    /// The value as stored.
    pub value: f32,
    /// The material property that gives the value instead, when the shader binds one (`None` for a fixed value).
    pub property: Option<String>,
}

/// What a pass is (`m_Type`): one with its own state and programs, a `UsePass` naming another shader's pass
/// (`m_UseName`, its state not here), or a `GrabPass` copying the screen to a texture (it draws nothing).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PassKind {
    /// A pass with its own state and programs.
    Normal,
    /// A `UsePass` of the named pass.
    Use(String),
    /// A `GrabPass`.
    Grab,
}

/// One render target's blend and write mask as the pass states them (`rtBlend0` to `rtBlend7`): Unity's `BlendMode`
/// factors and `BlendOp` operations, and the colour mask (bit 0 red to bit 3 alpha).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ShaderTargetBlend {
    /// `srcBlend`: the source colour factor.
    pub src: ShaderValue,
    /// `destBlend`: the destination colour factor.
    pub dst: ShaderValue,
    /// `srcBlendAlpha`: the source alpha factor.
    pub src_alpha: ShaderValue,
    /// `destBlendAlpha`: the destination alpha factor.
    pub dst_alpha: ShaderValue,
    /// `blendOp`: the colour operation.
    pub op: ShaderValue,
    /// `blendOpAlpha`: the alpha operation.
    pub op_alpha: ShaderValue,
    /// `colMask`: the colour write mask.
    pub mask: ShaderValue,
}

/// A stencil face's operations (`SerializedStencilOp`): Unity's `StencilOp` for pass, fail and depth-fail, and its
/// `CompareFunction` (0 disabled, 1 never, 2 less, 3 equal, 4 less or equal, 5 greater, 6 not equal, 7 greater or
/// equal, 8 always).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ShaderStencilOp {
    /// The operation when the stencil and depth tests pass.
    pub pass: ShaderValue,
    /// The operation when the stencil test fails.
    pub fail: ShaderValue,
    /// The operation when the depth test fails.
    pub z_fail: ShaderValue,
    /// The stencil comparison.
    pub comp: ShaderValue,
}

/// The rest of a pass's render state as stored (`m_State`): every render target's blend, the depth test and offset and
/// the stencil.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PassState {
    /// `rtBlend0` to `rtBlend7`; the first is the pass's `src_blend` and `dst_blend`.
    pub targets: Vec<ShaderTargetBlend>,
    /// `rtSeparateBlend`: whether targets 1 to 7 state their own blend (else they take target 0's).
    pub separate_blend: bool,
    /// `zTest`, a `CompareFunction`.
    pub z_test: ShaderValue,
    /// `Offset factor, units`.
    pub offset_factor: ShaderValue,
    /// `Offset factor, units`: the units.
    pub offset_units: ShaderValue,
    /// `stencilRef`, `stencilReadMask`, `stencilWriteMask`.
    pub stencil_ref: ShaderValue,
    /// `stencilReadMask`.
    pub stencil_read_mask: ShaderValue,
    /// `stencilWriteMask`.
    pub stencil_write_mask: ShaderValue,
    /// `stencilOp`, `stencilOpFront`, `stencilOpBack`.
    pub stencil_op: ShaderStencilOp,
    /// `stencilOpFront`.
    pub stencil_op_front: ShaderStencilOp,
    /// `stencilOpBack`.
    pub stencil_op_back: ShaderStencilOp,
}

/// A shader pass's kind and render state: its `LightMode` tag as stored (Unity upper-cases some: `FORWARDBASE`), the
/// first render target's blend factors (Unity's `BlendMode`: 0 Zero, 1 One, 5 SrcAlpha, 10 OneMinusSrcAlpha, ...),
/// depth writes and culling (0 off, 1 front, 2 back).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ShaderPass {
    /// What the pass is.
    pub kind: PassKind,
    /// The pass's `LightMode` tag as stored; `None` when it has none.
    pub light_mode: Option<String>,
    /// `srcBlend` of the first render target.
    pub src_blend: ShaderValue,
    /// `destBlend` of the first render target.
    pub dst_blend: ShaderValue,
    /// `zWrite`: whether the pass writes depth.
    pub zwrite: ShaderValue,
    /// `culling`: which faces are culled.
    pub cull: ShaderValue,
    /// The rest of the pass's render state.
    pub state: PassState,
}

/// A subshader: its tags as stored (Unity upper-cases some keys: `QUEUE`) and its passes in order.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct SubShader {
    /// The subshader's tags.
    pub tags: Vec<(String, String)>,
    /// The passes, in order.
    pub passes: Vec<ShaderPass>,
}
