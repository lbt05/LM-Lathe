//! Management of the inference engine assets.
//!
//! The runtime manager *starts* engines; this module owns the engines
//! themselves: where they come from, which versions exist, which version is
//! installed and which one is active. Three engines are known, matching the
//! runtime tree of the specification:
//!
//! * `llama.cpp` — native binaries published as GitHub releases.
//! * `mlx` — the Apple MLX runtime, installed from PyPI (`mlx-lm`).
//! * `sglang-omni` — the SGLang-Omni service, installed from PyPI.
//!
//! Every engine is installed into its own directory, one directory per
//! version, so that several versions can live side by side:
//!
//! ```text
//! ~/.lm-lathe/runtimes/
//!   engines.json                     # manifest: installed versions + active one
//!   llama.cpp/b12110/build/bin/llama-server
//!   mlx/0.32.2/venv/bin/mlx_lm.server
//!   sglang-omni/0.2.0/venv/bin/sgl-omni
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::{TtsError, TtsResult};
use crate::paths::AppPaths;

/// GitHub API used to discover the release assets.
const GITHUB_API: &str = "https://api.github.com";
/// PyPI API used to discover the published versions.
const PYPI_API: &str = "https://pypi.org/pypi";
/// Timeout of a single HTTP request against the release feeds.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout of the connection to the release hosts.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Number of lines kept in the installation log.
const LOG_TAIL: usize = 200;

/// How an engine is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineDistribution {
    /// A release archive that contains a native binary.
    Binary,
    /// A Python package that is installed into a virtual environment.
    PythonPackage,
    /// A source checkout that is built by a script of the repository.
    SourceBuild,
}

/// How a version of a source engine is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRef {
    /// A tag of the repository.
    Tag,
    /// A branch of the repository.
    Branch,
}

/// The delivery of an engine on one or more platforms.
///
/// The same engine is delivered differently depending on the platform: the
/// CUDA path of SGLang-Omni is a wheel on PyPI, the Apple Silicon path is a
/// source installation with its own script.
#[derive(Debug, Clone, Serialize)]
pub struct PlatformDelivery {
    /// The platforms this delivery applies to.
    pub platforms: Vec<String>,
    /// How the engine is installed.
    pub distribution: EngineDistribution,
    /// PyPI package for a `python-package` delivery.
    pub package: Option<String>,
    /// Extras of the PyPI package, e.g. `server`.
    ///
    /// The server of mlx-audio lives behind an extra, so the delivery names
    /// it here and the installer asks for `package[extras]`.
    pub extras: Option<String>,
    /// How the versions of a source engine are named.
    pub source_ref: SourceRef,
    /// Script inside the checkout that installs the engine.
    ///
    /// `{target}` is replaced with the directory of the version.
    pub install_script: Option<String>,
    /// Arguments of the install script.
    ///
    /// `{target}` is replaced with the directory of the version.
    pub script_args: Vec<String>,
    /// Path of the program, relative to the directory of the version.
    pub program_path: Option<String>,
    /// Python version of the virtual environment of a `python-package`
    /// delivery.
    ///
    /// The delivery creates the environment with `uv` when a version is
    /// named, because the system Python is often too new for the wheels of
    /// the package.
    pub python_version: Option<String>,
    /// The other binaries the runtime provides.
    pub programs: Vec<String>,
    /// Tools that have to be present on the machine.
    pub requirements: Vec<String>,
    /// What the delivery does, shown in the user interface.
    pub note: Option<String>,
}

impl EngineDistribution {
    /// The name of the distribution in the user interface.
    pub fn label(&self) -> &'static str {
        match self {
            EngineDistribution::Binary => "binary",
            EngineDistribution::PythonPackage => "python-package",
            EngineDistribution::SourceBuild => "source-build",
        }
    }
}

/// A runtime engine the manager knows about.
#[derive(Debug, Clone, Serialize)]
pub struct EngineDefinition {
    /// Stable identifier, also used as the directory name.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line description.
    pub summary: String,
    /// How the engine is delivered.
    pub distribution: EngineDistribution,
    /// GitHub repository `owner/name` of the engine.
    pub repository: Option<String>,
    /// Program that serves the inference API.
    pub program: String,
    /// Program used when the engine is managed by the operating system.
    ///
    /// The Apple path of the TTS subsystem for example is served by a custom
    /// build that is not part of the managed distribution.
    pub system_program: Option<String>,
    /// Arguments of the server. `{model}` and `{port}` are replaced.
    pub server_args: Vec<String>,
    /// Capabilities advertised in the user interface.
    pub capabilities: Vec<String>,
    /// How the engine is delivered on which platform.
    pub deliveries: Vec<PlatformDelivery>,
    /// Model engine ids this engine is able to serve.
    pub model_engine_ids: Vec<String>,
    /// Model engine ids the *managed* distribution is able to serve.
    ///
    /// The managed MLX runtime is an LLM server and cannot speak the TTS
    /// protocol, so the Apple path keeps using the system build.
    pub managed_model_engine_ids: Vec<String>,
    /// Tag prefixes that mark the normal release channel of the engine.
    ///
    /// llama.cpp publishes its binaries as `b<build>` tags and marks them as
    /// pre-releases, although they are the stable channel.
    pub build_tags: Vec<String>,
    /// The program of the managed installation that serves a model engine id.
    ///
    /// The native MLX runtime ships one server per model family, the id of
    /// the model decides which one is started.
    pub model_programs: Vec<(String, String)>,
}

impl EngineDefinition {
    /// The delivery that applies to a platform.
    pub fn delivery_for(&self, platform: &str) -> Option<&PlatformDelivery> {
        self.deliveries
            .iter()
            .find(|delivery| delivery.platforms.iter().any(|p| p == platform))
    }

    /// The delivery that applies to this machine.
    pub fn delivery_here(&self) -> Option<&PlatformDelivery> {
        self.delivery_for(&platform_key())
    }

    /// The platforms the engine runs on.
    pub fn platforms(&self) -> Vec<String> {
        let mut platforms: Vec<String> = self
            .deliveries
            .iter()
            .flat_map(|delivery| delivery.platforms.clone())
            .collect();
        platforms.sort();
        platforms.dedup();
        platforms
    }

    /// `true` when the engine can be installed on this machine.
    pub fn supported_here(&self) -> bool {
        self.delivery_here().is_some()
    }

    /// `true` when the engine serves models with the given engine id.
    pub fn serves(&self, model_engine_id: &str) -> bool {
        self.model_engine_ids
            .iter()
            .any(|id| id == model_engine_id)
    }

    /// The program that is used to serve the models.
    ///
    /// A system wide installation wins over the managed one, because it is
    /// the build the models were tested with.
    pub fn program_for(&self, system_binary: Option<&Path>) -> &str {
        if system_binary.is_some() {
            self.system_program.as_deref().unwrap_or(&self.program)
        } else {
            &self.program
        }
    }
}

/// A delivery that only names the platforms.
fn delivery(
    platforms: &[&str],
    distribution: EngineDistribution,
) -> PlatformDelivery {
    PlatformDelivery {
        platforms: platforms.iter().map(|p| p.to_string()).collect(),
        distribution,
        source_ref: SourceRef::Tag,
        package: None,
        extras: None,
        install_script: None,
        script_args: Vec::new(),
        program_path: None,
        python_version: None,
        programs: Vec::new(),
        requirements: Vec::new(),
        note: None,
    }
}

/// The engines known to the manager.
pub fn engine_definitions() -> Vec<EngineDefinition> {
    vec![
        EngineDefinition {
            id: "llama.cpp".to_string(),
            name: "llama.cpp".to_string(),
            summary: "CPU / GPU 通用推理运行时，GGUF 模型，官方二进制发行版。".to_string(),
            distribution: EngineDistribution::Binary,
            repository: Some("ggml-org/llama.cpp".to_string()),
            program: "llama-server".to_string(),
            system_program: None,
            server_args: vec![
                "--host".to_string(),
                "127.0.0.1".to_string(),
                "--port".to_string(),
                "{port}".to_string(),
                "--model".to_string(),
                "{model}".to_string(),
            ],
            capabilities: vec!["llm".to_string(), "gguf".to_string(), "cpu".to_string()],
            deliveries: vec![delivery(
                &[
                    "darwin-arm64",
                    "darwin-x64",
                    "linux-x64",
                    "linux-arm64",
                ],
                EngineDistribution::Binary,
            )],
            model_engine_ids: vec!["llama.cpp".to_string()],
            managed_model_engine_ids: vec!["llama.cpp".to_string()],
            build_tags: vec!["b".to_string()],
            model_programs: Vec::new(),
        },
        EngineDefinition {
            id: "mlx".to_string(),
            name: "Apple MLX".to_string(),
            summary: "Apple Silicon 原生 LLM 运行时，MLX 后端，服务端由 mlx-lm 提供。".to_string(),
            distribution: EngineDistribution::PythonPackage,
            repository: None,
            program: "mlx_lm.server".to_string(),
            system_program: None,
            server_args: vec![
                "--model".to_string(),
                "{model}".to_string(),
                "--port".to_string(),
                "{port}".to_string(),
            ],
            capabilities: vec!["llm".to_string(), "mlx".to_string(), "unified-memory".to_string()],
            deliveries: vec![PlatformDelivery {
                package: Some("mlx-lm".to_string()),
                program_path: Some("venv/bin/mlx_lm.server".to_string()),
                requirements: vec!["python3".to_string()],
                note: Some("PyPI 发行版，装进独立的虚拟环境；TTS 模型不在这个运行时里。".to_string()),
                ..delivery(&["darwin-arm64"], EngineDistribution::PythonPackage)
            }],
            model_engine_ids: Vec::new(),
            managed_model_engine_ids: Vec::new(),
            build_tags: Vec::new(),
            model_programs: Vec::new(),
        },
        EngineDefinition {
            id: "sglang-omni".to_string(),
            name: "SGLang-Omni".to_string(),
            summary: "TTS 推理服务：CUDA 走 PyPI 发行版，需要 NVIDIA GPU。".to_string(),
            distribution: EngineDistribution::PythonPackage,
            repository: Some("sgl-project/sglang-omni".to_string()),
            program: "sgl-omni".to_string(),
            system_program: None,
            server_args: vec![
                "serve".to_string(),
                "--model-path".to_string(),
                "{model}".to_string(),
                "--port".to_string(),
                "{port}".to_string(),
            ],
            capabilities: vec![
                "tts".to_string(),
                "voice-cloning".to_string(),
                "voice-design".to_string(),
                "streaming".to_string(),
            ],
            deliveries: vec![PlatformDelivery {
                package: Some("sglang-omni".to_string()),
                program_path: Some("venv/bin/sgl-omni".to_string()),
                requirements: vec!["uv".to_string()],
                note: Some("CUDA 路径：PyPI 发行版（wheel 带 CUDA 依赖），需要 CUDA 运行时。".to_string()),
                ..delivery(
                    &["linux-x64", "linux-arm64"],
                    EngineDistribution::PythonPackage,
                )
            }],
            model_engine_ids: vec!["sglang-omni".to_string()],
            managed_model_engine_ids: vec!["sglang-omni".to_string()],
            build_tags: Vec::new(),
            model_programs: Vec::new(),
        },
        EngineDefinition {
            id: "mlx-audio".to_string(),
            name: "mlx-audio".to_string(),
            summary:
                "Apple Silicon 上的 MLX 语音服务：1.7B Qwen3-TTS 的声音设计与克隆都走这条路径。"
                    .to_string(),
            distribution: EngineDistribution::PythonPackage,
            repository: Some("Blaizzy/mlx-audio".to_string()),
            program: "mlx_audio.server".to_string(),
            system_program: None,
            // The model is not a command line argument: mlx-audio loads the
            // checkpoint that the request names, so one server serves every
            // checkpoint of the catalogue.
            server_args: vec![
                "--host".to_string(),
                "127.0.0.1".to_string(),
                "--port".to_string(),
                "{port}".to_string(),
            ],
            capabilities: vec![
                "tts".to_string(),
                "voice-cloning".to_string(),
                "voice-design".to_string(),
                "mlx".to_string(),
                "unified-memory".to_string(),
            ],
            deliveries: vec![PlatformDelivery {
                package: Some("mlx-audio".to_string()),
                extras: Some("server".to_string()),
                program_path: Some("venv/bin/mlx_audio.server".to_string()),
                python_version: Some("3.12".to_string()),
                requirements: vec!["uv".to_string()],
                note: Some(
                    "PyPI 发行版，装进独立的 Python 3.12 虚拟环境；服务端是 OpenAI 兼容的 \
                     mlx_audio.server，模型由请求指定。"
                        .to_string(),
                ),
                ..delivery(&["darwin-arm64"], EngineDistribution::PythonPackage)
            }],
            model_engine_ids: vec!["mlx-audio".to_string()],
            managed_model_engine_ids: vec!["mlx-audio".to_string()],
            build_tags: Vec::new(),
            model_programs: Vec::new(),
        },
        EngineDefinition {
            id: "sglang-omni-mlx".to_string(),
            name: "SGLang-Omni 原生 MLX".to_string(),
            summary:
                "Apple Silicon 上的原生 C++ MLX 运行时，提供 fishaudio_drama3_server 等 TTS 服务二进制。"
                    .to_string(),
            distribution: EngineDistribution::SourceBuild,
            repository: Some("sgl-project/sglang-omni".to_string()),
            program: "fishaudio_drama3_server".to_string(),
            system_program: Some("fishaudio_drama3_server".to_string()),
            server_args: vec![
                "--model-path".to_string(),
                "{model}".to_string(),
                "--model-name".to_string(),
                "{model_name}".to_string(),
                "--port".to_string(),
                "{port}".to_string(),
            ],
            capabilities: vec![
                "tts".to_string(),
                "voice-cloning".to_string(),
                "mlx".to_string(),
                "unified-memory".to_string(),
            ],
            deliveries: vec![PlatformDelivery {
                source_ref: SourceRef::Branch,
                install_script: Some(
                    "sglang_omni_mlx/native/scripts/build_runtime.sh".to_string(),
                ),
                script_args: vec![
                    "{target}/runtime".to_string(),
                    "{target}/build".to_string(),
                ],
                program_path: Some("runtime/bin/fishaudio_drama3_server".to_string()),
                programs: vec![
                    "fishaudio_drama3_server".to_string(),
                    "fishaudio_drama3_synthesize".to_string(),
                    "qwen3_tts_server".to_string(),
                    "qwen3_asr_server".to_string(),
                ],
                requirements: vec!["uv".to_string(), "clang".to_string()],
                note: Some(
                    "原生 MLX 运行时：跟随仓库 main 分支，build_runtime.sh 用 uv 建 Python 3.12 \
                     环境（固定 mlx 0.32.3 与 cmake/ninja），再用 CMake/Ninja 编译出各模型的 \
                     C++ 服务二进制；不需要 CUDA、PyTorch 或 Python 运行时。发布标签里还没有 \
                     drama3 与 qwen3-tts 的服务，所以这里用的是主分支。"
                        .to_string(),
                ),
                ..delivery(&["darwin-arm64"], EngineDistribution::SourceBuild)
            }],
            model_engine_ids: vec![
                "fishaudio-drama3-mlx".to_string(),
                "sglang-omni-mlx".to_string(),
            ],
            managed_model_engine_ids: vec![
                "fishaudio-drama3-mlx".to_string(),
                "sglang-omni-mlx".to_string(),
            ],
            build_tags: Vec::new(),
            model_programs: vec![
                (
                    "fishaudio-drama3-mlx".to_string(),
                    "fishaudio_drama3_server".to_string(),
                ),
                (
                    "sglang-omni-mlx".to_string(),
                    "qwen3_tts_server".to_string(),
                ),
            ],
        },
    ]
}

/// The definition of one engine.
pub fn engine_definition(engine_id: &str) -> Option<EngineDefinition> {
    engine_definitions().into_iter().find(|d| d.id == engine_id)
}

/// The delivery of an engine on the current platform.
fn delivery_of(definition: &EngineDefinition) -> PlatformDelivery {
    definition
        .delivery_here()
        .cloned()
        .unwrap_or_else(|| delivery(&[], definition.distribution))
}

/// One installed version of an engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledEngine {
    /// The installed version, e.g. `b12110` or `0.32.2`.
    pub version: String,
    /// Directory the version lives in.
    pub path: PathBuf,
    /// The program that serves the engine.
    pub entry: Option<PathBuf>,
    /// Installation time as an ISO 8601 timestamp.
    pub installed_at: String,
    /// Size of the installation in bytes.
    pub size_bytes: u64,
    /// Where the version came from, e.g. `github` or `pypi`.
    pub source: String,
}

/// The state of one engine as the user interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct EngineAsset {
    /// Engine id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// One line description.
    pub summary: String,
    /// `binary`, `python-package` or `source-build`.
    pub distribution: String,
    /// GitHub repository of the engine.
    pub repository: Option<String>,
    /// PyPI package of the engine.
    pub package: Option<String>,
    /// Program that serves the engine.
    pub program: String,
    /// The other binaries the engine provides.
    pub programs: Vec<String>,
    /// What the delivery on this platform does.
    pub note: Option<String>,
    /// Tools that have to be present on the machine.
    pub requirements: Vec<String>,
    /// Capabilities of the engine.
    pub capabilities: Vec<String>,
    /// Platforms the engine supports.
    pub platforms: Vec<String>,
    /// `true` when the engine can be installed on this machine.
    pub supported_here: bool,
    /// The platform of this machine.
    pub platform: String,
    /// A system wide installation that was found on the `PATH`.
    pub system_entry: Option<PathBuf>,
    /// The version that is currently active.
    pub current_version: Option<String>,
    /// Every installed version.
    pub installed: Vec<InstalledEngine>,
    /// Total size of the installed versions.
    pub size_bytes: u64,
    /// `installed`, `system`, `available` or `unsupported`.
    pub status: String,
    /// The model engine ids this engine serves.
    pub model_engine_ids: Vec<String>,
}

/// One version that can be installed.
#[derive(Debug, Clone, Serialize)]
pub struct EngineVersion {
    /// The version, e.g. `b12110` or `0.32.2`.
    pub version: String,
    /// Publication date of the release.
    pub published_at: Option<String>,
    /// `true` for pre-releases.
    pub prerelease: bool,
    /// Size of the download in bytes.
    pub size_bytes: Option<u64>,
    /// Name of the release asset.
    pub asset_name: Option<String>,
    /// Direct download url of the release asset.
    pub download_url: Option<String>,
    /// `true` when the version is already installed.
    pub installed: bool,
}

/// Progress of a running installation.
#[derive(Debug, Clone, Serialize)]
pub struct EngineInstallProgress {
    /// The engine that is being installed.
    pub engine_id: String,
    /// The version that is being installed.
    pub version: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    /// Completion in percent, `None` when it cannot be determined.
    pub percent: Option<f64>,
    /// Bytes written so far.
    pub bytes: u64,
    /// Total number of bytes, when known.
    pub total_bytes: Option<u64>,
    /// Human readable status line.
    pub message: String,
    /// Tail of the installation output.
    pub log: Vec<String>,
    /// Error message when the installation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The result of a health check.
#[derive(Debug, Clone, Serialize)]
pub struct EngineHealth {
    /// The engine that was checked.
    pub engine_id: String,
    /// `healthy`, `unhealthy`, `missing` or `no-model`.
    pub status: String,
    /// Human readable detail.
    pub detail: String,
    /// The program that was used.
    pub entry: Option<PathBuf>,
    /// The endpoint that answered, when the service was started.
    pub endpoint: Option<String>,
    /// Duration of the check in milliseconds.
    pub duration_ms: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct EngineManifest {
    #[serde(default)]
    engines: BTreeMap<String, EngineManifestEntry>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct EngineManifestEntry {
    #[serde(default)]
    current: Option<String>,
    #[serde(default)]
    installed: Vec<InstalledEngine>,
}

/// Owns the engine assets on disk.
pub struct EngineAssetManager {
    paths: AppPaths,
    client: reqwest::Client,
    download_client: reqwest::Client,
    manifest: Mutex<EngineManifest>,
    progress: Mutex<BTreeMap<String, EngineInstallProgress>>,
}

impl EngineAssetManager {
    /// Creates a manager for the given data directory.
    pub fn new(paths: AppPaths) -> TtsResult<Arc<Self>> {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("lm-lathe/0.1")
            .build()
            .map_err(|err| TtsError::Internal(format!("http client: {err}")))?;
        // The download of a release archive is not bounded by a total
        // timeout, only the connection is.
        let download_client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("lm-lathe/0.1")
            .build()
            .map_err(|err| TtsError::Internal(format!("http client: {err}")))?;
        let manager = Arc::new(Self {
            paths,
            client,
            download_client,
            manifest: Mutex::new(EngineManifest::default()),
            progress: Mutex::new(BTreeMap::new()),
        });
        manager.load_manifest()?;
        Ok(manager)
    }

    /// The directory the engines are installed into.
    pub fn root(&self) -> PathBuf {
        self.paths.engines_root()
    }

    /// Reads the manifest from disk.
    fn load_manifest(&self) -> TtsResult<()> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(());
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| TtsError::io(&path, e))?;
        let manifest: EngineManifest =
            serde_json::from_str(&raw).map_err(TtsError::serialization)?;
        *self.manifest.lock().unwrap_or_else(|e| e.into_inner()) = manifest;
        Ok(())
    }

    /// Writes the manifest to disk.
    fn save_manifest(&self) -> TtsResult<()> {
        let path = self.manifest_path();
        let manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        let raw = serde_json::to_string_pretty(&*manifest).map_err(TtsError::serialization)?;
        std::fs::write(&path, raw).map_err(|e| TtsError::io(&path, e))
    }

    /// Path of the manifest file.
    fn manifest_path(&self) -> PathBuf {
        self.paths.engines_root().join("engines.json")
    }

    /// Directory of one installed version.
    fn version_dir(&self, engine_id: &str, version: &str) -> PathBuf {
        self.paths
            .engines_root()
            .join(engine_id)
            .join(version.replace('/', "_"))
    }

    /// The state of every known engine.
    ///
    /// An engine that has no delivery for this platform cannot run here and
    /// is not listed. An engine that is installed or that sits in the path
    /// stays in the list, so the user can still see and remove it.
    pub fn list(&self) -> Vec<EngineAsset> {
        let manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        engine_definitions()
            .into_iter()
            .map(|definition| {
                let entry = manifest.engines.get(&definition.id);
                let installed = entry.map(|e| e.installed.clone()).unwrap_or_default();
                let current_version = entry.and_then(|e| e.current.clone());
                let system_entry = find_in_path(&definition.program);
                let size_bytes = installed.iter().map(|i| i.size_bytes).sum();
                let status = if !definition.supported_here() {
                    "unsupported".to_string()
                } else if current_version.is_some() {
                    "installed".to_string()
                } else if system_entry.is_some() {
                    "system".to_string()
                } else {
                    "available".to_string()
                };
                let delivery = definition.delivery_here();
                EngineAsset {
                    id: definition.id.clone(),
                    name: definition.name.clone(),
                    summary: definition.summary.clone(),
                    distribution: delivery
                        .map(|delivery| delivery.distribution)
                        .unwrap_or(definition.distribution)
                        .label()
                        .to_string(),
                    repository: definition.repository.clone(),
                    package: delivery.and_then(|delivery| delivery.package.clone()),
                    program: definition.program.clone(),
                    programs: delivery
                        .map(|delivery| delivery.programs.clone())
                        .unwrap_or_default(),
                    note: delivery.and_then(|delivery| delivery.note.clone()),
                    requirements: delivery
                        .map(|delivery| delivery.requirements.clone())
                        .unwrap_or_default(),
                    capabilities: definition.capabilities.clone(),
                    platforms: definition.platforms(),
                    supported_here: definition.supported_here(),
                    platform: platform_key(),
                    system_entry,
                    current_version,
                    installed,
                    size_bytes,
                    status,
                    model_engine_ids: definition.model_engine_ids.clone(),
                }
            })
            .filter(|asset| {
                asset.supported_here
                    || !asset.installed.is_empty()
                    || asset.current_version.is_some()
                    || asset.system_entry.is_some()
            })
            .collect()
    }

    /// The state of one engine.
    pub fn get(&self, engine_id: &str) -> Option<EngineAsset> {
        self.list().into_iter().find(|asset| asset.id == engine_id)
    }

    /// The versions that can be installed.
    pub async fn versions(&self, engine_id: &str) -> TtsResult<Vec<EngineVersion>> {
        let definition =
            engine_definition(engine_id).ok_or_else(|| TtsError::InvalidRequest(format!("未知引擎 `{engine_id}`")))?;
        let installed: Vec<String> = {
            let manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
            manifest
                .engines
                .get(engine_id)
                .map(|e| e.installed.iter().map(|i| i.version.clone()).collect())
                .unwrap_or_default()
        };
        let delivery = definition.delivery_here().cloned();

        let mut versions = match delivery.as_ref().map(|d| d.distribution).unwrap_or(definition.distribution) {
            EngineDistribution::Binary => self.github_versions(&definition).await?,
            EngineDistribution::PythonPackage => {
                let package = delivery
                    .as_ref()
                    .and_then(|delivery| delivery.package.clone())
                    .ok_or_else(|| {
                        TtsError::Internal(format!("引擎 `{}` 没有 Python 包", definition.id))
                    })?;
                self.pypi_versions(&definition, &package).await?
            }
            EngineDistribution::SourceBuild => {
                let delivery = delivery.clone().unwrap_or_else(|| delivery_of(&definition));
                self.source_versions(&definition, &delivery).await?
            }
        };
        for version in &mut versions {
            version.installed = installed.contains(&version.version);
        }
        Ok(versions)
    }

    /// The versions published on GitHub.
    async fn github_versions(
        &self,
        definition: &EngineDefinition,
    ) -> TtsResult<Vec<EngineVersion>> {
        let repository = definition
            .repository
            .as_ref()
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{}` 没有仓库", definition.id)))?;
        let url = format!("{GITHUB_API}/repos/{repository}/releases?per_page=30");
        let releases: Vec<GithubRelease> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Internal(format!("无法访问 GitHub：{err}")))?
            .error_for_status()
            .map_err(|err| TtsError::Internal(format!("GitHub 返回错误：{err}")))?
            .json()
            .await
            .map_err(|err| TtsError::Internal(format!("GitHub 响应解析失败：{err}")))?;

        let mut versions = Vec::new();
        for release in releases {
            // Some releases only carry a marker file, the binaries live in the
            // build tags below them. Only the releases with an asset for this
            // platform are installable.
            let asset = select_asset(&release.assets, &platform_key());
            let Some(asset) = asset else { continue };
            let is_build = definition
                .build_tags
                .iter()
                .any(|prefix| release.tag_name.starts_with(prefix.as_str()));
            versions.push(EngineVersion {
                version: release.tag_name.trim_start_matches('v').to_string(),
                published_at: release.published_at,
                prerelease: release.prerelease && !is_build,
                size_bytes: Some(asset.size),
                asset_name: Some(asset.name.clone()),
                download_url: Some(asset.browser_download_url.clone()),
                installed: false,
            });
        }
        Ok(versions)
    }

    /// The versions that are published as tags of the repository.
    ///
    /// The source builds are installed from a checkout of a tag, so the tags
    /// are the versions.
    async fn source_versions(
        &self,
        definition: &EngineDefinition,
        delivery: &PlatformDelivery,
    ) -> TtsResult<Vec<EngineVersion>> {
        let repository = definition
            .repository
            .as_ref()
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{}` 没有仓库", definition.id)))?;

        // A branch is followed instead of a tag. The native runtime of the
        // Apple path is only complete on the main branch, the release tags do
        // not carry the servers of the newest models yet.
        if delivery.source_ref == SourceRef::Branch {
            return Ok(vec![EngineVersion {
                version: "main".to_string(),
                published_at: None,
                prerelease: false,
                size_bytes: None,
                asset_name: Some(format!("{repository}-main.tar.gz")),
                download_url: Some(format!(
                    "https://github.com/{repository}/archive/refs/heads/main.tar.gz"
                )),
                installed: false,
            }]);
        }

        let url = format!("{GITHUB_API}/repos/{repository}/tags?per_page=50");
        let tags: Vec<GithubTag> = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Internal(format!("无法访问 GitHub：{err}")))?
            .error_for_status()
            .map_err(|err| TtsError::Internal(format!("GitHub 返回错误：{err}")))?
            .json()
            .await
            .map_err(|err| TtsError::Internal(format!("GitHub 响应解析失败：{err}")))?;

        let mut versions: Vec<EngineVersion> = Vec::new();
        for tag in tags {
            // The tags of the macOS application describe the same tree, but
            // the package tags are the release channel of the engine.
            if !is_release_tag(&tag.name) {
                continue;
            }
            versions.push(EngineVersion {
                version: tag.name.clone(),
                published_at: None,
                prerelease: false,
                size_bytes: None,
                asset_name: Some(format!("{repository}-{}.tar.gz", tag.name)),
                download_url: Some(format!(
                    "https://github.com/{repository}/archive/refs/tags/{}.tar.gz",
                    tag.name
                )),
                installed: false,
            });
        }
        versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
        Ok(versions)
    }

    /// The versions published on PyPI.
    async fn pypi_versions(
        &self,
        _definition: &EngineDefinition,
        package: &str,
    ) -> TtsResult<Vec<EngineVersion>> {
        let url = format!("{PYPI_API}/{package}/json");
        let response: PypiResponse = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Internal(format!("无法访问 PyPI：{err}")))?
            .error_for_status()
            .map_err(|err| TtsError::Internal(format!("PyPI 返回错误：{err}")))?
            .json()
            .await
            .map_err(|err| TtsError::Internal(format!("PyPI 响应解析失败：{err}")))?;

        let mut versions: Vec<EngineVersion> = Vec::new();
        for (version, files) in &response.releases {
            if files.is_empty() {
                continue;
            }
            let file = files
                .iter()
                .find(|f| f.packagetype == "bdist_wheel")
                .or_else(|| files.first());
            let Some(file) = file else { continue };
            if file.yanked {
                continue;
            }
            versions.push(EngineVersion {
                version: version.clone(),
                published_at: file.upload_time.clone(),
                prerelease: version.contains("rc") || version.contains("dev"),
                size_bytes: Some(file.size),
                asset_name: Some(file.filename.clone()),
                download_url: None,
                installed: false,
            });
        }
        versions.sort_by(|a, b| compare_versions(&b.version, &a.version));
        Ok(versions)
    }

    /// Starts the installation of a version.
    ///
    /// The installation runs in the background; the progress is polled with
    /// [`Self::install_progress`].
    pub fn start_install(self: &Arc<Self>, engine_id: &str, version: Option<String>) -> TtsResult<()> {
        let definition =
            engine_definition(engine_id).ok_or_else(|| TtsError::InvalidRequest(format!("未知引擎 `{engine_id}`")))?;
        if !definition.supported_here() {
            return Err(TtsError::IncompatibleHardware {
                model: definition.name.clone(),
                reason: format!("当前平台 {} 不受支持", platform_key()),
            });
        }
        {
            let progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = progress.get(engine_id) {
                if existing.state == "running" {
                    return Err(TtsError::EngineBusy(format!(
                        "引擎 `{engine_id}` 正在安装 {}",
                        existing.version
                    )));
                }
            }
        }
        let manager = self.clone();
        tokio::spawn(async move {
            manager.run_install(&definition, version).await;
        });
        Ok(())
    }

    /// Installs a version.
    async fn run_install(self: &Arc<Self>, definition: &EngineDefinition, version: Option<String>) {
        let version = match version {
            Some(version) => version,
            None => match self.latest_version(&definition.id).await {
                Ok(version) => version,
                Err(err) => {
                    self.fail_install(&definition.id, &String::new(), err.to_string());
                    return;
                }
            },
        };

        self.set_progress(EngineInstallProgress {
            engine_id: definition.id.clone(),
            version: version.clone(),
            state: "running".to_string(),
            percent: Some(0.0),
            bytes: 0,
            total_bytes: None,
            message: format!("准备安装 {version}"),
            log: Vec::new(),
            error: None,
        });

        let target = self.version_dir(&definition.id, &version);
        if target.exists() {
            self.fail_install(
                &definition.id,
                &version,
                format!("版本 {version} 已经安装"),
            );
            return;
        }
        if let Err(err) = std::fs::create_dir_all(&target) {
            self.fail_install(&definition.id, &version, err.to_string());
            return;
        }

        let delivery = definition.delivery_here().cloned();
        let result = match delivery
            .as_ref()
            .map(|delivery| delivery.distribution)
            .unwrap_or(definition.distribution)
        {
            EngineDistribution::Binary => {
                self.install_binary(definition, &version, &target).await
            }
            EngineDistribution::PythonPackage => {
                self.install_python(definition, &version, &target).await
            }
            EngineDistribution::SourceBuild => {
                let delivery = match delivery.as_ref() {
                    Some(delivery) => delivery,
                    None => {
                        self.fail_install(
                            &definition.id,
                            &version,
                            format!("当前平台 {} 不受支持", platform_key()),
                        );
                        return;
                    }
                };
                self.install_source(definition, delivery, &version, &target).await
            }
        };

        match result {
            Ok(entry) => {
                let size_bytes = directory_size(&target).unwrap_or(0);
                let installed = InstalledEngine {
                    version: version.clone(),
                    path: target.clone(),
                    entry: entry.clone(),
                    installed_at: chrono::Utc::now().to_rfc3339(),
                    size_bytes,
                    source: match delivery
                        .as_ref()
                        .map(|delivery| delivery.distribution)
                        .unwrap_or(definition.distribution)
                    {
                        EngineDistribution::Binary => "github".to_string(),
                        EngineDistribution::PythonPackage => "pypi".to_string(),
                        EngineDistribution::SourceBuild => "source".to_string(),
                    },
                };
                {
                    // The manifest is read again, so that the installation of
                    // another process is not dropped.
                    if let Err(err) = self.load_manifest() {
                        tracing::warn!(error = %err, "无法重新读取引擎清单");
                    }
                    let mut manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
                    let entry = manifest.engines.entry(definition.id.clone()).or_default();
                    entry.installed.push(installed);
                    if entry.current.is_none() {
                        entry.current = Some(version.clone());
                    }
                }
                if let Err(err) = self.save_manifest() {
                    tracing::warn!(error = %err, "无法写入引擎清单");
                }
                self.update_progress(&definition.id, |progress| {
                    progress.state = "done".to_string();
                    progress.percent = Some(100.0);
                    progress.message = format!("{version} 安装完成");
                });
                tracing::info!(engine = %definition.id, %version, "引擎安装完成");
            }
            Err(err) => {
                let _ = std::fs::remove_dir_all(&target);
                self.fail_install(&definition.id, &version, err.to_string());
            }
        }
    }

    /// The newest version of an engine.
    async fn latest_version(&self, engine_id: &str) -> TtsResult<String> {
        let versions = self.versions(engine_id).await?;
        versions
            .into_iter()
            .find(|v| !v.prerelease)
            .map(|v| v.version)
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{engine_id}` 没有可用版本")))
    }

    /// Downloads and unpacks a release archive.
    async fn install_binary(
        &self,
        definition: &EngineDefinition,
        version: &str,
        target: &Path,
    ) -> TtsResult<Option<PathBuf>> {
        let versions = self.github_versions(definition).await?;
        let release = versions
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| TtsError::InvalidRequest(format!("版本 {version} 没有发布资产")))?;
        let url = release
            .download_url
            .ok_or_else(|| TtsError::InvalidRequest(format!("版本 {version} 没有适配当前平台的资产")))?;
        let total = release.size_bytes;

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("下载 {version}");
            progress.total_bytes = total;
        });

        let archive = target.join("download");
        let mut response = self
            .download_client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Internal(format!("下载失败：{err}")))?
            .error_for_status()
            .map_err(|err| TtsError::Internal(format!("下载失败：{err}")))?;

        let file = std::fs::File::create(&archive).map_err(|e| TtsError::io(&archive, e))?;
        let mut writer = std::io::BufWriter::new(file);
        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| TtsError::Internal(format!("下载中断：{err}")))?
        {
            use std::io::Write;
            writer
                .write_all(&chunk)
                .map_err(|e| TtsError::io(&archive, e))?;
            written += chunk.len() as u64;
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| written as f64 / total as f64 * 100.0);
            self.update_progress(&definition.id, |progress| {
                progress.bytes = written;
                progress.percent = percent;
            });
        }
        use std::io::Write;
        writer.flush().map_err(|e| TtsError::io(&archive, e))?;

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("解压 {version}");
        });
        let extract_dir = target.join("extract");
        std::fs::create_dir_all(&extract_dir).map_err(|e| TtsError::io(&extract_dir, e))?;
        let archive_name = release.asset_name.clone().unwrap_or_default();
        extract_archive(&archive, &extract_dir, &archive_name)?;
        let _ = std::fs::remove_file(&archive);

        // The release archives contain a `build/bin/<program>` layout, but the
        // exact prefix depends on the release, so the binary is searched.
        let entry = find_program(&extract_dir, &definition.program)
            .or_else(|| find_program(&extract_dir, "llama-cli"));
        if let Some(entry) = &entry {
            make_executable(entry)?;
        }
        Ok(entry)
    }

    /// Installs a Python package into a virtual environment.
    async fn install_python(
        self: &Arc<Self>,
        definition: &EngineDefinition,
        version: &str,
        target: &Path,
    ) -> TtsResult<Option<PathBuf>> {
        let package = definition
            .delivery_here()
            .and_then(|delivery| delivery.package.clone())
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{}` 没有 Python 包", definition.id)))?;
        // The server of some packages lives behind an extra, so the
        // requirement carries it.
        let requirement = match definition
            .delivery_here()
            .and_then(|delivery| delivery.extras.clone())
        {
            Some(extras) if !extras.is_empty() => format!("{package}[{extras}]=={version}"),
            _ => format!("{package}=={version}"),
        };

        self.update_progress(&definition.id, |progress| {
            progress.message = "创建虚拟环境".to_string();
            progress.percent = Some(10.0);
        });
        let venv = target.join("venv");
        // A delivery may name a Python version: the wheels of the package
        // are often not built for the Python of the machine, so `uv` builds
        // the environment with the version the package asks for.
        let python_version = definition
            .delivery_here()
            .and_then(|delivery| delivery.python_version.clone());
        let uv = uv_command();
        if let Some(version) = &python_version {
            let status = Command::new(&uv)
                .arg("venv")
                .arg("--python")
                .arg(version)
                .arg(&venv)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .status()
                .await
                .map_err(|err| TtsError::Internal(format!("无法创建虚拟环境：{err}")))?;
            if !status.success() {
                return Err(TtsError::Internal(format!(
                    "虚拟环境创建失败（{uv} venv --python {version}）：{status}"
                )));
            }
        } else {
            let python = python_command();
            let status = Command::new(&python)
                .arg("-m")
                .arg("venv")
                .arg(&venv)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .status()
                .await
                .map_err(|err| TtsError::Internal(format!("无法创建虚拟环境：{err}")))?;
            if !status.success() {
                return Err(TtsError::Internal(format!(
                    "虚拟环境创建失败（{python}）：{status}"
                )));
            }
        }

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("安装 {requirement}");
            progress.percent = Some(25.0);
        });
        let mut install = Command::new(&uv);
        if python_version.is_some() {
            // The environment of `uv` has no pip, the package is installed
            // with `uv pip` into it.
            install
                .arg("pip")
                .arg("install")
                .arg("--python")
                .arg(venv.join("bin").join("python"));
        } else {
            install = Command::new(venv.join("bin").join("pip"));
            install
                .arg("install")
                .arg("--disable-pip-version-check")
                .arg("--no-input");
        }
        let mut child = install
            .arg(&requirement)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| TtsError::Internal(format!("pip 调用失败：{err}")))?;

        // The output of pip is streamed into the installation log.
        let engine_id = definition.id.clone();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let readers = vec![
            tokio::spawn(stream_lines(stdout, self.clone(), engine_id.clone())),
            tokio::spawn(stream_lines(stderr, self.clone(), engine_id.clone())),
        ];
        let status = child.wait().await;
        for reader in readers {
            let _ = reader.await;
        }
        let status = status.map_err(|err| TtsError::Internal(format!("pip 调用失败：{err}")))?;
        if !status.success() {
            let tail = self
                .install_progress(&definition.id)
                .map(|progress| progress.log.join("\n"))
                .unwrap_or_default();
            return Err(TtsError::Internal(format!(
                "pip 安装失败：{}",
                tail.lines().last().unwrap_or("未知错误")
            )));
        }

        let entry = venv.join("bin").join(&definition.program);
        if !entry.exists() {
            return Err(TtsError::Internal(format!(
                "安装完成但找不到 {}",
                entry.display()
            )));
        }
        Ok(Some(entry))
    }

    /// Installs an engine from a source checkout.
    ///
    /// The checkout of the version is downloaded, unpacked and the install
    /// script of the repository builds it. The output of the script is
    /// streamed into the installation log.
    async fn install_source(
        self: &Arc<Self>,
        definition: &EngineDefinition,
        delivery: &PlatformDelivery,
        version: &str,
        target: &Path,
    ) -> TtsResult<Option<PathBuf>> {
        let repository = definition
            .repository
            .as_ref()
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{}` 没有仓库", definition.id)))?;
        let script = delivery
            .install_script
            .as_ref()
            .ok_or_else(|| TtsError::Internal(format!("引擎 `{}` 没有安装脚本", definition.id)))?;

        // The tools the script needs have to be there before it starts.
        for requirement in &delivery.requirements {
            if find_in_path(requirement).is_none() && !requirement.contains("python") {
                return Err(TtsError::IncompatibleHardware {
                    model: definition.name.clone(),
                    reason: format!("缺少 {requirement}，请先安装后再试"),
                });
            }
        }

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("下载源码 {version}");
            progress.percent = Some(5.0);
        });
        let url = match delivery.source_ref {
            SourceRef::Tag => format!(
                "https://github.com/{repository}/archive/refs/tags/{version}.tar.gz"
            ),
            SourceRef::Branch => format!(
                "https://github.com/{repository}/archive/refs/heads/{version}.tar.gz"
            ),
        };
        let archive = target.join("source.tar.gz");
        self.download_file(&definition.id, &url, &archive).await?;

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("解压源码 {version}");
            progress.percent = Some(15.0);
        });
        let checkout = target.join("checkout");
        std::fs::create_dir_all(&checkout).map_err(|e| TtsError::io(&checkout, e))?;
        extract_archive(&archive, &checkout, &format!("{repository}-{version}.tar.gz"))?;
        let _ = std::fs::remove_file(&archive);

        // The archive holds one directory that starts with the repository
        // name; its content is moved up, so the script path is stable.
        let root = std::fs::read_dir(&checkout)
            .map_err(|e| TtsError::io(&checkout, e))?
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_dir())
            .ok_or_else(|| TtsError::Internal("源码包是空的".to_string()))?;
        let work = target.join("source");
        std::fs::rename(&root, &work).map_err(|e| TtsError::io(&root, e))?;
        let _ = std::fs::remove_dir_all(&checkout);
        self.append_log(
            &definition.id,
            &format!("源码已解压到 {}", work.display()),
        );

        let script_path = work.join(script);
        if !script_path.exists() {
            return Err(TtsError::Internal(format!(
                "源码里没有安装脚本 {}",
                script_path.display()
            )));
        }
        let args: Vec<String> = delivery
            .script_args
            .iter()
            .map(|arg| arg.replace("{target}", &target.to_string_lossy()))
            .collect();

        self.update_progress(&definition.id, |progress| {
            progress.message = format!("执行 {script}");
            progress.percent = Some(20.0);
        });
        let mut command = Command::new("bash");
        command
            .arg(&script_path)
            .args(&args)
            .current_dir(&work)
            .env("NONINTERACTIVE", "1")
            .env("HOMEBREW_NO_AUTO_UPDATE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|err| TtsError::Internal(format!("安装脚本调用失败：{err}")))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let readers = vec![
            tokio::spawn(stream_lines(stdout, self.clone(), definition.id.clone())),
            tokio::spawn(stream_lines(stderr, self.clone(), definition.id.clone())),
        ];
        let status = child.wait().await;
        for reader in readers {
            let _ = reader.await;
        }
        let status = status.map_err(|err| TtsError::Internal(format!("安装脚本调用失败：{err}")))?;
        if !status.success() {
            let tail = self
                .install_progress(&definition.id)
                .map(|progress| progress.log.join("\n"))
                .unwrap_or_default();
            return Err(TtsError::Internal(format!(
                "安装脚本失败：{}",
                tail.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("未知错误")
            )));
        }

        let entry = delivery
            .program_path
            .as_ref()
            .map(|path| target.join(path))
            .filter(|path| path.exists());
        if entry.is_none() {
            // The script may put the program somewhere else in the checkout.
            let entry = find_program(&work, &definition.program)
                .or_else(|| find_program(&target, &definition.program));
            return Ok(entry);
        }
        Ok(entry)
    }

    /// Downloads a file into a path.
    async fn download_file(
        &self,
        engine_id: &str,
        url: &str,
        archive: &Path,
    ) -> TtsResult<()> {
        let mut response = self
            .download_client
            .get(url)
            .send()
            .await
            .map_err(|err| TtsError::Internal(format!("下载失败：{err}")))?
            .error_for_status()
            .map_err(|err| TtsError::Internal(format!("下载失败：{err}")))?;
        let total = response.content_length();
        let file = std::fs::File::create(archive).map_err(|e| TtsError::io(archive, e))?;
        let mut writer = std::io::BufWriter::new(file);
        let mut written = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| TtsError::Internal(format!("下载中断：{err}")))?
        {
            use std::io::Write;
            writer
                .write_all(&chunk)
                .map_err(|e| TtsError::io(archive, e))?;
            written += chunk.len() as u64;
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| written as f64 / total as f64 * 100.0);
            self.update_progress(engine_id, |progress| {
                progress.bytes = written;
                progress.total_bytes = total;
                progress.percent = percent;
            });
        }
        use std::io::Write;
        writer.flush().map_err(|e| TtsError::io(archive, e))?;
        Ok(())
    }

    /// Removes an installed version.
    pub fn uninstall(&self, engine_id: &str, version: &str) -> TtsResult<()> {
        // The manifest is shared by every process that uses the data
        // directory (the desktop application starts its own backend), so it
        // is read again before it is changed. A stale copy would drop the
        // installations of the other processes.
        self.load_manifest()?;
        let mut manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        let entry = manifest
            .engines
            .get_mut(engine_id)
            .ok_or_else(|| TtsError::InvalidRequest(format!("引擎 `{engine_id}` 没有安装记录")))?;
        let installed = entry
            .installed
            .iter()
            .position(|i| i.version == version)
            .ok_or_else(|| TtsError::InvalidRequest(format!("版本 {version} 未安装")))?;
        let record = entry.installed.remove(installed);
        if entry.current.as_deref() == Some(version) {
            entry.current = entry.installed.last().map(|i| i.version.clone());
        }
        drop(manifest);
        if let Err(err) = std::fs::remove_dir_all(&record.path) {
            tracing::warn!(error = %err, path = %record.path.display(), "无法删除引擎目录");
        }
        tracing::info!(engine = %engine_id, version = %version, "引擎版本已卸载");
        self.save_manifest()
    }

    /// Activates an installed version.
    pub fn activate(&self, engine_id: &str, version: &str) -> TtsResult<()> {
        self.load_manifest()?;
        let mut manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        let entry = manifest
            .engines
            .get_mut(engine_id)
            .ok_or_else(|| TtsError::InvalidRequest(format!("引擎 `{engine_id}` 没有安装记录")))?;
        if !entry.installed.iter().any(|i| i.version == version) {
            return Err(TtsError::InvalidRequest(format!("版本 {version} 未安装")));
        }
        entry.current = Some(version.to_string());
        drop(manifest);
        self.save_manifest()
    }

    /// The program that serves the engine.
    ///
    /// A system wide installation wins over the managed one, because it is the
    /// build the models were tested with.
    pub fn resolve_entry(&self, engine_id: &str) -> Option<PathBuf> {
        let definition = engine_definition(engine_id)?;
        let managed = self.managed_entry(&definition);
        if !definition.managed_model_engine_ids.is_empty() {
            managed.or_else(|| find_in_path(&definition.program))
        } else {
            definition
                .system_program
                .as_ref()
                .and_then(|program| find_in_path(program))
                .or(managed)
        }
    }

    /// The program that serves the models of the given engine id.
    ///
    /// This is the lookup the inference adapter uses: it prefers the managed
    /// installation of the engine and falls back to the system wide build.
    pub fn resolve_program(&self, model_engine_id: &str) -> Option<PathBuf> {
        for definition in engine_definitions() {
            if !definition.serves(model_engine_id) {
                continue;
            }
            if definition
                .managed_model_engine_ids
                .iter()
                .any(|id| id == model_engine_id)
            {
                // The runtime ships one server per model family, the id of the
                // model decides which one is started.
                let program = definition
                    .model_programs
                    .iter()
                    .find(|(id, _)| id == model_engine_id)
                    .map(|(_, program)| program.clone());
                if let Some(program) = program {
                    if let Some(directory) = self.managed_dir(&definition) {
                        if let Some(path) = find_program(&directory, &program) {
                            return Some(self.repair_launcher(&path));
                        }
                    }
                }
                if let Some(entry) = self.managed_entry(&definition) {
                    return Some(self.repair_launcher(&entry));
                }
            }
            if let Some(system) = definition
                .system_program
                .as_ref()
                .and_then(|program| find_in_path(program))
            {
                return Some(system);
            }
            if let Some(system) = find_in_path(&definition.program) {
                return Some(system);
            }
        }
        None
    }

    /// Repairs a launcher of a Python environment whose data directory has
    /// been moved: the script still names the interpreter of the old place,
    /// so the engine refuses to start.
    fn repair_launcher(&self, path: &Path) -> PathBuf {
        if let Some(repaired) = repair_shebang(path, self.paths.root()) {
            tracing::info!(
                path = %repaired.display(),
                "the launcher of the engine was repaired after the data directory moved"
            );
            return repaired;
        }
        path.to_path_buf()
    }

    /// The directory of the managed installation that is active.
    fn managed_dir(&self, definition: &EngineDefinition) -> Option<PathBuf> {
        let manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        let entry = manifest.engines.get(&definition.id)?;
        let current = entry.current.as_ref()?;
        let installed = entry.installed.iter().find(|i| &i.version == current)?;
        let path = installed.path.clone();
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
    }

    /// The program of the managed installation that is active.
    fn managed_entry(&self, definition: &EngineDefinition) -> Option<PathBuf> {
        let manifest = self.manifest.lock().unwrap_or_else(|e| e.into_inner());
        let entry = manifest.engines.get(&definition.id)?;
        let current = entry.current.as_ref()?;
        let installed = entry.installed.iter().find(|i| &i.version == current)?;
        let entry = installed.entry.clone()?;
        if entry.exists() {
            Some(entry)
        } else {
            None
        }
    }

    /// The progress of a running installation.
    pub fn install_progress(&self, engine_id: &str) -> Option<EngineInstallProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(engine_id)
            .cloned()
    }

    /// Updates the progress of an installation.
    fn update_progress(
        &self,
        engine_id: &str,
        update: impl FnOnce(&mut EngineInstallProgress),
    ) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = progress.get_mut(engine_id) {
            update(entry);
        }
    }

    /// Marks an installation as failed.
    fn fail_install(&self, engine_id: &str, version: &str, error: String) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        progress.insert(
            engine_id.to_string(),
            EngineInstallProgress {
                engine_id: engine_id.to_string(),
                version: version.to_string(),
                state: "failed".to_string(),
                percent: None,
                bytes: 0,
                total_bytes: None,
                message: format!("安装失败：{error}"),
                log: Vec::new(),
                error: Some(error.clone()),
            },
        );
        drop(progress);
        tracing::warn!(engine = engine_id, version = version, error = %error, "引擎安装失败");
    }

    /// Sets the progress of an installation.
    fn set_progress(&self, progress: EngineInstallProgress) {
        let mut entries = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(progress.engine_id.clone(), progress);
    }

    /// Appends lines to the installation log.
    fn append_log(&self, engine_id: &str, text: &str) {
        let mut entries = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = entries.get_mut(engine_id) else {
            return;
        };
        for line in text.lines() {
            entry.log.push(line.to_string());
        }
        if entry.log.len() > LOG_TAIL {
            let excess = entry.log.len() - LOG_TAIL;
            entry.log.drain(0..excess);
        }
    }
}

/// The platform key of the current machine.
pub fn platform_key() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64".to_string(),
        ("macos", "x86_64") => "darwin-x64".to_string(),
        ("linux", "x86_64") => "linux-x64".to_string(),
        ("linux", "aarch64") => "linux-arm64".to_string(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

/// The asset name fragment that matches the platform.
fn platform_asset_token(platform: &str) -> Option<&'static str> {
    match platform {
        "darwin-arm64" => Some("macos-arm64"),
        "darwin-x64" => Some("macos-x64"),
        "linux-x64" => Some("ubuntu-x64"),
        "linux-arm64" => Some("ubuntu-arm64"),
        _ => None,
    }
}

/// Picks the release asset that fits the platform.
fn select_asset<'a>(assets: &'a [GithubAsset], platform: &str) -> Option<&'a GithubAsset> {
    let token = platform_asset_token(platform)?;
    let matching = assets
        .iter()
        .filter(|asset| asset.name.contains(token) && asset.name.contains("bin"))
        .collect::<Vec<_>>();
    matching
        .iter()
        .find(|asset| asset.name.ends_with(".tar.gz"))
        .or_else(|| matching.iter().find(|asset| asset.name.ends_with(".zip")))
        .copied()
}

/// Searches a directory for a program.
fn find_program(root: &Path, program: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut found: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some(program) {
                found.push(path);
            }
        }
    }
    // Das installierte Laufzeitverzeichnis hat Vorrang vor dem Bauverzeichnis:
    // dort liegen die Programme, die der Nutzer bekommt. Die Programme im
    // Bauverzeichnis tragen den Pfad der Bauumgebung in sich und laufen
    // nur, solange diese Umgebung dort steht.
    found.sort_by_key(|path| {
        let components: Vec<_> = path.components().collect();
        match components
            .iter()
            .position(|component| component.as_os_str() == "runtime")
        {
            Some(index) => (0, index),
            None => (1, 0),
        }
    });
    found.into_iter().next()
}

/// Makes a file executable.
fn make_executable(path: &Path) -> TtsResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .map_err(|e| TtsError::io(path, e))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions).map_err(|e| TtsError::io(path, e))
}

/// Extracts a release archive.
fn extract_archive(archive: &Path, target: &Path, name: &str) -> TtsResult<()> {
    if name.ends_with(".zip") {
        let file = std::fs::File::open(archive).map_err(|e| TtsError::io(archive, e))?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|err| TtsError::Internal(format!("无法打开压缩包：{err}")))?;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|err| TtsError::Internal(format!("压缩包损坏：{err}")))?;
            let path = entry
                .enclosed_name()
                .ok_or_else(|| TtsError::Internal("压缩包包含非法路径".to_string()))?;
            let out = target.join(path);
            if entry.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| TtsError::io(&out, e))?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| TtsError::io(parent, e))?;
            }
            let mut out_file =
                std::fs::File::create(&out).map_err(|e| TtsError::io(&out, e))?;
            std::io::copy(&mut entry, &mut out_file).map_err(|e| TtsError::io(&out, e))?;
        }
        return Ok(());
    }

    let file = std::fs::File::open(archive).map_err(|e| TtsError::io(archive, e))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(target)
        .map_err(|err| TtsError::Internal(format!("解压失败：{err}")))
}

/// Searches the `PATH` for a program.
fn find_in_path(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The size of a directory in bytes.
fn directory_size(path: &Path) -> TtsResult<u64> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(metadata) = entry.metadata() {
                total += metadata.len();
            }
        }
    }
    Ok(total)
}

/// The command that runs Python on this machine.
fn python_command() -> String {
    std::env::var("LM_LATHE_PYTHON").unwrap_or_else(|_| "python3".to_string())
}

/// The command that builds a virtual environment with a named Python version.
fn uv_command() -> String {
    std::env::var("LM_LATHE_UV").unwrap_or_else(|_| "uv".to_string())
}

/// Reads a pipe of the child process and appends the lines to the log.
async fn stream_lines<R>(
    reader: Option<R>,
    manager: Arc<EngineAssetManager>,
    engine_id: String,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let Some(reader) = reader else { return };
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        manager.append_log(&engine_id, &line);
    }
}

/// Compares two version strings, newest first.
fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let parse = |value: &str| -> Vec<u64> {
        value
            .trim_start_matches('v')
            .split(|c: char| !c.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    parse(left).cmp(&parse(right))
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

/// A tag of a repository.
#[derive(Debug, Deserialize)]
struct GithubTag {
    name: String,
}

/// `true` when a tag marks a release of the package.
///
/// The repository also carries the tags of its macOS application
/// (`voxt-v…`); those describe the same tree, but the release channel of the
/// engine is the `v…` tag.
fn is_release_tag(tag: &str) -> bool {
    tag.starts_with('v')
        && tag.len() > 1
        && tag[1..]
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c.is_ascii_alphabetic())
        && !tag.contains("voxt")
}

#[derive(Debug, Deserialize)]
struct PypiResponse {
    #[serde(default)]
    releases: BTreeMap<String, Vec<PypiFile>>,
}

#[derive(Debug, Deserialize)]
struct PypiFile {
    #[serde(default)]
    filename: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    packagetype: String,
    #[serde(default)]
    upload_time: Option<String>,
    #[serde(default)]
    yanked: bool,
}

/// Starts the engine service and probes its `/health` endpoint.
pub async fn probe_engine(
    definition: &EngineDefinition,
    entry: &Path,
    model: Option<(&Path, &str)>,
) -> EngineHealth {
    let started = Instant::now();
    let Some((model, model_name)) = model else {
        return EngineHealth {
            engine_id: definition.id.clone(),
            status: "no-model".to_string(),
            detail: "没有绑定到该引擎的已安装模型，无法启动服务探活。".to_string(),
            entry: Some(entry.to_path_buf()),
            endpoint: None,
            duration_ms: started.elapsed().as_millis() as u64,
        };
    };

    let Ok(port) = free_port() else {
        return EngineHealth {
            engine_id: definition.id.clone(),
            status: "unhealthy".to_string(),
            detail: "无法分配端口。".to_string(),
            entry: Some(entry.to_path_buf()),
            endpoint: None,
            duration_ms: started.elapsed().as_millis() as u64,
        };
    };

    let args: Vec<String> = definition
        .server_args
        .iter()
        .map(|arg| {
            arg.replace("{port}", &port.to_string())
                .replace("{model}", &model.to_string_lossy())
                .replace("{model_name}", model_name)
        })
        .collect();

    let mut child = match Command::new(entry)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return EngineHealth {
                engine_id: definition.id.clone(),
                status: "unhealthy".to_string(),
                detail: format!("无法启动 {}：{err}", entry.display()),
                entry: Some(entry.to_path_buf()),
                endpoint: None,
                duration_ms: started.elapsed().as_millis() as u64,
            }
        }
    };

    let endpoint = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut detail = String::new();
    let mut status = "unhealthy".to_string();

    while Instant::now() < deadline {
        if let Ok(Some(exit)) = child.try_wait() {
            detail = format!("引擎进程退出（{exit}）。");
            break;
        }
        match client.get(&endpoint).send().await {
            Ok(response) if response.status().is_success() => {
                status = "healthy".to_string();
                detail = format!("{} 响应正常。", endpoint);
                break;
            }
            Ok(response) => {
                detail = format!("{} 返回 {}。", endpoint, response.status());
            }
            Err(_) => {
                detail = format!("等待 {} 响应……", endpoint);
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }

    let _ = child.kill().await;

    EngineHealth {
        engine_id: definition.id.clone(),
        status,
        detail,
        entry: Some(entry.to_path_buf()),
        endpoint: Some(endpoint),
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Reserves a free TCP port.
fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// Writes the current data directory into the shebang of a launcher script
/// when the interpreter it names does not exist any more. A data directory
/// that was moved leaves its Python environments behind: the scripts of the
/// environment still point to the old path, and the engine dies with "No
/// such file or directory" before it starts.
///
/// Only the first line of the script is touched. The compiled files of the
/// environment keep their bytes, because a change inside them would break
/// them.
fn repair_shebang(entry: &Path, root: &Path) -> Option<PathBuf> {
    let data = std::fs::read(entry).ok()?;
    let end = data
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(data.len());
    let (first, rest) = data.split_at(end);
    let text = std::str::from_utf8(first).ok()?;
    let interpreter = text.strip_prefix("#!")?.trim();
    if !interpreter.starts_with('/') || Path::new(interpreter).exists() {
        return None;
    }
    // The interpreter always lives under the data directory:
    // "<root>/runtimes/engines/<engine>/<version>/...". Only the root has
    // changed, the rest of the path stays the same.
    let marker = "runtimes/engines/";
    let index = interpreter.find(marker)?;
    let candidate = root.join(&interpreter[index..]);
    if !candidate.exists() {
        return None;
    }
    let mut repaired = Vec::with_capacity(data.len() + 32);
    repaired.extend_from_slice(format!("#!{}", candidate.display()).as_bytes());
    repaired.push(b'\n');
    repaired.extend_from_slice(rest);
    std::fs::write(entry, &repaired).ok()?;
    Some(entry.to_path_buf())
}

#[cfg(test)]
mod shebang_tests {
    use super::repair_shebang;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn a_launcher_of_a_moved_directory_is_repaired() {
        let root = PathBuf::from("/tmp/lm-lathe-test-root");
        let version = root.join("runtimes/engines/sglang-omni/v0.2.0");
        fs::create_dir_all(version.join("venv/bin")).unwrap();
        let interpreter = version.join("venv/bin/python");
        fs::write(&interpreter, b"").unwrap();

        let script = version.join("venv/bin/sgl-omni");
        fs::write(
            &script,
            "#!/old/place/runtimes/engines/sglang-omni/v0.2.0/venv/bin/python\nfrom app import main\n",
        )
        .unwrap();

        let repaired = repair_shebang(&script, &root).expect("the launcher is repaired");
        let text = fs::read_to_string(&repaired).unwrap();
        assert!(text.starts_with("#!/tmp/lm-lathe-test-root/runtimes/engines/"));
        assert!(text.ends_with("from app import main\n"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_launcher_with_a_working_interpreter_stays_untouched() {
        let root = PathBuf::from("/tmp/lm-lathe-test-root-2");
        fs::create_dir_all(&root).unwrap();
        let script = root.join("script");
        fs::write(&script, "#!/bin/sh\necho hallo\n").unwrap();

        assert!(repair_shebang(&script, &root).is_none());
        assert_eq!(fs::read_to_string(&script).unwrap(), "#!/bin/sh\necho hallo\n");

        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod list_tests {
    use super::{platform_key, EngineAssetManager};
    use crate::paths::AppPaths;

    /// An engine that has no delivery for this platform cannot run here and
    /// is not listed. The engines that run everywhere stay.
    #[test]
    fn an_engine_without_a_delivery_for_this_platform_is_not_listed() {
        let root = std::env::temp_dir().join(format!("lm-lathe-engines-{}", std::process::id()));
        let manager = EngineAssetManager::new(AppPaths::with_root(&root)).expect("the manager starts");
        let listed: Vec<String> = manager
            .list()
            .into_iter()
            .map(|asset| asset.id)
            .collect();

        let on_linux = platform_key().starts_with("linux");
        let on_macos = platform_key().starts_with("darwin");

        // The torch runtime of SGLang-Omni has no Apple Silicon delivery any
        // more: it crashed in the MPS backend.
        assert_eq!(
            listed.contains(&"sglang-omni".to_string()),
            on_linux,
            "the torch runtime is listed on {}",
            platform_key()
        );
        // The MLX runtimes only exist on Apple Silicon.
        assert_eq!(listed.contains(&"mlx-audio".to_string()), on_macos);
        assert_eq!(listed.contains(&"mlx".to_string()), on_macos);
        assert_eq!(
            listed.contains(&"sglang-omni-mlx".to_string()),
            on_macos
        );
        // llama.cpp runs everywhere.
        assert!(listed.contains(&"llama.cpp".to_string()));

        let _ = std::fs::remove_dir_all(&root);
    }
}
