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
    /// The size of the files that would be downloaded, filled in when the
    /// repository was looked at a second time.
    #[serde(skip)]
    pub size_bytes: Option<u64>,
    /// The number of parameters of the model, filled in when the repository
    /// was looked at a second time.
    #[serde(skip)]
    pub parameters: Option<u64>,
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
const QUANT_PREFERENCE: [&str; 12] = [
    "Q4_K_M",
    "Q5_K_M",
    "Q4_K_S",
    "Q6_K",
    "Q8_0",
    "Q5_0",
    "Q4_0",
    "IQ4_XS",
    "IQ4_NL",
    "Q3_K_M",
    "Q3_K_L",
    "Q2_K",
];

/// `true` when the file holds the weights without a quantisation.
///
/// Those files are the biggest ones of a repository, so they are the last
/// choice when the download can pick a quantisation.
fn is_unquantised(path: &str) -> bool {
    let name = path.to_uppercase();
    ["BF16", "F16", "F32", "FP16", "FP32"]
        .iter()
        .any(|marker| name.contains(marker))
}

/// The name of the file inside the repository.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The directory of the file inside the repository.
fn directory_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(directory, _)| directory).unwrap_or("")
}

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
        let limit = limit.clamp(1, 50);
        // The hub answers with the most downloaded repositories first, and
        // those are often the ones the runtime cannot serve. The search
        // therefore asks for more than it shows.
        let fetch = (limit * 3).clamp(10, 150);
        if query.is_empty() {
            // The page opens with the models that are downloaded most often.
            // Half of the place belongs to the language models and half to
            // the speech models, so both are seen.
            let half = (limit / 2).max(1);
            let mut language = self.query(query, half, Some("gguf")).await?;
            let mut speech = self.query(query, limit * 2, Some("text-to-speech")).await?;
            language.retain(HubModel::is_runnable);
            speech.retain(HubModel::is_runnable);
            language.truncate(half);
            speech.truncate(limit - half);
            let mut models = language;
            models.extend(speech);
            let models = self.with_sizes(models).await;
            return Ok(HubSearch {
                models,
                dropped: 0,
            });
        }
        let mut all = self.query(query, fetch, Some("gguf")).await?;
        all.extend(self.query(query, fetch, Some("text-to-speech")).await?);
        all.extend(self.query(query, fetch, Some("text-generation")).await?);
        all.sort_by_key(|model| std::cmp::Reverse(model.downloads));
        all.dedup_by(|left, right| left.id == right.id);
        let dropped = all.len();
        let mut models: Vec<HubModel> = all.into_iter().filter(HubModel::is_runnable).collect();
        models.truncate(limit);
        let models = self.with_sizes(models).await;
        Ok(HubSearch {
            models,
            dropped,
        })
    }

    /// Asks the hub for the size of every model of a result.
    ///
    /// The hub does not answer the search with sizes, so every repository
    /// is looked at once more. The lookups run together, so the answer
    /// takes about as long as the slowest one.
    async fn with_sizes(&self, models: Vec<HubModel>) -> Vec<HubModel> {
        let mut lookups = tokio::task::JoinSet::new();
        for model in models {
            let client = self.clone();
            let want_parameters = model.format() != "gguf";
            lookups.spawn(async move {
                let detail = client.detail(&model.id, want_parameters).await.ok();
                (model, detail)
            });
        }
        let mut sized = Vec::new();
        while let Some(joined) = lookups.join_next().await {
            if let Ok((mut model, detail)) = joined {
                if let Some(detail) = detail {
                    model.size_bytes = model.size_of(&detail);
                    if model.format() == "gguf" {
                        model.parameters = model.parameters_from_name();
                    } else {
                        model.parameters = detail.safetensors_parameters;
                    }
                }
                sized.push(model);
            }
        }
        sized
    }

    /// Asks the hub for the repositories of a search.
    async fn query(
        &self,
        query: &str,
        limit: usize,
        filter: Option<&str>,
    ) -> TtsResult<Vec<HubModel>> {
        let mut url = format!(
            "{}/api/models?limit={}&full=true&sort=downloads",
            self.endpoint, limit,
        );
        if !query.is_empty() {
            url.push_str(&format!("&search={}", encode(query)));
        }
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

    /// Asks the hub for the size of the files of a repository and for the
    /// number of its parameters.
    ///
    /// The search of the hub does not answer with sizes, so every result
    /// needs a second look before it can tell how large the download is.
    pub async fn detail(&self, repo: &str, want_parameters: bool) -> TtsResult<HubDetail> {
        let files = self.files(repo).await?;
        let mut detail = HubDetail::default();
        for file in &files {
            let Some(size) = file.size else { continue };
            if file.path.ends_with(".gguf") {
                detail.gguf_files.push((file.path.clone(), size));
            } else if file.path.ends_with(".safetensors") {
                detail.safetensors_bytes += size;
            }
        }
        if want_parameters {
            detail.safetensors_parameters = self.parameters(repo).await?;
        }
        Ok(detail)
    }

    /// Asks the hub for the number of parameters of a repository.
    ///
    /// The hub answers with the parameters of the safetensors files, which
    /// is the exact number of weights of the model.
    async fn parameters(&self, repo: &str) -> TtsResult<Option<u64>> {
        let url = format!(
            "{}/api/models/{}?expand[]=safetensors",
            self.endpoint, repo
        );
        let response = match self.client.get(&url).send().await {
            Ok(response) => response,
            // The parameters are a nicety; the search must not fail without
            // them.
            Err(_) => return Ok(None),
        };
        if !response.status().is_success() {
            return Ok(None);
        }
        let body: serde_json::Value = match response.json().await {
            Ok(body) => body,
            Err(_) => return Ok(None),
        };
        Ok(body
            .get("safetensors")
            .and_then(|info| {
                // The hub writes the number of the parameters below the
                // weights of the file; older answers keep it inside the
                // breakdown of the formats.
                info.get("total")
                    .or_else(|| info.get("parameters").and_then(|inner| inner.get("total")))
            })
            .and_then(|total| total.as_u64()))
    }
}

/// The size of the files of a repository.
#[derive(Debug, Clone, Default)]
pub struct HubDetail {
    /// The GGUF files of the repository with their size.
    pub gguf_files: Vec<(String, u64)>,
    /// The sum of the safetensors files of the repository.
    pub safetensors_bytes: u64,
    /// The number of parameters of the model, when the hub knows it.
    pub safetensors_parameters: Option<u64>,
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
                Some(self.speech_family())
            }
            Some("text-generation") | Some("image-text-to-text") | Some("text-to-text") => {
                Some(ModelFamily::Llm)
            }
            _ => {
                if !self.gguf_files().is_empty() {
                    Some(ModelFamily::Llm)
                } else if self.is_qwen3_tts() {
                    Some(ModelFamily::Qwen3Tts)
                } else if self.is_drama3() {
                    Some(ModelFamily::FishDrama3)
                } else {
                    None
                }
            }
        }
    }

    /// The family of a speech repository.
    ///
    /// Two families of speech models are spoken by the application: the
    /// Qwen3-TTS ones and the Fish Audio Drama 3 ones. Both are served by
    /// the same runtime, but they carry different weights.
    fn speech_family(&self) -> ModelFamily {
        if self.is_drama3() {
            ModelFamily::FishDrama3
        } else {
            ModelFamily::Qwen3Tts
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

    /// `true` when the repository holds a Fish Audio Drama 3 checkpoint.
    ///
    /// The community marks these repositories with `fish_omni` or with a
    /// name that carries the model, and the official ones with `sglang`.
    pub fn is_drama3(&self) -> bool {
        let id = self.id.to_lowercase();
        self.tags.iter().any(|tag| {
            tag == "fish_omni"
                || tag == "fish-audio"
                || tag == "fishaudio"
                || tag == "drama3"
                || tag.starts_with("drama3-")
                || tag == "sglang-omni"
        }) || id.contains("drama-3")
            || id.contains("drama3")
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
            Some(ModelFamily::FishDrama3) => self.is_drama3() && self.format() != "gguf",
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
        // The unquantised files are the biggest ones of a repository, so
        // they are the last choice.
        let quantised: Vec<&String> = files
            .iter()
            .filter(|path| !is_unquantised(path))
            .collect();
        let pool: Vec<&String> = if quantised.is_empty() {
            files.iter().collect()
        } else {
            quantised
        };
        // A split checkpoint keeps its parts next to each other, in the
        // directory of its quantisation. The parts of another quantisation
        // must not be added to it.
        if let Some(first) = pool
            .iter()
            .find(|path| file_name(path).contains("-of-"))
        {
            let directory = directory_of(first);
            // The name of a part ends with the number of the part:
            // `Model-Q4_K_M-00001-of-00002.gguf`. The prefix of the parts
            // is the name without that number.
            let before_of = file_name(first)
                .split("-of-")
                .next()
                .unwrap_or_default();
            let prefix = before_of
                .rsplit_once('-')
                .filter(|(_, number)| {
                    !number.is_empty() && number.chars().all(|char| char.is_ascii_digit())
                })
                .map(|(head, _)| head.to_string())
                .unwrap_or_else(|| before_of.to_string());
            let mut parts: Vec<String> = pool
                .iter()
                .filter(|path| {
                    directory_of(path) == directory
                        && file_name(path).starts_with(&prefix)
                        && file_name(path).contains("-of-")
                })
                .map(|path| (*path).clone())
                .collect();
            parts.sort();
            if !parts.is_empty() {
                return parts;
            }
        }
        // A single file per quantisation: the preferred one wins.
        for wanted in QUANT_PREFERENCE {
            if let Some(file) = pool
                .iter()
                .find(|path| file_name(path).to_uppercase().contains(wanted))
            {
                return vec![(*file).clone()];
            }
        }
        vec![pool[0].clone()]
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
        // A quantisation the application does not know: the name of the file
        // ends with it.
        let stem = name.strip_suffix(".gguf").unwrap_or(name);
        let last = stem.rsplit('-').next()?;
        if last.chars().any(|char| char.is_ascii_digit()) {
            Some(last.to_string())
        } else {
            None
        }
    }

    /// The size of the download, in bytes.
    ///
    /// A GGUF repository is downloaded with one quantisation only, so the
    /// size is the one of the chosen file. Every other repository is
    /// downloaded completely.
    pub fn size_of(&self, detail: &HubDetail) -> Option<u64> {
        if self.format() == "gguf" {
            let wanted = self.chosen_files();
            let mut size = 0u64;
            for path in &wanted {
                let found = detail
                    .gguf_files
                    .iter()
                    .find(|(name, _)| name == path)
                    .map(|(_, size)| *size);
                size += found?;
            }
            if size == 0 {
                None
            } else {
                Some(size)
            }
        } else if detail.safetensors_bytes > 0 {
            Some(detail.safetensors_bytes)
        } else {
            None
        }
    }

    /// The number of parameters of the model, read from the name of the
    /// repository.
    ///
    /// The hub only knows the exact number of parameters for a repository
    /// with safetensors files. A GGUF repository carries it in its name,
    /// like `Qwen3.5-9B-GGUF`.
    pub fn parameters_from_name(&self) -> Option<u64> {
        let name = self.id.to_uppercase();
        let bytes = name.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'B' && index > 0 && bytes[index - 1].is_ascii_digit() {
                // Read the number that ends with the `B` backwards.
                let mut start = index - 1;
                while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
                    start -= 1;
                }
                let text = &name[start..index];
                if let Ok(value) = text.parse::<f64>() {
                    // A number in billions: `9B` is nine billion parameters.
                    // The first one is the size of the model; a name like
                    // `Qwen3-30B-A3B` describes the active parameters after
                    // it.
                    return Some((value * 1_000_000_000.0) as u64);
                }
            }
            index += 1;
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
            parameters: self.parameters,
            features: match family {
                ModelFamily::Llm => Vec::new(),
                ModelFamily::FishDrama3 => vec![
                    TtsFeature::TextToSpeech,
                    TtsFeature::VoiceCloning,
                ],
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
            size_bytes: None,
            parameters: None,
        };
        assert!(!model.is_runnable());
        assert_eq!(model.family().map(|family| family.as_str()), Some("qwen3-tts"));
    }

    /// The Drama 3 checkpoints are speech models of the second family.
    /// The runtime serves them like the Qwen3-TTS ones, so they must not
    /// be dropped from the search.
    #[test]
    fn a_drama_checkpoint_is_a_speech_model() {
        let model = HubModel {
            id: "fishaudio/drama-3-int4".to_string(),
            pipeline_tag: Some("text-to-speech".to_string()),
            library_name: None,
            downloads: 100,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec![
                "safetensors".to_string(),
                "fish_omni".to_string(),
                "text-to-speech".to_string(),
                "sglang-omni".to_string(),
            ],
            siblings: Vec::new(),
            size_bytes: None,
            parameters: None,
        };
        assert!(model.is_runnable());
        assert_eq!(model.family().map(|family| family.as_str()), Some("fish-drama-3"));
        assert_eq!(model.engine_id(), "sglang-omni");
        assert_eq!(model.format(), "safetensors");
    }

    /// The community conversion of Drama 3 for the Apple hardware is
    /// read by the native runtime, not by the one of the other platforms.
    #[test]
    fn a_drama_checkpoint_of_apple_hardware_is_native() {
        let model = HubModel {
            id: "mlx-community/fish-audio-drama-3-4bit".to_string(),
            pipeline_tag: Some("text-to-speech".to_string()),
            library_name: None,
            downloads: 100,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec!["mlx".to_string(), "text-to-speech".to_string()],
            siblings: Vec::new(),
            size_bytes: None,
            parameters: None,
        };
        assert!(model.is_runnable());
        assert_eq!(model.engine_id(), "sglang-omni-mlx");
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
                size_bytes: None,
                parameters: None,
            };
            assert!(model.is_runnable(), "{pipeline_tag} was dropped");
            assert_eq!(model.format(), "gguf");
            assert_eq!(model.engine_id(), "llama.cpp");
        }
    }

    /// A repository that keeps its quantisations in directories must not
    /// mix the parts of one quantisation with the parts of another, and
    /// the unquantised files come last.
    #[test]
    fn a_split_repository_picks_one_quantisation() {
        let model = HubModel {
            id: "unsloth/Qwen3.5-35B-A3B-GGUF".to_string(),
            pipeline_tag: Some("image-text-to-text".to_string()),
            library_name: None,
            downloads: 100,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec!["gguf".to_string()],
            siblings: vec![
                crate::hub::HubSibling {
                    rfilename: "BF16/Qwen3.5-35B-A3B-BF16-00001-of-00002.gguf".to_string(),
                },
                crate::hub::HubSibling {
                    rfilename: "BF16/Qwen3.5-35B-A3B-BF16-00002-of-00002.gguf".to_string(),
                },
                crate::hub::HubSibling {
                    rfilename: "Qwen3.5-35B-A3B-Q4_K_M.gguf".to_string(),
                },
            ],
            size_bytes: None,
            parameters: None,
        };
        assert_eq!(
            model.chosen_files(),
            vec!["Qwen3.5-35B-A3B-Q4_K_M.gguf".to_string()]
        );
        assert_eq!(model.quant().as_deref(), Some("Q4_K_M"));
    }

    /// A repository whose checkpoint is split keeps every part of it.
    #[test]
    fn a_split_checkpoint_keeps_its_parts() {
        let model = HubModel {
            id: "someone/Model-GGUF".to_string(),
            pipeline_tag: None,
            library_name: None,
            downloads: 100,
            likes: 1,
            last_modified: None,
            created_at: None,
            tags: vec!["gguf".to_string()],
            siblings: vec![
                crate::hub::HubSibling {
                    rfilename: "Q4_K_M/Model-Q4_K_M-00001-of-00002.gguf".to_string(),
                },
                crate::hub::HubSibling {
                    rfilename: "Q4_K_M/Model-Q4_K_M-00002-of-00002.gguf".to_string(),
                },
                crate::hub::HubSibling {
                    rfilename: "Q8_0/Model-Q8_0-00001-of-00002.gguf".to_string(),
                },
            ],
            size_bytes: None,
            parameters: None,
        };
        assert_eq!(model.chosen_files().len(), 2);
        assert!(model
            .chosen_files()
            .iter()
            .all(|path| path.starts_with("Q4_K_M/")));
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
            size_bytes: None,
            parameters: None,
        };
        assert!(model.is_runnable());
        assert_eq!(model.family().map(|family| family.as_str()), Some("llm"));
    }
}
