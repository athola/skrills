//! The decision gate between reading a sync's items and writing them.
//!
//! An interactive sync runs the phases twice. The first pass only collects:
//! it asks the [`ItemDecider`] about every new or changed item and writes no
//! sync content, so cancelling leaves this target's items as they were. The
//! second pass is the normal run, with each answer looked up instead of
//! asked.
//!
//! Each answer is stored with a fingerprint of what the person was shown. An
//! item the second pass finds with no answer, or with a different
//! fingerprint, appeared or changed while the questions were answered;
//! nobody agreed to it, so it is treated as rejected and reported as
//! [`SkipReason::Unconfirmed`].
//!
//! A rejected item is never simply dropped from what the writer sees. Writers
//! sweep what is absent from their input (stale copies, `.skrills-synced`
//! markers), so a dropped changed item would be deleted from the target.
//! Rejecting a changed item substitutes the target's own copy; only a new
//! item, which has no target copy to lose, is dropped.

use crate::common::{Command, McpServer};
use crate::conflict::{item_hash, server_hash, Ledger};
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

/// What one answer covered. A category is its own subject, so an item that
/// happens to be named like its category (a skill called `skills`) never
/// shares an answer with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Subject {
    Item(String),
    Category,
}

/// An answer and a fingerprint of what it was given for.
#[derive(Debug, Clone)]
pub(crate) struct Answer {
    decision: Decision,
    seen: String,
}

/// Recorded answers, keyed by kind and subject.
pub(crate) type DecisionBook = RefCell<HashMap<(ItemKind, Subject), Answer>>;

pub(crate) enum Gate<'a> {
    Off,
    /// First pass: ask and record, write nothing.
    Collect {
        decider: &'a dyn ItemDecider,
        book: &'a DecisionBook,
    },
    /// Second pass: write, using the recorded answers.
    Apply {
        book: &'a DecisionBook,
    },
}

/// What the writer is to be given, and what was turned down.
pub(crate) struct Resolved {
    pub items: Vec<Command>,
    /// Names the person rejected, for the report.
    pub declined: Vec<String>,
    /// Names nobody was asked about in their current form, for the report.
    pub unconfirmed: Vec<String>,
    /// Set when a whole category was turned down: the writer must not run.
    pub skip_write: bool,
}

/// Decisions over MCP servers: the servers to hand the writer, and the names
/// rejected and unconfirmed.
pub(crate) type ResolvedServers = (HashMap<String, McpServer>, Vec<String>, Vec<String>);

impl<'a> Gate<'a> {
    pub fn off() -> Self {
        Self::Off
    }

    pub fn collect(decider: &'a dyn ItemDecider, book: &'a DecisionBook) -> Self {
        Self::Collect { decider, book }
    }

    pub fn apply(book: &'a DecisionBook) -> Self {
        Self::Apply { book }
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }

    pub fn is_collecting(&self) -> bool {
        matches!(self, Self::Collect { .. })
    }

    fn book(&self) -> Option<&'a DecisionBook> {
        match self {
            Self::Off => None,
            Self::Collect { book, .. } | Self::Apply { book } => Some(book),
        }
    }

    /// Whether the person turned down `kind` as a whole in the collecting
    /// pass.
    pub fn category_rejected(&self, kind: ItemKind) -> bool {
        self.book().is_some_and(|book| {
            book.borrow()
                .get(&(kind, Subject::Category))
                .is_some_and(|answer| answer.decision == Decision::Reject)
        })
    }

    /// What the person answered for `name` in the collecting pass, whether
    /// they were asked about the item or about its whole category. An answer
    /// the applying pass found stale is gone, so it reports nothing.
    pub fn answered(
        &self,
        kind: ItemKind,
        name: &str,
    ) -> Option<(Decision, crate::conflict::Scope)> {
        use crate::conflict::Scope;
        let book = self.book()?.borrow();
        book.get(&(kind, Subject::Item(name.to_string())))
            .map(|answer| (answer.decision, Scope::Item))
            .or_else(|| {
                book.get(&(kind, Subject::Category))
                    .map(|answer| (answer.decision, Scope::Category))
            })
    }

    /// Asks in the collecting pass; looks the answer up in the applying one.
    /// `seen` fingerprints what the question shows. `None` means nobody was
    /// asked about this item as it is now: it appeared between the passes,
    /// or changed since it was asked about.
    fn answer(&self, change: ItemChange, seen: String) -> Result<Option<Decision>> {
        let subject = match change.scope {
            DecisionScope::PerItem => Subject::Item(change.name.clone()),
            DecisionScope::WholeCategory => Subject::Category,
        };
        let key = (change.kind, subject);
        match self {
            Self::Collect { decider, book } => {
                let decision = decider.decide(&change).map_err(|e| {
                    // Any failure to ask stops the run, and the first pass has
                    // written nothing.
                    if e.downcast_ref::<crate::SyncError>().is_some() {
                        e
                    } else {
                        e.context(crate::SyncError::InteractiveAborted)
                    }
                })?;
                book.borrow_mut().insert(key, Answer { decision, seen });
                Ok(Some(decision))
            }
            Self::Apply { book } => {
                let mut book = book.borrow_mut();
                match book.get(&key) {
                    Some(answer) if answer.seen == seen => Ok(Some(answer.decision)),
                    Some(_) => {
                        // Stale: neither a decision for this content nor a
                        // resolution to log.
                        book.remove(&key);
                        Ok(None)
                    }
                    None => Ok(None),
                }
            }
            Self::Off => Ok(Some(Decision::Accept)),
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
            unconfirmed: Vec::new(),
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
            let seen = files_seen(std::slice::from_ref(&item), target, &key);
            let turned_down = match self.answer(request, seen)? {
                Some(Decision::Accept) => {
                    resolved.items.push(item);
                    continue;
                }
                Some(Decision::Reject) => &mut resolved.declined,
                None => &mut resolved.unconfirmed,
            };
            turned_down.push(item.name.clone());
            if let Some(current) = existing {
                resolved.items.push(current.clone());
            }
        }
        Ok(resolved)
    }

    /// One question for a whole category; `None` when nobody was asked
    /// about it as it is now.
    ///
    /// `names` are the items that would change, `conflicting` how many of
    /// the category's items both sides changed since the last sync, so the
    /// prompt can say so. `seen` fingerprints the content behind the names.
    pub fn decide_category(
        &self,
        kind: ItemKind,
        names: &[String],
        conflicting: usize,
        seen: String,
    ) -> Result<Option<Decision>> {
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
        self.answer(request, seen)
    }

    /// Decisions over MCP servers, one per server or one for all of them.
    pub fn resolve_servers(
        &self,
        scope: DecisionScope,
        servers: HashMap<String, McpServer>,
        target: &HashMap<String, McpServer>,
        ledger: &Ledger,
    ) -> Result<ResolvedServers> {
        let mut changed: Vec<&String> = servers
            .iter()
            .filter(|(name, server)| target.get(*name) != Some(*server))
            .map(|(name, _)| name)
            .collect();
        changed.sort();
        if changed.is_empty() {
            return Ok((servers, Vec::new(), Vec::new()));
        }
        if scope == DecisionScope::WholeCategory {
            let names: Vec<String> = changed.into_iter().cloned().collect();
            let conflicting = names
                .iter()
                .filter(|name| ledger.server_conflict(name, &servers[*name], target.get(*name)))
                .count();
            let seen = servers_seen(&servers, target, &names);
            return Ok(
                match self.decide_category(ItemKind::McpServer, &names, conflicting, seen)? {
                    Some(Decision::Accept) => (servers, Vec::new(), Vec::new()),
                    Some(Decision::Reject) => (HashMap::new(), names, Vec::new()),
                    None => (HashMap::new(), Vec::new(), names),
                },
            );
        }
        let mut kept = servers.clone();
        let (mut declined, mut unconfirmed) = (Vec::new(), Vec::new());
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
            let seen = servers_seen(&servers, target, std::slice::from_ref(name));
            let turned_down = match self.answer(request, seen)? {
                Some(Decision::Accept) => continue,
                Some(Decision::Reject) => &mut declined,
                None => &mut unconfirmed,
            };
            turned_down.push(name.clone());
            match existing {
                Some(current) => {
                    kept.insert(name.clone(), current.clone());
                }
                None => {
                    kept.remove(name);
                }
            }
        }
        Ok((kept, declined, unconfirmed))
    }

    /// Whether the `model` preference may be written; `None` when nobody was
    /// asked about this model.
    pub fn decide_model(&self, current: Option<&str>, incoming: &str) -> Result<Option<Decision>> {
        if current == Some(incoming) {
            return Ok(Some(Decision::Accept));
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
        let seen = format!("{}\n{incoming}", current.unwrap_or(""));
        self.answer(request, seen)
    }

    /// Turns the writer's report into the run's: an item the person rejected,
    /// or was never asked about as it is, is not a write, and says so.
    pub fn annotate(
        &self,
        mut report: WriteReport,
        declined: &[String],
        unconfirmed: &[String],
    ) -> WriteReport {
        report.skipped.retain(|reason| {
            !matches!(reason, SkipReason::Unchanged { item }
                if declined.contains(item) || unconfirmed.contains(item))
        });
        report.skipped.extend(
            declined
                .iter()
                .map(|item| SkipReason::Declined { item: item.clone() }),
        );
        report.skipped.extend(
            unconfirmed
                .iter()
                .map(|item| SkipReason::Unconfirmed { item: item.clone() }),
        );
        report
    }
}

/// A fingerprint of `items` and the target copies they would replace, so an
/// answer can be matched to the content it was given for.
pub(crate) fn files_seen(
    items: &[Command],
    target: &[Command],
    key: impl Fn(&Command) -> String,
) -> String {
    let held: HashMap<String, &Command> = target
        .iter()
        .filter(|t| t.plugin_origin.is_none())
        .map(|t| (key(t), t))
        .collect();
    let mut parts: Vec<String> = items
        .iter()
        .map(|item| {
            let target = held
                .get(&key(item))
                .map(|t| item_hash(t))
                .unwrap_or_default();
            format!("{}\0{}\0{target}", item.name, item_hash(item))
        })
        .collect();
    parts.sort();
    parts.join("\n")
}

/// [`files_seen`] for the MCP servers named in `names`.
fn servers_seen(
    servers: &HashMap<String, McpServer>,
    target: &HashMap<String, McpServer>,
    names: &[String],
) -> String {
    let mut parts: Vec<String> = names
        .iter()
        .map(|name| {
            let source = servers.get(name).map(server_hash).unwrap_or_default();
            let held = target.get(name).map(server_hash).unwrap_or_default();
            format!("{name}\0{source}\0{held}")
        })
        .collect();
    parts.sort();
    parts.join("\n")
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
