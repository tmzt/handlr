//! `PluginCtx`: what a plugin may do, and nothing else
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.3).
//!
//! The general agent's read-only operations with the same restrictions
//! (`list_files`, `read_file`, `recent_commits`, the focused project), the
//! commands the plugin declares, notifications to the calling device
//! during the call, and its own config table. It holds no herdr client:
//! only a [`ProjectLookup`], which reads the snapshot's folders, so a
//! plugin can never send keys, messages or commands to a pane.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use super::command::{self, CommandEnv, CommandOutput, CommandSpec};
use super::{PluginError, codes};
use crate::core::principal::Principal;
use crate::llm::ReadPattern;
use crate::project::{AgentFolder, FileText, Listing, MAX_READ_BYTES, ProjectFiles};

/// What reads say while `[llm.uses.planning] read` has no patterns.
pub const READING_OFF: &str = "Reading files is off: [llm.uses.planning] has no read patterns.";

/// The read-only lookup of an agent's folder a plugin's context holds.
#[async_trait::async_trait]
pub trait ProjectLookup: Send + Sync {
    /// The agent in `pane_id` and its folder on this machine; `None` is no
    /// project open.
    async fn folder(&self, pane_id: Option<&str>) -> Result<AgentFolder, String>;
}

/// The lookup over herdr: refused over SSH, else the snapshot's `cwd`
/// ([`crate::project::agent_folder_named`]). Its client is private and
/// used for `list_panes` only.
pub struct HerdrLookup {
    client: crate::transport::HerdrClient,
}

impl HerdrLookup {
    pub fn new(client: crate::transport::HerdrClient) -> Self {
        HerdrLookup { client }
    }
}

#[async_trait::async_trait]
impl ProjectLookup for HerdrLookup {
    async fn folder(&self, pane_id: Option<&str>) -> Result<AgentFolder, String> {
        crate::project::check_local(&self.client)?;
        let Some(pane_id) = pane_id else {
            return Err(crate::project::NO_PROJECT.to_string());
        };
        crate::project::agent_folder_named(&self.client, pane_id).await
    }
}

/// The focused project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Canonical.
    pub root: PathBuf,
    /// The root's folder name, e.g. "flankr".
    pub name: String,
    /// The agent whose folder it is, lowercased.
    pub agent: String,
    pub pane_id: String,
}

/// A notification sink: `(method, params)` sent as `plugin.notify` on the
/// calling connection, tagged with the plugin's namespace.
pub type Notifier = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// The commands one plugin may run now: declared, left on by the config,
/// with each binary as found at start (`None`: not on `PATH`).
#[derive(Debug, Clone, Default)]
pub struct Commands {
    pub(crate) on: Vec<(&'static CommandSpec, Option<PathBuf>)>,
    pub(crate) env: CommandEnv,
}

/// What one plugin call may do.
pub struct PluginCtx {
    ns: &'static str,
    project: Result<Project, PluginError>,
    /// The configured `[llm.uses.planning] read`.
    read: Vec<ReadPattern>,
    /// The plugin's own `read_patterns()`.
    plugin_read: Vec<ReadPattern>,
    commands: Commands,
    config: toml::Table,
    notifier: Option<Notifier>,
}

/// What [`PluginCtx::build`] is given.
pub struct CtxParts {
    pub ns: &'static str,
    pub principal: Principal,
    /// The call's `session`, else the conversation's current agent.
    pub pane: Option<String>,
    pub lookup: Arc<dyn ProjectLookup>,
    pub read: Vec<ReadPattern>,
    pub plugin_read: Vec<ReadPattern>,
    pub commands: Commands,
    pub config: toml::Table,
    pub notifier: Option<Notifier>,
}

impl PluginCtx {
    /// The context for one call: the focused project looked up once.
    pub async fn build(parts: CtxParts) -> PluginCtx {
        let project = resolve(&parts.principal, parts.pane.as_deref(), &*parts.lookup).await;
        PluginCtx {
            ns: parts.ns,
            project,
            read: parts.read,
            plugin_read: parts.plugin_read,
            commands: parts.commands,
            config: parts.config,
            notifier: parts.notifier,
        }
    }

    /// The focused project: the call's `session`, else the conversation's
    /// current agent; refused when there is none, it is on another
    /// machine, or this device may not reach its agent.
    pub fn project(&self) -> Result<&Project, PluginError> {
        self.project.as_ref().map_err(Clone::clone)
    }

    /// The project's files as a plugin may read them: where a configured
    /// pattern and one of the plugin's both match.
    fn files(&self) -> Result<ProjectFiles, PluginError> {
        if self.read.is_empty() {
            return Err(PluginError::new(codes::READ_REFUSED, READING_OFF));
        }
        let project = self.project()?;
        ProjectFiles::open(&project.root.to_string_lossy(), self.read.clone())
            .map(|f| f.narrowed(self.plugin_read.clone()))
            .map_err(|e| PluginError::new(codes::PROJECT_UNAVAILABLE, e))
    }

    /// `list_files(glob?)`: the readable files, sorted, at most 500.
    pub async fn list_files(&self, glob: Option<&str>) -> Result<Listing, PluginError> {
        let files = self.files()?;
        let root = files.root().display().to_string();
        let asked = glob.map(str::to_string);
        let listing = tokio::task::spawn_blocking(move || files.list(asked.as_deref()))
            .await
            .map_err(|e| PluginError::new(codes::READ_REFUSED, e.to_string()))?
            .map_err(PluginError::invalid)?;
        log::info!(
            "plugin {}: list_files in {root} (glob {glob:?}): {} path(s), {} more",
            self.ns,
            listing.paths.len(),
            listing.more
        );
        Ok(listing)
    }

    /// `read_file(path)`: at most 64 KiB of one readable file.
    pub async fn read_file(&self, path: &str) -> Result<FileText, PluginError> {
        let files = self.files()?;
        let root = files.root().display().to_string();
        let asked = path.to_string();
        let read = tokio::task::spawn_blocking(move || files.read(&asked))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
        match read {
            Ok(read) => {
                log::info!(
                    "plugin {}: read_file in {root}: '{path}' ({} bytes{})",
                    self.ns,
                    read.len,
                    if read.cut { ", cut" } else { "" }
                );
                Ok(read)
            }
            Err(e) => {
                log::info!(
                    "plugin {}: read_file in {root}: '{path}' refused: {e}",
                    self.ns
                );
                Err(PluginError::new(codes::READ_REFUSED, e))
            }
        }
    }

    /// `recent_commits(count?, path?, files?)`, as the general agent's:
    /// the `git` binary, the same clamps, environment and path check.
    pub async fn recent_commits(
        &self,
        count: Option<i64>,
        path: Option<&str>,
        files: bool,
    ) -> Result<crate::git::History, PluginError> {
        let project = self.project()?;
        let Some(git) = crate::git::binary() else {
            return Err(PluginError::new(
                codes::COMMAND_UNAVAILABLE,
                "git is not installed where flankr runs.",
            ));
        };
        self.recent_commits_with(git, project, count, path, files)
            .await
    }

    async fn recent_commits_with(
        &self,
        git: &Path,
        project: &Project,
        count: Option<i64>,
        path: Option<&str>,
        files: bool,
    ) -> Result<crate::git::History, PluginError> {
        let root = project.root.display().to_string();
        let count = crate::git::clamp_count(count);
        let path = path.map(str::trim).filter(|p| !p.is_empty());
        let spec = match path {
            Some(path) => Some(crate::git::check_path(path, &root).map_err(|e| {
                log::info!(
                    "plugin {}: recent_commits in {root}: '{path}' refused: {e}",
                    self.ns
                );
                PluginError::invalid(e)
            })?),
            None => None,
        };
        let history = crate::git::recent_commits(git, &project.root, count, spec.as_deref(), files)
            .await
            .map_err(|e| PluginError::new(codes::COMMAND_FAILED, e))?;
        log::info!(
            "plugin {}: recent_commits in {root} (count {count}, path {path:?}, files {files})",
            self.ns
        );
        Ok(history)
    }

    /// Runs the declared command `name` with `args` in the project's root.
    pub async fn run_command(
        &self,
        name: &str,
        args: &[&str],
    ) -> Result<CommandOutput, PluginError> {
        let Some((spec, binary)) = self.commands.on.iter().find(|(c, _)| c.name == name) else {
            return Err(PluginError::new(
                codes::COMMAND_UNAVAILABLE,
                format!("The command {name} is not declared, or the config switched it off."),
            ));
        };
        let Some(binary) = binary else {
            return Err(PluginError::new(
                codes::COMMAND_UNAVAILABLE,
                format!("{} is not installed where flankr runs.", spec.binary),
            ));
        };
        let project = self.project()?;
        let argv = command::argv(spec, &project.root, args)?;
        log::info!(
            "plugin {}: run_command {name} in {} ({} argument(s))",
            self.ns,
            project.root.display(),
            args.len()
        );
        command::run(spec, binary, &project.root, &argv, &self.commands.env).await
    }

    /// A `plugin.notify` to the calling device, during this call only.
    /// A bad method name or params over 256 KiB are dropped with a
    /// warning.
    pub fn notify(&self, method: &str, params: Value) {
        if !libflankr_proto::companion::is_plugin_method(method) {
            log::warn!(
                "plugin {}: notification {method:?} has a bad name; dropped",
                self.ns
            );
            return;
        }
        let size = serde_json::to_vec(&params)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > libflankr_proto::companion::PLUGIN_MAX_BYTES {
            log::warn!(
                "plugin {}: notification {method} is over 256 KiB; dropped",
                self.ns
            );
            return;
        }
        if let Some(notifier) = &self.notifier {
            notifier(method, params);
        }
    }

    /// Its own `[plugins."<ns>"]` table.
    pub fn config(&self) -> &toml::Table {
        &self.config
    }

    /// How much of a file `read_file` gives: 64 KiB.
    pub fn max_read_bytes(&self) -> usize {
        MAX_READ_BYTES
    }
}

async fn resolve(
    principal: &Principal,
    pane: Option<&str>,
    lookup: &dyn ProjectLookup,
) -> Result<Project, PluginError> {
    let unavailable = |e: String| PluginError::new(codes::PROJECT_UNAVAILABLE, e);
    let pane = pane.map(str::trim).filter(|p| !p.is_empty());
    let found = lookup.folder(pane).await.map_err(unavailable)?;
    if !principal.may_reach(&found.agent) {
        return Err(unavailable(format!(
            "This device may not reach the agent in pane {}.",
            found.pane_id
        )));
    }
    let root = std::fs::canonicalize(&found.folder).map_err(|e| {
        unavailable(format!(
            "The project's folder {} cannot be read: {e}",
            found.folder
        ))
    })?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Project {
        root,
        name,
        agent: found.agent,
        pane_id: found.pane_id,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A lookup over a fixed list of `(pane, agent, folder)`.
    pub(crate) struct FakeLookup(pub Vec<(String, String, String)>);

    #[async_trait::async_trait]
    impl ProjectLookup for FakeLookup {
        async fn folder(&self, pane_id: Option<&str>) -> Result<AgentFolder, String> {
            let Some(pane_id) = pane_id else {
                return Err(crate::project::NO_PROJECT.to_string());
            };
            self.0
                .iter()
                .find(|(p, _, _)| p == pane_id)
                .map(|(p, a, f)| AgentFolder {
                    pane_id: p.clone(),
                    agent: a.clone(),
                    folder: f.clone(),
                })
                .ok_or_else(|| {
                    format!("The folder of the current agent (pane {pane_id}) is not known.")
                })
        }
    }

    pub(crate) fn patterns(list: &[&str]) -> Vec<ReadPattern> {
        list.iter()
            .map(|p| ReadPattern::parse(p).unwrap())
            .collect()
    }

    fn write(root: &Path, rel: &str, text: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A project with plans, secrets and a link out; a lookup with it in
    /// `w1:p1` (claude) and another in `w1:p2` (codex).
    fn sample() -> (tempfile::TempDir, PathBuf, Arc<FakeLookup>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        for rel in [
            "README.md",
            "notes.md",
            "docs/design/20261028_01_A_PLAN.md",
            "docs/other.md",
            ".env",
            ".private/k.md",
        ] {
            write(&root, rel, b"text");
        }
        write(dir.path(), "outside.md", b"outside");
        std::os::unix::fs::symlink(
            dir.path().join("outside.md"),
            root.join("docs/design/out.md"),
        )
        .unwrap();
        let mut long = vec![b'a'; MAX_READ_BYTES + 10];
        long[0] = b'#';
        write(&root, "docs/design/long.md", &long);
        let other = dir.path().join("other");
        std::fs::create_dir(&other).unwrap();
        let lookup = Arc::new(FakeLookup(vec![
            ("w1:p1".into(), "claude".into(), root.display().to_string()),
            ("w1:p2".into(), "codex".into(), other.display().to_string()),
        ]));
        (dir, root, lookup)
    }

    async fn ctx(
        lookup: Arc<FakeLookup>,
        principal: Principal,
        pane: Option<&str>,
        read: &[&str],
    ) -> PluginCtx {
        PluginCtx::build(CtxParts {
            ns: "dev.flankr.plugins.test",
            principal,
            pane: pane.map(str::to_string),
            lookup,
            read: patterns(read),
            plugin_read: patterns(&["README.md", "docs/design/*.md"]),
            commands: Commands::default(),
            config: toml::Table::new(),
            notifier: None,
        })
        .await
    }

    #[tokio::test]
    async fn no_project_and_agents_out_of_reach() {
        let (_dir, _root, lookup) = sample();
        let none = ctx(lookup.clone(), Principal::local("t"), None, &["**/*.md"]).await;
        let err = none.project().unwrap_err();
        assert_eq!(err.code, codes::PROJECT_UNAVAILABLE);
        assert_eq!(err.message, crate::project::NO_PROJECT);
        assert_eq!(
            none.read_file("README.md").await.unwrap_err().message,
            crate::project::NO_PROJECT
        );
        let narrowed = Principal {
            agents: Some(vec!["codex".into()]),
            ..Principal::local("t")
        };
        let out = ctx(
            lookup.clone(),
            narrowed.clone(),
            Some("w1:p1"),
            &["**/*.md"],
        )
        .await;
        assert!(out.project().unwrap_err().message.contains("may not reach"));
        let codex = ctx(lookup, narrowed, Some("w1:p2"), &["**/*.md"]).await;
        assert_eq!(codex.project().unwrap().agent, "codex");
        assert_eq!(codex.project().unwrap().name, "other");
    }

    /// Decision 3.3, item 5: everything a context offers, over herdr's
    /// mock, sends nothing to a pane and moves no focus.
    #[tokio::test]
    async fn a_context_never_writes_to_herdr() {
        let (_dir, root, _) = sample();
        let mut state = crate::transport::MockState::default();
        state.snapshot.workspaces[0].tabs[0].panes[0].cwd = Some(root.display().to_string());
        let client = crate::transport::HerdrClient::mock_with_state(state);
        let c = PluginCtx::build(CtxParts {
            ns: "dev.flankr.plugins.test",
            principal: Principal::local("t"),
            pane: Some("p1".into()),
            lookup: Arc::new(HerdrLookup::new(client.clone())),
            read: patterns(&["**/*.md"]),
            plugin_read: patterns(&["**/*.md"]),
            commands: Commands::default(),
            config: toml::Table::new(),
            notifier: None,
        })
        .await;
        assert_eq!(
            c.project().unwrap().root,
            std::fs::canonicalize(&root).unwrap()
        );
        assert!(c.list_files(None).await.is_ok());
        assert!(c.read_file("README.md").await.is_ok());
        assert!(c.run_command("anything", &[]).await.is_err());
        c.notify("progress", serde_json::json!({}));
        let crate::transport::HerdrTransport::Mock { state } = client.transport() else {
            unreachable!()
        };
        let state = state.read().await;
        assert!(state.command_log.is_empty());
        assert!(state.keystroke_log.is_empty());
        assert!(state.read_log.is_empty(), "not even a pane's output");
        assert_eq!(
            state.focused_target,
            crate::transport::MockState::default().focused_target,
            "the focus did not move"
        );
    }

    #[tokio::test]
    async fn the_ssh_refusal_comes_from_the_lookup() {
        let client = crate::transport::HerdrClient::new(
            crate::transport::HerdrTransport::remote_ssh("user@example.invalid"),
        );
        let lookup = HerdrLookup::new(client);
        let err = lookup.folder(Some("w1:p1")).await.unwrap_err();
        assert!(err.contains("another machine"), "{err}");
    }

    #[tokio::test]
    async fn reads_need_both_pattern_sets_and_keep_the_refusals() {
        let (_dir, _root, lookup) = sample();
        let c = ctx(
            lookup.clone(),
            Principal::local("t"),
            Some("w1:p1"),
            &["*.md", "docs/**/*.md"],
        )
        .await;
        assert_eq!(c.read_file("README.md").await.unwrap().text, "text");
        assert_eq!(
            c.read_file("docs/design/20261028_01_A_PLAN.md")
                .await
                .unwrap()
                .text,
            "text"
        );
        // The configured read matches, the plugin's patterns don't.
        for path in ["notes.md", "docs/other.md"] {
            assert_eq!(
                c.read_file(path).await.unwrap_err().code,
                codes::READ_REFUSED,
                "{path}"
            );
        }
        // The plugin's match, the configured read doesn't.
        let narrow = ctx(
            lookup.clone(),
            Principal::local("t"),
            Some("w1:p1"),
            &["docs/**/*.md"],
        )
        .await;
        assert!(narrow.read_file("README.md").await.is_err());
        // Refused whatever the patterns say.
        let all = ctx(
            lookup.clone(),
            Principal::local("t"),
            Some("w1:p1"),
            &["**", "**/*"],
        )
        .await;
        for path in [
            ".env",
            ".private/k.md",
            "../outside.md",
            "docs/design/out.md",
            "docs/design/../../outside.md",
        ] {
            assert!(all.read_file(path).await.is_err(), "{path}");
        }
        // The 64 KiB cut.
        let long = c.read_file("docs/design/long.md").await.unwrap();
        assert!(long.cut);
        assert_eq!(long.text.len(), MAX_READ_BYTES);
        // Listing: both sets, no links.
        assert_eq!(
            c.list_files(None).await.unwrap().paths,
            [
                "README.md",
                "docs/design/20261028_01_A_PLAN.md",
                "docs/design/long.md"
            ]
        );
        // Reading off.
        let off = ctx(lookup, Principal::local("t"), Some("w1:p1"), &[]).await;
        let err = off.read_file("README.md").await.unwrap_err();
        assert_eq!(err.message, READING_OFF);
        assert_eq!(off.list_files(None).await.unwrap_err().message, READING_OFF);
    }

    #[tokio::test]
    async fn the_listing_cap() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(crate::project::MAX_LISTED + 2) {
            write(dir.path(), &format!("docs/design/{i:04}.md"), b"x");
        }
        let lookup = Arc::new(FakeLookup(vec![(
            "p".into(),
            "claude".into(),
            dir.path().display().to_string(),
        )]));
        let c = ctx(lookup, Principal::local("t"), Some("p"), &["docs/**/*.md"]).await;
        let listing = c.list_files(None).await.unwrap();
        assert_eq!(listing.paths.len(), crate::project::MAX_LISTED);
        assert_eq!(listing.more, 2);
    }

    #[tokio::test]
    async fn recent_commits_on_a_temporary_repository() {
        let Some(git) = crate::git::binary() else {
            eprintln!("git is not installed; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new(git)
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "T")
                .env("GIT_AUTHOR_EMAIL", "t@example.com")
                .env("GIT_COMMITTER_NAME", "T")
                .env("GIT_COMMITTER_EMAIL", "t@example.com")
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q"]);
        write(root, "docs/design/a.md", b"a");
        run(&["add", "."]);
        run(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "first plan",
        ]);
        let lookup = Arc::new(FakeLookup(vec![(
            "p".into(),
            "claude".into(),
            root.display().to_string(),
        )]));
        let c = ctx(lookup, Principal::local("t"), Some("p"), &["**/*.md"]).await;
        let project = c.project().unwrap().clone();
        let history = c
            .recent_commits_with(git, &project, Some(5), Some("docs/design"), true)
            .await
            .unwrap();
        let crate::git::History::Commits(commits) = history else {
            panic!("not a repository")
        };
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].subject, "first plan");
        assert_eq!(
            commits[0].files.as_deref(),
            Some(&["docs/design/a.md".to_string()][..])
        );
        let err = c
            .recent_commits_with(git, &project, None, Some("../.."), false)
            .await
            .unwrap_err();
        assert_eq!(err.code, libflankr_proto::envelope::codes::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn notifications_are_checked_and_sent() {
        let (_dir, _root, lookup) = sample();
        let sent: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let into = Arc::clone(&sent);
        let c = PluginCtx::build(CtxParts {
            ns: "dev.flankr.plugins.test",
            principal: Principal::local("t"),
            pane: None,
            lookup,
            read: vec![],
            plugin_read: vec![],
            commands: Commands::default(),
            config: toml::Table::new(),
            notifier: Some(Arc::new(move |m: &str, p: Value| {
                into.lock().unwrap().push((m.to_string(), p))
            })),
        })
        .await;
        c.notify("progress", serde_json::json!({"done": 1}));
        c.notify("Bad Name", serde_json::json!({}));
        c.notify(
            "big",
            Value::String("x".repeat(libflankr_proto::companion::PLUGIN_MAX_BYTES)),
        );
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "progress");
    }

    #[tokio::test]
    async fn commands_not_declared_or_not_installed() {
        const SPEC: CommandSpec = CommandSpec {
            name: "plans_list",
            binary: "tpt",
            fixed: &["plans", "list", "--json"],
            args: &[],
            root_flag: Some("-C"),
            max_output: 1024,
        };
        let (_dir, _root, lookup) = sample();
        let mut c = ctx(lookup, Principal::local("t"), Some("w1:p1"), &["**/*.md"]).await;
        let err = c.run_command("plans_list", &[]).await.unwrap_err();
        assert_eq!(err.code, codes::COMMAND_UNAVAILABLE);
        c.commands.on = vec![(&SPEC, None)];
        let err = c.run_command("plans_list", &[]).await.unwrap_err();
        assert_eq!(err.code, codes::COMMAND_UNAVAILABLE);
        assert!(err.message.contains("tpt is not installed"), "{err}");
    }
}
