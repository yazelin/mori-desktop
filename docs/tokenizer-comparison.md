# Tokenizer comparison:中文 vs 英文 starter profile

> v0.4.1 加入 EN starter 之前先做的 baseline 研究。給「想自己研究哪個 starter 套省 token」的 power user 看,提供數據 + 取捨建議。

Mori 的 system prompt 是 LLM 每輪 chat 都重送的「常駐成本」 — 用越多 token,就越快踩到 TPM rate limit + agent 多輪 tool-call 越貴。我們的 v0.3 系列 starter 是純中文,2026-05 加新範本時順手量了一下 EN 翻譯版的 token 數,結論直接影響 v0.4.1 的設計(中英 starter 並行 + Quickstart 偵測 OS locale 預設選 EN)。

## 1. 怎麼量

對比 4 對中英 starter(`USER-03.LINE貼文 / USER-04.哄老婆開心 / USER-05.提示詞優化 / AGENT-05.聽我指令`)的 system prompt body(去掉 YAML frontmatter,因為 LLM 只看 body)。

跑兩個 tokenizer:

- **`o200k_harmony`** — gpt-oss-120b / gpt-oss-20b 的官方 encoding。Mori 預設走 `provider: groq` + `gpt-oss-120b` 時 LLM 端用這個算 token
- **Gemini Flash** — 透過 `google-genai` SDK 的 `count_tokens()` 真打 API 算的(SentencePiece-like,跟 BPE 不同)

`cl100k_base`(GPT-4 / GPT-3.5)也跑了一輪當 reference,結果類似 o200k_harmony 但對中文更不友善。

跑法(可重現):

```bash
# 自帶 uv 環境,不污染本機 Python
uvx --with tiktoken python compare.py
uvx --with google-genai python compare_gemini.py
```

兩支 script 在 [`scratch/tokenizer-test/`](https://github.com/yazelin/mori-desktop/blob/main/scratch/tokenizer-test/)(repo 內 gitignore,但開發本機看得到)。`.env` 要有 `GEMINI_API_KEY` 才跑得了 Gemini 那條。

## 2. 結果

每筆是同份範本內容,中英兩語 LLM 看到的 system prompt 各算多少 token:

| profile | lang | chars | o200k_harmony(gpt-oss)| Gemini Flash | cl100k_base(GPT-4)| chars/tok(harmony)|
|---|---|---:|---:|---:|---:|---:|
| USER-03 LINE 貼文 | zh | 695 | **509** | 438 | 694 | 1.37 |
| USER-03 LINE post | en | 1543 | **370** | 386 | 375 | 4.17 |
| USER-04 哄老婆 | zh | 712 | **592** | 492 | 841 | 1.20 |
| USER-04 sweet | en | 1843 | **432** | 461 | 449 | 4.27 |
| USER-05 提示詞優化 | zh | 1104 | **703** | 597 | 957 | 1.57 |
| USER-05 prompt-optim | en | 2231 | **497** | 531 | 501 | 4.49 |
| AGENT-05 聽我指令 | zh | 1253 | **710** | 640 | 875 | 1.76 |
| AGENT-05 listen | en | 2055 | **555** | 606 | 557 | 3.70 |
| **TOTAL** | **zh** | 3764 | **2514** | **2167** | **3367** | 1.50 |
| **TOTAL** | **en** | 7672 | **1854** | **1984** | **1882** | 4.14 |

「EN 比 ZH 省 token」的差距:

| Tokenizer / Provider | EN 省了 |
|---|---|
| gpt-oss-120b(Groq 預設)| **-26.3%** |
| Gemini Flash(`gemini-3.1-flash-*`)| **-8.4%** |
| GPT-4(`cl100k_base`,僅參考)| -44.1% |

## 3. 為什麼差這麼多

中文是「字 = 概念」結構,平均 1.5 個字一顆 token(gpt-oss);英文是「詞 = 概念」結構,常見詞每 token 塞 4-5 個字。同樣的「意思密度」,英文每 token 多裝 2.7 倍的內容。

不同 tokenizer 對中文友善度差異:

- **GPT-4 cl100k_base**:中文最慘,1.1 字 / token(差不多每字一顆),所以對它而言 EN 省 44%
- **gpt-oss o200k_harmony**:中文改善到 1.5 字 / token,但英文也跟著更精煉(4.14 chars/tok)→ EN 仍省 26%
- **Gemini Flash**:中文相對最好 1.74 字 / token,英文反而沒 OpenAI 那條精煉(3.87 chars/tok)→ EN 只省 8.4%

## 4. 對 Mori 的實際影響

Groq 對 `openai/gpt-oss-120b` on-demand TPM 是 **8000**。Mori agent 多輪 tool-call(每輪重送 system prompt + tools schema + 對話歷史)會疊 TPM 很快。

舉個典型 case — 你切到 USER-05 提示詞優化 profile 講話:

- ZH system prompt ~700 tokens → 一輪對話 system 佔 8.7% TPM
- EN system prompt ~500 tokens → 一輪對話 system 佔 6.2% TPM

agent loop 跑 3-5 輪時,**ZH 比 EN 早 25% 踩到 8000 TPM limit**,Groq 回 413 → Mori fallback 或 user 等。

對 Gemini provider 影響小(8.4%),對 Groq / Claude / 其他 BPE family provider 都明顯。

## 5. 取捨建議

| 你的場景 | 建議 starter 語系 |
|---|---|
| 主要走 Groq(gpt-oss-120b)+ 多輪 agent | **EN**(省 TPM,踩 limit 機率低)|
| 主要走 Gemini provider | 看你習慣 — 中英差距 <10% 影響小 |
| 主要走 Claude(`claude-bash` / `claude-cli`)| 接近 GPT-4 cl100k_base 行為,**EN 較省** |
| 想自己改 .md 加 mapping table / 個人化 | **ZH**(改寫繁中內容比較順,token 多但個人化價值高) |
| 第一次裝、不確定 | **EN**(預設值)— 跑得久踩 limit 機率低 |

## 6. v0.4.1 的設計選擇

基於以上:

- **Quickstart 加 starter 語系 picker**:第一次啟動讓 user 選「中文 / 英文」。預設值依 OS locale(`navigator.language` 開頭 `zh-*` → 中文,其他 → 英文)
- **Binary 兩語都 `include_str!` 包**:`~/.mori/{voice_input,agent}/` 第一次自動 deploy 的份依 config `starter_locale` 決定
- **Profiles tab 加「加入範本」按鈕**:任何時候都可以從內建 starter 撈一份覆寫進去(改壞了想還原 / 想試另一語系版)

第一次選錯了沒關係,Profiles tab 永遠可補。

## 7. 限制 / 注意

- 量的是 **system prompt body**,沒算 **runtime 注入 context**(時間 / clipboard preview / selection / memory index)。後者每輪都加,實測整個 prompt 規模 system + 動態大約 1:1 至 1:2 比例
- 沒量 **tool schema** 跟 **agent loop 對話歷史**累積。多輪疊起來,system prompt 佔比反而下降
- 中英 starter 用詞密度可能受我翻譯品質影響(faithful 翻譯但不是同份語料,結構等價)。誤差約 ±5%
- 不同 prompt 結構中英 ratio 可能差異更大或更小(table / bullet / 標點密度都影響)
