use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use einstar_vega::{ProjectInfo, ProjectTree, RemotePath};
use einstar_vega_cli::ui::{Action, Model, Update, render};

fn tree(name: &str) -> ProjectTree {
    ProjectTree::new(
        name,
        ProjectInfo {
            path: RemotePath::parse(format!("/device/{name}")).unwrap(),
            date_time: "today".into(),
        },
        [
            RemotePath::parse(format!("/device/{name}/dir/file  <-  name")).unwrap(),
            RemotePath::parse(format!("/device/{name}/other")).unwrap(),
        ],
    )
    .unwrap()
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn selection_uses_native_identity_deduplicates_ancestors_and_blocks_overlapping_workers() {
    let mut model = Model::new();
    model.apply(Update::Tree(tree("Project  <-  1")));
    assert_eq!(model.visible_rows().len(), 1);
    model.key(key(KeyCode::Char(' ')));
    model.key(key(KeyCode::Enter));
    assert_eq!(model.visible_rows().len(), 3);
    model.key(key(KeyCode::Down));
    model.key(key(KeyCode::Char(' ')));
    let Action::Download(jobs) = model.key(key(KeyCode::Char('d'))) else {
        panic!("no jobs")
    };
    assert_eq!(jobs.len(), 2);
    assert!(
        jobs.iter()
            .all(|job| job.project_name() == "Project  <-  1")
    );
    assert!(
        jobs.iter()
            .any(|job| job.path().as_str().ends_with("file  <-  name"))
    );
    assert!(matches!(model.key(key(KeyCode::Char('d'))), Action::None));
    model.apply(Update::DownloadDone {
        completed: 2,
        failures: 0,
    });
    assert!(matches!(
        model.key(key(KeyCode::Char('d'))),
        Action::Download(_)
    ));
    assert!(matches!(model.key(key(KeyCode::Char('q'))), Action::Quit));
}

#[test]
fn real_widgets_show_metadata_sizes_logs_and_safe_control_characters_at_narrow_width() {
    let mut model = Model::new();
    let mut project = tree("Project");
    let files: Vec<_> = project
        .nodes()
        .iter()
        .filter(|node| node.kind == einstar_vega::NodeKind::File)
        .map(|node| node.id)
        .collect();
    for file in files {
        project.set_size(file, 1024).unwrap();
    }
    model.apply(Update::Tree(project));
    model.apply(Update::Info(std::collections::BTreeMap::from([
        ("batteryValue".into(), einstar_vega::Value::Float(0.85)),
        (
            "deviceParams".into(),
            einstar_vega::Value::Text("model:Vega_cpu:arm:64".into()),
        ),
        (
            "boardInfo".into(),
            einstar_vega::Value::Map(vec![(
                einstar_vega::Value::Text("board".into()),
                einstar_vega::Value::Text("main".into()),
            )]),
        ),
    ])));
    model.key(key(KeyCode::Char('x')));
    model.key(key(KeyCode::Char('r')));
    model.apply(Update::Log("raw\u{1b}[31m\ncontrol".into()));
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(&mut model, frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    for expected in [
        "Files",
        "Device Info",
        "Logs",
        "85%",
        "arm:64",
        "main",
        "2.0 KB",
        "unsupported",
        "raw\\u{1b}",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains('\u{1b}'));
    terminal
        .resize(ratatui::layout::Rect::new(0, 0, 12, 5))
        .unwrap();
    terminal.draw(|frame| render(&mut model, frame)).unwrap();
}

#[test]
fn narrow_browser_retains_file_names_instead_of_reserving_space_for_sizes() {
    let mut model = Model::new();
    model.apply(Update::Tree(tree("Project")));
    model.key(key(KeyCode::Enter));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(42, 16)).unwrap();
    terminal.draw(|frame| render(&mut model, frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Project"), "{text}");
    assert!(text.contains("other"), "{text}");
}
