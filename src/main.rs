//! `flankr serve` (the default) runs the daemon; `flankr ctl` sends it one
//! request (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md,
//! decision 1).
//!
//! ```text
//! flankr [serve] [--config <file>] [--transport <name> | --remote <user@host> |
//!                 --socket <herdr.sock> | --mock] [--listen <flankr.sock>] [--show-tokens]
//!                [--prompts <ADDITIONAL_PROMPTS.md>]
//! flankr ctl [--listen <flankr.sock>] [--client <name>] [--follow] [--handle <text>]
//!            <method> [<params as JSON>]
//! flankr pair [--listen <flankr.sock>] [--name <name>] [--protocols companion,harness]
//!             [--replace] [--code <code>] [--qr-png <file>]
//! flankr session tail [--listen <flankr.sock>] [--pane <id or name>] [--follow]
//!                     [--entries N] [--history N] [--outline] [--from N] [--to N]
//! flankr prompt [--use general_agent] [--agent <name>] [--prompts <ADDITIONAL_PROMPTS.md>]
//!               [--config <file>] [--transport <name> | --remote <user@host> |
//!                --socket <herdr.sock> | --mock]
//! ```

use std::path::{Path, PathBuf};

use flankr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig};

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
    /// `session tail`'s outline mode and content range.
    outline: bool,
    from: Option<String>,
    to: Option<String>,
    /// `prompt`'s use and agent (plan 20261027_04, decision 8).
    use_name: Option<String>,
    agent: Option<String>,
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
        outline: false,
        from: None,
        to: None,
        use_name: None,
        agent: None,
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
            "--outline" => parsed.outline = true,
            "--from" => parsed.from = args.next(),
            "--to" => parsed.to = args.next(),
            "--use" => parsed.use_name = args.next(),
            "--agent" => parsed.agent = args.next(),
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
fn select_transport(args: &AppArgs, config: &flankr::config::AppConfig) -> HerdrTransport {
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
        let socket_path = flankr::transport::default_local_socket_path();
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
        let socket_path = flankr::transport::default_local_socket_path();
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
        .unwrap_or_else(flankr::transport::default_local_socket_path);
    log::info!(
        "Connecting via UNIX domain socket (from config transport '{}'): {:?}",
        transport_name,
        socket_path
    );
    HerdrTransport::local(socket_path)
}

/// The daemon socket: `--listen`, else `[server] socket`, else the default.
fn daemon_socket(args: &AppArgs, config: &flankr::config::AppConfig) -> PathBuf {
    args.listen
        .as_ref()
        .or(config.server.socket.as_ref())
        .map(expand_tilde)
        .unwrap_or_else(flankr::server::default_socket_path)
}

async fn serve(args: AppArgs) -> Result<(), Box<dyn std::error::Error>> {
    let config = flankr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let transport = select_transport(&args, &config);
    let event_transport = (!args.mock_mode).then(|| transport.clone());
    let client = HerdrClient::new(transport);
    match client.list_panes().await {
        Ok(snapshot) => log::info!(
            "herdr reports {} pane(s)",
            flankr::core::agents::agents(&snapshot).len()
        ),
        Err(e) => log::warn!("herdr is not answering yet: {e}"),
    }

    // The link: its key is checked (and created) before anything listens.
    let settings = flankr::server::link::LinkSettings::from_config(&config.link);
    let link = if settings.enabled() {
        Some(flankr::server::link::Link::open(settings)?)
    } else {
        log::info!("The link is off ([link] listen = [])");
        None
    };

    let socket = daemon_socket(&args, &config);
    let listener = flankr::server::bind(&socket).await?;
    let link_listeners = match &link {
        Some(link) => link.bind().await?,
        None => Vec::new(),
    };
    log::info!("flankr serve listening on {}", socket.display());
    let mut options = flankr::server::ServeOptions {
        event_transport,
        ..Default::default()
    };
    options.show_tokens |= args.show_tokens;
    let turns = flankr::server::companion::TurnOptions {
        speech: flankr::server::speech::SpeechOptions {
            prompts_file: args.prompts.as_ref().map(expand_tilde),
            ..Default::default()
        },
        // `[llm]` is read again from this file while the daemon runs
        // (plan 20261027_02, decision 6).
        llm: flankr::llm::LlmOptions {
            config_path: Some(
                args.config
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(flankr::config::AppConfig::default_config_path),
            ),
            // The `planning` prompt comes from the same file
            // (plan 20261027_04, decision 6).
            prompts_file: args.prompts.as_ref().map(expand_tilde),
            ..Default::default()
        },
        ..Default::default()
    };
    let daemon = flankr::server::Daemon::with_turns(client, &config, link, turns);
    flankr::server::link::spawn(std::sync::Arc::clone(&daemon), link_listeners);
    flankr::server::run(daemon, listener, options).await;
    Ok(())
}

async fn pair(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let config = flankr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let usage = "usage: flankr pair [--listen <socket>] [--name <name>] [--protocols companion,harness] [--replace] [--code <code>] [--qr-png <file>]";
    if !args.positional.is_empty() {
        return Err(usage.into());
    }
    let options = flankr::pair::PairOptions {
        name: args.name.clone(),
        protocols: (!args.protocols.is_empty()).then(|| args.protocols.clone()),
        replace: args.replace,
        code: args.code.clone(),
        qr_png: args.qr_png.as_deref().map(PathBuf::from),
    };
    Ok(flankr::pair::run(&socket, options).await?)
}

async fn ctl(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    let config = flankr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let mut positional = args.positional.iter();
    let Some(method) = positional.next() else {
        return Err(
            "usage: flankr ctl [--listen <socket>] [--client <name>] [--follow] [--handle <text>] <method> [<params as JSON>]".into(),
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
    Ok(flankr::ctl::run(
        &socket,
        args.client.as_deref().unwrap_or("ctl"),
        method,
        params,
        args.follow,
    )
    .await?)
}

/// `flankr session tail` (plan 20261023_03, section 8): read-only.
async fn session(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    const USAGE: &str = "usage: flankr session tail [--listen <socket>] [--pane <id or name>] [--follow] [--entries N] [--history N] [--outline] [--from N] [--to N]";
    if args.positional.first().map(String::as_str) != Some("tail") || args.positional.len() > 1 {
        return Err(USAGE.into());
    }
    let number = |v: &Option<String>, default: usize| -> Result<usize, String> {
        v.as_deref().map_or(Ok(default), |n| {
            n.parse().map_err(|_| format!("not a number: {n}"))
        })
    };
    let config = flankr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let socket = daemon_socket(&args, &config);
    let options = flankr::tail::TailOptions {
        pane: args.pane.clone(),
        follow: args.follow,
        sections: number(
            &args.entries,
            libflankr_proto::common::SUBSCRIBE_SECTIONS_DEFAULT,
        )?,
        history: number(&args.history, 0)?,
        outline: args.outline,
        content: {
            let index = |v: &Option<String>| -> Result<Option<u64>, String> {
                v.as_deref()
                    .map(|n| n.parse().map_err(|_| format!("not an index: {n}")))
                    .transpose()
            };
            let (from, to) = (index(&args.from)?, index(&args.to)?);
            (from.is_some() || to.is_some()).then_some((from, to))
        },
    };
    let mut out = std::io::stdout();
    Ok(flankr::tail::run(&socket, &options, &mut out).await?)
}

/// `flankr prompt` (plan 20261027_04, decision 8): the general agent's
/// assembled prompt; reads the config and herdr, calls no LLM.
async fn prompt(args: AppArgs) -> Result<i32, Box<dyn std::error::Error>> {
    if !args.positional.is_empty() {
        return Err(PROMPT_USAGE.into());
    }
    let config = flankr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));
    let client = HerdrClient::new(select_transport(&args, &config));
    let prompts_file = args.prompts.as_ref().map(expand_tilde);
    let options = flankr::prompt_cmd::PromptOptions {
        use_name: args.use_name.as_deref(),
        agent: args.agent.as_deref(),
        prompts_file: prompts_file.as_deref(),
    };
    let env = flankr::llm::KeyEnv::from_process();
    let out = flankr::prompt_cmd::report(&config, &env, &client, &options).await?;
    print!("{out}");
    Ok(0)
}

const PROMPT_USAGE: &str = "usage: flankr prompt [--use general_agent] [--agent <name>] [--prompts <ADDITIONAL_PROMPTS.md>]";

const USAGE: &str = "\
flankr [serve] [--config <file>] [--transport <name> | --remote <user@host> |
                --socket <herdr.sock> | --mock] [--listen <flankr.sock>] [--show-tokens]
               [--prompts <ADDITIONAL_PROMPTS.md>]
flankr ctl [--listen <flankr.sock>] [--client <name>] [--follow] [--handle <text>]
           <method> [<params as JSON>]
flankr pair [--listen <flankr.sock>] [--name <name>] [--protocols companion,harness]
            [--replace] [--code <code>] [--qr-png <file>]
flankr session tail [--listen <flankr.sock>] [--pane <id or name>] [--follow]
                    [--entries N] [--history N] [--outline] [--from N] [--to N]
flankr prompt [--use general_agent] [--agent <name>] [--prompts <ADDITIONAL_PROMPTS.md>]
              [--config <file>] [--transport <name> | --remote <user@host> |
               --socket <herdr.sock> | --mock]";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv
        .iter()
        .any(|a| a == "--help" || a == "-h" || a == "help")
    {
        println!("{USAGE}");
        return Ok(());
    }
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let command = match argv.first().map(String::as_str) {
        Some("serve") | Some("ctl") | Some("pair") | Some("session") | Some("prompt") => {
            argv.remove(0)
        }
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
    if command == "prompt" {
        std::process::exit(prompt(args).await?);
    }
    if !args.positional.is_empty() {
        return Err(format!("unexpected arguments: {:?}", args.positional).into());
    }
    serve(args).await
}
