use cli_command_kit::Mode;
use einstar_vega::CancellationToken;
use einstar_vega_cli::cli;
use std::{
    io,
    process::{Command, Output, Stdio},
};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vega-cli"));
    for key in ["VEGA_IP", "VEGA_PORT", "VEGA_OUTPUT"] {
        command.env_remove(key);
    }
    command.env("AGENT", "1").stdin(Stdio::null());
    command
}

fn temporary() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

async fn run(mut command: Command) -> Output {
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_process_uses_environment_defaults_and_explicit_flags_override_every_setting() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let cancel = CancellationToken::new();
    let server = tokio::spawn(einstar_vega_cli::mock::serve(listener, cancel.clone()));
    let root = temporary();
    let from_env = root.path().join("from-env");
    let from_flags = root.path().join("from-flags");
    let unused = root.path().join("unused-environment-output");
    let mut env_command = command();
    env_command
        .env("VEGA_IP", "127.0.0.1")
        .env("VEGA_PORT", &port)
        .env("VEGA_OUTPUT", &from_env)
        .args(["project", "download", "Project2"]);
    let env_output = run(env_command).await;
    let mut flag_command = command();
    flag_command
        .env("VEGA_IP", "invalid host")
        .env("VEGA_PORT", "not-a-port")
        .env("VEGA_OUTPUT", &unused)
        .args([
            "project",
            "download",
            "Project1",
            "--ip",
            "127.0.0.1",
            "-p",
            &port,
            "-o",
        ])
        .arg(&from_flags);
    let flag_output = run(flag_command).await;
    cancel.cancel();
    server.await.unwrap().unwrap();
    for output in [env_output, flag_output] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("downloaded:3")
        );
    }
    for (directory, project) in [(&from_env, "Project2"), (&from_flags, "Project1")] {
        for file in ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"] {
            assert_eq!(
                std::fs::read(directory.join(project).join(file)).unwrap(),
                vec![0; 1024]
            );
        }
    }
    assert!(!from_env.join("Project1").exists());
    assert!(!from_flags.join("Project2").exists());
    assert!(!unused.exists());
}

#[tokio::test]
async fn every_supported_shell_completion_is_offline_and_output_failures_propagate() {
    for mode in [Mode::Human, Mode::Agent] {
        for shell in ["bash", "elvish", "fish", "powershell", "zsh"] {
            let mut bytes = Vec::new();
            cli::execute(
                ["vega-cli", "completion", shell],
                mode,
                &CancellationToken::new(),
                &mut bytes,
            )
            .await
            .unwrap();
            let text = String::from_utf8(bytes).unwrap();
            let options = if shell == "fish" {
                ["vega-cli", "-l ip", "-l port", "-l output"]
            } else {
                ["vega-cli", "--ip", "--port", "--output"]
            };
            for item in options {
                assert!(text.contains(item), "{shell} missing {item}: {text}");
            }
            assert!(!text.starts_with("ALERT:"));
        }
        for args in [
            vec!["vega-cli", "completion"],
            vec!["vega-cli", "completion", "unknown"],
            vec!["vega-cli", "completion", "bash", "extra"],
        ] {
            let mut bytes = Vec::new();
            let error = cli::execute(args, mode, &CancellationToken::new(), &mut bytes)
                .await
                .unwrap_err();
            assert_eq!(cli::exit_status(&error), 64);
            assert!(bytes.is_empty());
        }
        struct Broken;
        impl io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let error = cli::execute(
            ["vega-cli", "completion", "bash"],
            mode,
            &CancellationToken::new(),
            &mut Broken,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, einstar_vega_cli::Error::Io(ref error) if error.kind() == io::ErrorKind::BrokenPipe)
        );
    }
}

#[tokio::test]
async fn malformed_host_addresses_fail_offline_as_usage_errors_in_both_modes() {
    for mode in [Mode::Human, Mode::Agent] {
        for host in [
            "",
            " ",
            "user@127.0.0.1",
            "127.0.0.1/path",
            "http://127.0.0.1",
            "127.0.0.1?query=1",
            "[::1",
        ] {
            let mut bytes = Vec::new();
            let error = cli::execute(
                ["vega-cli", "--ip", host, "device", "inspect"],
                mode,
                &CancellationToken::new(),
                &mut bytes,
            )
            .await
            .unwrap_err();
            assert_eq!(cli::exit_status(&error), 64, "{host:?}: {error}");
            assert!(bytes.is_empty());
        }
    }
}

#[test]
fn unresolved_invalid_environment_ports_fail_before_io_or_terminal_access() {
    for args in [vec![], vec!["project", "list"], vec!["mock", "serve"]] {
        for port in ["not-a-port", "0", "65536", "-1"] {
            let output = command()
                .env("VEGA_PORT", port)
                .env("VEGA_IP", "127.0.0.1")
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(64),
                "{args:?}/{port}: {output:?}"
            );
            assert!(output.stdout.is_empty());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("--port"), "{error}");
            assert!(!error.contains("terminal"), "{error}");
        }
    }
}
