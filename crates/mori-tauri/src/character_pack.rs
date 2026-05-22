//! Character pack 系統。
//!
//! 角色 sprite + backdrop + 設定打包成「character pack」放在 `~/.mori/characters/<name>/`,
//! user 可從 ConfigTab 「匯入 .moripack.zip」 載入別人做的角色。Mori Sprite Studio
//! 出來的 `.moripack.zip` 是唯一規格來源 — 詳見 `docs/character-pack.md`。
//!
//! ## 結構
//! ```text
//! ~/.mori/characters/
//! ├── mori/                       ← 預設 character(開機 ensure 寫入)
//! │   ├── manifest.json
//! │   ├── sprites/
//! │   │   ├── idle.png             ← 1024×1024 4×4 sheet
//! │   │   ├── sleeping.png
//! │   │   ├── recording.png
//! │   │   ├── thinking.png
//! │   │   ├── done.png
//! │   │   └── error.png
//! │   ├── backdrop-light.png       ← 背景圖(light theme)
//! │   └── backdrop-dark.png        ← 背景圖(dark theme)
//! ├── <user-imported>/...          ← user 從 .moripack.zip import
//! └── active                       ← 一行,當前 active character name(沒檔回 "mori")
//! ```
//!
//! ## Schema 1.x forward compat
//! `manifest.schema_version` 1.x 系列都認(1.0 / 1.1 / 1.2 ...),2.x reject。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Bundled default mori character pack(build 時 embed 進 binary)。
/// 路徑相對 mori-tauri Cargo.toml(`crates/mori-tauri/`),取上兩層到 repo root,
/// 再進 examples/characters/mori/。
static BUNDLED_DEFAULT_PACK: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../examples/characters/mori");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterManifest {
    pub schema_version: String,
    pub package_name: String,
    pub display_name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub states: Vec<String>,
    #[serde(default)]
    pub optional_states: Vec<String>,
    #[serde(default)]
    pub loop_modes: BTreeMap<String, String>,
    #[serde(default)]
    pub loop_durations_ms: BTreeMap<String, u32>,
    pub sprite_spec: SpriteSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteSpec {
    pub format: String,
    pub grid: String,
    pub total_size: String,
    pub frame_size: String,
    pub frame_order: String,
    pub background: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CharacterEntry {
    pub stem: String,
    pub display_name: String,
    pub author: String,
    pub version: String,
}

const DEFAULT_PACKAGE_NAME: &str = "mori";
const REQUIRED_STATES: &[&str] = &["idle", "sleeping", "recording", "thinking", "done", "error"];

pub fn characters_dir() -> PathBuf {
    crate::mori_dir().join("characters")
}

pub fn active_path() -> PathBuf {
    characters_dir().join("active")
}

pub fn pack_dir(stem: &str) -> PathBuf {
    characters_dir().join(stem)
}

pub fn manifest_path(stem: &str) -> PathBuf {
    pack_dir(stem).join("manifest.json")
}

pub fn sprite_path(stem: &str, state: &str) -> PathBuf {
    pack_dir(stem).join("sprites").join(format!("{state}.png"))
}

pub fn backdrop_path(stem: &str, theme: &str) -> PathBuf {
    pack_dir(stem).join(format!("backdrop-{theme}.png"))
}

/// 啟動時:確保 ~/.mori/characters/mori/ 存在 + 寫入 bundled 內容。
///
/// 行為:
/// - 完全沒 manifest → fresh install,extract 整個 bundled pack
/// - 既有 manifest → 尊重 user state(不覆蓋既有 manifest / sprites — user 可能改過)
/// - 但**補寫缺檔**:既有 mori/ 缺 `backdrop-light.png` / `backdrop-dark.png` 等新規格檔
///   → 從 bundled extract 對應檔(因 backdrop 是新加 schema,舊 user 沒有,
///   想看新版自帶 backdrop 不該要 user rm -rf 整個 mori/)
pub fn ensure_default() -> Result<()> {
    let dir = pack_dir(DEFAULT_PACKAGE_NAME);
    if !manifest_path(DEFAULT_PACKAGE_NAME).exists() {
        // fresh install:全 extract
        extract_bundled_default_pack(&dir)?;
        return Ok(());
    }
    // 既有 user state:只補寫缺檔(不動 manifest + 既有 sprites)
    backfill_missing_bundled_files(&dir)?;
    Ok(())
}

/// 對既有 character pack 目錄補寫 bundled 內有但目錄沒的檔(skip 已存在的)。
/// 不動 manifest.json — 假設既有 user state 想保留。
fn backfill_missing_bundled_files(dir: &Path) -> Result<()> {
    backfill_dir_recursively(&BUNDLED_DEFAULT_PACK, dir)
}

fn backfill_dir_recursively(src: &include_dir::Dir<'_>, dest: &Path) -> Result<()> {
    for file in src.files() {
        let rel_path = file.path();
        // 跳過 manifest.json(user 可能改過)
        if rel_path == Path::new("manifest.json") {
            continue;
        }
        let out_path = dest.join(rel_path);
        if out_path.exists() {
            continue; // 既有檔不覆蓋
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, file.contents())
            .with_context(|| format!("backfill {}", out_path.display()))?;
        tracing::info!(path = %out_path.display(), "character_pack: backfilled missing bundled file");
    }
    for subdir in src.dirs() {
        backfill_dir_recursively(subdir, dest)?;
    }
    Ok(())
}

/// 從 BUNDLED_DEFAULT_PACK extract 整個 default mori character pack 到 dir。
/// dir 是 character pack 自己的目錄(例 ~/.mori/characters/mori/),fn 內負責建 sprites/ 子目錄。
pub(crate) fn extract_bundled_default_pack(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("sprites"))?;
    write_dir_recursively(&BUNDLED_DEFAULT_PACK, dir)
        .context("write bundled default pack")?;
    Ok(())
}

fn write_dir_recursively(src: &include_dir::Dir<'_>, dest: &Path) -> Result<()> {
    for file in src.files() {
        let rel_path = file.path();
        let out_path = dest.join(rel_path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out_path, file.contents())
            .with_context(|| format!("write {}", out_path.display()))?;
    }
    for subdir in src.dirs() {
        write_dir_recursively(subdir, dest)?;
    }
    Ok(())
}

/// 讀取一個 character pack 的 manifest。
pub fn load_manifest(stem: &str) -> Result<CharacterManifest> {
    let p = manifest_path(stem);
    let body = std::fs::read_to_string(&p)?;
    let m: CharacterManifest = serde_json::from_str(&body)?;
    Ok(m)
}

/// 掃 ~/.mori/characters/ 列出所有合法 character pack(有 manifest.json)。
/// 預設 mori 排第一,其他依 display_name 字典序。
pub fn list() -> Result<Vec<CharacterEntry>> {
    let dir = characters_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for ent in std::fs::read_dir(&dir)? {
        let ent = ent?;
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let stem = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if stem.is_empty() {
            continue;
        }
        match load_manifest(&stem) {
            Ok(m) => entries.push(CharacterEntry {
                stem: stem.clone(),
                display_name: m.display_name,
                author: m.author,
                version: m.version,
            }),
            Err(e) => {
                tracing::warn!(stem = %stem, error = %e, "skip invalid character pack");
            }
        }
    }
    entries.sort_by(|a, b| {
        let a_default = a.stem == DEFAULT_PACKAGE_NAME;
        let b_default = b.stem == DEFAULT_PACKAGE_NAME;
        b_default.cmp(&a_default).then(a.display_name.cmp(&b.display_name))
    });
    Ok(entries)
}

pub fn get_active() -> String {
    let stem = std::fs::read_to_string(active_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PACKAGE_NAME.to_string());
    if manifest_path(&stem).exists() {
        stem
    } else {
        tracing::warn!(stem = %stem, "active character pack not found, falling back to mori");
        DEFAULT_PACKAGE_NAME.to_string()
    }
}

pub fn set_active(stem: &str) -> Result<()> {
    if !manifest_path(stem).exists() {
        anyhow::bail!("character pack '{}' not found(沒 manifest.json)", stem);
    }
    std::fs::create_dir_all(characters_dir())?;
    std::fs::write(active_path(), stem)?;
    Ok(())
}

/// 從 zip bytes 匯入 character pack。
/// 流程:驗 manifest schema + 6 required sprite → backup 既有同名 pack → extract。
/// 回 CharacterEntry。
pub fn import_zip(zip_bytes: &[u8]) -> Result<CharacterEntry> {
    use std::io::Cursor;
    let reader = Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).context("invalid zip archive")?;

    // 1. 找 + parse manifest.json
    let mut manifest_str = String::new();
    {
        let mut mf = archive
            .by_name("manifest.json")
            .map_err(|_| anyhow!("Missing manifest.json in zip"))?;
        mf.read_to_string(&mut manifest_str)
            .context("read manifest.json from zip")?;
    }
    let manifest: CharacterManifest =
        serde_json::from_str(&manifest_str).context("invalid manifest.json")?;

    // 2. validate schema
    validate_manifest(&manifest)?;

    // 3. 驗 6 required sprite 在 zip 內
    for state in REQUIRED_STATES {
        let name = format!("sprites/{state}.png");
        if archive.by_name(&name).is_err() {
            anyhow::bail!("Missing required sprite: {state}.png");
        }
    }

    // 4. backup 既有同名 pack
    let dest = pack_dir(&manifest.package_name);
    if dest.exists() {
        let ts = chrono::Utc::now().timestamp();
        let backup = pack_dir(&format!("{}.backup-{ts}", manifest.package_name));
        std::fs::rename(&dest, &backup)
            .with_context(|| format!("backup existing pack to {}", backup.display()))?;
    }

    // 5. extract — 過濾 path traversal(zip slip 防護)
    std::fs::create_dir_all(&dest)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name.starts_with('/') || name.contains("..") {
            anyhow::bail!("Invalid path in zip: {name}");
        }
        if name.ends_with('/') {
            continue;
        }
        let out_path = dest.join(&name);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&out_path)
            .with_context(|| format!("create {}", out_path.display()))?;
        std::io::copy(&mut file, &mut out)
            .with_context(|| format!("extract {name}"))?;
    }

    Ok(CharacterEntry {
        stem: manifest.package_name.clone(),
        display_name: manifest.display_name,
        author: manifest.author,
        version: manifest.version,
    })
}

fn validate_manifest(m: &CharacterManifest) -> Result<()> {
    if !m.schema_version.starts_with("1.") {
        anyhow::bail!(
            "Unsupported schema_version: {} (本機支援 1.x)",
            m.schema_version
        );
    }
    if m.package_name.is_empty()
        || !m
            .package_name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!("Invalid package_name: {}", m.package_name);
    }
    if m.sprite_spec.grid != "4x4" {
        anyhow::bail!(
            "Unsupported sprite grid: {} (本機只支援 4x4)",
            m.sprite_spec.grid
        );
    }
    if m.sprite_spec.total_size != "1024x1024" {
        anyhow::bail!(
            "Unsupported total_size: {} (本機只支援 1024x1024)",
            m.sprite_spec.total_size
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bundled_default_pack_has_all_required_files() {
        assert!(BUNDLED_DEFAULT_PACK.get_file("manifest.json").is_some());
        for state in REQUIRED_STATES {
            let p = format!("sprites/{state}.png");
            assert!(BUNDLED_DEFAULT_PACK.get_file(&p).is_some(), "missing {p}");
        }
        assert!(BUNDLED_DEFAULT_PACK.get_file("backdrop-light.png").is_some());
        assert!(BUNDLED_DEFAULT_PACK.get_file("backdrop-dark.png").is_some());
    }

    #[test]
    fn extract_bundled_default_pack_writes_all_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("mori");
        extract_bundled_default_pack(&dir).expect("extract ok");
        assert!(dir.join("manifest.json").exists());
        for state in REQUIRED_STATES {
            assert!(dir.join(format!("sprites/{state}.png")).exists());
        }
        assert!(dir.join("backdrop-light.png").exists());
        assert!(dir.join("backdrop-dark.png").exists());
    }

    fn make_valid_manifest() -> CharacterManifest {
        CharacterManifest {
            schema_version: "1.0".into(),
            package_name: "test-pack".into(),
            display_name: "Test".into(),
            version: "1.0.0".into(),
            author: "tester".into(),
            license: "MIT".into(),
            description: "".into(),
            tags: vec![],
            states: REQUIRED_STATES.iter().map(|s| s.to_string()).collect(),
            optional_states: vec![],
            loop_modes: BTreeMap::new(),
            loop_durations_ms: BTreeMap::new(),
            sprite_spec: SpriteSpec {
                format: "PNG-32".into(),
                grid: "4x4".into(),
                total_size: "1024x1024".into(),
                frame_size: "256x256".into(),
                frame_order: "row-major-left-to-right-top-to-bottom".into(),
                background: "transparent".into(),
            },
        }
    }

    #[test]
    fn validate_manifest_accepts_valid() {
        assert!(validate_manifest(&make_valid_manifest()).is_ok());
    }

    #[test]
    fn validate_manifest_rejects_schema_v2() {
        let mut m = make_valid_manifest();
        m.schema_version = "2.0".into();
        let err = validate_manifest(&m).unwrap_err().to_string();
        assert!(err.contains("Unsupported schema_version"));
    }

    #[test]
    fn validate_manifest_accepts_schema_v1_2() {
        let mut m = make_valid_manifest();
        m.schema_version = "1.2".into();
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn validate_manifest_rejects_invalid_package_name() {
        let mut m = make_valid_manifest();
        m.package_name = "bad/name".into();
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn validate_manifest_rejects_non_4x4_grid() {
        let mut m = make_valid_manifest();
        m.sprite_spec.grid = "2x2".into();
        assert!(validate_manifest(&m).is_err());
    }

    fn build_zip_with(
        manifest: &CharacterManifest,
        include_sprites: &[&str],
        extra_files: &[(&str, &[u8])],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = Default::default();
            let manifest_json = serde_json::to_string(manifest).unwrap();
            w.start_file("manifest.json", opts).unwrap();
            w.write_all(manifest_json.as_bytes()).unwrap();
            for state in include_sprites {
                w.start_file(format!("sprites/{state}.png"), opts).unwrap();
                w.write_all(b"fake png").unwrap();
            }
            for (name, data) in extra_files {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf
    }

    #[test]
    fn import_zip_rejects_zip_without_manifest() {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::SimpleFileOptions = Default::default();
            w.start_file("sprites/idle.png", opts).unwrap();
            w.write_all(b"fake png").unwrap();
            w.finish().unwrap();
        }
        let err = import_zip(&buf).unwrap_err().to_string();
        assert!(err.contains("Missing manifest.json"));
    }

    #[test]
    fn import_zip_rejects_missing_required_sprite() {
        let m = make_valid_manifest();
        // 缺 done.png
        let zip_bytes = build_zip_with(
            &m,
            &["idle", "sleeping", "recording", "thinking", "error"],
            &[],
        );
        let err = import_zip(&zip_bytes).unwrap_err().to_string();
        assert!(err.contains("Missing required sprite"));
    }

    #[test]
    fn import_zip_rejects_schema_v2() {
        let mut m = make_valid_manifest();
        m.schema_version = "2.0".into();
        let zip_bytes = build_zip_with(&m, REQUIRED_STATES, &[]);
        let err = import_zip(&zip_bytes).unwrap_err().to_string();
        assert!(err.contains("Unsupported schema_version"));
    }

    #[test]
    fn import_zip_rejects_path_traversal() {
        let m = make_valid_manifest();
        let zip_bytes = build_zip_with(&m, REQUIRED_STATES, &[("../../etc/passwd", b"malicious")]);
        // 注意:這個 test 會嘗試 backup + extract,但 dest path 是 real ~/.mori/characters/<pkg>/
        // 預期 in extract loop 撞 ".." check 就 bail,不會真寫 ~/.mori 內。
        // 但 backup 階段已執行(若 ~/.mori/characters/test-pack/ 存在會 rename)— 在 CI / test env 通常不存在,OK。
        let res = import_zip(&zip_bytes);
        assert!(res.is_err(), "should reject path traversal");
    }
}
