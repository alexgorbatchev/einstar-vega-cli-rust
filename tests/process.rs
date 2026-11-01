use std::{
    io::{BufRead, BufReader, Read},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vega-cli"));
    for key in ["VEGA_IP", "VEGA_PORT", "VEGA_OUTPUT", "AGENT"] {
        command.env_remove(key);
    }
    command.stdin(Stdio::null());
    command
}

fn temporary() -> tempfile::TempDir {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

struct Running {
    child: Child,
    reader: Option<JoinHandle<Vec<u8>>>,
    finished: mpsc::Receiver<()>,
    port: u16,
}

impl Running {
    fn mock() -> Self {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let mut child = command()
            .env("AGENT", "1")
            .args(["mock", "serve", "--port", &port.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pipe = child.stdout.take().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(pipe);
            let mut output = String::new();
            reader.read_line(&mut output).unwrap();
            let _ = ready_tx.send(output.clone());
            let mut bytes = output.into_bytes();
            reader.read_to_end(&mut bytes).unwrap();
            let _ = finished_tx.send(());
            bytes
        });
        let running = Self {
            child,
            reader: Some(reader),
            finished,
            port,
        };
        let ready = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(ready, format!("mock:0.0.0.0:{port}\n"));
        running
    }

    #[cfg(unix)]
    fn signal(&self, signal: &str) {
        assert!(
            Command::new("/usr/bin/kill")
                .args([signal, &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }

    fn finish(mut self) -> (ExitStatus, Vec<u8>, Vec<u8>) {
        self.finished
            .recv_timeout(Duration::from_secs(10))
            .expect("mock did not finish owned shutdown");
        let status = self.child.wait().unwrap();
        let stdout = self.reader.take().unwrap().join().unwrap();
        let mut stderr = Vec::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        (status, stdout, stderr)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn process_metadata_and_diagnostics_obey_stream_and_exit_contracts() {
    for agent in ["0", "1"] {
        let version = command()
            .env("AGENT", agent)
            .arg("--version")
            .output()
            .unwrap();
        assert!(version.status.success());
        assert_eq!(version.stdout, b"0.1.0\n");
        assert!(version.stderr.is_empty());
        let skill = command().env("AGENT", agent).arg("skill").output().unwrap();
        assert!(skill.status.success());
        assert_eq!(skill.stdout, INSTALL_TEXT.as_bytes());
        assert!(skill.stderr.is_empty());
        for args in [
            vec!["--help"],
            vec!["project", "--help"],
            vec!["mock", "serve", "--help"],
            vec!["completion", "--help"],
            vec!["skill", "--help"],
        ] {
            let help = command().env("AGENT", agent).args(args).output().unwrap();
            assert!(help.status.success());
            assert!(help.stderr.is_empty());
            assert_eq!(help.stdout.starts_with(b"ALERT:"), agent == "1");
        }
        for args in [
            vec!["project", "download"],
            vec!["project", "list"],
            vec!["skill", "extra"],
        ] {
            let output = command().env("AGENT", agent).args(args).output().unwrap();
            assert_eq!(output.status.code(), Some(64));
            assert!(output.stdout.is_empty());
            let stderr = String::from_utf8(output.stderr).unwrap();
            let prefix = if agent == "1" { "ERR:" } else { "[ERROR]" };
            assert_eq!(stderr.matches(prefix).count(), 1, "{stderr}");
        }
    }
}

#[test]
fn nonterminal_root_fails_before_changing_terminal_modes() {
    let output = command()
        .env("AGENT", "1")
        .args(["--ip", "127.0.0.1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.contains(&0x1b));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("requires a terminal"));
    assert_eq!(stderr.matches("ERR:").count(), 1);
}

#[cfg(unix)]
#[test]
fn real_mock_process_handles_both_native_signals_and_releases_its_listener() {
    for signal in ["-INT", "-TERM"] {
        let server = Running::mock();
        let port = server.port;
        server.signal(signal);
        let (status, stdout, stderr) = server.finish();
        assert!(
            status.success(),
            "{signal}: {status}: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(stdout, format!("mock:0.0.0.0:{port}\n").as_bytes());
        assert!(stderr.is_empty());
        std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn child_download_publishes_all_projects_beneath_dot_output() {
    let server = Running::mock();
    let output = temporary();
    let result = command()
        .env("AGENT", "1")
        .current_dir(output.path())
        .args([
            "--ip",
            "127.0.0.1",
            "--port",
            &server.port.to_string(),
            "--output",
            ".",
            "project",
            "download",
            "--all",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("downloaded:6")
    );
    for project in ["Project1", "Project2"] {
        for file in ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"] {
            assert_eq!(
                std::fs::read(output.path().join(project).join(file)).unwrap(),
                vec![0; 1024]
            );
        }
    }
    server.signal("-TERM");
    let (status, _, stderr) = server.finish();
    assert!(status.success());
    assert!(stderr.is_empty());
}

const INSTALL_TEXT: &str = "Install the vega-cli skill using the Skills CLI:\n\n  npx skills add alexgorbatchev/einstar-vega-cli --skill vega-cli\n\nSkill:\n\n  https://github.com/alexgorbatchev/einstar-vega-cli/tree/main/skills/vega-cli\n";
