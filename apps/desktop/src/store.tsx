/**
 * Application store.
 *
 * Owns the connection to the local `lm-lathe-server`, the shared model/runtime
 * state, the inspector selection, the activity log and the toast queue.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  api,
  setBaseUrl,
  type EngineAsset,
  type EngineInstallProgress,
  type Install,
  type Model,
  type Paths,
  type Route,
  type Runtime,
  type Status,
  type Voice,
} from "./api";

/** The five top level views of the application. */
export type View = "models" | "runtime" | "voices" | "services" | "settings";

/** What the inspector shows. */
export type Inspector =
  | { kind: "model"; id: string; tab: string }
  | { kind: "runtime"; id: string; tab: string }
  | { kind: "voice"; id: string; tab: string }
  | { kind: "service"; id: string; tab: string }
  | null;

/** One entry of the activity log. */
export interface ActivityEvent {
  tone: "ok" | "warn" | "bad" | "info";
  time: string;
  text: string;
}

/** A download in flight. */
export interface InstallJob {
  id: string;
  name: string;
  percent: number;
  bytes: number;
  bytesTotal: number;
  state: string;
  error?: string;
}

/** A transient message in the corner. */
export interface Toast {
  id: number;
  text: string;
  tone?: "ok" | "bad";
}

/** The theme preference. */
export type Theme = "system" | "light" | "dark";

/** The address the local service listens on. */
export type BindAddress = "127.0.0.1" | "0.0.0.0";

interface Store {
  view: View;
  setView: (view: View) => void;

  base: string | null;
  error: string | null;
  starting: boolean;
  manualUrl: string;
  setManualUrl: (url: string) => void;
  /** The address the local service listens on. */
  bind: BindAddress;
  setBind: (bind: BindAddress) => void;
  /** Changes the address and restarts the local service. */
  applyBind: (bind: BindAddress) => Promise<void>;
  connect: (wanted?: BindAddress) => Promise<void>;

  /** The directories of the local service and its download settings. */
  paths: Paths | null;
  setDownloadConcurrency: (value: number) => Promise<void>;

  models: Model[];
  status: Status | null;
  engines: EngineAsset[];
  /** The progress of the engine installations that ran on this machine. */
  engineProgress: Record<string, EngineInstallProgress>;
  /** The engines whose installation runs at the moment. */
  installing: string[];
  /** Starts the installation of an engine version. */
  startEngineInstall: (id: string, version?: string) => Promise<void>;
  routes: Route[];
  /** The voice profiles that are stored on the disk. */
  voices: Voice[];
  refresh: () => Promise<void>;
  /** Every running model, speech and language alike. */
  instances: Runtime[];
  runtimeOf: (modelId: string) => Runtime | undefined;

  inspector: Inspector;
  setInspector: (inspector: Inspector) => void;

  events: ActivityEvent[];
  log: (tone: ActivityEvent["tone"], text: string) => void;

  toasts: Toast[];
  notify: (text: string, tone?: "ok" | "bad") => void;

  queue: InstallJob[];
  enqueue: (job: InstallJob) => void;
  retryInstall: (job: InstallJob) => void;
  cancelInstall: (job: InstallJob) => void;
  discardInstall: (job: InstallJob) => void;

  theme: Theme;
  setTheme: (theme: Theme) => void;
}

const StoreContext = createContext<Store | null>(null);

/** `true` when the frontend runs inside the Tauri webview. */
function inTauri() {
  return "__TAURI_INTERNALS__" in window;
}

const THEME_KEY = "lmlathe.theme";
const BACKEND_KEY = "lmlathe.backend";
const BIND_KEY = "lmlathe.bind";

const VIEWS: View[] = ["models", "runtime", "voices", "services", "settings"];

/** Reads the view from the location hash, e.g. `#/services`. */
function viewFromHash(): View {
  const hash = window.location.hash.replace(/^#\/?/, "");
  return (VIEWS as string[]).includes(hash) ? (hash as View) : "models";
}

export function StoreProvider({ children }: { children: ReactNode }) {
  const [view, setViewState] = useState<View>(() => viewFromHash());
  const [base, setBase] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [manualUrl, setManualUrlState] = useState(
    () => localStorage.getItem(BACKEND_KEY) ?? "http://127.0.0.1:54321",
  );
  // The address of the local service survives a reload, so it does not have
  // to be typed again after every restart.
  const setManualUrl = useCallback((url: string) => {
    setManualUrlState(url);
    localStorage.setItem(BACKEND_KEY, url);
  }, []);
  const [models, setModels] = useState<Model[]>([]);
  const [paths, setPaths] = useState<Paths | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [engines, setEngines] = useState<EngineAsset[]>([]);
  const [engineProgress, setEngineProgress] = useState<
    Record<string, EngineInstallProgress>
  >({});
  const [installing, setInstalling] = useState<string[]>([]);
  const [routes, setRoutes] = useState<Route[]>([]);
  const [voices, setVoices] = useState<Voice[]>([]);
  const [inspector, setInspector] = useState<Inspector>(null);
  const [events, setEvents] = useState<ActivityEvent[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [queue, setQueue] = useState<InstallJob[]>([]);
  const [theme, setThemeState] = useState<Theme>(
    () => (localStorage.getItem(THEME_KEY) as Theme | null) ?? "system",
  );
  // The address the service listens on. It survives a reload, like the
  // other preferences.
  const [bind, setBindState] = useState<BindAddress>(
    () => (localStorage.getItem(BIND_KEY) as BindAddress | null) ?? "127.0.0.1",
  );
  const setBind = useCallback((next: BindAddress) => {
    setBindState(next);
    localStorage.setItem(BIND_KEY, next);
  }, []);

  const toastId = useRef(0);

  const setView = useCallback((next: View) => {
    setViewState(next);
    window.location.hash = `#/${next}`;
  }, []);

  useEffect(() => {
    const onHashChange = () => setViewState(viewFromHash());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const notify = useCallback((text: string, tone?: "ok" | "bad") => {
    const id = (toastId.current += 1);
    setToasts((list) => [...list, { id, text, tone }]);
    window.setTimeout(() => {
      setToasts((list) => list.filter((toast) => toast.id !== id));
    }, 3200);
  }, []);

  const log = useCallback((tone: ActivityEvent["tone"], text: string) => {
    const now = new Date();
    const time = now.toLocaleTimeString("zh-CN", {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
    setEvents((list) =>
      [{ tone, time, text }, ...list].slice(0, 60),
    );
  }, []);

  const missingRef = useRef<string | null>(null);

  // The parts of the service that are not needed for the speech are not
  // vital: when they do not answer, the application says so instead of
  // showing an empty list as if nothing was there.
  const reportMissing = useCallback(
    (what: string, err: unknown) => {
      const message = err instanceof Error ? err.message : String(err);
      if (missingRef.current === message) return;
      missingRef.current = message;
      log("warn", `${what}：${message}`);
    },
    [log],
  );

  const refresh = useCallback(async () => {
    try {
      const [
        nextModels,
        nextStatus,
        nextEngines,
        nextRoutes,
        nextPaths,
        nextVoices,
      ] =
        await Promise.all([
          api.models(),
          api.status(),
          api.engines().catch((err) => {
            reportMissing("运行时列表", err);
            return [] as EngineAsset[];
          }),
          api.routes().catch((err) => {
            reportMissing("路由表", err);
            return [] as Route[];
          }),
          api.paths().catch((err) => {
            reportMissing("目录设置", err);
            return null as Paths | null;
          }),
          api.voices().catch((err) => {
            reportMissing("声音库", err);
            return [] as Voice[];
          }),
        ]);
      setModels(nextModels);
      setStatus(nextStatus);
      setEngines(nextEngines);
      setRoutes(nextRoutes);
      setPaths(nextPaths);
      setVoices(nextVoices);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [reportMissing]);

  const startEngineInstall = useCallback(
    async (id: string, version?: string) => {
      const name = engines.find((engine) => engine.id === id)?.name ?? id;
      try {
        await api.installEngine(id, version);
        setInstalling((current) =>
          current.includes(id) ? current : [...current, id],
        );
        setEngineProgress((current) => ({
          ...current,
          [id]: {
            engine_id: id,
            version: version ?? "",
            state: "running",
            percent: 0,
            bytes: 0,
            total_bytes: null,
            message: "准备安装",
            log: [],
          },
        }));
        log("info", `开始安装 ${name}${version ? ` ${version}` : ""}`);
        notify(`开始安装 ${name}`);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log("bad", `安装失败：${message}`);
        notify(message, "bad");
      }
    },
    [engines, log, notify],
  );

  // The installation of an engine runs in the backend. Its progress is
  // polled while it runs, and the list of the engines is refreshed when it
  // is finished. The poll only asks for the engines that are being
  // installed: a request per engine and per second would keep the service
  // busy for nothing.
  const installingKey = installing.join(",");
  useEffect(() => {
    if (!installingKey) return;
    let cancelled = false;
    const ids = installingKey.split(",").filter(Boolean);
    const tick = async () => {
      const entries = await Promise.all(
        ids.map(async (id) => {
          try {
            return await api.engineInstallProgress(id);
          } catch {
            return null;
          }
        }),
      );
      if (cancelled) return;
      const next: Record<string, EngineInstallProgress> = {};
      for (const entry of entries) if (entry) next[entry.engine_id] = entry;
      if (Object.keys(next).length) {
        setEngineProgress((current) => ({ ...current, ...next }));
      }
      // A finished installation is not polled any more: the error of a
      // failed one is shown once, and the list of the engines is refreshed.
      const finished = entries.filter(
        (entry) =>
          entry && (entry.state === "done" || entry.state === "failed"),
      );
      // The service does not answer any more: the installation cannot
      // continue, so the engine is not watched any longer.
      const lost = ids.filter(
        (id) => !entries.some((entry) => entry?.engine_id === id),
      );
      if (lost.length) {
        log("warn", `安装进度中断：${lost.join("、")}`);
        setInstalling((current) =>
          current.filter((id) => !lost.includes(id)),
        );
      }
      if (!finished.length) return;
      setInstalling((current) =>
        current.filter(
          (id) => !finished.some((entry) => entry?.engine_id === id),
        ),
      );
      for (const entry of finished) {
        if (!entry) continue;
        if (entry.state === "done") {
          log("ok", `${entry.engine_id} ${entry.version} 安装完成`);
          notify(`${entry.engine_id} ${entry.version} 安装完成`, "ok");
        } else {
          const message = entry.error ?? entry.message;
          log("bad", `${entry.engine_id} 安装失败：${message}`);
          notify(message, "bad");
        }
      }
      await refresh();
    };
    void tick();
    const timer = window.setInterval(tick, 1200);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [installingKey, log, notify, refresh]);

  const connect = useCallback(
    async (wanted?: BindAddress) => {
      setStarting(true);
      setError(null);
      try {
        let url: string;
        if (inTauri()) {
          url = await invoke<string>("start_server", { host: wanted ?? bind });
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
        const [nextModels, nextStatus] = await Promise.all([
          api.models(),
          api.status(),
        ]);
        setModels(nextModels);
        setStatus(nextStatus);
        setEngines(await api.engines().catch(() => []));
        setRoutes(await api.routes().catch(() => []));
        setPaths(await api.paths().catch(() => null));
        setVoices(await api.voices().catch(() => []));
        log("ok", `已连接本地服务 ${url}`);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setStarting(false);
      }
    },
    [manualUrl, bind, log],
  );

  // The address the service listens on belongs to the service, so the
  // service is restarted when it changes.
  const applyBind = useCallback(
    async (next: BindAddress) => {
      setBind(next);
      if (!inTauri()) {
        notify("浏览器模式下的监听地址由启动后端的人决定。");
        return;
      }
      try {
        await invoke("stop_server");
      } catch (err) {
        // The service was not running; it starts right away.
      }
      setBase(null);
      await connect(next);
    },
    [setBind, connect, notify],
  );

  // The number of the files that are fetched at the same time belongs to
  // the service, so the change is sent there.
  const setDownloadConcurrency = useCallback(
    async (value: number) => {
      try {
        const answer = await api.setDownloadConcurrency(value);
        setPaths((current) =>
          current ? { ...current, download_concurrency: answer.concurrency } : current,
        );
        log("ok", `同时下载的文件数改为 ${answer.concurrency}`);
      } catch (err) {
        notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [log, notify],
  );

  useEffect(() => {
    void connect();
  }, [connect]);

  // Keep the runtime table fresh while the app is open.
  useEffect(() => {
    if (!base) return;
    const timer = window.setInterval(() => {
      void api
        .status()
        .then(setStatus)
        .catch(() => undefined);
    }, 5000);
    return () => window.clearInterval(timer);
  }, [base]);

  // Poll the download queue while something is installing.
  const queueRef = useRef<InstallJob[]>([]);
  queueRef.current = queue;

  useEffect(() => {
    if (!queue.length) return;
    const timer = window.setInterval(() => {
      for (const job of queueRef.current) {
        void api
          .installProgress(job.id)
          .then((progress) => {
            // The progress is measured in bytes: a checkpoint holds a few
            // small files and one huge weight file, counting the files would
            // be meaningless.
            const percent = progress.bytes_total
              ? Math.min(100, (progress.bytes / progress.bytes_total) * 100)
              : progress.files_total
                ? (progress.files_done / progress.files_total) * 100
                : job.percent;
            if (progress.state === "done") {
              setQueue((current) => current.filter((item) => item.id !== job.id));
              log("ok", `${job.name} 安装完成`);
              notify(`${job.name} 安装完成`, "ok");
              void refresh();
              return;
            }
            if (progress.state === "cancelled") {
              // The user stopped the download: the job stays in the queue,
              // the download continues where it stopped.
              const known = queueRef.current.find((item) => item.id === job.id);
              setQueue((current) =>
                current.map((item) =>
                  item.id === job.id ? { ...item, state: "cancelled" } : item,
                ),
              );
              if (known?.state !== "cancelled") {
                log("warn", `${job.name} 下载已取消`);
              }
              return;
            }
            if (progress.state === "failed") {
              // The download failed: the job stays in the queue, so that the
              // user can repeat it. The server continues the interrupted
              // download where it stopped.
              const known = queueRef.current.find((item) => item.id === job.id);
              setQueue((current) =>
                current.map((item) =>
                  item.id === job.id
                    ? { ...item, state: "failed", error: progress.error }
                    : item,
                ),
              );
              if (known?.state !== "failed") {
                log(
                  "bad",
                  `${job.name} 安装失败${progress.error ? `：${progress.error}` : ""}`,
                );
                notify(`${job.name} 下载失败，可点击重试`, "bad");
                void refresh();
              }
              return;
            }
            setQueue((current) =>
              current.map((item) =>
                item.id === job.id
                  ? {
                      ...item,
                      percent,
                      bytes: progress.bytes || item.bytes,
                      bytesTotal: progress.bytes_total || item.bytesTotal,
                      // The service does not know a download that it did not
                      // start itself: the state of the queue stays.
                      state: progress.state === "idle" ? item.state : progress.state,
                    }
                  : item,
              ),
            );
          })
          .catch((err: unknown) => {
            // The backend does not answer any more: the download cannot
            // continue, the user can repeat it.
            const message = err instanceof Error ? err.message : String(err);
            const known = queueRef.current.find((item) => item.id === job.id);
            setQueue((current) =>
              current.map((item) =>
                item.id === job.id
                  ? { ...item, state: "failed", error: message }
                  : item,
              ),
            );
            if (known?.state !== "failed") {
              log("bad", `${job.name} 安装中断：${message}`);
              notify(`${job.name} 下载中断，可点击重试`, "bad");
            }
          });
      }
    }, 900);
    return () => window.clearInterval(timer);
  }, [queue.length, log, notify, refresh]);

  const retryInstall = useCallback(
    async (job: InstallJob) => {
      try {
        await api.installModel(job.id);
        setQueue((current) =>
          current.map((item) =>
            item.id === job.id
              ? { ...item, state: "running", error: undefined }
              : item,
          ),
        );
        // The bytes that are already on the disk stay: the server continues
        // the download at this byte.
        log("info", `继续下载 ${job.name}`);
        notify(`正在继续下载：${job.name}`);
      } catch (err) {
        notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [log, notify],
  );

  // The downloads that the service still knows about are shown again, also
  // after a restart of the interface: the download continues on the server
  // even when no window is open.
  const restoreQueue = useCallback(async () => {
    let installs: Install[] = [];
    try {
      installs = await api.installs();
    } catch {
      return;
    }
    const known = queueRef.current;
    const restored: InstallJob[] = installs
      .filter(
        (install) =>
          install.state === "running" ||
          install.state === "failed" ||
          install.state === "cancelled" ||
          install.state === "partial",
      )
      .map((install) => ({
        id: install.model_id,
        name: install.name,
        percent: install.bytes_total
          ? Math.min(100, (install.bytes / install.bytes_total) * 100)
          : 0,
        bytes: install.bytes,
        bytesTotal: install.bytes_total,
        state: install.state,
        error: install.error ?? undefined,
      }))
      .filter((job) => !known.some((item) => item.id === job.id));
    if (!restored.length) return;
    setQueue((current) => [
      ...current.filter((item) => !restored.some((job) => job.id === item.id)),
      ...restored,
    ]);
    log("info", `继续显示 ${restored.length} 个下载`);
  }, [log]);

  // Sobald der Dienst steht, zeigt die Warteschlange die Downloads wieder,
  // die im Hintergrund weiterlaufen.
  useEffect(() => {
    if (!base) return;
    void restoreQueue();
  }, [base, restoreQueue]);

  const cancelInstall = useCallback(
    async (job: InstallJob) => {
      try {
        await api.cancelInstall(job.id);
        setQueue((current) =>
          current.map((item) =>
            item.id === job.id ? { ...item, state: "cancelled" } : item,
          ),
        );
        log("warn", `已取消下载 ${job.name}`);
        notify(`已取消下载 ${job.name}`);
      } catch (err) {
        notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [log, notify],
  );

  const discardInstall = useCallback(
    async (job: InstallJob) => {
      try {
        await api.discardInstall(job.id);
        setQueue((current) => current.filter((item) => item.id !== job.id));
        log("warn", `已丢弃下载 ${job.name}`);
        notify(`已丢弃 ${job.name} 的残留文件`);
      } catch (err) {
        notify(err instanceof Error ? err.message : String(err), "bad");
      }
    },
    [log, notify],
  );

  const enqueue = useCallback((job: InstallJob) => {
    setQueue((current) => [
      ...current.filter((item) => item.id !== job.id),
      job,
    ]);
  }, []);

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next);
    try {
      localStorage.setItem(THEME_KEY, next);
    } catch {
      // storage is optional
    }
  }, []);

  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark =
        theme === "dark" || (theme === "system" && query.matches);
      document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
    };
    apply();
    query.addEventListener("change", apply);
    return () => query.removeEventListener("change", apply);
  }, [theme]);

  // The speech models and the language models run in different runtimes,
  // the interface only knows one list of instances. Every page reads this
  // one list, so the rail, the runtime sheet and the service sheet can
  // never disagree about what is running.
  const instances = useMemo<Runtime[]>(
    () => [...(status?.runtimes ?? []), ...(status?.llm_runtimes ?? [])],
    [status],
  );

  const runtimeOf = useCallback(
    (modelId: string) =>
      instances.find((runtime) => runtime.model_id === modelId),
    [instances],
  );

  const value = useMemo<Store>(
    () => ({
      view,
      setView,
      base,
      error,
      starting,
      manualUrl,
      setManualUrl,
      bind,
      setBind,
      applyBind,
      connect,
      paths,
      setDownloadConcurrency,
      models,
      status,
      engines,
      engineProgress,
      installing,
      startEngineInstall,
      routes,
      voices,
      refresh,
      instances,
      runtimeOf,
      inspector,
      setInspector,
      events,
      log,
      toasts,
      notify,
      queue,
      enqueue,
      retryInstall,
      cancelInstall,
      discardInstall,
      theme,
      setTheme,
    }),
    [
      view,
      base,
      error,
      starting,
      manualUrl,
      connect,
      bind,
      applyBind,
      setDownloadConcurrency,
      models,
      status,
      engines,
      engineProgress,
      installing,
      startEngineInstall,
      routes,
      voices,
      refresh,
      instances,
      runtimeOf,
      inspector,
      events,
      log,
      toasts,
      notify,
      queue,
      enqueue,
      retryInstall,
      cancelInstall,
      discardInstall,
      theme,
      setTheme,
    ],
  );

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>;
}

/** Access the application store. */
export function useStore(): Store {
  const store = useContext(StoreContext);
  if (!store) throw new Error("useStore must be used inside StoreProvider");
  return store;
}
