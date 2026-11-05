import { useState } from "react";
import { useStore, type Theme } from "../store";
import {
  Eyebrow,
  Icon,
  Note,
  Pending,
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

/** Reads how much space the local storage uses. */
function storageSize() {
  try {
    const raw = localStorage.getItem("lmlathe.theme");
    return raw ? `${(raw.length / 1024).toFixed(1)} KB` : "已启用 · 尚未保存";
  } catch {
    return "不可用 · 浏览器禁用了本地存储";
  }
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
            placeholder="http://127.0.0.1:8080"
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
          label="本地持久化"
          desc="主题与界面偏好保存在本机的浏览器存储里，不上传任何数据。"
        >
          <span className="mono" style={{ fontSize: "11.5px" }}>
            {storageSize()}
          </span>
        </SetRow>
      </section>

      <section className="sheet">
        <SheetHead eyebrow="STORAGE" title="存储与下载" />
        <SetRow
          label="模型目录"
          desc="所有权重文件的根目录，删除模型时只清理这个目录下的内容。"
        >
          <Pending label="未接入" />
        </SetRow>
        <SetRow
          label="运行时目录"
          desc="各推理引擎的安装位置，切换版本时只在这个目录内替换。"
        >
          <Pending label="未接入" />
        </SetRow>
        <SetRow
          label="下载并发"
          desc="同时下载的文件数。后端目前按顺序下载，暂不支持并发配置。"
        >
          <Pending label="未接入" />
        </SetRow>
      </section>

      <section className="sheet">
        <SheetHead eyebrow="ABOUT" title="关于" />
        <SetRow label="应用" desc="LM Lathe 桌面端，管理本机上的 TTS 模型与运行时。">
          <span className="mono" style={{ fontSize: "11.5px" }}>
            v0.1.0
          </span>
        </SetRow>
        <SetRow label="注册模型" desc="本地服务已知的模型数量。">
          <span className="mono" style={{ fontSize: "11.5px" }}>
            {store.models.length}
          </span>
        </SetRow>
        <SetRow label="设计系统" desc="界面遵循 designs/index.html 的纸面工作台规范。">
          <Eyebrow>PAPER WORKBENCH · V2</Eyebrow>
        </SetRow>
      </section>
    </>
  );
}
