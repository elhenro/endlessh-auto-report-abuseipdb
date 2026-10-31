//! starts the real binary for tests: clean env, reaped on drop

use std::{
    io::{BufRead, BufReader, Read},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub const BIN: &str = env!("CARGO_BIN_EXE_endlessh-reporter");

/// one test at a time: macos sets cloexec on sockets non-atomically, so a child spawned by a
/// parallel test can inherit another test's socket and keep it open or reset its clients
pub fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// a clean env, so nothing from the developer's shell leaks in
pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env_clear()
        .env("DRY_RUN", "1")
        .env("TARPIT_DELAY_MS", "30");
    c
}

/// runs the binary the way systemd does: own pid in LISTEN_PID (exec keeps the shell's pid), `fd3` on fd 3.
/// fd 3 arrives via stdin so the test needs no unsafe pre_exec
pub fn as_systemd(listen_fds: &str, fd3: Stdio) -> Command {
    let mut c = command("/bin/sh");
    let script = format!(r#"export LISTEN_PID=$$ LISTEN_FDS={listen_fds}; exec "$0" 3<&0 <&-"#);
    c.args(["-c", &script, BIN]).stdin(fd3);
    c
}

pub fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// killed on drop; `startup` is everything logged up to the listening line
pub struct Running {
    pub child: Child,
    pub startup: String,
    stdout: Option<BufReader<ChildStdout>>,
}

impl Running {
    pub fn start(cmd: &mut Command) -> Self {
        let mut child = cmd.stdout(Stdio::piped()).spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut startup = String::new();
        while !startup.contains("starting tarpit") {
            let n = stdout.read_line(&mut startup).unwrap();
            assert!(n > 0, "exited before listening");
        }
        Self {
            child,
            startup,
            stdout: Some(stdout),
        }
    }

    pub fn close_stdout(&mut self) {
        self.stdout = None;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// connects and waits for the first banner bytes
pub fn visit(port: u16) -> TcpStream {
    let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert!(c.read(&mut [0u8; 64]).unwrap() > 0);
    c
}
