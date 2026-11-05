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
use crate::types::ModelFamily;

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
    /// Size of the whole repository in bytes.
    pub bytes_total: u64,
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
                bytes_total: 0,
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
    llm_root: PathBuf,
    endpoint: String,
    token: Option<String>,
    /// How many files of a repository are fetched at the same time.
    concurrency: std::sync::atomic::AtomicUsize,
}

impl ModelInstaller {
    /// Creates an installer that stores the checkpoints under `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("lm-lathe")
            .build()
            .unwrap_or_default();
        let root = root.into();
        Self {
            client,
            llm_root: root.clone(),
            root,
            endpoint: HUB_ENDPOINT.to_string(),
            token: std::env::var("HF_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty()),
            // One file at a time; the user can raise the number.
            concurrency: std::sync::atomic::AtomicUsize::new(1),
        }
    }

    /// How many files of a repository are fetched at the same time.
    pub fn concurrency(&self) -> usize {
        self.concurrency
            .load(std::sync::atomic::Ordering::Relaxed)
            .max(1)
    }

    /// Sets how many files of a repository are fetched at the same time.
    ///
    /// The hub serves a few files at the same time without a problem, but
    /// too many of them slow the download down instead of making it
    /// faster.
    pub fn set_concurrency(&self, value: usize) -> usize {
        let value = value.clamp(1, 8);
        self.concurrency
            .store(value, std::sync::atomic::Ordering::Relaxed);
        value
    }

    /// Sets the directory the language model checkpoints live in.
    pub fn with_llm_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.llm_root = root.into();
        self
    }

    /// Overrides the hub endpoint.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into().trim_end_matches('/').to_string();
        self
    }

    /// The directory a checkpoint is stored in.
    pub fn target_dir(&self, spec: &TTSModelSpec) -> PathBuf {
        spec.local_path.clone().unwrap_or_else(|| {
            if spec.family == ModelFamily::Llm {
                self.llm_root.join(spec.slug())
            } else {
                self.root.join(spec.slug())
            }
        })
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

        // A repository that keeps one file per quantisation would download
        // every variant, so a model names the files it needs.
        let wanted: Vec<String> = spec.files.clone();
        Ok(files
            .into_iter()
            .filter(|file| file.kind == "file")
            .filter(|file| !is_optional_asset(&file.path))
            .filter(|file| wanted.is_empty() || wanted.iter().any(|path| path == &file.path))
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

        // The size of the repository, the progress is measured in bytes and
        // not in files: a checkpoint holds a few small files and one huge
        // weight file, so counting the files would be meaningless.
        let bytes_total: u64 = files.iter().filter_map(|file| file.size).sum();

        if let Some(tracker) = tracker {
            tracker.update(&spec.id, |progress| {
                progress.files_total = files.len();
                progress.bytes_total = bytes_total;
            });
        }

        // The bytes that are already on the disk. The counter is shared by
        // the files that are fetched at the same time, so the progress is
        // the sum of all of them.
        let counter = std::sync::atomic::AtomicU64::new(0);

        // The files that are already on the disk are counted first.
        let mut pending: Vec<(RepoFile, PathBuf)> = Vec::new();
        for file in files {
            let destination = root.join(&file.path);
            if let Some(size) = file.size {
                let complete = std::fs::metadata(&destination)
                    .map(|meta| meta.len() == size)
                    .unwrap_or(false);
                if complete {
                    report.skipped.push(file.path.clone());
                    counter.fetch_add(size, std::sync::atomic::Ordering::Relaxed);
                    if let Some(tracker) = tracker {
                        tracker.update(&spec.id, |progress| {
                            progress.files_done += 1;
                            progress.bytes = counter.load(std::sync::atomic::Ordering::Relaxed);
                        });
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

            pending.push((file, destination));
        }

        // The files are fetched in groups, so that the number of the open
        // connections stays where the user put it.
        let concurrency = self.concurrency();
        for group in pending.chunks(concurrency) {
            let downloads = group.iter().map(|(file, destination)| {
                self.download_file(spec, file, destination, tracker, &counter)
            });
            let results = futures::future::join_all(downloads).await;
            for ((file, _), fetched) in group.iter().zip(results) {
                let fetched = fetched?;
                report.bytes += fetched;
                report.downloaded.push(file.path.clone());
                tracing::info!(
                    model = %spec.id,
                    file = %file.path,
                    bytes = fetched,
                    "downloaded"
                );
                if let Some(tracker) = tracker {
                    tracker.update(&spec.id, |progress| {
                        progress.files_done += 1;
                        progress.bytes = counter.load(std::sync::atomic::Ordering::Relaxed);
                    });
                }
            }
        }

        Ok(report)
    }

    /// Downloads one file of the repository into `destination`.
    ///
    /// The bytes are streamed into a file next to the destination, so that a
    /// download of several gigabytes does not live in the memory. A file that
    /// was interrupted is continued where it stopped.
    async fn download_file(
        &self,
        spec: &TTSModelSpec,
        file: &RepoFile,
        destination: &Path,
        tracker: Option<&InstallTracker>,
        counter: &std::sync::atomic::AtomicU64,
    ) -> TtsResult<u64> {
        let part = part_path(destination);

        // The bytes of an interrupted download.
        let mut resume_from: u64 = 0;
        if let Ok(meta) = std::fs::metadata(&part) {
            if meta.is_file() {
                resume_from = meta.len();
            }
        }

        let url = format!(
            "{}/{}/resolve/main/{}",
            self.endpoint,
            spec.hf_repo,
            file.path.replace(' ', "%20")
        );
        let mut request = self.client.get(url);
        if resume_from > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
        }

        let mut response = self.authorized(request).send().await.map_err(|err| {
            TtsError::Engine(format!("failed to download `{}`: {err}", file.path))
        })?;

        if !response.status().is_success() {
            return Err(TtsError::Engine(format!(
                "failed to download `{}`: the hub answered {}",
                file.path,
                response.status()
            )));
        }

        // The hub continues the download when it answers with `206`, otherwise
        // the file is written from the beginning.
        let continues = response.status() == reqwest::StatusCode::PARTIAL_CONTENT
            && resume_from > 0;
        if !continues {
            resume_from = 0;
        }

        let mut sink = if continues {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .await
        } else {
            tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&part)
                .await
        }
        .map_err(|source| TtsError::io(&part, source))?;

        let mut received = resume_from;
        if resume_from > 0 {
            // The bytes of the interrupted download are already on the disk,
            // they belong to the progress of the file.
            counter.fetch_add(resume_from, std::sync::atomic::Ordering::Relaxed);
        }
        while let Some(chunk) = response.chunk().await.map_err(|err| {
            TtsError::Engine(format!("failed to download `{}`: {err}", file.path))
        })? {
            tokio::io::AsyncWriteExt::write_all(&mut sink, &chunk)
                .await
                .map_err(|source| TtsError::io(&part, source))?;
            received += chunk.len() as u64;
            counter.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Relaxed);
            if let Some(tracker) = tracker {
                tracker.update(&spec.id, |progress| {
                    progress.bytes = counter.load(std::sync::atomic::Ordering::Relaxed);
                });
            }
        }

        tokio::io::AsyncWriteExt::flush(&mut sink)
            .await
            .map_err(|source| TtsError::io(&part, source))?;
        drop(sink);

        // A file that is shorter than announced is broken, the next attempt
        // downloads it again.
        if let Some(expected) = file.size {
            if received != expected {
                let _ = std::fs::remove_file(&part);
                return Err(TtsError::Engine(format!(
                    "failed to download `{}`: the hub delivered {} bytes instead of {}",
                    file.path, received, expected
                )));
            }
        }

        std::fs::rename(&part, destination).map_err(|source| TtsError::Io {
            path: destination.to_path_buf(),
            source,
        })?;

        Ok(received)
    }

    /// Removes the checkpoint from the disk.
    pub fn uninstall(&self, spec: &TTSModelSpec) -> TtsResult<()> {
        let directory = self.target_dir(spec);
        tracing::warn!(
            model = %spec.id,
            path = %directory.display(),
            "removing the checkpoint from the disk"
        );
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

/// The path of the temporary file that carries a running download.
fn part_path(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    destination.with_file_name(format!("{name}.part"))
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
