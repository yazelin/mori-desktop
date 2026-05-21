// Stage `target/<profile>/mori[.exe]` 成 Tauri externalBin 認得的
// `target/release/mori-<host-triple>[.exe]`,讓 `tauri build` 能把 mori CLI
// 連同 Mori.exe 一起塞進 .msi / .nsis / .deb bundle。
//
// ## 為什麼 rename 又為什麼固定寫到 release/
//
// 1. Tauri 2 `bundle.externalBin` 要求 binary 名 `<name>-<target-triple>[.exe]`
//    (一份 config 跨平台用),build 完 installer 會自動把 suffix 拿掉,user
//    看到的就是純 `mori.exe`。
//
// 2. Tauri 的 build script 在 **每次** `cargo check`/`build`(不只 bundle 步)
//    都會驗 externalBin 指向的檔存在,且路徑寫死不分 debug/release profile。
//    所以 dev 期間就算還沒跑 release build,也要有一份 `target/release/mori-
//    <triple>.exe`,不然 mori-tauri 連 check 都過不了。
//
//    對策:不管 npm 觸發的是 predev 還是 prebuild,**一律 stage 到
//    `target/release/`**。Debug profile 跑 dev 時 stage 的是 debug 編出來的
//    mori.exe(暫充人手,反正 tauri dev 不會真的 bundle),release build
//    跑 prebuild 時 stage 的才是真正會進 installer 的 release binary。
//
// 跑時機:`prebuild` / `predev` hook,`cargo build -p mori-cli` 之後緊接著跑。
// 找不到 mori-cli output → exit 1(prebuild 整鏈失敗,避免 silent miss)。

import { existsSync, copyFileSync, mkdirSync } from "node:fs";
import { execSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const profile = process.argv[2] === "release" ? "release" : "debug";

// 從 `rustc -vV` 拿 host triple — Node 自家的 process.arch / process.platform
// 配不出 `x86_64-pc-windows-msvc` 這種 Rust 慣用 triple,直接問 rustc 最穩。
function rustHostTriple() {
  const out = execSync("rustc -vV", { encoding: "utf8" });
  for (const line of out.split(/\r?\n/)) {
    const m = line.match(/^host:\s*(\S+)/);
    if (m) return m[1];
  }
  throw new Error("rustc -vV 沒列出 host: line — rustc 壞了?");
}

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");
const triple = rustHostTriple();
const isWin = process.platform === "win32";
const ext = isWin ? ".exe" : "";

const src = join(repoRoot, "target", profile, `mori${ext}`);
// Always stage to release/ — tauri.conf.json externalBin 寫死指向那。
const dst = join(repoRoot, "target", "release", `mori-${triple}${ext}`);

if (!existsSync(src)) {
  console.error(`stage-mori-cli: source missing: ${src}`);
  console.error(`stage-mori-cli: 預期 \`cargo build -p mori-cli${profile === "release" ? " --release" : ""}\` 已經跑過。`);
  process.exit(1);
}

mkdirSync(dirname(dst), { recursive: true });
copyFileSync(src, dst);
console.log(`stage-mori-cli: ${src} → ${dst}`);
