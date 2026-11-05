//! Tests of the model specific payload translation.

use tts_core::adapter::fish_drama3::FishDrama3Protocol;
use tts_core::adapter::protocol::RequestContext;
use tts_core::adapter::qwen3_tts::Qwen3TTSProtocol;
use tts_core::adapter::ModelProtocol;
use tts_core::types::{
    AudioReference, EngineMode, EngineSpecificParameters, FishDrama3Parameters, Qwen3TtsParameters,
    Qwen3TtsTaskType, TTSRequest,
};
use tts_core::voice::VoiceProfile;

fn qwen_request(parameters: EngineSpecificParameters) -> TTSRequest {
    TTSRequest {
        model_id: "qwen3-tts-1.7b-voicedesign".to_string(),
        voice_id: None,
        text: "你好".to_string(),
        language: Some("Chinese".to_string()),
        parameters,
        response_format: tts_core::types::AudioFormat::Wav,
        speed: None,
    }
}

fn voice_design_repo() -> RequestContext {
    RequestContext {
        mode: EngineMode::SglangOmni,
        model_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign".to_string(),
        model_path: None,
    }
}

#[test]
fn voice_design_requires_instructions() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        task_type: Some(Qwen3TtsTaskType::VoiceDesign),
        instructions: None,
        ..Default::default()
    }));
    let error = protocol
        .build_payload(&request, &voice_design_repo())
        .unwrap_err();
    assert!(error.to_string().contains("instructions"));
}

#[test]
fn voice_design_payload_uses_the_task_type_and_the_instructions() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        task_type: Some(Qwen3TtsTaskType::VoiceDesign),
        instructions: Some("青年女性，稚嫩、害羞".to_string()),
        ..Default::default()
    }));
    let payload = protocol
        .build_payload(&request, &voice_design_repo())
        .unwrap();
    assert_eq!(payload["task_type"], "VoiceDesign");
    assert_eq!(payload["instructions"], "青年女性，稚嫩、害羞");
    assert_eq!(payload["input"], "你好");
    assert_eq!(payload["language"], "Chinese");
    assert_eq!(payload["model"], "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign");
    assert_eq!(payload["voice"], "default");
    assert_eq!(payload["response_format"], "wav");
}

#[test]
fn custom_voice_omits_the_task_type_and_uses_the_speaker() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        speaker: Some("Ryan".to_string()),
        ..Default::default()
    }));
    let ctx = RequestContext {
        mode: EngineMode::SglangOmni,
        model_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice".to_string(),
        model_path: None,
    };
    let payload = protocol.build_payload(&request, &ctx).unwrap();
    assert!(payload.get("task_type").is_none());
    assert_eq!(payload["voice"], "Ryan");
}

#[test]
fn a_voice_design_checkpoint_without_instructions_is_refused() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        speaker: Some("Ryan".to_string()),
        ..Default::default()
    }));
    let error = protocol
        .build_payload(&request, &voice_design_repo())
        .unwrap_err();
    assert!(error.to_string().contains("instructions"));
}

#[test]
fn the_task_type_is_derived_from_the_checkpoint() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        instructions: Some("a calm voice".to_string()),
        ..Default::default()
    }));
    let payload = protocol
        .build_payload(&request, &voice_design_repo())
        .unwrap();
    assert_eq!(payload["task_type"], "VoiceDesign");

    let custom = RequestContext {
        mode: EngineMode::SglangOmni,
        model_repo: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice".to_string(),
        model_path: None,
    };
    let payload = protocol.build_payload(&request, &custom).unwrap();
    assert!(payload.get("task_type").is_none());
    assert_eq!(payload["instructions"], "a calm voice");
}

#[test]
fn a_voice_profile_is_merged_into_the_request() {
    let protocol = Qwen3TTSProtocol;
    let mut request = qwen_request(EngineSpecificParameters::Qwen3Tts(
        Qwen3TtsParameters::default(),
    ));
    let profile = VoiceProfile {
        id: "voice_001".to_string(),
        name: "府中贴身婢女".to_string(),
        description: "青年女性".to_string(),
        language: Some("Chinese".to_string()),
        gender_presentation: None,
        age_style: None,
        personality: None,
        engine: "qwen3-tts".to_string(),
        model_id: "qwen3-tts-1.7b-voicedesign".to_string(),
        engine_config: serde_json::json!({
            "voice_design_prompt": "青年女性，稚嫩、害羞、略带怯意",
            "reference_audio": "/tmp/reference.wav",
            "reference_text": "参考文本"
        }),
        samples: Vec::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    protocol.hydrate(&mut request, &profile).unwrap();
    let params = request.parameters.as_qwen3_tts().unwrap();
    assert_eq!(
        params.instructions.as_deref(),
        Some("青年女性，稚嫩、害羞、略带怯意")
    );
    assert_eq!(params.references.len(), 1);
    assert_eq!(
        params.references[0].audio_path.as_deref(),
        Some("/tmp/reference.wav")
    );
    assert_eq!(params.references[0].text.as_deref(), Some("参考文本"));
}

#[test]
fn drama_3_sends_the_references() {
    let protocol = FishDrama3Protocol;
    let request = TTSRequest {
        model_id: "drama-3".to_string(),
        voice_id: None,
        text: "<|speaker:0|> 你好 <|speaker:1|> 再见".to_string(),
        language: None,
        parameters: EngineSpecificParameters::FishDrama3(FishDrama3Parameters {
            references: vec![AudioReference {
                audio_path: Some("/tmp/a.wav".to_string()),
                text: Some("参考".to_string()),
                ..Default::default()
            }],
            previous_takes: vec!["上一句".to_string()],
            seed: Some(7),
        }),
        response_format: tts_core::types::AudioFormat::Wav,
        speed: None,
    };
    let ctx = RequestContext {
        mode: EngineMode::SglangOmni,
        model_repo: "fishaudio/drama-3".to_string(),
        model_path: None,
    };
    let payload = protocol.build_payload(&request, &ctx).unwrap();
    assert_eq!(payload["input"], "<|speaker:0|> 你好 <|speaker:1|> 再见");
    assert_eq!(payload["model"], "fishaudio/drama-3");
    assert_eq!(payload["references"][0]["audio_path"], "/tmp/a.wav");
    assert_eq!(payload["references"][0]["text"], "参考");
    assert_eq!(payload["previous_takes"][0], "上一句");
    assert_eq!(payload["seed"], 7);
}

#[test]
fn the_native_mlx_runtime_refuses_remote_references() {
    let protocol = FishDrama3Protocol;
    let request = TTSRequest {
        model_id: "drama-3-mlx-4bit".to_string(),
        voice_id: None,
        text: "你好".to_string(),
        language: None,
        parameters: EngineSpecificParameters::FishDrama3(FishDrama3Parameters {
            references: vec![AudioReference {
                audio_path: Some("https://example.com/a.wav".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }),
        response_format: tts_core::types::AudioFormat::Wav,
        speed: None,
    };
    let ctx = RequestContext {
        mode: EngineMode::MlxNative,
        model_repo: "mlx-community/fish-audio-drama-3-4bit".to_string(),
        model_path: None,
    };
    let error = protocol.build_payload(&request, &ctx).unwrap_err();
    assert!(error.to_string().contains("local reference clips"));
}

/// The context of a checkpoint that mlx-audio serves.
fn mlx_audio_ctx(repo: &str) -> RequestContext {
    RequestContext {
        mode: EngineMode::MlxAudio,
        model_repo: repo.to_string(),
        model_path: Some("/models/tts/mlx-community--Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit".to_string()),
    }
}

#[test]
fn the_mlx_audio_payload_names_the_checkpoint_of_the_disk() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        task_type: Some(Qwen3TtsTaskType::VoiceDesign),
        instructions: Some("一位三十岁左右的女性".to_string()),
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit");
    let payload = protocol.build_payload(&request, &ctx).unwrap();
    // The server loads the checkpoint of the request, so the path on the
    // disk is the name of the model.
    assert_eq!(
        payload["model"],
        "/models/tts/mlx-community--Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit"
    );
    // mlx-audio calls the description `instruct` and knows the kind of the
    // checkpoint itself.
    assert_eq!(payload["instruct"], "一位三十岁左右的女性");
    assert!(payload.get("task_type").is_none());
    assert!(payload.get("instructions").is_none());
    assert_eq!(payload["response_format"], "wav");
    // The language is named in English.
    assert_eq!(payload["lang_code"], "chinese");
}

#[test]
fn the_mlx_audio_payload_of_a_custom_voice_checkpoint_names_the_speaker() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        speaker: Some("Vivian".to_string()),
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-CustomVoice-4bit");
    let payload = protocol.build_payload(&request, &ctx).unwrap();
    assert_eq!(payload["voice"], "Vivian");
}

#[test]
fn a_custom_voice_checkpoint_without_a_speaker_is_refused() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-CustomVoice-4bit");
    let error = protocol.build_payload(&request, &ctx).unwrap_err();
    assert!(error.to_string().contains("speaker"));
}

#[test]
fn the_mlx_audio_payload_of_a_base_checkpoint_carries_the_reference() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        references: vec![AudioReference {
            audio_path: Some("/tmp/reference.wav".to_string()),
            text: Some("参考音频的文本".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-Base-4bit");
    let payload = protocol.build_payload(&request, &ctx).unwrap();
    assert_eq!(payload["ref_audio"], "/tmp/reference.wav");
    assert_eq!(payload["ref_text"], "参考音频的文本");
    // The base checkpoint has no built-in speaker.
    assert!(payload.get("voice").is_none());
}

#[test]
fn a_base_checkpoint_without_a_reference_is_refused() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-Base-4bit");
    let error = protocol.build_payload(&request, &ctx).unwrap_err();
    assert!(error.to_string().contains("reference"));
}

#[test]
fn a_remote_reference_is_refused_by_the_mlx_audio_engine() {
    let protocol = Qwen3TTSProtocol;
    let request = qwen_request(EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
        references: vec![AudioReference {
            audio_path: Some("https://example.com/a.wav".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let ctx = mlx_audio_ctx("mlx-community/Qwen3-TTS-12Hz-1.7B-Base-4bit");
    let error = protocol.build_payload(&request, &ctx).unwrap_err();
    assert!(error.to_string().contains("local reference clip"));
}
