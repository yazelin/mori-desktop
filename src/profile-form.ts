// 5L-3: profile frontmatter parse / serialize helpers。
// 用 js-yaml round-trip,frontend 編完寫回 .md;Rust 端 parse_profile / parse_agent_profile
// 在 IPC write 時再驗一次(保險)。

import yaml from "js-yaml";

export type AnyObj = Record<string, any>;

/** 把 .md 拆成 frontmatter object + body string */
export function splitFrontmatter(content: string): { frontmatter: AnyObj; body: string } {
  const trimmed = content.trimStart();
  if (!trimmed.startsWith("---")) {
    return { frontmatter: {}, body: content.trim() };
  }
  const after = trimmed.slice(3).replace(/^[\r\n]+/, "");
  const closeIdx = after.indexOf("\n---");
  if (closeIdx < 0) {
    return { frontmatter: {}, body: content.trim() };
  }
  const fmText = after.slice(0, closeIdx);
  const bodyRaw = after.slice(closeIdx + 4).replace(/^[\r\n]+/, "");
  let frontmatter: AnyObj = {};
  try {
    const parsed = yaml.load(fmText);
    if (parsed && typeof parsed === "object") frontmatter = parsed as AnyObj;
  } catch (e) {
    // YAML 壞了:回空 frontmatter,讓使用者進 raw editor 修
    console.warn("[profile] frontmatter YAML parse error", e);
  }
  return { frontmatter, body: bodyRaw.trim() };
}

/** 把 frontmatter + body 序列化回 .md 內容 */
export function buildProfileText(frontmatter: AnyObj, body: string): string {
  // 過濾空值,避免 frontmatter 滿是 null 看起來髒
  const cleaned: AnyObj = {};
  for (const [k, v] of Object.entries(frontmatter)) {
    if (v == null) continue;
    if (typeof v === "string" && v === "") continue;
    if (Array.isArray(v) && v.length === 0) continue;
    cleaned[k] = v;
  }
  if (Object.keys(cleaned).length === 0) {
    return body.trim() + "\n";
  }
  const fmStr = yaml.dump(cleaned, {
    indent: 2,
    lineWidth: 120,
    noRefs: true,
    sortKeys: false,
  });
  // js-yaml 預設會在最後加 \n
  return `---\n${fmStr}---\n${body.trim()}\n`;
}
