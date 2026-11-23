//! Workflows on disk (`workflows/<id>.toml`) and run history
//! (`workflow-runs.json`), both in Setu's config folder.
//!
//! Files are written atomically (temp file + rename) and readable only by
//! you: workflows hold commands and host names, history holds command output.

use crate::board::{Board, BOARD_VERSION};
use crate::engine::{tail, RunRecord};
use crate::error::{invalid, WorkflowError};
use crate::model::{Workflow, FORMAT_VERSION};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Workflow files bigger than this are not opened.
pub const MAX_FILE: u64 = 512 * 1024;
/// Runs kept in history, newest first.
pub const MAX_RUNS: usize = 100;
/// Output kept per host in history (the live view shows more).
pub const HISTORY_OUTPUT: usize = 8 * 1024;
/// At most this many workflow files are listed.
pub const MAX_WORKFLOWS: usize = 500;

#[derive(Debug, Clone)]
pub struct WorkflowStore {
    dir: PathBuf,
    history: PathBuf,
    board: PathBuf,
}

const HEADER: &str =
    "# A Setu workflow. Edit it in Setu (Workflows) or by hand; see docs/workflows.md.\n";

/// Writes `data` to `path` through a temp file, 0600 on Unix.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), WorkflowError> {
    let err = |source| WorkflowError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent() {
        create_private_dir(dir).map_err(err)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(err)
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

fn read_capped(path: &Path) -> Result<String, WorkflowError> {
    let read_err = |source| WorkflowError::Read {
        path: path.to_path_buf(),
        source,
    };
    let meta = std::fs::metadata(path).map_err(read_err)?;
    if meta.len() > MAX_FILE {
        return Err(WorkflowError::Parse {
            path: path.to_path_buf(),
            message: format!("larger than {} KiB", MAX_FILE / 1024),
        });
    }
    std::fs::read_to_string(path).map_err(read_err)
}

/// Reads a workflow from TOML text and checks it.
pub fn parse(text: &str, path: &Path) -> Result<Workflow, WorkflowError> {
    let workflow: Workflow = toml::from_str(text).map_err(|e| WorkflowError::Parse {
        path: path.to_path_buf(),
        message: e.message().to_string(),
    })?;
    if workflow.version > FORMAT_VERSION {
        return Err(WorkflowError::NewerFormat {
            path: path.to_path_buf(),
            found: workflow.version,
        });
    }
    workflow.validate().map_err(|e| WorkflowError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(workflow)
}

/// A file that could not be opened, listed so the person can fix it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Broken {
    pub file: String,
    pub error: String,
}

impl WorkflowStore {
    pub fn in_dir(config_dir: &Path) -> Self {
        WorkflowStore {
            dir: config_dir.join("workflows"),
            history: config_dir.join("workflow-runs.json"),
            board: config_dir.join("board.toml"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_of(&self, id: &str) -> Result<PathBuf, WorkflowError> {
        // validate() already checks ids, but this is the gate to the file system.
        if id.is_empty()
            || id.len() > 64
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(invalid("not a valid workflow id"));
        }
        Ok(self.dir.join(format!("{id}.toml")))
    }

    /// Every workflow in the folder, by name, plus files that did not parse.
    pub fn list(&self) -> Result<(Vec<Workflow>, Vec<Broken>), WorkflowError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Vec::new(), Vec::new()))
            }
            Err(source) => {
                return Err(WorkflowError::Read {
                    path: self.dir.clone(),
                    source,
                })
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml") && p.is_file())
            .collect();
        paths.sort();
        paths.truncate(MAX_WORKFLOWS);
        let mut good = Vec::new();
        let mut broken = Vec::new();
        for path in paths {
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match read_capped(&path).and_then(|t| parse(&t, &path)) {
                // The file name wins, so saving writes back to the same file.
                Ok(mut w) => {
                    if let Some(stem) = path.file_stem() {
                        w.id = stem.to_string_lossy().into_owned();
                    }
                    if w.validate().is_ok() {
                        good.push(w);
                    } else {
                        broken.push(Broken {
                            file,
                            error: "the file name is not a valid workflow id".into(),
                        });
                    }
                }
                Err(e) => broken.push(Broken {
                    file,
                    error: e.to_string(),
                }),
            }
        }
        good.sort_by_key(|w| w.name.to_lowercase());
        Ok((good, broken))
    }

    pub fn load(&self, id: &str) -> Result<Workflow, WorkflowError> {
        let path = self.path_of(id)?;
        if !path.is_file() {
            return Err(WorkflowError::NotFound(id.into()));
        }
        let mut w = parse(&read_capped(&path)?, &path)?;
        w.id = id.into();
        Ok(w)
    }

    /// Saves `workflow` after checking it. With `previous` set to another id,
    /// it is a rename: the old file goes once the new one is written.
    pub fn save(&self, workflow: &Workflow, previous: Option<&str>) -> Result<(), WorkflowError> {
        workflow.validate()?;
        let path = self.path_of(&workflow.id)?;
        let renaming = previous.is_some_and(|p| p != workflow.id);
        if renaming && path.exists() {
            return Err(invalid(format!(
                "a workflow with the id `{}` already exists",
                workflow.id
            )));
        }
        let mut w = workflow.clone();
        w.version = FORMAT_VERSION;
        let body = toml::to_string(&w).map_err(|e| invalid(e.to_string()))?;
        write_atomic(&path, format!("{HEADER}{body}").as_bytes())?;
        if let (true, Some(old)) = (renaming, previous) {
            let old = self.path_of(old)?;
            let _ = std::fs::remove_file(old);
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), WorkflowError> {
        let path = self.path_of(id)?;
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(WorkflowError::NotFound(id.into()))
            }
            Err(source) => Err(WorkflowError::Write { path, source }),
            Ok(()) => Ok(()),
        }
    }

    /// Past runs, newest first. A damaged history file reads as empty.
    pub fn history(&self) -> Vec<RunRecord> {
        let Ok(text) = read_capped_history(&self.history) else {
            return Vec::new();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Adds a finished run at the top, trimming output and old runs.
    pub fn record(&self, run: &RunRecord) -> Result<(), WorkflowError> {
        let mut runs = self.history();
        runs.retain(|r| r.id != run.id);
        let mut run = run.clone();
        for step in &mut run.steps {
            for host in &mut step.hosts {
                host.output = tail(&host.output, HISTORY_OUTPUT);
            }
        }
        runs.insert(0, run);
        runs.truncate(MAX_RUNS);
        let json = serde_json::to_vec(&runs).map_err(|e| invalid(e.to_string()))?;
        write_atomic(&self.history, &json)
    }

    pub fn clear_history(&self) -> Result<(), WorkflowError> {
        match std::fs::remove_file(&self.history) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(WorkflowError::Write {
                path: self.history.clone(),
                source: e,
            }),
            _ => Ok(()),
        }
    }
}

const BOARD_HEADER: &str =
    "# Setu task board. Edit it in Setu (Workflows → Board) or by hand; see docs/workflows.md.\n";

impl WorkflowStore {
    /// The task board; a missing file is an empty board.
    pub fn load_board(&self) -> Result<Board, WorkflowError> {
        if !self.board.exists() {
            return Ok(Board::default());
        }
        let text = read_capped(&self.board)?;
        let board: Board = toml::from_str(&text).map_err(|e| WorkflowError::Parse {
            path: self.board.clone(),
            message: e.message().to_string(),
        })?;
        if board.version > BOARD_VERSION {
            return Err(WorkflowError::NewerFormat {
                path: self.board.clone(),
                found: board.version,
            });
        }
        board.validate().map_err(|e| WorkflowError::Parse {
            path: self.board.clone(),
            message: e.to_string(),
        })?;
        Ok(board)
    }

    pub fn save_board(&self, board: &Board) -> Result<(), WorkflowError> {
        board.validate()?;
        let mut b = board.clone();
        b.version = BOARD_VERSION;
        let body = toml::to_string(&b).map_err(|e| invalid(e.to_string()))?;
        write_atomic(&self.board, format!("{BOARD_HEADER}{body}").as_bytes())
    }

    pub fn board_path(&self) -> &Path {
        &self.board
    }
}

/// History can hold up to 100 runs of trimmed output; allow for that.
fn read_capped_history(path: &Path) -> std::io::Result<String> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(std::io::Error::other("history file too large"));
    }
    std::fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{HostRecord, RunStatus, StepRecord};
    use crate::model::{Link, Step, When};
    use crate::plan::State;

    fn sample(id: &str) -> Workflow {
        let mut w = Workflow::new(id, "Deploy web");
        w.steps = vec![
            Step::command("a", "Check", "df -h /", &["tag:web"]).at(40.0, 60.0),
            Step::approval("b", "Go?", "").at(300.0, 60.0),
        ];
        w.links = vec![Link::new("a", "b", When::Success)];
        w
    }

    #[test]
    fn save_list_load_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        assert!(store.list().expect("list").0.is_empty());
        store.save(&sample("deploy"), None).expect("save");
        let text = std::fs::read_to_string(dir.path().join("workflows/deploy.toml")).expect("read");
        assert!(text.starts_with("# A Setu workflow."), "{text}");
        let (list, broken) = store.list().expect("list");
        assert!(broken.is_empty());
        assert_eq!(list, vec![sample("deploy")]);
        assert_eq!(store.load("deploy").expect("load"), sample("deploy"));
        store.delete("deploy").expect("delete");
        assert!(matches!(
            store.load("deploy"),
            Err(WorkflowError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        store.save(&sample("deploy"), None).expect("save");
        let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join("workflows")), 0o700);
        assert_eq!(mode(&dir.path().join("workflows/deploy.toml")), 0o600);
    }

    #[test]
    fn rename_moves_the_file_and_refuses_to_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        store.save(&sample("one"), None).expect("save");
        store.save(&sample("two"), None).expect("save");
        assert!(store.save(&sample("two"), Some("one")).is_err());
        store.save(&sample("three"), Some("one")).expect("rename");
        let ids: Vec<_> = store
            .list()
            .expect("list")
            .0
            .into_iter()
            .map(|w| w.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"three".to_string()) && !ids.contains(&"one".to_string()));
    }

    #[test]
    fn ids_cannot_leave_the_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        for id in ["../hosts", "a/b", "..", ""] {
            assert!(store.load(id).is_err());
            assert!(store.delete(id).is_err());
        }
    }

    #[test]
    fn broken_files_are_listed_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        store.save(&sample("good"), None).expect("save");
        std::fs::write(store.dir().join("bad.toml"), "id = 3").expect("write");
        let mut looped = sample("loop");
        looped.links.push(Link::new("b", "a", When::Success));
        std::fs::write(
            store.dir().join("loop.toml"),
            toml::to_string(&looped).expect("toml"),
        )
        .expect("write");
        let (good, broken) = store.list().expect("list");
        assert_eq!(good.len(), 1);
        assert_eq!(broken.len(), 2);
        assert!(
            broken.iter().any(|b| b.error.contains("loop")),
            "{broken:?}"
        );
    }

    #[test]
    fn board_saves_privately_and_loads_back() {
        use crate::board::{Card, Column, Priority};
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        assert!(store.load_board().expect("empty").cards.is_empty());
        let board = Board {
            version: 1,
            cards: vec![Card {
                id: "c1".into(),
                title: "Rotate TLS certs".into(),
                column: Column::Doing,
                notes: "before Friday".into(),
                hosts: vec!["tag:web".into()],
                workflow: None,
                priority: Priority::High,
                due: None,
            }],
        };
        store.save_board(&board).expect("save");
        assert_eq!(store.load_board().expect("load"), board);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.board_path())
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(
            store.board_path(),
            "[[card]]\nid = \"../x\"\ntitle = \"t\"\n",
        )
        .expect("write");
        assert!(
            store.load_board().is_err(),
            "a bad file is reported, not loaded"
        );
    }

    #[test]
    fn history_keeps_newest_first_and_trims() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = WorkflowStore::in_dir(dir.path());
        let run = |id: &str| RunRecord {
            id: id.into(),
            workflow: "deploy".into(),
            workflow_name: "Deploy".into(),
            status: RunStatus::Succeeded,
            started_ms: 1,
            finished_ms: Some(2),
            steps: vec![StepRecord {
                step: "a".into(),
                name: "A".into(),
                state: State::Succeeded,
                started_ms: None,
                finished_ms: None,
                note: None,
                hosts: vec![HostRecord {
                    host: "h".into(),
                    state: State::Succeeded,
                    code: Some(0),
                    output: "x".repeat(HISTORY_OUTPUT * 2),
                    error: None,
                }],
            }],
        };
        for i in 0..(MAX_RUNS + 5) {
            store.record(&run(&format!("r{i}"))).expect("record");
        }
        let h = store.history();
        assert_eq!(h.len(), MAX_RUNS);
        assert_eq!(h[0].id, format!("r{}", MAX_RUNS + 4));
        assert!(h[0].steps[0].hosts[0].output.len() <= HISTORY_OUTPUT + 4);
        store.clear_history().expect("clear");
        assert!(store.history().is_empty());
    }
}
