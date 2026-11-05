//! Search of the Hugging Face Hub.
//!
//! The registry only knows the models that ship with the application; this
//! module looks the rest of the hub up. A search answers with the metadata
//! of the repositories, and the files of a repository decide which
//! checkpoint is actually downloaded: a GGUF repository keeps one file per
//! quantisation, and downloading all of them would waste tens of gigabytes.

use serde::Deserialize;

use crate::error::{TtsError, TtsResult};
use crate::installer::{RepoFile, HUB_ENDPOINT};
use crate::registry::TTSModelSpec;
use crate::types::{EngineMode, LlmFeature, ModelFamily, TtsFeature};

/// The answer of a search of the hub.
#[derive(Debug, Clone, Default)]
pub struct HubSearch {
    /// The models the runtime of the application can serve.
    pub models: Vec<HubModel>,
    /// The repositories that were found but dropped, because the runtime
    /// cannot serve them.
    pub dropped: usize,
}

/// A repository of the hub.
#[derive(Debug, Clone, Deserialize)]
pub struct HubModel {
    /// The `organisation/repository` id.
    pub id: String,
    /// The task the model was trained for, e.g. `text-to-speech`.
    #[serde(default)]
    pub pipeline_tag: Option<String>,
    /// The library the model is written for, e.g. `transformers`.
    #[serde(default)]
    pub library_name: Option<String>,
    /// How often the model was downloaded in the last month.
    #[serde(default)]
    pub downloads: u64,
    /// How many users marked the model.
    #[serde(default)]
    pub likes: u64,
    /// When the repository was touched the last time.
    #[serde(rename = "lastModified", default)]
    pub last_modified: Option<String>,
    /// When the repository was created.
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    /// The tags of the repository.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The files of the repository, when the hub was asked for them.
    #[serde(default)]
    pub siblings: Vec<HubSibling>,
}

/// One file of a repository.
#[derive(Debug, Clone, Deserialize)]
pub struct HubSibling {
    /// The path of the file inside the repository.
    pub rfilename: String,
}

/// The client of the hub.
#[derive(Debug, Clone)]
pub struct HubClient {
    client: reqwest::Client,
    endpoint: String,
}

/// The quantisations that are picked in this order.
///
/// The list starts at the size that runs everywhere and ends at the most
/// expensive one, so a repository that offers several of them gets the
/// middle of the road by default.
const QUANT_PREFERENCE: [&str; 10] = [
    "Q4_K_M",
    "Q5_K_M",
    "Q4_K_S",
    "Q6_K",
    "Q8_0",
    "Q5_0",
    "Q4_0",
    "Q3_K_M",
    "Q3_K_L",
    "Q2_K",
];

impl HubClient {
    /// Creates a client that talks to the public hub.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("lmlathe/0.1")
                .build()
                .unwrap_or_default(),
            endpoint: HUB_ENDPOINT.to_string(),
        }
    }

    /// Overrides the hub endpoint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into().trim_end_matches('/').to_string();
        self
    }

    /// Searches the hub.
    ///
    /// The hub is asked twice: once for the quantised language models and
    /// once for the speech models. Only the repositories that the runtime
    /// can serve are kept, because downloading the rest would put a file on
    /// the disk that nothing can run.
    /// Searches the hub for the models of a query.
    ///
    /// The answer holds the models the runtime can serve and the number of
    /// repositories that were dropped, so the interface can tell why a
    /// query came back empty.
    pub async fn search(&self, query: &str, limit: usize) -> TtsResult<HubSearch> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(HubSearch::default());
        }
        let limit = limit.clamp(1, 50);
        // The hub answers with the most downloaded repositories first, and
        // those are often the ones the runtime cannot serve. The search
        // therefore asks for more than it shows.
        let fetch = (limit * 3).clamp(10, 150);
        let mut all = self.query(query, fetch, Some("gguf")).await?;
        all.extend(self.query(query, fetch, Some("text-to-speech")).await?);
        all.extend(self.query(query, fetch, Some("text-generation")).await?);
        all.sort_by_key(|model| std::cmp::Reverse(model.downloads));
        all.dedup_by(|left, right| left.id == right.id);
        let dropped = all.len();
        let mut models: Vec<HubModel> = all.into_iter().filter(HubModel::is_runnable).collect();
        models.truncate(limit);
        Ok(HubSearch {
            models,
            dropped,
        })
    }

    /// Asks the hub for the repositories of a search.
    async fn query(
        &self,
        query: &str,
        limit: usize,
        filter: Option<&str>,
    ) -> TtsResult<Vec<HubModel>> {
        let mut url = format!(
            "{}/api/models?search={}&limit={}&full=true&sort=downloads",
            self.endpoint,
            encode(query),
            limit,
        );
        if let Some(filter) = filter {
            url.push_str(&format!("&filter={}", encode(filter)));
        }
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Engine(format!("the model hub is unreachable: {err}")))?;
        if !response.status().is_success() {
            return Err(TtsError::Engine(format!(
                "the model hub answered {} for the search `{query}`",
                response.status()
            )));
        }
        let models: Vec<HubModel> = response
            .json()
            .await
            .map_err(|err| TtsError::Serialization(err.to_string()))?;
        Ok(models)
    }

    /// Looks a single repository up.
    pub async fn model(&self, repo: &str) -> TtsResult<HubModel> {
        let url = format!("{}/api/models/{}", self.endpoint, repo);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Engine(format!("the model hub is unreachable: {err}")))?;
        if !response.status().is_success() {
            return Err(TtsError::Engine(format!(
                "the model hub answered {} for the repository `{repo}`",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|err| TtsError::Serialization(err.to_string()))
    }

    /// Lists the files of a repository with their sizes.
    pub async fn files(&self, repo: &str) -> TtsResult<Vec<RepoFile>> {
        let url = format!(
            "{}/api/models/{}/tree/main?recursive=true",
            self.endpoint, repo
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|err| TtsError::Engine(format!("the model hub is unreachable: {err}")))?;
        if !response.status().is_success() {
            return Err(TtsError::Engine(format!(
                "the model hub answered {} for the repository `{repo}`",
                response.status()
            )));
        }
        let files: Vec<RepoFile> = response
            .json()
            .await
            .map_err(|err| TtsError::Serialization(err.to_string()))?;
        Ok(files
            .into_iter()
            .filter(|file| file.kind == "file")
            .collect())
    }
}

impl HubModel {
    /// The family of the model, derived from the task it was trained for.
    ///
    /// The hub is not consistent: a multimodal model is marked as
    /// `image-text-to-text` and a community model often carries no task at
    /// all. A repository that holds a GGUF file is a language model then,
    /// because the language runtime reads nothing else.
    pub fn family(&self) -> Option<ModelFamily> {
        match self.pipeline_tag.as_deref() {
            Some("text-to-speech") | Some("text-to-speech-v2") => {
                Some(ModelFamily::Qwen3Tts)
            }
            Some("text-generation") | Some("image-text-to-text") | Some("text-to-text") => {
                Some(ModelFamily::Llm)
            }
            _ => {
                if !self.gguf_files().is_empty() {
                    Some(ModelFamily::Llm)
                } else if self.is_qwen3_tts() {
                    Some(ModelFamily::Qwen3Tts)
                } else {
                    None
                }
            }
        }
    }

    /// The checkpoint format of the repository.
    ///
    /// The files of the repository are the truth; the tags are only a hint,
    /// because a repository can carry a GGUF file without the tag.
    pub fn format(&self) -> &'static str {
        if !self.gguf_files().is_empty() {
            "gguf"
        } else if self.tags.iter().any(|tag| tag.contains("mlx")) {
            "mlx"
        } else {
            "safetensors"
        }
    }

    /// The licence of the repository.
    pub fn license(&self) -> Option<String> {
        self.tags
            .iter()
            .find_map(|tag| tag.strip_prefix("license:"))
            .map(|value| value.to_string())
    }

    /// The GGUF files of the repository.
    pub fn gguf_files(&self) -> Vec<String> {
        self.siblings
            .iter()
            .map(|sibling| sibling.rfilename.clone())
            .filter(|path| path.ends_with(".gguf"))
            .collect()
    }

    /// `true` when the repository holds a Qwen3-TTS checkpoint.
    pub fn is_qwen3_tts(&self) -> bool {
        self.tags.iter().any(|tag| tag == "qwen3_tts")
            || self.id.to_lowercase().contains("qwen3-tts")
    }

    /// `true` when the runtime of the application can serve the model.
    ///
    /// The language runtime serves every GGUF checkpoint. The speech
    /// runtime speaks the Qwen3-TTS protocol, so a speech checkpoint is
    /// only useful when it is a Qwen3-TTS one that is stored in the format
    /// the runtime reads.
    pub fn is_runnable(&self) -> bool {
        match self.family() {
            Some(ModelFamily::Llm) => !self.gguf_files().is_empty(),
            Some(ModelFamily::Qwen3Tts) => self.is_qwen3_tts() && self.format() != "gguf",
            _ => false,
        }
    }

    /// The engine that serves the model.
    pub fn engine_id(&self) -> &'static str {
        match self.family() {
            Some(ModelFamily::Llm) => "llama.cpp",
            _ => {
                if self.format() == "mlx" {
                    "sglang-omni-mlx"
                } else {
                    "sglang-omni"
                }
            }
        }
    }

    /// The files that have to be downloaded.
    ///
    /// A GGUF repository keeps one file per quantisation; only the chosen
    /// one is downloaded, together with the parts of a split checkpoint.
    pub fn chosen_files(&self) -> Vec<String> {
        if self.format() != "gguf" {
            return Vec::new();
        }
        let files = self.gguf_files();
        if files.is_empty() {
            return Vec::new();
        }
        // A split checkpoint keeps its parts next to each other and an
        // index file describes them; the parts belong together.
        if let Some(first) = files.iter().find(|path| {
            path.rsplit('/')
                .next()
                .map(|name| name.contains("-of-"))
                .unwrap_or(false)
        }) {
            let prefix = first
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .split("-of-")
                .next()
                .unwrap_or_default()
                .to_string();
            let mut parts: Vec<String> = files
                .iter()
                .filter(|path| {
                    path.rsplit('/')
                        .next()
                        .map(|name| name.starts_with(&prefix) && name.contains("-of-"))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            parts.sort();
            if !parts.is_empty() {
                return parts;
            }
        }
        // A single file per quantisation: the preferred one wins.
        for wanted in QUANT_PREFERENCE {
            if let Some(file) = files
                .iter()
                .find(|path| path.to_uppercase().contains(wanted))
            {
                return vec![file.clone()];
            }
        }
        vec![files[0].clone()]
    }

    /// The quantisation of the chosen checkpoint.
    pub fn quant(&self) -> Option<String> {
        let chosen = self.chosen_files();
        let name = chosen.first()?.rsplit('/').next()?;
        for wanted in QUANT_PREFERENCE {
            if name.to_uppercase().contains(wanted) {
                return Some(wanted.to_string());
            }
        }
        None
    }

    /// Builds the specification of the model.
    ///
    /// The size of the checkpoint is taken from the files of the
    /// repository, so the interface can show how much disk the download
    /// needs.
    pub fn to_spec(&self, files: &[RepoFile]) -> TtsResult<TTSModelSpec> {
        let family = self.family().ok_or_else(|| {
            TtsError::InvalidRequest(format!("`{}` 不是本应用能运行的模型", self.id))
        })?;
        let wanted = self.chosen_files();
        let mut size_bytes = 0u64;
        for file in files {
            if wanted.is_empty() || wanted.iter().any(|path| path == &file.path) {
                size_bytes += file.size.unwrap_or(0);
            }
        }
        let notes = self
            .license()
            .map(|license| format!("许可：{license}"))
            .unwrap_or_else(|| "许可：未标注".to_string());
        Ok(TTSModelSpec {
            // The id is used in the paths of the API, so it may not hold a
            // slash; the slug of the repository is the same name the
            // checkpoint gets on the disk.
            id: self.id.replace('/', "--"),
            name: self.id.clone(),
            family,
            engine_id: self.engine_id().to_string(),
            hf_repo: self.id.clone(),
            version: "hf".to_string(),
            format: self.format().to_string(),
            size_bytes,
            estimated_memory_bytes: size_bytes + size_bytes / 4,
            features: match family {
                ModelFamily::Llm => Vec::new(),
                _ => vec![TtsFeature::TextToSpeech],
            },
            llm_features: match family {
                ModelFamily::Llm => vec![LlmFeature::Chat],
                _ => Vec::new(),
            },
            files: wanted,
            hardware: Default::default(),
            mode: match family {
                ModelFamily::Llm => EngineMode::LlamaCpp,
                _ => {
                    if self.format() == "mlx" {
                        EngineMode::MlxNative
                    } else {
                        EngineMode::SglangOmni
                    }
                }
            },
            default_ttl_secs: None,
            aliases: Vec::new(),
            launch_args: match family {
                ModelFamily::Llm => vec!["--ctx-size".to_string(), "8192".to_string()],
                _ => Vec::new(),
            },
            local_path: None,
            notes: Some(notes),
        })
    }
}

/// Percent encodes a query for the hub.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::HubModel;

    const PAYLOAD: &str = r#"[
        {
            "id": "Qwen/Qwen3-TTS-12Hz-1.7B-Base",
            "pipeline_tag": "text-to-speech",
            "downloads": 3455553,
            "likes": 588,
            "lastModified": "2026-01-23T05:25:33.000Z",
            "createdAt": "2026-01-21T08:57:11.000Z",
            "tags": ["safetensors", "qwen3_tts", "license:apache-2.0"],
            "siblings": [
                { "rfilename": "config.json" },
                { "rfilename": "model.safetensors" }
            ]
        },
        {
            "id": "unsloth/Qwen3-8B-GGUF",
            "pipeline_tag": "text-generation",
            "downloads": 1200,
            "likes": 42,
            "tags": ["gguf", "license:apache-2.0"],
            "siblings": [
                { "rfilename": "Qwen3-8B-Q4_K_M.gguf" },
                { "rfilename": "Qwen3-8B-Q8_0.gguf" }
            ]
        }
    ]"#;

    fn parse() -> Vec<HubModel> {
        serde_json::from_str(PAYLOAD).unwrap()
    }

    #[test]
    fn the_hub_payload_is_understood() {
        let models = parse();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].downloads, 3455553);
        assert_eq!(models[0].last_modified.as_deref(), Some("2026-01-23T05:25:33.000Z"));
        assert_eq!(models[0].license().as_deref(), Some("apache-2.0"));
    }

    #[test]
    fn a_speech_model_of_the_hub_is_runnable() {
        let model = &parse()[0];
        assert!(model.is_runnable());
        assert_eq!(model.format(), "safetensors");
        assert_eq!(model.engine_id(), "sglang-omni");
    }

    #[test]
    fn a_gguf_repository_picks_the_preferred_quantisation() {
        let model = &parse()[1];
        assert!(model.is_runnable());
        assert_eq!(model.format(), "gguf");
        assert_eq!(model.chosen_files(), vec!["Qwen3-8B-Q4_K_M.gguf".to_string()]);
        assert_eq!(model.quant().as_deref(), Some("Q4_K_M"));
    }

    #[test]
    fn a_speech_model_that_cannot_be_served_is_rejected() {
        let model = HubModel {
            id: "hexgrad/Kokoro-82M".to_string(),
            pipeline_tag: Some("text-to-speech".to_string()),
            library_name: None,
            downloads: 10,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec!["safetensors".to_string()],
            siblings: Vec::new(),
        };
        assert!(!model.is_runnable());
        assert_eq!(model.family().map(|family| family.as_str()), Some("qwen3-tts"));
    }

    /// A multimodal model is marked as `image-text-to-text` by the hub, and
    /// a community model often carries no task at all. Both are language
    /// models when they hold a GGUF file.
    #[test]
    fn a_multimodal_gguf_repository_is_a_language_model() {
        for pipeline_tag in ["image-text-to-text", "text-generation"] {
            let model = HubModel {
                id: "unsloth/Qwen3.5-9B-GGUF".to_string(),
                pipeline_tag: Some(pipeline_tag.to_string()),
                library_name: None,
                downloads: 100,
                likes: 1,
                last_modified: None,
                created_at: None,
                tags: vec!["gguf".to_string()],
                siblings: vec![crate::hub::HubSibling {
                    rfilename: "Qwen3.5-9B-Q4_K_M.gguf".to_string(),
                }],
            };
            assert!(model.is_runnable(), "{pipeline_tag} was dropped");
            assert_eq!(model.format(), "gguf");
            assert_eq!(model.engine_id(), "llama.cpp");
        }
    }

    /// A community model without a task is a language model when it holds a
    /// GGUF file, and it is dropped when it does not.
    #[test]
    fn a_repository_without_a_task_is_read_from_its_files() {
        let model = HubModel {
            id: "HauhauCS/Qwen3.5-9B-Uncensored".to_string(),
            pipeline_tag: None,
            library_name: None,
            downloads: 100,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec!["gguf".to_string()],
            siblings: vec![crate::hub::HubSibling {
                rfilename: "Qwen3.5-9B-Q4_K_M.gguf".to_string(),
            }],
        };
        assert!(model.is_runnable());
        assert_eq!(model.family().map(|family| family.as_str()), Some("llm"));
    }
}
