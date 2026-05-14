# Wave 4 e2e 手動測試清單

> Wave 4 step 12。step 1-11 全 commit 完成,本檔給 yazelin 跑一輪實際 e2e
> 把所有 invariants 跟 happy path 都驗一遍。

## 前置

兩個 process 同時跑:

### Terminal 1 — annuli HTTP server

```bash
cd ~/mori-universe/annuli
ANNULI_SOUL_TOKEN=$(openssl rand -hex 32) \
  ANNULI_ADMIN_PASS= \
  .venv/bin/python main.py admin --port 5000
```

或者用部署機既有的(若你正式機 annuli-admin.service 已升到 Wave 3+ + Wave 4 prep PR 都 merged),`https://ching-tech.ddns.net/jinn/` 也可。

**記下 SOUL_TOKEN 值**(等下要寫 config.json)。

### Terminal 2 — 配 mori-desktop 連 annuli

編 `~/.mori/config.json`,加 `annuli` 段:

```json
{
  "annuli": {
    "enabled": true,
    "endpoint": "http://localhost:5000",
    "spirit_name": "mori",
    "user_id": "yazelin",
    "soul_token": "<上面 openssl rand -hex 32 印出的字串>",
    "basic_auth": null,
    "timeout_secs": 10
  }
}
```

(若用正式機 https endpoint,加 basic_auth `{ "user": "ct", "pass": "..." }`,但 prod 通常跑的是 Jinn vault 而非 Mori,要小心 spirit_name 對齊。)

### Terminal 3 — mori-desktop

```bash
cd ~/mori-universe/mori-desktop
npm run tauri dev
```

啟動 log 應看到:
```
INFO ... annuli memory store enabled — 透過 HTTP 跟 vault 互動
INFO ... endpoint=http://localhost:5000 spirit=mori user_id=yazelin
```

## 測試清單

### A. AnnuliTab UI 基本顯示

- [ ] 點 sidebar 「Annuli」tab(年輪 icon)
- [ ] status bar 顯示 🟢 connected + endpoint / spirit / user_id 對得上 config.json
- [ ] SOUL.md 顯示有內容(若 vault 是 mori-journal clone 過來的,應該有真實 SOUL 內容)
- [ ] MEMORY § sections 列表非空(或空就空,但不該 error)
- [ ] 今日 events 一開始可能 0(還沒對話),OK

### B. 對話事件 fire-and-forget POST /events

- [ ] 切到 Chat tab,跟 Mori 對話一輪(隨便打「你好」之類)
- [ ] 回到 Annuli tab,點重新整理
- [ ] 今日 events 應該多 **2** 條(role=user 一條、role=assistant 一條)
- [ ] 直接 `cat ~/mori-universe/spirits/mori/events/$(date +%Y-%m-%d).md` 看 JSONL 真的 append 了

### C. /sleep hotkey + 按鈕

- [ ] AnnuliTab 點「🌙 /sleep」按鈕
- [ ] 顯示「✅ ring 寫好:.../rings/<date>_ringN.md」
- [ ] 真實檔案存在:`ls ~/mori-universe/spirits/mori/rings/`
- [ ] **invariant**:`diff <(cat ~/mori-universe/spirits/mori/identity/SOUL.md)` 跟 sleep 前一樣(byte-exact)
- [ ] **invariant**:`diff <(cat ~/mori-universe/spirits/mori/memories/MEMORY.md)` 跟 sleep 前一樣

按 Ctrl+Alt+Z hotkey 應該也觸發同樣效果(X11 / Wayland 都該過,看 annuli log 應該收到 POST /rings/new)。

### D. SOUL.md X-Soul-Token guard

直接 curl 驗:

```bash
# 沒帶 token → 403
curl -X PUT http://localhost:5000/spirits/mori/soul \
  -d "EVIL LLM REWRITE"
# 應該回 403 Forbidden,SOUL.md 不變

# 帶錯 token → 403
curl -X PUT http://localhost:5000/spirits/mori/soul \
  -H "X-Soul-Token: wrong" -d "EVIL"
# 應該 403

# 帶對 token → 200
curl -X PUT http://localhost:5000/spirits/mori/soul \
  -H "X-Soul-Token: <你的 token>" -d "# Mori updated by user"
# 應該回 {"ok":true,"bytes_written":N}
# 然後 SOUL.md 真的改變了

# 還原:vim ~/mori-universe/spirits/mori/identity/SOUL.md
```

### E. POST /memory/section user-explicit memory write

mori-desktop 端通過 RememberSkill 觸發:對 Mori 說「**請記住:我下午會去咖啡店**」,
LLM 該呼叫 remember_memory tool。看 vault:

```bash
cat ~/mori-universe/spirits/mori/memories/MEMORY.md
# 應該末尾多一個 `## § <header>` section,body 是你說的內容
```

### F. ForgetMemorySkill curator toast

對 Mori 說「**忘掉 <id>**」(隨便挑一個 memory id),期望:
- chat panel 顯示 toast:「我不能直接刪 `<id>`。vault 設計上是 append-only — 真的要忘掉的話請走 `/sleep` 後跑 curator dry-run + yaml approve + apply 流程。」
- LLM **沒**因為 skill 失敗而重試 / 報錯,流程順
- MEMORY.md 沒被改

### G. annuli 沒跑時 graceful fallback

關掉 Terminal 1 的 annuli server,然後 reload mori-desktop(`Ctrl+R` 或重 launch)。

- [ ] **若 annuli.enabled=true 但 server 沒跑**:啟動仍成功,但 AnnuliTab 顯示 🔴 unreachable + 錯誤訊息,sleep 按鈕 disabled
- [ ] **對話照樣 work**(只是 events 寫不進 vault,log 有 warn)
- [ ] Memory tab(舊 LocalMarkdown 那個)走的是 AnnuliMemoryStore,所以 read_index 會回空 — 不太理想但本 Wave 預期。**未來**(Wave 5+)可以做 fallback:annuli 沒跑時切回 LocalMarkdown

### H. 跨機器 user_id 約定

(若你有第二台機器接同一個 vault,git pull 過來)

- [ ] 兩台機器 `~/mori-universe/spirits/mori/identity/user_id` 應該都是同一行(`yazelin`)
- [ ] 不論在 ct / yaze / yazel 哪個 OS user 跑,event source 標機器名,但 user_id 都是 yazelin

## 跑完之後

把這份 checklist 打勾(或回報 fail 哪幾條),Wave 4 收工。

接著開 PR 進 main,Wave 5 開新 branch 看條件成不成熟拆 annuli-creator。
