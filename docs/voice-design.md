# 用 LM-Lathe 做声音设计，拿到 WAV

一句话：**用一段自然语言描述 → 引擎渲染一段试听样本 → 把那个 WAV 拿走。**

本文里的每条命令都在本机实测过。

---

## 0. 三条路，选一条

| 方式 | 适合 |
|---|---|
| **桌面端** | 点点鼠标，不写代码 |
| **HTTP API（curl）** | 写脚本、接别的程序 |
| **命令行（CLI）** | 终端里一步到位，文件直接落盘 |

三条路用的是同一套后端，产物完全一样。

---

## 1. 前置条件

### 1.1 哪个模型能做声音设计

声音设计（VoiceDesign）需要 **Qwen3-TTS 的 VoiceDesign 检查点**。查一下你机器上有哪些能用：

```bash
curl -s http://127.0.0.1:54321/v1/tts/models | \
  python3 -c "
import json,sys
for m in json.load(sys.stdin):
    if 'voice_design' in (m.get('features') or []):
        print(f\"{m['id']:<40} 已下载={m['installed']}  能跑={m['compatible']}\")"
```

输出里有两行：

```
qwen3-tts-1.7b-voicedesign          已下载=False  能跑=False   ← CUDA 专用，Mac 上跑不了
qwen3-tts-1.7b-voicedesign-mlx      已下载=True   能跑=True    ← Apple Silicon 用这个
```

**Apple Silicon 上用 `qwen3-tts-1.7b-voicedesign-mlx`**（MLX 版，2.2G）。
Linux + NVIDIA 显卡上用 `qwen3-tts-1.7b-voicedesign`（CUDA 版，4.2G）。

没装的话：

```bash
./target/release/lm-lathe-server models install qwen3-tts-1.7b-voicedesign-mlx
```

### 1.2 引擎

声音设计走 **mlx-audio** 引擎（Apple Silicon）。它是 Python 包，用 `uv` 装：

```bash
# 桌面端：运行时页 → 找到 mlx-audio → 安装
# 命令行：
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/mlx-audio/install -d '{}' -H 'content-type: application/json'
```

查状态：

```bash
curl -s http://127.0.0.1:54321/v1/runtimes | \
  python3 -c "
import json,sys
for r in json.load(sys.stdin):
    if r['id']=='mlx-audio': print(r['status'], [i['version'] for i in r.get('installed',[])])"
# installed ['0.6.4']
```

### 1.3 端口

桌面端的后端固定在 **54321**。自己起一个的话：

```bash
./target/release/lm-lathe-server serve --port 54321
```

---

## 2. 方式一：HTTP API（curl）

### 2.1 设计声音

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
    "sample_text": "你好，我是小艾。今天也很高兴见到你。"
  }'
```

**只有 `prompt` 是必填的**，其余都是给档案记的标签。`prompt` 写得越具体，音色越像你想的那样：

| 想说的话 | 怎么写 |
|---|---|
| 性别年龄 | 「一位年轻的女性」「一个中年男性」 |
| 音色 | 「声音清澈温柔」「低沉沙哑」「明亮活泼」 |
| 语速 | 「语速略慢」「语速偏快」 |
| 气质 | 「带一点书卷气」「像是播音腔」「有点慵懒」 |

回答长这样：

```json
{
  "voice": {
    "id": "voice_58eb0166",
    "name": "小艾",
    "model_id": "qwen3-tts-1.7b-voicedesign-mlx",
    "engine_config": { "voice_design_prompt": "一位年轻的女性，声音清澈温柔……" },
    "samples": []
  },
  "sample": "samples/voice_58eb0166_001.wav",
  "sample_path": "/Users/lbt05/.lm-lathe/voices/voice_58eb0166/samples/voice_58eb0166_001.wav"
}
```

三个关键字段：

- **`voice.id`** —— 声音档案的 id（`voice_` 开头），后面合成别的句子要用
- **`sample`** —— 试听样本的文件名
- **`sample_path`** —— 样本在磁盘上的完整路径（**这个可以直接拿去用，不用再下载**）

> 渲染试听样本要几秒（首次还要起引擎，Apple Silicon 上约 7 秒）。
> 渲染失败时档案会被删掉——没有样本的声音档案没有意义。

### 2.2 看档案（确认样本存下来了）

```bash
curl -s http://127.0.0.1:54321/v1/tts/voices/voice_58eb0166 | python3 -m json.tool
```

```json
{
  "id": "voice_58eb0166",
  "name": "小艾",
  "model_id": "qwen3-tts-1.7b-voicedesign-mlx",
  "samples": ["samples/voice_58eb0166_001.wav"],
  "engine_config": { "voice_design_prompt": "一位年轻的女性，声音清澈温柔……" }
}
```

`samples` 数组里是**相对声音目录的路径**。

### 2.3 取 WAV

两种写法都行（路由里已经有一段 `/samples/`）：

```bash
# 写法一：只用文件名（推荐）
curl -s -o 小艾.wav \
  http://127.0.0.1:54321/v1/tts/voices/voice_58eb0166/samples/voice_58eb0166_001.wav

# 写法二：带上 samples/ 前缀（前端用的就是这种）
curl -s -o 小艾.wav \
  http://127.0.0.1:54321/v1/tts/voices/voice_58eb0166/samples/samples/voice_58eb0166_001.wav
```

拿到的是标准 WAV：

```
小艾.wav: RIFF (little-endian) data, WAVE audio, Microsoft PCM, 16 bit, mono 24000 Hz
```

> 24 kHz / 16 bit / 单声道 —— 这是 mlx-audio 的原生输出。

**其实不用下载**：2.1 的回答里 `sample_path` 指向的就是那个文件，直接拷走即可：

```bash
cp ~/.lm-lathe/voices/voice_58eb0166/samples/voice_58eb0166_001.wav ./小艾.wav
```

### 2.4 用这个声音说别的

设计出来的声音会存进档案，之后任何一句话都能用它说：

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "qwen3-tts-1.7b-voicedesign-mlx",
    "voice_id": "voice_58eb0166",
    "input": "这是用我刚刚设计的声音说的另一句话。",
    "response_format": "wav"
  }' \
  -o 另一句.wav
```

响应是音频本身（`Content-Type: audio/wav`），直接 `-o` 存成文件。

OpenAI 兼容的那个端点也行（`voice` 字段放 id）：

```bash
curl -s -X POST http://127.0.0.1:54321/v1/audio/speech \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3-tts-1.7b-voicedesign-mlx","voice":"voice_58eb0166","input":"你好"}' \
  -o 你好.wav
```

---

## 3. 方式二：命令行

一步到位，文件直接落盘：

```bash
./target/release/lm-lathe-server voices design \
  --model qwen3-tts-1.7b-voicedesign-mlx \
  --prompt "一位年长的男性，声音低沉沙哑，语速缓慢。" \
  --name "老张" \
  --language zh \
  --sample-text "你好，我是老张。"
```

输出：

```
created voice voice_26d4a8ef (老张)
sample: samples/voice_26d4a8ef_001.wav (Some(1360))
audio: /Users/lbt05/.lm-lathe/audio/generated/voice_26d4a8ef-20261110T072447740.wav
```

- `sample` —— 档案里的试听样本
- `audio` —— `audio/generated/` 下的一份拷贝，**带时间戳，不会被覆盖**

其它命令：

```bash
lm-lathe-server voices list                       # 列出所有声音
lm-lathe-server voices remove <voice_id>          # 删掉一个声音
```

---

## 4. 方式三：桌面端

1. 打开 **声音** 页（左侧导航）
2. 点 **设计声音**
3. 填：
   - **模型**：选 `qwen3-tts-1.7b-voicedesign-mlx`
   - **声音描述**：就是 2.1 里的 `prompt`，写具体点
   - **名称**、**语言**：档案标签，随便填
   - **试听文本**：想先听听什么效果
4. 点 **生成声音**
5. 几秒后档案出现，右侧是试听样本，可以直接播

> 桌面端的 **去合成** 按钮目前只是跳到「服务」页（那里没有合成表单），要用界面合成暂时还得走上面的 API。

---

## 5. 文件都在哪

```
~/.lm-lathe/
├── voices/
│   └── voice_58eb0166/
│       ├── profile.json                                  ← 档案（描述、标签、engine_config）
│       └── samples/
│           └── voice_58eb0166_001.wav                   ← 试听样本
└── audio/generated/
    └── voice_58eb0166-20261110T072447740.wav            ← 每次合成的留档
```

`profile.json` 长这样：

```json
{
  "id": "voice_58eb0166",
  "name": "小艾",
  "description": "一位年轻的女性，声音清澈温柔，语速略慢，带一点书卷气。",
  "language": "zh",
  "gender_presentation": "女声",
  "age_style": "青年",
  "personality": "温柔、耐心",
  "engine": "qwen3-tts",
  "model_id": "qwen3-tts-1.7b-voicedesign-mlx",
  "engine_config": { "voice_design_prompt": "一位年轻的女性，声音清澈温柔……" },
  "samples": ["samples/voice_58eb0166_001.wav"]
}
```

**`engine_config.voice_design_prompt` 就是那段描述** —— 合成时引擎靠它还原音色，所以改档案时别把这个字段删了。

---

## 6. 一键脚本

```bash
#!/usr/bin/env bash
set -euo pipefail

BASE=http://127.0.0.1:54321
MODEL=qwen3-tts-1.7b-voicedesign-mlx
PROMPT="一位年轻的女性，声音清澈温柔，语速略慢。"
NAME="${1:-小艾}"
OUT="${2:-$NAME.wav}"

# 1) 设计
ANSWER=$(curl -s -X POST "$BASE/v1/tts/voices/design" \
  -H 'Content-Type: application/json' \
  -d "{\"model\":\"$MODEL\",\"name\":\"$NAME\",\"prompt\":\"$PROMPT\",
       \"language\":\"zh\",\"sample_text\":\"你好，我是${NAME}。\"}")

VOICE=$(echo "$ANSWER" | python3 -c 'import json,sys; print(json.load(sys.stdin)["voice"]["id"])')
SAMPLE=$(echo "$ANSWER" | python3 -c 'import json,sys; print(json.load(sys.stdin)["sample"].split("/")[-1])')
echo "声音 id: $VOICE"

# 2) 取试听样本
curl -s -o "$OUT" "$BASE/v1/tts/voices/$VOICE/samples/$SAMPLE"

# 3) 用它再说一句
curl -s -X POST "$BASE/v1/tts/speak" \
  -H 'Content-Type: application/json' \
  -d "{\"model\":\"$MODEL\",\"voice_id\":\"$VOICE\",\"input\":\"这是同一把声音。\",\"response_format\":\"wav\"}" \
  -o "${OUT%.wav}_再说一句.wav"

echo "得到: $OUT  和  ${OUT%.wav}_再说一句.wav"
```

```bash
chmod +x design.sh && ./design.sh 小艾
```

---

## 7. 排错

| 现象 | 原因 |
|---|---|
| `unsupported_feature` | 这个模型不支持声音设计 → 用 `qwen3-tts-1.7b-voicedesign-mlx`（Mac）或 `qwen3-tts-1.7b-voicedesign`（CUDA）|
| `model_not_found` | 模型 id 写错了 → 先跑 1.1 那条命令看准确 id |
| `incompatible_hardware` | 机器跑不了这个检查点（CUDA 版在 Mac 上永远跑不了）|
| `the mlx-audio runtime is not installed` | 引擎没装 → 见 1.2 |
| `the voice description must not be empty` | `prompt` 没填 |
| 渲染失败、档案也没了 | 引擎起不来或显存/内存不够 → 看 `~/.lm-lathe/logs/tts/mlx-audio/` 里的日志 |
| 取样本 400 `sample ... does not exist` | 样本名写错了 → 先 `GET /v1/tts/voices/{id}` 看 `samples` 数组 |
| 取样本 400 `invalid sample name` | 名字里有 `..` 之类的东西 → 用 2.3 的写法 |

看引擎日志：

```bash
tail -40 ~/.lm-lathe/logs/tts/mlx-audio/*.log
```

---

## 8. 声音克隆

克隆是把**一段录音**变成一个声音档案，之后用它说任何话。

### 8.1 两个引擎，两个模型

| 引擎 | 模型 | 克隆效果 | 内存 |
|---|---|---|---|
| **Drama 3**（原生 MLX）| `drama-3-mlx-4bit` | 音色最像原声，支持多角色、情绪控制、音频编辑 | 权重 22.1G，峰值约 29G |
| Drama 3（原生 MLX）| `drama-3-mlx-8bit` | 同上，8bit 量化 | 权重 39.3G |
| Qwen3-TTS（mlx-audio）| `qwen3-tts-1.7b-base-mlx` | 快得多，内存小 | 权重 2.2G |

**想用 Drama 3 克隆，就选 `drama-3-mlx-4bit`**（下面第 3 节）。

### 8.2 用 Drama 3 克隆

前置条件（比声音设计多两条）：

```bash
# 1) 模型在盘上（22.1G）
curl -s http://127.0.0.1:54321/v1/tts/models | \
  python3 -c "
import json,sys
for m in json.load(sys.stdin):
    if 'drama-3-mlx' in m['id']: print(f\"{m['id']:<28} 已下载={m['installed']} 能跑={m['compatible']}\")"
# drama-3-mlx-4bit   已下载=True  能跑=True

# 2) 引擎装好了（原生 MLX 运行时，源码编译，第一次要十几分钟）
curl -s http://127.0.0.1:54321/v1/runtimes | \
  python3 -c "
import json,sys
for r in json.load(sys.stdin):
    if r['id']=='sglang-omni-mlx': print(r['status'], [i['version'] for i in r.get('installed',[])])"
# installed ['main']
```

装引擎：

```bash
curl -s -X POST http://127.0.0.1:54321/v1/runtimes/sglang-omni-mlx/install -H 'Content-Type: application/json' -d '{}'
```

装模型：

```bash
./target/release/lm-lathe-server models install drama-3-mlx-4bit
```

### 8.3 克隆请求

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/voices/clone \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "name": "小艾克隆",
    "description": "用 Drama 3 从录音克隆的声音",
    "language": "zh",
    "reference_audio": "/Users/me/录音.wav",
    "reference_text": "录音里说的原话",
    "sample_text": "这是克隆出来的声音在说另一句话。"
  }'
```

回答和声音设计一样：

```json
{
  "voice": {
    "id": "voice_1333ad1e",
    "name": "小艾克隆",
    "engine": "fish-drama-3",
    "model_id": "drama-3-mlx-4bit",
    "engine_config": {
      "reference_audio": "/Users/me/录音.wav",
      "reference_text": "录音里说的原话"
    },
    "samples": []
  },
  "sample": "samples/voice_1333ad1e_001.wav",
  "sample_path": "/Users/lbt05/.lm-lathe/voices/voice_1333ad1e/samples/voice_1333ad1e_001.wav"
}
```

取 WAV 的方式和 2.3 完全一样：

```bash
curl -s -o 克隆.wav http://127.0.0.1:54321/v1/tts/voices/voice_1333ad1e/samples/voice_1333ad1e_001.wav
```

用克隆的声音说别的：

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "voice_id": "voice_1333ad1e",
    "input": "克隆出来的声音，第二句。",
    "response_format": "wav"
  }' -o 第二句.wav
```

### 8.4 参考音频的要求

| 要求 | 说明 |
|---|---|
| 本机文件 | 原生 MLX 运行时只收本机路径，不收 URL |
| WAV | 应用会自己转成 44.1 kHz 单声道 16bit 的 WAV 再发给引擎；**MP3/M4A 之类要先转成 WAV** |
| 10 秒左右 | 太短音色不稳，太长会占内存 |
| 单人 | 多人说话会混在一起 |
| `reference_text` | 录音里说的**原话**（逐字）。写错或留空，音色会跑偏 |
| 干净的环境 | 底噪和混响会被一起学进去 |

转格式（需要 ffmpeg）：

```bash
ffmpeg -i 录音.m4a -ar 44100 -ac 1 录音.wav
```

### 8.5 声音设计 vs 声音克隆

| | 声音设计 | 声音克隆（Drama 3）| 声音克隆（Qwen3-TTS）|
|---|---|---|---|
| 输入 | 一段文字描述 | 一段录音 + 原话 | 一段录音 + 原话 |
| 端点 | `POST /v1/tts/voices/design` | `POST /v1/tts/voices/clone` | 同左 |
| 模型 | `qwen3-tts-1.7b-voicedesign-mlx` | `drama-3-mlx-4bit` | `qwen3-tts-1.7b-base-mlx` |
| 引擎 | mlx-audio | 原生 MLX（`sglang-omni-mlx`）| mlx-audio |
| 权重 | 2.2 G | 22.1 G | 2.2 G |
| 额外能力 | 声音设计 | **多角色、情绪控制、音频编辑** | 只有克隆 |
| 起步速度 | ~7 秒 | ~10 秒（含起引擎）| ~3 秒 |

出来的档案都一样用：一样有 `id`、一样有试听样本、一样能拿去 `/v1/tts/speak`。

> **内存提醒**：Drama 3 克隆时占掉约 29 G。调度器会把装不下的模型挤掉（语言模型优先被挤），腾出来的空间不够的话会直接报 `insufficient resources`。
