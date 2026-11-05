/**
 * Design-system primitives.
 *
 * Every class name here comes from `designs/index.html` so the React
 * implementation keeps pixel parity with the exported prototype.
 */

import type { ReactNode } from "react";

/** Icon paths, copied verbatim from the design export. */
const ICONS: Record<string, string> = {
  cube: "M12 3 20.2 7.2v9.6L12 21l-8.2-4.2V7.2z M3.8 7.2 12 11.6l8.2-4.4 M12 11.6V21",
  play: "M7.6 5v14L19 12z",
  layers:
    "M12 3.2 20.4 7.6 12 12 3.6 7.6z M3.6 12.6 12 17l8.4-4.4 M3.6 16.9 12 21.3l8.4-4.4",
  sliders:
    "M4 7h8 M16.5 7H20 M4 12h3.5 M12 12h8 M4 17h8 M16.5 17H20 M13.5 7a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0z M9 12a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0z M13.5 17a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0z",
  search:
    "M11 18.6a7.6 7.6 0 1 0 0-15.2 7.6 7.6 0 0 0 0 15.2z M20.5 20.5l-4.1-4.1",
  refresh: "M20.5 12a8.5 8.5 0 1 1-2.5-6 M20.5 4v5h-5",
  download: "M12 3.5v11 M7.6 10.4 12 14.8l4.4-4.4 M4 19.8h16",
  trash: "M4 6.6h16 M9.2 6.6V3.8h5.6v2.8 M6.2 6.6l.9 13.6h9.8l.9-13.6",
  folder:
    "M3.5 7.2a1.7 1.7 0 0 1 1.7-1.7h3.6l2 2.2h7a1.7 1.7 0 0 1 1.7 1.7v8.4a1.7 1.7 0 0 1-1.7 1.7H5.2a1.7 1.7 0 0 1-1.7-1.7z",
  plus: "M12 5v14 M5 12h14",
  x: "M6.4 6.4l11.2 11.2 M17.6 6.4 6.4 17.6",
  check: "M4.8 12.4 9.6 17.2 19.2 7.2",
  alert: "M12 3.6 21.6 20H2.4z M12 9.8v4.6 M12 17.2v.6",
  copy:
    "M9.2 9.2h9.1v9.1a1.7 1.7 0 0 1-1.7 1.7H9.2a1.7 1.7 0 0 1-1.7-1.7z M5 14.8H4.6a1.7 1.7 0 0 1-1.7-1.7V4.6a1.7 1.7 0 0 1 1.7-1.7h8.5A1.7 1.7 0 0 1 14.8 4.6v.6",
  terminal: "M4 4.8h16v14.4H4z M8 9.6l3 2.4-3 2.4 M13 15h4",
  chevron: "M6.5 9.5 12 15l5.5-5.5",
  more: "M6.2 12h.01 M12 12h.01 M17.8 12h.01",
  cpu: "M9 9h6v6H9z M9 3.4v3.2 M15 3.4v3.2 M9 17.4v3.2 M15 17.4v3.2 M3.4 9h3.2 M3.4 15h3.2 M17.4 9h3.2 M17.4 15h3.2 M6.6 6.6h10.8v10.8H6.6z",
  memory: "M3.4 7h17.2v10H3.4z M7.4 10v4 M11.4 10v4 M15.4 10v4",
  gpu: "M3 6.4h13v11.2H3z M16 10.4h4.6v3.2H16 M18.3 12h.01",
  disk: "M12 3.4a8.6 8.6 0 1 0 0 17.2 8.6 8.6 0 0 0 0-17.2z M12 7.8a4.2 4.2 0 1 0 0 8.4 4.2 4.2 0 0 0 0-8.4z",
  clock:
    "M12 3.4a8.6 8.6 0 1 0 0 17.2 8.6 8.6 0 0 0 0-17.2z M12 7.2V12l3 1.8",
  zap: "M13.4 2.6 4.8 13.4h5.4l-.6 8 8.6-10.8h-5.4z",
  mic: "M12 3.4a2.9 2.9 0 0 1 2.9 2.9v5.4a2.9 2.9 0 0 1-5.8 0V6.3A2.9 2.9 0 0 1 12 3.4z M5.6 11a6.4 6.4 0 0 0 12.8 0 M12 17.4v3.2",
  chat: "M4 5.4h16v10.2H9.6L4.8 19.6z",
  arrow: "M4.8 12h14 M13 6.2l6 5.8-6 5.8",
  info: "M12 3.4a8.6 8.6 0 1 0 0 17.2 8.6 8.6 0 0 0 0-17.2z M12 11.2v5 M12 8v.5",
  bulb: "M9.4 18.2h5.2 M10.4 21h3.2 M12 3.2a6 6 0 0 1 6 6c0 3-2 4.6-2 6H8c0-1.4-2-3-2-6a6 6 0 0 1 6-6z",
  filter: "M3.6 5.4h16.8l-6.6 7.8v6L10.2 17v-3.8z",
  sort: "M7 4.4v15.2 M7 19.6l-3-3 M7 19.6l3-3 M17 19.6V4.4 M17 4.4l-3 3 M17 4.4l3 3",
  open: "M14.2 4h5.8v5.8 M20 4l-8.6 8.6 M18.2 14v5.4a.8.8 0 0 1-.8.8H4.8a.8.8 0 0 1-.8-.8V6.6a.8.8 0 0 1 .8-.8H10",
  stop: "M7.4 7.4h9.2v9.2H7.4z",
  bolt: "M13.4 2.6 4.8 13.4h5.4l-.6 8 8.6-10.8h-5.4z",
  shield: "M12 3.2 19.6 6v6c0 4.4-3 7.6-7.6 8.8C7.4 19.6 4.4 16.4 4.4 12V6z M9 12.2l2.2 2.2 4-4.2",
  eye: "M2.8 12S6.6 5.8 12 5.8 21.2 12 21.2 12 17.4 18.2 12 18.2 2.8 12 2.8 12z M12 9.4a2.6 2.6 0 1 0 0 5.2 2.6 2.6 0 0 0 0-5.2z",
};

const FILLED = new Set(["play", "stop", "bolt", "zap"]);

/** Renders one of the design icons. */
export function Icon({
  name,
  size,
}: {
  name: string;
  size?: number;
}) {
  const d = ICONS[name];
  if (!d) return null;
  const style = size ? { "--od-icon-size": `${size}px` } as React.CSSProperties : undefined;
  if (FILLED.has(name)) {
    return (
      <svg
        className="od-icon"
        style={style}
        viewBox="0 0 24 24"
        fill="currentColor"
        stroke="none"
        aria-hidden="true"
      >
        <path d={d} />
      </svg>
    );
  }
  return (
    <svg
      className="od-icon"
      style={style}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={d} />
    </svg>
  );
}

/** A small uppercase label. */
export function Eyebrow({ children }: { children: ReactNode }) {
  return <span className="eyebrow">{children}</span>;
}

/** A status pill. */
export function Tag({
  tone = "idle",
  children,
}: {
  tone?: string;
  children: ReactNode;
}) {
  return (
    <span className={`tag tag--${tone}`}>
      <span className="tag__dot" />
      {children}
    </span>
  );
}

/** The `LLM` / `TTS` monogram. */
export function TypeMark({ type }: { type: string }) {
  return (
    <span className="tmark" data-type={type}>
      {type}
    </span>
  );
}

/** One row of a key/value list. */
export function Kv({
  k,
  v,
  mono,
}: {
  k: string;
  v: ReactNode;
  mono?: boolean;
}) {
  return (
    <>
      <span className="kv__k">{k}</span>
      <span className={`kv__v${mono ? " mono" : ""}`}>{v}</span>
    </>
  );
}

/** The header row of a sheet. */
export function SheetHead({
  eyebrow,
  title,
  meta,
  tools,
}: {
  eyebrow: string;
  title: string;
  meta?: ReactNode;
  tools?: ReactNode;
}) {
  return (
    <div className="sheet__head">
      <span className="cellstack">
        <Eyebrow>{eyebrow}</Eyebrow>
        <span className="sheet__title">{title}</span>
      </span>
      {tools ? <span className="sheet__tools">{tools}</span> : null}
      {meta ? <span className="sheet__meta">{meta}</span> : null}
    </div>
  );
}

/** The empty state of a table or panel. */
export function Empty({
  title,
  text,
  action,
}: {
  title: string;
  text?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <span className="empty__title">{title}</span>
      {text ? <span className="empty__text">{text}</span> : null}
      {action}
    </div>
  );
}

/** One fact cell of a facts strip. */
export function Fact({
  k,
  v,
  tone,
}: {
  k: string;
  v: ReactNode;
  tone?: string;
}) {
  return (
    <span className="fact">
      <Eyebrow>{k}</Eyebrow>
      <span className="fact__v" style={tone ? { color: `var(--${tone})` } : undefined}>
        {v}
      </span>
    </span>
  );
}

/** A labelled note block. */
export function Note({
  title,
  children,
  tone,
}: {
  title: string;
  children: ReactNode;
  tone?: string;
}) {
  return (
    <div className="note" data-tone={tone}>
      <h4>{title}</h4>
      {children}
    </div>
  );
}

/** A segmented progress bar. */
export function Meter({ percent, kind = "llm" }: { percent: number; kind?: string }) {
  return (
    <span className="meter">
      <span
        className="meter__seg"
        data-k={kind}
        style={{ width: `${Math.max(0, Math.min(100, percent)).toFixed(1)}%` }}
      />
    </span>
  );
}

/** The status → tone/label mapping shared by every page. */
export const STATUS_TONE: Record<string, [string, string]> = {
  running: ["ok", "运行中"],
  starting: ["info", "启动中"],
  stopped: ["idle", "已停止"],
  failed: ["bad", "失败"],
  ready: ["ok", "就绪"],
  convert: ["warn", "需转换"],
  over: ["bad", "显存不足"],
  noruntime: ["warn", "缺运行时"],
  installed: ["ok", "已安装"],
  none: ["idle", "未安装"],
  installing: ["info", "安装中"],
  downloading: ["info", "下载中"],
  done: ["ok", "已完成"],
  loaded: ["ok", "已加载"],
  loading: ["info", "加载中"],
  unloaded: ["idle", "未加载"],
};

/** A badge for the features that the backend does not expose yet. */
export function Pending({ label = "未接入" }: { label?: string }) {
  return (
    <span className="tag tag--idle">
      <span className="tag__dot" />
      {label}
    </span>
  );
}

/** Renders a status pill for a backend state. */
export function StatusTag({ state }: { state: string }) {
  const [tone, label] = STATUS_TONE[state] ?? ["idle", state];
  return (
    <Tag tone={tone}>
      {label}
    </Tag>
  );
}
