/**
 * Application store.
 *
 * Owns the connection to the local `tts-server`, the shared model/runtime
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
  type Model,
  type Paths,
  type Route,
  type Runtime,
  type Status,
} from "./api";

/** The four top level views of the application. */
export type View = "models" | "runtime" | "services" | "settings";

/** What the inspector shows. */
export type Inspector =
  | { kind: "model"; id: string; tab: string }
  | { kind: "runtime"; id: string; tab: string }
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
  routes: Route[];
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

const VIEWS: View[] = ["models", "runtime", "services", "settings"];

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
  const [routes, setRoutes] = useState<Route[]>([]);
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

  const refresh = useCallback(async () => {
    try {
      const [nextModels, nextStatus, nextEngines, nextRoutes, nextPaths] =
        await Promise.all([
          api.models(),
          api.status(),
          api.engines().catch(() => []),
          api.routes().catch(() => []),
          api.paths().catch(() => null),
        ]);
      setModels(nextModels);
      setStatus(nextStatus);
      setEngines(nextEngines);
      setRoutes(nextRoutes);
      setPaths(nextPaths);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

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
                      state: progress.state,
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
              ? { ...item, percent: 0, bytes: 0, state: "running", error: undefined }
              : item,
          ),
        );
        log("info", `重新下载 ${job.name}`);
        notify(`正在重试：${job.name}`);
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
      routes,
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
      routes,
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
