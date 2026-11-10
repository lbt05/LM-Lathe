//! Factory that creates the adapter matching a model.

use std::sync::Arc;

use crate::adapter::engine::{EngineSettings, SGLangOmniAdapter};
use crate::adapter::fish_drama3::{FishDrama3Adapter, FishDrama3Protocol};
#[cfg(feature = "mock-engine")]
use crate::adapter::mock::MockAdapter;
use crate::adapter::qwen3_tts::{Qwen3TTSAdapter, Qwen3TTSProtocol};
use crate::adapter::TTSAdapter;
use crate::error::{TtsError, TtsResult};
use crate::registry::TTSModelSpec;
use crate::types::ModelFamily;

/// Creates the adapters of the TTS subsystem.
///
/// The factory owns a single engine adapter so that the inference service is
/// never managed twice, exactly as required by the specification.
pub struct AdapterFactory {
    engine: Arc<SGLangOmniAdapter>,
}

impl AdapterFactory {
    /// Creates a factory with the given engine settings.
    pub fn new(settings: EngineSettings) -> Self {
        let engine = SGLangOmniAdapter::new(settings);
        engine.register_protocol(ModelFamily::Qwen3Tts, Arc::new(Qwen3TTSProtocol));
        engine.register_protocol(ModelFamily::FishDrama3, Arc::new(FishDrama3Protocol));
        Self { engine }
    }

    /// The shared engine adapter.
    pub fn engine(&self) -> &Arc<SGLangOmniAdapter> {
        &self.engine
    }

    /// The log of the last start attempt of a model.
    pub fn logs_for(&self, model_id: &str) -> String {
        self.engine.logs_for(model_id)
    }

    /// The program that serves the model, when the engine is there.
    ///
    /// A model whose engine is not installed has no program. The check runs
    /// before the memory of the other models is taken: a missing engine
    /// would evict them for nothing.
    pub fn engine_program(&self, spec: &TTSModelSpec) -> Option<std::path::PathBuf> {
        // The mock engine is part of the application: it has no program.
        #[cfg(feature = "mock-engine")]
        if spec.mode == crate::types::EngineMode::Mock {
            return Some(std::path::PathBuf::new());
        }
        let settings = self.engine.settings();
        let (engine_id, fallback) = match spec.mode {
            crate::types::EngineMode::MlxNative => {
                (spec.engine_id.as_str(), settings.mlx_binary.as_str())
            }
            crate::types::EngineMode::MlxAudio => {
                ("mlx-audio", settings.mlx_audio_binary.as_str())
            }
            crate::types::EngineMode::SglangOmni => {
                ("sglang-omni", settings.sglang_binary.as_str())
            }
            _ => (spec.engine_id.as_str(), settings.sglang_binary.as_str()),
        };
        settings.program_for(engine_id, fallback)
    }

    /// Creates the adapter that serves the given model.
    pub fn create(&self, spec: &TTSModelSpec) -> TtsResult<Arc<dyn TTSAdapter>> {
        Ok(match spec.family {
            ModelFamily::Qwen3Tts => {
                Arc::new(Qwen3TTSAdapter::new(self.engine.clone())) as Arc<dyn TTSAdapter>
            }
            ModelFamily::FishDrama3 => {
                Arc::new(FishDrama3Adapter::new(self.engine.clone())) as Arc<dyn TTSAdapter>
            }
            #[cfg(feature = "mock-engine")]
            ModelFamily::Mock => Arc::new(MockAdapter::new()) as Arc<dyn TTSAdapter>,
            ModelFamily::Llm => {
                return Err(TtsError::AdapterNotFound(
                    "the language models are served by their own runtime".to_string(),
                ))
            }
        })
    }
}
