use axum::{Router, routing::post};
use einstar_vega::{CancellationToken, Client, DownloadSpec, OutputDirectory, RemotePath};

fn temporary_directory() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

async fn server(content: &'static [u8], size: u64) -> (Client, tokio::task::JoinHandle<()>) {
    let router = Router::new()
        .route(
            "/file/info",
            post(move || async move { size.to_be_bytes().to_vec() }),
        )
        .route("/file/content", post(move || async move { content }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Client::new(&listener.local_addr().unwrap().to_string()).unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (client, task)
}

fn spec() -> DownloadSpec {
    DownloadSpec::new(
        "Project",
        RemotePath::parse("/device/Project").unwrap(),
        RemotePath::parse("/device/Project/sub/mesh.beb").unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn failed_transfer_preserves_existing_bytes_and_success_atomically_replaces_them() {
    let temporary = temporary_directory();
    let existing = temporary.path().join("Project/sub/mesh.beb");
    std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
    std::fs::write(&existing, b"original").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    let output = OutputDirectory::open(temporary.path()).unwrap();
    let (client, task) = server(b"short", 8).await;
    assert!(
        output
            .download(&client, &spec(), &CancellationToken::new(), |_| {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");
    assert_eq!(
        std::fs::read_dir(existing.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    task.abort();
    let _ = task.await;
    let (client, task) = server(b"complete", 8).await;
    assert_eq!(
        output
            .download(&client, &spec(), &CancellationToken::new(), |_| {})
            .await
            .unwrap(),
        std::path::PathBuf::from("Project/sub/mesh.beb")
    );
    assert_eq!(std::fs::read(&existing).unwrap(), b"complete");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&existing).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn invalid_project_names_and_foreign_files_are_rejected_before_publication() {
    let root = RemotePath::parse("/device/P").unwrap();
    let file = RemotePath::parse("/device/P/a").unwrap();
    for name in ["..", ".", "../outside", "/outside", "a\\b", ""] {
        assert!(DownloadSpec::new(name, root.clone(), file.clone()).is_err());
    }
    assert!(DownloadSpec::new("P", root, RemotePath::parse("/device/P-more/a").unwrap()).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_output_parents_cannot_escape_the_output_capability() {
    let temporary = temporary_directory();
    let outside = temporary_directory();
    std::os::unix::fs::symlink(outside.path(), temporary.path().join("Project")).unwrap();
    let (client, task) = server(b"complete", 8).await;
    let output = OutputDirectory::open(temporary.path()).unwrap();
    assert!(
        output
            .download(&client, &spec(), &CancellationToken::new(), |_| {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    task.abort();
    let _ = task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_failure_after_rename_reports_that_bytes_were_published() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = temporary_directory();
    let output = OutputDirectory::open(temporary.path()).unwrap();
    let (client, task) = server(b"complete", 8).await;
    let parent = temporary.path().join("Project/sub");
    let mut staging = None;
    let result = output
        .download(&client, &spec(), &CancellationToken::new(), |_| {
            let path = std::fs::read_dir(&parent)
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".vega-download-")
                })
                .unwrap();
            // Rename needs write/search access; cleanup's directory enumeration
            // additionally needs read access. This is a genuine native failure.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o300)).unwrap();
            staging = Some(path);
        })
        .await;
    let staging = staging.unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(std::fs::read(parent.join("mesh.beb")).unwrap(), b"complete");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("published"),
        "untruthful outcome: {error}"
    );
    assert!(
        matches!(error, einstar_vega::Error::Published { path, staging: stage, cleanup } if path == spec().relative_path() && temporary.path().join(&stage) == staging && cleanup.kind() == std::io::ErrorKind::PermissionDenied)
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn cancellation_after_bytes_are_written_preserves_original_and_cleans_staging() {
    let temporary = temporary_directory();
    let existing = temporary.path().join(spec().relative_path());
    std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
    std::fs::write(&existing, b"original").unwrap();
    let output = OutputDirectory::open(temporary.path()).unwrap();
    let (client, task) = server(b"complete", 8).await;
    let token = CancellationToken::new();
    assert!(matches!(
        output
            .download(&client, &spec(), &token, |_| token.cancel())
            .await,
        Err(einstar_vega::Error::Cancelled)
    ));
    assert_eq!(std::fs::read(&existing).unwrap(), b"original");
    assert_eq!(
        std::fs::read_dir(existing.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    task.abort();
    let _ = task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn final_symlink_and_destination_changes_are_rejected_without_following_them() {
    let temporary = temporary_directory();
    let outside = temporary_directory();
    let outside_file = outside.path().join("sentinel");
    std::fs::write(&outside_file, b"original").unwrap();
    let destination = temporary.path().join(spec().relative_path());
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside_file, &destination).unwrap();
    let output = OutputDirectory::open(temporary.path()).unwrap();
    let (client, task) = server(b"complete", 8).await;
    assert!(
        output
            .download(&client, &spec(), &CancellationToken::new(), |_| {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&outside_file).unwrap(), b"original");
    std::fs::remove_file(&destination).unwrap();
    assert!(
        output
            .download(&client, &spec(), &CancellationToken::new(), |_| {
                std::fs::create_dir(&destination).unwrap();
            })
            .await
            .is_err()
    );
    assert!(destination.is_dir());
    assert_eq!(
        std::fs::read_dir(destination.parent().unwrap())
            .unwrap()
            .count(),
        1
    );
    task.abort();
    let _ = task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_transfer_reports_cleanup_failure_and_preserves_recovery_path() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = temporary_directory();
    let output = OutputDirectory::open(temporary.path()).unwrap();
    let (client, task) = server(b"short", 8).await;
    let parent = temporary.path().join("Project/sub");
    let mut staging = None;
    let result = output
        .download(&client, &spec(), &CancellationToken::new(), |_| {
            let path = std::fs::read_dir(&parent)
                .unwrap()
                .map(|e| e.unwrap().path())
                .next()
                .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
            staging = Some(path);
        })
        .await;
    let staging = staging.unwrap();
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!parent.join("mesh.beb").exists());
    assert_eq!(std::fs::read(staging.join("content")).unwrap(), b"short");
    assert!(
        matches!(result, Err(einstar_vega::Error::Cleanup { cause, staging: stage, cleanup }) if matches!(*cause, einstar_vega::Error::Size { expected: 8, received: 5 }) && temporary.path().join(&stage) == staging && cleanup.kind() == std::io::ErrorKind::PermissionDenied)
    );
    task.abort();
    let _ = task.await;
}
