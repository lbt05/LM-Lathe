# TTS 子系统技术设计规格书

**所属项目：** 本地 AI 模型资产管理与显存动态调度系统  
**适用里程碑：** Milestone 1  
**版本：** v1.0  
**状态：** Proposed

---

## 1. 设计目标

### 1.1 项目定位

TTS 子系统是本地 AI Runtime Manager 的一个核心功能模块，负责管理和运行本地语音合成模型。

M1 阶段重点支持：

- Fish Audio Drama 3
- Qwen3-TTS

系统需要提供统一的模型生命周期管理、语音合成调用接口、Voice Design 能力和声音资产管理。

### 1.2 核心设计原则

1. **统一生命周期管理**：所有 TTS 模型由统一 Runtime Manager 管理。
2. **引擎与模型解耦**：模型不直接依赖桌面应用，底层推理引擎通过 Adapter 接入。
3. **按需加载**：模型在收到使用请求时加载，闲置后自动卸载。
4. **统一资源调度**：TTS 与 LLM 共享同一个全局显存调度器。
5. **声音资产独立**：Voice Profile 不与具体推理进程绑定。
6. **完全离线运行**：模型、参考音频、生成音频和配置全部保存在本地。

---

## 2. 技术选型

### 2.1 推理服务框架

首选：**SGLang-Omni**

项目地址：

- https://github.com/sgl-project/sglang
- https://github.com/sgl-project/sglang-omni （以项目实际仓库及版本为准）

选择原因：

- 提供多模态推理服务能力。
- 支持多种 TTS 模型。
- 提供统一的服务接口。
- 可以减少不同 TTS 模型的服务部署工作。
- 能够与现有 SGLang 生态及经验复用。

注意：具体模型的支持情况、API 参数、模型版本和硬件兼容性需要以对应版本的官方文档为准。

### 2.2 M1 支持模型

| 模型 | 主要用途 | 核心能力 |
|---|---|---|
| Fish Audio Drama 3 | 高表现力语音合成 | 多角色、情绪、表演、长文本 |
| Qwen3-TTS | 声音设计及语音合成 | Voice Design、Voice Cloning、文本合成 |

#### Qwen3-TTS

重点支持：

- Voice Design：通过自然语言描述创建目标声音。
- Voice Cloning：通过参考音频创建相似声音。
- Custom Voice：使用模型支持的预设或指定说话人声音。
- Text-to-Speech：普通文本语音合成。

建议优先使用 Qwen3-TTS VoiceDesign 模型作为声音设计工作流的基础。

#### Fish Audio Drama 3

重点支持：

- 多角色语音生成。
- 情绪与表演控制。
- 角色声音参考。
- 长文本和场景化语音生成。

Drama 3 的角色生成和表演控制能力不应简单映射成 Qwen3-TTS 的 VoiceDesign 参数，需要由独立 Adapter 处理。

---

## 3. 整体架构

```text
Tauri Desktop Application
          |
          v
      TTS Module
          |
    +-----+-------------------+
    |                         |
    v                         v
Voice Library          TTS Runtime Manager
    |                         |
    |                   Global Scheduler
    |                         |
    |                   Engine Adapter
    |                         |
    |                    SGLang-Omni
    |                         |
    |              +----------+----------+
    |              |                     |
    |              v                     v
    |          Qwen3-TTS             Drama 3
    |              |                     |
    +--------------+---------------------+
                   |
                   v
             Audio Manager
                   |
                   v
             Local Storage
```

### 3.1 模块职责

| 模块 | 职责 |
|---|---|
| Voice Library | 管理声音资产、声音描述、参考音频及生成样本 |
| TTS Runtime Manager | 管理 TTS 模型加载、卸载、运行状态 |
| Global Scheduler | 统一管理 LLM、TTS 等模型的硬件资源 |
| Engine Adapter | 屏蔽不同推理引擎的启动和调用差异 |
| SGLang-Omni | 提供底层 TTS 推理服务 |
| Audio Manager | 管理音频生成结果、格式转换和播放 |

---

## 4. Runtime 管理设计

### 4.1 统一 Runtime 管理

TTS 模型不直接管理自己的显存和生命周期。

所有模型统一注册到 Runtime Manager。

例如：

```text
Runtime Manager
    |
    +-- LLM Runtime
    |      +-- llama.cpp
    |      +-- MLX
    |
    +-- TTS Runtime
           +-- SGLang-Omni
           |      +-- Qwen3-TTS
           |      +-- Drama 3
           +-- SGLang-Omni 原生 MLX（Apple Silicon）
                  +-- Drama 3
                  +-- Qwen3-TTS（0.6B CustomVoice，MLX 权重）
```

同一引擎在不同平台上的分发方式不同，由引擎资产管理器负责：

| 引擎 | 平台 | 分发 |
|---|---|---|
| llama.cpp | macOS / Linux | GitHub Release 的原生二进制 |
| MLX | macOS arm64 | PyPI 的 `mlx-lm`，装进独立虚拟环境 |
| SGLang-Omni | Linux | PyPI 的 `sglang-omni`（CUDA wheel） |
| SGLang-Omni | macOS arm64 | 仓库源码安装：`install.sh` 建 Python 3.12 环境并装 SGLang 的 MPS 版本 |
| SGLang-Omni 原生 MLX | macOS arm64 | 仓库源码构建：`build_runtime.sh` 用 CMake/Ninja 编译出各模型的 C++ 服务二进制 |

Apple Silicon 上的 TTS 模型由原生 MLX 运行时服务（`fishaudio_drama3_server`、
`qwen3_tts_server`），它不需要 CUDA、PyTorch 或 Python 运行时；发布标签里
还没有这两个服务，所以原生运行时跟随仓库的主分支。原生运行时按模型家族提供
服务二进制，模型所属的引擎 id 决定启动哪一个：`fishaudio-drama3-mlx` 用
`fishaudio_drama3_server`，`sglang-omni-mlx` 用 `qwen3_tts_server`。

在这台 Mac 上 PyTorch（MPS）路径会在加载语音 tokenizer 时触发 MPS 内核的
段错误（`Process pipeline died during startup (exit code -11)`），所以
Qwen3-TTS 在 Apple Silicon 上走原生 MLX 路径：模型目录里的
`qwen3-tts-0.6b-customvoice-mlx`（`mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-bf16`）
是原生运行时能服务的 Qwen3-TTS 权重。

TTS Runtime Manager 负责：

- 向全局调度器申请资源。
- 启动 SGLang-Omni 服务。
- 加载指定 TTS 模型。
- 检查服务健康状态。
- 维护模型运行状态。
- 执行模型卸载。
- 释放硬件资源。
- 收集推理日志。

### 4.2 模型加载策略

M1 默认采用：

**一个 SGLang-Omni 实例一次运行一个 TTS 模型。**

例如：

```text
当前状态：

SGLang-Omni
    |
    +-- Qwen3-TTS [Running]

用户请求：

Load Drama 3

调度过程：

1. 检查当前模型状态
2. 检查是否存在活动推理任务
3. 卸载 Qwen3-TTS
4. 确认进程退出及资源释放
5. 向全局调度器申请资源
6. 加载 Drama 3
7. 检查服务健康状态
8. 返回 Running
```

如果未来 SGLang-Omni 支持满足要求的多模型并行部署，可以在后续版本中扩展。

注意：一个服务实例只能运行一个模型是当前产品策略，不应将其作为推理框架的硬性技术限制。

### 4.3 生命周期状态

```text
Unloaded
    |
    v
Loading
    |
    v
Running
    |
    v
Unloading
    |
    v
Unloaded
```

异常状态：

```text
Failed
```

Pin 是运行实例的属性，不是独立生命周期状态。

```rust
struct TTSRuntime {
    model_id: String,
    engine_id: String,
    state: RuntimeState,
    pinned: bool,
    process_id: Option<u32>,
    endpoint: Option<String>,
    last_used_at: Option<DateTime<Utc>>,
    active_requests: u32,
    estimated_memory: u64,
    actual_memory: Option<u64>,
}
```

---

## 5. Engine Adapter 设计

### 5.1 设计目标

通过 Adapter 统一不同推理服务的生命周期操作和请求调用。

桌面应用不应该直接感知 SGLang-Omni 的内部实现。

### 5.2 接口定义

```rust
#[async_trait]
pub trait TTSAdapter: Send + Sync {
    async fn start(
        &self,
        config: TTSRuntimeConfig
    ) -> Result<RuntimeHandle>;

    async fn stop(
        &self,
        handle: RuntimeHandle
    ) -> Result<()>;

    async fn health_check(
        &self,
        handle: &RuntimeHandle
    ) -> Result<HealthStatus>;

    async fn synthesize(
        &self,
        request: TTSRequest
    ) -> Result<AudioOutput>;

    async fn unload_model(
        &self,
        handle: &RuntimeHandle
    ) -> Result<()>;

    async fn get_logs(
        &self,
        handle: &RuntimeHandle
    ) -> Result<String>;
}
```

具体 Adapter：

```text
TTSAdapter
    |
    +-- SGLangOmniAdapter
           |
           +-- Qwen3TTSAdapter
           |
           +-- FishDrama3Adapter
```

其中：

- SGLangOmniAdapter 负责服务进程管理。
- Qwen3TTSAdapter 负责 Qwen3-TTS 特定 API 参数转换。
- FishDrama3Adapter 负责 Drama 3 特定 API 参数转换。

实际工程中可以将服务级 Adapter 与模型级 Adapter 分开实现，避免重复管理同一个服务进程。

---

## 6. Voice Library 设计

### 6.1 设计目标

Voice Library 是独立于推理引擎的声音资产管理中心。

用户创建的声音资产应长期保存在本地，即使底层模型卸载，也不会丢失。

### 6.2 Voice Profile

每个 Voice Profile 包含：

| 字段 | 说明 |
|---|---|
| id | 唯一标识 |
| name | 声音名称 |
| description | 声音自然语言描述 |
| language | 主要语言 |
| gender_presentation | 声音性别表现 |
| age_style | 声音年龄感描述 |
| personality | 性格及情绪特征 |
| engine | 创建时使用的引擎 |
| model_id | 使用的模型 |
| created_at | 创建时间 |
| updated_at | 更新时间 |

### 6.3 引擎特定配置

不同模型的声音设计参数应独立保存。

```json
{
  "id": "voice_001",
  "name": "府中贴身婢女",
  "description": "青年女性，稚嫩、害羞、略带怯意",
  "engine": "qwen3-tts",
  "model_id": "Qwen3-TTS-12Hz-1.7B-VoiceDesign",
  "engine_config": {
    "voice_design_prompt": "自然语言声音描述"
  },
  "samples": [
    "samples/voice_001_001.wav"
  ]
}
```

对于 Drama 3：

```json
{
  "id": "voice_002",
  "name": "戏剧角色 A",
  "description": "情绪丰富，表现力强的女性角色",
  "engine": "fish-drama-3",
  "model_id": "Drama-3",
  "engine_config": {
    "character_config": {}
  },
  "samples": [
    "samples/voice_002_001.wav"
  ]
}
```

说明：示例中的 `character_config` 只是预留结构，实际字段由对应模型 API 决定。

### 6.4 声音资产目录

```text
~/.lm-lathe/
    |
    +-- models/
    |      +-- llm/
    |      +-- tts/
    |             +-- qwen3-tts/
    |             +-- drama-3/
    |
    +-- voices/
    |      +-- voice_001/
    |      |      +-- profile.json
    |      |      +-- samples/
    |      |
    |      +-- voice_002/
    |             +-- profile.json
    |             +-- samples/
    |
    +-- audio/
    |      +-- generated/
    |
    +-- logs/
```

---

## 7. TTS API 统一设计

### 7.1 统一调用入口

TTS 模块应提供统一的内部调用接口。

```rust
struct TTSRequest {
    model_id: String,
    voice_id: Option<String>,
    text: String,
    language: Option<String>,
    parameters: EngineSpecificParameters,
}
```

其中：

```rust
enum EngineSpecificParameters {
    Qwen3TTS(Qwen3TTSParameters),
    FishDrama3(FishDrama3Parameters),
}
```

模型特定参数由对应 Adapter 负责解释和转换。

### 7.2 OpenAI-compatible API

建议尽量兼容 OpenAI Audio API：

```http
POST /v1/audio/speech
```

例如：

```json
{
  "model": "qwen3-tts",
  "input": "你好，欢迎使用本地 AI 工作台。",
  "voice": "voice_001"
}
```

注意：

Voice Design、Voice Cloning、Drama 3 多角色对话等高级功能，不一定能完整映射到标准 OpenAI Speech API。

因此建议：

- 普通语音合成使用兼容接口。
- 高级声音设计使用内部扩展 API。
- 复杂多角色生成使用专用接口。

---

## 8. 全局显存调度集成

### 8.1 统一调度原则

TTS 与 LLM 共享同一个 Scheduler。

```text
Global Scheduler
    |
    +-- LLM Models
    |
    +-- TTS Models
    |
    +-- Future Diffusion Models
```

不能为 TTS 单独建立一套显存管理机制。

### 8.2 资源申请流程

```text
Load TTS Model
       |
       v
Request Memory Reservation
       |
       v
Check Available Resources
       |
       +---- Enough Memory
       |          |
       |          v
       |      Start Runtime
       |
       +---- Insufficient Memory
                  |
                  v
             Find LRU Models
                  |
                  v
             Evict Unpinned
                  |
                  v
             Recheck Resources
                  |
                  +---- Enough -> Start
                  |
                  +---- Failed -> Reject
```

### 8.3 调度约束

1. Pinned 模型不能被自动卸载。
2. 正在执行推理任务的模型不能被强制卸载。
3. 卸载前必须确认活动请求已经结束。
4. 进程退出后需要等待资源释放。
5. 无法满足资源需求时返回明确错误。
6. 资源申请必须支持超时和取消。

### 8.4 TTL 管理

默认 TTL：30 分钟。

卸载条件：

```text
pinned == false
AND active_requests == 0
AND idle_duration > TTL
```

TTL 可以由全局设置和模型级设置共同控制。

---

## 9. 桌面 UI 设计建议

TTS 模块建议分成三个主要页面。

### 9.1 TTS Model Hub

展示：

- 模型名称。
- 模型版本。
- 模型大小。
- 模型格式。
- 预计内存需求。
- 当前运行状态。
- Load / Unload。
- Pin 状态。

### 9.2 Voice Library

展示：

- 声音名称。
- 声音描述。
- 关联模型。
- 声音样本播放器。
- 创建时间。
- 编辑及删除操作。

支持按模型和语言筛选。

### 9.3 Voice Design Studio

主要工作流程：

```text
输入自然语言声音描述
          |
          v
选择 Voice Design Engine
          |
          v
选择对应模型
          |
          v
生成声音样本
          |
          v
试听和评价
          |
          v
调整 Prompt
          |
          v
保存 Voice Profile
```

建议支持同一个声音描述在不同模型上进行对比生成。

例如：

```text
Voice Description:
青年女性，稚嫩、害羞、略带怯意。

       +-------------------+
       |                   |
       v                   v
   Qwen3-TTS           Drama 3
       |                   |
       v                   v
   Sample A             Sample B
       |                   |
       +---------+---------+
                 |
                 v
             A/B 对比
```

注意：只有模型支持相应的声音设计方式时才开放对应选项。

---

## 10. M1 实施范围

### P0：必须完成

- [ ] TTS Model Registry。
- [ ] SGLang-Omni Runtime Adapter。
- [ ] Qwen3-TTS Adapter。
- [ ] Fish Drama 3 Adapter。
- [ ] 模型加载与卸载。
- [ ] 运行状态监控。
- [ ] 全局显存调度集成。
- [ ] Pin / LRU / TTL。
- [ ] 基础文本语音合成。
- [ ] Voice Profile 本地存储。
- [ ] 音频文件管理。
- [ ] 推理日志查看。

### P1：建议完成

- [ ] Voice Design Studio。
- [ ] Qwen3-TTS Voice Design。
- [ ] Voice Cloning。
- [ ] 声音样本试听。
- [ ] Voice Profile 编辑。
- [ ] 多模型声音对比。
- [ ] 音频格式转换。
- [ ] OpenAI-compatible Speech API。

### Out of Scope

M1 暂不实现：

- 云端 TTS 服务。
- 在线声音市场。
- 多用户声音资产共享。
- 复杂语音编辑工作站。
- 多 GPU 分布式 TTS 推理。
- 完整的多角色剧本编辑器。

---

## 11. 硬件兼容性与限制

不同模型具有不同的硬件要求。

需要特别注意：

| 模型 | NVIDIA CUDA | Apple Silicon | 其他平台 |
|---|---|---|---|
| Qwen3-TTS | 需验证对应版本 | 需验证 MLX / PyTorch 路径 | 按官方支持情况 |
| Fish Drama 3 | 官方支持路径优先 | 不应默认支持 MPS | 按官方支持情况 |

具体要求需要随模型版本、精度和推理框架版本更新。

特别是 Fish Drama 3，不能因为桌面应用支持 macOS，就假定 Drama 3 也能在 Apple Silicon 上运行。

模型安装和加载前应执行硬件兼容性检查。

---

## 12. 架构决策总结

| 决策项 | 选择 |
|---|---|
| TTS 服务框架 | SGLang-Omni |
| Voice Design 主力模型 | Qwen3-TTS |
| 高表现力语音模型 | Fish Drama 3 |
| 服务管理 | TTS Runtime Manager |
| 模型适配 | Engine Adapter |
| 声音资产管理 | 独立 Voice Library |
| 显存调度 | 全局 Scheduler |
| 资源回收 | LRU + TTL |
| 音频存储 | 本地文件系统 |
| API | OpenAI-compatible + 内部扩展 API |
| 运行策略 | 按需加载、闲置卸载 |

**最终架构原则：**

TTS 是本地 AI Runtime Manager 中的一种模型类型，而不是一个独立的资源管理系统。

所有 TTS 模型必须遵循统一的 Runtime 生命周期管理和全局硬件资源调度机制。

Voice Library 则独立于 Runtime 生命周期，确保用户创建的声音资产可以长期保存，并且不依赖某个具体推理引擎。
