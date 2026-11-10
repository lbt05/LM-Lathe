//! File system implementation of the voice library.
//!
//! ```text
//! voices/
//!   voice_001/
//!     profile.json
//!     samples/
//!       voice_001_001.wav
//! ```

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::{TtsError, TtsResult};
use crate::types::AudioOutput;
use crate::voice::{
    NewVoiceProfile, VoiceFilter, VoiceLibrary, VoicePatch, VoiceProfile, VoiceSample,
};

/// Voice library backed by the local file system.
#[derive(Debug, Clone)]
pub struct FsVoiceLibrary {
    root: PathBuf,
}

impl FsVoiceLibrary {
    /// Creates a library rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Root directory of the library.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory of a single voice.
    pub fn voice_dir(&self, id: &str) -> PathBuf {
        self.root.join(sanitize(id))
    }

    /// Path of the profile file.
    pub fn profile_path(&self, id: &str) -> PathBuf {
        self.voice_dir(id).join("profile.json")
    }

    /// Directory holding the samples of a voice.
    pub fn samples_dir(&self, id: &str) -> PathBuf {
        self.voice_dir(id).join("samples")
    }

    /// Writes a profile to disk.
    fn write_profile(&self, profile: &VoiceProfile) -> TtsResult<()> {
        let dir = self.voice_dir(&profile.id);
        std::fs::create_dir_all(&dir).map_err(|e| TtsError::io(&dir, e))?;
        std::fs::create_dir_all(self.samples_dir(&profile.id))
            .map_err(|e| TtsError::io(self.samples_dir(&profile.id), e))?;
        let path = self.profile_path(&profile.id);
        let raw = serde_json::to_string_pretty(profile).map_err(TtsError::serialization)?;
        std::fs::write(&path, raw).map_err(|e| TtsError::io(&path, e))?;
        Ok(())
    }

    /// Reads a profile from disk.
    fn read_profile(&self, id: &str) -> TtsResult<Option<VoiceProfile>> {
        let path = self.profile_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path).map_err(|e| TtsError::io(&path, e))?;
        let profile = serde_json::from_str(&raw).map_err(TtsError::serialization)?;
        Ok(Some(profile))
    }
}

#[async_trait]
impl VoiceLibrary for FsVoiceLibrary {
    async fn create(&self, profile: NewVoiceProfile) -> TtsResult<VoiceProfile> {
        let id = format!("voice_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let now = chrono::Utc::now();
        let voice = VoiceProfile {
            id,
            name: profile.name.unwrap_or_else(|| "Untitled voice".to_string()),
            description: profile.description.unwrap_or_default(),
            language: profile.language,
            gender_presentation: profile.gender_presentation,
            age_style: profile.age_style,
            personality: profile.personality,
            engine: profile.engine.unwrap_or_else(|| "unknown".to_string()),
            model_id: profile.model_id.unwrap_or_else(|| "unknown".to_string()),
            engine_config: if profile.engine_config.is_null() {
                serde_json::Value::Object(serde_json::Map::new())
            } else {
                profile.engine_config
            },
            samples: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.write_profile(&voice)?;
        Ok(voice)
    }

    async fn get(&self, id: &str) -> TtsResult<Option<VoiceProfile>> {
        self.read_profile(id)
    }

    async fn list(&self, filter: VoiceFilter) -> TtsResult<Vec<VoiceProfile>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut profiles = Vec::new();
        let entries = std::fs::read_dir(&self.root).map_err(|e| TtsError::io(&self.root, e))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if let Some(profile) = self.read_profile(id)? {
                if matches(&filter, &profile) {
                    profiles.push(profile);
                }
            }
        }
        profiles.sort_by_key(|profile| profile.created_at);
        Ok(profiles)
    }

    async fn update(&self, id: &str, patch: VoicePatch) -> TtsResult<VoiceProfile> {
        let mut profile = self
            .read_profile(id)?
            .ok_or_else(|| TtsError::VoiceNotFound(id.to_string()))?;
        if let Some(name) = patch.name {
            profile.name = name;
        }
        if let Some(description) = patch.description {
            profile.description = description;
        }
        if let Some(language) = patch.language {
            profile.language = Some(language);
        }
        if let Some(gender) = patch.gender_presentation {
            profile.gender_presentation = Some(gender);
        }
        if let Some(age) = patch.age_style {
            profile.age_style = Some(age);
        }
        if let Some(personality) = patch.personality {
            profile.personality = Some(personality);
        }
        if let Some(engine_config) = patch.engine_config {
            profile.engine_config = engine_config;
        }
        profile.updated_at = chrono::Utc::now();
        self.write_profile(&profile)?;
        Ok(profile)
    }

    async fn delete(&self, id: &str) -> TtsResult<()> {
        let dir = self.voice_dir(id);
        if !dir.exists() {
            return Err(TtsError::VoiceNotFound(id.to_string()));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| TtsError::io(&dir, e))?;
        Ok(())
    }

    async fn add_sample(
        &self,
        id: &str,
        audio: &AudioOutput,
        label: Option<&str>,
    ) -> TtsResult<VoiceSample> {
        let mut profile = self
            .read_profile(id)?
            .ok_or_else(|| TtsError::VoiceNotFound(id.to_string()))?;
        let samples_dir = self.samples_dir(id);
        std::fs::create_dir_all(&samples_dir).map_err(|e| TtsError::io(&samples_dir, e))?;
        let index = profile.samples.len() + 1;
        let base = label
            .map(sanitize)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| profile.id.clone());
        let name = format!("{base}_{index:03}.{}", audio.format.extension());
        let path = samples_dir.join(&name);
        std::fs::write(&path, &audio.data).map_err(|e| TtsError::io(&path, e))?;
        let relative = format!("samples/{name}");
        if !profile.samples.contains(&relative) {
            profile.samples.push(relative.clone());
        }
        profile.updated_at = chrono::Utc::now();
        self.write_profile(&profile)?;
        Ok(VoiceSample {
            name: relative,
            path: path.to_string_lossy().to_string(),
            duration_ms: audio.duration_ms,
        })
    }

    async fn remove_sample(&self, id: &str, sample: &str) -> TtsResult<()> {
        let mut profile = self
            .read_profile(id)?
            .ok_or_else(|| TtsError::VoiceNotFound(id.to_string()))?;
        let path = self.voice_dir(id).join(sample);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| TtsError::io(&path, e))?;
        }
        profile.samples.retain(|entry| entry != sample);
        profile.updated_at = chrono::Utc::now();
        self.write_profile(&profile)?;
        Ok(())
    }

    async fn sample_path(&self, id: &str, sample: &str) -> TtsResult<PathBuf> {
        // The name comes from the outside, so it has to stay inside the
        // directory of the voice.
        if sample.is_empty()
            || sample.starts_with('/')
            || sample
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(TtsError::InvalidRequest(format!(
                "invalid sample name `{sample}`"
            )));
        }
        let direct = self.voice_dir(id).join(sample);
        if direct.exists() {
            return Ok(direct);
        }
        // The samples live below the `samples` directory of the voice. The
        // route already names that directory, so the name of the sample may
        // come with it or without it.
        let nested = self.samples_dir(id).join(sample);
        if nested.exists() {
            return Ok(nested);
        }
        Err(TtsError::InvalidRequest(format!(
            "sample `{sample}` does not exist"
        )))
    }

    async fn samples(&self, id: &str) -> TtsResult<Vec<VoiceSample>> {
        let profile = self
            .read_profile(id)?
            .ok_or_else(|| TtsError::VoiceNotFound(id.to_string()))?;
        let mut samples = Vec::new();
        for relative in profile.samples {
            let path = self.voice_dir(id).join(&relative);
            let duration_ms = crate::audio::inspect_file(&path)
                .ok()
                .map(|info| info.duration_ms);
            samples.push(VoiceSample {
                name: relative,
                path: path.to_string_lossy().to_string(),
                duration_ms,
            });
        }
        Ok(samples)
    }
}

/// Checks whether a profile matches a filter.
fn matches(filter: &VoiceFilter, profile: &VoiceProfile) -> bool {
    if let Some(model_id) = &filter.model_id {
        if &profile.model_id != model_id {
            return false;
        }
    }
    if let Some(engine) = &filter.engine {
        if &profile.engine != engine {
            return false;
        }
    }
    if let Some(language) = &filter.language {
        if profile.language.as_deref() != Some(language.as_str()) {
            return false;
        }
    }
    if let Some(query) = &filter.query {
        let query = query.to_lowercase();
        let haystack = format!(
            "{} {} {}",
            profile.name,
            profile.description,
            profile.personality.clone().unwrap_or_default()
        )
        .to_lowercase();
        if !haystack.contains(&query) {
            return false;
        }
    }
    true
}

/// Removes characters that are unsafe in file names.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}
