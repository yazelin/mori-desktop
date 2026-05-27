//! BI-1:body part manifest 目錄 + bundled 第一方 body part manifest。
//! MoriPack Studio 是第一個 registered body part(artifact-first 工具)。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// bundled 第一方 body part manifest:MoriPack Studio。
/// 對齊 docs/moripack-integration.md §Phase 3 sample。
const MORIPACK_STUDIO_MANIFEST: &str = r#"{
  "schema_version": 1,
  "id": "mori.moripack-studio",
  "name": "MoriPack Studio",
  "kind": "standalone_app",
  "description": "Mori 角色包(.moripack.zip)的外部編輯器。",
  "entrypoints": { "web": "https://mori-sprite-studio.vercel.app/" },
  "capabilities": ["character_pack.edit", "character_pack.export"],
  "permissions": ["filesystem.read.character_pack", "filesystem.write.character_pack_export"],
  "data_policy": { "owns_raw_data": false, "default_ingestion": "off" }
}
"#;

/// `~/.mori/body-parts/`。使用 mori_dir()(與 character_pack::characters_dir() 同根)。
pub fn body_parts_dir() -> PathBuf {
    crate::mori_dir().join("body-parts")
}

/// 啟動時確保 bundled 第一方 body part manifest 存在(不覆蓋既有 — user/第三方可能改過)。
pub fn ensure_bundled_body_parts() -> Result<()> {
    write_if_absent(
        &body_parts_dir()
            .join("mori.moripack-studio")
            .join("manifest.json"),
        MORIPACK_STUDIO_MANIFEST,
    )
}

fn write_if_absent(path: &Path, body: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_moripack_manifest_is_valid_and_parses() {
        // bundled manifest 本身必須是合法、可被 mori-core 解析的 v1 manifest。
        let m = mori_core::body::parse_manifest(MORIPACK_STUDIO_MANIFEST)
            .expect("bundled manifest must parse");
        assert_eq!(m.id, "mori.moripack-studio");
        assert_eq!(m.kind, mori_core::body::BodyKind::StandaloneApp);
        assert_eq!(
            mori_core::body::manifest_status(&m),
            mori_core::body::ManifestStatus::Valid
        );
    }

    #[test]
    fn write_if_absent_writes_then_skips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().join("a/manifest.json");
        write_if_absent(&p, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");
        write_if_absent(&p, "second").unwrap(); // 已存在 → 不覆蓋
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");
    }
}
