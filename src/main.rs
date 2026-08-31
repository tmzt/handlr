use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use audio_core::cpal_capture::CpalCapture;
use audio_core::ensure_audio_ring;
use handlr::reasoning::{ReasoningEngine, EngineType, ReasoningEngineBuilder};
use handlr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig, HerdrToolExecutor};
use libconversate::stt::{SttBackend, SttConfig, SttMode};
use libconversate::tts::{TtsConfig, TtsBackend};
use libconversate::s2s::SpeechToSpeechBuilder;

struct AppArgs {
    api_key: Option<String>,
    remote: Option<String>,
    socket: Option<String>,
    silero_model: Option<String>,
    whisper_model: Option<String>,
    use_sine_tts: bool,
    mock_mode: bool,
    auto_download: bool,
}

fn parse_args() -> AppArgs {
    let mut args = std::env::args().skip(1);
    let mut api_key = std::env::var("GEMINI_API_KEY").ok();
    let mut remote = None;
    let mut socket = None;
    let mut silero_model = None;
    let mut whisper_model = None;
    let mut use_sine_tts = false;
    let mut mock_mode = false;
    let mut auto_download = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--api-key" => api_key = args.next(),
            "--remote" => remote = args.next(),
            "--socket" => socket = args.next(),
            "--silero" => silero_model = args.next(),
            "--whisper" => whisper_model = args.next(),
            "--sine-tts" => use_sine_tts = true,
            "--mock" => mock_mode = true,
            "--auto-download" => auto_download = true,
            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    AppArgs {
        api_key,
        remote,
        socket,
        silero_model,
        whisper_model,
        use_sine_tts,
        mock_mode,
        auto_download,
    }
}

fn download_model(repo_id: &str, repo_filename: &str, dest_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dest_dir = dest_path.parent().ok_or("No parent dir for dest path")?;
    std::fs::create_dir_all(dest_dir)?;

    let python_code = format!(
        "from huggingface_hub import hf_hub_download; import shutil, os; \
         path = hf_hub_download(repo_id='{}', filename='{}'); \
         shutil.copy(path, '{}')",
        repo_id,
        repo_filename,
        dest_path.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'")
    );

    log::info!("Running python downloader for {}...", repo_filename);
    let status = std::process::Command::new("python3")
        .arg("-c")
        .arg(&python_code)
        .status()?;

    if !status.success() {
        return Err(format!("Python download process exited with error status: {}", status).into());
    }

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args();

    // 1. Initialize Reasoning Engine
    let system_prompt = handlr::reasoning::default_system_prompt()
        .ok_or_else(|| "could not find <system_prompt> tag in AGENTS.md".to_string())?;
    let tools = handlr::reasoning::declare_gemini_tools();

    let engine_type = if args.mock_mode {
        log::info!("Starting in MOCK reasoning mode");
        EngineType::Mock {
            default_response: Some("This is a mock response from handlr.".to_string()),
        }
    } else {
        log::info!("Initializing Gemini Reasoning Engine...");
        EngineType::Gemini {
            api_key: args.api_key.clone(),
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
        let config = RemoteSshConfig::new(ssh_target.clone());
        HerdrTransport::remote_ssh_with_config(config)
    } else {
        let socket_path = args
            .socket
            .clone()
            .unwrap_or_else(|| "/tmp/herdr.sock".to_string());
        log::info!("Connecting via UNIX domain socket: {}", socket_path);
        HerdrTransport::local(socket_path)
    };

    let herdr_client = Arc::new(HerdrClient::new(transport));

    // 3. Resolve model paths (supporting symlinks in ~/.handlr/models)
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/tmeade".to_string());
    let models_dir = PathBuf::from(home).join(".handlr").join("models");
    std::fs::create_dir_all(&models_dir)?;

    let silero_path = resolve_model_path(
        args.silero_model.clone(),
        "silero_vad.onnx",
        "onnx-community/silero-vad",
        "onnx/model.onnx",
        args.auto_download,
        &models_dir,
    )?;

    let whisper_path = resolve_model_path(
        args.whisper_model.clone(),
        "ggml-base.en.bin",
        "ggerganov/whisper.cpp",
        "ggml-base.en.bin",
        args.auto_download,
        &models_dir,
    )?;

    let mut builder = SpeechToSpeechBuilder::default();

    if args.silero_model.is_some() || args.whisper_model.is_some() {
        let stt_backend = SttBackend::Whisper {
            model: whisper_path,
        };
        let stt_config = SttConfig::new(silero_path, stt_backend).with_input_pull(true);
        builder = builder.with_stt_config(stt_config);
    }

    if args.use_sine_tts {
        let tts_config = TtsConfig::new(TtsBackend::Sine);
        builder = builder.with_tts_config(tts_config);
    }

    // 4. Build the Speech-to-Speech Pipeline
    log::info!("Building Speech-to-Speech Pipeline...");
    let pipeline = builder
        .with_async_processor(move |transcript| {
            let client = Arc::clone(&herdr_client);
            let engine = Arc::clone(&reasoning_engine);
            Box::pin(async move {
                log::info!("Received transcript: '{}'", transcript);
                let executor = HerdrToolExecutor((*client).clone());
                match engine.process_transcript(&transcript, &executor).await {
                    Ok(response) => {
                        if let Some(ref reply) = response {
                            log::info!("Engine response: '{}'", reply);
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

    // 5. Start audio capture stream
    log::info!("Starting CPAL microphone audio capture...");
    let _capture = CpalCapture::start_with_processor(|samples| {
        ensure_audio_ring().push_samples(samples);
    }).map_err(|e| format!("Capture error: {}", e))?;

    log::info!("handlr is running. Speak into your microphone...");

    // Main thread event loop
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Ok(())
}
