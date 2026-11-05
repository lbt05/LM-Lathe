//! Filesystem layout of LM-Lathe.
//!
//! ```text
//! ~/.lm-lathe/
//!   models/
//!     llm/
//!     tts/
//!   voices/
//!     voice_001/
//!       profile.json
//!       samples/
//!   audio/
//!     generated/
//!   logs/
//!     tts/
//! ```

use std::path::{Path, PathBuf};

use crate::error::{TtsError, TtsResult};

/// Environment variable that overrides the root directory.
pub const ROOT_ENV: &str = "LM_LATHE_HOME";

/// The directory of the first versions of the application. It is still read
/// when it is set, so that an old installation keeps working.
pub const LEGACY_ROOT_ENV: &str = "LOCAL_AI_MANAGER_HOME";

/// Resolved locations of every asset directory.
#[derive(Debug, Clone)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// Creates paths rooted at `root`.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Discovers the root directory.
    ///
    /// `LM_LATHE_HOME` wins, then the name of the first versions, otherwise
    /// `~/.lm-lathe` is used.
    pub fn discover() -> Self {
        let root = std::env::var_os(ROOT_ENV)
            .or_else(|| std::env::var_os(LEGACY_ROOT_ENV))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                home.join(".lm-lathe")
            });
        Self::with_root(root)
    }

    /// Root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `models/`
    pub fn models_root(&self) -> PathBuf {
        self.root.join("models")
    }

    /// `models/llm/`
    pub fn llm_models_root(&self) -> PathBuf {
        self.models_root().join("llm")
    }

    /// `models/tts/`
    pub fn tts_models_root(&self) -> PathBuf {
        self.models_root().join("tts")
    }

    /// `voices/`
    pub fn voices_root(&self) -> PathBuf {
        self.root.join("voices")
    }

    /// `audio/`
    pub fn audio_root(&self) -> PathBuf {
        self.root.join("audio")
    }

    /// `audio/generated/`
    pub fn generated_audio_root(&self) -> PathBuf {
        self.audio_root().join("generated")
    }

    /// `logs/`
    pub fn logs_root(&self) -> PathBuf {
        self.root.join("logs")
    }
    /// `runtimes/`
    ///
    /// Holds the installed inference engines, one directory per engine and
    /// version.
    pub fn runtimes_root(&self) -> PathBuf {
        self.root.join("runtimes")
    }

    /// `runtimes/engines/`
    pub fn engines_root(&self) -> PathBuf {
        self.runtimes_root().join("engines")
    }

    /// `logs/tts/`
    pub fn tts_logs_root(&self) -> PathBuf {
        self.logs_root().join("tts")
    }

    /// `logs/llm/`
    pub fn llm_logs_root(&self) -> PathBuf {
        self.logs_root().join("llm")
    }

    /// Creates every directory the subsystem needs.
    pub fn ensure(&self) -> TtsResult<()> {
        for dir in [
            self.models_root(),
            self.llm_models_root(),
            self.tts_models_root(),
            self.voices_root(),
            self.audio_root(),
            self.generated_audio_root(),
            self.logs_root(),
            self.tts_logs_root(),
            self.llm_logs_root(),
            self.runtimes_root(),
            self.engines_root(),
        ] {
            std::fs::create_dir_all(&dir).map_err(|e| TtsError::io(&dir, e))?;
        }
        Ok(())
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::discover()
    }
}
