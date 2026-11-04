import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { api, setBaseUrl, type Model } from "./api";
import ModelHub from "./pages/ModelHub";
import VoiceLibrary from "./pages/VoiceLibrary";
import VoiceStudio from "./pages/VoiceStudio";

type Page = "hub" | "voices" | "studio";

const TABS: { id: Page; label: string }[] = [
  { id: "hub", label: "TTS 模型中心" },
  { id: "voices", label: "声音库" },
  { id: "studio", label: "声音设计工作台" },
];

/** `true` when the frontend runs inside the Tauri webview. */
function inTauri() {
  return "__TAURI_INTERNALS__" in window;
}

export default function App() {
  const [page, setPage] = useState<Page>("hub");
  const [base, setBase] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [models, setModels] = useState<Model[]>([]);
  const [manualUrl, setManualUrl] = useState("http://127.0.0.1:8080");

  /** Starts the backend and waits until it answers. */
  const start = useCallback(async () => {
    setStarting(true);
    setError(null);
    try {
      let url: string;
      if (inTauri()) {
        url = await invoke<string>("start_server");
      } else {
        url = manualUrl;
      }
      setBaseUrl(url);

      // Wait for the server to accept requests.
      const deadline = Date.now() + 30_000;
      for (;;) {
        try {
          await api.health();
          break;
        } catch (err) {
          if (Date.now() > deadline) throw err;
          await new Promise((resolve) => window.setTimeout(resolve, 250));
        }
      }

      setBase(url);
      setModels(await api.models());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setStarting(false);
    }
  }, [manualUrl]);

  useEffect(() => {
    void start();
  }, [start]);

  const refreshModels = useCallback(async () => {
    if (!base) return;
    try {
      setModels(await api.models());
    } catch {
      // the status bar shows the error
    }
  }, [base]);

  return (
    <div className="app">
      <nav className="sidebar">
        <div className="brand">
          <div className="brand-title">本地 AI 工作台</div>
          <div className="muted small">TTS 子系统 · Milestone 1</div>
        </div>
        {TABS.map((tab) => (
          <button
            key={tab.id}
            className={`nav-item ${page === tab.id ? "active" : ""}`}
            onClick={() => setPage(tab.id)}
          >
            {tab.label}
          </button>
        ))}
        <div className="sidebar-footer">
          {base ? (
            <>
              <div className="badge ok">服务运行中</div>
              <div className="muted small">{base}</div>
            </>
          ) : (
            <div className="badge bad">服务未连接</div>
          )}
          <button onClick={() => void start()} disabled={starting}>
            {starting ? "启动中…" : "重新连接"}
          </button>
        </div>
      </nav>

      <main className="content">
        {error && (
          <div className="alert error">
            {error}
            {!inTauri() && (
              <div className="field">
                <input
                  value={manualUrl}
                  onChange={(event) => setManualUrl(event.target.value)}
                  placeholder="http://127.0.0.1:8080"
                />
                <button onClick={() => void start()}>连接</button>
              </div>
            )}
          </div>
        )}

        {page === "hub" && <ModelHub />}
        {page === "voices" && (
          <VoiceLibrary models={models} />
        )}
        {page === "studio" && (
          <VoiceStudio models={models} onSaved={() => void refreshModels()} />
        )}
      </main>
    </div>
  );
}
