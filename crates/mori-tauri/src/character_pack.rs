//! Character pack 系統。
//!
//! 角色 sprite + 設定打包成「character pack」放在 `~/.mori/characters/<name>/`,
//! user 可替換成自製角色 — 設計目標是讓未來 yazelin 寫的 generator app 能輸出
//! 完全符合規格的 `.moripack.zip`,user 解壓進來就能切換。
//!
//! ## 結構
//! ```text
//! ~/.mori/characters/
//! ├── mori/                       ← 預設 character(開機 ensure 寫入)
//! │   ├── manifest.json
//! │   └── sprites/
//! │       ├── idle.png             ← 256×256(default placeholder)
//! │       ├── sleeping.png         ← 之後 placeholder script 會升 1024×1024 4×4
//! │       ├── recording.png
//! │       ├── thinking.png
//! │       ├── done.png
//! │       └── error.png
//! ├── <user-imported>/...          ← user 自己加 / 從 .moripack.zip import
//! └── active                       ← 一行,當前 active character name(沒檔回 "mori")
//! ```
//!
//! ## Schema versioning
//! `manifest.schema_version` 讓未來 schema 改不破壞舊 pack — engine 讀到不認識的
//! version 會 warn + 嘗試 best-effort 載入(沿用必含欄位)。
//!
//! ## 規範文件
//! 完整給 generator app 開發者 + import 角色 user 的規範在
//! `docs/character-pack.md`。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 完整 character pack manifest — 對應 manifest.json。
/// `schema_version` 必含;其他欄位走 serde default 容忍 partial / forward-compat。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterManifest {
    /// 規範版本(目前 "1.0")
    pub schema_version: String,
    /// 唯一 ID(snake-case),import 時當資料夾名
    pub package_name: String,
    /// UI 顯示名
    pub display_name: String,
    /// 此 pack 版本(semver)
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
    /// 必含 sprite states(讀不到對應 sprite 會 fallback 到 default mori)
    pub states: Vec<String>,
    /// 可選 sprite states(沒提供不算錯)
    #[serde(default)]
    pub optional_states: Vec<String>,
    /// 每 state 是 loop 還是 one-shot
    #[serde(default)]
    pub loop_modes: BTreeMap<String, String>,
    /// 每 state 一個 loop 跑完多久(ms)
    #[serde(default)]
    pub loop_durations_ms: BTreeMap<String, u32>,
    /// Sprite sheet 規格(engine 依此決定 CSS animation)
    pub sprite_spec: SpriteSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpriteSpec {
    /// "PNG-32"(RGBA)
    pub format: String,
    /// "1x1"(single-frame static)或 "4x4"(16-frame animation)等
    pub grid: String,
    /// 整張 PNG 尺寸,例 "256x256" / "1024x1024" / "2048x2048"
    pub total_size: String,
    /// 單 frame 尺寸,例 "256x256" / "512x512"
    pub frame_size: String,
    /// "row-major-left-to-right-top-to-bottom"(目前唯一支援)
    pub frame_order: String,
    /// "transparent" / "white" / 等
    pub background: String,
}

/// 給 UI 列舉 character pack 用的精簡 entry。
#[derive(Debug, Clone, Serialize)]
pub struct CharacterEntry {
    /// 資料夾名(=`package_name`)
    pub stem: String,
    pub display_name: String,
    pub author: String,
    pub version: String,
}

const DEFAULT_PACKAGE_NAME: &str = "mori";
const DEFAULT_SCHEMA_VERSION: &str = "1.0";

// 5P-1: 預設 mori 角色 sprite 內嵌 binary,ensure 時寫入 ~/.mori/characters/mori/。
// 之後 user 跑 scripts/sprite-placeholder.sh 把這幾張 256×256 升 1024×1024 4×4
// placeholder(動畫 ON 看起來不閃),再之後正式 sprite generator app 出來覆蓋。
const SPRITE_IDLE: &[u8] = include_bytes!("../../../public/floating/mori-idle.png");
const SPRITE_SLEEPING: &[u8] = include_bytes!("../../../public/floating/mori-sleeping.png");
const SPRITE_RECORDING: &[u8] = include_bytes!("../../../public/floating/mori-recording.png");
const SPRITE_THINKING: &[u8] = include_bytes!("../../../public/floating/mori-thinking.png");
const SPRITE_DONE: &[u8] = include_bytes!("../../../public/floating/mori-done.png");
const SPRITE_ERROR: &[u8] = include_bytes!("../../../public/floating/mori-error.png");

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

/// 啟動時:確保 ~/.mori/characters/mori/ 存在 + 寫入 default sprite + manifest。
/// 已存在的檔不覆蓋 — user 編輯過(或自製 sprite 替換過)保留。
///
/// 5P-2: 寫入時就把內嵌 256×256 single-frame PNG tile 升 1024×1024 4×4
/// placeholder(16 格全是同一張 frame 1)— 動畫 ON 看起來不閃(每 frame
/// 都長一樣),正式 sprite generator app 出來後 user 替換同檔名就會動。
/// 不依賴 ImageMagick — 用 Rust `image` crate inline 做。
pub fn ensure_default() -> Result<()> {
    let dir = pack_dir(DEFAULT_PACKAGE_NAME);
    std::fs::create_dir_all(dir.join("sprites"))?;

    let manifest = default_manifest();
    let manifest_p = manifest_path(DEFAULT_PACKAGE_NAME);
    if !manifest_p.exists() {
        let json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(&manifest_p, json)?;
    }

    let sprite_dir = dir.join("sprites");
    for (state, bytes) in [
        ("idle", SPRITE_IDLE),
        ("sleeping", SPRITE_SLEEPING),
        ("recording", SPRITE_RECORDING),
        ("thinking", SPRITE_THINKING),
        ("done", SPRITE_DONE),
        ("error", SPRITE_ERROR),
    ] {
        let p = sprite_dir.join(format!("{state}.png"));
        if !p.exists() {
            let tiled = tile_4x4_placeholder(bytes)
                .with_context(|| format!("tile sprite '{state}' to 4x4 placeholder"))?;
            std::fs::write(&p, tiled)?;
        }
    }
    Ok(())
}

/// 把 256×256(或任意大小)PNG 拼成 1024×1024 4×4 sheet,16 格全是同一張。
/// 動畫 ON 看起來不閃,等正式 16-frame loop sheet 上來覆蓋同檔名就會動。
fn tile_4x4_placeholder(png_bytes: &[u8]) -> Result<Vec<u8>> {
    use image::{ImageBuffer, ImageEncoder, Rgba};

    let src = image::load_from_memory(png_bytes)
        .context("decode source PNG")?
        .to_rgba8();
    // 強制 normalize 到 256×256(若 source 不是這個尺寸)
    let cell = image::imageops::resize(
        &src,
        256,
        256,
        image::imageops::FilterType::Lanczos3,
    );

    // 1024×1024 RGBA empty canvas
    let mut sheet: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(1024, 1024);
    for row in 0..4 {
        for col in 0..4 {
            image::imageops::replace(
                &mut sheet,
                &cell,
                (col * 256) as i64,
                (row * 256) as i64,
            );
        }
    }

    // Encode 回 PNG bytes
    let mut out = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut out);
        image::codecs::png::PngEncoder::new(&mut cursor)
            .write_image(
                sheet.as_raw(),
                sheet.width(),
                sheet.height(),
                image::ExtendedColorType::Rgba8,
            )
            .context("encode tiled PNG")?;
    }
    Ok(out)
}

fn default_manifest() -> CharacterManifest {
    let mut loop_modes = BTreeMap::new();
    let mut loop_durations_ms = BTreeMap::new();
    for (state, mode, dur) in [
        ("idle", "loop", 3000u32),
        ("sleeping", "loop", 5000),
        ("recording", "loop", 1500),
        ("thinking", "loop", 2000),
        ("done", "one-shot", 600),
        ("error", "one-shot", 800),
    ] {
        loop_modes.insert(state.to_string(), mode.to_string());
        loop_durations_ms.insert(state.to_string(), dur);
    }

    CharacterManifest {
        schema_version: DEFAULT_SCHEMA_VERSION.to_string(),
        package_name: DEFAULT_PACKAGE_NAME.to_string(),
        display_name: "Mori".to_string(),
        version: "1.0.0".to_string(),
        author: "yazelin".to_string(),
        license: "CC-BY-NC-SA-4.0".to_string(),
        description: "森林精靈,Mori-desktop 預設角色".to_string(),
        tags: vec!["fantasy".into(), "elf".into(), "cute".into(), "official".into()],
        states: vec![
            "idle".into(),
            "sleeping".into(),
            "recording".into(),
            "thinking".into(),
            "done".into(),
            "error".into(),
        ],
        optional_states: vec!["walking".into(), "dragging".into()],
        loop_modes,
        loop_durations_ms,
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

/// 讀取一個 character pack 的 manifest。檔案不存在 / JSON 無效都回 Err。
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

/// 讀 ~/.mori/characters/active(一行 stem)。沒檔 / 空檔 / 對應 pack 不存在 → "mori"。
pub fn get_active() -> String {
    let stem = std::fs::read_to_string(active_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_PACKAGE_NAME.to_string());
    // 驗證 manifest 存在,否則 fallback default
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

/// 5P-2:升級任意 character pack 內的 sprite PNG 到 4×4 1024×1024 placeholder。
/// User 從 generator app import 的 pack 若是 single-frame 256×256,呼叫這個讓
/// frame 1 duplicate fill 16 格(動畫 ON 不閃)。
/// - 已是 1024×1024 的檔跳過
/// - 原檔備份到 sprites/.backup-<timestamp>/
/// - 完成後寫回 manifest.json 的 sprite_spec(grid="4x4")
/// 回傳 (upgraded_count, skipped_count)。
pub fn upgrade_pack_to_4x4(stem: &str) -> Result<(usize, usize)> {
    use image::GenericImageView;

    let sprites_dir = pack_dir(stem).join("sprites");
    if !sprites_dir.exists() {
        anyhow::bail!("character pack '{}' has no sprites/ directory", stem);
    }
    let manifest_p = manifest_path(stem);
    if !manifest_p.exists() {
        anyhow::bail!("character pack '{}' has no manifest.json", stem);
    }

    let ts = chrono::Utc::now().timestamp();
    let backup_dir = sprites_dir.join(format!(".backup-{ts}"));
    let mut upgraded = 0usize;
    let mut skipped = 0usize;

    for entry in std::fs::read_dir(&sprites_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        // 略過 hidden / backup 目錄
        if path.file_name().and_then(|s| s.to_str()).map(|n| n.starts_with('.')).unwrap_or(false) {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let img = match image::load_from_memory(&bytes) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "skip non-decodable PNG");
                continue;
            }
        };
        let (w, h) = img.dimensions();
        if w == 1024 && h == 1024 {
            skipped += 1;
            continue;
        }
        // 備份
        std::fs::create_dir_all(&backup_dir)?;
        let fname = path.file_name().unwrap();
        std::fs::copy(&path, backup_dir.join(fname))?;
        // 升級
        let tiled = tile_4x4_placeholder(&bytes)
            .with_context(|| format!("tile {}", path.display()))?;
        std::fs::write(&path, tiled)?;
        upgraded += 1;
    }

    if upgraded > 0 {
        let mut manifest = load_manifest(stem)?;
        manifest.sprite_spec.grid = "4x4".into();
        manifest.sprite_spec.total_size = "1024x1024".into();
        manifest.sprite_spec.frame_size = "256x256".into();
        let json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(&manifest_p, json)?;
    }

    Ok((upgraded, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    #[test]
    fn tile_4x4_outputs_1024_square() {
        // 任 input(用內嵌 idle.png 當 sample)→ 1024×1024 RGBA 16-cell sheet
        let out = tile_4x4_placeholder(SPRITE_IDLE).expect("tile ok");
        let img = image::load_from_memory(&out).expect("decode tiled");
        let (w, h) = img.dimensions();
        assert_eq!((w, h), (1024, 1024), "tile output must be 1024×1024");
    }

    #[test]
    fn tile_4x4_cells_are_identical() {
        // frame 1 跟 frame 6(任意中間格)pixel 該完全一樣 — placeholder 不閃
        let out = tile_4x4_placeholder(SPRITE_IDLE).expect("tile ok");
        let img = image::load_from_memory(&out).expect("decode").to_rgba8();
        // 比較 (10, 10) vs (10+512, 10)(row 0 cell 0 vs row 0 cell 2)
        let p0 = img.get_pixel(10, 10);
        let p2 = img.get_pixel(10 + 512, 10);
        assert_eq!(p0, p2, "frame 1 跟 frame 3 該 pixel 完全相同");
        // 跨 row 也檢查
        let p_row1_col0 = img.get_pixel(10, 10 + 256);
        assert_eq!(p0, p_row1_col0, "row 0 vs row 1 該 pixel 相同");
    }
}
