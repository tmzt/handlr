//! `handlr` until the daemon exists: load the config, connect to herdr and
//! log the agents it can see. `handlr serve` replaces this
//! (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md, step 5).

use std::path::{Path, PathBuf};

use handlr::notify::{agent_identities, speakable_name};
use handlr::transport::{HerdrClient, HerdrTransport, RemoteSshConfig};

struct AppArgs {
    config: Option<String>,
    transport_name: Option<String>,
    remote: Option<String>,
    socket: Option<String>,
    mock_mode: bool,
}

fn parse_args() -> AppArgs {
    let mut args = std::env::args().skip(1);
    let mut parsed = AppArgs {
        config: None,
        transport_name: None,
        remote: None,
        socket: None,
        mock_mode: false,
    };

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => parsed.config = args.next(),
            "--transport" | "-t" => parsed.transport_name = args.next(),
            "--remote" => parsed.remote = args.next(),
            "--socket" => parsed.socket = args.next(),
            "--mock" => parsed.mock_mode = true,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = parse_args();
    let config = handlr::config::AppConfig::load_or_default(args.config.as_deref().map(Path::new));

    let client = HerdrClient::new(select_transport(&args, &config));
    let snapshot = client.list_panes().await?;
    let agents = agent_identities(&snapshot);
    log::info!("herdr reports {} agent(s)", agents.len());
    for agent in &agents {
        log::info!(
            "{} {} -> \"{}\"",
            agent.pane_id,
            agent.agent,
            speakable_name(agent, &agents)
        );
    }
    Ok(())
}
