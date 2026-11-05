# LM-Lathe · 桌面应用

Tauri 2 外壳，给本地 AI 服务套一层图形界面：模型、运行时、服务、设置四个页面。

| 页面 | 内容 |
|---|---|
| 模型 | 发现模型（Hugging Face 搜索与预置列表）、已下载的模型、下载队列、加载 / 卸载 / 固定 / 删除 |
| 运行时 | 4 个推理引擎的版本、安装、切版本、卸载、健康检查；运行实例与内存调度 |
| 服务 | 本地网关的状态、路由数量、运行中的实例与日志 |
| 设置 | 后端地址、监听地址、主题、目录、下载并发 |

## 运行

```bash
npm install
npm run tauri:dev      # 开发模式（Vite + Tauri 窗口）
npm run tauri:build    # 打包成 LM-Lathe.app
```

两个命令都会先跑 `npm run sidecar`：先把后端编译一遍（没改动时 1 秒），再把 `lm-lathe-server` 复制到 `src-tauri/binaries/`，按 Tauri 的命名规则。所以改了后端代码，直接跑这两个命令就行。

```bash
npm run build          # 只构建前端（tsc --noEmit + vite build）
npm run preview        # 只起前端，用浏览器调试
npm run icon           # 用脚本生成图标
```

## 结构

```
src/
├── api.ts               与 HTTP API 的类型化客户端
├── App.tsx              外壳：标题栏、导航、主题
├── store.tsx            全局状态：连接、模型、实例、下载队列、活动日志
├── ui.tsx               基础组件：Icon、Tag、SheetHead、Modal 等
├── pages/
│   ├── Models.tsx       模型
│   ├── Runtimes.tsx     运行时
│   ├── Services.tsx     服务
│   └── Settings.tsx     设置
└── styles.css           样式（纸面工作台设计系统）
src-tauri/
├── src/lib.rs           Tauri 入口
├── src/server.rs        后端进程的生命周期
├── tauri.conf.json      窗口、CSP 与打包配置
└── capabilities/        权限（本应用不需要额外权限）
```

## 后端进程

窗口打开时，Rust 侧通过 `start_server` 命令启动 `lm-lathe-server`：

1. 在 `54321`（`LMLATHE_PORT` 可改）上找端口；已经被**同一份构建**占着就复用它，
2. 否则以后台进程启动 `lm-lathe-server serve --host <绑定地址> --port <端口>`，
3. 等端口开始接受连接，把 Base URL 返回给前端。

后端通过标准输入管道跟随宿主进程的生命周期：窗口关闭、宿主退出或进程收到 `SIGTERM` 时，管道关闭，后端随之退出，不会留下孤儿进程。

二进制按以下顺序查找：

1. 环境变量 `TTS_SERVER_BIN`，
2. 打包进 `.app` 的 sidecar（已安装的应用优先用它自己带的那一份），
3. 工作区的 `target/release/lm-lathe-server` 或 `target/debug/lm-lathe-server`（按修改时间取最新的）。

后端的输出写入 `~/.lm-lathe/logs/tts/desktop-server.log`；数据目录可以用 `LM_LATHE_HOME` 改变。

## 浏览器中的调试

前端不依赖 Tauri 也能运行：`npm run preview` 之后在设置页填入后端地址（例如 `http://127.0.0.1:54321`）。浏览器里改不了监听地址——那需要重启后端，只有桌面端能做。
