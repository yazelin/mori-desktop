//! Body Interface — Mori universe 各身體部件接入 Mori 的 semantic 契約。
//! 見 `docs/mori-body-interface.md`。BI-0 只放 artifact;BI-1+ 再加
//! manifest / event / permission / cue 等型別。

pub mod artifact;

pub use artifact::{
    classify_artifact, MoriArtifact, SuggestedAction, Visibility, KIND_CHARACTER_PACK,
};
