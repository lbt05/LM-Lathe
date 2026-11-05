# Local AI Manager — TTS Subsystem

Implementation of the TTS subsystem described in [`todo.md`](./todo.md) (Milestone 1).

The subsystem is a Rust workspace with three crates:

| Crate | Purpose |
|---|---|
| `crates/ai-scheduler` | The **global** hardware scheduler. Every model runtime — LLM, TTS, later diffusion — registers here and competes for one memory pool. |
| `crates/tts-core` | The TTS subsystem: model registry, engine adapters, runtime manager, engine assets, voice library, audio manager. |
| `crates/tts-server` | HTTP API (OpenAI compatible + internal extensions) and a command line interface. |
| `apps/desktop` | The Tauri desktop application: the model / runtime / service / settings pages of the paper-workbench UI. |

```
apps/desktop (Tauri)                HTTP client / CLI
          │                                │
          └──────────────┬─────────────────┘
                         ▼
      TTS Module  ──────────────────  crates/tts-core
          │
          ├── Voice Library          crates/tts-core/src/voice
          ├── TTS Runtime Manager    crates/tts-core/src/runtime.rs
          │         │
          │         ├── Global Scheduler      crates/ai-scheduler
          │         └── Engine Adapter        crates/tts-core/src/adapter
          │                  └── SGLang-Omni  (sgl-omni serve)
          └── Audio Manager          crates/tts-core/src/audio.rs
```

## Quick start

```bash
cargo build --release

# Inspect the machine and the compatibility of every model
./target/release/tts-server doctor

# List the models and whether they are installed
./target/release/tts-server models list

# Download a checkpoint (the compatibility is checked first)
./target/release/tts-server models install qwen3-tts-1.7b-voicedesign

# Speak (the mock engine needs no GPU and no model download)
./target/release/tts-server speak --model mock --text "你好，世界" --out hello.wav

# Start the HTTP API
./target/release/tts-server serve --port 54321
```

The local gateway listens on a fixed port, `54321`, so its URL does not change
with every restart. `LMLATHE_PORT` moves it. When the port is already taken
by a server of the same build, the desktop application reuses that server
instead of starting a second one; when a different build holds the port, the
application moves to the next free port and shows the URL it uses.

All state lives in `~/.local-ai-manager` (override with `--data-dir` or
`LOCAL_AI_MANAGER_HOME`):

```
~/.local-ai-manager/
├── models/{llm,tts}/          model directories
├── voices/<voice_id>/
│   ├── profile.json           voice profile
│   └── samples/               generated samples
├── audio/generated/           synthesised audio
└── logs/tts/                  engine logs
```

## How the specification is implemented

| Specification section | Where |
|---|---|
| §2 SGLang-Omni as inference service | `tts-core/src/adapter/engine.rs` — spawns `sgl-omni serve --model-path … --port …` and waits for `/health` |
| §2.2 Qwen3-TTS, Fish Audio Drama 3 | `tts-core/src/registry.rs` (catalog), `adapter/qwen3_tts.rs`, `adapter/fish_drama3.rs` |
| §3 Architecture / module split | `runtime.rs`, `voice/`, `audio.rs`, `adapter/`, `ai-scheduler` |
| §4 Unified runtime management | `tts-core/src/runtime.rs` — `TTSRuntime`, `RuntimeState` (`Unloaded → Loading → Running → Unloading → Unloaded`, `Failed`), `Pin` as an attribute of the instance |
| §4.2 One model per engine instance | `Inner::load_model_locked` — the running model is unloaded before another one is started; a *pinned* model is never replaced |
| §5 Engine adapter | `tts-core/src/adapter/mod.rs` — `TTSAdapter` (public trait) + `EngineAdapter` (service level) + `ModelProtocol` (model level), so the service process is managed exactly once |
| §6 Voice library | `tts-core/src/voice/` — profile with `engine_config` kept verbatim, samples stored next to the profile, independent of any running model |
| §7 Unified TTS API | `TTSRequest` / `EngineSpecificParameters` in `tts-core/src/types.rs`; OpenAI compatible route `POST /v1/audio/speech` |
| §8 Global VRAM scheduling | `crates/ai-scheduler` — one pool, LRU eviction, pin, TTL sweeper, timeouts, `MemoryLease` |
| §9 Desktop UI | `apps/desktop` — a Tauri application with the 模型 / 运行时 / 服务 / 设置 pages, following the design system in `designs/index.html` |
| §10 M1 scope | See the status table at the end |
| §11 Hardware compatibility | `tts-core/src/hardware.rs` + `registry::check_compatibility` — Drama 3 is refused on machines without CUDA |
| §11 Model installation | `tts-core/src/installer.rs` — the checkpoints are downloaded from the Hugging Face Hub, the compatibility is checked **before** the download starts |

### Runtime lifecycle

```
Unloaded ──load──▶ Loading ──health ok──▶ Running ──unload──▶ Unloading ──▶ Unloaded
                        │
                        └──── error ────▶ Failed
```

Loading a model follows the flow of the specification:

1. resolve the model in the registry and check the hardware compatibility,
2. take the engine gate (only one model may occupy the engine),
3. unload the model that currently runs — unless it is pinned,
4. ask the global scheduler for a memory reservation (other runtimes are
   evicted in LRU order when needed),
5. start the engine and wait until it answers `/health`,
6. mark the runtime as `Running`.

Unloading waits until the model has no in-flight request left, stops the
process (SIGTERM, then SIGKILL), releases the reservation and removes the
tenant from the scheduler.

### Global scheduler

`ai-scheduler` implements the constraints of §8.3:

* pinned runtimes are never evicted automatically,
* a runtime with in-flight requests is never force-unloaded,
* memory is released only after the process exited,
* an unsatisfiable reservation fails with an explicit error,
* reservations support a timeout and cancellation.

TTL defaults to 30 minutes and is evaluated per tenant by the background
sweeper. A tenant is evicted when

```
pinned == false AND active_requests == 0 AND idle_duration > TTL
```

## Model catalog

| Id | Family | Checkpoint | Memory | Notes |
|---|---|---|---|---|
| `qwen3-tts-1.7b-voicedesign` | Qwen3-TTS | `Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign` | 4.2 GiB / 8 GiB | Voice Design: `task_type=VoiceDesign` + `instructions` |
| `qwen3-tts-1.7b-customvoice` | Qwen3-TTS | `Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice` | 4.2 GiB / 8 GiB | Built-in speakers (`Vivian`, `Ryan`, …) |
| `qwen3-tts-1.7b-base` | Qwen3-TTS | `Qwen/Qwen3-TTS-12Hz-1.7B-Base` | 4.2 GiB / 8 GiB | Voice cloning, needs a reference clip |
| `qwen3-tts-0.6b-base` | Qwen3-TTS | `Qwen/Qwen3-TTS-12Hz-0.6B-Base` | 2.3 GiB / 4 GiB | Voice cloning, needs a reference clip |
| `qwen3-tts-0.6b-customvoice-mlx` | Qwen3-TTS | `mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-bf16` | 2.3 GiB / 8 GiB | Apple Silicon, native MLX runtime (`qwen3_tts_server`) |
| `drama-3` | Drama 3 | `fishaudio/drama-3` | 38.7 GiB / 45 GiB | CUDA only, 37 GB of weights |
| `drama-3-int4` | Drama 3 | `fishaudio/drama-3-int4` | 22.4 GiB / 28 GiB | GPTQ, needs compute capability 8.0+ |
| `drama-3-mlx-4bit` | Drama 3 | `mlx-community/fish-audio-drama-3-4bit` | 22.1 GiB / 29 GiB | Apple Silicon, native MLX runtime |
| `drama-3-mlx-8bit` | Drama 3 | `mlx-community/fish-audio-drama-3-8bit` | 39.3 GiB / 48 GiB | Apple Silicon, native MLX runtime |
| `mock-echo` | Mock | – | 64 MiB | Synthetic engine for tests and UI development |

Aliases: `qwen3-tts` → the VoiceDesign checkpoint, `fish-drama-3` / `drama3` →
`drama-3`, `mock` → `mock-echo`.

The table above is only the catalog that ships with the application. The
desktop application searches the Hugging Face Hub for the rest:
`GET /v1/tts/discover?q=` asks the hub for the repositories of a query and
keeps the ones the runtime can serve — the GGUF checkpoints for the language
runtime and the Qwen3-TTS checkpoints for the speech runtime.
`POST /v1/tts/discover/install` downloads such a repository; the model joins
the catalog, so its progress is reported like the progress of every other
model. A GGUF repository keeps one file per quantisation, and only the
preferred one (`Q4_K_M`) is downloaded.

### Hardware rules

* **Drama 3 is refused on machines without CUDA.** The desktop application
  running on macOS must not assume that Drama 3 runs there.
* The MLX variants require Apple Silicon and enough unified memory
  (4-bit: 48 GiB, 8-bit: 64 GiB).
* The native MLX runtime only accepts local reference clips; they are
  converted to base64 mono 44.1 kHz WAV automatically. Remote URLs are
  refused with a clear error.

`GET /v1/tts/models` returns the verdict for every model, so the UI can hide
options the machine cannot serve.

## Running a real engine

```bash
# CUDA machines
uv pip install --prerelease=allow -v -e ".[fishaudio]"
hf download Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign
sgl-omni serve --model-path Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign --port 8000
```

The runtime manager starts and stops the engine itself. The binary is
resolved through the engine assets (see below) and falls back to `PATH`
(`sgl-omni` for the CUDA path, `fishaudio_drama3_server` for the native MLX
path).

### Managing the engines

The engines themselves are assets that the manager owns: which versions
exist, which one is installed and which one is active.

```bash
tts-server engines list
tts-server engines versions llama.cpp
tts-server engines install llama.cpp b12110
tts-server engines use llama.cpp b12109
tts-server engines uninstall llama.cpp b12110
```

| Engine | Platform | Delivery | Source |
|---|---|---|---|
| `llama.cpp` | macOS, Linux | native binary | GitHub Releases (`ggml-org/llama.cpp`) |
| `mlx` | macOS arm64 | Python package | PyPI (`mlx-lm`) |
| `sglang-omni` | Linux | Python package (CUDA) | PyPI (`sglang-omni`) |
| `sglang-omni` | macOS arm64 | source installation (MPS) | `sgl-project/sglang-omni` + `install.sh` |
| `sglang-omni-mlx` | macOS arm64 | source build (native MLX) | `sgl-project/sglang-omni` + `build_runtime.sh` |

The delivery of an engine depends on the platform, because the upstream
project ships the Apple Silicon paths differently from the CUDA one:

* **CUDA** — `sglang-omni` is a wheel on PyPI that pulls the CUDA build of
  PyTorch.
* **Apple Silicon (MPS)** — the wheel is CUDA only, so the engine is built
  from a checkout of the repository: `install.sh` creates a Python 3.12
  environment (`.venv-apple`), installs `ffmpeg@7` and `uv` with Homebrew and
  installs SGLang from source with the `all_mps` extra.
* **Apple Silicon (native MLX)** — the TTS models are served by the C++
  runtime of the repository. `build_runtime.sh` creates a Python 3.12
  environment with the pinned `mlx` build and compiles the servers with
  CMake and Ninja; the result contains `fishaudio_drama3_server`,
  `qwen3_tts_server` and the other model servers. It needs neither CUDA nor
  PyTorch. The release tags do not carry the servers of the newest models
  yet, so this runtime follows the `main` branch.

The native runtime serves one server per model family, the engine id of the
model decides which one is started: `fishaudio-drama3-mlx` →
`fishaudio_drama3_server`, `sglang-omni-mlx` → `qwen3_tts_server`. The
`qwen3-tts-0.6b-customvoice-mlx` checkpoint is the Qwen3-TTS model that the
native runtime can serve, it works without CUDA and without PyTorch. The
PyTorch path (`sgl-omni serve`) crashes inside the MPS kernels on this
machine, the native path is the one that runs.

The available versions are fetched live from GitHub Releases, from the GitHub
tags (the source builds) or from the PyPI JSON API; llama.cpp marks its
build tags as pre-releases, those are the normal release channel. The
installation is *managed*: it happens inside the data directory, several
versions can live side by side and the active one is switched without
touching the others.

```
~/.local-ai-manager/runtimes/
  engines.json                     # manifest: installed versions + active one
  engines/llama.cpp/b12110/        # one directory per installed version
  engines/mlx/0.32.2/venv/         # virtual environment of the Python engines
  engines/sglang-omni-mlx/v0.2.0/  # checkout, build directory and runtime
```

llama.cpp is downloaded from the release asset that matches the platform
(`macos-arm64`, `macos-x64`, `ubuntu-x64`, `ubuntu-arm64`) and unpacked; the
Python engines get a virtual environment and `pip install <package>==<version>`;
the source installations download the archive of a tag, unpack it and run the
install script of the repository. The installation runs in the background,
the progress is polled through `GET /v1/runtimes/{id}/install-progress` and
the output of `pip` or of the build script is streamed into the log.

### Installing the checkpoints

```bash
tts-server models install qwen3-tts-1.7b-voicedesign
tts-server models install drama-3-int4
tts-server models uninstall drama-3-int4
```

The download uses the Hugging Face Hub API: the file list comes from
`/api/models/<repo>/tree/main`, every file is fetched from
`/<repo>/resolve/main/<file>`. Files that are already on the disk are
skipped, so an interrupted installation can be resumed, and demo material
(`*.mp4`, `*.png`, …) is left out.

The checkpoints are stored in the layout of the specification:

```
~/.local-ai-manager/models/tts/<organisation>--<repository>/
```

`HF_TOKEN` is used when it is set, which is needed for gated repositories.
The hardware compatibility is checked **before** the download starts, and a
model that is not on the disk is refused immediately instead of waiting for
the engine to time out.

## HTTP API

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness, free memory and the address the service listens on |
| `GET` | `/v1/paths` | The directories of the service and the number of the parallel downloads |
| `POST` | `/v1/settings/download-concurrency` | Sets how many files of a repository are fetched at the same time |
| `GET` | `/v1/models` | The models the server can serve: the checkpoint is on the disk and the machine can run it |
| `GET` | `/v1/tts/models` | The full catalog with compatibility and runtime state, including the models that are not downloaded yet |
| `GET` | `/v1/tts/status` | Runtime states, scheduler snapshot, hardware profile |
| `POST` | `/v1/audio/speech` | **OpenAI compatible** synthesis |
| `POST` | `/v1/tts/speak` | Synthesis with the engine specific parameters |
| `GET` | `/v1/tts/voices` | List voices (filter by `model`, `engine`, `language`, `query`) |
| `POST` | `/v1/tts/voices` | Create a voice profile |
| `GET`/`PATCH`/`DELETE` | `/v1/tts/voices/{id}` | Read, edit and delete a voice |
| `POST` | `/v1/tts/voices/design` | Voice design: description → voice + preview sample |
| `POST` | `/v1/tts/voices/clone` | Voice cloning: reference clip → voice + preview sample |
| `GET` | `/v1/tts/voices/{id}/samples` | Samples of a voice |
| `GET` | `/v1/tts/voices/{id}/samples/{name}` | Download a sample |
| `POST` | `/v1/tts/models/{id}/load` \| `/unload` \| `/pin` \| `/unpin` | Runtime lifecycle |
| `POST` | `/v1/tts/models/{id}/install` | Starts the download of the checkpoint (`202 Accepted`) |
| `GET` | `/v1/tts/models/{id}/install` | Progress of the download |
| `POST` | `/v1/tts/models/{id}/uninstall` | Removes the checkpoint from the disk; refused for a running model |
| `GET` | `/v1/tts/models/{id}/logs` | Engine log of the model: the running one or the last start attempt |
| `GET` | `/v1/tts/discover?q=` | Searches the Hugging Face Hub for the models the runtime can serve |
| `POST` | `/v1/tts/discover/install` | Downloads a model of the hub (`{"repo": "organisation/repository"}`), answers with its id |
| `GET` | `/v1/runtimes` | The inference engines and their installation state |
| `GET` | `/v1/runtimes/{id}` | One engine with its installed versions |
| `GET` | `/v1/runtimes/{id}/versions` | Versions that can be installed (GitHub Releases / PyPI) |
| `POST` | `/v1/runtimes/{id}/install` | Starts the installation of an engine version (`202 Accepted`) |
| `GET` | `/v1/runtimes/{id}/install-progress` | Progress of the engine installation |
| `POST` | `/v1/runtimes/{id}/activate` | Switches to another installed version |
| `POST` | `/v1/runtimes/{id}/uninstall` | Removes an installed version from the disk |
| `POST` | `/v1/runtimes/{id}/health` | Starts the engine service and probes `/health` |
| `GET` | `/v1/audio/{file}` | Download generated audio |

### OpenAI compatible synthesis

```bash
curl -X POST http://127.0.0.1:54321/v1/audio/speech \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen3-tts","input":"你好，欢迎使用本地 AI 工作台。","voice":"voice_001"}' \
  --output speech.wav
```

`voice` accepts a voice profile id. Advanced features that do not fit into
the OpenAI schema use the internal extensions:

```bash
# Voice design
curl -X POST http://127.0.0.1:54321/v1/tts/voices/design \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen3-tts","prompt":"青年女性，稚嫩、害羞、略带怯意","name":"府中贴身婢女"}'

# Voice cloning
curl -X POST http://127.0.0.1:54321/v1/tts/voices/clone \
  -H "Content-Type: application/json" \
  -d '{"model":"qwen3-tts","reference_audio":"/path/reference.wav","reference_text":"参考文本"}'

# Multi character scene (Drama 3)
curl -X POST http://127.0.0.1:54321/v1/tts/speak \
  -H "Content-Type: application/json" \
  -d '{"model":"drama-3","input":"<|speaker:0|> 你好 <|speaker:1|> 再见",
       "parameters":{"family":"fish_drama3","references":[{"audio_path":"/tmp/a.wav","text":"参考"}]}}'
```

## Voice profiles

A voice profile is independent of the running model. The engine specific part
is stored verbatim in `engine_config`:

```json
{
  "id": "voice_001",
  "name": "府中贴身婢女",
  "description": "青年女性，稚嫩、害羞、略带怯意",
  "language": "Chinese",
  "engine": "qwen3-tts",
  "model_id": "qwen3-tts-1.7b-voicedesign",
  "engine_config": { "voice_design_prompt": "自然语言声音描述" },
  "samples": ["samples/voice_001_001.wav"],
  "created_at": "2026-01-01T00:00:00Z",
  "updated_at": "2026-01-01T00:00:00Z"
}
```

Understood keys of `engine_config`:

| Key | Used by | Meaning |
|---|---|---|
| `voice_design_prompt` | Qwen3-TTS | Natural language voice description → `instructions` |
| `speaker` | Qwen3-TTS | Built-in speaker name |
| `reference_audio` / `reference_text` | all | Reference clip for voice cloning |
| `references` | all | List of `{audio_path, text}` reference clips |
| `character_config.speakers` | Drama 3 | Cast of a scene, the position is the `<\|speaker:N\|>` index |

## Command line interface

```bash
tts-server doctor                                   # hardware and compatibility
tts-server models list|show|load|unload|pin|unpin|logs|install|uninstall
tts-server voices list|design|clone|remove
tts-server engines list|versions|install|use|uninstall
tts-server speak --model qwen3-tts --text "你好" --voice voice_001 --out hi.wav
tts-server play hi.wav
tts-server status
tts-server serve --host 127.0.0.1 --port 54321
```

## Audio manager

* generated audio is written to `audio/generated/` with a timestamp,
* WAV metadata (sample rate, channels, duration) is read from the header,
* conversions between WAV variants (sample rate, channel count, bit depth)
  are done natively; compressed formats (`mp3`, `flac`, `opus`, `aac`, `ogg`)
  are delegated to `ffmpeg` when it is installed,
* `play` uses `afplay` on macOS and `ffplay` on Linux.

## Tests

```bash
cargo test          # 61 tests
cargo clippy --all-targets

cd apps/desktop
npm install
npm run build       # TypeScript and Vite
```

The suite covers the scheduler (reservation, LRU eviction, pin, busy tenants,
TTL sweep), the registry and the compatibility rules, the checkpoint
installation against a fake hub, the voice library, the payload translation
of both model families, the runtime manager (load, unload, model switching,
pin protection, synthesis), the life cycle of the server process and the HTTP
API end to end.

## Desktop application

```bash
cd apps/desktop
npm install
npm run tauri:dev      # development
npm run tauri:build    # bundles Local AI Manager.app
```

The window starts the `tts-server` backend itself (free port, background
process, log file in `logs/tts/desktop-server.log`) and stops it when the
window closes — the backend follows the life cycle of the desktop process
through a pipe, so no orphan process is left behind. The window follows the
paper workbench design of `designs/index.html` and implements four pages
against the HTTP API:

* **模型 / MODELS** — the registry split into *发现模型* and *已下载*, the
  download queue with progress in bytes, compatibility, Load / Unload, Pin
  and the model log. Deleting a checkpoint asks a second time, it removes
  gigabytes from the disk.
* **运行时 / ENGINES** — the inference engines (llama.cpp, MLX,
  SGLang-Omni, native MLX runtime) as real assets: the versions that can be installed, the
  managed installation, the version switch, the removal and the health check
  that starts the service and probes `/health`.
* **服务 / ENDPOINTS** — the local gateway, its routes, the running
  instances and the event stream.
* **设置 / PREFERENCES** — the backend connection, the appearance and the
  local storage.

## Status of the M1 scope

**P0**

- [x] TTS Model Registry
- [x] Model installation from the model hub
- [x] SGLang-Omni Runtime Adapter
- [x] Qwen3-TTS Adapter
- [x] Fish Drama 3 Adapter
- [x] Model loading and unloading
- [x] Runtime state monitoring
- [x] Global memory scheduling integration
- [x] Pin / LRU / TTL
- [x] Basic text to speech
- [x] Voice profile storage
- [x] Audio file management
- [x] Inference log viewer

**P1**

- [x] Voice Design workflow (API, CLI and the studio page)
- [x] Qwen3-TTS Voice Design
- [x] Voice Cloning
- [x] Sample playback (download + local player)
- [x] Voice profile editing
- [x] Multi model comparison (the studio renders the same description on
      several models)
- [x] Audio format conversion
- [x] OpenAI compatible speech API

**Not part of this milestone**

Cloud TTS, the voice market, multi user voice libraries, the speech editing
workstation, multi GPU TTS and the screenplay editor are out of scope as
stated in the specification.

The engine adapters are written against the official SGLang-Omni
documentation and covered by protocol tests, but no checkpoint has been
loaded on real hardware yet — the mock engine is what exercises the whole
pipeline in the test suite.
