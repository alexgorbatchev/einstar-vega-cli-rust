#![cfg(unix)]

use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    poll::{PollFd, PollFlags, poll},
    pty::{Winsize, openpty},
    sys::termios::tcgetattr,
};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Browser {
    child: Child,
    master: File,
    slave: File,
    stop: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
    chunks: mpsc::Receiver<Vec<u8>>,
    output: Vec<u8>,
}
impl Browser {
    fn start(port: u16, output: &std::path::Path) -> Self {
        let pty = openpty(
            Some(&Winsize {
                ws_row: 30,
                ws_col: 100,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .unwrap();
        let master = File::from(pty.master);
        let slave = File::from(pty.slave);
        let flags = OFlag::from_bits_retain(fcntl(&master, FcntlArg::F_GETFL).unwrap());
        fcntl(&master, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_vega-cli"));
        for name in ["VEGA_IP", "VEGA_PORT", "VEGA_OUTPUT", "AGENT"] {
            command.env_remove(name);
        }
        let child = command
            .env("TERM", "xterm-256color")
            .args(["--ip", "127.0.0.1", "--port", &port.to_string(), "--output"])
            .arg(output)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()))
            .spawn()
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let mut input = master.try_clone().unwrap();
        let (tx, chunks) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut buffer = [0; 16384];
            while !stopped.load(Ordering::Acquire) {
                let mut ready = [PollFd::new(input.as_fd(), PollFlags::POLLIN)];
                if poll(&mut ready, 20_u16).unwrap() == 0 {
                    continue;
                }
                match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if tx.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            child,
            master,
            slave,
            stop,
            reader: Some(reader),
            chunks,
            output: Vec::new(),
        }
    }
    fn wait(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !String::from_utf8_lossy(&self.output).contains(text) {
            let chunk = self
                .chunks
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "waiting for {text}: {error}: {}",
                        String::from_utf8_lossy(&self.output)
                    )
                });
            self.output.extend(chunk);
        }
    }
    fn send(&mut self, text: &[u8]) {
        self.master.write_all(text).unwrap();
    }
    fn finish(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "{status}");
                break;
            }
            assert!(Instant::now() < deadline, "browser failed to stop");
            if let Ok(chunk) = self.chunks.recv_timeout(Duration::from_millis(10)) {
                self.output.extend(chunk);
            }
        }
        self.stop.store(true, Ordering::Release);
        self.reader.take().unwrap().join().unwrap();
        self.output.extend(self.chunks.try_iter().flatten());
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_pty_browses_downloads_and_restores_native_terminal_state() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cancel = einstar_vega::CancellationToken::new();
    let server = tokio::spawn(einstar_vega_cli::mock::serve(listener, cancel.clone()));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
    std::fs::create_dir_all(&root).unwrap();
    let output = tempfile::tempdir_in(root).unwrap();
    let mut browser = Browser::start(port, output.path());
    // openpty starts with canonical input and echo; the browser must restore it.
    browser.wait("Project1");
    assert!(
        !tcgetattr(&browser.slave)
            .unwrap()
            .local_flags
            .contains(nix::sys::termios::LocalFlags::ICANON)
    );
    browser.send(b"\r d");
    browser.wait("downloaded:3");
    browser.send(b"xr");
    browser.wait("unsupported");
    browser.send(b"q");
    browser.finish();
    let flags = tcgetattr(&browser.slave).unwrap().local_flags;
    assert!(
        flags.contains(nix::sys::termios::LocalFlags::ICANON | nix::sys::termios::LocalFlags::ECHO)
    );
    assert!(
        browser
            .output
            .windows(b"\x1b[?1049h".len())
            .any(|bytes| bytes == b"\x1b[?1049h")
    );
    assert!(
        browser
            .output
            .windows(b"\x1b[?1049l".len())
            .any(|bytes| bytes == b"\x1b[?1049l")
    );
    for file in ["mesh.beb", "preview.png", "frame0/DepthImg_0.dat"] {
        assert_eq!(
            std::fs::read(output.path().join("Project1").join(file)).unwrap(),
            vec![0; 1024]
        );
    }
    assert!(!output.path().join("Project2").exists());
    cancel.cancel();
    server.await.unwrap().unwrap();
}
