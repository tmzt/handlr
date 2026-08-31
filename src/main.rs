use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use smol::Timer;

use audio_core::cpal_capture::CpalCapture;
use audio_core::ensure_audio_ring;
use handlr::reasoning::{GeminiReasoningEngine, MockReasoningEngine, ReasoningEngine};
use handlr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig};
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

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--api-key" => api_key = args.next(),
            "--remote" => remote = args.next(),
            "--socket" => socket = args.next(),
            "--silero" => silero_model = args.next(),
            "--whisper" => whisper_model = args.next(),
            "--sine-tts" => use_sine_tts = true,
            "--mock" => mock_mode = true,
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
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args();

    // 1. Initialize Reasoning Engine
    let reasoning_engine: Arc<dyn ReasoningEngine> = if args.mock_mode {
        log::info!("Starting in MOCK reasoning mode");
        Arc::new(MockReasoningEngine::new(Some(
            "This is a mock response from handlr.".to_string(),
        )))
    } else {
        let key = args.api_key.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Gemini API key is required (use --api-key or GEMINI_API_KEY environment variable)",
            )
        })?;
        log::info!("Initializing Gemini Reasoning Engine...");
        Arc::new(GeminiReasoningEngine::new(key)?)
    };

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

    // 3. Initialize STT and TTS Configs
    let silero_path = PathBuf::from(
        args.silero_model
            .unwrap_or_else(|| "silero_vad.onnx".to_string()),
    );
    let whisper_path = PathBuf::from(
        args.whisper_model
            .unwrap_or_else(|| "ggml-base.en.bin".to_string()),
    );

    let stt_backend = SttBackend::Whisper {
        model: whisper_path,
    };
    let stt_config = SttConfig::new(silero_path, stt_backend).with_input_pull(true);

    let tts_backend = if args.use_sine_tts {
        TtsBackend::Sine
    } else {
        TtsBackend::Sine // Default to Sine for simplicity
    };
    let tts_config = TtsConfig::new(tts_backend);

    // 4. Build the Speech-to-Speech Pipeline
    log::info!("Building Speech-to-Speech Pipeline...");
    let pipeline = SpeechToSpeechBuilder::new(stt_config, tts_config)
        .with_processor(move |transcript| {
            log::info!("Received transcript: '{}'", transcript);
            let client = Arc::clone(&herdr_client);
            let engine = Arc::clone(&reasoning_engine);
            smol::block_on(async {
                match engine.process_transcript(transcript, client.as_ref()).await {
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
    smol::block_on(async {
        loop {
            Timer::after(Duration::from_secs(1)).await;
        }
    });

    Ok(())
}
