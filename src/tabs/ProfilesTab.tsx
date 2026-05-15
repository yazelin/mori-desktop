// 5L-3: profile list + edit modal(ProfileEditor 拆到 ProfileEditor.tsx,
// 內含 frontmatter typed form + shell_skills 表格 + raw 切換)。
// v0.4.1:加「+ 範本」按鈕 + StarterTemplateModal — 從 binary 內建的 zh/en
// starter 範本撈一份寫進 ~/.mori/,用於還原 / 加裝另一語系版本。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ProfileEditor } from "../ProfileEditor";
import { IconVoiceMic, IconTree, IconClose } from "../icons";

type ProfileEntry = { stem: string; display: string };
type Kind = "voice" | "agent";
type StarterTemplate = { filename: string; lang: "zh" | "en"; display: string };
type TokenEstimate = { gpt_oss: number; gemini: number };

function OpenFolderButton({ kind }: { kind: Kind }) {
  const { t } = useTranslation();
  const open = () =>
    invoke("open_profile_dir", { kind }).catch((e: any) =>
      alert(`${t("profiles_tab.open_folder_button")} 失敗：${e}`),
    );
  return (
    <button className="mori-btn" onClick={open}>
      {t("profiles_tab.open_folder_button")}
    </button>
  );
}

function AddTemplateButton({
  kind,
  onAdded,
}: {
  kind: Kind;
  onAdded: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <button className="mori-btn" onClick={() => setOpen(true)}>
        {t("profiles_tab.add_template_button")}
      </button>
      {open && (
        <StarterTemplateModal
          kind={kind}
          onClose={() => setOpen(false)}
          onInstalled={() => {
            onAdded();
            setOpen(false);
          }}
        />
      )}
    </>
  );
}

function StarterTemplateModal({
  kind,
  onClose,
  onInstalled,
}: {
  kind: Kind;
  onClose: () => void;
  onInstalled: () => void;
}) {
  const { t } = useTranslation();
  const [templates, setTemplates] = useState<StarterTemplate[] | null>(null);
  const [langFilter, setLangFilter] = useState<"all" | "zh" | "en">("all");
  const [installing, setInstalling] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<StarterTemplate[]>("list_starter_templates", { kind })
      .then(setTemplates)
      .catch((e) => setError(String(e)));
  }, [kind]);

  const install = async (filename: string, overwrite = false) => {
    setInstalling(filename);
    setError(null);
    try {
      await invoke("install_starter_template", { kind, filename, overwrite });
      onInstalled();
    } catch (e: any) {
      const msg = String(e);
      if (msg.includes("already exists") && !overwrite) {
        // 已存在 → 問 user 要不要覆蓋
        if (confirm(t("profiles_tab.template_overwrite_confirm", { filename }))) {
          await install(filename, true);
          return;
        }
        setError(null);
      } else {
        setError(msg);
      }
    } finally {
      setInstalling(null);
    }
  };

  const filtered = (templates ?? []).filter((tpl) =>
    langFilter === "all" ? true : tpl.lang === langFilter,
  );

  return (
    <div className="mori-modal-backdrop" onClick={onClose}>
      <div className="mori-modal mori-template-modal" onClick={(e) => e.stopPropagation()}>
        <div className="mori-modal-header">
          <div className="mori-modal-title">
            <span className="mori-modal-kind">
              {kind === "voice" ? <IconVoiceMic width={12} height={12} /> : <IconTree width={12} height={12} />}
              {" "}{t("profiles_tab.add_template_button")}
            </span>
            <span className="mori-modal-stem">
              {kind === "voice" ? t("profiles_tab.voice_section") : t("profiles_tab.agent_section")}
            </span>
          </div>
          <button className="mori-btn ghost" onClick={onClose}>
            <IconClose width={14} height={14} />
          </button>
        </div>
        <div className="mori-modal-body">
          <p className="mori-tab-hint">{t("profiles_tab.add_template_hint")}</p>
          <div className="mori-template-filter">
            {(["all", "zh", "en"] as const).map((lang) => (
              <button
                key={lang}
                className={`mori-btn small ${langFilter === lang ? "primary" : ""}`}
                onClick={() => setLangFilter(lang)}
              >
                {t(`profiles_tab.template_lang_${lang}`)}
              </button>
            ))}
          </div>
          {error && <div className="mori-config-error">{error}</div>}
          {templates === null ? (
            <div className="mori-modal-loading">{t("profiles_tab.template_loading")}</div>
          ) : filtered.length === 0 ? (
            <div className="mori-profiles-empty">{t("profiles_tab.template_no_match")}</div>
          ) : (
            <div className="mori-template-list">
              {filtered.map((tpl) => (
                <div key={tpl.filename} className="mori-template-row">
                  <div className="mori-template-row-info">
                    <span className="mori-template-row-display">{tpl.display}</span>
                    <span className={`mori-template-row-lang lang-${tpl.lang}`}>{tpl.lang}</span>
                    <span className="mori-template-row-filename">{tpl.filename}</span>
                  </div>
                  <button
                    className="mori-btn small primary"
                    disabled={installing === tpl.filename}
                    onClick={() => install(tpl.filename)}
                  >
                    {installing === tpl.filename
                      ? t("profiles_tab.template_installing")
                      : t("profiles_tab.template_install_button")}
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

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

/** v0.4.3:profile system prompt token 估算 chip。
 *  數字格式 `~512 / ~440` — 前 gpt-oss(o200k_harmony,Mori 預設 groq provider
 *  跑的)/ 後 Gemini Flash。啟發法,±10% 準確度。title 解釋 disclaimer。
 *  沒拿到估算(profile 不在磁碟 / 後端錯)就不渲染,不卡 row。 */
function TokenBadge({ est }: { est: TokenEstimate | undefined }) {
  const { t } = useTranslation();
  if (!est) return null;
  return (
    <span
      className="mori-profile-row-tokens"
      title={t("profiles_tab.token_estimate_tooltip", {
        gpt_oss: est.gpt_oss,
        gemini: est.gemini,
      })}
    >
      ~{est.gpt_oss}/{est.gemini} tok
    </span>
  );
}

function ProfilesTab() {
  const { t } = useTranslation();
  const [voice, setVoice] = useState<ProfileEntry[]>([]);
  const [agent, setAgent] = useState<ProfileEntry[]>([]);
  const [editing, setEditing] = useState<{ kind: Kind; stem: string } | null>(null);
  // v0.4.3:每筆 profile 的 token 估算(gpt-oss / Gemini)— 啟發法,±10%
  const [voiceTokens, setVoiceTokens] = useState<Record<string, TokenEstimate>>({});
  const [agentTokens, setAgentTokens] = useState<Record<string, TokenEstimate>>({});

  const fetchTokens = async (kind: Kind, entries: ProfileEntry[]) => {
    const results = await Promise.all(
      entries.map(async (p) => {
        try {
          const est = await invoke<TokenEstimate>("estimate_profile_tokens", { kind, stem: p.stem });
          return [p.stem, est] as const;
        } catch {
          return null;
        }
      }),
    );
    const map: Record<string, TokenEstimate> = {};
    for (const r of results) if (r) map[r[0]] = r[1];
    return map;
  };

  const reload = async () => {
    try {
      const [v, a] = await Promise.all([
        invoke<ProfileEntry[]>("picker_list_voice_profiles"),
        invoke<ProfileEntry[]>("picker_list_agent_profiles"),
      ]);
      setVoice(v);
      setAgent(a);
      // token 估算並行抓,reload 後立即顯示;個別失敗不擋整列表 render
      void fetchTokens("voice", v).then(setVoiceTokens);
      void fetchTokens("agent", a).then(setAgentTokens);
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
          <div className="mori-profiles-section-actions">
            <OpenFolderButton kind="voice" />
            <AddTemplateButton kind="voice" onAdded={reload} />
            <NewProfileButton kind="voice" onCreated={reload} />
          </div>
        </div>
        <div className="mori-profiles-list">
          {voice.map((p) => (
            <div key={p.stem} className="mori-profile-row">
              <div className="mori-profile-row-info">
                <div className="mori-profile-row-line1">
                  <span className="mori-profile-row-name">{p.display}</span>
                  <TokenBadge est={voiceTokens[p.stem]} />
                </div>
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
          <div className="mori-profiles-section-actions">
            <OpenFolderButton kind="agent" />
            <AddTemplateButton kind="agent" onAdded={reload} />
            <NewProfileButton kind="agent" onCreated={reload} />
          </div>
        </div>
        <div className="mori-profiles-list">
          {agent.map((p) => (
            <div key={p.stem} className="mori-profile-row">
              <div className="mori-profile-row-info">
                <div className="mori-profile-row-line1">
                  <span className="mori-profile-row-name">{p.display}</span>
                  <TokenBadge est={agentTokens[p.stem]} />
                </div>
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
