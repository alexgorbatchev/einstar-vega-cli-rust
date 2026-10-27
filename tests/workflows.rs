use cli_command_kit::Mode;
use einstar_vega::CancellationToken;
use einstar_vega_cli::{cli, mock};

fn temporary() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

async fn server() -> (
    u16,
    CancellationToken,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let token = CancellationToken::new();
    let task = tokio::spawn(mock::serve(listener, token.clone()));
    (port, token, task)
}

#[tokio::test]
async fn mock_lists_correct_projects_and_headless_all_downloads_six_exact_files() {
    let (port, shutdown, task) = server().await;
    for mode in [Mode::Agent, Mode::Human] {
        let mut output = Vec::new();
        cli::execute(
            [
                "vega-cli",
                "--ip",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "project",
                "list",
            ],
            mode,
            &CancellationToken::new(),
            &mut output,
        )
        .await
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Project1") && text.contains("Project2"));
        if mode == Mode::Agent {
            assert!(!text.contains('│'));
        }
    }
    let output = temporary();
    let mut text = Vec::new();
    cli::execute(
        [
            "vega-cli",
            "--ip",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--output",
            output.path().to_str().unwrap(),
            "project",
            "download",
            "--all",
        ],
        Mode::Agent,
        &CancellationToken::new(),
        &mut text,
    )
    .await
    .unwrap();
    for project in ["Project1", "Project2"] {
        for file in ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"] {
            assert_eq!(
                std::fs::read(output.path().join(project).join(file)).unwrap(),
                vec![0; 1024]
            );
        }
    }
    assert!(String::from_utf8(text).unwrap().contains("downloaded:6"));
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn device_inspection_and_missing_project_are_observable_without_false_success() {
    let (port, shutdown, task) = server().await;
    let prefix = ["vega-cli", "--ip", "127.0.0.1", "--port", &port.to_string()];
    let mut output = Vec::new();
    cli::execute(
        prefix.into_iter().chain(["device", "inspect"]),
        Mode::Agent,
        &CancellationToken::new(),
        &mut output,
    )
    .await
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("firmwareVersion") && text.contains("1.3.0-22"));
    let mut output = Vec::new();
    let error = cli::execute(
        prefix.into_iter().chain(["project", "download", "unknown"]),
        Mode::Agent,
        &CancellationToken::new(),
        &mut output,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Project1") && error.to_string().contains("Project2"));
    assert!(output.is_empty());
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn validation_help_version_skill_and_output_failures_are_offline() {
    for mode in [Mode::Agent, Mode::Human] {
        let mut output = Vec::new();
        cli::execute(
            ["vega-cli", "--version"],
            mode,
            &CancellationToken::new(),
            &mut output,
        )
        .await
        .unwrap();
        assert_eq!(output, b"0.1.0\n");
        let mut output = Vec::new();
        cli::execute(
            ["vega-cli", "skill"],
            mode,
            &CancellationToken::new(),
            &mut output,
        )
        .await
        .unwrap();
        assert_eq!(output, cli::SKILL_POINTER.as_bytes());
        for args in [
            vec!["vega-cli", "project", "download"],
            vec!["vega-cli", "project", "download", "P", "--all"],
            vec!["vega-cli", "--port", "0", "project", "list"],
            vec!["vega-cli", "skill", "extra"],
        ] {
            let error = cli::execute(args, mode, &CancellationToken::new(), &mut Vec::new())
                .await
                .unwrap_err();
            assert_eq!(cli::exit_status(&error), 64);
        }
        let mut output = Vec::new();
        cli::execute(
            ["vega-cli", "--help"],
            mode,
            &CancellationToken::new(),
            &mut output,
        )
        .await
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("project") && text.contains("download"));
        assert_eq!(text.starts_with("ALERT:"), mode == Mode::Agent);
    }
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(
        cli::execute(
            ["vega-cli", "skill"],
            Mode::Agent,
            &CancellationToken::new(),
            &mut Broken
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn mock_rejects_foreign_files_and_invalid_content_ranges() {
    let (port, shutdown, task) = server().await;
    let client = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}");
    assert_eq!(
        client
            .post(format!("{base}/file/info"))
            .body("/outside")
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let body = cbor4ii::serde::to_vec(
        Vec::new(),
        &serde_json::json!({"path":"/mnt/sdcard/Project1/mesh.beb","pos":1020,"size":5}),
    )
    .unwrap();
    assert_eq!(
        client
            .post(format!("{base}/file/content"))
            .body(body)
            .send()
            .await
            .unwrap()
            .status(),
        416
    );
    assert_eq!(
        client
            .get(format!("{base}/file/info"))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn human_exact_project_download_reports_success_and_leaves_other_project_absent() {
    let (port, shutdown, task) = server().await;
    let root = temporary();
    let mut output = Vec::new();
    cli::execute(
        [
            "vega-cli",
            "project",
            "download",
            "Project2",
            "--ip",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--output",
            root.path().to_str().unwrap(),
        ],
        Mode::Human,
        &CancellationToken::new(),
        &mut output,
    )
    .await
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(
        text.contains("[INFO]") && text.contains("[OK] Downloaded"),
        "{text}"
    );
    assert!(
        text.contains("Downloaded 3 files; 3 published; 0 failed."),
        "{text}"
    );
    for file in ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"] {
        assert_eq!(
            std::fs::read(root.path().join("Project2").join(file)).unwrap(),
            vec![0; 1024]
        );
    }
    assert!(!root.path().join("Project1").exists());
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

#[test]
fn returned_errors_use_the_selected_mode_and_preserve_write_failures() {
    use einstar_vega_cli::output;
    for (mode, expected) in [
        (Mode::Agent, "ERR: broken\\nline\n"),
        (Mode::Human, "[ERROR] broken\\nline\n"),
    ] {
        let mut bytes = Vec::new();
        output::report_error(&mut bytes, mode, &std::io::Error::other("broken\nline")).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), expected);
    }
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        output::report_error(&mut Broken, Mode::Human, &std::io::Error::other("failed"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn device_metadata_preserves_native_cbor_scalar_and_binary_values_in_both_modes() {
    use einstar_vega::Value;
    let info = std::collections::BTreeMap::from([
        ("temperature".into(), Value::Float(21.5)),
        ("charging".into(), Value::Bool(true)),
        ("optional".into(), Value::Null),
        ("binary".into(), Value::Bytes(vec![0, 255])),
    ]);
    for mode in [Mode::Human, Mode::Agent] {
        let mut output = Vec::new();
        einstar_vega_cli::output::device(&mut output, mode, &info).unwrap();
        let text = String::from_utf8(output).unwrap();
        for expected in ["21.5", "true", "null", "Bytes([0, 255])"] {
            assert!(text.contains(expected), "{text}");
        }
    }
}
