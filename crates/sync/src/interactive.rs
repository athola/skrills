//! The decision gate between reading a sync's items and writing them.
//!
//! An interactive sync runs the phases twice. The first pass only collects:
//! it asks the [`ItemDecider`] about every new or changed item and writes
//! nothing, so cancelling leaves the target untouched. The second pass is the
//! normal run, with each answer looked up instead of asked.
//!
//! A rejected item is never simply dropped from what the writer sees. Writers
//! sweep what is absent from their input (stale copies, `.skrills-synced`
//! markers), so a dropped changed item would be deleted from the target.
//! Rejecting a changed item substitutes the target's own copy; only a new
//! item, which has no target copy to lose, is dropped.

use crate::common::{Command, McpServer};
use crate::conflict::Ledger;
use crate::decision::{
    ChangeKind, ConflictSides, Decision, DecisionScope, ItemChange, ItemDecider, ItemKind,
};
use crate::orchestrator::{existing_key, is_unchanged};
use crate::report::{SkipReason, WriteReport};
use crate::text_diff::unified_diff;
use crate::Result;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// Context lines around each change in a shown diff.
const DIFF_CONTEXT: usize = 3;

/// Recorded answers, keyed by kind and item name.
pub(crate) type DecisionBook = RefCell<HashMap<(ItemKind, String), Decision>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    /// First pass: ask and record, write nothing.
    Collect,
    /// Second pass: write, using the recorded answers.
    Apply,
}

pub(crate) struct Gate<'a> {
    mode: Mode,
    decider: Option<&'a dyn ItemDecider>,
    book: Option<&'a DecisionBook>,
}

/// What the writer is to be given, and what was turned down.
pub(crate) struct Resolved {
    pub items: Vec<Command>,
    /// Names the person rejected, for the report.
    pub declined: Vec<String>,
    /// Set when a whole category was rejected: the writer must not run.
    pub skip_write: bool,
}

impl<'a> Gate<'a> {
    pub fn off() -> Self {
        Self {
            mode: Mode::Off,
            decider: None,
            book: None,
        }
    }

    pub fn collect(decider: &'a dyn ItemDecider, book: &'a DecisionBook) -> Self {
        Self {
            mode: Mode::Collect,
            decider: Some(decider),
            book: Some(book),
        }
    }

    pub fn apply(book: &'a DecisionBook) -> Self {
        Self {
            mode: Mode::Apply,
            decider: None,
            book: Some(book),
        }
    }

    pub fn is_off(&self) -> bool {
        self.mode == Mode::Off
    }

    pub fn is_collecting(&self) -> bool {
        self.mode == Mode::Collect
    }

    /// Whether the person turned down this item in the collecting pass.
    pub fn rejected(&self, kind: ItemKind, name: &str) -> bool {
        self.book.is_some_and(|book| {
            book.borrow().get(&(kind, name.to_string())) == Some(&Decision::Reject)
        })
    }

    /// What the person answered for `name` in the collecting pass, whether
    /// they were asked about the item or about its whole category.
    pub fn answered(
        &self,
        kind: ItemKind,
        name: &str,
    ) -> Option<(Decision, crate::conflict::Scope)> {
        use crate::conflict::Scope;
        let book = self.book?.borrow();
        book.get(&(kind, name.to_string()))
            .map(|decision| (*decision, Scope::Item))
            .or_else(|| {
                book.get(&(kind, kind.plural().to_string()))
                    .map(|decision| (*decision, Scope::Category))
            })
    }

    /// Asks in the collecting pass; looks the answer up in the applying one.
    /// An item the first pass never asked about is rejected: it appeared
    /// between the passes, and nobody agreed to write it.
    fn answer(&self, change: ItemChange) -> Result<Decision> {
        let key = (change.kind, change.name.clone());
        match (self.decider, self.book) {
            (Some(decider), Some(book)) => {
                let decision = decider.decide(&change).map_err(|e| {
                    // Any failure to ask stops the run, and the first pass has
                    // written nothing.
                    if e.downcast_ref::<crate::SyncError>().is_some() {
                        e
                    } else {
                        e.context(crate::SyncError::InteractiveAborted)
                    }
                })?;
                book.borrow_mut().insert(key, decision);
                Ok(decision)
            }
            (None, Some(book)) => Ok(book.borrow().get(&key).copied().unwrap_or(Decision::Reject)),
            _ => Ok(Decision::Accept),
        }
    }

    /// Per-item decisions over file artifacts, matched to the target by `key`.
    pub fn resolve_files(
        &self,
        kind: ItemKind,
        items: Vec<Command>,
        target: &[Command],
        key: impl Fn(&Command) -> String,
        ledger: &Ledger,
    ) -> Result<Resolved> {
        // A plugin-cache copy is not where the writer puts a flat file:
        // matching it would substitute a copy the writer would then create.
        let by_key: HashMap<String, &Command> = target
            .iter()
            .filter(|t| t.plugin_origin.is_none())
            .map(|t| (key(t), t))
            .collect();
        let mut resolved = Resolved {
            items: Vec::with_capacity(items.len()),
            declined: Vec::new(),
            skip_write: false,
        };
        for item in items {
            let existing = by_key.get(&key(&item)).copied();
            let change = match existing {
                None => ChangeKind::New,
                Some(t) if is_unchanged(t, &item) => {
                    resolved.items.push(item);
                    continue;
                }
                Some(_) => ChangeKind::Changed,
            };
            let conflict = ledger
                .file_conflict(kind, &item, existing)
                .then(|| ConflictSides {
                    target: existing.map(|t| lossy(&t.content)).unwrap_or_default(),
                    source: lossy(&item.content),
                });
            let request = ItemChange {
                kind,
                name: item.name.clone(),
                change,
                scope: DecisionScope::PerItem,
                diff: file_diff(existing, &item),
                conflict,
                conflicting_items: 0,
            };
            match (self.answer(request)?, existing) {
                (Decision::Accept, _) => resolved.items.push(item),
                (Decision::Reject, Some(current)) => {
                    resolved.declined.push(item.name.clone());
                    resolved.items.push(current.clone());
                }
                (Decision::Reject, None) => resolved.declined.push(item.name.clone()),
            }
        }
        Ok(resolved)
    }

    /// One question for a whole category. `true` means write it.
    ///
    /// `conflicting` is how many of the category's items both sides changed
    /// since the last sync, so the prompt can say so.
    pub fn accept_category(
        &self,
        kind: ItemKind,
        names: &[String],
        conflicting: usize,
    ) -> Result<bool> {
        let diff = if names.is_empty() {
            "(the target rewrites these files when it writes them)".to_string()
        } else {
            names.join("\n")
        };
        let request = ItemChange {
            kind,
            name: kind.plural().to_string(),
            change: ChangeKind::Changed,
            scope: DecisionScope::WholeCategory,
            diff,
            conflict: None,
            conflicting_items: conflicting,
        };
        Ok(self.answer(request)? == Decision::Accept)
    }

    /// Decisions over MCP servers, one per server or one for all of them.
    pub fn resolve_servers(
        &self,
        scope: DecisionScope,
        servers: HashMap<String, McpServer>,
        target: &HashMap<String, McpServer>,
        ledger: &Ledger,
    ) -> Result<(HashMap<String, McpServer>, Vec<String>)> {
        let mut changed: Vec<&String> = servers
            .iter()
            .filter(|(name, server)| target.get(*name) != Some(*server))
            .map(|(name, _)| name)
            .collect();
        changed.sort();
        if changed.is_empty() {
            return Ok((servers, Vec::new()));
        }
        if scope == DecisionScope::WholeCategory {
            let names: Vec<String> = changed.into_iter().cloned().collect();
            let conflicting = names
                .iter()
                .filter(|name| ledger.server_conflict(name, &servers[*name], target.get(*name)))
                .count();
            if self.accept_category(ItemKind::McpServer, &names, conflicting)? {
                return Ok((servers, Vec::new()));
            }
            return Ok((HashMap::new(), names));
        }
        let mut kept = servers.clone();
        let mut declined = Vec::new();
        for name in changed {
            let existing = target.get(name);
            let conflict = ledger
                .server_conflict(name, &servers[name], existing)
                .then(|| ConflictSides {
                    target: existing.map(server_text).unwrap_or_default(),
                    source: server_text(&servers[name]),
                });
            let request = ItemChange {
                kind: ItemKind::McpServer,
                name: name.clone(),
                change: if existing.is_some() {
                    ChangeKind::Changed
                } else {
                    ChangeKind::New
                },
                scope: DecisionScope::PerItem,
                diff: unified_diff(
                    &existing.map(server_text).unwrap_or_default(),
                    &server_text(&servers[name]),
                    DIFF_CONTEXT,
                ),
                conflict,
                conflicting_items: 0,
            };
            if self.answer(request)? == Decision::Reject {
                declined.push(name.clone());
                match existing {
                    Some(current) => {
                        kept.insert(name.clone(), current.clone());
                    }
                    None => {
                        kept.remove(name);
                    }
                }
            }
        }
        Ok((kept, declined))
    }

    /// Whether the `model` preference may be written.
    pub fn accept_model(&self, current: Option<&str>, incoming: &str) -> Result<bool> {
        if current == Some(incoming) {
            return Ok(true);
        }
        let request = ItemChange {
            kind: ItemKind::Preferences,
            name: "model".to_string(),
            change: if current.is_some() {
                ChangeKind::Changed
            } else {
                ChangeKind::New
            },
            scope: DecisionScope::PerItem,
            diff: unified_diff(current.unwrap_or(""), incoming, DIFF_CONTEXT),
            conflict: None,
            conflicting_items: 0,
        };
        Ok(self.answer(request)? == Decision::Accept)
    }

    /// Turns the writer's report into the run's: a rejected item is not a
    /// write, and says so.
    pub fn annotate(&self, mut report: WriteReport, declined: &[String]) -> WriteReport {
        report.skipped.retain(
            |reason| !matches!(reason, SkipReason::Unchanged { item } if declined.contains(item)),
        );
        report.skipped.extend(
            declined
                .iter()
                .map(|item| SkipReason::Declined { item: item.clone() }),
        );
        report
    }
}

/// Names of the items the target does not already hold byte for byte.
pub(crate) fn changed_names(
    items: &[Command],
    target: &[Command],
    key: impl Fn(&Command) -> String,
) -> Vec<String> {
    let by_key: HashMap<String, &Command> = target.iter().map(|t| (key(t), t)).collect();
    items
        .iter()
        .filter(|item| {
            !by_key
                .get(&key(item))
                .is_some_and(|t| is_unchanged(t, item))
        })
        .map(|item| item.name.clone())
        .collect()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The target-to-source diff of one single-file item. Items with modules
/// (skills) are never asked about one at a time, so only the main file shows.
fn file_diff(existing: Option<&Command>, item: &Command) -> String {
    let diff = unified_diff(
        &existing.map(|t| lossy(&t.content)).unwrap_or_default(),
        &lossy(&item.content),
        DIFF_CONTEXT,
    );
    if diff.is_empty() {
        "(bytes differ; no text change to show)".to_string()
    } else {
        diff
    }
}

/// An MCP server as text for diffing. Environment and header values are
/// shown as a short hash, so a changed secret shows as changed without
/// being printed.
fn server_text(server: &McpServer) -> String {
    let hashed = |map: &HashMap<String, String>| -> BTreeMap<String, String> {
        map.iter()
            .map(|(name, value)| {
                let digest = Sha256::digest(value.as_bytes());
                (
                    name.clone(),
                    format!("sha256:{}", hex::encode(&digest[..4])),
                )
            })
            .collect()
    };
    let mut shown = server.clone();
    shown.env = hashed(&server.env).into_iter().collect();
    shown.headers = server
        .headers
        .as_ref()
        .map(|h| hashed(h).into_iter().collect());
    serde_json::to_string_pretty(&shown).unwrap_or_else(|_| format!("{shown:?}"))
}

/// Key a target's items the way a writer names them.
pub(crate) fn file_key(kind: ItemKind, item: &Command) -> String {
    match kind {
        ItemKind::Skill => existing_key(&crate::adapters::utils::skill_folder_name(item)),
        _ => existing_key(&item.name),
    }
}
