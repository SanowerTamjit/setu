//! Work memory: keeps the commands typed in terminals (on this device,
//! secrets masked) and turns repeated work into snippet and workflow
//! suggestions. See `setu-memory` and docs/user/work-memory.md.
//!
//! Remembering is off until the person turns it on, and the check lives
//! here, not in the web view. Nothing is sent anywhere.

use crate::hosts::Hosts;
use crate::workflows::Workflows;
use serde::Serialize;
use setu_core::paths::{self, Os, ProcessEnv};
use setu_memory::store::Recorded;
use setu_memory::{
    suggestions, summary, to_workflow, Entry, MemoryFile, MemoryStore, Settings, Snippet,
    Suggestion,
};
use setu_workflow::Workflow;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

/// The memory file, with a lock so two commands never write it at once.
#[derive(Debug)]
pub struct Memory {
    store: Result<MemoryStore, String>,
    lock: Mutex<()>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Memory {
    pub fn from_env() -> Self {
        let store = paths::config_dir(&ProcessEnv, Os::current())
            .map(|dir| MemoryStore::in_dir(&dir))
            .map_err(|e| e.to_string());
        Memory {
            store,
            lock: Mutex::new(()),
        }
    }

    fn store(&self) -> Result<&MemoryStore, String> {
        self.store.as_ref().map_err(Clone::clone)
    }

    fn read<T>(&self, f: impl FnOnce(&MemoryFile) -> T) -> Result<T, String> {
        let store = self.store()?;
        let _guard = self.lock.lock().map_err(|_| "memory lock poisoned")?;
        let file = store.load().map_err(|e| e.to_string())?;
        Ok(f(&file))
    }

    fn update<T>(&self, f: impl FnOnce(&mut MemoryFile) -> Result<T, String>) -> Result<T, String> {
        let store = self.store()?;
        let _guard = self.lock.lock().map_err(|_| "memory lock poisoned")?;
        let mut file = store.load().map_err(|e| e.to_string())?;
        let out = f(&mut file)?;
        store.save(&file).map_err(|e| e.to_string())?;
        Ok(out)
    }
}

fn host_tags(hosts: &Hosts) -> Vec<(String, Vec<String>)> {
    hosts
        .inventory()
        .map(|inv| inv.hosts.into_iter().map(|h| (h.name, h.tags)).collect())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
pub struct MemoryView {
    settings: Settings,
    /// Newest first, at most 300.
    entries: Vec<Entry>,
    total: usize,
    snippets: Vec<Snippet>,
    suggestions: Vec<Suggestion>,
    path: String,
}

#[tauri::command]
pub fn memory_view(
    state: State<'_, Memory>,
    hosts: State<'_, Hosts>,
) -> Result<MemoryView, String> {
    let tags = host_tags(&hosts);
    let path = state.store()?.path().display().to_string();
    state.read(|m| MemoryView {
        settings: m.settings.clone(),
        entries: m.entries.iter().rev().take(300).cloned().collect(),
        total: m.entries.len(),
        snippets: m.snippets.clone(),
        suggestions: suggestions(m, &tags),
        path,
    })
}

/// Keeps one command. Returns what happened, so the UI can say nothing
/// at all when remembering is off.
#[tauri::command]
pub fn memory_record(
    state: State<'_, Memory>,
    command: String,
    host: Option<String>,
    session: String,
) -> Result<Recorded, String> {
    // Cheap check first: most calls arrive while remembering is off.
    if !state.read(|m| m.settings.recording)? {
        return Ok(Recorded::Off);
    }
    state.update(|m| {
        m.record(&command, host.as_deref(), &session, now())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn memory_settings(state: State<'_, Memory>, settings: Settings) -> Result<(), String> {
    state.update(|m| m.set_settings(settings, now()).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn memory_delete(state: State<'_, Memory>, id: u64) -> Result<(), String> {
    state.update(|m| {
        m.delete(id);
        Ok(())
    })
}

/// Forgets every command. `everything` also removes snippets and settings
/// by deleting the file.
#[tauri::command]
pub fn memory_clear(state: State<'_, Memory>, everything: bool) -> Result<(), String> {
    if everything {
        let store = state.store()?;
        let _guard = state.lock.lock().map_err(|_| "memory lock poisoned")?;
        return store.wipe().map_err(|e| e.to_string());
    }
    state.update(|m| {
        m.clear();
        Ok(())
    })
}

#[tauri::command]
pub fn memory_dismiss(state: State<'_, Memory>, id: String) -> Result<(), String> {
    state.update(|m| m.dismiss(&id).map_err(|e| e.to_string()))
}

#[tauri::command]
pub fn memory_snippet_save(
    state: State<'_, Memory>,
    title: String,
    command: String,
) -> Result<Snippet, String> {
    state.update(|m| {
        m.add_snippet(&title, &command, now())
            .map_err(|e| e.to_string())
    })
}

#[tauri::command]
pub fn memory_snippet_delete(state: State<'_, Memory>, id: String) -> Result<(), String> {
    state.update(|m| {
        m.delete_snippet(&id);
        Ok(())
    })
}

/// A workflow draft for a suggestion, found again by id here so the web
/// view cannot hand in commands of its own. It is not saved.
#[tauri::command]
pub fn memory_workflow_draft(
    state: State<'_, Memory>,
    hosts: State<'_, Hosts>,
    workflows: State<'_, Workflows>,
    id: String,
) -> Result<Workflow, String> {
    let tags = host_tags(&hosts);
    let found = state.read(|m| suggestions(m, &tags).into_iter().find(|s| s.id == id))?;
    let suggestion = found.ok_or("that suggestion is gone; refresh the list")?;
    let taken = workflows.ids();
    to_workflow(&suggestion, &tags, &taken)
}

/// A Markdown summary of routine work, to copy for a teammate or an AI
/// assistant. Masked commands are left out.
#[tauri::command]
pub fn memory_summary(state: State<'_, Memory>, hosts: State<'_, Hosts>) -> Result<String, String> {
    let tags = host_tags(&hosts);
    state.read(|m| summary(m, &tags))
}
