//! Adoption of the checkpoints that are on the disk but not in the catalogue.
//!
//! The catalogue of the application lives in the memory of the service: it
//! holds the models that ship with the application and the ones that were
//! found in a search of the hub. A model of a search is added to the
//! catalogue while it is downloaded, and nothing writes it down afterwards.
//! A restart of the service therefore forgets it: the files of the
//! checkpoint are still on the disk, but no model knows them any more, so
//! the interface shows them as if they had never been downloaded.
//!
//! This module reads the directories of the checkpoints and rebuilds the
//! specifications of the ones that the catalogue does not know. The disk is
//! the truth: a checkpoint that is there can be shown, loaded and deleted
//! again, even when the application never saw it before.
//!
//! The rebuild works without the network: the name of the directory is the
//! name of the repository with the slash replaced by two dashes, and the
//! files on the disk tell the format of the checkpoint. The hub is only
//! asked afterwards, to learn the things that only the hub knows — the
//! licence, the number of the parameters, and the exact name of the
//! repository when the owner of it holds a dash itself.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::hub::{HubClient, HubModel, HubSibling};
use crate::installer::RepoFile;
use crate::registry::{TTSModelRegistry, TTSModelSpec};

/// A checkpoint that was found on the disk.
#[derive(Debug, Clone)]
pub struct AdoptedCheckpoint {
    /// The specification that was rebuilt from the files on the disk.
    pub spec: TTSModelSpec,
    /// The directory the checkpoint lives in.
    pub directory: PathBuf,
    /// The repositories the name of the directory may belong to.
    ///
    /// The name of the directory is the name of the repository with the
    /// slash replaced by two dashes, so the owner and the name of the
    /// repository cannot always be told apart: an owner may hold a dash
    /// itself. The hub decides which of the candidates is the right one.
    pub candidates: Vec<String>,
}

/// Scans the directories of the checkpoints.
///
/// Every directory that holds weights and that the catalogue does not know
/// is turned into a model again. A directory without weights belongs to a
/// download that was interrupted: the progress of the download knows about
/// it, the catalogue does not.
pub fn scan(registry: &TTSModelRegistry) -> Vec<AdoptedCheckpoint> {
    let known = known_directories(registry);
    let mut adopted = Vec::new();
    for root in [registry.models_root(), registry.llm_models_root()] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        // The order of the directories does not depend on the file system,
        // so the catalogue looks the same on every start.
        let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') || known.contains(name) {
                continue;
            }
            let Some(checkpoint) = adopt(&path, name) else {
                continue;
            };
            adopted.push(checkpoint);
        }
    }
    adopted
}

/// Adds the adopted checkpoints to the catalogue.
///
/// The number of the models that were added is returned.
pub fn apply(registry: &TTSModelRegistry, adopted: &[AdoptedCheckpoint]) -> usize {
    for checkpoint in adopted {
        registry.insert(checkpoint.spec.clone());
    }
    adopted.len()
}

/// Asks the hub about the checkpoints that were adopted.
///
/// The name of the directory tells the repository only when the owner of it
/// does not hold a dash itself. The hub decides which of the candidates is
/// the right one, and it knows the licence and the number of the parameters
/// of the checkpoint as well. A hub that cannot be reached changes nothing:
/// the model that was rebuilt from the disk stays as it is.
pub async fn enrich(hub: &HubClient, registry: &TTSModelRegistry, adopted: &[AdoptedCheckpoint]) {
    for checkpoint in adopted {
        let Some(model) = resolve(hub, &checkpoint.candidates).await else {
            continue;
        };
        let files = hub.files(&model.id).await.unwrap_or_default();
        let Ok(mut spec) = model.to_spec(&files) else {
            continue;
        };
        // The disk is the truth: the name of the directory, the directory
        // itself and the files that are on it stay as they are.
        spec.id = checkpoint.spec.id.clone();
        spec.local_path = Some(checkpoint.directory.clone());
        spec.files = checkpoint.spec.files.clone();
        spec.size_bytes = checkpoint.spec.size_bytes;
        spec.estimated_memory_bytes = checkpoint.spec.estimated_memory_bytes;
        spec.parameters = parameters(hub, &model).await;
        spec.notes = Some(notes(&model));
        registry.insert(spec);
    }
}

/// The directories the catalogue already knows.
fn known_directories(registry: &TTSModelRegistry) -> HashSet<String> {
    registry
        .list()
        .iter()
        .filter_map(|spec| registry.install_path(&spec.id))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
        })
        .collect()
}

/// Rebuilds the model of a directory of a checkpoint.
fn adopt(directory: &Path, name: &str) -> Option<AdoptedCheckpoint> {
    let files = collect_files(directory);
    // A download that was interrupted leaves a directory without weights.
    if !files.iter().any(|file| is_weight(&file.path)) {
        return None;
    }
    let candidates = repo_candidates(name);
    let repo = candidates.first()?.clone();
    let model = local_model(&repo, &files);
    // The runtime of the application cannot serve everything that carries
    // weights: a directory of an unknown kind stays a directory.
    if !model.is_runnable() {
        return None;
    }
    let mut spec = model.to_spec(&files).ok()?;
    // The checkpoint lives where it lives: the path is not derived from the
    // name of the repository, because the two can differ.
    spec.local_path = Some(directory.to_path_buf());
    let size: u64 = files.iter().filter_map(|file| file.size).sum();
    spec.size_bytes = size;
    spec.estimated_memory_bytes = size + size / 4;
    spec.notes = Some(RECOVERED.to_string());
    Some(AdoptedCheckpoint {
        spec,
        directory: directory.to_path_buf(),
        candidates,
    })
}

/// The note that tells the user where the model comes from.
const RECOVERED: &str = "这个检查点是在启动时从磁盘上的目录恢复的。";

/// The note of a model, with the licence the hub knows.
fn notes(model: &HubModel) -> String {
    match model.license() {
        Some(license) => format!("{RECOVERED} 许可：{license}"),
        None => RECOVERED.to_string(),
    }
}

/// The number of the parameters of a model, as the hub knows it.
async fn parameters(hub: &HubClient, model: &HubModel) -> Option<u64> {
    if model.format() == "gguf" {
        // A GGUF repository carries the number of its parameters in its name.
        model.parameters_from_name()
    } else {
        hub.detail(&model.id, true)
            .await
            .ok()
            .and_then(|detail| detail.safetensors_parameters)
    }
}

/// Asks the hub which of the candidates is the repository.
async fn resolve(hub: &HubClient, candidates: &[String]) -> Option<HubModel> {
    for candidate in candidates {
        if let Ok(model) = hub.model(candidate).await {
            return Some(model);
        }
    }
    None
}

/// The repositories the name of a directory may belong to.
///
/// The name of the directory is the name of the repository with the slash
/// replaced by two dashes. Every pair of dashes is a possible place for the
/// slash, so every one of them is a candidate. The first one is the most
/// likely: an owner rarely holds a dash himself.
fn repo_candidates(name: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut rest = name;
    while let Some((owner, repository)) = rest.split_once("--") {
        if !owner.is_empty() && !repository.is_empty() {
            candidates.push(format!("{owner}/{repository}"));
        }
        rest = repository;
    }
    candidates
}

/// The model of the hub that the files on the disk describe.
///
/// The hub answers a search with the metadata of a repository; the files on
/// the disk tell the same story well enough for a model that is not in the
/// catalogue yet.
fn local_model(repo: &str, files: &[RepoFile]) -> HubModel {
    let mut tags = Vec::new();
    let name = repo.to_lowercase();
    // The name of the repository tells the family of a speech checkpoint.
    let mut speech = name.contains("qwen3-tts") || name.contains("qwen3tts");
    if name.contains("drama3") || name.contains("drama-3") {
        speech = true;
        tags.push("drama3".to_string());
    }
    if speech {
        tags.push("qwen3_tts".to_string());
    }
    if name.contains("mlx") {
        tags.push("mlx".to_string());
    }
    HubModel {
        id: repo.to_string(),
        pipeline_tag: None,
        library_name: None,
        downloads: 0,
        likes: 0,
        last_modified: None,
        created_at: None,
        tags,
        siblings: files
            .iter()
            .map(|file| HubSibling {
                rfilename: file.path.clone(),
            })
            .collect(),
        size_bytes: None,
        parameters: None,
    }
}

/// The files of a checkpoint, with their sizes.
///
/// The paths are relative to the directory of the checkpoint, so that they
/// look like the paths of the repository they came from.
fn collect_files(directory: &Path) -> Vec<RepoFile> {
    let mut files = Vec::new();
    let mut stack = vec![(directory.to_path_buf(), String::new())];
    while let Some((path, prefix)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, relative));
                continue;
            }
            let size = entry.metadata().ok().map(|meta| meta.len());
            files.push(RepoFile {
                kind: "file".to_string(),
                path: relative,
                size,
            });
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

/// `true` when the file holds the weights of a checkpoint.
///
/// The list is the one of the registry, so that a checkpoint is installed
/// and adopted under the same rule.
fn is_weight(path: &str) -> bool {
    path.ends_with(".safetensors")
        || path.ends_with(".safetensors.index.json")
        || path.ends_with(".gguf")
        || path.ends_with(".bin")
        || path.ends_with(".pt")
}
