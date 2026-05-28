//! Body Interface — Mori universe 各身體部件接入 Mori 的 semantic 契約。
//! 見 `docs/mori-body-interface.md`。BI-0 只放 artifact;BI-1+ 再加
//! manifest / event / permission / cue 等型別。

pub mod artifact;
pub mod manifest;
pub mod registry;
pub mod permission;
pub mod permission_audit;
pub mod cue_state;

pub use artifact::{
    classify_artifact, MoriArtifact, SuggestedAction, Visibility, KIND_CHARACTER_PACK,
};
pub use manifest::{
    manifest_status, parse_manifest, BodyKind, BodyManifest, DataPolicy, Entrypoints, Interface,
    ManifestStatus, Transport, SUPPORTED_MANIFEST_SCHEMA,
};
pub use registry::{scan_body_parts, DiscoveredBodyPart};
pub use permission::{
    default_policy, evaluate, BrokerResponse, Decision, Lease, PermissionRequest, PolicyRule,
    PolicyTable, RiskClass, SUPPORTED_PERMISSION_SCHEMA,
};
pub use permission_audit::{
    append_audit, broker_decide, read_audit_tail, PermissionAuditEntry,
};
pub use cue_state::{
    append_state, is_snooze_active, read_state_map, CueAction, CueStateEntry,
};
