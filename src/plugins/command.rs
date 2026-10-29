//! `PluginCtx::run_command`: the commands a plugin declares in code, run
//! with no shell (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.3,
//! item 4).
//!
//! A command is a binary found on `PATH` when the daemon starts, then the
//! project's root after `root_flag`, then fixed words, then the caller's
//! arguments, each checked against its [`ArgShape`] before a process
//! starts. The process gets no stdin, the project's root as its working
//! directory, an environment of only `PATH`, `HOME`, `LC_ALL=C` and `TZ`,
//! and [`TIMEOUT`]. The config may only narrow what is declared.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{PluginError, codes};

/// How long one command may run before it is killed.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// How much of stderr is kept, for the error message only.
pub const STDERR_MAX: usize = 4 * 1024;

/// One command a plugin may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// The name the plugin runs it by: "plans_list".
    pub name: &'static str,
    /// A bare binary name, found on `PATH` at start: "tpt".
    pub binary: &'static str,
    /// The words after the root: ["plans", "list", "--json"].
    pub fixed: &'static [&'static str],
    /// The caller's arguments, in order, each of this shape.
    pub args: &'static [ArgShape],
    /// `Some("-C")`: the flag given the project's root, before `fixed`.
    pub root_flag: Option<&'static str>,
    /// At most this many bytes of stdout; at most `PLUGIN_MAX_BYTES`.
    pub max_output: usize,
}

/// What one of the caller's arguments must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgShape {
    /// A plan's base name: `^\d{8}_\d{2}(_[A-Z0-9]+)+$`.
    PlanName,
    /// One of these words.
    Literal(&'static [&'static str]),
}

impl ArgShape {
    pub fn accepts(self, arg: &str) -> bool {
        match self {
            ArgShape::PlanName => is_plan_name(arg),
            ArgShape::Literal(words) => words.contains(&arg),
        }
    }
}

/// Whether `name` is a plan's base name: eight digits, `_`, two digits,
/// then one or more `_`-joined runs of `A-Z0-9`.
pub fn is_plan_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.len() < 13 || !b[..8].iter().all(u8::is_ascii_digit) || b[8] != b'_' {
        return false;
    }
    if !b[9..11].iter().all(u8::is_ascii_digit) || b[11] != b'_' {
        return false;
    }
    name[12..].split('_').all(|part| {
        !part.is_empty() && part.bytes().all(|c| matches!(c, b'A'..=b'Z' | b'0'..=b'9'))
    })
}

/// What a command printed: its stdout, UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
}

/// The environment a command runs with: the daemon's `PATH` (as at start),
/// `HOME` and `TZ` if set, and `LC_ALL=C`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandEnv {
    pub path: Option<OsString>,
    pub home: Option<OsString>,
    pub tz: Option<OsString>,
}

impl CommandEnv {
    pub fn from_process() -> Self {
        CommandEnv {
            path: std::env::var_os("PATH"),
            home: std::env::var_os("HOME"),
            tz: std::env::var_os("TZ"),
        }
    }

    /// `binary` on this `PATH`: the first executable regular file of that
    /// name in an absolute folder.
    pub fn find(&self, binary: &str) -> Option<PathBuf> {
        find_on_path(binary, self.path.as_deref())
    }
}

/// `binary` on `path_var` (a `PATH` value).
pub fn find_on_path(binary: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if binary.is_empty() || binary.contains('/') {
        return None;
    }
    std::env::split_paths(path_var?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(binary))
        .find(|p| {
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// The arguments after the binary: `root_flag root`, `fixed`, then each
/// of `args` checked against its shape. Refused before anything runs.
pub fn argv(spec: &CommandSpec, root: &Path, args: &[&str]) -> Result<Vec<OsString>, PluginError> {
    if args.len() != spec.args.len() {
        return Err(PluginError::invalid(format!(
            "The command {} takes {} argument(s), not {}.",
            spec.name,
            spec.args.len(),
            args.len()
        )));
    }
    let mut argv: Vec<OsString> = Vec::new();
    if let Some(flag) = spec.root_flag {
        argv.push(flag.into());
        argv.push(root.as_os_str().to_owned());
    }
    argv.extend(spec.fixed.iter().map(OsString::from));
    for (arg, shape) in args.iter().zip(spec.args) {
        if !shape.accepts(arg) {
            return Err(PluginError::invalid(format!(
                "The command {} does not accept the argument {arg:?}.",
                spec.name
            )));
        }
        argv.push(arg.into());
    }
    Ok(argv)
}

/// The names of `declared` that `[plugins."<ns>"] commands` leaves on:
/// all when the key is absent or `true`, none when `false`, and those
/// named when it is a list. A name the plugin does not declare is logged
/// and ignored; the config can never add a command.
pub fn narrow(
    ns: &str,
    declared: &[CommandSpec],
    setting: Option<&toml::Value>,
) -> Vec<&'static str> {
    let all = || declared.iter().map(|c| c.name).collect();
    match setting {
        None | Some(toml::Value::Boolean(true)) => all(),
        Some(toml::Value::Boolean(false)) => Vec::new(),
        Some(toml::Value::Array(names)) => {
            let mut kept = Vec::new();
            for name in names {
                match name
                    .as_str()
                    .and_then(|n| declared.iter().find(|c| c.name == n))
                {
                    Some(spec) => kept.push(spec.name),
                    None => log::warn!(
                        "[plugins.\"{ns}\"] commands names {name}, which the plugin does not \
                         declare; ignored"
                    ),
                }
            }
            kept
        }
        Some(other) => {
            log::warn!(
                "[plugins.\"{ns}\"] commands = {other} is not true, false or a list of names; \
                 every command is off"
            );
            Vec::new()
        }
    }
}

/// Runs `binary` with `argv` in `root` (see the module's notes).
pub async fn run(
    spec: &CommandSpec,
    binary: &Path,
    root: &Path,
    argv: &[OsString],
    env: &CommandEnv,
) -> Result<CommandOutput, PluginError> {
    run_with_timeout(spec, binary, root, argv, env, TIMEOUT).await
}

pub(crate) async fn run_with_timeout(
    spec: &CommandSpec,
    binary: &Path,
    root: &Path,
    argv: &[OsString],
    env: &CommandEnv,
    timeout: Duration,
) -> Result<CommandOutput, PluginError> {
    let failed = |message: String| PluginError::new(codes::COMMAND_FAILED, message);
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(argv)
        .current_dir(root)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in [("PATH", &env.path), ("HOME", &env.home), ("TZ", &env.tz)] {
        if let Some(value) = value {
            cmd.env(key, value);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| failed(format!("{} can't run: {e}", spec.binary)))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let max = spec.max_output;
    let work = async {
        let (out, err) = tokio::join!(
            read_capped(stdout, max, false),
            read_capped(stderr, STDERR_MAX, true)
        );
        let status = child.wait().await;
        (out, err, status)
    };
    let (out, err, status) = match tokio::time::timeout(timeout, work).await {
        Ok(done) => done,
        // Dropping the child kills it (`kill_on_drop`).
        Err(_) => {
            return Err(failed(format!(
                "{} took longer than {} s",
                spec.binary,
                timeout.as_secs_f32()
            )));
        }
    };
    let (out, over) = out;
    if over {
        return Err(failed(format!(
            "{} printed more than {} KiB",
            spec.binary,
            max / 1024
        )));
    }
    let status = status.map_err(|e| failed(format!("{} can't run: {e}", spec.binary)))?;
    if !status.success() {
        let stderr = String::from_utf8_lossy(&err.0);
        let first = stderr.lines().next().unwrap_or("").trim();
        return Err(failed(if first.is_empty() {
            format!("{} failed ({status})", spec.binary)
        } else {
            format!("{} failed: {first}", spec.binary)
        }));
    }
    let stdout = String::from_utf8(out)
        .map_err(|_| failed(format!("{} printed text that is not UTF-8", spec.binary)))?;
    Ok(CommandOutput { stdout })
}

/// At most `max` bytes of `stream`, and whether there was more. With
/// `drain`, the rest is read and dropped (stderr, so the process never
/// blocks on it); without, reading stops and the pipe is closed.
async fn read_capped(
    stream: Option<impl AsyncRead + Unpin>,
    max: usize,
    drain: bool,
) -> (Vec<u8>, bool) {
    let Some(mut stream) = stream else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut buf = [0u8; 8192];
    let mut over = false;
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = max.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
                if n > room {
                    over = true;
                    if !drain {
                        break;
                    }
                }
            }
        }
    }
    (kept, over)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const SHOW: CommandSpec = CommandSpec {
        name: "plans_show",
        binary: "stub",
        fixed: &["plans", "show", "--json"],
        args: &[ArgShape::PlanName],
        root_flag: Some("-C"),
        max_output: 64,
    };

    /// A `stub` script in a temporary folder that is the only `PATH`.
    fn stub(script: &str) -> (tempfile::TempDir, CommandEnv, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let path = bin.join("stub");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let env = CommandEnv {
            path: Some(format!("{}:/bin:/usr/bin", bin.display()).into()),
            home: Some("/home/test".into()),
            tz: None,
        };
        (dir, env, root)
    }

    #[test]
    fn plan_names() {
        for good in [
            "20261028_04_PLUGINS_PLAN",
            "20261028_04_A",
            "20260831_01_X1_Y2",
        ] {
            assert!(is_plan_name(good), "{good}");
        }
        for bad in [
            "",
            "20261028_04",
            "20261028_04_",
            "20261028_4_PLAN",
            "2026102_04_PLAN",
            "20261028_04_plugins",
            "20261028_04_PLUGINS__PLAN",
            "20261028_04_PLUGINS_PLAN.md",
            "20261028_04_PLAN;rm",
            "../20261028_04_PLAN",
            "20261028_04_PLAN ",
        ] {
            assert!(!is_plan_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_argument_vector_is_root_fixed_then_checked_arguments() {
        let root = Path::new("/src/flankr");
        let argv = argv(&SHOW, root, &["20261028_04_PLUGINS_PLAN"]).unwrap();
        assert_eq!(
            argv,
            [
                "-C",
                "/src/flankr",
                "plans",
                "show",
                "--json",
                "20261028_04_PLUGINS_PLAN"
            ]
            .map(OsString::from)
        );
        for bad in [
            &["20261028_04_plugins"][..],
            &["20261028_04_PLAN", "extra"],
            &[],
            &["20261028_04_PLAN; rm -rf /"],
            &["$(id)"],
        ] {
            let err = super::argv(&SHOW, root, bad).unwrap_err();
            assert_eq!(
                err.code,
                libflankr_proto::envelope::codes::INVALID_PARAMS,
                "{bad:?}"
            );
        }
        let literal = CommandSpec {
            args: &[ArgShape::Literal(&["plan", "spec"])],
            ..SHOW
        };
        assert!(super::argv(&literal, root, &["plan"]).is_ok());
        assert!(super::argv(&literal, root, &["plans"]).is_err());
    }

    #[test]
    fn the_config_only_narrows() {
        const SPECS: [CommandSpec; 2] = [
            SHOW,
            CommandSpec {
                name: "plans_list",
                ..SHOW
            },
        ];
        assert_eq!(narrow("a.b.c", &SPECS, None), ["plans_show", "plans_list"]);
        assert_eq!(
            narrow("a.b.c", &SPECS, Some(&toml::Value::Boolean(true))),
            ["plans_show", "plans_list"]
        );
        assert!(narrow("a.b.c", &SPECS, Some(&toml::Value::Boolean(false))).is_empty());
        let list = toml::Value::Array(vec!["plans_list".into(), "rm".into()]);
        assert_eq!(
            narrow("a.b.c", &SPECS, Some(&list)),
            ["plans_list"],
            "rm ignored"
        );
        assert!(narrow("a.b.c", &SPECS, Some(&toml::Value::Integer(1))).is_empty());
    }

    #[tokio::test]
    async fn a_stub_gets_exactly_its_vector_and_a_cleared_environment() {
        let (_dir, env, root) = stub(r#"printf '%s|' "$0" "$@"; echo; env | sort"#);
        let binary = env.find("stub").unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let argv = argv(&SHOW, &root, &["20261028_04_PLUGINS_PLAN"]).unwrap();
        let spec = CommandSpec {
            max_output: 4096,
            ..SHOW
        };
        let out = run(&spec, &binary, &root, &argv, &env).await.unwrap();
        let mut lines = out.stdout.lines();
        assert_eq!(
            lines.next().unwrap(),
            format!(
                "{}|-C|{}|plans|show|--json|20261028_04_PLUGINS_PLAN|",
                binary.display(),
                root.display()
            )
        );
        let vars: Vec<&str> = lines
            .map(|l| l.split('=').next().unwrap())
            .filter(|k| !matches!(*k, "PWD" | "SHLVL" | "_" | "OLDPWD"))
            .collect();
        assert_eq!(vars, ["HOME", "LC_ALL", "PATH"], "{}", out.stdout);
        assert!(out.stdout.contains("LC_ALL=C"));
    }

    #[tokio::test]
    async fn failures_timeouts_and_large_output() {
        let root_args = |root: &Path| argv(&SHOW, root, &["20261028_04_PLAN"]).unwrap();
        // A non-zero exit carries stderr's first line.
        let (_d, env, root) = stub("echo 'no such plan' >&2; echo second >&2; exit 3");
        let bin = env.find("stub").unwrap();
        let err = run(&SHOW, &bin, &root, &root_args(&root), &env)
            .await
            .unwrap_err();
        assert_eq!(err.code, codes::COMMAND_FAILED);
        assert_eq!(err.message, "stub failed: no such plan");
        // More than max_output (64 bytes).
        let (_d, env, root) = stub("head -c 100000 /dev/zero | tr '\\0' x");
        let bin = env.find("stub").unwrap();
        let err = run(&SHOW, &bin, &root, &root_args(&root), &env)
            .await
            .unwrap_err();
        assert!(err.message.contains("printed more than"), "{err}");
        // Within the cap.
        let (_d, env, root) = stub("printf '{\"schema\": 1}'");
        let bin = env.find("stub").unwrap();
        let out = run(&SHOW, &bin, &root, &root_args(&root), &env)
            .await
            .unwrap();
        assert_eq!(out.stdout, "{\"schema\": 1}");
        // The timeout kills it.
        let (_d, env, root) = stub("sleep 30");
        let bin = env.find("stub").unwrap();
        let started = std::time::Instant::now();
        let err = run_with_timeout(
            &SHOW,
            &bin,
            &root,
            &root_args(&root),
            &env,
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("took longer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // A missing binary is not found; a path is never looked up.
        assert_eq!(env.find("tpt-not-here"), None);
        assert_eq!(env.find("/bin/sh"), None);
        assert_eq!(env.find("../stub"), None);
    }
}
