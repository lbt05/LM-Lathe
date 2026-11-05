import { useEffect, useState } from "react";
import { api, formatBytes } from "../api";
import { useStore } from "../store";
import {
  Empty,
  Eyebrow,
  Fact,
  Icon,
  Kv,
  Note,
  Pending,
  SheetHead,
  StatusTag,
  Tag,
  TypeMark,
} from "../ui";

/** The HTTP surface the local service exposes. */
export default function Services() {
  const store = useStore();
  const runtimes = store.status?.runtimes ?? [];
  const running = runtimes.filter((runtime) => runtime.state === "running");
  // The route table comes from the gateway itself, so it cannot drift away
  // from the routes that are really served.
  const routes = store.routes;
  const modelOf = (id: string) =>
    store.models.find((model) => model.id === id)?.name ?? id;

  return (
    <>
      <section className="sheet">
        <SheetHead
          eyebrow="GATEWAY"
          title="本地服务"
          meta={store.base ? "运行中" : "已停止"}
          tools={
            <button
              className="btn"
              type="button"
              onClick={() => store.setView("settings")}
            >
              服务设置
            </button>
          }
        />
        <div className="facts">
          <Fact
            k="STATE"
            v={
              store.base ? (
                <Tag tone="ok">运行中</Tag>
              ) : (
                <Tag tone="idle">已停止</Tag>
              )
            }
          />
          <Fact k="BIND" v={store.base ?? "—"} />
          <Fact k="ROUTES" v={String(routes.length)} />
          <Fact k="INSTANCES" v={String(running.length)} />
          <Fact
            k="AUTH"
            v={<Pending label="网关鉴权未接入" />}
          />
        </div>
        <div className="sheet__body" style={{ paddingTop: 0 }}>
          {routes.length ? (
            <div className="tablewrap">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>方法</th>
                    <th>路径</th>
                    <th>说明</th>
                  </tr>
                </thead>
                <tbody>
                  {routes.map((route) => (
                    <tr key={`${route.method} ${route.path}`}>
                      <td className="mono">{route.method}</td>
                      <td className="mono">{route.path}</td>
                      <td>{route.summary}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <Empty
              title="还没有路由信息"
              text={store.base ? "网关没有报告路由表。" : "尚未连接本地服务"}
            />
          )}
        </div>
      </section>

      <section className="sheet">
        <SheetHead
          eyebrow="INSTANCES"
          title="服务实例"
          meta={`${runtimes.length} 个 · 在模型页创建`}
        />
        {runtimes.length ? (
          <div className="tablewrap">
            <table className="tbl">
              <thead>
                <tr>
                  <th>实例</th>
                  <th>运行时</th>
                  <th>端点</th>
                  <th className="num">显存</th>
                  <th className="num">活动请求</th>
                  <th>状态</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {runtimes.map((runtime) => (
                  <tr
                    key={runtime.model_id}
                    data-act="pick"
                    tabIndex={0}
                    aria-selected={
                      store.inspector?.kind === "service" &&
                      store.inspector.id === runtime.model_id
                    }
                    onClick={() =>
                      store.setInspector({
                        kind: "service",
                        id: runtime.model_id,
                        tab: "overview",
                      })
                    }
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        store.setInspector({
                          kind: "service",
                          id: runtime.model_id,
                          tab: "overview",
                        });
                      }
                    }}
                  >
                    <td>
                      <span className="cellrow">
                        <TypeMark type="TTS" />
                        <span className="cellstack">
                          <span className="name">
                            {modelOf(runtime.model_id)}
                          </span>
                          <span className="sub">{runtime.model_id}</span>
                        </span>
                      </span>
                    </td>
                    <td>
                      <span className="sub">{runtime.engine_id}</span>
                    </td>
                    <td>
                      <span className="sub">{runtime.endpoint ?? "—"}</span>
                    </td>
                    <td className="num">
                      {formatBytes(
                        runtime.actual_memory || runtime.estimated_memory,
                      )}
                    </td>
                    <td className="num">{runtime.active_requests}</td>
                    <td>
                      <StatusTag state={runtime.state} />
                    </td>
                    <td>
                      <div className="tbl__actions">
                        <button
                          className="btn btn--icon"
                          type="button"
                          title="查看日志"
                          aria-label="查看日志"
                          onClick={(event) => {
                            event.stopPropagation();
                            store.setInspector({
                              kind: "service",
                              id: runtime.model_id,
                              tab: "logs",
                            });
                          }}
                        >
                          <Icon name="terminal" />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <Empty
            title="还没有服务实例"
            text="在模型页加载一个模型，这里会出现对应的实例。"
          />
        )}
      </section>

      <section className="sheet">
        <SheetHead eyebrow="ACTIVITY" title="事件流" meta={`最近 ${store.events.length} 条`} />
        {store.events.length ? (
          <div className="events">
            {store.events.map((event, index) => (
              <div className="event" key={`${event.time}-${index}`}>
                <span className="event__time">{event.time}</span>
                <span className="event__dot" data-tone={event.tone} />
                <span className="event__text">{event.text}</span>
              </div>
            ))}
          </div>
        ) : (
          <Empty
            title="暂无事件"
            text="安装、加载与停止模型时，这里会记录操作流水。"
          />
        )}
      </section>
    </>
  );
}

/** The right hand panel for one running instance. */
export function ServiceInspector({ id, tab }: { id: string; tab: string }) {
  const store = useStore();
  const runtime = (store.status?.runtimes ?? []).find(
    (item) => item.model_id === id,
  );
  const model = store.models.find((item) => item.id === id);
  const [logs, setLogs] = useState<string | null>(null);

  useEffect(() => {
    if (tab !== "logs" || logs !== null) return;
    void api
      .modelLogs(id)
      .then(setLogs)
      .catch(() => setLogs("无法读取日志"));
  }, [tab, logs, id]);

  if (!runtime) {
    return (
      <div className="insp__head">
        <span className="cellstack">
          <Eyebrow>INSTANCE</Eyebrow>
          <span className="insp__title">实例不存在</span>
        </span>
      </div>
    );
  }

  const tabs: [string, string][] = [
    ["overview", "概览"],
    ["logs", "日志"],
  ];

  const setTab = (next: string) =>
    store.setInspector({ kind: "service", id, tab: next });

  const body =
    tab === "logs" ? (
      <div className="term">
        {logs === null ? (
          <span className="term__line">正在读取日志…</span>
        ) : logs.trim() ? (
          logs
            .split("\n")
            .map((line, index) => (
              <span className="term__line" key={index}>
                {line}
              </span>
            ))
        ) : (
          <span className="term__line">暂无日志输出</span>
        )}
      </div>
    ) : (
      <>
        <div className="cellrow">
          <TypeMark type="TTS" />
          <StatusTag state={runtime.state} />
        </div>
        <div className="kv">
          <Kv k="模型" v={model?.name ?? id} />
          <Kv k="运行时" v={runtime.engine_id} mono />
          <Kv k="端点" v={runtime.endpoint ?? "—"} mono />
          <Kv k="进程" v={runtime.process_id ?? "—"} mono />
          <Kv k="显存" v={formatBytes(runtime.actual_memory || runtime.estimated_memory)} mono />
          <Kv k="活动请求" v={String(runtime.active_requests)} mono />
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
        {model ? (
          <div className="cellrow">
            <button
              className="btn"
              type="button"
              onClick={() =>
                store.setInspector({ kind: "model", id, tab: "overview" })
              }
            >
              <Icon name="cube" />
              打开模型
            </button>
            <span className="sub">启停操作在模型详情的底部</span>
          </div>
        ) : null}
        <Note title="接入方式">
          <p className="mono">
            POST {store.base ?? "http://127.0.0.1:54321"}/v1/audio/speech
          </p>
        </Note>
      </>
    );

  return (
    <>
      <div className="insp__head">
        <span className="cellstack">
          <Eyebrow>INSTANCE</Eyebrow>
          <span className="insp__title">{model?.name ?? id}</span>
          <span className="insp__sub">{runtime.engine_id}</span>
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
    </>
  );
}
