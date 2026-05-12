// 5M placeholder — 5L+ 才會列出當前 profile 啟用的 skill 詳細資料
function SkillsTab() {
  return (
    <div className="mori-tab mori-tab-skills">
      <h2 className="mori-tab-title">Skills</h2>
      <p className="mori-tab-hint">
        列出當前 profile 啟用的 built-in skill + 自訂 shell_skill,
        含參數 schema、最近呼叫紀錄、單測按鈕。
      </p>
      <div className="mori-tab-placeholder">
        <p>5L+ 階段做 skill registry browser + 試跑面板。</p>
        <p>當前可用 skill 列表(透過 mori CLI):
           <code> mori skill list</code></p>
      </div>
    </div>
  );
}

export default SkillsTab;
