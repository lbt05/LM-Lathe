//! Mock adapter.
//!
//! The mock engine produces a synthetic sine wave instead of real speech. It
//! makes the whole pipeline - runtime manager, scheduler, voice library,
//! audio manager and HTTP API - testable on machines without a GPU.

use std::sync::RwLock;

use async_trait::async_trait;

use crate::adapter::TTSAdapter;
use crate::error::{TtsError, TtsResult};
use crate::types::{
    AudioFormat, AudioOutput, HealthStatus, RuntimeHandle, TTSRequest, TTSRuntimeConfig,
};

/// Adapter that synthesises a deterministic tone.
#[derive(Debug, Default)]
pub struct MockAdapter {
    handle: RwLock<Option<RuntimeHandle>>,
    sample_rate: u32,
}

impl MockAdapter {
    /// Creates a mock adapter.
    pub fn new() -> Self {
        Self {
            handle: RwLock::new(None),
            sample_rate: 24_000,
        }
    }

    /// Generates the WAV payload for a request.
    fn render(&self, request: &TTSRequest) -> AudioOutput {
        let sample_rate = self.sample_rate;
        let duration_ms = std::time::Duration::from_millis(
            (request.text.chars().count() as u64 * 80).clamp(400, 8_000),
        );
        let frames = (sample_rate as u64 * duration_ms.as_millis() as u64 / 1000) as usize;
        let frequency = 180.0 + (request.text.len() % 240) as f32;
        let mut samples = Vec::with_capacity(frames);
        for index in 0..frames {
            let t = index as f32 / sample_rate as f32;
            samples.push((2.0 * std::f32::consts::PI * frequency * t).sin() * 0.2);
        }
        let pcm = crate::audio::PcmSamples {
            samples,
            sample_rate,
            channels: 1,
        };
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::new(&mut buffer, spec).expect("in-memory wav");
            for sample in &pcm.samples {
                writer
                    .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .expect("in-memory wav");
            }
            writer.finalize().expect("in-memory wav");
        }
        AudioOutput {
            data: buffer.into_inner(),
            format: AudioFormat::Wav,
            sample_rate: Some(sample_rate),
            channels: Some(1),
            duration_ms: Some(duration_ms.as_millis() as u64),
            finish_reason: Some("stop".to_string()),
        }
    }
}

#[async_trait]
impl TTSAdapter for MockAdapter {
    async fn start(&self, _config: TTSRuntimeConfig) -> TtsResult<RuntimeHandle> {
        let handle = RuntimeHandle {
            model_id: "mock".to_string(),
            model_repo: Some("mock://echo".to_string()),
            engine_id: "mock".to_string(),
            process_id: None,
            endpoint: Some("mock://local".to_string()),
            started_at: chrono::Utc::now(),
            log_path: None,
            mode: crate::types::EngineMode::Mock,
        };
        *self.handle.write().unwrap_or_else(|e| e.into_inner()) = Some(handle.clone());
        Ok(handle)
    }

    async fn stop(&self, _handle: RuntimeHandle) -> TtsResult<()> {
        *self.handle.write().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    async fn health_check(&self, _handle: &RuntimeHandle) -> TtsResult<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }

    async fn synthesize(&self, request: TTSRequest) -> TtsResult<AudioOutput> {
        if self
            .handle
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            return Err(TtsError::Engine(
                "the mock engine is not running".to_string(),
            ));
        }
        Ok(self.render(&request))
    }

    async fn unload_model(&self, _handle: &RuntimeHandle) -> TtsResult<()> {
        Ok(())
    }

    async fn get_logs(&self, _handle: &RuntimeHandle) -> TtsResult<String> {
        Ok("mock engine: no logs available".to_string())
    }
}
