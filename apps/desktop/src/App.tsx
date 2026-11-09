import { useEffect, useState } from "react";
import { StoreProvider, useStore, type View } from "./store";
import { beginWindowDrag } from "./drag";
import { Icon } from "./ui";
import Models, { ModelInspector } from "./pages/Models";
import Runtimes, { RuntimeInspector } from "./pages/Runtimes";
import Services, { ServiceInspector } from "./pages/Services";
import Settings from "./pages/Settings";
import Voices, { VoiceInspector } from "./pages/Voices";

/** The navigation entries, grouped exactly like the design export. */
const NAV: {
  id: View;
  label: string;
  icon: string;
  group: string;
  count?: (running: number) => number | null;
}[] = [
  { id: "models", label: "模型", icon: "cube", group: "资产" },
  { id: "runtime", label: "运行时", icon: "layers", group: "资产" },
  { id: "voices", label: "声音", icon: "mic", group: "资产" },
  {
    id: "services",
    label: "服务",
    icon: "play",
    group: "运行",
    count: (running) => running,
  },
  { id: "settings", label: "设置", icon: "sliders", group: "系统" },
];

const PAGES: Record<View, { title: string; sub: string }> = {
  models: { title: "模型", sub: "MODELS" },
  runtime: { title: "运行时", sub: "ENGINES" },
  voices: { title: "声音", sub: "VOICES" },
  services: { title: "服务", sub: "ENDPOINTS" },
  settings: { title: "设置", sub: "PREFERENCES" },
};

/** `true` when the frontend runs inside the Tauri webview. */
function inTauri() {
  return "__TAURI_INTERNALS__" in window;
}

function Shell() {

  const store = useStore();
  const [query, setQuery] = useState("");

  // The model search lives in the top bar, so it has to survive page switches.
  useEffect(() => {
    if (store.view !== "models") setQuery("");
  }, [store.view]);

  const running = store.instances.filter(
    (runtime) => runtime.state === "running",
  ).length;

  const groups = NAV.reduce<{ name: string; items: typeof NAV }[]>((acc, item) => {
    const last = acc[acc.length - 1];
    if (last && last.name === item.group) last.items.push(item);
    else acc.push({ name: item.group, items: [item] });
    return acc;
  }, []);

  const page = PAGES[store.view];

  return (
    <div
      className={`app${store.inspector ? " has-insp" : ""}${
        inTauri() ? " is-tauri" : ""
      }`}
    >
      {/* The bar of the window carries the name of the application and the
          address of the backend. It drags the window: on macOS the native
          drag of the system cannot be started from a webview (tauri#9503),
          so the window follows the mouse itself. */}
      <header className="titlebar" onPointerDown={beginWindowDrag}>
        <span className="lights">
          <i />
          <i />
          <i />
        </span>
        <span className="titlebar__name">LM-Lathe</span>
        <span className="titlebar__ctx">{store.base ?? "未连接"}</span>
      </header>

      <aside className="rail">
        <nav>
          {groups.map((group) => (
            <div className="rail__grp" key={group.name}>
              <span className="rail__label eyebrow">{group.name}</span>
              {group.items.map((item) => {
                const count = item.count?.(running) ?? null;
                return (
                  <button
                    className="rail__item"
                    type="button"
                    key={item.id}
                    aria-current={store.view === item.id ? "page" : "false"}
                    title={item.label}
                    onClick={() => store.setView(item.id)}
                  >
                    <Icon name={item.icon} />
                    <span>{item.label}</span>
                    {count !== null ? (
                      <span className="rail__count">{count}</span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          ))}
        </nav>
        <div className="rail__foot">
          <span className="rail__status">
            <span
              className="rail__dot"
              data-tone={store.error ? "bad" : store.starting ? "warn" : undefined}
            />
            <span>
              {store.error
                ? "daemon · 异常"
                : store.starting
                  ? "daemon · 启动中"
                  : store.base
                    ? "daemon · 正常"
                    : "daemon · 未连接"}
            </span>
          </span>
        </div>
      </aside>

      <main className="main">
        {/* The bar of the page drags the window as well, so that it can be
            moved from the place the eye already is. The buttons and the
            input inside it keep their clicks. */}
        <div className="topbar" onPointerDown={beginWindowDrag}>
          <span className="topbar__title">{page.title}</span>
          <span className="topbar__sub">{page.sub}</span>
          <span className="od-fill" />
          <span className="topbar__tools">
            {store.view === "models" ? (
              <span className="search">
                <Icon name="search" />
                <input
                  type="search"
                  value={query}
                  placeholder="搜索模型…"
                  aria-label="搜索模型"
                  onChange={(event) => setQuery(event.target.value)}
                />
              </span>
            ) : (
              <button
                className="btn btn--ghost"
                type="button"
                onClick={() => void store.refresh()}
              >
                <Icon name="refresh" />
                刷新
              </button>
            )}
          </span>
        </div>
        <div className="scroll">
          <div className="view">
            {store.view === "models" && (
              <Models query={query} onQueryChange={setQuery} />
            )}
            {store.view === "runtime" && <Runtimes />}
            {store.view === "voices" && <Voices />}
            {store.view === "services" && <Services />}
            {store.view === "settings" && <Settings />}
          </div>
        </div>
      </main>

      {store.inspector ? (
        <aside className="insp">
          {store.inspector.kind === "model" && (
            <ModelInspector id={store.inspector.id} tab={store.inspector.tab} />
          )}
          {store.inspector.kind === "runtime" && (
            // The key makes the inspector start again when another engine
            // is picked: the versions of the old one would stay in the list.
            <RuntimeInspector
              key={store.inspector.id}
              id={store.inspector.id}
              tab={store.inspector.tab}
            />
          )}
          {store.inspector.kind === "voice" && (
            <VoiceInspector id={store.inspector.id} tab={store.inspector.tab} />
          )}
          {store.inspector.kind === "service" && (
            <ServiceInspector id={store.inspector.id} tab={store.inspector.tab} />
          )}
        </aside>
      ) : null}

      <div className="toasts">
        {store.toasts.map((toast) => (
          <div className="toast" data-tone={toast.tone} key={toast.id}>
            {toast.text}
          </div>
        ))}
      </div>
    </div>
  );
}

export default function App() {
  return (
    <StoreProvider>
      <Shell />
    </StoreProvider>
  );
}
