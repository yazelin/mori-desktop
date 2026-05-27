//! Body Interface 的 semantic artifact envelope。
//!
//! 對應 `docs/mori-body-interface.md` §Semantic schema 的 `MoriArtifactMetadata`
//! 與 `docs/moripack-integration.md` 的 Artifact Contract。raw 內容留在來源,
//! 這個 envelope 只描述「它是什麼、在哪、能對它做什麼」。

use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// character pack artifact 的 kind 常數。
pub const KIND_CHARACTER_PACK: &str = "mori.character-pack";

/// 一個可在 body part 之間 handoff 的 artifact 的 metadata。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoriArtifact {
    pub artifact_id: String,
    /// 開放詞彙的 kind,例如 `mori.character-pack`。
    pub kind: String,
    pub path: String,
    pub visibility: Visibility,
    pub mime: String,
    pub suggested_actions: Vec<SuggestedAction>,
}

/// 資料可見度。對應 body-interface 的 data policy。BI-0 只用到 `Local`,
/// 其餘三層是鎖定契約的一部分,先列出(schema 保留,非 build)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Local,
    Public,
    Internal,
    Private,
}

/// Mori 對這個 artifact 建議可做的動作。BI-0 只有 character pack 的三個。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestedAction {
    Validate,
    Import,
    Activate,
}

impl MoriArtifact {
    /// 產生帶新 id 的 artifact envelope。
    pub fn new(
        kind: impl Into<String>,
        path: impl Into<String>,
        visibility: Visibility,
        mime: impl Into<String>,
        suggested_actions: Vec<SuggestedAction>,
    ) -> Self {
        Self {
            artifact_id: format!("artifact_{}", Uuid::new_v4().simple()),
            kind: kind.into(),
            path: path.into(),
            visibility,
            mime: mime.into(),
            suggested_actions,
        }
    }
}

/// 看一個本機檔案路徑,判斷 Mori 認不認得它、能對它做什麼。
/// 認得 → artifact envelope;不認得 → `None`。
///
/// BI-0 只認 character pack(`.moripack.zip` / `.moripack` / `.zip`)。
/// 真正的內容驗證仍在 import 時做(manifest / required sprites / zip-slip)。
pub fn classify_artifact(path: &Path) -> Option<MoriArtifact> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".moripack.zip") || name.ends_with(".moripack") || name.ends_with(".zip") {
        return Some(MoriArtifact::new(
            KIND_CHARACTER_PACK,
            path.to_string_lossy().into_owned(),
            Visibility::Local,
            "application/zip",
            vec![
                SuggestedAction::Validate,
                SuggestedAction::Import,
                SuggestedAction::Activate,
            ],
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_serializes_to_doc_contract_shape() {
        let a = MoriArtifact {
            artifact_id: "character_pack_001".into(),
            kind: KIND_CHARACTER_PACK.into(),
            path: "/tmp/mori.moripack.zip".into(),
            visibility: Visibility::Local,
            mime: "application/zip".into(),
            suggested_actions: vec![
                SuggestedAction::Validate,
                SuggestedAction::Import,
                SuggestedAction::Activate,
            ],
        };
        let v: serde_json::Value = serde_json::to_value(&a).unwrap();
        assert_eq!(v["kind"], "mori.character-pack");
        assert_eq!(v["visibility"], "local");
        assert_eq!(
            v["suggested_actions"],
            serde_json::json!(["validate", "import", "activate"])
        );
        let back: MoriArtifact = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn new_generates_prefixed_unique_id() {
        let a = MoriArtifact::new(
            KIND_CHARACTER_PACK,
            "/x.zip",
            Visibility::Local,
            "application/zip",
            vec![],
        );
        let b = MoriArtifact::new(
            KIND_CHARACTER_PACK,
            "/y.zip",
            Visibility::Local,
            "application/zip",
            vec![],
        );
        assert!(a.artifact_id.starts_with("artifact_"));
        assert_ne!(a.artifact_id, b.artifact_id);
    }

    #[test]
    fn classify_recognizes_moripack_zip() {
        let a = classify_artifact(Path::new("/home/u/Downloads/mori.moripack.zip")).unwrap();
        assert_eq!(a.kind, KIND_CHARACTER_PACK);
        assert_eq!(a.visibility, Visibility::Local);
        assert_eq!(a.mime, "application/zip");
        assert_eq!(a.path, "/home/u/Downloads/mori.moripack.zip");
        assert!(a.suggested_actions.contains(&SuggestedAction::Activate));
    }

    #[test]
    fn classify_recognizes_plain_zip_and_moripack_ext() {
        assert!(classify_artifact(Path::new("/x/pack.zip")).is_some());
        assert!(classify_artifact(Path::new("/x/pack.moripack")).is_some());
    }

    #[test]
    fn classify_rejects_unknown_extension() {
        assert!(classify_artifact(Path::new("/x/notes.txt")).is_none());
        assert!(classify_artifact(Path::new("/x/no-extension")).is_none());
    }
}
