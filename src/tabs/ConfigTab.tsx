// 5M placeholder — 5L 會塞真的 config.json 表單編輯器
function ConfigTab() {
  return (
    <div className="mori-tab mori-tab-config">
      <h2 className="mori-tab-title">Config</h2>
      <p className="mori-tab-hint">
        編輯 ~/.mori/config.json:provider / stt_provider / providers.* / api_keys / routing。
      </p>
      <div className="mori-tab-placeholder">
        <p>5L 階段才會做真的表單編輯器。</p>
        <p>現在請直接編輯 <code>~/.mori/config.json</code>;改完不需要重啟,
           下一次熱鍵會即時讀新設定。</p>
      </div>
    </div>
  );
}

export default ConfigTab;
