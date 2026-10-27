//! Unity's Animator side as typed data: an Avatar (the skeleton Mecanim animates, and for a humanoid the compiled
//! muscle axes and the HumanDescription it was built from), an AnimatorController (layers, state machines, states with
//! their blend trees and transitions, parameters). Nothing here is
//! evaluated: it is what the files hold, in Unity's frame (rotations as `[x, y, z, w]`), with the names Unity keeps
//! only as CRC-32 hashes resolved through the object's own name table (`tos`) where it has one.

mod read;

use serde::Serialize;

use crate::player::ObjRef;

/// The hash Unity's Animator data names things by (a state, a layer, a parameter, a binding's path): the CRC-32 (the zlib one)
/// of the name's bytes.
pub fn name_hash(name: &str) -> u32 {
    let mut crc = !0u32;
    for byte in name.bytes() {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// A transform as Mecanim stores it (`xform`): translation, rotation as a quaternion `[x, y, z, w]` and scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Xform {
    /// Translation.
    pub t: [f32; 3],
    /// Rotation, a quaternion `[x, y, z, w]`.
    pub q: [f32; 4],
    /// Scale.
    pub s: [f32; 3],
}

/// One bone's muscle axes (`Axes`): its pre- and post-rotation, the sign of each axis, the limits of its rotation in
/// radians, its length and its type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AvatarAxes {
    /// `m_PreQ`: the rotation applied before the bone's own.
    pub pre_q: [f32; 4],
    /// `m_PostQ`: the rotation applied after it.
    pub post_q: [f32; 4],
    /// `m_Sgn`: the sign of each axis.
    pub sgn: [f32; 3],
    /// `m_Min`: the lower limit of each axis, in radians.
    pub min: [f32; 3],
    /// `m_Max`: the upper limit of each axis, in radians.
    pub max: [f32; 3],
    /// `m_Length`.
    pub length: f32,
    /// `m_Type`.
    pub kind: i64,
}

/// A skeleton (`Skeleton`): per node its parent (`-1` for a root) and its axes index (`-1` for none), the CRC-32 of
/// each node's path, and the axes the nodes index.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct AvatarSkeleton {
    /// Per node, its parent (`m_ParentId`, -1 for a root) and its axes index (`m_AxesId`, -1 for none).
    pub nodes: Vec<(i64, i64)>,
    /// `m_ID`: the CRC-32 of each node's path.
    pub ids: Vec<u32>,
    /// `m_AxesArray`: the axes the nodes index.
    pub axes: Vec<AvatarAxes>,
}

/// A humanoid's compiled data (`Human`): the human skeleton with its pose and axes, which skeleton bone each human bone
/// is (`bone_index`, `-1` for none), the proportions the retargeting uses.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AvatarHuman {
    /// `m_RootX`.
    pub root: Xform,
    /// `m_Skeleton`: the human skeleton.
    pub skeleton: AvatarSkeleton,
    /// `m_SkeletonPose`: a transform for each node of the human skeleton.
    pub pose: Vec<Xform>,
    /// `m_LeftHand`'s `m_HandBoneIndex`.
    pub left_hand: Vec<i64>,
    /// `m_RightHand`'s `m_HandBoneIndex`.
    pub right_hand: Vec<i64>,
    /// `m_HumanBoneIndex`: which skeleton bone each human bone is, -1 for none.
    pub bone_index: Vec<i64>,
    /// `m_HumanBoneMass`.
    pub bone_mass: Vec<f32>,
    /// `m_Scale`.
    pub scale: f32,
    /// `m_ArmTwist`.
    pub arm_twist: f32,
    /// `m_ForeArmTwist`.
    pub fore_arm_twist: f32,
    /// `m_UpperLegTwist`.
    pub upper_leg_twist: f32,
    /// `m_LegTwist`.
    pub leg_twist: f32,
    /// `m_ArmStretch`.
    pub arm_stretch: f32,
    /// `m_LegStretch`.
    pub leg_stretch: f32,
    /// `m_FeetSpacing`.
    pub feet_spacing: f32,
    /// `m_HasLeftHand`.
    pub has_left_hand: bool,
    /// `m_HasRightHand`.
    pub has_right_hand: bool,
    /// `m_HasTDoF`.
    pub has_translation_dof: bool,
}

/// One human bone of the HumanDescription: the skeleton bone it names, the human bone it is and its stated limits.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanBone {
    /// `m_BoneName`: the skeleton bone.
    pub bone_name: String,
    /// `m_HumanName`: the human bone, as Unity names it.
    pub human_name: String,
    /// `m_Min`.
    pub min: [f32; 3],
    /// `m_Max`.
    pub max: [f32; 3],
    /// `m_Value`.
    pub value: [f32; 3],
    /// `m_Length`.
    pub length: f32,
    /// `m_Modified`.
    pub modified: bool,
}

/// One bone of the HumanDescription's skeleton: its name, its parent's, and its local transform.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkeletonBone {
    /// `m_Name`.
    pub name: String,
    /// `m_ParentName`.
    pub parent: String,
    /// `m_Position`.
    pub position: [f32; 3],
    /// `m_Rotation`, a quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// `m_Scale`.
    pub scale: [f32; 3],
}

/// What the avatar was built from (`HumanDescription`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanDescription {
    /// `m_Human`: the human bones.
    pub human: Vec<HumanBone>,
    /// `m_Skeleton`: the skeleton's bones.
    pub skeleton: Vec<SkeletonBone>,
    /// `m_ArmTwist`.
    pub arm_twist: f32,
    /// `m_ForeArmTwist`.
    pub fore_arm_twist: f32,
    /// `m_UpperLegTwist`.
    pub upper_leg_twist: f32,
    /// `m_LegTwist`.
    pub leg_twist: f32,
    /// `m_ArmStretch`.
    pub arm_stretch: f32,
    /// `m_LegStretch`.
    pub leg_stretch: f32,
    /// `m_FeetSpacing`.
    pub feet_spacing: f32,
    /// `m_GlobalScale`.
    pub global_scale: f32,
    /// `m_RootMotionBoneName`.
    pub root_motion_bone_name: String,
    /// `m_HasTranslationDoF`.
    pub has_translation_dof: bool,
    /// `m_HasExtraRoot`.
    pub has_extra_root: bool,
    /// `m_SkeletonHasParents`.
    pub skeleton_has_parents: bool,
}

/// An Avatar object as stored: the skeleton Mecanim animates and, for a humanoid, its muscle axes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityAvatar {
    /// The Avatar object.
    pub avatar: ObjRef,
    /// `m_Name`.
    pub name: String,
    /// `m_AvatarSkeleton`: the nodes Mecanim animates.
    pub skeleton: AvatarSkeleton,
    /// The skeleton's pose (`m_AvatarSkeletonPose`) and its default pose (`m_DefaultPose`), a transform a node.
    pub skeleton_pose: Vec<Xform>,
    /// `m_DefaultPose`: a transform for each node.
    pub default_pose: Vec<Xform>,
    /// `m_SkeletonNameIDArray`.
    pub skeleton_name_ids: Vec<u32>,
    /// A humanoid's data; `None` for a generic avatar (its human skeleton is empty).
    pub human: Option<AvatarHuman>,
    /// Which avatar skeleton node each human skeleton node is (`m_HumanSkeletonIndexArray`) and the reverse.
    pub human_skeleton_index: Vec<i64>,
    /// `m_HumanSkeletonReverseIndexArray`.
    pub human_skeleton_reverse: Vec<i64>,
    /// `m_RootMotionBoneIndex`.
    pub root_motion_bone_index: i64,
    /// `m_RootMotionBoneX`.
    pub root_motion_bone: Xform,
    /// `m_RootMotionSkeleton`.
    pub root_motion_skeleton: AvatarSkeleton,
    /// Empty when the avatar stores no pose for its root motion nodes.
    pub root_motion_pose: Vec<Xform>,
    /// `m_RootMotionSkeletonIndexArray`.
    pub root_motion_skeleton_index: Vec<i64>,
    /// The CRC-32 of a path and the path (`m_TOS`).
    pub tos: Vec<(u32, String)>,
    /// `m_HumanDescription`.
    pub description: HumanDescription,
}

/// A transition condition (`ConditionConstant`): its mode (1 true, 2 false, 3 greater, 4 less, 6 equals, 7 not equal,
/// as Unity numbers them), the CRC-32 of its parameter's name, its threshold and, for an exit-time condition, the time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Condition {
    /// `m_ConditionMode`.
    pub mode: i64,
    /// `m_EventID`: the CRC-32 of the parameter's name.
    pub parameter: u32,
    /// `m_EventThreshold`.
    pub threshold: f32,
    /// `m_ExitTime`.
    pub exit_time: f32,
}

/// What a destination at or above this is: a transition's `destination` (and a selector transition's) below it names a
/// state of its machine, `SELECTOR_BASE + n` the selector node `n` (an entry or an exit).
pub const SELECTOR_BASE: i64 = 30000;

/// A transition (`TransitionConstant`) of a state or from any state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Transition {
    /// `m_ConditionConstantArray`.
    pub conditions: Vec<Condition>,
    /// A state, or [`SELECTOR_BASE`] plus a selector node.
    pub destination: i64,
    /// `m_FullPathID`.
    pub full_path_id: u32,
    /// `m_ID`.
    pub id: u32,
    /// `m_UserID`.
    pub user_id: u32,
    /// `m_TransitionDuration`.
    pub duration: f32,
    /// `m_TransitionOffset`.
    pub offset: f32,
    /// `m_ExitTime`.
    pub exit_time: f32,
    /// `m_HasExitTime`.
    pub has_exit_time: bool,
    /// `m_HasFixedDuration`.
    pub has_fixed_duration: bool,
    /// `m_InterruptionSource`.
    pub interruption_source: i64,
    /// `m_OrderedInterruption`.
    pub ordered_interruption: bool,
    /// `m_CanTransitionToSelf`.
    pub can_transition_to_self: bool,
}

/// One node of a blend tree (`BlendTreeNodeConstant`): its kind (0 1D, 1 simple 2D, 2 freeform directional, 3 freeform
/// cartesian, 4 direct, as Unity numbers them), the parameters it blends by, its children and thresholds or positions,
/// and for a leaf the clip (`clip`: an index of the controller's clips, `u32::MAX` for none).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlendNode {
    /// `m_BlendType`.
    pub blend_type: i64,
    /// `m_BlendEventID`: the CRC-32 of the parameter the node blends by.
    pub event_id: u32,
    /// `m_BlendEventYID`: the second parameter, of a 2D node.
    pub event_y_id: u32,
    /// `m_ChildIndices`: the children's indices in the tree's nodes.
    pub children: Vec<u32>,
    /// `m_ChildThresholdArray` of the 1D data.
    pub thresholds: Vec<f32>,
    /// `m_ChildPositionArray` of the 2D data.
    pub positions: Vec<[f32; 2]>,
    /// `m_ChildMagnitudeArray` of the 2D data.
    pub magnitudes: Vec<f32>,
    /// `m_ChildPairVectorArray` of the 2D data.
    pub pair_vectors: Vec<[f32; 2]>,
    /// `m_ChildPairAvgMagInvArray` of the 2D data.
    pub pair_avg_mag_inv: Vec<f32>,
    /// `m_ChildNeighborListArray`: per child the indices of its neighbours in the 2D tree.
    pub neighbors: Vec<Vec<u32>>,
    /// `m_ChildBlendEventIDArray` of the direct data: the CRC-32 of each child's parameter.
    pub direct_events: Vec<u32>,
    /// `m_NormalizedBlendValues` of the direct data.
    pub normalized_blend_values: bool,
    /// `m_ClipID`.
    pub clip: u32,
    /// `m_Duration`.
    pub duration: f32,
    /// `m_CycleOffset`.
    pub cycle_offset: f32,
    /// `m_Mirror`.
    pub mirror: bool,
}

/// A state (`StateConstant`): its transitions, its blend trees (one per motion set; a plain clip is a tree of one
/// leaf), the CRC-32 of its name and path and the parameters that drive its speed, mirror, offset and time.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct State {
    /// `m_TransitionConstantArray`.
    pub transitions: Vec<Transition>,
    /// `m_BlendTreeConstantIndexArray`.
    pub blend_tree_index: Vec<i64>,
    /// `m_BlendTreeConstantArray`: each tree's nodes (`m_NodeArray`).
    pub blend_trees: Vec<Vec<BlendNode>>,
    /// `m_NameID`.
    pub name_id: u32,
    /// `m_PathID`.
    pub path_id: u32,
    /// `m_FullPathID`.
    pub full_path_id: u32,
    /// `m_TagID`.
    pub tag_id: u32,
    /// `m_SpeedParamID`.
    pub speed_param: u32,
    /// `m_MirrorParamID`.
    pub mirror_param: u32,
    /// `m_CycleOffsetParamID`.
    pub cycle_offset_param: u32,
    /// `m_TimeParamID`.
    pub time_param: u32,
    /// `m_Speed`.
    pub speed: f32,
    /// `m_CycleOffset`.
    pub cycle_offset: f32,
    /// `m_IKOnFeet`.
    pub ik_on_feet: bool,
    /// `m_WriteDefaultValues`.
    pub write_default_values: bool,
    /// `m_Loop`.
    pub looping: bool,
    /// `m_Mirror`.
    pub mirror: bool,
}

/// A transition out of an entry or exit selector (`SelectorTransitionConstant`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectorTransition {
    /// A state, or [`SELECTOR_BASE`] plus a selector node.
    pub destination: u32,
    /// `m_ConditionConstantArray`.
    pub conditions: Vec<Condition>,
}

/// An entry or exit node of a state machine (`SelectorStateConstant`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SelectorState {
    /// `m_TransitionConstantArray`.
    pub transitions: Vec<SelectorTransition>,
    /// `m_FullPathID`.
    pub full_path_id: u32,
    /// `m_IsEntry`: an entry node, else an exit.
    pub is_entry: bool,
}

/// A state machine (`StateMachineConstant`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StateMachine {
    /// `m_StateConstantArray`.
    pub states: Vec<State>,
    /// `m_AnyStateTransitionConstantArray`.
    pub any_state_transitions: Vec<Transition>,
    /// `m_SelectorStateConstantArray`.
    pub selectors: Vec<SelectorState>,
    /// `m_DefaultState`.
    pub default_state: i64,
    /// `m_SynchronizedLayerCount`.
    pub synchronized_layer_count: i64,
}

/// A layer (`LayerConstant`): the state machine it runs, the body and skeleton masks, how it blends in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Layer {
    /// `m_StateMachineIndex`: an index of the controller's state machines.
    pub state_machine: i64,
    /// `m_StateMachineSynchronizedLayerIndex`.
    pub synchronized_layer: i64,
    /// `m_BodyMask`'s three words.
    pub body_mask: [u32; 3],
    /// `m_SkeletonMask`: the CRC-32 of a path and its weight.
    pub skeleton_mask: Vec<(u32, f32)>,
    /// The CRC-32 of the layer's name.
    pub binding: u32,
    /// `m_LayerBlendingMode`.
    pub blending_mode: i64,
    /// `m_DefaultWeight`.
    pub default_weight: f32,
    /// `m_IKPass`.
    pub ik_pass: bool,
    /// `m_SyncedLayerAffectsTiming`.
    pub synced_layer_affects_timing: bool,
}

/// A controller parameter (`ValueConstant`): the CRC-32 of its name, its kind (1 float, 3 int, 4 bool, 9 trigger, as
/// Unity numbers them) and its place in the default values of that kind.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Parameter {
    /// `m_ID`: the CRC-32 of the parameter's name.
    pub id: u32,
    /// `m_Type`.
    pub kind: i64,
    /// `m_Index`.
    pub index: i64,
}

/// An AnimatorController object as stored (the compiled `ControllerConstant`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnityAnimatorController {
    /// The AnimatorController object.
    pub controller: ObjRef,
    /// `m_Name`.
    pub name: String,
    /// `m_LayerArray`.
    pub layers: Vec<Layer>,
    /// `m_StateMachineArray`.
    pub state_machines: Vec<StateMachine>,
    /// `m_ValueArray`.
    pub parameters: Vec<Parameter>,
    /// `m_FloatValues` of the default values.
    pub default_floats: Vec<f32>,
    /// `m_IntValues` of the default values.
    pub default_ints: Vec<i64>,
    /// `m_BoolValues` of the default values.
    pub default_bools: Vec<bool>,
    /// The clips the blend trees' leaves index (`m_AnimationClips`): one slot per entry, `None` for a null pointer (a leaf's
    /// index counts it).
    pub clips: Vec<Option<ObjRef>>,
    /// The scripts of the state machine behaviours (`m_StateMachineBehaviours`), by object.
    pub behaviours: Vec<ObjRef>,
    /// The CRC-32 of a name (a state, path, layer or parameter) and the name (`m_TOS`).
    pub tos: Vec<(u32, String)>,
}

impl UnityAnimatorController {
    /// The name a hash stands for in this controller's name table.
    pub fn name_of(&self, hash: u32) -> Option<&str> {
        self.tos.iter().find(|(h, _)| *h == hash).map(|(_, n)| n.as_str())
    }
}
