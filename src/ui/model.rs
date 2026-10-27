use crate::output;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use einstar_vega::{DeviceInfo, DownloadSpec, Node, NodeId, NodeKind, Progress, ProjectTree};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
};

#[derive(Debug)]
pub enum Update {
    Info(DeviceInfo),
    Tree(ProjectTree),
    Size {
        project: String,
        node: NodeId,
        size: u64,
    },
    Log(String),
    Progress {
        path: PathBuf,
        progress: Progress,
    },
    DownloadDone {
        completed: usize,
        failures: usize,
    },
}

#[derive(Debug)]
pub enum Action {
    None,
    Quit,
    Download(Vec<DownloadSpec>),
}

pub struct Row<'a> {
    pub project: &'a ProjectTree,
    pub node: &'a Node,
    pub depth: usize,
}

#[derive(Default)]
enum DownloadState {
    #[default]
    Idle,
    Running,
}

/// All interaction uses native tree identities; labels are display-only.
pub struct Model {
    pub(super) trees: BTreeMap<String, ProjectTree>,
    pub(super) info: DeviceInfo,
    pub(super) logs: VecDeque<String>,
    pub(super) expanded: BTreeSet<NodeId>,
    pub(super) selected: BTreeSet<NodeId>,
    pub(super) focus: usize,
    download: DownloadState,
    pub(super) progress: Option<String>,
    pub(super) table: ratatui::widgets::TableState,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    pub fn new() -> Self {
        let mut model = Self {
            trees: BTreeMap::new(),
            info: DeviceInfo::new(),
            logs: VecDeque::new(),
            expanded: BTreeSet::new(),
            selected: BTreeSet::new(),
            focus: 0,
            download: DownloadState::Idle,
            progress: None,
            table: Default::default(),
        };
        model.log("INFO: Application started".into());
        model
    }

    fn log(&mut self, text: String) {
        if self.logs.len() == 1000 {
            self.logs.pop_front();
        }
        self.logs.push_back(output::safe(&text));
    }

    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Info(info) => self.info = info,
            Update::Tree(tree) => {
                if let Some(previous) = self.trees.insert(tree.name().to_owned(), tree) {
                    for node in previous.nodes() {
                        self.expanded.remove(&node.id);
                        self.selected.remove(&node.id);
                    }
                }
                self.focus = self.focus.min(self.visible_rows().len().saturating_sub(1));
            }
            Update::Size {
                project,
                node,
                size,
            } => {
                if let Some(tree) = self.trees.get_mut(&project)
                    && let Err(error) = tree.set_size(node, size)
                {
                    self.log(format!("ERROR: {error}"));
                }
            }
            Update::Log(text) => self.log(text),
            Update::Progress { path, progress } => {
                self.progress = Some(output::safe(&format!(
                    "{}: {}/{} bytes",
                    cli_command_kit::display_path(&path),
                    progress.downloaded,
                    progress.total
                )));
            }
            Update::DownloadDone {
                completed,
                failures,
            } => {
                self.download = DownloadState::Idle;
                self.progress = None;
                self.log(format!("downloaded:{completed} failed:{failures}"));
            }
        }
    }

    pub fn visible_rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        for project in self.trees.values() {
            let mut pending = vec![(project.root(), 0)];
            while let Some((id, depth)) = pending.pop() {
                let node = project
                    .node(id)
                    .expect("tree children retain their own identity");
                rows.push(Row {
                    project,
                    node,
                    depth,
                });
                if self.expanded.contains(&id) {
                    pending.extend(node.children.iter().rev().map(|id| (*id, depth + 1)));
                }
            }
        }
        rows
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if key.code == KeyCode::Char('q')
            || key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return Action::Quit;
        }
        let rows = self.visible_rows();
        let count = rows.len();
        let current = rows.get(self.focus).map(|row| (row.node.id, row.node.kind));
        match key.code {
            KeyCode::Down => self.focus = (self.focus + 1).min(count.saturating_sub(1)),
            KeyCode::Up => self.focus = self.focus.saturating_sub(1),
            KeyCode::Home => self.focus = 0,
            KeyCode::End => self.focus = count.saturating_sub(1),
            KeyCode::PageDown => {
                self.focus = self.focus.saturating_add(10).min(count.saturating_sub(1))
            }
            KeyCode::PageUp => self.focus = self.focus.saturating_sub(10),
            KeyCode::Char(' ') => {
                if let Some((id, _)) = current
                    && !self.selected.remove(&id)
                {
                    self.selected.insert(id);
                }
            }
            KeyCode::Enter | KeyCode::Right => {
                if let Some((id, NodeKind::Directory)) = current
                    && (key.code == KeyCode::Right || !self.expanded.remove(&id))
                {
                    self.expanded.insert(id);
                }
            }
            KeyCode::Left => {
                if let Some((id, _)) = current {
                    self.expanded.remove(&id);
                }
            }
            KeyCode::Char('x') => self.log("Delete is unsupported by the device protocol".into()),
            KeyCode::Char('r') => self.log("Rename is unsupported by the device protocol".into()),
            KeyCode::Char('d') => {
                if matches!(self.download, DownloadState::Running) {
                    self.log("A download is already running".into());
                    return Action::None;
                }
                let selected = if self.selected.is_empty() {
                    current
                        .map(|(id, _)| BTreeSet::from([id]))
                        .unwrap_or_default()
                } else {
                    self.selected.clone()
                };
                let mut jobs = Vec::new();
                for tree in self.trees.values() {
                    let ids = tree
                        .nodes()
                        .iter()
                        .filter(|node| selected.contains(&node.id))
                        .map(|node| node.id)
                        .collect::<Vec<_>>();
                    let result = tree.files_for(&ids).and_then(|paths| {
                        paths
                            .into_iter()
                            .map(|path| {
                                DownloadSpec::new(tree.name(), tree.project().path.clone(), path)
                            })
                            .collect::<einstar_vega::Result<Vec<_>>>()
                    });
                    match result {
                        Ok(files) => jobs.extend(files),
                        Err(error) => {
                            let message = error.to_string();
                            self.log(format!("ERROR: {message}"));
                            return Action::None;
                        }
                    }
                }
                if jobs.is_empty() {
                    self.log("No files selected".into());
                } else {
                    self.download = DownloadState::Running;
                    return Action::Download(jobs);
                }
            }
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
#[path = "../../tests/unit/model.rs"]
mod tests;
