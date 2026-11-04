//! Factory that creates the adapter matching a model.

use std::sync::Arc;

use crate::adapter::engine::{EngineSettings, SGLangOmniAdapter};
use crate::adapter::fish_drama3::{FishDrama3Adapter, FishDrama3Protocol};
use crate::adapter::mock::MockAdapter;
use crate::adapter::qwen3_tts::{Qwen3TTSAdapter, Qwen3TTSProtocol};
use crate::adapter::TTSAdapter;
use crate::error::TtsResult;
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

    /// Creates the adapter that serves the given model.
    pub fn create(&self, spec: &TTSModelSpec) -> TtsResult<Arc<dyn TTSAdapter>> {
        Ok(match spec.family {
            ModelFamily::Qwen3Tts => {
                Arc::new(Qwen3TTSAdapter::new(self.engine.clone())) as Arc<dyn TTSAdapter>
            }
            ModelFamily::FishDrama3 => {
                Arc::new(FishDrama3Adapter::new(self.engine.clone())) as Arc<dyn TTSAdapter>
            }
            ModelFamily::Mock => Arc::new(MockAdapter::new()) as Arc<dyn TTSAdapter>,
        })
    }
}
