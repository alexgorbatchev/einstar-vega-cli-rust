use einstar_vega::{NodeKind, ProjectInfo, ProjectTree, RemotePath};

#[test]
fn intermediate_directories_have_their_own_paths_and_selection_is_deduplicated() {
    let project = ProjectInfo {
        path: RemotePath::parse("/projects/P  space").unwrap(),
        date_time: "date".into(),
    };
    let tree = ProjectTree::new(
        "P  space",
        project,
        [
            "/projects/P  space/sub/a",
            "/projects/P  space/sub/b",
            "/projects/P  space/mesh <- ",
        ]
        .map(|p| RemotePath::parse(p).unwrap()),
    )
    .unwrap();
    let root = tree.root();
    let directory = tree
        .nodes()
        .iter()
        .find(|n| n.path.as_str().ends_with("/sub"))
        .unwrap();
    assert_eq!(directory.kind, NodeKind::Directory);
    assert_eq!(directory.name, "sub");
    let leaf = tree.nodes().iter().find(|n| n.name == "a").unwrap();
    let files = tree.files_for(&[root, directory.id, leaf.id]).unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(
        tree.nodes()
            .iter()
            .find(|n| n.name == "mesh <- ")
            .unwrap()
            .path
            .as_str(),
        "/projects/P  space/mesh <- "
    );
}

#[test]
fn path_components_cannot_escape_or_masquerade_as_project_prefixes() {
    for path in [
        "relative/a",
        "/p/../a",
        "/p/./a",
        "/p//a",
        "/p/a\\b",
        "/p/a\0b",
        "//",
        "///",
    ] {
        assert!(RemotePath::parse(path).is_err(), "accepted {path:?}");
    }
    let root = RemotePath::parse("/p/one").unwrap();
    assert_eq!(
        RemotePath::parse("/p/one/dir/a")
            .unwrap()
            .relative_to(&root)
            .unwrap(),
        std::path::PathBuf::from("dir/a")
    );
    assert!(
        RemotePath::parse("/p/one-more/a")
            .unwrap()
            .relative_to(&root)
            .is_err()
    );
}

#[test]
fn identities_from_another_tree_cannot_select_or_change_files() {
    let metadata = ProjectInfo {
        path: RemotePath::parse("/p").unwrap(),
        date_time: String::new(),
    };
    let first = ProjectTree::new(
        "first",
        metadata.clone(),
        [RemotePath::parse("/p/a").unwrap()],
    )
    .unwrap();
    let mut second =
        ProjectTree::new("second", metadata, [RemotePath::parse("/p/b").unwrap()]).unwrap();
    let foreign = first
        .nodes()
        .iter()
        .find(|n| n.kind == NodeKind::File)
        .unwrap()
        .id;
    assert!(second.node(foreign).is_err());
    assert!(second.files_for(&[foreign]).is_err());
    assert!(second.set_size(foreign, 10).is_err());
    assert_eq!(second.node(second.root()).unwrap().size, None);
}

#[test]
fn sizes_remain_unknown_until_complete_and_overflow_is_transactional() {
    let metadata = ProjectInfo {
        path: RemotePath::parse("/p").unwrap(),
        date_time: "date".into(),
    };
    let mut tree = ProjectTree::new(
        "P",
        metadata.clone(),
        ["/p/a", "/p/sub/b"].map(|p| RemotePath::parse(p).unwrap()),
    )
    .unwrap();
    let a = tree.nodes().iter().find(|n| n.name == "a").unwrap().id;
    let b = tree.nodes().iter().find(|n| n.name == "b").unwrap().id;
    assert_eq!(tree.name(), "P");
    assert_eq!(tree.project(), &metadata);
    assert!(tree.set_size(tree.root(), 1).is_err());
    tree.set_size(a, 7).unwrap();
    assert_eq!(tree.node(tree.root()).unwrap().size, None);
    tree.set_size(b, 11).unwrap();
    assert_eq!(tree.node(tree.root()).unwrap().size, Some(18));
    assert!(tree.set_size(a, u64::MAX).is_err());
    assert_eq!(tree.node(a).unwrap().size, Some(7));
    assert_eq!(tree.node(tree.root()).unwrap().size, Some(18));
    assert!(
        ProjectTree::new(
            "P",
            metadata.clone(),
            ["/p/a", "/p/a/b"].map(|p| RemotePath::parse(p).unwrap())
        )
        .is_err()
    );
    let empty = ProjectTree::new("P", metadata, []).unwrap();
    assert!(empty.files_for(&[empty.root()]).unwrap().is_empty());
    assert_eq!(empty.node(empty.root()).unwrap().size, Some(0));
}

#[test]
fn root_and_trailing_separator_paths_keep_file_boundaries() {
    let root = RemotePath::parse("/").unwrap();
    assert_eq!(root.to_string(), "/");
    assert!(root.relative_to(&root).is_err());
    let directory = RemotePath::parse("/p/").unwrap();
    assert_eq!(directory.as_str(), "/p");
    assert_eq!(
        RemotePath::parse("/p/a")
            .unwrap()
            .relative_to(&root)
            .unwrap(),
        std::path::PathBuf::from("p/a")
    );
    assert!(directory.relative_to(&directory).is_err());
}
