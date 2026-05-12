// 5M placeholder — 5L 才真的填編輯器內容。
// 先列出 voice / agent profile 讓使用者可以快速切換,當作 picker 的「無熱鍵版」。
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type ProfileEntry = { stem: string; display: string };

function ProfilesTab() {
  const [voice, setVoice] = useState<ProfileEntry[]>([]);
  const [agent, setAgent] = useState<ProfileEntry[]>([]);

  useEffect(() => {
    invoke<ProfileEntry[]>("picker_list_voice_profiles").then(setVoice).catch(console.error);
    invoke<ProfileEntry[]>("picker_list_agent_profiles").then(setAgent).catch(console.error);
  }, []);

  const switchVoice = (stem: string) =>
    invoke("picker_switch_voice_profile", { stem }).catch(console.error);
  const switchAgent = (stem: string) =>
    invoke("picker_switch_agent_profile", { stem }).catch(console.error);

  return (
    <div className="mori-tab mori-tab-profiles">
      <h2 className="mori-tab-title">Profiles</h2>
      <p className="mori-tab-hint">
        切換 voice / agent profile。完整 frontmatter / body 編輯器 5L 階段才會做,
        現在這頁等同 picker (Ctrl+Alt+P) 的滑鼠版。
      </p>

      <section className="mori-profiles-section">
        <h3>🎙 VoiceInput Profiles ({voice.length})</h3>
        <div className="mori-profiles-grid">
          {voice.map((p) => (
            <button
              key={p.stem}
              className="mori-profile-card"
              onClick={() => switchVoice(p.stem)}
              title={`切到 ${p.display}`}
            >
              <span className="mori-profile-card-name">{p.display}</span>
              <span className="mori-profile-card-stem">{p.stem}</span>
            </button>
          ))}
          {voice.length === 0 && <div className="mori-profiles-empty">(目錄沒有 USER-*.md)</div>}
        </div>
      </section>

      <section className="mori-profiles-section">
        <h3>🌳 Agent Profiles ({agent.length})</h3>
        <div className="mori-profiles-grid">
          {agent.map((p) => (
            <button
              key={p.stem}
              className="mori-profile-card"
              onClick={() => switchAgent(p.stem)}
              title={`切到 ${p.display}`}
            >
              <span className="mori-profile-card-name">{p.display}</span>
              <span className="mori-profile-card-stem">{p.stem}</span>
            </button>
          ))}
          {agent.length === 0 && <div className="mori-profiles-empty">(目錄沒有 AGENT*.md)</div>}
        </div>
      </section>
    </div>
  );
}

export default ProfilesTab;
