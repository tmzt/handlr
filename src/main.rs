use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use audio_core::cpal_capture::CpalCapture;
use audio_core::ensure_audio_ring;
use handlr::reasoning::{ReasoningEngine, EngineType, ReasoningEngineBuilder, Message, SessionManager, SessionBuilder};
use handlr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig, HerdrToolExecutor, HerdrContextContributor};
use libconversate::stt::{SttBackend, SttConfig, SttMode};
use libconversate::tts::{TtsConfig, TtsBackend};
use libconversate::s2s::SpeechToSpeechBuilder;

#[allow(dead_code)]
struct AppArgs {
    config: Option<String>,
    api_key: Option<String>,
    remote: Option<String>,
    socket: Option<String>,
    silero_model: Option<String>,
    whisper_model: Option<String>,
    tts_engine: Option<String>,
    kokoro_model: Option<String>,
    kokoro_voices: Option<String>,
    sherpa_model_dir: Option<String>,
    use_sine_tts: bool,
    tts_voice: Option<String>,
    tts_speed: Option<f32>,
    mock_mode: bool,
    auto_download: bool,
}

fn parse_args() -> AppArgs {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut api_key = None;
    let mut remote = None;
    let mut socket = None;
    let mut silero_model = None;
    let mut whisper_model = None;
    let mut tts_engine = None;
    let mut kokoro_model = None;
    let mut kokoro_voices = None;
    let mut sherpa_model_dir = None;
    let mut use_sine_tts = false;
    let mut tts_voice = None;
    let mut tts_speed = None;
    let mut mock_mode = false;
    let mut auto_download = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => config = args.next(),
            "--api-key" => api_key = args.next(),
            "--remote" => remote = args.next(),
            "--socket" => socket = args.next(),
            "--silero" => silero_model = args.next(),
            "--whisper" => whisper_model = args.next(),
            "--tts-engine" | "--tts-backend" => tts_engine = args.next(),
            "--kokoro-model" => kokoro_model = args.next(),
            "--kokoro-voices" => kokoro_voices = args.next(),
            "--sherpa-model-dir" => sherpa_model_dir = args.next(),
            "--sine-tts" => use_sine_tts = true,
            "--tts-voice" => tts_voice = args.next(),
            "--tts-speed" => {
                if let Some(s) = args.next() {
                    tts_speed = s.parse::<f32>().ok();
                }
            }
            "--mock" => mock_mode = true,
            "--auto-download" => auto_download = true,
            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    AppArgs {
        config,
        api_key,
        remote,
        socket,
        silero_model,
        whisper_model,
        tts_engine,
        kokoro_model,
        kokoro_voices,
        sherpa_model_dir,
        use_sine_tts,
        tts_voice,
        tts_speed,
        mock_mode,
        auto_download,
    }
}

fn download_model(repo_id: &str, repo_filename: &str, dest_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dest_dir = dest_path.parent().ok_or("No parent dir for dest path")?;
    std::fs::create_dir_all(dest_dir)?;

    let url = format!("https://huggingface.co/{repo_id}/resolve/main/{repo_filename}");
    log::info!("Downloading {} from {} via reqwest...", repo_filename, url);

    let client = reqwest::blocking::Client::builder()
        .user_agent("handlr/0.1.0")
        .build()?;
    let mut resp = client.get(&url).send()?.error_for_status()?;

    let temp_path = dest_path.with_extension("downloading");
    let mut file = std::fs::File::create(&temp_path)?;
    std::io::copy(&mut resp, &mut file)?;
    file.sync_all()?;
    std::fs::rename(temp_path, dest_path)?;

    log::info!("Successfully downloaded {}", dest_path.display());
    Ok(())
}

fn download_sherpa_bundle(dest_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dest_dir)?;

    let url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-en-v0_19.tar.bz2";
    log::info!("Downloading Sherpa Kokoro bundle from {} via reqwest...", url);

    let client = reqwest::blocking::Client::builder()
        .user_agent("handlr/0.1.0")
        .build()?;
    let resp = client.get(url).send()?.error_for_status()?;

    let decompressor = bzip2::read::BzDecoder::new(resp);
    let mut archive = tar::Archive::new(decompressor);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();

        // Strip top-level directory (e.g. `kokoro-en-v0_19/...`)
        let mut components = path.components();
        let _root = components.next();
        let sub_path = components.as_path();
        if sub_path.as_os_str().is_empty() {
            continue;
        }

        let out_path = dest_dir.join(sub_path);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&out_path)?;
        }
    }

    log::info!("Successfully unpacked Sherpa Kokoro bundle to {}", dest_dir.display());
    Ok(())
}

fn resolve_model_path(
    arg_path: Option<String>,
    default_filename: &str,
    repo_id: &str,
    repo_filename: &str,
    auto_download: bool,
    models_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = arg_path {
        return Ok(PathBuf::from(path));
    }

    let target_path = models_dir.join(default_filename);
    let exists = target_path.exists() || target_path.symlink_metadata().is_ok();

    if !exists {
        if auto_download {
            log::info!("Model {} not found. Auto-downloading to {}...", default_filename, target_path.display());
            download_model(repo_id, repo_filename, &target_path)?;
        } else {
            log::warn!(
                "Model {} not found in {} and --auto-download not specified.",
                default_filename,
                models_dir.display()
            );
        }
    } else {
        log::info!("Model {} found at {} (symlink or file)", default_filename, target_path.display());
    }

    Ok(target_path)
}

fn expand_tilde(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    if let Ok(stripped) = p.strip_prefix("~") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    p.to_path_buf()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args();
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));

    // 1. Initialize Reasoning Engine
    let system_prompt = handlr::reasoning::default_system_prompt()
        .ok_or_else(|| "could not find <system_prompt> tag in AGENTS.md".to_string())?;
    let tools = handlr::reasoning::declare_gemini_tools();

    let api_key = args
        .api_key
        .or(config.gemini.api_key)
        .or_else(|| std::env::var("GEMINI_API_KEY").ok());

    let engine_type = if args.mock_mode {
        log::info!("Starting in MOCK reasoning mode");
        EngineType::Mock {
            default_response: Some("This is a mock response from handlr.".to_string()),
        }
    } else {
        log::info!("Initializing Gemini Reasoning Engine...");
        EngineType::Gemini {
            api_key,
            tools,
        }
    };

    let reasoning_engine = ReasoningEngineBuilder::new(engine_type)
        .with_system_prompt(system_prompt)
        .build()?;

    // 2. Initialize Transport / Herdr Client
    let transport = if args.mock_mode {
        log::info!("Using Mock Transport");
        HerdrTransport::mock()
    } else if let Some(ref ssh_target) = args.remote {
        log::info!("Connecting via SSH to: {}", ssh_target);
        let ssh_config = RemoteSshConfig::new(ssh_target.clone());
        HerdrTransport::remote_ssh_with_config(ssh_config)
    } else if let Some(ref socket_str) = args.socket {
        let socket_path = expand_tilde(socket_str);
        log::info!("Connecting via UNIX domain socket: {:?}", socket_path);
        HerdrTransport::local(socket_path)
    } else if config.transport.mode.eq_ignore_ascii_case("remote") {
        if let Some(ref remote_target) = config.transport.remote {
            log::info!("Connecting via SSH (from config) to: {}", remote_target);
            let ssh_config = RemoteSshConfig::new(remote_target.clone());
            HerdrTransport::remote_ssh_with_config(ssh_config)
        } else {
            let socket_path = handlr::transport::default_local_socket_path();
            log::warn!("Config transport mode is 'remote' but 'remote' string is missing; falling back to local socket: {:?}", socket_path);
            HerdrTransport::local(socket_path)
        }
    } else {
        let socket_path = config
            .transport
            .socket
            .map(expand_tilde)
            .unwrap_or_else(handlr::transport::default_local_socket_path);
        log::info!("Connecting via UNIX domain socket: {:?}", socket_path);
        HerdrTransport::local(socket_path)
    };

    let herdr_client = Arc::new(HerdrClient::new(transport));

    // 3. Resolve model paths (supporting symlinks in ~/.handlr/models)
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/tmeade".to_string());
    let models_dir = PathBuf::from(home).join(".handlr").join("models");
    std::fs::create_dir_all(&models_dir)?;

    let silero_model_arg = args.silero_model.or(config.voice.stt.silero_model);
    let whisper_model_arg = args.whisper_model.or(config.voice.stt.whisper_model);

    let silero_path = resolve_model_path(
        silero_model_arg.clone(),
        "silero_vad.onnx",
        "onnx-community/silero-vad",
        "onnx/model.onnx",
        args.auto_download,
        &models_dir,
    )?;

    let whisper_path = resolve_model_path(
        whisper_model_arg.clone(),
        "ggml-base.en.bin",
        "ggerganov/whisper.cpp",
        "ggml-base.en.bin",
        args.auto_download,
        &models_dir,
    )?;

    let mut builder = SpeechToSpeechBuilder::default();

    if silero_model_arg.is_some() || whisper_model_arg.is_some() {
        let stt_backend = SttBackend::Whisper {
            model: whisper_path,
        };
        let stt_config = SttConfig::new(silero_path, stt_backend).with_input_pull(true);
        builder = builder.with_stt_config(stt_config);
    }

    let tts_engine_choice = args
        .tts_engine
        .or_else(|| if args.use_sine_tts { Some("sine".to_string()) } else { None })
        .unwrap_or(config.voice.tts.engine);

    let backend = match tts_engine_choice.to_lowercase().as_str() {
        "sine" => {
            log::info!("TTS backend: Sine test tone");
            TtsBackend::Sine
        }
        "system" => {
            log::info!("TTS backend: System TTS");
            TtsBackend::System
        }
        #[cfg(feature = "kokoro")]
        "kokoro" => {
            let model_path = resolve_model_path(
                args.kokoro_model.or(config.voice.tts.kokoro_model),
                "kokoro-v1.0.onnx",
                "hexgrad/Kokoro-82M",
                "kokoro-v1.0.onnx",
                args.auto_download,
                &models_dir,
            )?;
            let voices_path = resolve_model_path(
                args.kokoro_voices.or(config.voice.tts.kokoro_voices),
                "voices.bin",
                "hexgrad/Kokoro-82M",
                "voices.bin",
                args.auto_download,
                &models_dir,
            )?;
            log::info!(
                "TTS backend: Kokoro (model={}, voices={})",
                model_path.display(),
                voices_path.display()
            );
            TtsBackend::Kokoro {
                model: model_path,
                voices: voices_path,
            }
        }
        #[cfg(not(feature = "kokoro"))]
        "kokoro" => {
            return Err("Kokoro TTS requested, but handlr was built without '--features kokoro'. Recompile with --features kokoro.".into());
        }
        #[cfg(feature = "sherpa-kokoro")]
        "sherpa-kokoro" | "sherpa" => {
            let model_dir = if let Some(dir) = args.sherpa_model_dir.or(config.voice.tts.sherpa_model_dir) {
                PathBuf::from(dir)
            } else {
                models_dir.join("sherpa-kokoro")
            };
            if !model_dir.exists() || !model_dir.join("model.onnx").exists() {
                if args.auto_download {
                    log::info!("Sherpa Kokoro bundle not found. Auto-downloading to {}...", model_dir.display());
                    download_sherpa_bundle(&model_dir)?;
                } else {
                    log::warn!(
                        "Sherpa Kokoro assets not found at {} and --auto-download not specified.",
                        model_dir.display()
                    );
                }
            } else {
                log::info!("Sherpa Kokoro bundle found at {}", model_dir.display());
            }
            log::info!(
                "TTS backend: Sherpa Kokoro (model_dir={})",
                model_dir.display()
            );
            TtsBackend::SherpaKokoro {
                model_dir,
            }
        }
        #[cfg(not(feature = "sherpa-kokoro"))]
        "sherpa-kokoro" | "sherpa" => {
            return Err("Sherpa Kokoro TTS requested, but handlr was built without '--features sherpa-kokoro'. Recompile with --features sherpa-kokoro.".into());
        }
        unknown => {
            return Err(format!("Unknown TTS engine: '{}'. Valid options: system, sine, kokoro, sherpa-kokoro", unknown).into());
        }
    };

    let mut tts_config = TtsConfig::new(backend);
    if let Some(voice) = args.tts_voice.or(config.voice.tts.voice) {
        tts_config.voice = voice;
    }
    let speed = args.tts_speed.unwrap_or(config.voice.tts.speed);
    tts_config.speed = speed;
    log::info!("TTS configured with voice='{}' speed={}", tts_config.voice, tts_config.speed);
    builder = builder.with_tts_config(tts_config);
    let mute_atomic = Arc::new(std::sync::atomic::AtomicBool::new(false));
    builder = builder.with_mute_flag(Arc::clone(&mute_atomic));

    // Initialize Session Manager
    let session_manager = SessionManager::new().map_err(|e| format!("session manager error: {e}"))?;
    let session_id = "default_session".to_string();

    // 4. Build the Speech-to-Speech Pipeline
    log::info!("Building Speech-to-Speech Pipeline...");
    let mute_for_proc = Arc::clone(&mute_atomic);
    let pipeline = builder
        .with_async_processor(move |transcript| {
            let client = Arc::clone(&herdr_client);
            let engine = Arc::clone(&reasoning_engine);
            let manager = session_manager.clone();
            let sid = session_id.clone();
            let proc_mute = Arc::clone(&mute_for_proc);
            Box::pin(async move {
                // Instantly mute mic while reasoning & generating response
                proc_mute.store(true, std::sync::atomic::Ordering::SeqCst);
                log::info!("Received transcript: '{}'", transcript);

                // Append the user turn to session history
                let user_msg = Message {
                    role: "user".to_string(),
                    content: transcript.to_string(),
                    name: None,
                    thought_signature: None,
                };
                if let Err(e) = manager.append_message(&sid, &user_msg) {
                    log::error!("failed to save user turn to session: {}", e);
                }

                // Compile history using contributors (RAG layout/agent snapshot context injection)
                let builder = SessionBuilder::new(sid.clone(), manager.clone())
                    .with_contributor(Box::new(HerdrContextContributor::new((*client).clone())));

                let messages = match builder.build().await {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        log::error!("failed to compile session messages: {}", e);
                        // fallback to a single user message if builder fails
                        vec![user_msg]
                    }
                };

                let executor = HerdrToolExecutor((*client).clone());
                match engine.process_turns(&messages, &executor).await {
                    Ok(response) => {
                        if let Some(ref reply) = response {
                            log::info!("Engine response: '{}'", reply);
                            // Append model turn to session history
                            let model_msg = Message {
                                role: "model".to_string(),
                                content: reply.clone(),
                                name: None,
                                thought_signature: None,
                            };
                            if let Err(e) = manager.append_message(&sid, &model_msg) {
                                log::error!("failed to save model turn to session: {}", e);
                            }
                        }
                        response
                    }
                    Err(e) => {
                        log::error!("Processing error: {}", e);
                        Some("Error processing request.".to_string())
                    }
                }
            })
        })
        .build()?;

    // Set STT to Live (continuous capture) mode and start recording
    pipeline.stt().set_mode(SttMode::Live);
    pipeline.stt().set_recording(true);

    // 5. Start audio capture stream (with atomic hardware muting)
    log::info!("Starting CPAL microphone audio capture...");
    let mute_for_capture = Arc::clone(&mute_atomic);
    let _capture = CpalCapture::start_with_processor(move |samples| {
        if mute_for_capture.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        ensure_audio_ring().push_samples(samples);
    }).map_err(|e| format!("Capture error: {}", e))?;

    log::info!("handlr is running. Speak into your microphone...");

    // Main thread event loop
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}
