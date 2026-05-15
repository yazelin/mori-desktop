//! STT corrections baseline — `~/.mori/corrections.md` fresh-install 內容。
//!
//! 過去 user 拿到 Mori,`corrections.md` 是空檔,每個人從零自建字典 — 門檻太
//! 高沒人會做。v0.5.1 起 ship 一份預載 baseline:把常見中文 STT 諧音錯字
//! (技術詞 / 模型 / 工具 / 概念詞)整理進去,profile body 內 `#file:` 引用
//! 馬上有 200+ 條基礎校正。
//!
//! ## 來源
//!
//! 主要從 [ZeroType](https://github.com/wholee/zerotype) 的 `SYSTEM.md`
//! mapping table 取經 — Will / 保哥 / 多奇團隊用 ZeroType 跑了一段時間累積的
//! 「常見 STT 把這念成那」清單。`Names & Identity` 那段太屬於他們圈子的個人化
//! 不抄,**通用技術 / 諧音 / 品牌 / 模型 / 工具**那兩段是黃金內容,Mori 用
//! 戶大多是台灣中文 + AI / 技術領域,重疊度很高。
//!
//! ## 修改規則
//!
//! - 「baseline」段(Mori bundle)— user **不該手動改**(會被 ensure_init 覆蓋)
//! - 「user」段 — user 自己加自己的圈子人名 / 公司專有名詞 / 任何個人化
//!
//! 兩段以 markdown header 區隔,parser 可分辨。

/// `~/.mori/corrections.md` 的預載內容。
/// `ensure_corrections_md_initialized` 寫到磁碟前 prefix 一個檔案級警示
/// (user 編輯時看到知道頂部是 Mori 自帶 baseline,下面才是自己加的)。
pub const DEFAULT_CORRECTIONS_MD: &str = r#"# Mori STT 校正字典

> 這是 voice input / agent profile 透過 `#file:~/.mori/corrections.md` 引用的共用校正字典。
>
> 改了下次熱鍵生效,**不必重啟 Mori**(`enable_read: true` 的 profile 才會載入)。

## Baseline(Mori 自帶 — 致謝 ZeroType / Will / 保哥團隊)

以下兩段是常見中文 STT 諧音錯字校正(技術詞、模型、工具、概念),由 [ZeroType](https://github.com/wholee/zerotype) Will / 保哥團隊累積的 mapping table 啟發整理。**你的 user 段在下方**,要客製化請寫到「User」段,不要直接動 baseline(下次 Mori 更新可能覆蓋)。

### 常見對話 / 諧音校正

- 馬當, 馬档, modem, Modem, mod on -> Markdown
- 滾刮爛手 -> 滾瓜爛熟
- 穴油環境 -> Shell 環境
- 殺箱, 殺核 -> 沙盒
- 可以學度嗎 -> 可以 sudo 嗎
- 大羽模型, 大圓模型, 大語文模型 -> 大語言模型
- 供單 -> 工單
- 技術在 -> 技術債
- 後目錄, 加目錄 -> 家目錄
- 發批 -> 發 PR
- 稀草 -> C 槽
- 首科 -> 手刻
- 搭內 -> dotnet
- 血扣 -> 寫 Code
- 勁囊妙技 -> 錦囊妙技
- 英檔 -> 音檔
- 新制圖 -> 心智圖
- 藍群組 -> LINE 群組
- 掌骨 -> 講古
- 呆剃頭 -> 難搞
- 幣圓, 幣原 -> 閉源
- 真六 -> 蒸餾
- 模擬模型, Mowie -> MoE 模型
- 鏟 code -> 產生程式碼
- 思維在 -> 思維債
- 違憲指令 -> 危險指令
- 全帳快取 -> 全域快取
- 雙名文件 -> 說明文件
- 韓式庫 -> 函式庫
- 質變了 -> 值變了
- 版本相容性相異性 -> 版本相容性相依性
- 檔案中文 -> 檔案總管
- 名碼 -> 明碼
- 陽純 -> 陽春
- Batch 的環境 -> Bash 的環境
- ago 部署 -> Azure 部署
- Plunk 過長 -> prompt 過長
- RM 兼 RA 負 -> rm -rf
- 中端機, 中斷機 -> 終端機
- 大內八 -> .NET 8
- 這意思 -> JavaScript
- 以色列 -> Excel
- 寫扣 -> 寫程式
- 擬核 -> 擬合
- 倍式統計 -> 貝氏統計
- 討論算 -> 討論串
- 轉入 -> 轉錄(語音轉錄場景)

### 技術詞 / 品牌 / 模型名

- CheckGPT, ChairGBT, Chad GPT, ChatGBT -> ChatGPT
- GPT-4-O, GPT-4U -> GPT-4o
- gpd4 -> GPT-4
- GPD 5 -> GPT-5
- Offer Mini, Ofo mini, O4 Mini -> o4-mini
- Clawd, Cloud 3.7, ClaudMax, Quad Code, Cloud Code -> Claude(及對應版本 / 工具)
- Cloud Office -> Claude Opus
- Sony 4, Sonic 4, Solnet -> Sonnet
- Manthropic, Enthropic -> Anthropic
- GitHub Compiler, GearCopilot, GiaCopilot -> GitHub Copilot
- Codex URI, Cortex CLI, codec cli -> Codex CLI
- NodeBugle M, Notebook LM, NotebookRM -> NotebookLM
- C-Shop, C Sharp -> C#
- Remy, remit -> README
- prong, Prong -> Prompt
- Ninus, Ninos, Neenus -> Linux
- GRIP -> grep
- CUIL, CEYL, CURL -> curl
- WSOD, WSO -> WSL
- should do -> sudo
- 100C -> Ctrl+C
- Symbolink -> Symbolic link
- Arc Key -> API Key
- pre-tier -> Prettier
- WinFoam, 文鳳 -> WinForms
- TDS -> TTS
- RIG -> RAG
- Accursor -> Cursor
- SweetGlue -> SwiGLU
- BBPE -> BPE
- Publicity -> Perplexity
- TempMonkey -> TamperMonkey
- gig clone, gig,clone -> git clone
- Mysterio -> Mistral
- Fellow Search -> Felo Search
- Gemiini -> Gemini

### Mori 自家詞(避免 STT 把 Mori 念成其他字)

- 莫里, 沒理, 摸里, Maury, Moori -> Mori
- 安努利, Annuly, 安路里 -> Annuli
- 沃德利, world tree -> world-tree
- 索爾, 騷, Sol MD -> SOUL.md
- 啟動模式 -> 喚醒模式

## User(以下你自己加)

<!-- 範例:
- 我們公司的「巨象」 -> 鉅鏡
- 我同事 阿明 = 林志明
- 「布禮」這個專案 -> Brewery
-->
"#;

/// 取 `~/.mori/corrections.md` 路徑。沒 HOME / USERPROFILE 回 None。
fn corrections_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("corrections.md"))
}

/// Fresh install:`~/.mori/corrections.md` 不存在時寫一份 baseline。
/// 已存在絕不覆蓋(user 改過的內容保留)。
/// 失敗只 warn 不 panic — 觀測層不擋業務。
pub fn ensure_corrections_md_initialized() {
    let Some(path) = corrections_path() else {
        tracing::warn!("corrections: no HOME / USERPROFILE, skipped initialization");
        return;
    };
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(?e, "corrections: mkdir parent failed");
            return;
        }
    }
    match std::fs::write(&path, DEFAULT_CORRECTIONS_MD) {
        Ok(()) => tracing::info!(
            path = %path.display(),
            chars = DEFAULT_CORRECTIONS_MD.chars().count(),
            "corrections: deployed baseline",
        ),
        Err(e) => tracing::warn!(?e, "corrections: write failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_has_attribution() {
        // 致謝 ZeroType / Will 保哥團隊 — license 來源透明
        assert!(DEFAULT_CORRECTIONS_MD.contains("ZeroType"));
        assert!(DEFAULT_CORRECTIONS_MD.contains("Will"));
        assert!(DEFAULT_CORRECTIONS_MD.contains("保哥"));
    }

    #[test]
    fn baseline_includes_user_section() {
        // 一定要有 User 段讓使用者自填,baseline / user 分離才不會被覆蓋
        assert!(DEFAULT_CORRECTIONS_MD.contains("## User"));
    }

    #[test]
    fn baseline_includes_mori_specific() {
        assert!(DEFAULT_CORRECTIONS_MD.contains("-> Mori"));
        assert!(DEFAULT_CORRECTIONS_MD.contains("-> Annuli"));
    }

    #[test]
    fn baseline_substantial_size() {
        // 至少 200 條校正規則 — 算法:每行 `- X -> Y` 都算一條
        let rules = DEFAULT_CORRECTIONS_MD
            .lines()
            .filter(|l| l.contains(" -> "))
            .count();
        assert!(rules >= 80, "expected >=80 baseline rules, got {rules}");
    }
}
