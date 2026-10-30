//! The companion without audio (plan 20261022_04, step 2 of the 18:10 EDT
//! amendment): each typed line is an utterance, an empty line is the tap,
//! and `pair <URL, flankr1… or data: URL>` pairs. `/start`, `/submit`,
//! `/clear` and `/scratch` are the dictation buttons; `/view <pane>` opens an
//! agent view and `/close` closes it. Events are printed as they arrive.
//!
//! ```text
//! cargo run -p flankr-companion --example text -- [--keys <dir>] [--socket <path>]
//!     [--client <name>] [--name <device name>] [--pair <what flankr pair printed>]
//! ```
//!
//! `--pair` (or a `pair` line) prints the device code to paste into
//! `flankr pair`, then connects once it has been pasted. `--name` is the
//! name the device asks to be paired as (default: the hostname).
//!
//! `--keys` is where the key and the paired server are kept (default
//! `~/.config/flankr-companion/`). Without a paired server it uses the
//! daemon's local socket (`--socket`, default
//! `~/.config/flankr/flankr.sock`).

use std::io::BufRead;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use flankr_companion::{Companion, CompanionConfig, Event, FileKeyStore, Input};

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
        Event::PairingWords {
            server,
            address,
            words,
            fingerprint,
            expires_in_secs,
            fallback,
            ..
        } => {
            if let Some(fallback) = &fallback {
                if fallback.replace {
                    println!("* {server} lists this phone's lost key: use flankr pair --replace");
                }
                println!("* {server} was not asked directly; paste this code into flankr pair:");
                println!("{}", fallback.code);
            }
            match words {
                Some(words) => {
                    println!("* pairing words: {words}");
                    println!("* if flankr pair on {server} shows the same, type yes there");
                }
                None => println!("* the link has no offer: no pairing words to compare"),
            }
            let left = expires_in_secs
                .map(|s| format!(", {s} s left"))
                .unwrap_or_default();
            println!("* {address}, daemon fingerprint {fingerprint}{left}");
        }
        Event::Paired { name } => println!("* paired as {name}"),
        Event::PairFailed(why) => println!("! pairing failed: {why}"),
        Event::StopSpeaking => println!("* stop speaking"),
        Event::Arm => println!("* listening"),
        Event::Ding => println!("* ding: an agent is done"),
        Event::Refreshed { new } => println!("* refreshed: {new} new"),
        Event::Cache { bytes } => println!("* cache: {bytes} bytes"),
        Event::SessionCards(cards) => {
            for c in cards {
                println!("* {:?} from {}: {}", c.kind, c.agent, c.text);
            }
        }
        Event::Remotes(rows) => {
            for r in rows {
                let use_ = if r.online {
                    " (in use)"
                } else if r.active {
                    " (in use, offline)"
                } else {
                    ""
                };
                println!("* remote {} {} {}{use_}", r.name, r.host, r.fingerprint);
            }
        }
        Event::RemoteRemoved { name, daemon, .. } => {
            println!("* removed {name}{}", if daemon { "" } else { " here only" })
        }
        Event::RemoteUpdated { name, .. } => {
            println!("* already paired with {name}: its addresses are updated")
        }
        Event::Unpaired { daemon } => {
            println!("* unpaired{}", if daemon { "" } else { " here only" })
        }
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
        Event::Llm(llm) => println!(
            "* llm: general agent {}, summaries {}{}",
            llm.general_agent,
            llm.summaries,
            llm.reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default()
        ),
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
            sections,
            pinned,
            changed: _,
            loading,
            typed,
            loading_earlier,
            start,
            terminal_on,
            new_content,
            held_end: _,
            held_newer: _,
            bookends: _,
            child,
            children: _,
            cached,
            ended,
        } => {
            if cached {
                println!("* cached");
            }
            if ended {
                println!("* this session has ended");
            }
            if new_content {
                println!("* new content");
            }
            if let Some(c) = child {
                println!("* subagent {c}");
            }
            if let Some(t) = typed {
                println!("* typed: {t}");
            }
            if let Some(t) = terminal_on {
                println!("* terminal on: {t}");
            }
            if loading_earlier {
                println!("* loading earlier…");
            }
            if start {
                println!("* start of session");
            }
            let (entries, cards, pinned) = flankr_companion::view::flatten_view(&sections, pinned);
            let loading = if loading { ", loading" } else { "" };
            println!(
                "* view of {name} ({status}{loading}): {} entries",
                entries.len()
            );
            for (i, c) in cards.iter().enumerate() {
                let pin = if Some(i) == pinned { " (pinned)" } else { "" };
                let live = if c.live { "live" } else { "resolved" };
                println!("  card at {}: {} [{live}]{pin}: {}", c.at, c.title, c.text);
            }
        }
        Event::Agents(agents) => println!("* {} agents", agents.len()),
        Event::PendingPanes(panes) => println!("* {} panes waiting for approval", panes.len()),
        // Gemini's lines also come as Say.
        Event::Conversation { .. } => {}
        Event::Error(e) => println!("! {e}"),
        Event::Background { remote, event } => println!("* in the background, {remote}: {event:?}"),
        other => println!("* {other:?}"),
    }
}
