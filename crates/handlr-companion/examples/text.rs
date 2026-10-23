//! The companion without audio (plan 20261022_04, step 2 of the 18:10 EDT
//! amendment): each typed line is an utterance, an empty line is the tap,
//! and `pair <URL, handlr1… or data: URL>` pairs. `/start`, `/submit`,
//! `/clear` and `/scratch` are the dictation buttons; `/view <pane>` opens an
//! agent view and `/close` closes it. Events are printed as they arrive.
//!
//! ```text
//! cargo run -p handlr-companion --example text -- [--keys <dir>] [--socket <path>]
//!     [--client <name>] [--name <device name>] [--pair <what handlr pair printed>]
//! ```
//!
//! `--pair` (or a `pair` line) prints the device code to paste into
//! `handlr pair`, then connects once it has been pasted. `--name` is the
//! name the device asks to be paired as (default: the hostname).
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

const USAGE: &str = "usage: text [--keys <dir>] [--socket <path>] [--client <name>] \
     [--name <device name>] [--pair <pairing URL, string or data URL>]\n\
     Lines are utterances, an empty line is the tap, `pair <…>` pairs,\n\
     /start, /submit, /clear and /scratch are the dictation buttons,\n\
     /view <pane> opens an agent view and /close closes it.";

fn main() {
    let mut keys = FileKeyStore::default_dir();
    let mut config = CompanionConfig::new("text");
    let mut pair = None;
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
            "--name" => config.device_name = value(),
            "--pair" => pair = Some(value()),
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
    if let Some(text) = pair {
        companion.send(Input::Pair(text));
    }
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
    } else if line == "/start" {
        Input::StartDictation
    } else if line == "/submit" {
        Input::Submit
    } else if line == "/clear" {
        Input::Clear
    } else if line == "/scratch" {
        Input::Scratch
    } else if let Some(pane) = line.strip_prefix("/view ") {
        Input::OpenAgent(pane.trim().to_string())
    } else if line == "/close" {
        Input::CloseAgent
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
            ..
        } => {
            let label = label.map(|l| format!(" [{l}]")).unwrap_or_default();
            println!("? {text}{label} {phrase}");
        }
        Event::Connected { server } => println!("* connected to {server}"),
        Event::Disconnected { retry_in_ms } => {
            println!("* disconnected; retrying in {retry_in_ms} ms")
        }
        Event::PairingCode { code, server } => {
            println!("* paste this code into handlr pair on {server}:");
            println!("{code}");
        }
        Event::Paired { name } => println!("* paired as {name}"),
        Event::PairFailed(why) => println!("! pairing failed: {why}"),
        Event::StopSpeaking => println!("* stop speaking"),
        Event::Arm => println!("* listening"),
        Event::Answered { approve, .. } => {
            println!("* {}", if approve { "approving" } else { "denying" })
        }
        Event::ReplyFailed { message, .. } => println!("! {message}"),
        Event::Resolved { outcome, .. } => println!("* confirmation {outcome}"),
        Event::TurnEnded { outcome, stopped } => {
            if stopped.is_empty() {
                println!("* turn {outcome}");
            } else {
                println!("* turn {outcome}; stopped {}", stopped.join(", "));
            }
        }
        Event::Vocabulary(words) => println!("* vocabulary: {}", words.join(", ")),
        Event::Focus(focus) => match focus {
            Some(f) => println!("* focus: {}", f.name),
            None => println!("* focus: none"),
        },
        Event::Dictation(d) if !d.active => println!("* message: off"),
        Event::Dictation(d) => {
            let target = d
                .target
                .map(|t| t.name)
                .unwrap_or_else(|| "no agent".into());
            let moved = d
                .focus_moved
                .map(|m| format!(" (focus moved to {m})"))
                .unwrap_or_default();
            let countdown = d
                .submit_in
                .map(|s| format!(" submitting in {s} s"))
                .unwrap_or_default();
            println!(
                "* message for {target}{moved} [{:?}]{countdown}: {}",
                d.phase, d.text
            );
        }
        Event::AgentView {
            name,
            status,
            entries,
            cards,
            pinned,
        } => {
            println!("* view of {name} ({status}): {} entries", entries.len());
            for (i, c) in cards.iter().enumerate() {
                let pin = if Some(i) == pinned { " (pinned)" } else { "" };
                let live = if c.live { "live" } else { "resolved" };
                println!("  card at {}: {} [{live}]{pin}: {}", c.at, c.title, c.text);
            }
        }
        Event::Error(e) => println!("! {e}"),
    }
}
