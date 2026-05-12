// 5L-3: profile list + edit modal(ProfileEditor 拆到 ProfileEditor.tsx,
// 內含 frontmatter typed form + shell_skills 表格 + raw 切換)。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ProfileEditor } from "../ProfileEditor";

type ProfileEntry = { stem: string; display: string };
type Kind = "voice" | "agent";

function NewProfileButton({
  kind,
  onCreated,
}: {
  kind: Kind;
  onCreated: () => void;
}) {
  const create = async () => {
    const name = prompt(
      kind === "voice"
        ? "新 voice profile 檔名(例 USER-07.工作筆記),不含 .md"
        : "新 agent profile 檔名(例 AGENT-04.我的助理),不含 .md"
    );
    if (!name) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    if (!/^[A-Za-z0-9._\- ()一-鿿]+$/.test(trimmed)) {
      alert("檔名只接受字母 / 數字 / 中文 / . _ - 空格 + 括號");
      return;
    }
    const starter = kind === "voice"
      ? `---\nprovider: groq\nstt_provider: groq\nenable_read: true\n---\n你是 voice input 助理。請描述這個 profile 的行為...\n\n## 共用 STT 校正\n\n#file:~/.mori/corrections.md\n`
      : `---\nprovider: claude-bash\nenable_read: true\n---\n你是 Mori。請描述這個 agent 的角色...\n\n## 共用 STT 校正\n\n#file:~/.mori/corrections.md\n`;
    try {
      await invoke("profile_write", { kind, stem: trimmed, text: starter });
      onCreated();
    } catch (e: any) {
      alert(`建立失敗:${e}`);
    }
  };
  return (
    <button className="mori-btn" onClick={create}>
      + 新增
    </button>
  );
}

function ProfilesTab() {
  const [voice, setVoice] = useState<ProfileEntry[]>([]);
  const [agent, setAgent] = useState<ProfileEntry[]>([]);
  const [editing, setEditing] = useState<{ kind: Kind; stem: string } | null>(null);

  const reload = async () => {
    try {
      const [v, a] = await Promise.all([
        invoke<ProfileEntry[]>("picker_list_voice_profiles"),
        invoke<ProfileEntry[]>("picker_list_agent_profiles"),
      ]);
      setVoice(v);
      setAgent(a);
    } catch (e) { console.error(e); }
  };

  useEffect(() => { reload(); }, []);

  const switchVoice = (stem: string) =>
    invoke("picker_switch_voice_profile", { stem }).catch(console.error);
  const switchAgent = (stem: string) =>
    invoke("picker_switch_agent_profile", { stem }).catch(console.error);

  return (
    <div className="mori-tab mori-tab-profiles">
      <h2 className="mori-tab-title">Profiles</h2>
      <p className="mori-tab-hint">
        點「切換」即時生效;點「編輯」開啟 .md 編輯器,儲存後下一次熱鍵
        即時讀新內容(不需要重啟)。Profile body 可用 <code>#file:</code> 引用其他檔。
      </p>

      <section className="mori-profiles-section">
        <div className="mori-profiles-section-head">
          <h3>🎙 VoiceInput Profiles ({voice.length})</h3>
          <NewProfileButton kind="voice" onCreated={reload} />
        </div>
        <div className="mori-profiles-list">
          {voice.map((p) => (
            <div key={p.stem} className="mori-profile-row">
              <div className="mori-profile-row-info">
                <span className="mori-profile-row-name">{p.display}</span>
                <span className="mori-profile-row-stem">{p.stem}</span>
              </div>
              <div className="mori-profile-row-actions">
                <button className="mori-btn small" onClick={() => switchVoice(p.stem)}>切換</button>
                <button
                  className="mori-btn small primary"
                  onClick={() => setEditing({ kind: "voice", stem: p.stem })}
                >
                  編輯
                </button>
              </div>
            </div>
          ))}
          {voice.length === 0 && <div className="mori-profiles-empty">(目錄沒有 USER-*.md)</div>}
        </div>
      </section>

      <section className="mori-profiles-section">
        <div className="mori-profiles-section-head">
          <h3>🌳 Agent Profiles ({agent.length})</h3>
          <NewProfileButton kind="agent" onCreated={reload} />
        </div>
        <div className="mori-profiles-list">
          {agent.map((p) => (
            <div key={p.stem} className="mori-profile-row">
              <div className="mori-profile-row-info">
                <span className="mori-profile-row-name">{p.display}</span>
                <span className="mori-profile-row-stem">{p.stem}</span>
              </div>
              <div className="mori-profile-row-actions">
                <button className="mori-btn small" onClick={() => switchAgent(p.stem)}>切換</button>
                <button
                  className="mori-btn small primary"
                  onClick={() => setEditing({ kind: "agent", stem: p.stem })}
                >
                  編輯
                </button>
              </div>
            </div>
          ))}
          {agent.length === 0 && <div className="mori-profiles-empty">(目錄沒有 AGENT*.md)</div>}
        </div>
      </section>

      {editing && (
        <ProfileEditor
          kind={editing.kind}
          stem={editing.stem}
          onClose={() => setEditing(null)}
          onSaved={reload}
        />
      )}
    </div>
  );
}

export default ProfilesTab;
