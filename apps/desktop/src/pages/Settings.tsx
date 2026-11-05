import { useState } from "react";
import { version } from "../../package.json";
import { useStore, type Theme } from "../store";
import {
  Icon,
  Note,
  SheetHead,
  Tag,
} from "../ui";

/** One row of the settings sheet. */
function SetRow({
  label,
  desc,
  children,
}: {
  label: string;
  desc: string;
  children: React.ReactNode;
}) {
  return (
    <div className="setrow">
      <span className="cellstack">
        <span className="setrow__label">{label}</span>
        <span className="setrow__desc">{desc}</span>
      </span>
      <span className="setrow__ctl">{children}</span>
    </div>
  );
}

const THEMES: [Theme, string, string][] = [
  ["system", "跟随系统", "与 macOS 的外观保持一致"],
  ["light", "浅色", "纸面工作台的浅色纸底"],
  ["dark", "深色", "低对比的夜间配色"],
];

/** The preferences that the application keeps on this machine. */
function storedPreferences(): { label: string; value: string }[] {
  const labels: Record<string, string> = {
    "lmlathe.theme": "主题",
    "lmlathe.backend": "后端地址",
    "lmlathe.bind": "监听地址",
  };
  const saved: { label: string; value: string }[] = [];
  for (const [key, label] of Object.entries(labels)) {
    let value: string | null = null;
    try {
      value = localStorage.getItem(key);
    } catch {
      // The browser forbids the local storage.
      return [];
    }
    if (value) saved.push({ label, value });
  }
  return saved;
}

export default function Settings() {
  const store = useStore();
  const [url, setUrl] = useState(store.manualUrl);

  return (
    <>
      <section className="sheet">
        <SheetHead eyebrow="CONNECTION" title="本地服务连接" />
        <SetRow
          label="后端地址"
          desc="桌面端由 Rust 侧启动本地服务；浏览器调试时使用下面的地址。"
        >
          <span className="mono" style={{ fontSize: "11.5px" }}>
            {store.base ?? "未连接"}
          </span>
        </SetRow>
        <SetRow
          label="手动地址"
          desc="只有在浏览器里以开发模式运行时才需要填写。"
        >
          <input
            className="input"
            style={{ width: "220px" }}
            value={url}
            placeholder="http://127.0.0.1:54321"
            onChange={(event) => setUrl(event.target.value)}
          />
          <button
            className="btn btn--primary"
            type="button"
            disabled={store.starting}
            onClick={() => {
              store.setManualUrl(url);
              void store.connect();
            }}
          >
            <Icon name="refresh" />
            {store.starting ? "连接中…" : "连接"}
          </button>
        </SetRow>
        <SetRow
          label="监听地址"
          desc="只允许本机时别的机器访问不到这个服务；允许局域网后，同一网络里的设备可以用这个地址调用接口。"
        >
          <div className="radios">
            {(
              [
                ["127.0.0.1", "只允许本机", "服务只在这台机器上可见。"],
                ["0.0.0.0", "允许局域网", "同一网络里的其它设备也能访问。"],
              ] as [typeof store.bind, string, string][]
            ).map(([value, title, desc]) => (
              <label className="radio" key={value}>
                <input
                  type="radio"
                  name="bind"
                  checked={store.bind === value}
                  disabled={store.starting}
                  onChange={() => void store.applyBind(value)}
                />
                <span>
                  <span className="radio__title">{title}</span>
                  <span className="radio__desc">{desc}</span>
                </span>
              </label>
            ))}
          </div>
        </SetRow>
        <SetRow label="状态" desc="连接失败时服务日志会给出原因。">
          {store.error ? (
            <Tag tone="bad">异常</Tag>
          ) : store.starting ? (
            <Tag tone="info">启动中</Tag>
          ) : store.base ? (
            <Tag tone="ok">正常</Tag>
          ) : (
            <Tag tone="idle">未连接</Tag>
          )}
        </SetRow>
        {store.error ? (
          <div className="sheet__body">
            <Note title="连接失败" tone="bad">
              <p>{store.error}</p>
            </Note>
          </div>
        ) : null}
      </section>

      <section className="sheet">
        <SheetHead eyebrow="APPEARANCE" title="外观" />
        <SetRow label="主题" desc="设计系统在浅色与深色之间共用同一套结构与字阶。">
          <div className="radios">
            {THEMES.map(([value, title, desc]) => (
              <label className="radio" key={value}>
                <input
                  type="radio"
                  name="theme"
                  checked={store.theme === value}
                  onChange={() => store.setTheme(value)}
                />
                <span>
                  <span className="radio__title">{title}</span>
                  <span className="radio__desc">{desc}</span>
                </span>
              </label>
            ))}
          </div>
        </SetRow>
        <SetRow
          label="偏好存储"
          desc="主题、后端地址和监听地址保存在这台机器上，不上传任何数据；删除应用的数据目录会一并清除。"
        >
          {storedPreferences().length ? (
            <div className="taglist">
              {storedPreferences().map((item) => (
                <Tag key={item.label} tone="info">
                  {item.label} {item.value}
                </Tag>
              ))}
            </div>
          ) : (
            <span className="mono" style={{ fontSize: "11.5px" }}>
              还没有保存偏好
            </span>
          )}
        </SetRow>
      </section>

      <section className="sheet">
        <SheetHead eyebrow="STORAGE" title="存储与下载" />
        <SetRow
          label="模型目录"
          desc="所有权重文件的根目录，删除模型时只清理这个目录下的内容。"
        >
          <span className="mono" style={{ fontSize: "11.5px" }}>
            {store.paths?.tts_models_root ?? "未连接"}
          </span>
        </SetRow>
        <SetRow
          label="运行时目录"
          desc="各推理引擎的安装位置，切换版本时只在这个目录内替换。"
        >
          <span className="mono" style={{ fontSize: "11.5px" }}>
            {store.paths?.engines_root ?? "未连接"}
          </span>
        </SetRow>
        <SetRow
          label="下载并发"
          desc="同时下载的文件数。权重文件越大，连接数越多越快。"
        >
          <select
            className="input"
            style={{ width: "120px" }}
            value={store.paths?.download_concurrency ?? 1}
            disabled={!store.paths}
            onChange={(event) =>
              void store.setDownloadConcurrency(Number(event.target.value))
            }
          >
            {[1, 2, 3, 4, 6, 8].map((value) => (
              <option key={value} value={value}>
                {value} 个文件
              </option>
            ))}
          </select>
        </SetRow>
      </section>

      <section className="sheet">
        <SheetHead eyebrow="ABOUT" title="关于" />
        <SetRow label="应用" desc="LM-Lathe 桌面端，管理本机上的 TTS 模型与运行时。">
          <span className="mono" style={{ fontSize: "11.5px" }}>
            v{version}
          </span>
        </SetRow>
      </section>
    </>
  );
}
