//! A child process whose stdout streams back while it runs (`commit_command`,
//! curl), with cancel and a time limit. The child is always reaped.

use std::io::{Read, Write};
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
/// How long to wait, after the exit, for stderr and the stdin writer: a
/// background grandchild may hold the pipes open.
const GRACE: Duration = Duration::from_millis(500);

pub const CANCELLED: &str = "Cancelled";
pub const TIMED_OUT: &str = "Timed out";

pub struct Finished {
    pub status: ExitStatus,
    pub stderr: String,
    /// Writing `input` failed, e.g. the process closed its stdin early.
    pub stdin_error: Option<String>,
}

/// Feed `input` to `child`'s stdin, pass its stdout to `on_chunk` as it
/// arrives and wait for it to exit. `child` must have piped stdin, stdout and
/// stderr. stdin is written from its own thread, so a process that never reads
/// it can't block cancel or the timeout. On cancel or `timeout` the child is
/// killed and the error is [`CANCELLED`] or [`TIMED_OUT`].
pub fn stream(
    mut child: Child,
    input: Vec<u8>,
    cancel: &AtomicBool,
    timeout: Duration,
    on_chunk: &mut dyn FnMut(&[u8]),
) -> Result<Finished, String> {
    let (wtx, wrx) = mpsc::channel::<Result<(), String>>();
    if let Some(mut stdin) = child.stdin.take() {
        thread::spawn(move || {
            // Dropping `stdin` at the end closes it: the child sees EOF.
            let _ = wtx.send(stdin.write_all(&input).map_err(|e| e.to_string()));
        });
    }
    let (etx, erx) = mpsc::channel::<String>();
    if let Some(mut err) = child.stderr.take() {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = err.read_to_end(&mut bytes);
            let _ = etx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
    }
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    if let Some(mut out) = child.stdout.take() {
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    }
    let started = Instant::now();
    let stop = || {
        if cancel.load(Ordering::Relaxed) {
            Some(CANCELLED)
        } else if started.elapsed() > timeout {
            Some(TIMED_OUT)
        } else {
            None
        }
    };
    loop {
        match rx.recv_timeout(POLL) {
            Ok(bytes) => on_chunk(&bytes),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Some(why) = stop() {
            return Err(kill(&mut child, why));
        }
    }
    // stdout closed; the process may still be running.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(kill(&mut child, &e.to_string())),
        }
        if let Some(why) = stop() {
            return Err(kill(&mut child, why));
        }
        thread::sleep(POLL / 4);
    };
    let deadline = Instant::now() + GRACE;
    let stderr = erx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_default();
    let stdin_error = wrx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok().and_then(Result::err);
    Ok(Finished { status, stderr, stdin_error })
}

fn kill(child: &mut Child, why: &str) -> String {
    // Errors here mean the child already exited; `wait` still reaps it.
    let _ = child.kill();
    let _ = child.wait();
    why.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn sh(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn streams_stdout_and_reports_status_and_stderr() {
        let mut chunks = Vec::new();
        let done = stream(
            sh("cat; sleep 0.2; echo late; echo oops >&2; exit 3"),
            b"early\n".to_vec(),
            &AtomicBool::new(false),
            Duration::from_secs(10),
            &mut |c| chunks.push(String::from_utf8_lossy(c).into_owned()),
        )
        .unwrap();
        assert_eq!(chunks, ["early\n", "late\n"]);
        assert_eq!(done.status.code(), Some(3));
        assert_eq!(done.stderr, "oops\n");
        assert!(done.stdin_error.is_none());
    }

    #[test]
    fn a_process_that_ignores_a_big_stdin_still_times_out() {
        // 1 MiB never read: writing it inline would block past the pipe buffer.
        let started = Instant::now();
        let err = stream(
            sh("exec sleep 30"),
            vec![b'x'; 1 << 20],
            &AtomicBool::new(false),
            Duration::from_millis(300),
            &mut |_| {},
        )
        .err();
        assert_eq!(err.as_deref(), Some(TIMED_OUT));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancel_kills_and_a_closed_stdin_is_reported() {
        let err =
            stream(sh("exec sleep 30"), Vec::new(), &AtomicBool::new(true), Duration::from_secs(10), &mut |_| {}).err();
        assert_eq!(err.as_deref(), Some(CANCELLED));
        let done = stream(
            sh("exec 0<&-; echo ok"),
            vec![b'x'; 1 << 20],
            &AtomicBool::new(false),
            Duration::from_secs(10),
            &mut |_| {},
        )
        .unwrap();
        assert!(done.status.success());
        assert!(done.stdin_error.is_some());
    }
}
