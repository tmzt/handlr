//! The desktop entry: the companion in libmorph's emulated phone.
//!
//! ```text
//! cargo run --release -- [--models <dir>] [--quant q4k|q4_0|q8] [--voice <name>]
//!     [--pair <pairing URL, flankr1… or data: URL>] [--keys <dir>] [--name <device name>]
//! ```
//!
//! `--models` holds `phonon-31m-en` and `stt-1b-en_fr` (default `models/`).
//! Keys are in the login Keychain on macOS, or with `--keys`, and on other
//! desktops, in files (`~/.config/flankr-companion/` by default).

use std::process::ExitCode;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match libmorph::run::<companion::App>() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
