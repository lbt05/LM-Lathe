# 本地 AI 工作台 · 桌面应用

Tauri 2 桌面外壳，为 `todo.md` §9 的三个页面提供界面：

| 页面 | 规格 | 内容 |
|---|---|---|
| TTS 模型中心 | §9.1 | 名称、版本、大小、格式、预计内存、运行状态、Load / Unload、Pin、安装状态与下载、日志 |
| 声音库 | §9.2 | 名称、描述、关联模型、样本播放器、创建时间、编辑与删除，按模型与语言筛选 |
| 声音设计工作台 | §9.3 | 自然语言描述 → 引擎 → 模型 → 生成样本 → 试听评价 → 调整 Prompt → 保存 Voice Profile，支持多模型对比 |

## 运行

```bash
npm install
npm run tauri:dev      # 开发模式（Vite + Tauri 窗口）
npm run tauri:build    # 打包成 Local AI Manager.app
```

`npm run tauri:build` 会先执行 `npm run sidecar`：它编译工作区里的
`tts-server`，并按 Tauri 的命名规则复制到 `src-tauri/binaries/`。

## 结构

```
src/
├── api.ts               与 HTTP API 的类型化客户端
├── App.tsx              外壳：标签页、服务启动、状态
├── pages/
│   ├── ModelHub.tsx     TTS 模型中心
│   ├── VoiceLibrary.tsx 声音库
│   └── VoiceStudio.tsx  声音设计工作台
└── styles.css           样式
src-tauri/
├── src/lib.rs           Tauri 入口
├── src/server.rs        后端进程的生命周期
├── tauri.conf.json      窗口、CSP 与打包配置
└── capabilities/        权限（本应用不需要额外权限）
```

## 后端进程

窗口打开时，Rust 侧通过 `start_server` 命令启动 `tts-server`：

1. 在 `127.0.0.1` 上取一个空闲端口，
2. 以后台进程启动 `tts-server serve --host 127.0.0.1 --port <port>`，
3. 等待端口开始接受连接，
4. 把 Base URL 返回给前端。

后端通过标准输入管道跟随宿主进程的生命周期：窗口关闭、宿主退出或进程
收到 `SIGTERM` 时，管道关闭，后端随之退出，不会留下孤儿进程。

二进制按以下顺序查找：

1. 环境变量 `TTS_SERVER_BIN`，
2. 工作区的 `target/release/tts-server` 或 `target/debug/tts-server`，
3. 打包进 `.app` 的 sidecar。

后端的输出写入 `~/.local-ai-manager/logs/tts/desktop-server.log`；数据目录
可以用 `LOCAL_AI_MANAGER_HOME` 改变。窗口关闭时后端随之退出。

## 浏览器中的调试

前端不依赖 Tauri 也能运行：`npm run dev` 之后在页面左下角填入
`http://127.0.0.1:8080`（由 `tts-server serve` 提供）即可连接已有后端。
