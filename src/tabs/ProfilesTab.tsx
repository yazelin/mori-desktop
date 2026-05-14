// 5L-3: profile list + edit modal(ProfileEditor 拆到 ProfileEditor.tsx,
// 內含 frontmatter typed form + shell_skills 表格 + raw 切換)。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ProfileEditor } from "../ProfileEditor";
import { IconVoiceMic, IconTree } from "../icons";

type ProfileEntry = { stem: string; display: string };
type Kind = "voice" | "agent";

function NewProfileButton({
  kind,
  onCreated,
}: {
  kind: Kind;
  onCreated: () => void;
}) {
  const { t } = useTranslation();
  const create = async () => {
    const name = prompt(
      kind === "voice"
        ? t("profiles_tab.new_voice_prompt")
        : t("profiles_tab.new_agent_prompt"),
    );
    if (!name) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    if (!/^[A-Za-z0-9._\- ()一-鿿]+$/.test(trimmed)) {
      alert(t("profiles_tab.invalid_name"));
      return;
    }
    const starter = kind === "voice"
      ? `---\nprovider: groq\nstt_provider: groq\nenable_read: true\n---\n你是 voice input 助理。請描述這個 profile 的行為...\n\n## 共用 STT 校正\n\n#file:~/.mori/corrections.md\n`
      : `---\nprovider: claude-bash\nenable_read: true\n---\n你是 Mori。請描述這個 agent 的角色...\n\n## 共用 STT 校正\n\n#file:~/.mori/corrections.md\n`;
    try {
      await invoke("profile_write", { kind, stem: trimmed, text: starter });
      onCreated();
    } catch (e: any) {
      alert(`${t("profiles_tab.create_failed")}${e}`);
    }
  };
  return (
    <button className="mori-btn" onClick={create}>
      {t("profiles_tab.new_button")}
    </button>
  );
}

function ProfilesTab() {
  const { t } = useTranslation();
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
      <h2 className="mori-tab-title">{t("profiles_tab.title")}</h2>
      <p className="mori-tab-hint">{t("profiles_tab.hint")}</p>

      <section className="mori-profiles-section">
        <div className="mori-profiles-section-head">
          <h3><IconVoiceMic width={14} height={14} /> {t("profiles_tab.voice_section")} ({voice.length})</h3>
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
                <button className="mori-btn small" onClick={() => switchVoice(p.stem)}>{t("profiles_tab.switch_button")}</button>
                <button
                  className="mori-btn small primary"
                  onClick={() => setEditing({ kind: "voice", stem: p.stem })}
                >
                  {t("profiles_tab.edit_button")}
                </button>
              </div>
            </div>
          ))}
          {voice.length === 0 && <div className="mori-profiles-empty">{t("profiles_tab.empty_voice")}</div>}
        </div>
      </section>

      <section className="mori-profiles-section">
        <div className="mori-profiles-section-head">
          <h3><IconTree width={14} height={14} /> {t("profiles_tab.agent_section")} ({agent.length})</h3>
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
                <button className="mori-btn small" onClick={() => switchAgent(p.stem)}>{t("profiles_tab.switch_button")}</button>
                <button
                  className="mori-btn small primary"
                  onClick={() => setEditing({ kind: "agent", stem: p.stem })}
                >
                  {t("profiles_tab.edit_button")}
                </button>
              </div>
            </div>
          ))}
          {agent.length === 0 && <div className="mori-profiles-empty">{t("profiles_tab.empty_agent")}</div>}
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
