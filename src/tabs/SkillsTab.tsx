// 5L-4: Skills inspector。
// 列當前 active agent profile 啟用的全部 skills(built-in + shell_skills),
// 每個顯示 name / description / kind / parameters schema。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type SkillInfo = {
  name: string;
  description: string;
  parameters: any;
  kind: "builtin" | "shell";
};

function SkillCard({ skill }: { skill: SkillInfo }) {
  const [expanded, setExpanded] = useState(false);
  const props = skill.parameters?.properties ?? {};
  const required: string[] = Array.isArray(skill.parameters?.required) ? skill.parameters.required : [];
  const paramKeys = Object.keys(props);

  return (
    <div className={`mori-skill-card ${expanded ? "expanded" : ""}`}>
      <div className="mori-skill-card-head" onClick={() => setExpanded(!expanded)}>
        <span className={`mori-skill-kind kind-${skill.kind}`}>
          {skill.kind === "shell" ? "🛠 shell" : "⚙ built-in"}
        </span>
        <span className="mori-skill-name">{skill.name}</span>
        <span className="mori-skill-param-count">
          {paramKeys.length > 0 ? `${paramKeys.length} param${paramKeys.length > 1 ? "s" : ""}` : "no params"}
        </span>
        <span className="mori-skill-toggle">{expanded ? "▾" : "▸"}</span>
      </div>
      <div className="mori-skill-card-desc">
        {skill.description}
      </div>
      {expanded && (
        <div className="mori-skill-card-body">
          {paramKeys.length === 0 ? (
            <div className="mori-shell-skills-empty small">沒有參數</div>
          ) : (
            <table className="mori-skill-params">
              <thead>
                <tr>
                  <th>name</th>
                  <th>type</th>
                  <th>required</th>
                  <th>description</th>
                </tr>
              </thead>
              <tbody>
                {paramKeys.map((k) => {
                  const p: any = props[k];
                  return (
                    <tr key={k}>
                      <td className="param-name">{k}</td>
                      <td className="param-type">{p.type ?? "?"}</td>
                      <td className="param-required">{required.includes(k) ? "✓" : ""}</td>
                      <td className="param-desc">{p.description ?? <em>(no description)</em>}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}

function SkillsTab() {
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<"all" | "builtin" | "shell">("all");

  const reload = async () => {
    setLoading(true);
    try {
      const list = await invoke<SkillInfo[]>("skills_list");
      setSkills(list);
      setError(null);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { reload(); }, []);

  const filtered = skills.filter((s) => filter === "all" || s.kind === filter);
  const counts = {
    builtin: skills.filter((s) => s.kind === "builtin").length,
    shell: skills.filter((s) => s.kind === "shell").length,
  };

  return (
    <div className="mori-tab mori-tab-skills">
      <h2 className="mori-tab-title">Skills</h2>
      <p className="mori-tab-hint">
        當前 active agent profile 啟用的 skills。Built-in(translate / polish /
        記憶 skill / 動作 skill 等)+ shell_skill(profile frontmatter
        定義的自訂 CLI 包裝)。改 profile 後按 🔄 重新讀取。
      </p>

      {error && <div className="mori-config-error">{error}</div>}

      <div className="mori-skill-filter">
        <button
          className={`mori-view-tab ${filter === "all" ? "active" : ""}`}
          onClick={() => setFilter("all")}
        >全部 ({skills.length})</button>
        <button
          className={`mori-view-tab ${filter === "builtin" ? "active" : ""}`}
          onClick={() => setFilter("builtin")}
        >Built-in ({counts.builtin})</button>
        <button
          className={`mori-view-tab ${filter === "shell" ? "active" : ""}`}
          onClick={() => setFilter("shell")}
        >Shell ({counts.shell})</button>
        <button className="mori-btn" onClick={reload}>🔄</button>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>讀取中…</p></div>
      ) : filtered.length === 0 ? (
        <div className="mori-tab-placeholder"><p>沒有 skill 符合篩選</p></div>
      ) : (
        <div className="mori-skill-list">
          {filtered.map((s) => <SkillCard key={s.name} skill={s} />)}
        </div>
      )}
    </div>
  );
}

export default SkillsTab;
