# Point tags —— Drama 3 的表演标签

Drama 3 把**表演**和**音效**直接写在台词里。模型读到标签，就把那个动作演出来。

官方叫 **point tags**（模型卡里叫 "Point tags"）：
> the tags written into the text **at the position where the event should land**

本机实测机型：Apple Silicon（M 系列），模型 `drama-3-mlx-4bit`（原生 MLX 运行时，4bit 量化），
声音 `voice_a89ad40d`（克隆自 `core_01.wav`）。

---

## 1. 语法

**方括号**，不是圆括号：

```
[mumbles] 说了不要开枪……没有子弹的啊……
```

**位置很重要**——标签写在事件发生的地方。想在哪句话前面叹口气，就写在那句话前面：

```
[sighs] 你终于来了。[laughs] 不过嘛，还好赶上了。
```

> ⚠️ **圆括号是错的**。`(mumbles)` 会被当成普通文字念出来——模型会一本正经地
> 读出「括号、mumbles、括号」。本机实测：`(laughs)` 2.9 秒（把标签念了一遍），
> `[laughs]` 2.4 秒（真的在笑）。

---

## 2. 表演标签

同一句台词「你好，今天天气不错。」，只换标签：

| 标签 | 时长 | 响度(RMS) | 相对基线 |
|---|---|---|---|
| *（不带标签）* | 2.37 s | 2178 | 基线：平读一句 |
| `[laughs]` | 3.72 s | 1372 | +1.4 s 笑 |
| `[chuckle]` | 3.72 s | 1617 | +1.4 s 轻笑 |
| `[breaks into laughter]` | 3.81 s | 1738 | +1.4 s 笑出来 |
| `[sighs]` | 4.04 s | 1455 | +1.7 s 叹气 |
| `[crying]` | 4.64 s | 1189 | +2.3 s 哭 |
| `[gasps]` | 3.58 s | 1229 | +1.2 s 倒吸气 |
| `[coughs]` | 3.30 s | 1176 | +0.9 s 咳 |
| `[clears throat]` | 3.30 s | 1597 | +0.9 s 清嗓子 |
| `[sniffs]` | 3.11 s | 1140 | +0.7 s 吸鼻子 |
| `[humming]` | 3.53 s | 1376 | +1.2 s 哼歌 |
| `[grunts]` | 3.81 s | 1568 | +1.4 s 哼哧 |
| `[screams]` | 2.41 s | **2463** | 最响：尖叫 |
| `[whispers]` | 2.23 s | **1024** | 最轻：耳语 |
| `[mumbles]` | 2.18 s | 1786 | 嘟囔着说 |

**怎么读这张表**：

- **非发音类标签**（笑、叹气、哭、咳、清嗓、哼歌、哼哧）会让音频**变长**——
  事件被演出来之后，台词才接着说。
- **`[whispers]` 最轻**（RMS 1024，只有基线的一半）——耳语是**音量**上的变化，不是时长。
- **`[screams]` 最响**（RMS 2463）——尖叫是**音色**上的变化，时长几乎没变。
- **`[mumbles]` 时长和基线差不多**——它是把台词**含混地**念出来，不是加一个动作。

> 同一个请求每次的长度会差零点几秒（默认 `temperature 1.0`），表里是某一次测量值。
> 响度（RMS）比时长更能说明标签有没有起作用。

---

## 3. 拟声词和音效

同一个机制。台词用「咯咯咯」：

| 写法 | 时长 | 响度(RMS) | 听感 |
|---|---|---|---|
| 不带标签 | 1.72 s | 1500 | 把三个字念出来 |
| `[chicken clucks] 咯咯咯` | 0.79 s | **3096** | ✅ 干净的一声鸡叫 |
| `[chicken clucking] 咯咯咯` | 0.84 s | **3112** | ✅ 同上 |
| `[cluck] 咯咯咯` | 0.70 s | 2109 | ✅ 短促一声 |
| `[cackles] 咯咯咯` | 4.46 s | 1241 | 叫了几声 |
| `[clucking] 咯咯咯` | 47.55 s | 1427 | ⚠️ 一直叫下去 |
| `[chicken clucks]`（不跟文字）| 1.02 s | **209** | ❌ 几乎没声 |

### 三个坑

**① 标签和文字要一起给。** 只写标签几乎不出声（响度只有 200 出头），
标签 + 拟声词（`咯咯咯`、`哞`、`汪`）才是那个声音。

**② 别单独用 `[clucking]`。** 模型会理解成「一直叫」，一路叫到长度上限（47 秒）。
想要几声就在标签后面跟一句短的。

**③ 标签别连排。** `[clucks][clucks][clucks] 咯咯咯` 同样失控（47 秒）。

### 别的动物

换个标签换个拟声词就行：

```
[moo] 哞          →  1.63 s，RMS 2451
```

---

## 4. 多角色

Drama 3 可以在一次合成里安排多个说话人。用 `<|speaker:N|>` 在文本里点名，
`N` 是参考音频在列表里的位置（从 0 开始）：

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "input": "<|speaker:0|>你终于来了。<|speaker:1|>我等了很久了。",
    "response_format": "wav",
    "parameters": {
      "character_config": {
        "speakers": [
          { "name": "甲", "audio_path": "/Users/me/甲.wav", "text": "甲的参考台词" },
          { "name": "乙", "audio_path": "/Users/me/乙.wav", "text": "乙的参考台词" }
        ]
      }
    }
  }' -o 对话.wav
```

- 每个说话人给一段**参考音频** + 它的**逐字台词**
- `audio_path` 必须是**本机 WAV**（原生运行时只收本机路径）
- 标签和多角色可以混用：`<|speaker:0|> [sighs] 你终于来了。`

实测：两个说话人、两句台词 → 5.85 秒音频 ✓

---

## 5. 连贯编辑（previous_takes）

把上一遍的音频作为上下文传回来，模型会在那个基础上改：

```json
{
  "model": "drama-3-mlx-4bit",
  "input": "季报周四中午交，麻烦改成周五。",
  "previous_takes": ["季报周四中午交，请在中午之前完成。"],
  "parameters": { "character_config": { "speakers": [ … ] } }
}
```

`previous_takes` 是**音频**，不是文字——先合成一遍，把结果存下来再传回去。
（应用里这条路径还没有界面，只有 API 能用。）

---

## 6. 怎么调

```bash
curl -s -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "drama-3-mlx-4bit",
    "voice_id": "voice_a89ad40d",
    "input": "[mumbles] 说了不要开枪……没有子弹的啊……",
    "response_format": "wav"
  }' -o 喃喃.wav
```

一次跑完全部标签，方便对比：

```bash
#!/usr/bin/env bash
set -euo pipefail
BASE=http://127.0.0.1:54321
VOICE=voice_a89ad40d
TEXT="你好，今天天气不错。"

for t in laughs whispers sighs mumbles coughs crying gasps humming chuckle grunts screams; do
  curl -s -X POST "$BASE/v1/tts/speak" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"drama-3-mlx-4bit\",\"voice_id\":\"$VOICE\",\"input\":\"[$t] $TEXT\",\"response_format\":\"wav\"}" \
    -o "[$t].wav"
  echo "[$t].wav  $(file -b "[$t].wav" | cut -c1-30)"
done
```

---

## 7. 试听文件

| 目录 | 内容 |
|---|---|
| `~/Downloads/point_tags测试/` | 每个表演标签一句（15 个文件，按标签命名）|
| `~/Downloads/拟声词测试/` | 拟声词的各种写法（9 个文件）|

---

## 8. 出处

- 模型卡：<https://huggingface.co/fishaudio/drama-3>（"Point tags" 一节）
- 本机镜像：`~/.lm-lathe/models/tts/mlx-community--fish-audio-drama-3-4bit/`

相关文档：[API 指南](api.md) · [声音设计指南](voice-design.md)
