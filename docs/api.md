# LM-Lathe HTTP API

本地网关的完整调用手册。所有例子都在本机实测过。

**基址**：`http://127.0.0.1:54321`（桌面端起的后端固定用这个端口）

---

## 0. 约定

| 约定 | 说明 |
|---|---|
| 端口 | 桌面端的后端在 **54321**；`lm-lathe-server serve --port X` 可换 |
| 只监听本机 | 默认 `--host 127.0.0.1`；`--host 0.0.0.0` 才让局域网访问 |
| 模型 id | 用**目录里/目录册里的 id**：内置模型 `qwen3-tts-1.7b-voicedesign-mlx`，从 Hub 搜来的 `owner--repo` |
| 声音 id | `voice_` + 8 位十六进制，创建时返回 |
| 音频格式 | `wav`（默认）、`mp3`、`flac`、`opus`、`aac`、`pcm`；转码要 ffmpeg |
| 错误 | 一律 `{"kind": "...", "message": "..."}`，HTTP 状态码同时有意义 |
| 时间 | RFC3339（UTC）|

**先看一眼有什么**：

```bash
curl -s http://127.0.0.1:54321/v1/routes        # 全部路由
curl -s http://127.0.0.1:54321/v1/paths         # 目录、端口、下载并发
curl -s http://127.0.0.1:54321/health           # 活着吗
```

---

## 1. 模型

### 1.1 目录（全部模型，含未下载的）

```bash
curl -s http://127.0.0.1:54321/v1/tts/models | python3 -m json.tool
```

每个模型的字段：

```json
{
  "id": "qwen3-tts-1.7b-voicedesign-mlx",
  "family": "qwen3-tts",
  "engine_id": "mlx-audio",
  "features": ["text_to_speech", "voice_design", "instruction_control", "streaming"],
  "llm_features": [],
  "size_bytes": 2312058795,
  "estimated_memory_bytes": 6442450944,
  "installed": true,
  "compatible": true,
  "path": "/Users/me/.lm-lathe/models/tts/mlx-community--Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit",
  "state": "running",
  "pinned": false
}
```

### 1.2 已下载且机器能跑的

```bash
curl -s http://127.0.0.1:54321/v1/models
```

比上面少一个：只有 `installed && compatible` 的。

### 1.3 运行时状态（谁在跑、占多少内存）

```bash
curl -s http://127.0.0.1:54321/v1/tts/status | python3 -m json.tool
```

```json
{
  "runtimes": [                       // 语音模型
    { "model_id": "drama-3-mlx-4bit", "state": "running",
      "engine_id": "fishaudio-drama3-mlx", "process_id": 42973,
      "endpoint": "http://127.0.0.1:54007", "active_requests": 0,
      "estimated_memory": 31138512896, "actual_memory": 5607047168,
      "checkpoint_bytes": 24781258240, "pinned": false }
  ],
  "llm_runtimes": [                   // 语言模型
    { "model_id": "mlx-community--Qwen3.8-27B-4bit", "state": "running",
      "engine_id": "mlx", "endpoint": "http://127.0.0.1:51393",
      "estimated_memory": 20074394048, "actual_memory": 15676783616 }
  ],
  "active_model": null,
  "scheduler": { "total_bytes": …, "reserved_bytes": …, "used_bytes": …, "available_bytes": …,
                 "tenants": [ { "key": "tts:drama-3-mlx-4bit", "state": "running", … } ] },
  "hardware": { "platform": "macos", "apple_silicon": true, "total_ram_bytes": … }
}
```

> **两个内存数字别搞混**：
> - `checkpoint_bytes` —— 权重文件多大（模型真正的体积）
> - `actual_memory` —— 引擎进程此刻**常驻**多少。权重是 mmap 映射的，没用到的那部分不算
> - `estimated_memory` —— 调度器用来仲裁的估算值（保守）

### 1.4 加载 / 卸载 / 固定

```bash
# 加载（会按需自动起引擎；内存不够时调度器会挤掉别的模型）
curl -s -X POST http://127.0.0.1:54321/v1/tts/models/drama-3-mlx-4bit/load \
  -H 'Content-Type: application/json' -d '{}'

# 卸载
curl -s -X POST http://127.0.0.1:54321/v1/tts/models/drama-3-mlx-4bit/unload -d '{}'

# 固定（固定后不会被自动挤掉，别人要加载时直接报 EngineBusy）
curl -s -X POST http://127.0.0.1:54321/v1/tts/models/drama-3-mlx-4bit/pin -d '{}'
curl -s -X POST http://127.0.0.1:54321/v1/tts/models/drama-3-mlx-4bit/unpin -d '{}'
```

---

## 2. 合成

### 2.1 OpenAI 兼容（最省事）

```bash
curl -s -X POST http://127.0.0.1:54321/v1/audio/speech \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3-tts-1.7b-voicedesign-mlx","input":"你好","voice":"voice_58eb0166"}' \
  -o out.wav
```

### 2.2 带引擎参数（Drama 3 的表演标签、多角色）

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "voice_id": "voice_a89ad40d",
    "input": "[mumbles] 说了不要开枪……没有子弹的啊……",
    "response_format": "wav"
  }' -o out.wav
```

请求字段：

| 字段 | 说明 |
|---|---|
| `model` | 必填，模型 id |
| `input` | 必填，要说的文本 |
| `voice_id` | 声音档案 id（`/v1/audio/speech` 里叫 `voice`）|
| `language` | `zh` / `en` … |
| `response_format` | `wav`（默认）/`mp3`/`flac`/`opus`/`aac`/`pcm` |
| `parameters` | 引擎自己的参数，原样透传 |

**Drama 3 的表演标签写在文本里**（官方叫 point tags），**用方括号**，运行时自己解析：

```
[mumbles] 说了不要开枪……没有子弹的啊……
```

标签要写在**事件发生的位置**——想在哪句话前面叹口气，就把 `[sighs]` 写在那句话前面：

```
[sighs] 你终于来了。[laughs] 不过嘛，还好赶上了。
```

常用的是 `[laughs]` `[whispers]` `[sighs]` `[mumbles]` `[coughs]` `[crying]` `[gasps]` `[humming]` `[chuckle]` `[grunts]` `[screams]`。

台词里的 `……` 会被读成停顿。

#### 拟声词和音效

同一个机制，标签写成动作，声音就是那个动作。本机实测（Drama 3 MLX 4bit，
`voice_a89ad40d`，台词都是「咯咯咯」）：

| 写法 | 时长 | 响度（RMS） | 听感 |
|---|---|---|---|
| 不带标签 | 1.7 s | 1500 | 把三个字念出来 |
| `[chicken clucks] 咯咯咯` | **0.8–1.2 s** | **2900–3100** | ✅ 干净的一声鸡叫 |
| `[chicken clucking] 咯咯咯` | **0.8 s** | **3112** | ✅ 同上 |
| `[cluck] 咯咯咯` | 0.7–1.6 s | 2100 | ✅ 短促一声 |
| `[cackles] 咯咯咯` | 4.5 s | 1241 | 叫了几声 |
| `[clucking] 咯咯咯` | 47.6 s | 1427 | ⚠️ 模型会一直叫下去 |
| `[chicken clucks]`（不跟文字）| 1.0 s | **209** | ❌ 几乎没声 |

**要点**：

- 想要**短促的一声**，用 `[chicken clucks]` 或 `[chicken clucking]`——不到一秒，
  而且比念字响得多（响度翻倍）。
- **标签和文字要一起给**：只写标签几乎不出声（响度只有 200 出头），
  标签 + 拟声词（`咯咯咯`、`哞`、`汪`）才是那个声音。
- **别单独用 `[clucking]`**：模型会当成「一直叫」，一路叫到长度上限（47 秒）。
  要连着叫几声就写几个标签（`[clucks][clucks][clucks]` 也一样会失控），
  想要几声就在标签后面跟一句短的。
- 别的动物同理，换标签换拟声词就行：`[moo] 哞` → 1.6 s、响度 2451。

> 同一个请求每次的长度会不一样（默认 `temperature 1.0`），上面的时长是某一次的测量值。

试听文件在 `~/Downloads/拟声词测试/`，按上表的顺序命名。

**多角色**（Drama 3 专属）：

```json
{
  "model": "drama-3-mlx-4bit",
  "input": "<|speaker:0|>你终于来了。<|speaker:1|>我等了很久。",
  "parameters": {
    "character_config": {
      "speakers": [
        { "name": "甲", "audio_path": "/Users/me/甲.wav", "text": "甲的参考台词" },
        { "name": "乙", "audio_path": "/Users/me/乙.wav", "text": "乙的参考台词" }
      ]
    }
  }
}
```

### 2.3 返回的是什么

| 端点 | 响应 |
|---|---|
| `/v1/audio/speech` | 音频本体（`Content-Type: audio/wav`）|
| `/v1/tts/speak` | 音频本体（同上）|

**都用 `-o 文件名` 存下来**。每次合成还会留一份档：

```
~/.lm-lathe/audio/generated/<voice_id>-<时间戳>.wav
```

### 2.4 声音档案

```bash
# 列出
curl -s http://127.0.0.1:54321/v1/tts/voices | python3 -m json.tool

# 看一个
curl -s http://127.0.0.1:54321/v1/tts/voices/voice_a89ad40d | python3 -m json.tool

# 取试听样本（文件名可带可不带 samples/ 前缀）
curl -s -o sample.wav \
  http://127.0.0.1:54321/v1/tts/voices/voice_a89ad40d/samples/voice_a89ad40d_001.wav

# 删除
curl -s -X DELETE http://127.0.0.1:54321/v1/tts/voices/voice_a89ad40d
```

### 2.5 声音设计

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/voices/design \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "qwen3-tts-1.7b-voicedesign-mlx",
    "name": "小艾",
    "prompt": "一位年轻的女性，声音清澈温柔，语速略慢，带一点书卷气。",
    "language": "zh",
    "gender_presentation": "女声",
    "age_style": "青年",
    "personality": "温柔、耐心",
    "sample_text": "你好，我是小艾。"
  }'
```

只有 `prompt` 必填。细节见 [声音设计指南](voice-design.md)。

### 2.6 声音克隆

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/voices/clone \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "name": "我的声音",
    "reference_audio": "/Users/me/录音.wav",
    "reference_text": "录音里说的原话，逐字",
    "sample_text": "这是克隆出来的声音。"
  }'
```

`reference_audio` 必须是**本机 WAV**（运行时只收本机路径）；`reference_text` 要逐字写对，写错音色会跑偏。

---

## 3. 从 Hugging Face 找模型

### 3.1 搜索

```bash
curl -s "http://127.0.0.1:54321/v1/tts/discover?q=Qwen3.8&limit=20" | python3 -m json.tool
```

```json
{
  "models": [
    { "id": "mlx-community--Qwen3.8-27B-4bit",
      "repo": "mlx-community/Qwen3.8-27B-4bit",
      "family": "llm", "engine_id": "mlx", "format": "mlx",
      "size_bytes": 16074596352, "installed": false }
  ],
  "dropped": 106
}
```

`dropped` 是被过滤掉的仓库数（运行时跑不了的，比如纯 safetensors 的非 MLX 仓库）。

### 3.2 下载

```bash
# 搜索结果里的仓库
curl -s -X POST http://127.0.0.1:54321/v1/tts/discover/install \
  -H 'Content-Type: application/json' -d '{"repo":"mlx-community/Qwen3.8-27B-4bit"}'

# 目录里已知的模型
curl -s -X POST http://127.0.0.1:54321/v1/tts/models/qwen3-8b-gguf/install -d '{}'
```

### 3.3 下载进度

```bash
# 所有下载
curl -s http://127.0.0.1:54321/v1/tts/installs | python3 -m json.tool

# 单个
curl -s http://127.0.0.1:54321/v1/tts/models/qwen3-8b-gguf/install
```

```json
{ "model_id": "qwen3-8b-gguf", "state": "running",
  "bytes": 1073741824, "bytes_total": 5027783296,
  "files_done": 3, "files_total": 5, "error": null }
```

其它：`/v1/tts/models/{id}/install/cancel`（中止，保留已下部分）、`/v1/tts/models/{id}/install/discard`（丢弃重来）。

---

## 4. 语言模型（OpenAI 兼容）

```bash
curl -s -X POST http://127.0.0.1:54321/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "mlx-community--Qwen3.8-27B-4bit",
    "messages": [{"role":"user","content":"用一句话介绍你自己"}],
    "max_tokens": 800
  }' | python3 -m json.tool
```

```json
{ "id": "chatcmpl-…", "object": "chat.completion",
  "model": "mlx-community--Qwen3.8-27B-4bit",
  "choices": [{ "index": 0, "finish_reason": "stop",
                "message": { "role": "assistant", "content": "……" } }],
  "usage": { "prompt_tokens": 14, "completion_tokens": 122, "total_tokens": 136 } }
```

**模型没加载会自动加载**（第一次要等引擎起来，MLX 27B 约 8 秒）。

> **思考型模型**（Qwen3、DeepSeek 这类）会把推理过程放在 `reasoning_content` 里，
> `content` 是最终答案。`max_tokens` 给小了只看到思考、正文是空的 —— 给 800 以上。

普通补全：`POST /v1/completions`，字段是 `prompt` 不是 `messages`。

---

## 5. 运行时（引擎）

```bash
# 列表（装了什么、有什么版本可装）
curl -s http://127.0.0.1:54321/v1/runtimes | python3 -m json.tool

# 装一个引擎
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/mlx-audio/install -d '{}'

# 进度
curl -s http://127.0.0.1:54321/v1/runtimes/mlx-audio/install-progress

# 探活（真起一个服务探 /health）
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/mlx-audio/health

# 切版本 / 卸载
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/mlx-audio/activate -d '{"version":"0.6.4"}'
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/mlx-audio/uninstall -d '{"version":"0.6.4"}'
```

| 引擎 | 程序 | 跑什么 |
|---|---|---|
| `llama.cpp` | `llama-server`（二进制发行版）| GGUF 语言模型 |
| `mlx` | `mlx_lm.server`（PyPI `mlx-lm`）| MLX 的 safetensors 语言模型（Apple Silicon）|
| `mlx-audio` | `mlx_audio.server`（PyPI `mlx-audio`）| Qwen3-TTS 的声音设计与克隆 |
| `sglang-omni-mlx` | 源码编译 | Drama 3、Qwen3-TTS（原生 MLX）|
| `sglang-omni` | PyPI（CUDA）| Qwen3-TTS、Drama 3（NVIDIA 显卡）|

---

## 6. 引擎日志

```bash
curl -s http://127.0.0.1:54321/v1/tts/models/drama-3-mlx-4bit/logs
```

盘上的日志：

```
~/.lm-lathe/logs/tts/<engine>/<model>-<时间戳>.log
~/.lm-lathe/logs/llm/<model>-<时间戳>.log
```

---

## 7. 错误

```json
{ "kind": "engine_error", "message": "the `fishaudio-drama3-mlx` runtime is not installed, install it on the runtime page first" }
```

| kind | 什么时候 | 怎么办 |
|---|---|---|
| `invalid_request` | 请求本身不对（缺字段、路径穿越）| 看 message |
| `model_not_found` | 模型 id 不存在 | `/v1/tts/models` 查准确 id |
| `unsupported_feature` | 模型不支持这功能（设计/克隆）| 换模型 |
| `incompatible_hardware` | 机器跑不了这个检查点 | 换模型或换机器 |
| `engine_error` | 引擎起不来 / 回答出错 | 看引擎日志 |
| `scheduler_error` | 内存不够 | 卸载一个模型，或把保留的**固定**住 |
| `voice_not_found` | 声音 id 不存在 | `/v1/tts/voices` 查 |
| `internal_error` | 后端自己出错 | 看后端日志 |

---

## 8. 一个能直接跑的脚本

```bash
#!/usr/bin/env bash
set -euo pipefail
BASE=http://127.0.0.1:54321

# 用某个声音说一句话
speak() {
  local voice="$1" text="$2" out="$3"
  curl -s -X POST "$BASE/v1/tts/speak" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"drama-3-mlx-4bit\",\"voice_id\":\"$voice\",\"input\":$text,\"response_format\":\"wav\"}" \
    -o "$out"
  echo "$out  $(file -b "$out" | cut -c1-40)"
}

speak voice_a89ad40d '"(mumbles) 说了不要开枪……没有子弹的啊……"' 喃喃.wav
```

```bash
chmod +x speak.sh && ./speak.sh
# 喃喃.wav  RIFF (little-endian) data, WAVE audio, Micros
```

---

## 9. 相关文档

- [声音设计指南](voice-design.md) —— 设计/克隆一个声音，拿到 WAV
- `README.md` —— 总览、数据目录、构建
