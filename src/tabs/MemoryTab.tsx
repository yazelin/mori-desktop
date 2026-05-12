// 5M placeholder — 5L+ 才會做真的 memory browser
function MemoryTab() {
  return (
    <div className="mori-tab mori-tab-memory">
      <h2 className="mori-tab-title">Memory</h2>
      <p className="mori-tab-hint">
        瀏覽 / 搜尋 / 編輯 ~/.mori/memory/ 內的長期記憶。
      </p>
      <div className="mori-tab-placeholder">
        <p>5L+ 階段做 browse / search / edit UI。</p>
        <p>目前 Mori 自己會用 RememberSkill / RecallMemorySkill / ForgetMemorySkill /
           EditMemorySkill 維護,使用者直接編輯 markdown 也可以。</p>
      </div>
    </div>
  );
}

export default MemoryTab;
