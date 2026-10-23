//! Prints a QR code of the first argument: `--kitty` or `--iterm2` as an
//! inline image, `--data-url` as a `data:` URL, else as terminal text.
//!
//! ```text
//! cargo run -p qrcodeurl --example show -- --kitty 'handlr://example.ts.net:7347/pair#test'
//! ```

use qrcodeurl::{Inline, Png, Qr, Terminal};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().filter(|a| a.starts_with("--"));
    if mode.is_some() {
        args.remove(0);
    }
    let Some(text) = args.first() else {
        eprintln!("usage: show [--kitty|--iterm2|--data-url] <text>");
        std::process::exit(2);
    };
    let qr = match Qr::new(text) {
        Ok(qr) => qr,
        Err(e) => {
            eprintln!("show: {e}");
            std::process::exit(1);
        }
    };
    match mode.as_deref() {
        Some("--kitty") => print!("{}", qr.inline(Png::default(), Inline::Kitty)),
        Some("--iterm2") => print!("{}", qr.inline(Png::default(), Inline::Iterm2)),
        Some("--data-url") => println!("{}", qr.data_url(Png::default())),
        _ => print!("{}", qr.terminal(Terminal::default())),
    }
}
