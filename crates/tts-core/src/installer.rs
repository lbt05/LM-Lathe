//! Download of the model checkpoints from the Hugging Face Hub.
//!
//! The registry only describes the models; this module puts the actual
//! weights on the disk. The layout follows the specification: every
//! checkpoint lives in `models/tts/<organisation>--<repository>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::{TtsError, TtsResult};
use crate::registry::TTSModelSpec;

/// Default Hugging Face endpoint.
pub const HUB_ENDPOINT: &str = "https://huggingface.co";

/// A file inside a repository.
#[derive(Debug, Clone, Deserialize)]
pub struct RepoFile {
    /// `file` or `directory`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Path of the file inside the repository.
    pub path: String,
    /// Size of the file in bytes.
    #[serde(default)]
    pub size: Option<u64>,
}

/// The result of a finished installation.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InstallReport {
    /// The model that was installed.
    pub model_id: String,
    /// Directory the checkpoint lives in.
    pub path: PathBuf,
    /// Files that were downloaded.
    pub downloaded: Vec<String>,
    /// Files that were already present.
    pub skipped: Vec<String>,
    /// Number of bytes written.
    pub bytes: u64,
}

/// Progress of a running installation.
#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    /// The model that is being installed.
    pub model_id: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    /// Files that are already on the disk.
    pub files_done: usize,
    /// Files of the repository.
    pub files_total: usize,
    /// Bytes written so far.
    pub bytes: u64,
    /// Error message when the installation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Keeps track of the running installations.
#[derive(Clone, Default)]
pub struct InstallTracker {
    entries: Arc<Mutex<HashMap<String, InstallProgress>>>,
}

impl InstallTracker {
    /// Registers a new installation.
    pub fn start(&self, model_id: &str) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.insert(
            model_id.to_string(),
            InstallProgress {
                model_id: model_id.to_string(),
                state: "running".to_string(),
                files_done: 0,
                files_total: 0,
                bytes: 0,
                error: None,
            },
        );
    }

    /// Updates the progress of an installation.
    pub fn update(&self, model_id: &str, update: impl FnOnce(&mut InstallProgress)) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(progress) = entries.get_mut(model_id) {
            update(progress);
        }
    }

    /// Marks an installation as finished.
    pub fn finish(&self, model_id: &str, error: Option<String>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(progress) = entries.get_mut(model_id) {
            progress.state = if error.is_some() { "failed" } else { "done" }.to_string();
            progress.error = error;
        }
    }

    /// The progress of one installation.
    pub fn get(&self, model_id: &str) -> Option<InstallProgress> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.get(model_id).cloned()
    }

    /// The progress of every installation.
    pub fn list(&self) -> Vec<InstallProgress> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<InstallProgress> = entries.values().cloned().collect();
        list.sort_by(|a, b| a.model_id.cmp(&b.model_id));
        list
    }
}

/// Downloads the model checkpoints from the Hugging Face Hub.
pub struct ModelInstaller {
    client: reqwest::Client,
    root: PathBuf,
    endpoint: String,
    token: Option<String>,
}

impl ModelInstaller {
    /// Creates an installer that stores the checkpoints under `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("local-ai-manager/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            client,
            root: root.into(),
            endpoint: HUB_ENDPOINT.to_string(),
            token: std::env::var("HF_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty()),
        }
    }

    /// Overrides the hub endpoint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into().trim_end_matches('/').to_string();
        self
    }

    /// The directory a checkpoint is stored in.
    pub fn target_dir(&self, spec: &TTSModelSpec) -> PathBuf {
        spec.local_path
            .clone()
            .unwrap_or_else(|| self.root.join(spec.slug()))
    }

    /// `true` when the checkpoint is present on the disk.
    pub fn is_installed(&self, spec: &TTSModelSpec) -> bool {
        self.target_dir(spec).is_dir()
    }

    /// The number of bytes the checkpoint occupies.
    pub fn size_on_disk(&self, spec: &TTSModelSpec) -> u64 {
        directory_size(&self.target_dir(spec))
    }

    /// Lists the files of the repository.
    pub async fn list_files(&self, spec: &TTSModelSpec) -> TtsResult<Vec<RepoFile>> {
        let url = format!(
            "{}/api/models/{}/tree/main?recursive=true",
            self.endpoint, spec.hf_repo
        );
        let response = self
            .authorized(self.client.get(url))
            .send()
            .await
            .map_err(|err| TtsError::Engine(format!("the model hub is unreachable: {err}")))?;

        if !response.status().is_success() {
            return Err(TtsError::Engine(format!(
                "the model hub answered {} for repository `{}`",
                response.status(),
                spec.hf_repo
            )));
        }

        let files: Vec<RepoFile> = response
            .json()
            .await
            .map_err(|err| TtsError::Serialization(err.to_string()))?;

        Ok(files
            .into_iter()
            .filter(|file| file.kind == "file")
            .filter(|file| !is_optional_asset(&file.path))
            .collect())
    }

    /// Downloads the checkpoint into the local model directory.
    ///
    /// Files that are already present are skipped, so an interrupted
    /// installation can be resumed.
    pub async fn install(
        &self,
        spec: &TTSModelSpec,
        tracker: Option<&InstallTracker>,
    ) -> TtsResult<InstallReport> {
        let files = self.list_files(spec).await?;
        let root = self.target_dir(spec);
        let mut report = InstallReport {
            model_id: spec.id.clone(),
            path: root.clone(),
            ..Default::default()
        };

        if let Some(tracker) = tracker {
            tracker.update(&spec.id, |progress| progress.files_total = files.len());
        }

        for file in files {
            let destination = root.join(&file.path);
            if let Some(size) = file.size {
                let complete = std::fs::metadata(&destination)
                    .map(|meta| meta.len() == size)
                    .unwrap_or(false);
                if complete {
                    report.skipped.push(file.path.clone());
                    if let Some(tracker) = tracker {
                        tracker.update(&spec.id, |progress| progress.files_done += 1);
                    }
                    continue;
                }
            }

            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|source| TtsError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }

            let url = format!(
                "{}/{}/resolve/main/{}",
                self.endpoint,
                spec.hf_repo,
                file.path.replace(' ', "%20")
            );
            let response = self
                .authorized(self.client.get(url))
                .send()
                .await
                .map_err(|err| {
                    TtsError::Engine(format!("failed to download `{}`: {err}", file.path))
                })?;

            if !response.status().is_success() {
                return Err(TtsError::Engine(format!(
                    "failed to download `{}`: the hub answered {}",
                    file.path,
                    response.status()
                )));
            }

            let bytes = response.bytes().await.map_err(|err| {
                TtsError::Engine(format!("failed to download `{}`: {err}", file.path))
            })?;

            std::fs::write(&destination, &bytes).map_err(|source| TtsError::Io {
                path: destination.clone(),
                source,
            })?;

            report.bytes += bytes.len() as u64;
            report.downloaded.push(file.path.clone());
            tracing::info!(
                model = %spec.id,
                file = %file.path,
                bytes = bytes.len(),
                "downloaded"
            );

            if let Some(tracker) = tracker {
                tracker.update(&spec.id, |progress| {
                    progress.files_done += 1;
                    progress.bytes += bytes.len() as u64;
                });
            }
        }

        Ok(report)
    }

    /// Removes the checkpoint from the disk.
    pub fn uninstall(&self, spec: &TTSModelSpec) -> TtsResult<()> {
        let directory = self.target_dir(spec);
        if directory.exists() {
            std::fs::remove_dir_all(&directory).map_err(|source| TtsError::Io {
                path: directory.clone(),
                source,
            })?;
        }
        Ok(())
    }

    /// Adds the authorization header when a token is available.
    fn authorized(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

/// Files that are only demonstration material and are not needed to run the
/// model.
const OPTIONAL_ASSETS: [&str; 6] = ["mp4", "png", "jpg", "jpeg", "gif", "webm"];

/// `true` when the file is a demo asset that can be skipped.
fn is_optional_asset(path: &str) -> bool {
    let extension = path
        .rsplit('.')
        .next()
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    OPTIONAL_ASSETS.contains(&extension.as_str())
}

/// The size of a directory in bytes.
fn directory_size(path: &Path) -> u64 {
    let mut total = 0;
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            total += directory_size(&entry.path());
        } else {
            total += metadata.len();
        }
    }
    total
}
