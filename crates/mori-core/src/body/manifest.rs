//! Body Part Manifest — body part 對 Mori Desktop 自我描述的 semantic 契約。
//! 見 docs/mori-body-interface.md §Body Part Manifest。BI-1 v1 欄位刻意最小;
//! 未知 transport / 未知欄位都「能讀就降級讀、不 crash」(§Versioning)。

use serde::{Deserialize, Serialize};

/// 目前支援的 manifest schema major。
pub const SUPPORTED_MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub kind: BodyKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entrypoints: Entrypoints,
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub data_policy: DataPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    StandaloneApp,
    LocalService,
    Cli,
    Crate,
    Plugin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entrypoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_api: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    pub name: String,
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// BI-1 只實作 http/sse/cli;其餘(zenoh/ros2/dds…)吃進 `Other`,
/// 不報錯也不處理 —— schema 不禁止未來 binding,但 BI-1 不動它們。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Http,
    Sse,
    Cli,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataPolicy {
    #[serde(default)]
    pub owns_raw_data: bool,
    #[serde(default = "default_ingestion")]
    pub default_ingestion: String,
}

impl Default for DataPolicy {
    fn default() -> Self {
        Self { owns_raw_data: false, default_ingestion: default_ingestion() }
    }
}

fn default_ingestion() -> String {
    "off".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status", content = "detail")]
pub enum ManifestStatus {
    Valid,
    UnsupportedSchema(u32),
}

/// 結構解析(serde)。缺 required 欄位 → Err。
pub fn parse_manifest(json: &str) -> Result<BodyManifest, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

/// 語意有效性:schema_version 是否支援。
pub fn manifest_status(m: &BodyManifest) -> ManifestStatus {
    if m.schema_version == SUPPORTED_MANIFEST_SCHEMA {
        ManifestStatus::Valid
    } else {
        ManifestStatus::UnsupportedSchema(m.schema_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MORIPACK_JSON: &str = r#"{
        "schema_version": 1,
        "id": "mori.moripack-studio",
        "name": "MoriPack Studio",
        "kind": "standalone_app",
        "capabilities": ["character_pack.edit", "character_pack.export"],
        "entrypoints": { "web": "https://mori-sprite-studio.vercel.app/" },
        "permissions": ["filesystem.read.character_pack"],
        "data_policy": { "owns_raw_data": false, "default_ingestion": "off" }
    }"#;

    #[test]
    fn parses_valid_moripack_manifest() {
        let m = parse_manifest(MORIPACK_JSON).expect("should parse");
        assert_eq!(m.id, "mori.moripack-studio");
        assert_eq!(m.kind, BodyKind::StandaloneApp);
        assert!(m.capabilities.contains(&"character_pack.export".to_string()));
        assert_eq!(m.entrypoints.web.as_deref(), Some("https://mori-sprite-studio.vercel.app/"));
        assert!(!m.data_policy.owns_raw_data);
        assert_eq!(manifest_status(&m), ManifestStatus::Valid);
    }

    #[test]
    fn unknown_transport_degrades_to_other_not_error() {
        // §Versioning:不懂的 transport 記錄但不 crash。
        let json = r#"{"schema_version":1,"id":"x","name":"X","kind":"local_service",
            "interfaces":[{"name":"events","transport":"zenoh","url":"z"}]}"#;
        let m = parse_manifest(json).expect("zenoh interface should still parse");
        assert_eq!(m.interfaces[0].transport, Transport::Other);
    }

    #[test]
    fn future_schema_version_is_unsupported_not_parse_error() {
        let json = r#"{"schema_version":99,"id":"x","name":"X","kind":"cli"}"#;
        let m = parse_manifest(json).expect("should still parse structurally");
        assert_eq!(manifest_status(&m), ManifestStatus::UnsupportedSchema(99));
    }

    #[test]
    fn missing_required_field_is_parse_error() {
        // 缺 id → serde 失敗 → Err
        let json = r#"{"schema_version":1,"name":"X","kind":"cli"}"#;
        assert!(parse_manifest(json).is_err());
    }
}
