import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  formatBytes,
  type DiscoveredModel,
  type Model,
} from "../api";
import { useStore } from "../store";
import {
  Empty,
  Eyebrow,
  Icon,
  Kv,
  Meter,
  Note,
  SheetHead,
  StatusTag,
  Tag,
  TypeMark,
} from "../ui";

/** The Chinese name of a capability. */
const LLM_FEATURE_LABELS: Record<string, string> = {
  chat: "对话",
  completion: "文本补全",
  tool_use: "工具调用",
  vision: "图像输入",
  reasoning: "思维链",
  long_context: "长上下文",
};

const FEATURE_LABELS: Record<string, string> = {
  text_to_speech: "语音合成",
  voice_design: "声音设计",
  voice_cloning: "声音克隆",
  custom_voice: "内置音色",
  multi_speaker: "多说话人",
  emotional_control: "情感控制",
  instruction_control: "指令控制",
  point_tags: "韵律标记",
  streaming: "流式输出",
  audio_editing: "音频编辑",
};

/** The Chinese name of a capability, or the raw name. */
function featureLabel(feature: string): string {
  return FEATURE_LABELS[feature] ?? feature;
}

/** The purpose badge of a model. */
function purposeOf(model: Model): string {
  if (model.family === "llm") return "LLM";
  return "TTS";
}

/** The capabilities of a model, the speech ones and the language ones. */
function capabilitiesOf(model: Model): string[] {
  const speech = model.features
    .filter((feature) => feature !== "text_to_speech")
    .map(featureLabel);
  const language = (model.llm_features ?? []).map(
    (feature) => LLM_FEATURE_LABELS[feature] ?? feature,
  );
  return [...speech, ...language];
}

/** `safetensors` → `Safetensors`. */
function formatOf(model: Model): string {
  return model.format ? model.format.toUpperCase() : "—";
}

/**
 * A warning mark that tells when it is read: the reason is in the tooltip.
 */
function WarnMark({ warnings }: { warnings?: string[] }) {
  if (!warnings || !warnings.length) return null;
  return (
    <span className="warnmark" title={warnings.join("\n")}>
      <Icon name="alert" size={13} />
    </span>
  );
}

/** The number of the parameters, e.g. `1.7B`. */
function parametersOf(model: Model): string {
  if (!model.parameters) return "—";
  const billions = model.parameters / 1e9;
  return billions >= 10
    ? `${billions.toFixed(0)}B`
    : `${billions.toFixed(1)}B`;
}

function matches(model: Model, query: string) {
  if (!query) return true;
  const needle = query.toLowerCase();
  const haystack = [
    model.name,
    model.id,
    model.family,
    model.engine_id,
    model.hf_repo,
    model.version,
  ]
    .join(" ")
    .toLowerCase();
  return haystack.includes(needle);
}

/** A filter chip group. */
function FilterGroup({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: [string, string][];
  onChange: (value: string) => void;
}) {
  return (
    <span className="fgroup">
      <span className="fgroup__label">{label}</span>
      {options.map(([key, text]) => (
        <button
          className="ftog"
          type="button"
          key={key}
          aria-pressed={value === key}
          onClick={() => onChange(key)}
        >
          {text}
        </button>
      ))}
    </span>
  );
}

/** A sort dropdown. */
function SortSelect({
  value,
  options,
  onChange,
}: {
  value: string;
  options: [string, string][];
  onChange: (value: string) => void;
}) {
  return (
    <span className="fgroup">
      <span className="fgroup__label">排序</span>
      <select
        className="sel"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map(([key, text]) => (
          <option value={key} key={key}>
            {text}
          </option>
        ))}
      </select>
    </span>
  );
}

export default function Models({
  query,
  onQueryChange,
}: {
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const store = useStore();
  const [tab, setTab] = useState<"discover" | "local">("discover");
  const [type, setType] = useState("all");
  const [sort, setSort] = useState("memory");
  // Deleting a checkpoint removes gigabytes from the disk, the row waits for
  // a second click before it does.
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  // The confirmation for throwing away the files of a download that was
  // stopped.
  const [confirmDiscard, setConfirmDiscard] = useState<string | null>(null);
  // The search of the Hugging Face Hub, which is where the models that
  // are not part of the application come from.
  const [hubQuery, setHubQuery] = useState("");
  const [hubResults, setHubResults] = useState<DiscoveredModel[]>([]);
  const [hubDropped, setHubDropped] = useState(0);
  const [hubLoading, setHubLoading] = useState(false);
  const [hubSearched, setHubSearched] = useState(false);
  const [hubSort, setHubSort] = useState("size");

  const models = store.models;

  // The confirmation expires, a pending deletion must not stay armed.
  useEffect(() => {
    if (!confirmDelete) return;
    const timer = window.setTimeout(() => setConfirmDelete(null), 6000);
    return () => window.clearTimeout(timer);
  }, [confirmDelete]);

  // The same for the files of a download that was stopped.
  useEffect(() => {
    if (!confirmDiscard) return;
    const timer = window.setTimeout(() => setConfirmDiscard(null), 6000);
    return () => window.clearTimeout(timer);
  }, [confirmDiscard]);

  const catalog = useMemo(
    () => models.filter((model) => !model.installed),
    [models],
  );
  const local = useMemo(
    () => models.filter((model) => model.installed),
    [models],
  );

  const filter = useCallback(
    (model: Model) => {
      if (type !== "all" && purposeOf(model) !== type) return false;
      if (!matches(model, query)) return false;
      return true;
    },
    [type, query],
  );

  const bySort = useCallback(
    (list: Model[]) =>
      list.slice().sort((a, b) => {
        if (sort === "size") return b.size_bytes - a.size_bytes;
        if (sort === "name") return a.name.localeCompare(b.name, "zh");
        return b.estimated_memory_bytes - a.estimated_memory_bytes;
      }),
    [sort],
  );

  const installedList = useMemo(
    () => bySort(local.filter(filter)),
    [local, filter, bySort],
  );

  // Only the downloaded models take up space; the rest of the catalogue is
  // still on the hub.
  const totalBytes = useMemo(
    () => local.reduce((sum, model) => sum + model.size_bytes, 0),
    [local],
  );

  // The order of the results of the hub. The models without a size come
  // last; the hub does not know the size of every repository.
  const sortedHub = useMemo(() => {
    const list = [...hubResults];
    list.sort((left, right) => {
      switch (hubSort) {
        case "parameters":
          return (right.parameters ?? 0) - (left.parameters ?? 0);
        case "name":
          return left.repo.localeCompare(right.repo);
        case "downloads":
          return right.downloads - left.downloads;
        default: {
          const leftSize = left.size_bytes ?? 0;
          const rightSize = right.size_bytes ?? 0;
          if (!leftSize && !rightSize) return left.repo.localeCompare(right.repo);
          if (!leftSize) return 1;
          if (!rightSize) return -1;
          return leftSize - rightSize;
        }
      }
    });
    return list;
  }, [hubResults, hubSort]);

  // Searches the Hugging Face Hub. The hub is asked when the search is
  // confirmed, not with every keystroke. An empty query asks for the
  // models that are downloaded most often.
  const searchHub = useCallback(
    async (wanted?: string) => {
      const query = (wanted ?? hubQuery).trim();
      setHubLoading(true);
      try {
        const answer = await api.discover(query, 20);
        setHubResults(answer.models);
        setHubDropped(answer.dropped);
        setHubSearched(true);
        store.log(
          "info",
          query
            ? `在 Hugging Face 上搜索 ${query}，找到 ${answer.models.length} 个模型`
            : `列出 Hugging Face 上最常用的 ${answer.models.length} 个模型`,
        );
      } catch (err) {
        setHubResults([]);
        setHubDropped(0);
        setHubSearched(true);
        const message = err instanceof Error ? err.message : String(err);
        store.log("bad", `搜索失败：${message}`);
        store.notify(message, "bad");
      } finally {
        setHubLoading(false);
      }
    },
    [hubQuery, store],
  );

  // The page opens with the models that are downloaded most often, so
  // there is something to look at before a word was typed. The search
  // waits for the connection, because the address of the service is only
  // known then.
  const loadedBase = useRef<string | null>(null);
  useEffect(() => {
    if (!store.base) return;
    if (loadedBase.current === store.base) return;
    loadedBase.current = store.base;
    void searchHub("");
  }, [store.base, searchHub]);

  // Downloads a model of the hub. The model joins the catalogue of the
  // application, so its progress is reported like the progress of every
  // other model.
  const installDiscovered = useCallback(
    async (model: DiscoveredModel) => {
      try {
        const started = await api.installDiscovered(model.repo);
        store.enqueue({
          id: started.model_id,
          name: started.name || model.name,
          percent: 0,
          bytes: 0,
          bytesTotal: 0,
          state: "running",
        });
        store.log("info", `开始下载 ${model.name}`);
        store.notify(`已加入下载队列：${model.name}`);
        await store.refresh();
      } catch (err) {
        store.log("bad", `下载 ${model.name} 失败`);
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const uninstall = useCallback(
    async (model: Model) => {
      try {
        await api.uninstallModel(model.id);
        store.log("warn", `已删除本地模型 ${model.name}`);
        store.notify(`已删除 ${model.name}`);
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const toggleLoad = useCallback(
    async (model: Model) => {
      const runtime = store.runtimeOf(model.id);
      try {
        if (runtime?.state === "running") {
          await api.unloadModel(model.id);
          store.log("info", `已停止 ${model.name}`);
        } else {
          store.log("info", `正在加载 ${model.name}`);
          await api.loadModel(model.id);
          store.log("ok", `${model.name} 已就绪`);
        }
        await store.refresh();
      } catch (err) {
        store.log("bad", `${model.name} 加载失败`);
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const togglePin = useCallback(
    async (model: Model) => {
      try {
        await api.pinModel(model.id, !model.pinned);
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const selected = (model: Model) =>
    store.inspector?.kind === "model" && store.inspector.id === model.id;

  const pick = (model: Model) =>
    store.setInspector({ kind: "model", id: model.id, tab: "overview" });

  const queue = store.queue;

  return (
    <>
      <div className="tabbar" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "discover"}
          onClick={() => setTab("discover")}
        >
          发现模型
          <span className="tabbar__n">{catalog.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "local"}
          onClick={() => setTab("local")}
        >
          已下载
          <span className="tabbar__n">{local.length}</span>
        </button>
      </div>

      {queue.length ? (
        <section className="sheet">
          <SheetHead
            eyebrow="DOWNLOAD QUEUE"
            title="下载队列"
            meta={`${queue.length} 个任务`}
          />
          <div>
            {queue.map((job) => (
              <div className="qrow" key={job.id}>
                <div className="qrow__meter">
                  <span className="cellrow">
                    <span className="name" style={{ fontSize: "12.5px" }}>
                      {job.name}
                    </span>
                    <span className="qrow__pct">
                      {Math.floor(job.percent)}% · {formatBytes(job.bytes)}
                      {job.bytesTotal ? ` / ${formatBytes(job.bytesTotal)}` : ""}
                    </span>
                  </span>
                  <Meter percent={job.percent} />
                  {job.state === "failed" && job.error ? (
                    <span className="qrow__error">{job.error}</span>
                  ) : null}
                </div>
                <span className="cellrow">
                  {job.state === "failed" ? (
                    <>
                      <Tag tone="bad">失败</Tag>
                      <button
                        className="btn btn--ghost"
                        type="button"
                        onClick={() => void store.retryInstall(job)}
                      >
                        {job.bytes > 0 ? "继续下载" : "重试"}
                      </button>
                    </>
                  ) : job.state === "partial" ? (
                    <>
                      <Tag tone="warn">未完成</Tag>
                      <button
                        className="btn btn--ghost"
                        type="button"
                        onClick={() => void store.retryInstall(job)}
                      >
                        继续下载
                      </button>
                    </>
                  ) : job.state === "cancelled" ? (
                    <>
                      <Tag tone="idle">已取消</Tag>
                      <button
                        className="btn btn--ghost"
                        type="button"
                        onClick={() => void store.retryInstall(job)}
                      >
                        {job.bytes > 0 ? "继续下载" : "重新下载"}
                      </button>
                    </>
                  ) : (
                    <>
                      <Tag tone="info">下载中</Tag>
                      <button
                        className="btn btn--ghost"
                        type="button"
                        title="取消下载，已经下载的部分会保留"
                        onClick={() => void store.cancelInstall(job)}
                      >
                        取消
                      </button>
                    </>
                  )}
                  {job.state === "failed" ||
                  job.state === "partial" ||
                  job.state === "cancelled" ? (
                    confirmDiscard === job.id ? (
                      <button
                        className="btn btn--danger"
                        type="button"
                        onClick={() => {
                          setConfirmDiscard(null);
                          void store.discardInstall(job);
                        }}
                      >
                        确认丢弃
                      </button>
                    ) : (
                      <button
                        className="btn btn--ghost"
                        type="button"
                        title="丢弃已经下载的部分，腾出磁盘空间"
                        onClick={() => setConfirmDiscard(job.id)}
                      >
                        丢弃
                      </button>
                    )
                  ) : null}
                </span>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      {tab === "discover" ? (
        <>
          <section className="sheet">
            <SheetHead
              eyebrow="HUGGING FACE"
              title="在 Hugging Face 上找模型"
              meta={
                hubLoading
                  ? "搜索中…"
                  : hubSearched
                    ? `${hubResults.length} 个结果${hubDropped ? ` · ${hubDropped} 个不支持` : ""}`
                    : "搜索仓库里的模型"
              }
            />            <div className="filters">
              <span className="fgroup">
                <input
                  className="finput"
                  type="search"
                  value={hubQuery}
                  placeholder="搜索 Hugging Face 上的模型…"
                  onChange={(event) => setHubQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") void searchHub();
                  }}
                />
              </span>
              <button
                className="btn btn--primary"
                type="button"
                disabled={hubLoading}
                onClick={() => void searchHub()}
              >
                <Icon name="search" />
                搜索
              </button>
              {hubResults.length ? (
                <SortSelect
                  value={hubSort}
                  options={[
                    ["size", "文件大小"],
                    ["parameters", "参数量"],
                    ["name", "名称"],
                    ["downloads", "下载量"],
                  ]}
                  onChange={setHubSort}
                />
              ) : null}
            </div>
            {hubResults.length ? (
              <div className="tablewrap">
                <table className="tbl">
                  <thead>
                    <tr>
                      <th>模型</th>
                      <th className="num">参数量</th>
                      <th className="num">文件大小</th>
                      <th>用途</th>
                      <th>格式</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {sortedHub.map((model) => (
                      <tr key={model.repo}>
                        <td>
                          <span className="cellstack">
                            <span className="cellrow">
                              <TypeMark type={model.family === "llm" ? "LLM" : "TTS"} />
                              <span className="name">{model.name}</span>
                              <WarnMark warnings={model.warnings} />
                            </span>
                            <span className="sub">
                              {model.quant
                                ? `${model.quant} · 只下载这一个量化`
                                : model.format === "gguf"
                                  ? "GGUF 权重"
                                  : model.format === "mlx"
                                    ? "MLX 权重"
                                    : "safetensors 权重"}
                            </span>
                          </span>
                        </td>
                        <td className="num">
                          {model.parameters
                            ? `${(model.parameters / 1e9).toFixed(1)}B`
                            : "—"}
                        </td>
                        <td className="num">
                          {model.size_bytes ? formatBytes(model.size_bytes) : "—"}
                        </td>
                        <td>
                          <Tag tone={model.family === "llm" ? "llm" : "tts"}>
                            {model.family === "llm" ? "LLM" : "TTS"}
                          </Tag>
                        </td>
                        <td>{model.format}</td>
                        <td>
                          <div className="tbl__actions">
                            {model.installed ? (
                              <Tag tone="ok">已下载</Tag>
                            ) : (
                              <button
                                className="btn btn--primary"
                                type="button"
                                onClick={() => void installDiscovered(model)}
                              >
                                <Icon name="download" />
                                下载
                              </button>
                            )}
                          </div>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : hubSearched ? (
              <Empty
                title="没有找到能跑的模型"
                text={
                  hubDropped
                    ? `Hugging Face 上有 ${hubDropped} 个同名仓库，但当前运行时不支持它们：语言模型需要 GGUF 权重，语音模型需要 Qwen3-TTS 权重。`
                    : "Hugging Face 上没有同名仓库。当前运行时支持 GGUF 语言模型和 Qwen3-TTS 语音模型。"
                }
              />
            ) : (
              <Empty
                title="搜一搜 Hugging Face"
                text="输入模型名字，比如 Qwen3-TTS、llama、fish-audio，搜索结果可以直接下载。"
              />
            )}
          </section>
        </>
      ) : null}

      {tab === "local" ? (
        <section className="sheet">
          <SheetHead
            eyebrow="LOCAL · INSTALLED"
            title="已下载"
            meta={`${local.length} 个模型 · ${formatBytes(totalBytes)}`}
          />
          <div className="filters">
            <FilterGroup
              label="用途"
              value={type}
              options={[
                ["all", "全部"],
                ["LLM", "LLM"],
                ["TTS", "TTS"],
              ]}
              onChange={setType}
            />
            <SortSelect
              value={sort}
              options={[
                ["memory", "内存估算"],
                ["size", "文件大小"],
                ["name", "名称"],
              ]}
              onChange={setSort}
            />
            <span className="fgroup">
              <input
                className="finput"
                type="search"
                value={query}
                placeholder="搜索已下载的模型…"
                onChange={(event) => onQueryChange(event.target.value)}
              />
            </span>
          </div>
          {installedList.length ? (
            <div className="tablewrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>模型</th>
                    <th>用途</th>
                    <th>格式</th>
                    <th>参数量</th>
                    <th className="num">文件大小</th>
                    <th className="num">内存估算</th>
                    <th>就绪度</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {installedList.map((model) => {
                    const runtime = store.runtimeOf(model.id);
                    const running = runtime?.state === "running";
                    return (
                      <tr
                        key={model.id}
                        data-act="pick"
                        tabIndex={0}
                        aria-selected={selected(model)}
                        onClick={() => pick(model)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") pick(model);
                        }}
                      >
                        <td>
                          <span className="cellstack">
                            <span className="cellrow">
                              <TypeMark type={purposeOf(model)} />
                              <span className="name">{model.name}</span>
                              <WarnMark warnings={model.warnings} />
                              {model.pinned ? (
                                <Icon name="shield" />
                              ) : null}
                            </span>
                            <span className="sub">{model.hf_repo}</span>
                          </span>
                        </td>
                        <td>
                          <Tag tone="tts">{purposeOf(model)}</Tag>
                        </td>
                        <td>{formatOf(model)}</td>
                        <td>{parametersOf(model)}</td>
                        <td className="num">{formatBytes(model.size_bytes)}</td>
                        <td className="num">
                          {formatBytes(model.estimated_memory_bytes)}
                        </td>
                        <td>
                          {runtime ? (
                            <StatusTag state={runtime.state} />
                          ) : (
                            <Tag tone="idle">未加载</Tag>
                          )}
                        </td>
                        <td>
                          <div className="tbl__actions">
                            <button
                              className="btn"
                              type="button"
                              onClick={(event) => {
                                event.stopPropagation();
                                void toggleLoad(model);
                              }}
                            >
                              <Icon name="play" />
                              {running ? "卸载" : "加载"}
                            </button>
                            <button
                              className="btn btn--icon"
                              type="button"
                              title={model.pinned ? "取消固定" : "固定"}
                              aria-label={model.pinned ? "取消固定" : "固定"}
                              onClick={(event) => {
                                event.stopPropagation();
                                void togglePin(model);
                              }}
                            >
                              <Icon name="shield" />
                            </button>
                            {confirmDelete === model.id ? (
                              <button
                                className="btn btn--danger"
                                type="button"
                                title="确认删除"
                                aria-label="确认删除"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setConfirmDelete(null);
                                  void uninstall(model);
                                }}
                              >
                                确认删除
                              </button>
                            ) : (
                              <button
                                className="btn btn--icon"
                                type="button"
                                title="删除"
                                aria-label="删除"
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setConfirmDelete(model.id);
                                }}
                              >
                                <Icon name="trash" />
                              </button>
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
              title={store.base ? "没有匹配的本地模型" : "尚未连接本地服务"}
              text={
                store.base
                  ? "换个筛选条件，或切到「发现模型」下载一个。"
                  : "已下载的模型清单来自本地的 lm-lathe-server。"
              }
              action={
                store.base ? (
                  <button
                    className="btn"
                    type="button"
                    onClick={() => setTab("discover")}
                  >
                    去发现模型
                  </button>
                ) : (
                  <button
                    className="btn"
                    type="button"
                    onClick={() => void store.connect()}
                  >
                    重新连接
                  </button>
                )
              }
            />
          )}
        </section>
      ) : null}
    </>
  );
}

/** The right hand panel of one model. */
export function ModelInspector({ id, tab }: { id: string; tab: string }) {
  const store = useStore();
  const model = store.models.find((item) => item.id === id);
  const [logs, setLogs] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);

  const runtime = useMemo(
    () => store.runtimeOf(id),
    [store, id],
  );

  useEffect(() => {
    if (tab !== "logs") return;
    // The engine writes its log while it runs, so the panel reads it again
    // and again instead of showing the state of the first second.
    let alive = true;
    const read = () => {
      void api
        .modelLogs(id)
        .then((text) => {
          if (alive) setLogs(text);
        })
        .catch(() => {
          if (alive) setLogs("无法读取日志");
        });
    };
    read();
    const timer = window.setInterval(read, 3000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [tab, id]);

  const toggleLoad = useCallback(
    async (model: Model) => {
      try {
        if (runtime?.state === "running") {
          await api.unloadModel(model.id);
          store.log("info", `已停止 ${model.name}`);
        } else {
          store.log("info", `正在加载 ${model.name}`);
          await api.loadModel(model.id);
          store.log("ok", `${model.name} 已就绪`);
        }
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [runtime, store],
  );

  const togglePin = useCallback(
    async (model: Model) => {
      try {
        await api.pinModel(model.id, !model.pinned);
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const install = useCallback(
    async (model: Model) => {
      try {
        await api.installModel(model.id);
        store.enqueue({
          id: model.id,
          name: model.name,
          percent: 0,
          bytes: 0,
          bytesTotal: 0,
          state: "running",
        });
        store.log("info", `开始下载 ${model.name}`);
        store.notify(`已加入下载队列：${model.name}`);
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  const uninstall = useCallback(
    async (model: Model) => {
      try {
        await api.uninstallModel(model.id);
        store.log("warn", `已删除本地模型 ${model.name}`);
        store.notify(`已删除 ${model.name}`);
        await store.refresh();
      } catch (err) {
        store.notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [store],
  );

  if (!model) {
    return (
      <>
        <div className="insp__head">
          <span className="cellstack">
            <Eyebrow>MODEL</Eyebrow>
            <span className="insp__title">模型不存在</span>
          </span>
        </div>
      </>
    );
  }

  const tabs: [string, string][] = [
    ["overview", "概览"],
    ["usage", "用量"],
    ["logs", "日志"],
  ];

  const setTab = (next: string) =>
    store.setInspector({ kind: "model", id, tab: next });

  let body;
  if (tab === "logs") {
    body = (
      <div className="term">
        {logs === null ? (
          <span className="term__line">正在读取日志…</span>
        ) : logs.trim() ? (
          logs
            .split("\n")
            .slice(-300)
            .map((line, index) => (
              <span className="term__line" key={index}>
                {line}
              </span>
            ))
        ) : (
          <span className="term__line">
            {runtime
              ? "暂无日志输出"
              : "没有日志：模型还没有启动过，或启动时没有留下输出。"}
          </span>
        )}
      </div>
    );
  } else if (tab === "usage") {
    body = runtime ? (
      <div className="cellstack">
        <div className="kv">
          <Kv k="引擎" v={runtime.engine_id} mono />
          <Kv k="端点" v={runtime.endpoint ?? "—"} mono />
          <Kv k="进程" v={runtime.process_id ?? "—"} mono />
          <Kv k="活动请求" v={String(runtime.active_requests)} mono />
          <Kv k="估算内存" v={formatBytes(runtime.estimated_memory)} mono />
          <Kv k="实际内存" v={formatBytes(runtime.actual_memory)} mono />
          <Kv
            k="最近使用"
            v={
              runtime.last_used_at
                ? new Date(runtime.last_used_at).toLocaleString("zh-CN", {
                    hour12: false,
                  })
                : "—"
            }
            mono
          />
        </div>
        <div className="cellrow">
          <StatusTag state={runtime.state} />
        </div>
      </div>
    ) : (
      <Empty
        title="还没有使用记录"
        text="加载这个模型之后，这里会显示它的运行时占用。"
      />
    );
  } else {
    body = (
      <>
        <div className="cellrow">
          <TypeMark type={purposeOf(model)} />
          {model.compatible ? (
            <Tag tone="ok">兼容</Tag>
          ) : (
            <Tag tone="bad">不兼容</Tag>
          )}
          {model.pinned ? <Tag tone="info">已固定</Tag> : null}
        </div>
        <div className="kv">
          <Kv k="模型" v={model.id} mono />
          <Kv k="仓库" v={model.hf_repo} mono />
          <Kv k="引擎" v={model.engine_id} mono />
          <Kv k="格式" v={formatOf(model)} />
          <Kv k="参数量" v={parametersOf(model)} />
          <Kv k="文件大小" v={formatBytes(model.size_bytes)} mono />
          <Kv
            k="内存估算"
            v={formatBytes(model.estimated_memory_bytes)}
            mono
          />
          <Kv k="能力" v={capabilitiesOf(model).join(" / ")} />
        </div>
        {model.compatibility ? (
          <Note title="兼容性">
            <p>{model.compatibility}</p>
          </Note>
        ) : null}
        {model.notes.length ? (
          <Note title="说明">
            <p>{model.notes.join("\n")}</p>
          </Note>
        ) : null}
        {model.warnings?.length ? (
          <Note title="注意">
            <p>{model.warnings.join("\n")}</p>
          </Note>
        ) : null}
      </>
    );
  }

  return (
    <>
      <div className="insp__head">
        <span className="cellstack">
          <Eyebrow>MODEL · {purposeOf(model)}</Eyebrow>
          <span className="insp__title">{model.name}</span>
          <span className="insp__sub">
            {capabilitiesOf(model).join(" / ")}
          </span>
        </span>
        <button
          className="btn btn--icon insp__x"
          type="button"
          aria-label="关闭"
          onClick={() => store.setInspector(null)}
        >
          <Icon name="x" />
        </button>
      </div>
      <div className="insp__tabs">
        {tabs.map(([key, label]) => (
          <button
            type="button"
            key={key}
            aria-selected={tab === key}
            onClick={() => setTab(key)}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="insp__body">{body}</div>
      <div className="insp__foot">
        {model.installed ? (
          <>
            <button
              className="btn"
              type="button"
              onClick={() => void toggleLoad(model)}
            >
              <Icon name="play" />
              {runtime?.state === "running" ? "卸载" : "加载"}
            </button>
            <button
              className="btn btn--ghost"
              type="button"
              onClick={() => void togglePin(model)}
            >
              <Icon name="shield" />
              {model.pinned ? "取消固定" : "固定"}
            </button>
            {confirm ? (
              <button
                className="btn btn--danger"
                type="button"
                onClick={() => {
                  setConfirm(false);
                  void uninstall(model);
                }}
              >
                确认删除
              </button>
            ) : (
              <button
                className="btn btn--ghost"
                type="button"
                onClick={() => setConfirm(true)}
              >
                删除
              </button>
            )}
          </>
        ) : (
          <button
            className="btn btn--primary"
            type="button"
            disabled={!model.compatible}
            onClick={() => void install(model)}
          >
            <Icon name="download" />
            下载
          </button>
        )}
      </div>
    </>
  );
}
