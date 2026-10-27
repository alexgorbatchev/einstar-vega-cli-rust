use super::*;
use einstar_vega::{ProjectInfo, RemotePath};

fn tree() -> ProjectTree {
    ProjectTree::new(
        "Project",
        ProjectInfo {
            path: RemotePath::parse("/Project").unwrap(),
            date_time: "today".into(),
        },
        ["/Project/dir/a", "/Project/dir/b", "/Project/z"]
            .map(|path| RemotePath::parse(path).unwrap()),
    )
    .unwrap()
}
fn press(model: &mut Model, code: KeyCode) -> Action {
    model.key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn keyboard_navigation_expansion_toggle_and_selection_track_visible_nodes() {
    let mut model = Model::default();
    assert!(matches!(
        press(&mut model, KeyCode::Char('d')),
        Action::None
    ));
    assert!(model.logs.back().unwrap().contains("No files selected"));
    model.apply(Update::Tree(tree()));
    press(&mut model, KeyCode::Right);
    press(&mut model, KeyCode::Right);
    assert_eq!(model.visible_rows().len(), 3);
    press(&mut model, KeyCode::Down);
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.visible_rows().len(), 5);
    press(&mut model, KeyCode::PageDown);
    assert_eq!(model.focus, 4);
    press(&mut model, KeyCode::PageUp);
    assert_eq!(model.focus, 0);
    press(&mut model, KeyCode::End);
    assert_eq!(model.focus, 4);
    press(&mut model, KeyCode::Up);
    assert_eq!(model.focus, 3);
    press(&mut model, KeyCode::Home);
    assert_eq!(model.focus, 0);
    press(&mut model, KeyCode::Char(' '));
    assert_eq!(model.selected.len(), 1);
    press(&mut model, KeyCode::Char(' '));
    assert!(model.selected.is_empty());
    press(&mut model, KeyCode::Left);
    assert_eq!(model.visible_rows().len(), 1);
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.visible_rows().len(), 1);
    let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(matches!(model.key(release), Action::None));
    assert!(matches!(
        model.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Action::Quit
    ));
}

#[test]
fn replacement_and_size_failures_preserve_native_identity_and_bounded_logs() {
    let mut model = Model::new();
    let old = tree();
    let old_root = old.root();
    model.apply(Update::Tree(old));
    press(&mut model, KeyCode::Right);
    press(&mut model, KeyCode::Char(' '));
    model.apply(Update::Tree(tree()));
    assert!(model.selected.is_empty());
    assert!(model.expanded.is_empty());
    assert!(
        !model
            .visible_rows()
            .iter()
            .any(|row| row.node.id == old_root)
    );
    model.apply(Update::Size {
        project: "Project".into(),
        node: old_root,
        size: 1,
    });
    assert!(model.logs.back().unwrap().contains("another project"));
    model.apply(Update::Size {
        project: "missing".into(),
        node: old_root,
        size: 1,
    });
    for index in 0..1002 {
        model.apply(Update::Log(format!("line {index}")));
    }
    assert_eq!(model.logs.len(), 1000);
    assert_eq!(model.logs.front().unwrap(), "line 2");
    model.apply(Update::Progress {
        path: PathBuf::from("Project/a"),
        progress: Progress {
            downloaded: 5,
            total: 10,
        },
    });
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 12)).unwrap();
    terminal
        .draw(|frame| super::super::render(&mut model, frame))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Project/a: 5/10 bytes"));
    model.apply(Update::DownloadDone {
        completed: 1,
        failures: 0,
    });
    assert!(model.progress.is_none());
}
