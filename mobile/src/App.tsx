// Copyright (c) 2026 Dreamsequence Ltd
// SPDX-License-Identifier: MIT
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface HarnessConfig {
  name: string;
  log_path: string;
  log_format: string;
}

interface MobileConfig {
  harnesses: HarnessConfig[];
  output_dir: string;
  anthologies_dir: string;
  allow_remote_analysis: boolean;
  auto_approve_remote_analysis: boolean;
}

interface AnthologySummary {
  id: string;
  date: string;
  generated_at: string;
  executive_summary: string;
  candidate_tools_count: number;
}

interface Anthology extends AnthologySummary {
  significant_milestones: string[];
  candidate_tools: CandidateTool[];
  steering_events: SteeringEvent[];
}

interface CandidateTool {
  id: string;
  name: string;
  priority: "High" | "Medium" | "Low";
  reason: string;
  estimated_time_saved: string;
  confidence: number;
}

interface SteeringEvent {
  category: string;
  description: string;
  severity: number;
}

interface AccountInfo {
  account_id: string;
  device_id: string;
  api_url: string;
}

interface DeviceAuthorization {
  device_code: string;
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string;
  expires_in: number;
  interval: number;
}

interface SyncSummary {
  uploaded: number;
  skipped: number;
  failed: number;
}

type Screen = "home" | "sources" | "settings";

function App() {
  const [screen, setScreen] = useState<Screen>("home");
  const [config, setConfig] = useState<MobileConfig | null>(null);
  const [anthologies, setAnthologies] = useState<AnthologySummary[]>([]);
  const [selectedAnthology, setSelectedAnthology] = useState<Anthology | null>(null);
  const [account, setAccount] = useState<AccountInfo | null>(null);
  const [auth, setAuth] = useState<DeviceAuthorization | null>(null);
  const [running, setRunning] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadAll = async () => {
    try {
      const cfg = await invoke<MobileConfig>("load_config");
      setConfig(cfg);
      const list = await invoke<AnthologySummary[]>("list_anthologies");
      setAnthologies(list);
      const status = await invoke<AccountInfo | null>("cloud_status");
      setAccount(status);
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    loadAll();
  }, []);

  const showMessage = (text: string) => {
    setMessage(text);
    setTimeout(() => setMessage(null), 4000);
  };

  const handleRunAnalysis = async () => {
    setRunning(true);
    setError(null);
    try {
      const summary = await invoke<AnthologySummary>("run_analysis");
      setAnthologies((prev) => [summary, ...prev]);
      showMessage("Analysis complete");
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const handleLoadAnthology = async (id: string) => {
    try {
      const anthology = await invoke<Anthology | null>("load_anthology", { id });
      setSelectedAnthology(anthology);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDiscover = async () => {
    try {
      const harnesses = await invoke<HarnessConfig[]>("discover_harnesses");
      if (config) {
        const updated = { ...config, harnesses };
        await invoke("save_config", { config: updated });
        setConfig(updated);
        showMessage(`Discovered ${harnesses.length} sources`);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const handleToggleRemote = async () => {
    if (!config) return;
    const updated = { ...config, allow_remote_analysis: !config.allow_remote_analysis };
    try {
      const saved = await invoke<MobileConfig>("save_config", { config: updated });
      setConfig(saved);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleToggleAutoApprove = async () => {
    if (!config) return;
    const updated = {
      ...config,
      auto_approve_remote_analysis: !config.auto_approve_remote_analysis,
    };
    try {
      const saved = await invoke<MobileConfig>("save_config", { config: updated });
      setConfig(saved);
    } catch (e) {
      setError(String(e));
    }
  };

  const handlePair = async () => {
    setError(null);
    try {
      const authorization = await invoke<DeviceAuthorization>("request_authorization", {
        api_url: null,
      });
      setAuth(authorization);
      await invoke("open_url", { url: authorization.verification_uri_complete });
      const info = await invoke<AccountInfo>("complete_pairing", {
        api_url: null,
        device_code: authorization.device_code,
        expires_in: authorization.expires_in,
        interval: authorization.interval,
      });
      setAccount(info);
      setAuth(null);
      showMessage("Device paired");
    } catch (e) {
      setAuth(null);
      setError(String(e));
    }
  };

  const handleLogout = async () => {
    try {
      await invoke("logout");
      setAccount(null);
      showMessage("Logged out");
    } catch (e) {
      setError(String(e));
    }
  };

  const handleSync = async () => {
    setSyncing(true);
    try {
      const summary = await invoke<SyncSummary>("sync_with_cloud");
      showMessage(`Sync: ${summary.uploaded} uploaded, ${summary.failed} failed`);
      await loadAll();
    } catch (e) {
      setError(String(e));
    } finally {
      setSyncing(false);
    }
  };

  const renderHome = () => (
    <div className="screen">
      <h1>Dreamseq Mobile</h1>
      <div className="card">
        <button onClick={handleRunAnalysis} disabled={running} className="primary">
          {running ? "Analyzing…" : "Run on-device analysis"}
        </button>
        <p className="hint">
          Aggregates logs from configured mobile sources and runs the Dreamseq pipeline.
        </p>
      </div>
      <h2>Recent anthologies</h2>
      {anthologies.length === 0 && <p className="empty">No anthologies yet.</p>}
      <ul className="list">
        {anthologies.map((a) => (
          <li key={a.id} className="list-item" onClick={() => handleLoadAnthology(a.id)}>
            <div className="title">{a.date}</div>
            <div className="subtitle">{a.candidate_tools_count} candidate tools</div>
          </li>
        ))}
      </ul>
      {selectedAnthology && (
        <div className="modal" onClick={() => setSelectedAnthology(null)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <h2>{selectedAnthology.date}</h2>
            <p>{selectedAnthology.executive_summary}</p>
            <h3>Candidate tools</h3>
            <ul className="tool-list">
              {selectedAnthology.candidate_tools.map((tool) => (
                <li key={tool.id}>
                  <strong>{tool.name}</strong> ({tool.priority}) — {tool.reason}
                </li>
              ))}
            </ul>
            <button onClick={() => setSelectedAnthology(null)}>Close</button>
          </div>
        </div>
      )}
    </div>
  );

  const renderSources = () => (
    <div className="screen">
      <h1>Sources</h1>
      <button onClick={handleDiscover}>Discover mobile log folders</button>
      {config?.harnesses.length === 0 && <p className="empty">No sources configured.</p>}
      <ul className="list">
        {config?.harnesses.map((h) => (
          <li key={h.name} className="list-item">
            <div className="title">{h.name}</div>
            <div className="subtitle">{h.log_path}</div>
            <div className="subtitle">{h.log_format}</div>
          </li>
        ))}
      </ul>
      <p className="hint">
        Place logs under the app&apos;s Documents/Dreamseq/logs folder, organized by format.
      </p>
    </div>
  );

  const renderSettings = () => (
    <div className="screen">
      <h1>Settings</h1>
      <div className="card">
        <label className="toggle">
          <input
            type="checkbox"
            checked={config?.allow_remote_analysis ?? false}
            onChange={handleToggleRemote}
          />
          Allow remote analysis
        </label>
        <label className="toggle">
          <input
            type="checkbox"
            checked={config?.auto_approve_remote_analysis ?? false}
            onChange={handleToggleAutoApprove}
          />
          Auto-approve remote analysis
        </label>
        <p className="hint">
          When enabled, redacted log excerpts are sent to Dreamsequence or your configured BYOK
          endpoints for LLM analysis. Auto-approve skips the interactive confirmation.
        </p>
      </div>
      <div className="card">
        {account ? (
          <>
            <p>
              <strong>Paired:</strong> {account.account_id}
            </p>
            <p className="subtitle">{account.api_url}</p>
            <button onClick={handleLogout}>Logout</button>
            <button onClick={handleSync} disabled={syncing}>
              {syncing ? "Syncing…" : "Sync with cloud"}
            </button>
          </>
        ) : (
          <>
            <p>Not paired with Dreamsequence.</p>
            <button onClick={handlePair} disabled={!!auth}>
              {auth ? `Enter code ${auth.user_code}` : "Pair device"}
            </button>
          </>
        )}
      </div>
    </div>
  );

  return (
    <div className="app">
      <main>
        {screen === "home" && renderHome()}
        {screen === "sources" && renderSources()}
        {screen === "settings" && renderSettings()}
      </main>
      <nav className="tab-bar">
        <button className={screen === "home" ? "active" : ""} onClick={() => setScreen("home")}>
          Home
        </button>
        <button
          className={screen === "sources" ? "active" : ""}
          onClick={() => setScreen("sources")}
        >
          Sources
        </button>
        <button
          className={screen === "settings" ? "active" : ""}
          onClick={() => setScreen("settings")}
        >
          Settings
        </button>
      </nav>
      {message && <div className="toast">{message}</div>}
      {error && <div className="error">{error}</div>}
    </div>
  );
}

export default App;
