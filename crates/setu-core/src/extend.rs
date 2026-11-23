//! Extension points: hooks the app calls at a few important moments.
//!
//! Setu works the same with no extension installed: the defaults record
//! nothing and allow every command. Another build of the app (a company's
//! own, or the Enterprise edition) can pass its own [`Extensions`] when it
//! starts, to keep an audit trail or to refuse commands a team has ruled out.
//!
//! The hooks get only what they need. An [`AuditEvent`] never carries a
//! password, key, passphrase or command output. A [`CommandCheck`] sees the
//! command text, because that is what a policy decides on; an implementation
//! must not log it unmasked.

use std::fmt;
use std::sync::Arc;

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

/// The default policy: every command may run.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

impl CommandPolicy for AllowAll {
    fn check(&self, _command: &CommandCheck<'_>) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

/// The hooks a build of the app starts with. `Extensions::default()` is the
/// open-source behaviour: nothing recorded, nothing blocked.
#[derive(Debug, Clone)]
pub struct Extensions {
    audit: Arc<dyn AuditSink>,
    policy: Arc<dyn CommandPolicy>,
}

impl Default for Extensions {
    fn default() -> Self {
        Extensions {
            audit: Arc::new(NoAudit),
            policy: Arc::new(AllowAll),
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
    fn names_are_stable() {
        assert_eq!(AuditAction::SshConnect.as_str(), "ssh.connect");
        assert_eq!(AuditOutcome::Blocked.as_str(), "blocked");
    }
}
