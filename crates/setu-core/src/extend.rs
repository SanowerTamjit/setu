//! Extension points: hooks the app calls at a few important moments.
//!
//! Setu works the same with no extension installed: the defaults record
//! nothing and allow every command. Another build of the app (a company's
//! own, or the Enterprise edition) can pass its own [`Extensions`] when it
//! starts, to keep an audit trail or to refuse commands a team has ruled out.
//!
//! A build can also record SSH terminal sessions ([`SessionRecorder`]) so
//! they can be replayed for an audit. The app then tells the person in the
//! terminal ([`RECORDING_NOTICE`]) before anything is recorded. A recorder
//! sees what the server sends (the screen), never what the person types,
//! so a password typed at a prompt that does not echo it is not recorded.
//!
//! A build can also add a settings page of its own ([`SettingsPanel`]): the
//! app draws it from plain data (rows, lines, buttons), so the extension
//! never runs code in the web view.
//!
//! The hooks get only what they need. An [`AuditEvent`] never carries a
//! password, key, passphrase or command output. A [`CommandCheck`] sees the
//! command text, because that is what a policy decides on; an implementation
//! must not log it unmasked.

use std::fmt;
use std::sync::{Arc, Mutex};

/// What happened, for an audit trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AuditAction {
    /// An SSH login from a terminal tab, successful or not.
    SshConnect,
    /// A file or folder copied over SFTP.
    FileTransfer,
    /// A workflow run that finished (succeeded, failed or was stopped).
    WorkflowRun,
    /// A command that a [`CommandPolicy`] refused to run.
    CommandBlocked,
}

impl AuditAction {
    /// A stable, lowercase name, for files and exports.
    pub fn as_str(self) -> &'static str {
        match self {
            AuditAction::SshConnect => "ssh.connect",
            AuditAction::FileTransfer => "file.transfer",
            AuditAction::WorkflowRun => "workflow.run",
            AuditAction::CommandBlocked => "command.blocked",
        }
    }
}

/// How it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AuditOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Blocked,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Succeeded => "succeeded",
            AuditOutcome::Failed => "failed",
            AuditOutcome::Cancelled => "cancelled",
            AuditOutcome::Blocked => "blocked",
        }
    }
}

/// One thing worth recording. Holds no secrets and no command output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub action: AuditAction,
    pub outcome: AuditOutcome,
    /// Where: `user@host:port` for SSH and files, the workflow id for runs.
    pub target: String,
    /// A short, secret-free description: the file copied and its direction,
    /// the workflow's name, the policy's reason.
    pub detail: String,
}

/// Receives audit events. Called on the thread that did the work, so it
/// must return quickly (queue the event, do slow I/O elsewhere).
pub trait AuditSink: Send + Sync + fmt::Debug {
    fn record(&self, event: &AuditEvent);
}

/// The default sink: records nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAudit;

impl AuditSink for NoAudit {
    fn record(&self, _event: &AuditEvent) {}
}

/// Where a command came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CommandOrigin<'a> {
    /// A command step of a workflow run.
    Workflow { workflow: &'a str, step: &'a str },
}

/// A command Setu is about to run on a host without a person typing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandCheck<'a> {
    /// The saved host's name.
    pub host: &'a str,
    pub command: &'a str,
    pub origin: CommandOrigin<'a>,
}

/// A policy's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    /// Do not run it. The reason is shown to the person and recorded, so it
    /// must not contain secrets (name the rule, not the command).
    Deny {
        reason: String,
    },
}

/// Decides whether a command may run.
pub trait CommandPolicy: Send + Sync + fmt::Debug {
    fn check(&self, command: &CommandCheck<'_>) -> PolicyDecision;
}

/// The default policy: every command may run, every login may happen.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

impl CommandPolicy for AllowAll {
    fn check(&self, _command: &CommandCheck<'_>) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

impl AccessPolicy for AllowAll {
    fn check_connect(&self, _login: &ConnectCheck<'_>) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

/// An SSH login Setu is about to make, from a terminal tab or a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectCheck<'a> {
    /// The saved host's name, or the address when the host is not saved.
    pub name: &'a str,
    /// The address connected to (DNS name or IP).
    pub address: &'a str,
    pub port: u16,
    /// The remote user name.
    pub login: &'a str,
    /// The saved host's tags (empty when not saved).
    pub tags: &'a [String],
}

/// Decides whether a login may happen, before Setu connects. It may take
/// a moment (it can ask a server): the app calls it off the main thread.
pub trait AccessPolicy: Send + Sync + fmt::Debug {
    fn check_connect(&self, login: &ConnectCheck<'_>) -> PolicyDecision;
}

/// What the app writes at the top of a terminal it records, before any
/// output from the server, so the person always knows.
pub const RECORDING_NOTICE: &str =
    "\x1b[2m[Setu] This session is recorded by your organization.\x1b[0m\r\n";

/// An SSH shell that just opened, for a [`SessionRecorder`] to decide on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStart<'a> {
    /// The saved host's name, or the address when the host is not saved.
    pub name: &'a str,
    pub address: &'a str,
    pub port: u16,
    pub login: &'a str,
    /// The saved host's tags (empty when not saved).
    pub tags: &'a [String],
    pub cols: u16,
    pub rows: u16,
}

/// Decides which sessions are recorded, and starts a [`SessionTape`] for
/// each. Called once per shell, on an async task: return quickly (open a
/// file, do not ask a server).
pub trait SessionRecorder: Send + Sync + fmt::Debug {
    /// `None` leaves this session unrecorded.
    fn start(&self, session: &SessionStart<'_>) -> Option<Box<dyn SessionTape>>;
}

/// One recording. Its methods are called on the thread that reads the
/// session, so they must return quickly (buffer, write to a local file).
pub trait SessionTape: Send + fmt::Debug {
    /// Bytes the server sent, exactly as the terminal got them (a
    /// multi-byte character can be split across two calls).
    fn output(&mut self, bytes: &[u8]);
    /// The terminal changed size.
    fn resize(&mut self, cols: u16, rows: u16);
    /// The session ended: `Some(code)` when the shell exited with one.
    /// Called once; nothing is sent after it.
    fn finish(&mut self, exit_code: Option<u32>);
}

/// The default recorder: records nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRecording;

impl SessionRecorder for NoRecording {
    fn start(&self, _session: &SessionStart<'_>) -> Option<Box<dyn SessionTape>> {
        None
    }
}

/// A recording under way, shared by a session's reader (output), its tab
/// (resize, close) and its exit. After [`Recording::finish`] every call
/// does nothing, so finishing twice (a closed tab whose shell then exits)
/// is safe.
#[derive(Debug, Clone)]
pub struct Recording(Arc<Mutex<Option<Box<dyn SessionTape>>>>);

impl Recording {
    fn with(&self, f: impl FnOnce(&mut dyn SessionTape)) {
        let mut tape = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = tape.as_mut() {
            f(t.as_mut());
        }
    }

    pub fn output(&self, bytes: &[u8]) {
        self.with(|t| t.output(bytes));
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.with(|t| t.resize(cols, rows));
    }

    pub fn finish(&self, exit_code: Option<u32>) {
        let tape = self.0.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(mut t) = tape {
            t.finish(exit_code);
        }
    }
}

/// How a [`PanelSection`] is doing, shown as a coloured badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelStatus {
    Ok,
    Warning,
    Error,
}

/// A button on a panel. Pressing it calls [`SettingsPanel::action`] with `id`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PanelAction {
    pub id: String,
    pub label: String,
}

/// One block of a settings panel: a heading, label/value rows, optional
/// monospace lines (a log tail, say) and buttons. All plain text.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct PanelSection {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PanelStatus>,
    /// A sentence under the heading.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub rows: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PanelAction>,
}

/// What pressing a panel button did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Show this sentence.
    Message(String),
    /// Save `contents` as a new file named `file_name` in the person's
    /// Downloads folder (the app picks the folder and never overwrites).
    Save { file_name: String, contents: String },
    /// Open `url` in the person's web browser (a sign-in page, say) and
    /// show `message`. The app opens only `https://` addresses, or
    /// `http://` to this computer (127.0.0.1, localhost), and refuses
    /// anything else.
    OpenUrl { url: String, message: String },
}

/// A settings page a build adds, such as an organisation's licence, audit
/// log and policy.
pub trait SettingsPanel: Send + Sync + fmt::Debug {
    /// The page's name in Settings, e.g. "Organization".
    fn title(&self) -> String;
    /// The edition the build runs as, e.g. "Enterprise", for the title bar
    /// and status bar. `None` keeps the plain product name.
    fn edition(&self) -> Option<String>;
    /// The page's contents, read each time it is shown.
    fn sections(&self) -> Vec<PanelSection>;
    /// Runs the button `id`. `Err` is shown to the person.
    fn action(&self, id: &str) -> Result<ActionOutcome, String>;
}

/// The hooks a build of the app starts with. `Extensions::default()` is the
/// open-source behaviour: nothing recorded, nothing blocked.
#[derive(Debug, Clone)]
pub struct Extensions {
    audit: Arc<dyn AuditSink>,
    policy: Arc<dyn CommandPolicy>,
    access: Arc<dyn AccessPolicy>,
    recorder: Arc<dyn SessionRecorder>,
    panel: Option<Arc<dyn SettingsPanel>>,
}

impl Default for Extensions {
    fn default() -> Self {
        Extensions {
            audit: Arc::new(NoAudit),
            policy: Arc::new(AllowAll),
            access: Arc::new(AllowAll),
            recorder: Arc::new(NoRecording),
            panel: None,
        }
    }
}

impl Extensions {
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    pub fn with_policy(mut self, policy: Arc<dyn CommandPolicy>) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_access(mut self, access: Arc<dyn AccessPolicy>) -> Self {
        self.access = access;
        self
    }

    pub fn with_recorder(mut self, recorder: Arc<dyn SessionRecorder>) -> Self {
        self.recorder = recorder;
        self
    }

    /// Asks the recorder whether to record this session. When it says
    /// yes, show [`RECORDING_NOTICE`] in the terminal before any output.
    pub fn start_recording(&self, session: &SessionStart<'_>) -> Option<Recording> {
        self.recorder
            .start(session)
            .map(|tape| Recording(Arc::new(Mutex::new(Some(tape)))))
    }

    pub fn with_panel(mut self, panel: Arc<dyn SettingsPanel>) -> Self {
        self.panel = Some(panel);
        self
    }

    /// The build's own settings page, if it has one.
    pub fn panel(&self) -> Option<&Arc<dyn SettingsPanel>> {
        self.panel.as_ref()
    }

    /// The edition name the build runs as, if it says.
    pub fn edition(&self) -> Option<String> {
        self.panel.as_ref().and_then(|p| p.edition())
    }

    pub fn audit(&self, event: AuditEvent) {
        self.audit.record(&event);
    }

    /// Asks the policy about `command`. A refusal is recorded in the audit
    /// trail (with the reason, not the command) before it is returned.
    pub fn check_command(&self, command: &CommandCheck<'_>) -> PolicyDecision {
        let decision = self.policy.check(command);
        if let PolicyDecision::Deny { reason } = &decision {
            self.audit(AuditEvent {
                action: AuditAction::CommandBlocked,
                outcome: AuditOutcome::Blocked,
                target: command.host.to_string(),
                detail: match command.origin {
                    CommandOrigin::Workflow { workflow, step } => {
                        format!("workflow {workflow}, step {step}: {reason}")
                    }
                },
            });
        }
        decision
    }

    /// Asks the access policy about a login. A refusal is recorded in the
    /// audit trail as a blocked `ssh.connect` before it is returned.
    pub fn check_connect(&self, login: &ConnectCheck<'_>) -> PolicyDecision {
        let decision = self.access.check_connect(login);
        if let PolicyDecision::Deny { reason } = &decision {
            self.audit(AuditEvent {
                action: AuditAction::SshConnect,
                outcome: AuditOutcome::Blocked,
                target: format!("{}@{}:{}", login.login, login.address, login.port),
                detail: format!("not allowed: {reason}"),
            });
        }
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct Recorder(Mutex<Vec<AuditEvent>>);

    impl AuditSink for Recorder {
        fn record(&self, event: &AuditEvent) {
            self.0.lock().expect("lock").push(event.clone());
        }
    }

    #[derive(Debug)]
    struct NoReboot;

    impl CommandPolicy for NoReboot {
        fn check(&self, c: &CommandCheck<'_>) -> PolicyDecision {
            if c.command.contains("reboot") {
                PolicyDecision::Deny {
                    reason: "rule `no-reboot`".into(),
                }
            } else {
                PolicyDecision::Allow
            }
        }
    }

    fn check(command: &str) -> CommandCheck<'_> {
        CommandCheck {
            host: "web1",
            command,
            origin: CommandOrigin::Workflow {
                workflow: "deploy",
                step: "restart",
            },
        }
    }

    #[derive(Debug)]
    struct NoRoot;

    impl AccessPolicy for NoRoot {
        fn check_connect(&self, c: &ConnectCheck<'_>) -> PolicyDecision {
            if c.login == "root" {
                PolicyDecision::Deny {
                    reason: "no access rule of your teams covers this host and login".into(),
                }
            } else {
                PolicyDecision::Allow
            }
        }
    }

    #[test]
    fn a_refused_login_is_audited() {
        let rec = Arc::new(Recorder::default());
        let ext = Extensions::default()
            .with_audit(rec.clone())
            .with_access(Arc::new(NoRoot));
        let tags = vec!["web".to_string()];
        let login = |login| ConnectCheck {
            name: "web1",
            address: "10.0.0.5",
            port: 22,
            login,
            tags: &tags,
        };
        assert_eq!(ext.check_connect(&login("deploy")), PolicyDecision::Allow);
        assert!(matches!(
            ext.check_connect(&login("root")),
            PolicyDecision::Deny { .. }
        ));
        let events = rec.0.lock().expect("lock").clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, AuditAction::SshConnect);
        assert_eq!(events[0].outcome, AuditOutcome::Blocked);
        assert_eq!(events[0].target, "root@10.0.0.5:22");
        // The open-source default lets every login through.
        assert_eq!(
            Extensions::default().check_connect(&login("root")),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn defaults_allow_everything() {
        let ext = Extensions::default();
        assert_eq!(
            ext.check_command(&check("sudo reboot")),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn a_refusal_is_audited_without_the_command() {
        let rec = Arc::new(Recorder::default());
        let ext = Extensions::default()
            .with_audit(rec.clone())
            .with_policy(Arc::new(NoReboot));
        assert_eq!(ext.check_command(&check("uptime")), PolicyDecision::Allow);
        assert!(matches!(
            ext.check_command(&check("sudo reboot --password=hunter2")),
            PolicyDecision::Deny { .. }
        ));
        let events = rec.0.lock().expect("lock").clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, AuditAction::CommandBlocked);
        assert_eq!(events[0].target, "web1");
        assert_eq!(
            events[0].detail,
            "workflow deploy, step restart: rule `no-reboot`"
        );
        assert!(!events[0].detail.contains("hunter2"));
    }

    #[test]
    fn no_panel_by_default() {
        let ext = Extensions::default();
        assert!(ext.panel().is_none());
        assert_eq!(ext.edition(), None);
    }

    #[test]
    fn names_are_stable() {
        assert_eq!(AuditAction::SshConnect.as_str(), "ssh.connect");
        assert_eq!(AuditOutcome::Blocked.as_str(), "blocked");
    }

    #[derive(Debug, Default)]
    struct Log(Arc<Mutex<Vec<String>>>);

    impl SessionTape for Log {
        fn output(&mut self, bytes: &[u8]) {
            self.0
                .lock()
                .expect("lock")
                .push(format!("o {}", String::from_utf8_lossy(bytes)));
        }
        fn resize(&mut self, cols: u16, rows: u16) {
            self.0
                .lock()
                .expect("lock")
                .push(format!("r {cols}x{rows}"));
        }
        fn finish(&mut self, exit_code: Option<u32>) {
            self.0
                .lock()
                .expect("lock")
                .push(format!("end {exit_code:?}"));
        }
    }

    #[derive(Debug, Default)]
    struct ProdOnly(Arc<Mutex<Vec<String>>>);

    impl SessionRecorder for ProdOnly {
        fn start(&self, s: &SessionStart<'_>) -> Option<Box<dyn SessionTape>> {
            s.tags
                .iter()
                .any(|t| t == "prod")
                .then(|| Box::new(Log(Arc::clone(&self.0))) as Box<dyn SessionTape>)
        }
    }

    fn start(tags: &[String]) -> SessionStart<'_> {
        SessionStart {
            name: "db1",
            address: "10.0.0.7",
            port: 22,
            login: "ops",
            tags,
            cols: 80,
            rows: 24,
        }
    }

    #[test]
    fn nothing_is_recorded_by_default() {
        let prod = vec!["prod".to_string()];
        assert!(Extensions::default()
            .start_recording(&start(&prod))
            .is_none());
    }

    #[test]
    fn a_recording_ends_once_and_then_ignores_everything() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let ext = Extensions::default().with_recorder(Arc::new(ProdOnly(Arc::clone(&log))));
        assert!(ext.start_recording(&start(&[])).is_none());
        let prod = vec!["prod".to_string()];
        let rec = ext.start_recording(&start(&prod)).expect("recorded");
        let tab = rec.clone();
        rec.output(b"$ uptime\r\n");
        tab.resize(120, 40);
        tab.finish(None);
        // The shell's own exit comes after the tab closed it.
        rec.finish(Some(0));
        rec.output(b"late");
        assert_eq!(
            *log.lock().expect("lock"),
            vec!["o $ uptime\r\n", "r 120x40", "end None"]
        );
    }

    #[test]
    fn the_notice_is_one_line_that_resets_its_style() {
        assert!(RECORDING_NOTICE.ends_with("\x1b[0m\r\n"));
        assert_eq!(RECORDING_NOTICE.matches('\n').count(), 1);
        assert!(RECORDING_NOTICE.contains("recorded"));
    }
}
