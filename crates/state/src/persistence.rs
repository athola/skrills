use crate::env::home_dir;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: u64,
    pub skills: Vec<String>,
}

const HISTORY_LIMIT: usize = 50;
const AUTO_PIN_WINDOW: usize = 5;
const AUTO_PIN_MIN_HITS: usize = 2;

pub fn pinned_file() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/skills-pinned.json"))
}

pub fn auto_pin_file() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/skills-autopin.json"))
}

pub fn history_file() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/skills-history.json"))
}

pub fn load_pinned() -> Result<HashSet<String>> {
    let path = pinned_file()?;
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let data = std::fs::read_to_string(path)?;
    let list: Vec<String> = serde_json::from_str(&data)?;
    Ok(list.into_iter().collect())
}

pub fn save_pinned(pinned: &HashSet<String>) -> Result<()> {
    let path = pinned_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let list: Vec<&String> = pinned.iter().collect();
    std::fs::write(path, serde_json::to_string_pretty(&list)?)?;
    Ok(())
}

pub fn load_auto_pin_flag() -> Result<bool> {
    let path = auto_pin_file()?;
    if !path.exists() {
        return Ok(false);
    }
    let data = std::fs::read_to_string(path)?;
    serde_json::from_str(&data).map_err(Into::into)
}

pub fn save_auto_pin_flag(value: bool) -> Result<()> {
    let path = auto_pin_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

pub fn load_history() -> Result<Vec<HistoryEntry>> {
    let path = history_file()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = std::fs::read_to_string(path)?;
    let mut list: Vec<HistoryEntry> = serde_json::from_str(&data)?;
    if list.len() > HISTORY_LIMIT {
        list.drain(0..list.len() - HISTORY_LIMIT);
    }
    Ok(list)
}

pub fn save_history(mut history: Vec<HistoryEntry>) -> Result<()> {
    if history.len() > HISTORY_LIMIT {
        history.drain(0..history.len() - HISTORY_LIMIT);
    }
    let path = history_file()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&history)?)?;
    Ok(())
}

pub fn auto_pin_from_history(history: &[HistoryEntry]) -> HashSet<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let window_iter = history.iter().rev().take(AUTO_PIN_WINDOW);
    for entry in window_iter {
        for skill in entry.skills.iter() {
            *counts.entry(skill.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c >= AUTO_PIN_MIN_HITS)
        .map(|(s, _)| s.to_string())
        .collect()
}

pub fn print_history(limit: usize) -> Result<()> {
    let history = load_history().unwrap_or_default();
    let mut entries: Vec<_> = history.into_iter().rev().take(limit).collect();
    if entries.is_empty() {
        println!("(no history)");
        return Ok(());
    }
    for entry in entries.drain(..) {
        println!("{} | {}", entry.ts, entry.skills.join(", "));
    }
    Ok(())
}
