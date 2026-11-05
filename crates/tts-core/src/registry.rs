//! Registry of the TTS models known to the application.
//!
//! The registry is the single source of truth for
//!
//! * which models exist (id, version, size, format),
//! * which engine serves them,
//! * which capabilities a model exposes,
//! * how much memory a model is expected to need, and
//! * whether the current machine can run the model at all.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{TtsError, TtsResult};
use crate::hardware::HardwareProfile;
use crate::types::{EngineMode, ModelFamily, TtsFeature};

/// Hardware requirements of a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareRequirement {
    /// Platforms the model is supported on, e.g. `["linux", "macos"]`.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// A CUDA capable NVIDIA GPU is required.
    #[serde(default)]
    pub require_cuda: bool,
    /// The model needs Apple Silicon.
    #[serde(default)]
    pub require_apple_silicon: bool,
    /// The model needs the native MLX runtime.
    #[serde(default)]
    pub require_mlx: bool,
    /// Minimum amount of physical memory in bytes.
    #[serde(default)]
    pub min_total_ram_bytes: u64,
    /// Minimum CUDA compute capability, when the checkpoint is quantised for it.
    #[serde(default)]
    pub min_cuda_compute: Option<u32>,
    /// Free-form note shown in the UI.
    #[serde(default)]
    pub notes: Option<String>,
}

impl Default for HardwareRequirement {
    fn default() -> Self {
        Self {
            platforms: vec!["linux".to_string(), "macos".to_string()],
            require_cuda: false,
            require_apple_silicon: false,
            require_mlx: false,
            min_total_ram_bytes: 0,
            min_cuda_compute: None,
            notes: None,
        }
    }
}

/// How well a model fits the current machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityLevel {
    /// Officially supported on this machine.
    Supported,
    /// Plausible but not verified by the upstream project.
    RequiresVerification,
    /// Cannot run on this machine.
    Unsupported,
}

impl CompatibilityLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            CompatibilityLevel::Supported => "supported",
            CompatibilityLevel::RequiresVerification => "requires_verification",
            CompatibilityLevel::Unsupported => "unsupported",
        }
    }
}

/// Result of a compatibility check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// Model the report belongs to.
    pub model_id: String,
    /// Verdict.
    pub level: CompatibilityLevel,
    /// `true` when the model may be installed and loaded.
    pub compatible: bool,
    /// Human readable explanations.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A TTS model known to the application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TTSModelSpec {
    /// Stable identifier used in the API and on disk.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Model family, selects the adapter.
    pub family: ModelFamily,
    /// Engine that serves the model.
    pub engine_id: String,
    /// Hugging Face repository id or local path.
    pub hf_repo: String,
    /// Version or revision of the checkpoint.
    #[serde(default)]
    pub version: String,
    /// Checkpoint format, e.g. `safetensors`.
    #[serde(default)]
    pub format: String,
    /// Size of the checkpoint on disk in bytes.
    #[serde(default)]
    pub size_bytes: u64,
    /// Memory the runtime is expected to occupy while serving.
    #[serde(default)]
    pub estimated_memory_bytes: u64,
    /// Capabilities of the model.
    #[serde(default)]
    pub features: Vec<TtsFeature>,
    /// Hardware requirements.
    #[serde(default)]
    pub hardware: HardwareRequirement,
    /// How the engine is launched.
    #[serde(default)]
    pub mode: EngineMode,
    /// Idle time-to-live override in seconds.
    #[serde(default)]
    pub default_ttl_secs: Option<u64>,
    /// Alternative names accepted by the API.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Extra command line arguments for the engine.
    #[serde(default)]
    pub launch_args: Vec<String>,
    /// Local directory the checkpoint lives in.
    #[serde(default)]
    pub local_path: Option<PathBuf>,
    /// Free-form note shown in the UI.
    #[serde(default)]
    pub notes: Option<String>,
}

impl TTSModelSpec {
    /// `true` when the checkpoint is present on disk.
    pub fn is_installed(&self) -> bool {
        self.local_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// `true` when the engine of the model is part of the application.
    ///
    /// The mock engine needs no checkpoint on the disk, so there is nothing
    /// to download for it.
    pub fn is_builtin(&self) -> bool {
        self.mode == EngineMode::Mock
    }

    /// `true` when the model exposes the given capability.
    pub fn supports(&self, feature: TtsFeature) -> bool {
        self.features.contains(&feature)
    }

    /// Directory name used underneath `models/tts/`.
    pub fn slug(&self) -> String {
        self.hf_repo.replace('/', "--")
    }
}

/// Registry of all known TTS models.
pub struct TTSModelRegistry {
    specs: RwLock<BTreeMap<String, TTSModelSpec>>,
    aliases: RwLock<BTreeMap<String, String>>,
    models_root: PathBuf,
}

impl Default for TTSModelRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl TTSModelRegistry {
    /// Creates an empty registry.
    pub fn empty() -> Self {
        Self {
            specs: RwLock::new(BTreeMap::new()),
            aliases: RwLock::new(BTreeMap::new()),
            models_root: crate::paths::AppPaths::discover().tts_models_root(),
        }
    }

    /// Creates a registry pre-filled with the models supported in M1.
    pub fn builtin() -> Self {
        let registry = Self::empty();
        for spec in builtin_models() {
            registry.insert(spec);
        }
        registry
    }

    /// Sets the directory the checkpoints live in.
    pub fn with_models_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.models_root = root.into();
        self
    }

    /// The directory the checkpoints live in.
    pub fn models_root(&self) -> &Path {
        &self.models_root
    }

    /// The directory a checkpoint is stored in.
    ///
    /// A model that carries an explicit `local_path` keeps it, every other
    /// model is stored below the model root.
    pub fn install_path(&self, id: &str) -> Option<PathBuf> {
        let spec = self.resolve(id)?;
        Some(
            spec.local_path
                .clone()
                .unwrap_or_else(|| self.models_root.join(spec.slug())),
        )
    }

    /// `true` when the checkpoint of the model is on the disk.
    ///
    /// A model whose engine is part of the application is always available.
    /// A checkpoint that is only half downloaded does not count: it carries
    /// no weights yet.
    pub fn is_installed(&self, id: &str) -> bool {
        if let Some(spec) = self.resolve(id) {
            if spec.is_builtin() {
                return true;
            }
        }
        self.install_path(id)
            .map(|path| has_weights(&path))
            .unwrap_or(false)
    }

    /// Adds or replaces a model.
    pub fn insert(&self, spec: TTSModelSpec) {
        let mut specs = self.specs.write().unwrap_or_else(|e| e.into_inner());
        let mut aliases = self.aliases.write().unwrap_or_else(|e| e.into_inner());
        for alias in &spec.aliases {
            aliases.insert(alias.to_ascii_lowercase(), spec.id.clone());
        }
        specs.insert(spec.id.clone(), spec);
    }

    /// Removes a model.
    pub fn remove(&self, id: &str) -> Option<TTSModelSpec> {
        let mut specs = self.specs.write().unwrap_or_else(|e| e.into_inner());
        let mut aliases = self.aliases.write().unwrap_or_else(|e| e.into_inner());
        let removed = specs.remove(id);
        if removed.is_some() {
            aliases.retain(|_, target| target != id);
        }
        removed
    }

    /// Looks a model up by id.
    pub fn get(&self, id: &str) -> Option<TTSModelSpec> {
        self.specs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Looks a model up by id or alias.
    pub fn resolve(&self, id_or_alias: &str) -> Option<TTSModelSpec> {
        if let Some(spec) = self.get(id_or_alias) {
            return Some(spec);
        }
        let aliases = self.aliases.read().unwrap_or_else(|e| e.into_inner());
        aliases
            .get(&id_or_alias.to_ascii_lowercase())
            .and_then(|id| self.get(id))
    }

    /// All registered models.
    pub fn list(&self) -> Vec<TTSModelSpec> {
        self.specs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Models of one family.
    pub fn list_by_family(&self, family: ModelFamily) -> Vec<TTSModelSpec> {
        self.list()
            .into_iter()
            .filter(|s| s.family == family)
            .collect()
    }

    /// Number of registered models.
    pub fn len(&self) -> usize {
        self.specs.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// `true` when no model is registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Checks whether the model can run on the given machine.
    pub fn check_compatibility(
        &self,
        spec: &TTSModelSpec,
        profile: &HardwareProfile,
    ) -> CompatibilityReport {
        let mut notes = Vec::new();
        let mut level = CompatibilityLevel::Supported;

        let req = &spec.hardware;
        if !req.platforms.is_empty() && !req.platforms.contains(&profile.platform) {
            notes.push(format!(
                "model is only supported on: {}",
                req.platforms.join(", ")
            ));
            level = CompatibilityLevel::Unsupported;
        }
        if req.require_cuda && !profile.cuda_available {
            notes.push("no CUDA capable NVIDIA GPU detected".to_string());
            level = CompatibilityLevel::Unsupported;
        }
        if req.require_apple_silicon && !profile.apple_silicon {
            notes.push("this checkpoint requires Apple Silicon".to_string());
            level = CompatibilityLevel::Unsupported;
        }
        if req.min_total_ram_bytes > 0 && profile.total_ram_bytes < req.min_total_ram_bytes {
            notes.push(format!(
                "needs at least {} GiB of memory, this machine has {:.1} GiB",
                req.min_total_ram_bytes / 1024 / 1024 / 1024,
                profile.total_ram_bytes as f64 / 1024.0 / 1024.0 / 1024.0
            ));
            level = CompatibilityLevel::Unsupported;
        }
        if req.min_cuda_compute.is_some() && !profile.cuda_available {
            notes.push(format!(
                "the quantised checkpoint needs CUDA compute capability {}",
                req.min_cuda_compute.unwrap_or_default()
            ));
            level = CompatibilityLevel::Unsupported;
        }
        if level == CompatibilityLevel::Supported
            && profile.platform == "macos"
            && req.require_cuda
            && !req.require_mlx
        {
            notes.push(
                "CUDA path is not available on macOS; the upstream project does not publish a \
                 verified MPS/MLX build for this checkpoint"
                    .to_string(),
            );
            level = CompatibilityLevel::Unsupported;
        }
        if let Some(note) = &req.notes {
            notes.push(note.clone());
        }
        if let Some(note) = &spec.notes {
            notes.push(note.clone());
        }

        CompatibilityReport {
            model_id: spec.id.clone(),
            level,
            compatible: level != CompatibilityLevel::Unsupported,
            notes,
        }
    }

    /// Loads additional models from a JSON catalog file.
    ///
    /// The file contains an array of [`TTSModelSpec`] objects and is merged
    /// into the registry.
    pub fn load_catalog(&self, path: &Path) -> TtsResult<usize> {
        let raw = std::fs::read_to_string(path).map_err(|e| TtsError::io(path, e))?;
        let specs: Vec<TTSModelSpec> =
            serde_json::from_str(&raw).map_err(TtsError::serialization)?;
        let count = specs.len();
        for spec in specs {
            self.insert(spec);
        }
        Ok(count)
    }

    /// Writes the registry to a JSON catalog file.
    pub fn save_catalog(&self, path: &Path) -> TtsResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| TtsError::io(parent, e))?;
        }
        let specs = self.list();
        let raw = serde_json::to_string_pretty(&specs).map_err(TtsError::serialization)?;
        std::fs::write(path, raw).map_err(|e| TtsError::io(path, e))?;
        Ok(())
    }
}

/// The display names of the catalogue families.
/// `true` when the directory holds the weights of a checkpoint.
///
/// The installer writes the files of a model one by one, so a directory with
/// nothing but the configuration files belongs to a download that is still
/// running.
fn has_weights(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if !path.is_file() {
            return false;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        name.ends_with(".safetensors")
            || name.ends_with(".safetensors.index.json")
            || name.ends_with(".gguf")
            || name.ends_with(".bin")
            || name.ends_with(".pt")
    })
}

pub fn builtin_models() -> Vec<TTSModelSpec> {
    const GB: u64 = 1024 * 1024 * 1024;

    vec![
        TTSModelSpec {
            id: "qwen3-tts-1.7b-voicedesign".to_string(),
            name: "Qwen3-TTS 1.7B VoiceDesign".to_string(),
            family: ModelFamily::Qwen3Tts,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign".to_string(),
            version: "1.7B".to_string(),
            format: "safetensors".to_string(),
            size_bytes: 4_520_163_832,
            estimated_memory_bytes: 8 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceDesign,
                TtsFeature::InstructionControl,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string(), "macos".to_string()],
                require_cuda: false,
                require_apple_silicon: false,
                require_mlx: false,
                min_total_ram_bytes: 16 * GB,
                min_cuda_compute: None,
                notes: Some(
                    "VoiceDesign requires `task_type=VoiceDesign` and a non-empty `instructions` \
                     field. The macOS path is not verified by the upstream project."
                        .to_string(),
                ),
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec!["qwen3-tts".to_string(), "qwen3-tts-voicedesign".to_string()],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "qwen3-tts-1.7b-customvoice".to_string(),
            name: "Qwen3-TTS 1.7B CustomVoice".to_string(),
            family: ModelFamily::Qwen3Tts,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice".to_string(),
            version: "1.7B".to_string(),
            format: "safetensors".to_string(),
            size_bytes: 4_520_218_951,
            estimated_memory_bytes: 8 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::CustomVoice,
                TtsFeature::InstructionControl,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string(), "macos".to_string()],
                min_total_ram_bytes: 16 * GB,
                notes: Some(
                    "Built-in speakers: Vivian, Ryan, Dylan, Eric, and others.".to_string(),
                ),
                ..Default::default()
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec!["qwen3-tts-customvoice".to_string()],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "qwen3-tts-1.7b-base".to_string(),
            name: "Qwen3-TTS 1.7B Base".to_string(),
            family: ModelFamily::Qwen3Tts,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-Base".to_string(),
            version: "1.7B".to_string(),
            format: "safetensors".to_string(),
            size_bytes: 4_544_229_700,
            estimated_memory_bytes: 8 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string(), "macos".to_string()],
                min_total_ram_bytes: 16 * GB,
                notes: Some("Voice cloning requires a reference clip.".to_string()),
                ..Default::default()
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "qwen3-tts-0.6b-base".to_string(),
            name: "Qwen3-TTS 0.6B Base".to_string(),
            family: ModelFamily::Qwen3Tts,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "Qwen/Qwen3-TTS-12Hz-0.6B-Base".to_string(),
            version: "0.6B".to_string(),
            format: "safetensors".to_string(),
            size_bytes: 2_516_106_051,
            estimated_memory_bytes: 4 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string(), "macos".to_string()],
                min_total_ram_bytes: 8 * GB,
                notes: Some("Voice cloning requires a reference clip.".to_string()),
                ..Default::default()
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "qwen3-tts-0.6b-customvoice-mlx".to_string(),
            name: "Qwen3-TTS 0.6B CustomVoice (MLX)".to_string(),
            family: ModelFamily::Qwen3Tts,
            engine_id: "sglang-omni-mlx".to_string(),
            hf_repo: "mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-bf16".to_string(),
            version: "0.6B".to_string(),
            format: "safetensors".to_string(),
            size_bytes: 2_498_419_405,
            estimated_memory_bytes: 8 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["macos".to_string()],
                min_total_ram_bytes: 16 * GB,
                notes: Some(
                    "Native MLX runtime on Apple Silicon. The Torch path of \
                     SGLang-Omni crashes in the MPS backend, this checkpoint \
                     is served by the native qwen3_tts_server."
                        .to_string(),
                ),
                ..Default::default()
            },
            mode: EngineMode::MlxNative,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "drama-3".to_string(),
            name: "Fish Audio Drama 3 (FP8)".to_string(),
            family: ModelFamily::FishDrama3,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "fishaudio/drama-3".to_string(),
            version: "3".to_string(),
            format: "safetensors (fp8 experts)".to_string(),
            size_bytes: 41_502_571_262,
            estimated_memory_bytes: 45 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::MultiSpeaker,
                TtsFeature::EmotionalControl,
                TtsFeature::PointTags,
                TtsFeature::AudioEditing,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string()],
                require_cuda: true,
                require_apple_silicon: false,
                require_mlx: false,
                min_total_ram_bytes: 64 * GB,
                min_cuda_compute: None,
                notes: Some(
                    "35B MoE with 3B active parameters; 37 GB of weights. Runs on NVIDIA CUDA \
                     only - do not assume macOS support."
                        .to_string(),
                ),
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec!["fish-drama-3".to_string(), "drama3".to_string()],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "drama-3-int4".to_string(),
            name: "Fish Audio Drama 3 (INT4)".to_string(),
            family: ModelFamily::FishDrama3,
            engine_id: "sglang-omni".to_string(),
            hf_repo: "fishaudio/drama-3-int4".to_string(),
            version: "3-int4".to_string(),
            format: "safetensors (gptq int4)".to_string(),
            size_bytes: 24_029_281_487,
            estimated_memory_bytes: 28 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::MultiSpeaker,
                TtsFeature::EmotionalControl,
                TtsFeature::PointTags,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["linux".to_string()],
                require_cuda: true,
                min_total_ram_bytes: 48 * GB,
                min_cuda_compute: Some(80),
                notes: Some(
                    "GPTQ-Marlin kernels need compute capability 8.0 or newer. Weights take \
                     20.8 GB instead of 37.0 GB."
                        .to_string(),
                ),
                ..Default::default()
            },
            mode: EngineMode::SglangOmni,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "drama-3-mlx-4bit".to_string(),
            name: "Fish Audio Drama 3 (MLX 4-bit)".to_string(),
            family: ModelFamily::FishDrama3,
            engine_id: "fishaudio-drama3-mlx".to_string(),
            hf_repo: "mlx-community/fish-audio-drama-3-4bit".to_string(),
            version: "3-mlx-4bit".to_string(),
            format: "safetensors (mlx)".to_string(),
            size_bytes: 23_781_119_297,
            estimated_memory_bytes: 29 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::MultiSpeaker,
                TtsFeature::EmotionalControl,
                TtsFeature::PointTags,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["macos".to_string()],
                require_apple_silicon: true,
                require_mlx: true,
                min_total_ram_bytes: 48 * GB,
                notes: Some(
                    "Native MLX runtime. Peaks above 28.7 GB, so a 32 GB or 36 GB Mac is not \
                     enough."
                        .to_string(),
                ),
                ..Default::default()
            },
            mode: EngineMode::MlxNative,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: Some(
                "Reference clips are sent as base64 WAV; up to 4 clips and 30 s in total."
                    .to_string(),
            ),
        },
        TTSModelSpec {
            id: "drama-3-mlx-8bit".to_string(),
            name: "Fish Audio Drama 3 (MLX 8-bit)".to_string(),
            family: ModelFamily::FishDrama3,
            engine_id: "fishaudio-drama3-mlx".to_string(),
            hf_repo: "mlx-community/fish-audio-drama-3-8bit".to_string(),
            version: "3-mlx-8bit".to_string(),
            format: "safetensors (mlx)".to_string(),
            size_bytes: 42_177_609_321,
            estimated_memory_bytes: 48 * GB,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceCloning,
                TtsFeature::MultiSpeaker,
                TtsFeature::EmotionalControl,
                TtsFeature::PointTags,
            ],
            hardware: HardwareRequirement {
                platforms: vec!["macos".to_string()],
                require_apple_silicon: true,
                require_mlx: true,
                min_total_ram_bytes: 64 * GB,
                notes: Some("Native MLX runtime, peaks around 47.1 GB.".to_string()),
                ..Default::default()
            },
            mode: EngineMode::MlxNative,
            default_ttl_secs: None,
            aliases: vec![],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
        TTSModelSpec {
            id: "mock-echo".to_string(),
            name: "Mock Echo (development)".to_string(),
            family: ModelFamily::Mock,
            engine_id: "mock".to_string(),
            hf_repo: "mock://echo".to_string(),
            version: "1".to_string(),
            format: "none".to_string(),
            size_bytes: 0,
            estimated_memory_bytes: 64 * 1024 * 1024,
            features: vec![
                TtsFeature::TextToSpeech,
                TtsFeature::VoiceDesign,
                TtsFeature::VoiceCloning,
                TtsFeature::CustomVoice,
                TtsFeature::MultiSpeaker,
                TtsFeature::PointTags,
                TtsFeature::Streaming,
            ],
            hardware: HardwareRequirement {
                notes: Some("Synthetic engine for tests and UI development.".to_string()),
                ..Default::default()
            },
            mode: EngineMode::Mock,
            default_ttl_secs: Some(300),
            aliases: vec!["mock".to_string()],
            launch_args: vec![],
            local_path: None,
            notes: None,
        },
    ]
}
