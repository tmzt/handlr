//! The companion without audio (plan 20261022_04, step 2 of the 18:10 EDT
//! amendment): each typed line is an utterance, an empty line is the tap,
//! and `pair <handlr1…>` pairs. Events are printed as they arrive.
//!
//! ```text
//! cargo run -p handlr-companion --example text -- [--keys <dir>] [--socket <path>] [--client <name>]
//! ```
//!
//! `--keys` is where the key and the paired server are kept (default
//! `~/.config/handlr-companion/`). Without a paired server it uses the
//! daemon's local socket (`--socket`, default
//! `~/.config/handlr/handlr.sock`).

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use handlr_companion::{Companion, CompanionConfig, Event, FileKeyStore, Input};

const USAGE: &str = "usage: text [--keys <dir>] [--socket <path>] [--client <name>]\n\
     Lines are utterances, an empty line is the tap, `pair <handlr1…>` pairs.";

fn main() {
    let mut keys = FileKeyStore::default_dir();
    let mut config = CompanionConfig::new("text");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{arg} needs a value\n{USAGE}");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--keys" => keys = PathBuf::from(value()),
            "--socket" => config.socket = PathBuf::from(value()),
            "--client" => config.client = value(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            other => {
                eprintln!("unknown argument {other:?}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let companion = Companion::spawn(config, Box::new(FileKeyStore::new(keys)));
    let (lines_tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });

    loop {
        for event in companion.poll() {
            print_event(event);
        }
        match lines.recv_timeout(Duration::from_millis(20)) {
            Ok(line) => companion.send(input(&line)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    // Give the last events a moment, then leave.
    while let Some(event) = companion.wait_timeout(Duration::from_millis(200)) {
        print_event(event);
    }
}

fn input(line: &str) -> Input {
    let line = line.trim();
    if line.is_empty() {
        Input::Tap
    } else if let Some(string) = line.strip_prefix("pair ") {
        Input::Pair(string.trim().to_string())
    } else {
        Input::Utterance(line.to_string())
    }
}

fn print_event(event: Event) {
    match event {
        Event::Say(text) => println!("< {text}"),
        Event::Confirm {
            text,
            label,
            phrase,
        } => {
            let label = label.map(|l| format!(" [{l}]")).unwrap_or_default();
            println!("? {text}{label} {phrase}");
        }
        Event::Connected { server } => println!("* connected to {server}"),
        Event::Disconnected { retry_in_ms } => {
            println!("* disconnected; retrying in {retry_in_ms} ms")
        }
        Event::Paired { name } => println!("* paired as {name}"),
        Event::PairFailed(why) => println!("! pairing failed: {why}"),
        Event::StopSpeaking => println!("* stop speaking"),
        Event::Arm => println!("* listening"),
        Event::Resolved { outcome } => println!("* confirmation {outcome}"),
        Event::TurnEnded { outcome, stopped } => {
            if stopped.is_empty() {
                println!("* turn {outcome}");
            } else {
                println!("* turn {outcome}; stopped {}", stopped.join(", "));
            }
        }
        Event::Vocabulary(words) => println!("* vocabulary: {}", words.join(", ")),
        Event::Error(e) => println!("! {e}"),
    }
}
