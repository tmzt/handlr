//! `handlr serve` (the default) runs the daemon; `handlr ctl` sends it one
//! request (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md,
//! decision 1).
//!
//! ```text
//! handlr [serve] [--config <file>] [--transport <name> | --remote <user@host> |
//!                 --socket <herdr.sock> | --mock] [--listen <handlr.sock>] [--show-tokens]
//! handlr ctl [--listen <handlr.sock>] [--client <name>] [--follow] <method> [<params as JSON>]
//! ```

use std::path::{Path, PathBuf};

use handlr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig};

struct AppArgs {
    config: Option<String>,
    transport_name: Option<String>,
    remote: Option<String>,
    socket: Option<String>,
    mock_mode: bool,
    /// The daemon socket, for `serve` and `ctl`.
    listen: Option<String>,
    show_tokens: bool,
    follow: bool,
    /// `ctl`'s client label.
    client: Option<String>,
    /// `ctl`'s method and params, in order.
    positional: Vec<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> AppArgs {
    let mut args = args;
    let mut parsed = AppArgs {
        config: None,
        transport_name: None,
        remote: None,
        socket: None,
        mock_mode: false,
        listen: None,
        show_tokens: false,
        follow: false,
        client: None,
        positional: Vec::new(),
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => parsed.config = args.next(),
            "--transport" | "-t" => parsed.transport_name = args.next(),
            "--remote" => parsed.remote = args.next(),
            "--socket" => parsed.socket = args.next(),
            "--mock" => parsed.mock_mode = true,
            "--listen" => parsed.listen = args.next(),
            "--show-tokens" => parsed.show_tokens = true,
            "--follow" => parsed.follow = true,
            "--client" => parsed.client = args.next(),
            _ if !arg.starts_with("--") => parsed.positional.push(arg),
            _ => {
                eprintln!("Unknown argument: {}", arg);
            }
        }
    }

    parsed
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

/// The herdr transport named by the flags, else by `config.toml`, else the
/// default local socket.
fn select_transport(args: &AppArgs, config: &handlr::config::AppConfig) -> HerdrTransport {
    if args.mock_mode {
        log::info!("Using Mock Transport");
        return HerdrTransport::mock();
    }
    if let Some(ref ssh_target) = args.remote {
        log::info!("Connecting via SSH to: {}", ssh_target);
        return HerdrTransport::remote_ssh_with_config(RemoteSshConfig::new(ssh_target.clone()));
    }
    if let Some(ref socket_str) = args.socket {
        let socket_path = expand_tilde(socket_str);
        log::info!("Connecting via UNIX domain socket: {:?}", socket_path);
        return HerdrTransport::local(socket_path);
    }
    let Some(t) = config.find_transport(args.transport_name.as_deref()) else {
        let socket_path = handlr::transport::default_local_socket_path();
        log::info!(
            "Connecting via default UNIX domain socket: {:?}",
            socket_path
        );
        return HerdrTransport::local(socket_path);
    };
    let transport_name = t.name.as_deref().unwrap_or("unnamed");
    if t.mode.eq_ignore_ascii_case("remote") {
        if let Some(ref remote_target) = t.remote {
            log::info!(
                "Connecting via SSH (from config transport '{}') to: {}",
                transport_name,
                remote_target
            );
            return HerdrTransport::remote_ssh_with_config(RemoteSshConfig::new(
                remote_target.clone(),
            ));
        }
        let socket_path = handlr::transport::default_local_socket_path();
        log::warn!("Config transport '{}' mode is 'remote' but 'remote' string is missing; falling back to local socket: {:?}", transport_name, socket_path);
        return HerdrTransport::local(socket_path);
    }
    let socket_path = t
        .socket
        .as_ref()
        .map(expand_tilde)
        .unwrap_or_else(handlr::transport::default_local_socket_path);
    log::info!(
        "Connecting via UNIX domain socket (from config transport '{}'): {:?}",
        transport_name,
        socket_path
    );
    HerdrTransport::local(socket_path)
}

/// The daemon socket: `--listen`, else `[server] socket`, else the default.
fn daemon_socket(args: &AppArgs, config: &handlr::config::AppConfig) -> PathBuf {
    args.listen
        .as_ref()
        .or(config.server.socket.as_ref())
        .map(expand_tilde)
        .unwrap_or_else(handlr::server::default_socket_path)
}

async fn serve(args: AppArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let transport = select_transport(&args, &config);
    let event_transport = (!args.mock_mode).then(|| transport.clone());
    let client = HerdrClient::new(transport);
    match client.list_panes().await {
        Ok(snapshot) => log::info!(
            "herdr reports {} pane(s)",
            handlr::core::agents::agents(&snapshot).len()
        ),
        Err(e) => log::warn!("herdr is not answering yet: {e}"),
    }

    let socket = daemon_socket(&args, &config);
    let listener = handlr::server::bind(&socket).await?;
    log::info!("handlr serve listening on {}", socket.display());
    let mut options = handlr::server::ServeOptions {
        event_transport,
        ..Default::default()
    };
    options.show_tokens |= args.show_tokens;
    handlr::server::run(
        handlr::server::Daemon::new(client, &config),
        listener,
        options,
    )
    .await;
    Ok(())
}

async fn ctl(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let mut positional = args.positional.iter();
    let Some(method) = positional.next() else {
        return Err(
            "usage: handlr ctl [--listen <socket>] [--client <name>] [--follow] <method> [<params as JSON>]".into(),
        );
    };
    let params = match positional.next() {
        Some(json) => {
            serde_json::from_str(json).map_err(|e| format!("params are not JSON: {e}"))?
        }
        None => serde_json::json!({}),
    };
    Ok(handlr::ctl::run(
        &socket,
        args.client.as_deref().unwrap_or("ctl"),
        method,
        params,
        args.follow,
    )
    .await?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match argv.first().map(String::as_str) {
        Some("serve") | Some("ctl") => argv.remove(0),
        _ => "serve".to_string(),
    };
    let args = parse_args(argv.into_iter());
    if command == "ctl" {
        std::process::exit(ctl(args).await?);
    }
    if !args.positional.is_empty() {
        return Err(format!("unexpected arguments: {:?}", args.positional).into());
    }
    serve(args).await
}
