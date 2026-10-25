//! flankr-audio without flankr: it hears an utterance and says it back,
//! through the same `Event`s and `Input`s the companion uses, to check the
//! devices and the models.
//!
//! ```text
//! cargo run -p flankr-audio --features cpal --release --example echo -- \
//!     [--models <dir>] [--quant q4k|q4_0|q8] [--voice <name>] \
//!     [--vocab-file <path>] [--vocabulary <a,b,…>] [--say <text>] \
//!     [--stop-after-ms <n>] [--input-wav <file>]
//! ```
//!
//! Enter is the tap: it arms the gate (`Event::Arm`), or, while speaking,
//! stops the speech (`Event::StopSpeaking`). Each `Input::Utterance` is
//! answered with `Event::Say` of the same text. End of input (Ctrl-D)
//! quits.
//!
//! - `--say` speaks once at the start.
//! - `--stop-after-ms` gives `StopSpeaking` that long after each
//!   `Speaking(true)`, and logs how long the speaker took to fall silent.
//! - `--vocabulary` writes the vocabulary file, as `Event::Vocabulary`
//!   would.
//! - `--input-wav` opens only the output device and plays the file (16-bit
//!   or float WAV) into the microphone's end in real time, armed once the
//!   STT model has loaded, for a machine with no microphone. It quits once
//!   the echo has been said.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use flankr_audio::libvoicexn::capture::Mic;
use flankr_audio::libvoicexn::config::SttQuant;
use flankr_audio::{AudioConfig, AudioEvent, AudioSession, Format};
use flankr_companion::{Event, Input};

const TICK: Duration = Duration::from_millis(20);

const USAGE: &str = "usage: echo [--models <dir>] [--quant q4k|q4_0|q8] [--voice <name>] \
                     [--vocab-file <path>] [--vocabulary <a,b,…>] [--say <text>] \
                     [--stop-after-ms <n>] [--input-wav <file>]";

struct Args {
    config: AudioConfig,
    vocabulary: Option<Vec<String>>,
    say: Option<String>,
    stop_after: Option<Duration>,
    input_wav: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut models = PathBuf::from("models");
    let (mut quant, mut voice, mut vocab_file) = (None, None, None);
    let (mut vocabulary, mut say, mut stop_after, mut input_wav) = (None, None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--models" => models = PathBuf::from(value()?),
            "--quant" => {
                let name = value()?;
                quant = Some(
                    SttQuant::parse(&name).ok_or(format!("--quant {name:?}: q4k, q4_0 or q8"))?,
                );
            }
            "--voice" => voice = Some(value()?),
            "--vocab-file" => vocab_file = Some(PathBuf::from(value()?)),
            "--vocabulary" => {
                vocabulary = Some(value()?.split(',').map(str::to_string).collect());
            }
            "--say" => say = Some(value()?),
            "--stop-after-ms" => {
                let ms = value()?;
                let ms: u64 = ms
                    .parse()
                    .map_err(|_| format!("--stop-after-ms {ms:?}: a number"))?;
                stop_after = Some(Duration::from_millis(ms));
            }
            "--input-wav" => input_wav = Some(PathBuf::from(value()?)),
            _ => return Err(format!("unknown argument {arg:?}\n{USAGE}")),
        }
    }
    let mut config = AudioConfig::new(&models);
    if let Some(quant) = quant {
        config.stt.quant = quant;
    }
    config.voice = voice;
    if vocab_file.is_some() {
        config.vocab_file = vocab_file;
    }
    Ok(Args {
        config,
        vocabulary,
        say,
        stop_after,
        input_wav,
    })
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Logs the companion event and hands it to the audio.
fn give(audio: &mut AudioSession, event: Event) {
    log::info!("event: {event:?}");
    audio.handle(&event);
}

fn run(args: Args) -> Result<(), String> {
    // With a file for the microphone, a thread plays it in once told to.
    let (mut audio, start_wav) = match &args.input_wav {
        Some(path) => {
            let (format, samples) = read_wav(path)?;
            let (audio, mic) = AudioSession::open_default_output(args.config, format)?;
            let (start, started) = mpsc::channel();
            std::thread::spawn(move || feed(mic, format, &samples, started));
            (audio, Some(start))
        }
        None => (AudioSession::open_default(args.config)?, None),
    };
    if let Some(words) = args.vocabulary {
        give(&mut audio, Event::Vocabulary(words));
    }

    let (keys, enter) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if line.is_err() || keys.send(()).is_err() {
                return;
            }
        }
    });

    let (mut stt_loaded, mut tts_loaded) = (false, false);
    let mut say = args.say;
    // When speaking started, and when `StopSpeaking` was given.
    let (mut speaking, mut stopped) = (None::<Instant>, None::<Instant>);
    let mut echoed = false;
    loop {
        match enter.try_recv() {
            Ok(()) if speaking.is_some() => give(&mut audio, Event::StopSpeaking),
            Ok(()) if stt_loaded => give(&mut audio, Event::Arm),
            Ok(()) => log::info!("echo: the STT model is still loading"),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                if start_wav.is_none() {
                    return Ok(());
                }
            }
        }
        if tts_loaded {
            if let Some(text) = say.take() {
                give(&mut audio, Event::Say(text));
            }
        }
        if let (Some(after), Some(since), None) = (args.stop_after, speaking, stopped) {
            if since.elapsed() >= after {
                give(&mut audio, Event::StopSpeaking);
                stopped = Some(Instant::now());
            }
        }
        for event in audio.poll() {
            match event {
                AudioEvent::Input(input) => {
                    log::info!("input: {input:?}");
                    match input {
                        Input::Utterance(text) => {
                            println!("> {text}");
                            give(&mut audio, Event::Say(text));
                            echoed = true;
                        }
                        Input::Speaking(true) => speaking = Some(Instant::now()),
                        Input::Speaking(false) => {
                            if let Some(at) = stopped.take() {
                                log::info!(
                                    "echo: silent {} ms after StopSpeaking",
                                    at.elapsed().as_millis()
                                );
                            }
                            speaking = None;
                            if echoed && start_wav.is_some() {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
                AudioEvent::SttLoaded { load_ms } => {
                    stt_loaded = true;
                    log::info!("stt: loaded in {load_ms} ms; press Enter to talk");
                    if let Some(start) = &start_wav {
                        give(&mut audio, Event::Arm);
                        let _ = start.send(());
                    }
                }
                AudioEvent::TtsLoaded {
                    load_ms, voices, ..
                } => {
                    tts_loaded = true;
                    log::info!("tts: loaded in {load_ms} ms, {voices} voices");
                }
                AudioEvent::SttLoadFailed(e) => return Err(format!("stt: {e}")),
                AudioEvent::TtsLoadFailed(e) => return Err(format!("tts: {e}")),
                AudioEvent::Speed { step_ms, dropped } => {
                    log::info!("stt: step {step_ms:.1} ms of 80, dropped {dropped}");
                }
                other => log::info!("audio: {other:?}"),
            }
        }
        std::thread::sleep(TICK);
    }
}

/// Plays `samples` into `mic` in 20 ms blocks at real time once `start`
/// says so, after 300 ms of silence, then silence for good.
fn feed(mut mic: Mic, format: Format, samples: &[f32], start: mpsc::Receiver<()>) {
    if start.recv().is_err() {
        return;
    }
    let block = (format.rate as usize / 50) * usize::from(format.channels);
    let silence = vec![0.0; block];
    let lead = std::iter::repeat_n(&silence[..], 15);
    let mut next = Instant::now();
    for chunk in lead
        .chain(samples.chunks(block))
        .chain(std::iter::repeat(&silence[..]))
    {
        mic.push(chunk);
        next += TICK;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// A 16-bit PCM or 32-bit float WAV: its format and interleaved samples.
fn read_wav(path: &Path) -> Result<(Format, Vec<f32>), String> {
    let fail = |e: &str| format!("{}: {e}", path.display());
    let bytes = std::fs::read(path).map_err(|e| fail(&e.to_string()))?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(fail("not a WAV file"));
    }
    let (mut fmt, mut data) = (None, None);
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    let (fmt, data) = (
        fmt.ok_or(fail("no fmt chunk"))?,
        data.ok_or(fail("no data"))?,
    );
    let tag = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    let samples = match (tag, bits) {
        (1, 16) => data
            .chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect(),
        (3, 32) => data
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        _ => return Err(fail(&format!("WAV format {tag}, {bits} bits"))),
    };
    Ok((Format { rate, channels }, samples))
}
