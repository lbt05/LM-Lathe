import { useCallback, useEffect, useState } from "react";
import {
  api,
  formatBytes,
  type InstallProgress,
  type Model,
  type Status,
} from "../api";

/**
 * TTS Model Hub (§9.1).
 *
 * Shows every model of the registry with its version, size, format, memory
 * estimate and running state, and offers the load, unload and pin actions.
 */
export default function ModelHub() {
  const [models, setModels] = useState<Model[]>([]);
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, InstallProgress>>({});
  const [logs, setLogs] = useState<{ model: string; text: string } | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [modelList, runtimeStatus] = await Promise.all([
        api.models(),
        api.status(),
      ]);
      setModels(modelList);
      setStatus(runtimeStatus);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 4000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const run = async (id: string, action: () => Promise<unknown>) => {
    setBusy(`${id}`);
    try {
      await action();
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
      await refresh();
    }
  };

  const install = async (model: Model) => {
    setBusy(model.id);
    try {
      await api.installModel(model.id);
      setProgress((current) => ({
        ...current,
        [model.id]: {
          model_id: model.id,
          state: "running",
          files_done: 0,
          files_total: 0,
          bytes: 0,
        },
      }));
      const timer = window.setInterval(async () => {
        try {
          const state = await api.installProgress(model.id);
          setProgress((current) => ({ ...current, [model.id]: state }));
          if (state.state !== "running") {
            window.clearInterval(timer);
            await refresh();
          }
        } catch {
          window.clearInterval(timer);
        }
      }, 1000);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  };

  const showLogs = async (model: Model) => {
    try {
      const text = await api.modelLogs(model.id);
      setLogs({ model: model.id, text });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const used = status?.scheduler.used_bytes ?? 0;
  const total = status?.scheduler.total_bytes ?? 0;

  return (
    <section className="page">
      <header className="page-header">
        <div>
          <h2>TTS 模型中心</h2>
          <p className="muted">
            全部模型由统一的 Runtime Manager 管理，与 LLM 共享同一个全局调度器。
          </p>
        </div>
        <div className="memory-meter">
          <div className="memory-meter-label">
            显存占用 {formatBytes(used)} / {formatBytes(total)}
          </div>
          <div className="memory-meter-bar">
            <span style={{ width: total ? `${(used / total) * 100}%` : "0%" }} />
          </div>
          <div className="muted small">
            活动模型：{status?.active_model ?? "无"}
          </div>
        </div>
      </header>

      {error && <div className="alert error">{error}</div>}

      <table className="grid">
        <thead>
          <tr>
            <th>模型</th>
            <th>版本</th>
            <th>格式</th>
            <th>大小</th>
            <th>预计内存</th>
            <th>已安装</th>
            <th>兼容性</th>
            <th>状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          {models.map((model) => {
            const state = progress[model.id];
            const running = model.state === "running";
            const compatible = model.compatible;
            return (
              <tr key={model.id}>
                <td>
                  <div className="cell-title">{model.name}</div>
                  <div className="muted small">{model.id}</div>
                  <div className="muted small">{model.hf_repo}</div>
                </td>
                <td>{model.version}</td>
                <td>{model.format}</td>
                <td>{formatBytes(model.size_bytes)}</td>
                <td>{formatBytes(model.estimated_memory_bytes)}</td>
                <td>
                  {model.installed ? (
                    <span className="badge ok">已安装</span>
                  ) : (
                    <span className="badge muted">未安装</span>
                  )}
                  {state && state.state === "running" && (
                    <div className="muted small">
                      下载中 {state.files_done}/{state.files_total}（
                      {formatBytes(state.bytes)}）
                    </div>
                  )}
                </td>
                <td>
                  <span
                    className={`badge ${
                      compatible ? "ok" : "bad"
                    }`}
                    title={model.notes.join("；")}
                  >
                    {model.compatibility}
                  </span>
                </td>
                <td>
                  <span
                    className={`badge ${
                      running ? "ok" : model.state === "failed" ? "bad" : "muted"
                    }`}
                  >
                    {model.state}
                  </span>
                  {model.pinned && <span className="badge pin">已固定</span>}
                </td>
                <td className="actions">
                  {running ? (
                    <button
                      disabled={busy === model.id}
                      onClick={() =>
                        run(model.id, () => api.unloadModel(model.id))
                      }
                    >
                      卸载
                    </button>
                  ) : (
                    <button
                      disabled={busy === model.id || !compatible}
                      title={
                        compatible
                          ? "加载模型"
                          : model.notes.join("；") || "本机无法运行该模型"
                      }
                      onClick={() => run(model.id, () => api.loadModel(model.id))}
                    >
                      加载
                    </button>
                  )}
                  <button
                    disabled={busy === model.id}
                    onClick={() =>
                      run(model.id, () =>
                        api.pinModel(model.id, !model.pinned),
                      )
                    }
                  >
                    {model.pinned ? "取消固定" : "固定"}
                  </button>
                  {!model.installed ? (
                    <button
                      disabled={busy === model.id || !compatible}
                      onClick={() => void install(model)}
                    >
                      下载
                    </button>
                  ) : (
                    <button
                      disabled={busy === model.id}
                      onClick={() =>
                        run(model.id, () => api.uninstallModel(model.id))
                      }
                    >
                      移除
                    </button>
                  )}
                  <button onClick={() => void showLogs(model)}>日志</button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {logs && (
        <div className="modal-backdrop" onClick={() => setLogs(null)}>
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <header className="modal-header">
              <h3>推理日志 · {logs.model}</h3>
              <button onClick={() => setLogs(null)}>关闭</button>
            </header>
            <pre className="logs">{logs.text || "（暂无日志）"}</pre>
          </div>
        </div>
      )}
    </section>
  );
}
