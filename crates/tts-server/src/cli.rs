//! Command line interface of the TTS subsystem.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use tts_core::paths::AppPaths;
use tts_core::service::TtsService;
use tts_core::types::{
    AudioFormat, EngineSpecificParameters, FishDrama3Parameters, MockParameters,
    Qwen3TtsParameters, Qwen3TtsTaskType, TTSRequest,
};
use tts_core::voice::{NewVoiceProfile, VoiceFilter};

/// Command line interface of the TTS subsystem.
#[derive(Debug, Parser)]
#[command(
    name = "tts-server",
    version,
    about = "Local TTS subsystem: model runtime, voice library and HTTP API"
)]
pub struct Cli {
    /// Directory that holds models, voices, audio and logs.
    #[arg(long, global = true, env = "LOCAL_AI_MANAGER_HOME")]
    pub data_dir: Option<PathBuf>,

    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Available commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Starts the HTTP API server.
    Serve(ServeArgs),
    /// Manages the TTS models.
    Models {
        /// The model command.
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Manages the voice library.
    Voices {
        /// The voice command.
        #[command(subcommand)]
        command: VoiceCommand,
    },
    /// Manages the inference engines.
    Engines {
        /// The engine command.
        #[command(subcommand)]
        command: EngineCommand,
    },
    /// Synthesises speech.
    Speak(SpeakArgs),
    /// Asks a language model a question.
    Chat(ChatArgs),
/// Plays an audio file with the platform player.
    Play(PlayArgs),
    /// Shows the state of the subsystem.
    Status,
    /// Shows the hardware and the compatibility of every model.
    Doctor,
}

/// Arguments of the `chat` command.
#[derive(Debug, Args)]
pub struct ChatArgs {
    /// The language model that answers.
    #[arg(long)]
    pub model: String,
    /// The question.
    #[arg(long, short)]
    pub prompt: String,
    /// The system prompt of the conversation.
    #[arg(long)]
    pub system: Option<String>,
    /// The number of tokens the answer may use.
    #[arg(long, default_value_t = 1024)]
    pub max_tokens: u32,
}

/// Arguments of the `serve` command.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Interface to bind to.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Port to bind to.
    #[arg(long, default_value_t = 54321)]
    pub port: u16,
}

/// Arguments of the `play` command.
#[derive(Debug, Args)]
pub struct PlayArgs {
    /// File to play. Relative paths are resolved inside the generated audio
    /// directory.
    pub file: String,
}

/// Model commands.
#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Lists the registered models.
    List,
    /// Shows one model.
    Show {
        /// Model id or alias.
        id: String,
    },
    /// Loads a model.
    Load {
        /// Model id or alias.
        id: String,
    },
    /// Unloads a model.
    Unload {
        /// Model id or alias.
        id: String,
    },
    /// Pins a model so it is never unloaded automatically.
    Pin {
        /// Model id or alias.
        id: String,
    },
    /// Removes the pin of a model.
    Unpin {
        /// Model id or alias.
        id: String,
    },
    /// Prints the engine log of a model.
    Logs {
        /// Model id or alias.
        id: String,
    },
    /// Downloads the checkpoint of a model.
    Install {
        /// Model id or alias.
        id: String,
    },
    /// Removes the checkpoint of a model from the disk.
    Uninstall {
        /// Model id or alias.
        id: String,
    },
}

/// Voice commands.
#[derive(Debug, Subcommand)]
pub enum VoiceCommand {
    /// Lists the stored voices.
    List {
        /// Only voices of this model.
        #[arg(long)]
        model: Option<String>,
        /// Only voices of this language.
        #[arg(long)]
        language: Option<String>,
        /// Free text search.
        #[arg(long)]
        query: Option<String>,
    },
    /// Creates a voice from a natural language description.
    Design {
        /// Model that creates the voice.
        #[arg(long)]
        model: String,
        /// Description of the wanted voice.
        #[arg(long)]
        prompt: String,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// Language of the voice.
        #[arg(long)]
        language: Option<String>,
        /// Text of the preview sample.
        #[arg(long)]
        sample_text: Option<String>,
    },
    /// Creates a voice from a reference clip.
    Clone {
        /// Model that clones the voice.
        #[arg(long)]
        model: String,
        /// Reference clip.
        #[arg(long)]
        reference: String,
        /// Transcript of the reference clip.
        #[arg(long)]
        reference_text: Option<String>,
        /// Display name.
        #[arg(long)]
        name: Option<String>,
        /// Language of the voice.
        #[arg(long)]
        language: Option<String>,
        /// Text of the preview sample.
        #[arg(long)]
        sample_text: Option<String>,
    },
    /// Deletes a voice.
    Remove {
        /// Voice id.
        id: String,
    },
}

/// Arguments of the `speak` command.
#[derive(Debug, Args)]
pub struct SpeakArgs {
    /// Model id or alias.
    #[arg(long)]
    pub model: String,
    /// The text to speak.
    #[arg(long)]
    pub text: String,
    /// Voice profile id.
    #[arg(long)]
    pub voice: Option<String>,
    /// Language of the text.
    #[arg(long)]
    pub language: Option<String>,
    /// Output format.
    #[arg(long, default_value = "wav")]
    pub format: String,
    /// File the audio is written to.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

/// Runs the command line interface.
pub async fn run(cli: Cli) -> Result<()> {
    let paths = match &cli.data_dir {
        Some(dir) => AppPaths::with_root(dir.clone()),
        None => AppPaths::discover(),
    };
    let service =
        TtsService::with_paths(paths).context("failed to initialise the TTS subsystem")?;

    let result = match cli.command {
        Command::Serve(args) => serve(service.clone(), args).await,
        Command::Models { command } => models(service.clone(), command).await,
        Command::Voices { command } => voices(service.clone(), command).await,
        Command::Engines { command } => engines(service.clone(), command).await,
        Command::Speak(args) => speak(service.clone(), args).await,
        Command::Chat(args) => chat(service.clone(), args).await,
        Command::Play(args) => play(service.clone(), args),
        Command::Status => status(service.clone()),
        Command::Doctor => doctor(service.clone()),
    };
    service.shutdown().await;
    result
}

/// Starts the HTTP API server.
async fn serve(service: std::sync::Arc<TtsService>, args: ServeArgs) -> Result<()> {
    let app = crate::api::router(service.clone(), &args.host);
    let address = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    tracing::info!(%address, "TTS API listening");
    println!("TTS API listening on http://{address}");
    // The desktop application asks the server to follow its life cycle.
    let watch_stdin = std::env::var("TTS_SERVER_EXIT_ON_STDIN_EOF")
        .map(|value| value == "1" || value == "true")
        .unwrap_or(false);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if watch_stdin {
                tokio::select! {
                    _ = shutdown_signal() => {}
                    _ = stdin_closed() => {}
                }
            } else {
                shutdown_signal().await;
            }
        })
        .await
        .context("the HTTP server failed")?;
    Ok(())
}

/// Waits for Ctrl-C.
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutting down");
}

/// Waits until the standard input reaches its end.
///
/// The desktop application holds the write end of the pipe; when it dies the
/// server sees the end and stops, so no backend is left behind.
async fn stdin_closed() {
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut buffer = Vec::new();
    let _ = stdin.read_to_end(&mut buffer).await;
    tracing::info!("the parent process closed the connection");
}

/// Engine commands.
#[derive(Debug, Subcommand)]
pub enum EngineCommand {
    /// Lists the engines and their installation state.
    List,
    /// Shows the versions of an engine that can be installed.
    Versions {
        /// The engine id.
        id: String,
    },
    /// Installs a version of an engine.
    Install {
        /// The engine id.
        id: String,
        /// The version to install. The newest one is used when it is missing.
        version: Option<String>,
    },
    /// Activates an installed version of an engine.
    Use {
        /// The engine id.
        id: String,
        /// The version to activate.
        version: String,
    },
    /// Removes an installed version of an engine.
    Uninstall {
        /// The engine id.
        id: String,
        /// The version to remove.
        version: String,
    },
}

/// Engine commands.
async fn engines(service: std::sync::Arc<TtsService>, command: EngineCommand) -> Result<()> {
    match command {
        EngineCommand::List => {
            println!(
                "{:<14} {:<12} {:<12} {:>10} {:>8}  {}",
                "ENGINE", "STATUS", "VERSION", "SIZE", "VERS", "SOURCE"
            );
            for engine in service.engines.list() {
                println!(
                    "{:<14} {:<12} {:<12} {:>9.1}M {:>8}  {}",
                    engine.id,
                    engine.status,
                    engine.current_version.clone().unwrap_or_else(|| "-".to_string()),
                    engine.size_bytes as f64 / 1024.0 / 1024.0,
                    engine.installed.len(),
                    engine
                        .repository
                        .clone()
                        .or_else(|| engine.package.clone())
                        .unwrap_or_else(|| "-".to_string()),
                );
            }
            Ok(())
        }
        EngineCommand::Versions { id } => {
            let versions = service.engines.versions(&id).await?;
            println!("{:<16} {:<12} {:>10}  {}", "VERSION", "KIND", "SIZE", "PUBLISHED");
            for version in versions {
                println!(
                    "{:<16} {:<12} {:>9.1}M  {}",
                    version.version,
                    if version.prerelease { "pre" } else { "stable" },
                    version.size_bytes.unwrap_or(0) as f64 / 1024.0 / 1024.0,
                    version.published_at.clone().unwrap_or_else(|| "-".to_string()),
                );
            }
            Ok(())
        }
        EngineCommand::Install { id, version } => {
            service.engines.start_install(&id, version)?;
            println!("installing {id}…");
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                let Some(progress) = service.engines.install_progress(&id) else {
                    continue;
                };
                match progress.percent {
                    Some(percent) => println!(
                        "  {} {:.0}% — {}",
                        progress.state, percent, progress.message
                    ),
                    None => println!("  {} — {}", progress.state, progress.message),
                }
                if progress.state == "done" {
                    println!("installed {}", progress.version);
                    return Ok(());
                }
                if progress.state == "failed" {
                    anyhow::bail!("the installation failed: {}", progress.message);
                }
            }
        }
        EngineCommand::Use { id, version } => {
            service.engines.activate(&id, &version)?;
            println!("{id} now uses {version}");
            Ok(())
        }
        EngineCommand::Uninstall { id, version } => {
            service.engines.uninstall(&id, &version)?;
            println!("removed {id} {version}");
            Ok(())
        }
    }
}

/// Model commands.
async fn models(service: std::sync::Arc<TtsService>, command: ModelCommand) -> Result<()> {
    match command {
        ModelCommand::List => {
            let runtimes = service.manager.runtimes();
            println!(
                "{:<32} {:<12} {:<10} {:>6} {:>6} {:>12} {:>10}",
                "MODEL", "FAMILY", "STATE", "SIZE", "MEM", "COMPAT", "INSTALLED"
            );
            for spec in service.registry.list() {
                let report = service.manager.compatibility(&spec);
                let state = runtimes
                    .iter()
                    .find(|runtime| runtime.model_id == spec.id)
                    .map(|runtime| runtime.state.as_str().to_string())
                    .unwrap_or_else(|| "unloaded".to_string());
                println!(
                    "{:<32} {:<12} {:<10} {:>5.1}G {:>5.1}G {:>12} {:>10}",
                    spec.id,
                    spec.family.as_str(),
                    state,
                    spec.size_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                    spec.estimated_memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                    report.level.as_str(),
                    if service.registry.is_installed(&spec.id) {
                        "yes"
                    } else {
                        "no"
                    },
                );
            }
            Ok(())
        }
        ModelCommand::Show { id } => {
            let spec = service
                .registry
                .resolve(&id)
                .with_context(|| format!("unknown model `{id}`"))?;
            let report = service.manager.compatibility(&spec);
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "id": spec.id,
                    "name": spec.name,
                    "family": spec.family.as_str(),
                    "engine": spec.engine_id,
                    "repository": spec.hf_repo,
                    "version": spec.version,
                    "format": spec.format,
                    "size_bytes": spec.size_bytes,
                    "estimated_memory_bytes": spec.estimated_memory_bytes,
                    "features": spec.features.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
                    "compatibility": report.level.as_str(),
                    "notes": report.notes,
                    "installed": service.registry.is_installed(&spec.id),
                    "path": service.registry.install_path(&spec.id),
                }))?
            );
            Ok(())
        }
        ModelCommand::Load { id } => {
            let runtime = service.manager.load_model(&id).await?;
            println!(
                "loaded {} (state={}, endpoint={:?})",
                runtime.model_id,
                runtime.state.as_str(),
                runtime.endpoint
            );
            Ok(())
        }
        ModelCommand::Unload { id } => {
            service.manager.unload_model(&id).await?;
            println!("unloaded {id}");
            Ok(())
        }
        ModelCommand::Pin { id } => {
            service.manager.set_pinned(&id, true)?;
            println!("pinned {id}");
            Ok(())
        }
        ModelCommand::Unpin { id } => {
            service.manager.set_pinned(&id, false)?;
            println!("unpinned {id}");
            Ok(())
        }
        ModelCommand::Install { id } => install(service.clone(), &id).await,
        ModelCommand::Uninstall { id } => uninstall(service.clone(), &id),
        ModelCommand::Logs { id } => {
            let logs = service.manager.logs(&id).await?;
            print!("{logs}");
            Ok(())
        }
    }
}

/// Downloads the checkpoint of a model.
///
/// The hardware compatibility is checked before the download starts, as the
/// specification demands.
async fn install(service: std::sync::Arc<TtsService>, id: &str) -> Result<()> {
    let spec = service
        .registry
        .resolve(id)
        .with_context(|| format!("unknown model `{id}`"))?;

    let report = service.manager.compatibility(&spec);
    if !report.compatible {
        anyhow::bail!(
            "`{}` cannot run on this machine: {}",
            spec.id,
            report.notes.join("; ")
        );
    }

    let target = service.registry.install_path(&spec.id).unwrap_or_default();
    println!("installing {} into {}", spec.id, target.display());

    service.installs.start(&spec.id);
    let result = service
        .installer
        .install(&spec, Some(&service.installs))
        .await;
    service
        .installs
        .finish(&spec.id, result.as_ref().err().map(|err| err.to_string()));

    let report = result?;
    println!(
        "installed {} — {} files ({} downloaded, {} skipped), {:.1} GiB in {}",
        spec.id,
        report.downloaded.len() + report.skipped.len(),
        report.downloaded.len(),
        report.skipped.len(),
        report.bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        report.path.display()
    );
    Ok(())
}

/// Removes the checkpoint of a model from the disk.
fn uninstall(service: std::sync::Arc<TtsService>, id: &str) -> Result<()> {
    let spec = service
        .registry
        .resolve(id)
        .with_context(|| format!("unknown model `{id}`"))?;
    service.installer.uninstall(&spec)?;
    println!("removed the checkpoint of {}", spec.id);
    Ok(())
}

/// Voice commands.
async fn voices(service: std::sync::Arc<TtsService>, command: VoiceCommand) -> Result<()> {
    match command {
        VoiceCommand::List {
            model,
            language,
            query,
        } => {
            let voices = service
                .library
                .list(VoiceFilter {
                    model_id: model,
                    engine: None,
                    language,
                    query,
                })
                .await?;
            println!(
                "{:<14} {:<24} {:<14} {:<24}",
                "ID", "NAME", "MODEL", "SAMPLES"
            );
            for voice in voices {
                println!(
                    "{:<14} {:<24} {:<14} {:<24}",
                    voice.id,
                    voice.name,
                    voice.model_id,
                    voice.samples.len()
                );
            }
            Ok(())
        }
        VoiceCommand::Design {
            model,
            prompt,
            name,
            language,
            sample_text,
        } => {
            let spec = service
                .registry
                .resolve(&model)
                .with_context(|| format!("unknown model `{model}`"))?;
            let mut engine_config = serde_json::Map::new();
            engine_config.insert(
                "voice_design_prompt".to_string(),
                serde_json::Value::String(prompt.clone()),
            );
            let voice = service
                .library
                .create(NewVoiceProfile {
                    name: name.clone(),
                    description: Some(prompt.clone()),
                    language: language.clone(),
                    engine: Some(spec.family.as_str().to_string()),
                    model_id: Some(spec.id.clone()),
                    engine_config: serde_json::Value::Object(engine_config),
                    ..Default::default()
                })
                .await?;
            println!("created voice {} ({})", voice.id, voice.name);
            render_preview(&service, &spec, &voice, sample_text.as_deref()).await?;
            Ok(())
        }
        VoiceCommand::Clone {
            model,
            reference,
            reference_text,
            name,
            language,
            sample_text,
        } => {
            let spec = service
                .registry
                .resolve(&model)
                .with_context(|| format!("unknown model `{model}`"))?;
            let mut engine_config = serde_json::Map::new();
            engine_config.insert(
                "reference_audio".to_string(),
                serde_json::Value::String(reference.clone()),
            );
            if let Some(text) = &reference_text {
                engine_config.insert(
                    "reference_text".to_string(),
                    serde_json::Value::String(text.clone()),
                );
            }
            let voice = service
                .library
                .create(NewVoiceProfile {
                    name: name.clone(),
                    description: Some(format!("cloned from {reference}")),
                    language: language.clone(),
                    engine: Some(spec.family.as_str().to_string()),
                    model_id: Some(spec.id.clone()),
                    engine_config: serde_json::Value::Object(engine_config),
                    ..Default::default()
                })
                .await?;
            println!("created voice {} ({})", voice.id, voice.name);
            render_preview(&service, &spec, &voice, sample_text.as_deref()).await?;
            Ok(())
        }
        VoiceCommand::Remove { id } => {
            service.library.delete(&id).await?;
            println!("removed voice {id}");
            Ok(())
        }
    }
}

/// Renders and stores a preview sample of a new voice.
async fn render_preview(
    service: &std::sync::Arc<TtsService>,
    spec: &tts_core::registry::TTSModelSpec,
    voice: &tts_core::voice::VoiceProfile,
    sample_text: Option<&str>,
) -> Result<()> {
    let text = sample_text.unwrap_or("你好，这是这个声音的试听样本。");
    let parameters = match spec.family {
        tts_core::types::ModelFamily::Qwen3Tts => {
            EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters {
                task_type: Some(Qwen3TtsTaskType::VoiceDesign),
                instructions: voice
                    .engine_config
                    .get("voice_design_prompt")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                ..Default::default()
            })
        }
        tts_core::types::ModelFamily::FishDrama3 => {
            EngineSpecificParameters::FishDrama3(FishDrama3Parameters::default())
        }
        tts_core::types::ModelFamily::Mock => {
            EngineSpecificParameters::Mock(MockParameters::default())
        }
        tts_core::types::ModelFamily::Llm => {
            return Err(anyhow::anyhow!("`{}` is a language model", spec.id))
        }
    };
    let request = TTSRequest {
        model_id: spec.id.clone(),
        voice_id: Some(voice.id.clone()),
        text: text.to_string(),
        language: voice.language.clone(),
        parameters,
        response_format: AudioFormat::Wav,
        speed: None,
    };
    let result = service.manager.synthesize(request).await?;
    let sample = service
        .library
        .add_sample(&voice.id, &result.output, None)
        .await?;
    println!("sample: {} ({:?})", sample.name, sample.duration_ms);
    if let Some(path) = &result.file_path {
        println!("audio: {path}");
    }
    Ok(())
}

/// Synthesises speech from the command line.
async fn speak(service: std::sync::Arc<TtsService>, args: SpeakArgs) -> Result<()> {
    let spec = service
        .registry
        .resolve(&args.model)
        .with_context(|| format!("unknown model `{}`", args.model))?;
    let format = AudioFormat::parse(&args.format).unwrap_or(AudioFormat::Wav);
    let parameters = match spec.family {
        tts_core::types::ModelFamily::Qwen3Tts => {
            EngineSpecificParameters::Qwen3Tts(Qwen3TtsParameters::default())
        }
        tts_core::types::ModelFamily::FishDrama3 => {
            EngineSpecificParameters::FishDrama3(FishDrama3Parameters::default())
        }
        tts_core::types::ModelFamily::Mock => {
            EngineSpecificParameters::Mock(MockParameters::default())
        }
        tts_core::types::ModelFamily::Llm => {
            return Err(anyhow::anyhow!("`{}` is a language model", spec.id))
        }
    };
    let request = TTSRequest {
        model_id: spec.id.clone(),
        voice_id: args.voice.clone(),
        text: args.text.clone(),
        language: args.language.clone(),
        parameters,
        response_format: format,
        speed: None,
    };
    let result = service.manager.synthesize(request).await?;
    match &args.out {
        Some(path) => {
            std::fs::write(path, &result.output.data)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("wrote {}", path.display());
        }
        None => {
            if let Some(path) = &result.file_path {
                println!("wrote {path}");
            } else {
                std::io::Write::write_all(&mut std::io::stdout(), &result.output.data)?;
            }
        }
    }
    Ok(())
}

/// Asks a language model a question.
async fn chat(service: std::sync::Arc<TtsService>, args: ChatArgs) -> Result<()> {
    let spec = service
        .registry
        .resolve(&args.model)
        .with_context(|| format!("unknown model `{}`", args.model))?;
    if spec.family != tts_core::types::ModelFamily::Llm {
        return Err(anyhow::anyhow!(
            "`{}` is not a language model, use `speak` for the speech models",
            spec.id
        ));
    }

    let mut messages = Vec::new();
    if let Some(system) = args.system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    messages.push(serde_json::json!({"role": "user", "content": args.prompt}));

    let body = serde_json::json!({
        "model": spec.id,
        "messages": messages,
        "max_tokens": args.max_tokens,
    });
    let answer = service.llm.chat(&spec.id, body).await?;
    let content = answer
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let reasoning = answer
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("reasoning_content"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if !reasoning.is_empty() {
        println!("{reasoning}");
    }
    if content.trim().is_empty() {
        // A thinking model spends the budget on its thoughts, the answer then
        // needs a larger `--max-tokens`.
        println!("（模型仍在思考，用 --max-tokens 给它更多预算）");
    } else {
        println!("{content}");
    }
    Ok(())
}

/// Plays an audio file.
fn play(service: std::sync::Arc<TtsService>, args: PlayArgs) -> Result<()> {
    let candidate = std::path::Path::new(&args.file);
    let path = if candidate.exists() {
        candidate.to_path_buf()
    } else {
        service.audio.generated_root().join(&args.file)
    };
    service.audio.play(&path)?;
    println!("playing {}", path.display());
    Ok(())
}

/// Prints the status of the subsystem.
fn status(service: std::sync::Arc<TtsService>) -> Result<()> {
    let status = service.manager.status();
    println!(
        "active model: {}",
        status.active_model.as_deref().unwrap_or("-")
    );
    println!(
        "memory: {:.1} GiB used of {:.1} GiB, {:.1} GiB available",
        status.scheduler.used_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        status.scheduler.total_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
        status.scheduler.available_bytes as f64 / 1024.0 / 1024.0 / 1024.0,
    );
    println!(
        "\n{:<32} {:<10} {:>8} {:>8}",
        "MODEL", "STATE", "REQUESTS", "PINNED"
    );
    for runtime in &status.runtimes {
        println!(
            "{:<32} {:<10} {:>8} {:>8}",
            runtime.model_id,
            runtime.state.as_str(),
            runtime.active_requests,
            runtime.pinned
        );
    }
    Ok(())
}

/// Prints the hardware and the compatibility of every model.
fn doctor(service: std::sync::Arc<TtsService>) -> Result<()> {
    let hardware = &service.manager.hardware();
    println!("platform:      {}", hardware.platform);
    println!("architecture:  {}", hardware.architecture);
    println!("apple silicon: {}", hardware.apple_silicon);
    println!(
        "cuda:          {} {}",
        hardware.cuda_available,
        hardware.nvidia_gpu.clone().unwrap_or_default()
    );
    println!(
        "memory:        {:.1} GiB",
        hardware.total_ram_bytes as f64 / 1024.0 / 1024.0 / 1024.0
    );
    println!();
    for spec in service.registry.list() {
        let report = service.manager.compatibility(&spec);
        println!("{} [{}]", spec.id, report.level.as_str());
        for note in report.notes {
            println!("  - {note}");
        }
    }
    Ok(())
}
