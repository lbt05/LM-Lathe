# LM-Lathe

本机 AI 运行时管理器。一个应用里下载、加载、运行 **TTS** 与 **LLM** 模型，用统一的内存调度器分配资源，并通过 OpenAI 兼容的 HTTP 接口对外提供语音合成与对话能力。

桌面端是 Tauri 应用（四个页面：模型 / 运行时 / 服务 / 设置），后端是 Rust workspace，配一个命令行工具。所有数据留在本机，不上传任何内容。

---

## 功能

| 领域 | 能做什么 |
|---|---|
| **模型** | 12 个内置模型（Qwen3-TTS、Fish Audio Drama 3、GGUF 语言模型），加上 Hugging Face 实时搜索——只列出当前运行时真能跑的仓库 |
| **下载** | 断点续传、**字节级**进度条、并行下载（1–8 个文件）、失败重试、删除前二次确认 |
| **运行时** | 4 个引擎当作真实资产管理：安装、切换版本、卸载、健康检查（真启动服务探 `/health`） |
| **加载** | 按需加载 / 卸载 / 固定；全局调度器按内存占用分配，LRU + TTL 自动回收 |
| **语音** | `POST /v1/audio/speech`（OpenAI 兼容），支持 Qwen3-TTS 的声音设计与克隆、Drama 3 的多角色 |
| **对话** | `POST /v1/chat/completions`、`/v1/completions`（OpenAI 兼容），llama.cpp 跑 GGUF |
| **声音库** | 声音档案存本地，附试听样本；声音设计与克隆走 API 或 CLI |
| **可观测** | 每个模型的引擎日志，下载进度，运行实例状态，全局调度快照 |

---

## 快速开始

```bash
# 1. 编译
cargo build --release

# 2. 看看这台机器能跑什么
./target/release/lm-lathe-server doctor

# 3. 下载一个模型（会先做兼容性检查）
./target/release/lm-lathe-server models install qwen3-tts-1.7b-voicedesign

# 4. 说一句话
./target/release/lm-lathe-server speak --model qwen3-tts-1.7b-voicedesign --text "你好，世界" --out hello.wav

# 5. 起服务
./target/release/lm-lathe-server serve --port 54321
```

用桌面端就三步：

```bash
cd apps/desktop
npm install
npm run tauri:dev
```

窗口会自己拉起后端、连上、显示四个页面。关窗口，后端跟着退。

---

## 构建

### 需要什么

| 依赖 | 版本 | 备注 |
|---|---|---|
| Rust | ≥ 1.82 | `rustup` |
| Node.js | ≥ 20 | 只用于桌面端的前端 |
| Xcode CLT | — | macOS 上编译 Tauri 外壳需要 |
| ffmpeg | 可选 | 只有把音频转成 `mp3/flac/opus/aac/ogg` 时才需要 |

### 后端

```bash
cargo build --release          # 产出 ./target/release/lm-lathe-server
cargo test --workspace         # 76 个测试
cargo clippy --all-targets
```

### 桌面应用

```bash
cd apps/desktop
npm install
npm run build                  # TypeScript 检查 + Vite 打包
npm run tauri:dev              # 开发模式（前端热更新）
npm run tauri:build            # 打包 .app
```

`tauri:dev` 和 `tauri:build` 会先跑 `npm run sidecar`：先把后端编译一遍（没改动时 1 秒），再复制成 sidecar 二进制。所以**改完后端直接跑这两个命令就行**，不会带着旧后端启动。

打包产物在 `apps/desktop/src-tauri/target/release/bundle/macos/LM-Lathe.app`，**前后端都在里面**：

```
LM-Lathe.app/Contents/MacOS/
├── lm-lathe          8.7 MB   窗口程序，前端（dist/）已经嵌进去
└── lm-lathe-server   8.2 MB   后端，作为 sidecar 一起打包
```

把这个 `.app` 拷到别的 Mac 上就能跑——不需要 Rust，也不需要 Node。只有模型权重需要另外下载。

---

## 使用

### 桌面应用的四个页面

| 页面 | 内容 |
|---|---|
| **模型** | *发现模型*：Hugging Face 搜索与预置列表，带参数量、文件大小、用途、格式，可排序；*已下载*：本地模型的加载 / 卸载 / 固定 / 删除。下载队列显示字节级进度，可重试 |
| **运行时** | 4 个引擎的当前版本、可安装版本、安装 / 切版本 / 卸载 / 健康检查；运行实例、内存调度快照、本机硬件 |
| **服务** | 本地网关的状态、绑定地址、路由数量、运行中的实例；实例日志每 3 秒自动刷新 |
| **设置** | 后端地址（自动或手填）、监听地址（只允许本机 / 允许局域网）、主题、模型与运行时目录、下载并发 |

深链接：`#/models`、`#/runtime`、`#/services`、`#/settings`。

### 命令行

```bash
lm-lathe-server doctor                                   # 硬件与每个模型的兼容性
lm-lathe-server models list|show|load|unload|pin|unpin|logs|install|uninstall
lm-lathe-server engines list|versions|install|use|uninstall
lm-lathe-server voices design|clone|remove
lm-lathe-server speak --model qwen3-tts-1.7b-voicedesign --text "你好" --out hi.wav
lm-lathe-server chat --model qwen3-4b-gguf --prompt "用一句话解释光合作用"
lm-lathe-server play hi.wav
lm-lathe-server status
lm-lathe-server serve --host 127.0.0.1 --port 54321
```

### HTTP 接口

```bash
# 语音合成（OpenAI 兼容）
curl -X POST http://127.0.0.1:54321/v1/audio/speech \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen3-tts-1.7b-voicedesign","input":"你好，世界"}' \
  -o hello.wav

# 对话（OpenAI 兼容）
curl -X POST http://127.0.0.1:54321/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen3-4b-gguf","messages":[{"role":"user","content":"你好"}]}'
```

全部路由（`GET /v1/routes` 返回同一份机器可读的表）：

| 方法 | 路径 | 用途 |
|---|---|---|
| `GET` | `/health` | 服务是否在运行，以及机器还剩多少内存 |
| `GET` | `/v1/routes` | 网关路由表 |
| `GET` | `/v1/paths` | 应用使用的目录与下载设置 |
| `POST` | `/v1/settings/download-concurrency` | 设置同时下载的文件数 |
| `GET` | `/v1/models` | 可以服务的模型（已下载且机器能跑） |
| `GET` | `/v1/tts/models` | 完整模型目录，含未下载的模型 |
| `GET` | `/v1/tts/status` | 运行时状态与调度器快照 |
| `POST` | `/v1/audio/speech` | OpenAI 兼容的语音合成 |
| `POST` | `/v1/chat/completions`、`/v1/completions` | OpenAI 兼容的对话 / 文本补全 |
| `POST` | `/v1/tts/speak` | 语音合成，可带引擎参数 |
| `GET`/`POST` | `/v1/tts/voices` | 声音库列表 / 新建声音 |
| `POST` | `/v1/tts/voices/design`、`/clone` | 声音设计 / 声音克隆 |
| `GET`/`PATCH`/`DELETE` | `/v1/tts/voices/{id}` | 读取 / 修改 / 删除声音 |
| `GET` | `/v1/tts/voices/{id}/samples`、`/samples/{name}` | 声音的试听样本 |
| `POST` | `/v1/tts/models/{id}/load`\|`unload`\|`pin`\|`unpin` | 运行时的生命周期 |
| `POST` | `/v1/tts/models/{id}/install` | 下载模型（`202 Accepted`） |
| `GET` | `/v1/tts/models/{id}/install` | 下载进度 |
| `POST` | `/v1/tts/models/{id}/uninstall` | 删除本地模型（运行中的会被拒绝） |
| `GET` | `/v1/tts/models/{id}/logs` | 引擎日志 |
| `GET` | `/v1/tts/discover?q=` | 在 Hugging Face 上搜索模型 |
| `POST` | `/v1/tts/discover/install` | 下载搜索到的模型 |
| `GET` | `/v1/runtimes`、`/v1/runtimes/{id}` | 引擎列表 / 单个引擎 |
| `GET` | `/v1/runtimes/{id}/versions` | 可安装的版本 |
| `POST` | `/v1/runtimes/{id}/install`\|`activate`\|`uninstall`\|`health` | 安装 / 切版本 / 卸载 / 健康检查 |
| `GET` | `/v1/runtimes/{id}/install-progress` | 引擎安装进度 |
| `GET` | `/v1/audio/{file}` | 生成的音频文件 |

---

## 数据目录

默认 `~/.lm-lathe`，用 `--data-dir` 或环境变量 `LM_LATHE_HOME` 改。

```
~/.lm-lathe/
├── models/
│   ├── llm/                 GGUF 权重
│   └── tts/                 TTS 权重
├── runtimes/engines/        引擎的安装目录 + engines.json（当前版本）
├── voices/<voice_id>/       profile.json + samples/
├── audio/generated/         生成的音频
└── logs/{tts,llm}/          引擎日志
```

网关固定监听 `54321`，`LMLATHE_PORT` 可改。端口被同一份构建占着时，桌面端直接复用它；被别的构建占着时，桌面端换到下一个空闲端口并显示自己用的地址。

---

## 模型与引擎

| 模型 | 类型 | 引擎 | 权重 | 内存 |
|---|---|---|---|---|
| `qwen3-tts-1.7b-voicedesign` | TTS | sglang-omni | 4.2 G | 8.0 G |
| `qwen3-tts-1.7b-customvoice` | TTS | sglang-omni | 4.2 G | 8.0 G |
| `qwen3-tts-1.7b-base` | TTS | sglang-omni | 4.2 G | 8.0 G |
| `qwen3-tts-0.6b-base` | TTS | sglang-omni | 2.3 G | 4.0 G |
| `qwen3-tts-0.6b-customvoice-mlx` | TTS | sglang-omni-mlx | 2.3 G | 8.0 G |
| `drama-3` | TTS | sglang-omni | 38.7 G | 45.0 G |
| `drama-3-int4` | TTS | sglang-omni | 22.4 G | 28.0 G |
| `drama-3-mlx-4bit` | TTS | sglang-omni-mlx | 22.1 G | 29.0 G |
| `drama-3-mlx-8bit` | TTS | sglang-omni-mlx | 39.3 G | 48.0 G |
| `qwen3-8b-gguf` | LLM | llama.cpp | 4.7 G | 6.0 G |
| `qwen3-4b-gguf` | LLM | llama.cpp | 2.3 G | 3.0 G |
| `llama-3.3-70b-instruct-gguf` | LLM | llama.cpp | 39.6 G | 48.0 G |

| 引擎 | 来源 | 提供 |
|---|---|---|
| `llama.cpp` | GitHub Releases（`ggml-org/llama.cpp`） | 语言模型 |
| `mlx` | PyPI（`mlx-lm`） | Apple 上的语言模型 |
| `sglang-omni` | GitHub 源码构建（`sgl-project/sglang-omni`） | Qwen3-TTS（Torch MPS 路径） |
| `sglang-omni-mlx` | GitHub 源码构建（原生 MLX 运行时） | Qwen3-TTS、Drama 3（Apple Silicon 路径） |

安装引擎就是把它下载到 `runtimes/engines/<引擎>/<版本>/`，版本可以并存，切换只改一个指针。

---

## 测试

```bash
cargo test --workspace    # 76 个测试，14 个 target
```

覆盖调度器（预留、LRU 淘汰、固定、TTL）、模型注册表与兼容性、下载（断点续传、并行、字节进度）、声音库、两个模型族的参数翻译、运行时管理器、HTTP 接口端到端。

测试用的是内置的假引擎，`cargo test` 会自动打开 `mock-engine` feature；**发布构建里没有它**。

---

## 待办清单

### 桌面应用

- [ ] 应用图标还是旧的波形图标，换成 LM-Lathe 的标志
- [ ] 声音库没有界面：声音设计与克隆只能在 API 与 CLI 里用
- [ ] 语言模型没有聊天界面：只能通过 `/v1/chat/completions` 或 `lm-lathe-server chat`
- [ ] `.app` 没有签名与公证，换机器安装时 Gatekeeper 会拦

### 模型与运行时

- [ ] Qwen3-TTS 1.7B 在 Apple Silicon 上一加载就崩（Torch MPS 的 SIGSEGV，上游 bug）；0.6B 的 MLX 转换版可用
- [ ] Drama 3 的所有变体都超过 34 GB 机器的内存；需要更小的量化或更大的机器
- [ ] MLX 变体的参数量还是 `—`（Hugging Face 上没有 safetensors 元数据）
- [ ] 命令行安装模型与引擎时没有周期进度输出，只有桌面端有
- [ ] 引擎健康检查对没有绑定模型的引擎只回答 `no-model`

### 功能缺口（`todo.md` 的 P1）

- [ ] Voice Design Studio 的界面
- [ ] 多模型声音对比（A/B）
- [ ] 音频格式转换的命令与界面（后端已经实现）
- [ ] 采样率、位深、声道数的转换入口
- [ ] 声音档案的批量管理
