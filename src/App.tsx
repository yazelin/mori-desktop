import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [coreVersion, setCoreVersion] = useState<string>("");
  const [phase, setPhase] = useState<string>("");

  useEffect(() => {
    invoke<string>("mori_version").then(setCoreVersion).catch(() => setCoreVersion("(unavailable)"));
    invoke<string>("mori_phase").then(setPhase).catch(() => setPhase("(unavailable)"));
  }, []);

  return (
    <main className="container">
      <header>
        <h1>Mori</h1>
        <p className="subtitle">森林精靈 Mori 的桌面身體</p>
      </header>

      <section className="status">
        <div className="status-row">
          <span className="label">core</span>
          <span className="value">{coreVersion || "loading..."}</span>
        </div>
        <div className="status-row">
          <span className="label">phase</span>
          <span className="value">{phase || "loading..."}</span>
        </div>
      </section>

      <section className="next-steps">
        <p>
          Phase 1 scaffold ready. Voice features land in the next PR — see{" "}
          <code>docs/roadmap.md</code>.
        </p>
      </section>
    </main>
  );
}

export default App;
