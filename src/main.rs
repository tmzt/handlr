//! `handlr serve` (the default) runs the daemon; `handlr ctl` sends it one
//! request (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md,
//! decision 1).
//!
//! ```text
//! handlr [serve] [--config <file>] [--transport <name> | --remote <user@host> |
//!                 --socket <herdr.sock> | --mock] [--listen <handlr.sock>] [--show-tokens]
//!                [--prompts <ADDITIONAL_PROMPTS.md>]
//! handlr ctl [--listen <handlr.sock>] [--client <name>] [--follow] [--handle <text>]
//!            <method> [<params as JSON>]
//! handlr pair [--listen <handlr.sock>] [--name <name>] [--protocols companion,harness]
//!             [--replace] [--code <code>] [--qr-png <file>]
//! handlr session tail [--listen <handlr.sock>] [--pane <id or name>] [--follow]
//!                     [--entries N] [--history N]
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
    /// `serve`'s prompts file, in place of the shipped
    /// `ADDITIONAL_PROMPTS.md` (plan 20261023_05, decision 7).
    prompts: Option<String>,
    follow: bool,
    /// `ctl`'s client label.
    client: Option<String>,
    /// `ctl`'s `handle` param (`confirm.status --handle "…"`).
    handle: Option<String>,
    /// `ctl`'s method and params, in order.
    positional: Vec<String>,
    /// `pair`'s options.
    name: Option<String>,
    protocols: Vec<String>,
    replace: bool,
    /// The device code, instead of prompting for it.
    code: Option<String>,
    qr_png: Option<String>,
    /// `session tail`'s pane, backfill size and history to page back.
    pane: Option<String>,
    entries: Option<String>,
    history: Option<String>,
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
        prompts: None,
        follow: false,
        client: None,
        handle: None,
        positional: Vec::new(),
        name: None,
        protocols: Vec::new(),
        replace: false,
        code: None,
        qr_png: None,
        pane: None,
        entries: None,
        history: None,
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
            "--prompts" => parsed.prompts = args.next(),
            "--follow" => parsed.follow = true,
            "--client" => parsed.client = args.next(),
            "--handle" => parsed.handle = args.next(),
            "--name" => parsed.name = args.next(),
            "--protocols" => {
                if let Some(list) = args.next() {
                    parsed.protocols.extend(
                        list.split(',')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(str::to_string),
                    );
                }
            }
            "--code" => parsed.code = args.next(),
            "--replace" => parsed.replace = true,
            "--qr-png" => parsed.qr_png = args.next(),
            "--pane" => parsed.pane = args.next(),
            "--entries" => parsed.entries = args.next(),
            "--history" => parsed.history = args.next(),
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
    if let Ok(stripped) = p.strip_prefix("~")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(stripped);
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
        log::warn!(
            "Config transport '{}' mode is 'remote' but 'remote' string is missing; falling back to local socket: {:?}",
            transport_name,
            socket_path
        );
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

    // The link: its key is checked (and created) before anything listens.
    let settings = handlr::server::link::LinkSettings::from_config(&config.link);
    let link = if settings.enabled() {
        Some(handlr::server::link::Link::open(settings)?)
    } else {
        log::info!("The link is off ([link] listen = [])");
        None
    };

    let socket = daemon_socket(&args, &config);
    let listener = handlr::server::bind(&socket).await?;
    let link_listeners = match &link {
        Some(link) => link.bind().await?,
        None => Vec::new(),
    };
    log::info!("handlr serve listening on {}", socket.display());
    let mut options = handlr::server::ServeOptions {
        event_transport,
        ..Default::default()
    };
    options.show_tokens |= args.show_tokens;
    let turns = handlr::server::companion::TurnOptions {
        speech: handlr::server::speech::SpeechOptions {
            prompts_file: args.prompts.as_ref().map(expand_tilde),
            ..Default::default()
        },
        ..Default::default()
    };
    let daemon = handlr::server::Daemon::with_turns(client, &config, link, turns);
    handlr::server::link::spawn(std::sync::Arc::clone(&daemon), link_listeners);
    handlr::server::run(daemon, listener, options).await;
    Ok(())
}

async fn pair(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let usage = "usage: handlr pair [--listen <socket>] [--name <name>] [--protocols companion,harness] [--replace] [--code <code>] [--qr-png <file>]";
    if !args.positional.is_empty() {
        return Err(usage.into());
    }
    let options = handlr::pair::PairOptions {
        name: args.name.clone(),
        protocols: (!args.protocols.is_empty()).then(|| args.protocols.clone()),
        replace: args.replace,
        code: args.code.clone(),
        qr_png: args.qr_png.as_deref().map(PathBuf::from),
    };
    Ok(handlr::pair::run(&socket, options).await?)
}

async fn ctl(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let mut positional = args.positional.iter();
    let Some(method) = positional.next() else {
        return Err(
            "usage: handlr ctl [--listen <socket>] [--client <name>] [--follow] [--handle <text>] <method> [<params as JSON>]".into(),
        );
    };
    let mut params: serde_json::Value = match positional.next() {
        Some(json) => {
            serde_json::from_str(json).map_err(|e| format!("params are not JSON: {e}"))?
        }
        None => serde_json::json!({}),
    };
    if let Some(handle) = &args.handle {
        let obj = params
            .as_object_mut()
            .ok_or("--handle needs params that are a JSON object")?;
        obj.insert("handle".to_string(), serde_json::json!(handle));
    }
    Ok(handlr::ctl::run(
        &socket,
        args.client.as_deref().unwrap_or("ctl"),
        method,
        params,
        args.follow,
    )
    .await?)
}

/// `handlr session tail` (plan 20261023_03, section 8): read-only.
async fn session(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: handlr session tail [--listen <socket>] [--pane <id or name>] [--follow] [--entries N] [--history N]";
    if args.positional.first().map(String::as_str) != Some("tail") || args.positional.len() > 1 {
        return Err(USAGE.into());
    }
    let number = |v: &Option<String>, default: usize| -> Result<usize, String> {
        v.as_deref().map_or(Ok(default), |n| {
            n.parse().map_err(|_| format!("not a number: {n}"))
        })
    };
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let options = handlr::tail::TailOptions {
        pane: args.pane.clone(),
        follow: args.follow,
        sections: number(
            &args.entries,
            libhandlr_proto::common::SUBSCRIBE_SECTIONS_DEFAULT,
        )?,
        history: number(&args.history, 0)?,
    };
    let mut out = std::io::stdout();
    Ok(handlr::tail::run(&socket, &options, &mut out).await?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    let command = match argv.first().map(String::as_str) {
        Some("serve") | Some("ctl") | Some("pair") | Some("session") => argv.remove(0),
        _ => "serve".to_string(),
    };
    let args = parse_args(argv.into_iter());
    if command == "ctl" {
        std::process::exit(ctl(args).await?);
    }
    if command == "pair" {
        std::process::exit(pair(args).await?);
    }
    if command == "session" {
        std::process::exit(session(args).await?);
    }
    if !args.positional.is_empty() {
        return Err(format!("unexpected arguments: {:?}", args.positional).into());
    }
    serve(args).await
}
