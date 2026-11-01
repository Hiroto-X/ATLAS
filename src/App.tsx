import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SystemInfoSchema, type SystemInfo } from "./contracts";

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  try {
    return JSON.stringify(error);
  } catch {
    return "An unexpected error occurred.";
  }
}

export default function App() {
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function loadSystemInfo() {
    setLoading(true);
    setError(null);
    setSystemInfo(null);

    try {
      const result: unknown = await invoke("get_system_info");
      const validatedResult = SystemInfoSchema.parse(result);

      setSystemInfo(validatedResult);
    } catch (err: unknown) {
      setError(getErrorMessage(err));
    } finally {
      setLoading(false);
    }
  }

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark">A</span><span>ATLAS</span></div>
        <div className="nav-label">WORKSPACE</div>
        <button className="nav-item active">◈ <span>Universe</span></button>
        <button className="nav-item" disabled>⌕ <span>Discover</span></button>
        <button className="nav-item" disabled>◷ <span>Activity</span></button>
        <button className="nav-item" disabled>✧ <span>Intelligence</span></button>
        <div className="sidebar-bottom">
          <div className="connection"><span className="status-dot" /> Runtime connected</div>
          <div className="version">EARLY FOUNDATION BUILD · 0.1.0</div>
        </div>
      </aside>

      <section className="main-area">
        <header className="topbar">
          <div><span className="breadcrumb-muted">Workspace</span><span className="crumb-sep">/</span><strong>Universe</strong></div>
          <div className="topbar-right"><span className="local-badge">LOCAL ENVIRONMENT</span></div>
        </header>

        <div className="content">
          <div className="eyebrow">YOUR DIGITAL ENVIRONMENT</div>
          <h1>Explore your <span>digital universe.</span></h1>
          <p className="intro">ATLAS is ready to establish its first connection with this computer.</p>

          <section className="panel system-panel">
            <div className="panel-heading">
              <div>
                <div className="eyebrow">PHASE 1 · FOUNDATION</div>
                <h2>System connection</h2>
              </div>
              <span className="read-only">READ ONLY</span>
            </div>
            <p className="panel-copy">Request basic system information from the Rust runtime through Tauri IPC.</p>
            <button className="primary-button" onClick={loadSystemInfo} disabled={loading}>
              {loading ? "Connecting…" : "Connect to this system"} <span>→</span>
            </button>

            {error && <div className="error-box" role="alert">{error}</div>}
            {systemInfo && (
              <div className="info-grid">
                <Info label="Operating system" value={systemInfo.os} />
                <Info label="Architecture" value={systemInfo.architecture} />
                <Info label="Device name" value={systemInfo.hostname} />
                <Info label="ATLAS version" value={systemInfo.appVersion} />
              </div>
            )}
          </section>

          <div className="foundation-note">
            <span className="note-icon">i</span>
            <p><strong>Foundation build.</strong> This is a deliberately small vertical slice. No background monitoring, filesystem indexing, or system modifications are active.</p>
          </div>
        </div>
      </section>
    </main>
  );
}

function Info({ label, value }: { label: string; value: string }) {
  return <div className="info-cell"><span>{label}</span><strong>{value || "Unavailable"}</strong></div>;
}
