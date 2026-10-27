use crate::{Error, ProjectInfo, RemotePath, Result};
use std::collections::{BTreeMap, BTreeSet};

/// Stable identity within one immutable project tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NodeId {
    tree: uuid::Uuid,
    index: usize,
}

/// A file or directory; empty directories are distinct from zero-byte files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    File,
    Directory,
}

/// Device identity and hierarchy, independent of any rendered label.
#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub path: RemotePath,
    pub kind: NodeKind,
    pub size: Option<u64>,
    pub children: Vec<NodeId>,
}

/// A project tree constructed from the device's flat file list.
#[derive(Clone, Debug)]
pub struct ProjectTree {
    name: String,
    project: ProjectInfo,
    nodes: Vec<Node>,
}

impl ProjectTree {
    /// Constructs a deterministic tree, rejecting foreign paths and file/
    /// directory collisions. Repeated file paths are represented once.
    pub fn new(
        name: impl Into<String>,
        project: ProjectInfo,
        paths: impl IntoIterator<Item = RemotePath>,
    ) -> Result<Self> {
        let name = name.into();
        let identity = uuid::Uuid::new_v4();
        let root = Node {
            id: NodeId {
                tree: identity,
                index: 0,
            },
            name: name.clone(),
            path: project.path.clone(),
            kind: NodeKind::Directory,
            size: None,
            children: Vec::new(),
        };
        let mut tree = Self {
            name,
            project,
            nodes: vec![root],
        };
        let mut known = BTreeMap::from([(String::new(), tree.root())]);
        let paths: BTreeSet<_> = paths.into_iter().collect();
        for path in paths {
            let relative = path.relative_to(&tree.project.path)?;
            let parts: Vec<_> = relative.iter().collect();
            let mut current = String::new();
            let mut parent = tree.root();
            for (index, component) in parts.iter().enumerate() {
                let name = component
                    .to_str()
                    .ok_or_else(|| Error::InvalidPath(path.to_string()))?;
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(name);
                let kind = if index + 1 == parts.len() {
                    NodeKind::File
                } else {
                    NodeKind::Directory
                };
                let id = if let Some(&id) = known.get(&current) {
                    if tree.nodes[id.index].kind != kind {
                        return Err(Error::Protocol(format!(
                            "file/directory collision: {current}"
                        )));
                    }
                    id
                } else {
                    let id = NodeId {
                        tree: identity,
                        index: tree.nodes.len(),
                    };
                    let absolute = format!(
                        "{}/{}",
                        tree.project.path.as_str().trim_end_matches('/'),
                        current
                    );
                    tree.nodes.push(Node {
                        id,
                        name: name.into(),
                        path: RemotePath::parse(absolute)?,
                        kind,
                        size: None,
                        children: Vec::new(),
                    });
                    tree.nodes[parent.index].children.push(id);
                    known.insert(current.clone(), id);
                    id
                };
                parent = id;
            }
        }
        if tree.nodes.len() == 1 {
            tree.nodes[0].size = Some(0);
        }
        Ok(tree)
    }

    /// Project name as supplied by the device, without display decoration.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Project metadata associated with every node in this tree.
    pub fn project(&self) -> &ProjectInfo {
        &self.project
    }
    /// The project's root directory identity.
    pub fn root(&self) -> NodeId {
        self.nodes[0].id
    }
    /// Nodes in parent-before-child order.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    /// Looks up a node, rejecting identities belonging to another tree.
    pub fn node(&self, id: NodeId) -> Result<&Node> {
        if id.tree != self.root().tree {
            return Err(Error::Protocol(
                "node belongs to another project tree".into(),
            ));
        }
        self.nodes
            .get(id.index)
            .ok_or_else(|| Error::Protocol("unknown tree node".into()))
    }

    /// Returns each selected file at most once, including directory descendants.
    pub fn files_for(&self, selected: &[NodeId]) -> Result<Vec<RemotePath>> {
        let mut files = BTreeSet::new();
        let mut pending = selected.to_vec();
        while let Some(id) = pending.pop() {
            let node = self.node(id)?;
            if node.kind == NodeKind::File {
                files.insert(node.path.clone());
            } else {
                pending.extend_from_slice(&node.children);
            }
        }
        Ok(files.into_iter().collect())
    }

    /// Updates a file size and recomputes checked aggregate directory sizes.
    /// A directory remains unknown until all descendants have known sizes.
    pub fn set_size(&mut self, id: NodeId, size: u64) -> Result<()> {
        if self.node(id)?.kind != NodeKind::File {
            return Err(Error::Protocol("size belongs to a file node".into()));
        }
        // Compute on a copy so overflow cannot partially update the model.
        let mut sizes: Vec<_> = self.nodes.iter().map(|node| node.size).collect();
        sizes[id.index] = Some(size);
        for node in self.nodes.iter().rev() {
            if node.kind != NodeKind::Directory {
                continue;
            }
            let mut total = Some(0_u64);
            for child in &node.children {
                total = match (total, sizes[child.index]) {
                    (Some(total), Some(size)) => Some(
                        total
                            .checked_add(size)
                            .ok_or_else(|| Error::Protocol("project size overflow".into()))?,
                    ),
                    _ => None,
                };
            }
            sizes[node.id.index] = total;
        }
        for (node, size) in self.nodes.iter_mut().zip(sizes) {
            node.size = size;
        }
        Ok(())
    }
}
