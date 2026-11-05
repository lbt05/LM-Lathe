import { useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  formatBytes,
  formatDate,
  type EngineAsset,
  type EngineHealth,
  type EngineInstallProgress,
  type EngineVersion,
} from "../api";
import { useStore } from "../store";
import {
  Empty,
  Eyebrow,
  Fact,
  Icon,
  Meter,
  Note,
  SheetHead,
  StatusTag,
  Tag,
} from "../ui";

/** How the state of an engine is shown in the table. */
const ENGINE_STATUS: Record<string, { tone: string; label: string }> = {
  installed: { tone: "ok", label: "已安装" },
  system: { tone: "info", label: "系统安装" },
  available: { tone: "idle", label: "未安装" },
  unsupported: { tone: "warn", label: "平台不支持" },
};

/** The models an engine is able to serve. */
function boundModels(store: ReturnType<typeof useStore>, engine: EngineAsset) {
  return store.models.filter((model) => engine.model_engine_ids.includes(model.engine_id));
}

/** The engines that speak instead of answering. */
const TTS_ENGINES = ["sglang-omni", "sglang-omni-mlx", "mlx-audio"];

export default function Runtimes() {
  const store = useStore();
  const [progress, setProgress] = useState<Record<string, EngineInstallProgress>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);

  // The installation runs in the backend, the progress is polled.
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      const entries = await Promise.all(
        store.engines.map(async (engine) => {
          try {
            return await api.engineInstallProgress(engine.id);
          } catch {
            return null;
          }
        }),
      );
      if (cancelled) return;
      const next: Record<string, EngineInstallProgress> = {};
      for (const entry of entries) if (entry) next[entry.engine_id] = entry;
      setProgress(next);
    };
    void tick();
    const timer = window.setInterval(tick, 1200);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [store.engines]);

  // A finished installation refreshes the engine list.
  const finished = Object.values(progress).filter(
    (entry) => entry.state === "done" || entry.state === "failed",
  );
  useEffect(() => {
    if (!finished.length) return;
    const timer = window.setTimeout(() => void store.refresh(), 600);
    return () => window.clearTimeout(timer);
  }, [finished.length, store]);

  const checkHealth = useCallback(
    async (engine: EngineAsset) => {
      setBusy(engine.id);
      store.log("info", `健康检查 ${engine.name}……`);
      try {
        const result = await api.engineHealth(engine.id);
        store.log(
          result.status === "healthy" ? "ok" : "bad",
          `健康检查 ${engine.name}：${result.status}（${result.duration_ms} ms）`,
        );
        store.notify(`${engine.name}：${result.detail}`, result.status === "healthy" ? "ok" : "bad");
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        store.log("bad", `健康检查 ${engine.name} 失败：${message}`);
        store.notify(message, "bad");
      } finally {
        setBusy(null);
      }
    },
    [store],
  );

  const uninstall = useCallback(
    async (engine: EngineAsset, version: string) => {
      setConfirm(null);
      try {
        await api.uninstallEngine(engine.id, version);
        store.log("info", `已卸载 ${engine.name} ${version}`);
        store.notify(`已卸载 ${engine.name} ${version}`);
        await store.refresh();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        store.log("bad", `卸载 ${engine.name} 失败：${message}`);
        store.notify(message, "bad");
      }
    },
    [store],
  );

  const hardware = store.status?.hardware;
  const scheduler = store.status?.scheduler;
  const instances = store.instances;
  const installedCount = store.engines.filter((engine) => engine.installed.length > 0).length;

  return (
    <>
      <section className="sheet">
        <SheetHead
          eyebrow="ENGINES"
          title="推理运行时"
          meta={`${installedCount} / ${store.engines.length} 已安装 · ${store.engines.length} 个引擎`}
          tools={
            <button className="btn btn--ghost" type="button" onClick={() => void store.refresh()}>
              刷新
            </button>
          }
        />

        {/* The installation of an engine is shown while it runs. */}
        {Object.values(progress)
          .filter((entry) => entry.state === "running")
          .map((entry) => (
            <div className="qrow" key={entry.engine_id}>
              <span className="cellstack">
                <span className="name">安装 {entry.engine_id} {entry.version}</span>
                <span className="qrow__meter">
                  <Meter
                    percent={entry.percent ?? 0}
                    kind={TTS_ENGINES.includes(entry.engine_id) ? "tts" : "llm"}
                  />
                </span>
                <span className="sub">{entry.message}</span>
              </span>
              <span className="qrow__pct">
                {entry.percent !== null ? `${entry.percent.toFixed(0)}%` : "…"}
                {entry.total_bytes
                  ? ` · ${formatBytes(entry.bytes)} / ${formatBytes(entry.total_bytes)}`
                  : ""}
              </span>
            </div>
          ))}

        {store.engines.length ? (
          <div className="tablewrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>运行时</th>
                  <th>当前版本</th>
                  <th>状态</th>
                  <th>能力</th>
                  <th>绑定模型</th>
                  <th className="num">占用</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {store.engines.map((engine) => {
                  const models = boundModels(store, engine);
                  const status = ENGINE_STATUS[engine.status] ?? ENGINE_STATUS.available;
                  const running = progress[engine.id]?.state === "running";
                  const key = `${engine.id}:${engine.current_version ?? ""}`;
                  return (
                    <tr
                      key={engine.id}
                      data-act="pick"
                      tabIndex={0}
                      aria-selected={
                        store.inspector?.kind === "runtime" && store.inspector.id === engine.id
                      }
                      onClick={() =>
                        store.setInspector({ kind: "runtime", id: engine.id, tab: "overview" })
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          store.setInspector({
                            kind: "runtime",
                            id: engine.id,
                            tab: "overview",
                          });
                        }
                      }}
                    >
                      <td>
                        <span className="cellrow">
                          <Icon name="layers" />
                          <span className="cellstack">
                            <span className="name">{engine.name}</span>
                            <span className="sub">{engine.summary}</span>
                          </span>
                        </span>
                      </td>
                      <td>
                        <span className="mono">{engine.current_version ?? "—"}</span>
                        {engine.installed.length > 1 ? (
                          <span className="sub"> · 共 {engine.installed.length} 个版本</span>
                        ) : null}
                      </td>
                      <td>
                        <Tag tone={status.tone}>{status.label}</Tag>
                        {running ? <Tag tone="info">安装中</Tag> : null}
                      </td>
                      <td>
                        <span className="sub">{engine.capabilities.join(" / ")}</span>
                      </td>
                      <td>
                        <span className="sub">
                          {models.length
                            ? `${models.length} 个 · ${models[0].name}${
                                models.length > 1 ? " …" : ""
                              }`
                            : engine.model_engine_ids.length
                              ? "无已下载模型"
                              : "未绑定模型"}
                        </span>
                      </td>
                      <td className="num">{formatBytes(engine.size_bytes)}</td>
                      <td>
                        <div className="tbl__actions">
                          {engine.status === "unsupported" ? (
                            <span className="sub">当前平台不支持</span>
                          ) : (
                            <>
                              <button
                                className="btn"
                                type="button"
                                disabled={running}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  store.setInspector({
                                    kind: "runtime",
                                    id: engine.id,
                                    tab: "versions",
                                  });
                                }}
                              >
                                {engine.installed.length ? "切版本" : "安装"}
                              </button>
                              <button
                                className="btn"
                                type="button"
                                disabled={busy === engine.id}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  void checkHealth(engine);
                                }}
                              >
                                {busy === engine.id ? "检查中" : "健康检查"}
                              </button>
                              {engine.current_version ? (
                                confirm === key ? (
                                  <button
                                    className="btn btn--danger"
                                    type="button"
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      void uninstall(engine, engine.current_version!);
                                    }}
                                  >
                                    确认卸载
                                  </button>
                                ) : (
                                  <button
                                    className="btn btn--ghost"
                                    type="button"
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      setConfirm(key);
                                    }}
                                  >
                                    卸载
                                  </button>
                                )
                              ) : null}
                            </>
                          )}
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty
            title={store.base ? "没有已知的运行时" : "尚未连接本地服务"}
            text="运行时由本地的 lm-lathe-server 提供，连接之后才会出现。"
          />
        )}
      </section>

      <section className="sheet">
        <SheetHead
          eyebrow="INSTANCES"
          title="运行实例"
          meta={`${instances.filter((item) => item.state === "running").length} / ${instances.length} 运行中`}
        />
        {instances.length ? (
          <div className="tablewrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>模型</th>
                  <th>引擎</th>
                  <th>状态</th>
                  <th>端点</th>
                  <th className="num">占用内存</th>
                  <th className="num">请求</th>
                </tr>
              </thead>
              <tbody>
                {instances.map((instance) => (
                  <tr key={instance.model_id}>
                    <td>
                      <span className="name">
                        {store.models.find((model) => model.id === instance.model_id)?.name ??
                          instance.model_id}
                      </span>
                      {instance.pinned ? <span className="sub"> · 已固定</span> : null}
                    </td>
                    <td>
                      <span className="mono">{instance.engine_id}</span>
                    </td>
                    <td>
                      <StatusTag state={instance.state} />
                    </td>
                    <td>
                      <span className="sub">{instance.endpoint ?? "—"}</span>
                    </td>
                    <td className="num">
                      {formatBytes(instance.actual_memory)}
                    </td>
                    <td className="num">{instance.active_requests}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty title="没有运行实例" text="在模型页加载一个模型之后，这里会显示它的运行时实例。" />
        )}
      </section>

      <section className="sheet">
        <SheetHead eyebrow="HARDWARE" title="本机硬件" />
        <div className="facts">
          <Fact k="平台" v={`${hardware?.platform ?? "—"} / ${hardware?.architecture ?? "—"}`} />
          <Fact
            k="Apple Silicon"
            v={hardware ? (hardware.apple_silicon ? "是" : "否") : "—"}
          />
          <Fact
            k="CUDA"
            v={hardware ? (hardware.cuda_available ? "可用" : "不可用") : "—"}
          />
          <Fact k="GPU" v={hardware?.nvidia_gpu ?? "—"} />
          <Fact
            k="统一内存"
            v={hardware ? formatBytes(hardware.total_ram_bytes) : "—"}
          />
          <Fact
            k="空闲内存"
            v={
              hardware?.available_ram_bytes !== undefined
                ? formatBytes(hardware.available_ram_bytes)
                : "—"
            }
          />
        </div>
      </section>

      <section className="sheet">
        <SheetHead eyebrow="SCHEDULER" title="内存调度" />
        <div className="facts">
          <Fact k="总池" v={scheduler ? formatBytes(scheduler.total_bytes) : "—"} />
          <Fact
            k="已预留"
            v={scheduler ? formatBytes(scheduler.reserved_bytes) : "—"}
          />
          <Fact
            k="已用"
            v={scheduler ? formatBytes(scheduler.used_bytes) : "—"}
          />
          <Fact
            k="可用（估算）"
            v={scheduler ? formatBytes(scheduler.available_bytes) : "—"}
          />
        </div>
        {scheduler?.tenants.length ? (
          <div className="tablewrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>租户</th>
                  <th>类型</th>
                  <th>状态</th>
                  <th className="num">请求</th>
                  <th className="num">预留</th>
                  <th>最近使用</th>
                </tr>
              </thead>
              <tbody>
                {scheduler.tenants.map((tenant) => (
                  <tr key={tenant.key}>
                    <td>
                      <span className="mono">{tenant.key}</span>
                    </td>
                    <td>
                      <span className="sub">{tenant.kind}</span>
                    </td>
                    <td>
                      <Tag tone={tenant.state === "active" ? "ok" : "idle"}>{tenant.state}</Tag>
                    </td>
                    <td className="num">{tenant.active_requests}</td>
                    <td className="num">{formatBytes(tenant.reserved_bytes)}</td>
                    <td>
                      <span className="sub">{formatDate(tenant.last_used_at)}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty title="没有租约" text="模型加载之后会在这里显示内存租约。" />
        )}
      </section>
    </>
  );
}

/** The inspector of one engine. */
export function RuntimeInspector({ id, tab }: { id: string; tab: string }) {
  const store = useStore();
  const engine = useMemo(
    () => store.engines.find((entry) => entry.id === id),
    [store.engines, id],
  );
  const [versions, setVersions] = useState<EngineVersion[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string>("");
  const [health, setHealth] = useState<EngineHealth | null>(null);
  const [progress, setProgress] = useState<EngineInstallProgress | null>(null);

  // The installation output is polled, so that the log of a running
  // installation is visible while it works.
  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const next = await api.engineInstallProgress(id);
        if (!cancelled) setProgress(next);
      } catch {
        /* the backend is not reachable */
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [id]);

  const loadVersions = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await api.engineVersions(id);
      setVersions(list);
      setSelected(list.find((entry) => !entry.installed)?.version ?? list[0]?.version ?? "");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setVersions(null);
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    if (tab === "versions" && versions === null && !loading) void loadVersions();
  }, [tab, versions, loading, loadVersions]);

  if (!engine) {
    return (
      <div className="insp__body">
        <Empty title="引擎不存在" text="它可能已经被移除。" />
      </div>
    );
  }

  const models = boundModels(store, engine);
  const status = ENGINE_STATUS[engine.status] ?? ENGINE_STATUS.available;

  const doInstall = async (version?: string) => {
    try {
      await api.installEngine(engine.id, version);
      store.log("info", `开始安装 ${engine.name}${version ? ` ${version}` : ""}`);
      store.notify(`开始安装 ${engine.name}`);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `安装失败：${message}`);
      store.notify(message, "bad");
    }
  };

  const doHealth = async () => {
    store.log("info", `健康检查 ${engine.name}……`);
    try {
      const result = await api.engineHealth(engine.id);
      setHealth(result);
      store.notify(`${engine.name}：${result.detail}`, result.status === "healthy" ? "ok" : "bad");
    } catch (err) {
      store.notify(err instanceof Error ? err.message : String(err), "bad");
    }
  };

  const doActivate = async (version: string) => {
    try {
      await api.activateEngine(engine.id, version);
      store.log("ok", `${engine.name} 已切换到 ${version}`);
      store.notify(`${engine.name} → ${version}`, "ok");
      await store.refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `切换版本失败：${message}`);
      store.notify(message, "bad");
    }
  };

  const doUninstall = async (version: string) => {
    try {
      await api.uninstallEngine(engine.id, version);
      store.log("info", `已卸载 ${engine.name} ${version}`);
      store.notify(`已卸载 ${engine.name} ${version}`);
      await store.refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      store.log("bad", `卸载失败：${message}`);
      store.notify(message, "bad");
    }
  };

  return (
    <div className="insp__body">
      <div className="insp__tabs" role="tablist">
        {["overview", "versions", "logs"].map((name) => (
          <button
            key={name}
            role="tab"
            type="button"
            aria-selected={tab === name}
            onClick={() =>
              store.setInspector({ kind: "runtime", id: engine.id, tab: name })
            }
          >
            {name === "overview" ? "概览" : name === "versions" ? "版本" : "日志"}
          </button>
        ))}
      </div>

      {tab === "overview" ? (
        <>
          <div className="matrix">
            <div className="matrix__row">
              <span className="matrix__k">状态</span>
              <span className="matrix__v">
                <Tag tone={status.tone}>{status.label}</Tag>
                {engine.current_version ? (
                  <span className="matrix__why">当前版本 {engine.current_version}</span>
                ) : null}
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">分发</span>
              <span className="matrix__v">
                <span className="mono">
                  {engine.distribution === "binary"
                    ? "GitHub Release"
                    : engine.distribution === "source-build"
                      ? "源码构建"
                      : "PyPI 包"}
                </span>
                <span className="matrix__why">
                  {engine.repository ?? engine.package ?? "—"}
                </span>
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">程序</span>
              <span className="matrix__v">
                <span className="mono">{engine.program}</span>
                {engine.programs.length ? (
                  <span className="matrix__why">
                    运行时提供：{engine.programs.join(" / ")}
                  </span>
                ) : null}
                {engine.system_entry ? (
                  <span className="matrix__why">系统安装：{engine.system_entry}</span>
                ) : null}
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">平台</span>
              <span className="matrix__v">
                <span className="mono">{engine.platform}</span>
                <span className="matrix__why">
                  支持 {engine.platforms.join(" / ")}
                  {engine.supported_here ? " · 当前平台可用" : " · 当前平台不可用"}
                </span>
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">能力</span>
              <span className="matrix__v">
                <span className="mono">{engine.capabilities.join(" / ")}</span>
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">绑定模型</span>
              <span className="matrix__v">
                {models.length ? (
                  models.map((model) => (
                    <span className="matrix__why" key={model.id}>
                      {model.name} · {model.installed ? "已下载" : "未下载"}
                    </span>
                  ))
                ) : (
                  <span className="matrix__why">
                    {engine.model_engine_ids.length
                      ? "该引擎可服务的模型尚未下载。"
                      : "M1 阶段只做引擎资产管理，未绑定模型。"}
                  </span>
                )}
              </span>
            </div>
            <div className="matrix__row">
              <span className="matrix__k">占用</span>
              <span className="matrix__v">
                <span className="mono">{formatBytes(engine.size_bytes)}</span>
                <span className="matrix__why">
                  已安装 {engine.installed.length} 个版本
                </span>
              </span>
            </div>
          </div>

          {engine.note ? (
            <Note title="安装方式" tone="info">
              <p>{engine.note}</p>
              {engine.requirements.length ? (
                <p>前置工具：{engine.requirements.join(" / ")}</p>
              ) : null}
            </Note>
          ) : null}

          {health ? (
            <Note
              title="健康检查"
              tone={health.status === "healthy" ? "ok" : health.status === "missing" ? "warn" : "bad"}
            >
              <p>{health.detail}</p>
              {health.endpoint ? <p className="mono">{health.endpoint}</p> : null}
              <p>耗时 {health.duration_ms} ms</p>
            </Note>
          ) : null}

          <div className="tbl__actions">
            <button className="btn" type="button" onClick={() => void doHealth()}>
              健康检查
            </button>
            <button
              className="btn btn--primary"
              type="button"
              onClick={() =>
                store.setInspector({ kind: "runtime", id: engine.id, tab: "versions" })
              }
            >
              {engine.installed.length ? "切换版本" : "安装"}
            </button>
          </div>
        </>
      ) : null}

      {tab === "versions" ? (
        <>
          <Eyebrow>已安装版本</Eyebrow>
          {engine.installed.length ? (
            <div className="verlist">
              {engine.installed.map((installed) => (
                <div className="ver" key={installed.version}>
                  <span>{installed.version}</span>
                  {installed.version === engine.current_version ? (
                    <Tag tone="ok">当前版本</Tag>
                  ) : (
                    <button
                      className="btn btn--ghost ver__tag"
                      type="button"
                      onClick={() => void doActivate(installed.version)}
                    >
                      切换
                    </button>
                  )}
                  <span className="sub">
                    {formatBytes(installed.size_bytes)} · {formatDate(installed.installed_at)}
                  </span>
                  <button
                    className="btn btn--ghost ver__tag"
                    type="button"
                    onClick={() => void doUninstall(installed.version)}
                  >
                    卸载
                  </button>
                </div>
              ))}
            </div>
          ) : (
            <Empty title="尚未安装" text="从下面的可用版本里选一个开始安装。" />
          )}

          <Eyebrow>可用版本</Eyebrow>
          {error ? (
            <Note title="获取失败" tone="warn">
              <p>{error}</p>
              <p>离线或网络受限时无法获取版本列表。</p>
            </Note>
          ) : null}

          <div className="filters">
            <span className="field">
              <span className="field__label">版本</span>
              <select
                className="input"
                value={selected}
                disabled={!versions?.length}
                onChange={(event) => setSelected(event.target.value)}
              >
                {versions?.length ? (
                  versions.map((version) => (
                    <option key={version.version} value={version.version}>
                      {version.version}
                      {version.installed ? "（已安装）" : ""}
                      {version.prerelease ? " · 预发布" : ""}
                    </option>
                  ))
                ) : (
                  <option value="">{loading ? "获取中……" : "没有可用版本"}</option>
                )}
              </select>
            </span>
            <button
              className="btn btn--primary"
              type="button"
              disabled={!selected || loading}
              onClick={() => void doInstall(selected)}
            >
              安装所选版本
            </button>
            <button className="btn" type="button" onClick={() => void loadVersions()}>
              刷新版本
            </button>
          </div>

          {versions?.length ? (
            <div className="verlist">
              {versions.slice(0, 12).map((version) => (
                <div className="ver" key={version.version}>
                  <span>{version.version}</span>
                  <span className="sub">
                    {version.published_at ? formatDate(version.published_at) : "—"}
                    {version.size_bytes ? ` · ${formatBytes(version.size_bytes)}` : ""}
                  </span>
                  {version.installed ? (
                    <Tag tone="ok">已安装</Tag>
                  ) : (
                    <button
                      className="btn btn--ghost ver__tag"
                      type="button"
                      onClick={() => void doInstall(version.version)}
                    >
                      安装
                    </button>
                  )}
                </div>
              ))}
            </div>
          ) : null}
        </>
      ) : null}

      {tab === "logs" ? (
        <>
          <Eyebrow>安装日志</Eyebrow>
          <div className="term">
            <span className="term__line">
              {engine.installed.length
                ? engine.installed
                    .map(
                      (installed) =>
                        `${installed.version} · ${formatBytes(installed.size_bytes)} · ${installed.source} · ${formatDate(installed.installed_at)}`,
                    )
                    .join("\n")
                : "该引擎尚未安装，没有安装日志。"}
            </span>
          </div>
          {progress?.log.length ? (
            <>
              <Eyebrow>当前安装输出</Eyebrow>
              <div className="term">
                <span className="term__line">{progress.log.join("\n")}</span>
              </div>
            </>
          ) : null}
          <Note title="说明" tone="">
            <p>
              安装由后端完成：llama.cpp 从 GitHub Release 下载并解压；
              MLX 用 pip 装进独立虚拟环境；SGLang-Omni 在 CUDA 机器上用 PyPI
              发行版，在 Apple Silicon 上用仓库自带的脚本源码安装；
              原生 MLX 运行时则用 CMake/Ninja 编译出各模型的 C++ 服务二进制。
            </p>
            <p>日志会显示安装过程中的输出。</p>
          </Note>
        </>
      ) : null}
    </div>
  );
}
