//! Generic subprocess driver: spawn a child, drain stdout + stderr to a
//! log file while forwarding lines to the progress sink, race against a
//! cancellation token, and return a structured outcome.
//!
//! Why this is its own module: the parser-per-language and the analyzer
//! invocations have nearly-identical scaffolding (spawn, drain output,
//! await with cancel, capture tail on failure). Keeping that one place
//! means the parser and analyzer call sites stay focused on their own
//! arg construction.
//!
//! Stream handling: we drain both `stdout` and `stderr`. We *must* drain
//! stdout because the parser emits a JSONL progress stream there — leaving
//! that pipe full would deadlock the child as soon as it exceeded ~64 KiB
//! of buffered output. Each line off either stream is first attempted as
//! a [`graphengine_progress::EngineEvent`] via [`try_parse_line`]; a
//! successful parse becomes a [`ProgressEvent::Engine`], and an
//! unparseable line falls through to [`ProgressEvent::Raw`]. This single
//! routing point means a future engine that gains JSONL emission
//! automatically lights up structured progress in every consumer — no
//! consumer-side changes required.
//!
//! Stderr is also written to a log file under `scratch_dir` so a failure
//! message can quote the tail without reconstructing it from buffer
//! fragments. We don't log stdout to disk: that stream is "live progress",
//! not diagnostics, and doubling it on disk would multiply scratch usage
//! for no debug value.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

use graphengine_progress::try_parse_line;

use crate::error::RunError;
use crate::progress::{ProgressEvent, ProgressSink, Stage};

/// Maximum number of stderr bytes we keep in the in-memory tail buffer
/// for inclusion in error messages. Anything beyond this lives in the
/// on-disk log file only. 8 KiB is enough to capture a Rust panic with
/// a backtrace; bigger means a single failed scan with a chatty
/// engine could blow up the heap.
const STDERR_TAIL_CAP: usize = 8 * 1024;

/// Inputs to one subprocess invocation. Borrows everything from the
/// caller — this struct doesn't outlive a single function call.
pub struct SubprocessSpec<'a> {
    pub bin: &'a Path,
    pub args: &'a [std::ffi::OsString],
    pub stage: Stage,
    pub language: Option<String>,
    pub stderr_log_path: PathBuf,
    pub progress: &'a mut (dyn ProgressSink + Send),
    pub cancel: &'a CancellationToken,
}

/// Outcome of one subprocess invocation. The caller decides what to do
/// with `success == false` (typically: build a typed `RunError` variant
/// that includes the stderr log path).
pub struct SubprocessOutcome {
    pub success: bool,
    pub exit_code: i32,
    pub stderr_tail: String,
}

pub async fn run_with_progress(spec: SubprocessSpec<'_>) -> Result<SubprocessOutcome, RunError> {
    let SubprocessSpec {
        bin,
        args,
        stage,
        language,
        stderr_log_path,
        progress,
        cancel,
    } = spec;

    let mut command = Command::new(bin);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(RunError::Io)?;

    let stdout = child
        .stdout
        .take()
        .expect("stdout piped on spawn() — this is enforced above");
    let stderr = child
        .stderr
        .take()
        .expect("stderr piped on spawn() — this is enforced above");

    // Phase 1: pre-buffer both streams.
    //
    // The parser emits a JSONL progress stream to stdout; if we don't
    // drain it the kernel pipe (~64 KiB on Linux) fills and the child
    // blocks. We pull both streams into bounded in-memory buffers using
    // tokio tasks so neither pipe can stall the child, then replay them
    // through the sink in arrival order on the foreground task.
    //
    // Why not forward to the sink directly from the drain tasks? The
    // sink is `&mut` borrowed by the caller for the lifetime of this
    // call. Crossing it into a tokio task would require `Arc<Mutex<…>>`
    // and the per-event lock cost is real for a pipeline that emits
    // thousands of events. The buffer-then-replay approach keeps the
    // hot path lock-free.
    let (stdout_tx, stdout_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (stderr_tx, stderr_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    let stdout_task = tokio::spawn(forward_lines(stdout, stdout_tx));
    let stderr_task = tokio::spawn(drain_stderr_to_disk_and_channel(
        stderr,
        stderr_log_path,
        stderr_tx,
    ));

    let replay = replay_streams(stdout_rx, stderr_rx, stage, language.as_deref(), progress);

    let outcome = tokio::select! {
        result = wait_with_replay(&mut child, replay, stdout_task, stderr_task) => result,
        _ = cancel.cancelled() => {
            // Best-effort kill; ignore the result because the child
            // may have already exited between the cancel arrival and
            // here, and there's nothing useful to do with the kill error.
            let _ = child.start_kill();
            // Drain whatever output is buffered so cancel doesn't leave
            // a half-written log file.
            let _ = child.wait().await;
            return Err(RunError::Cancelled);
        }
    };

    outcome
}

/// Awaits the child's exit and pairs it with the replay completion. We
/// have to await replay even on the success path so the log file is
/// fully flushed and the sink has seen the last line.
async fn wait_with_replay<F>(
    child: &mut Child,
    replay: F,
    stdout_task: tokio::task::JoinHandle<()>,
    stderr_task: tokio::task::JoinHandle<Result<String, RunError>>,
) -> Result<SubprocessOutcome, RunError>
where
    F: std::future::Future<Output = ()>,
{
    // Replay first so every line lands on disk + sink before the child
    // exits and we read status. The replay future completes when both
    // mpsc channels close (i.e. both drain tasks finished).
    let (_replay_done, _stdout_done, stderr_result, wait_result) =
        tokio::join!(replay, stdout_task, stderr_task, child.wait());

    let stderr_tail = stderr_result
        .map_err(|e| RunError::Io(std::io::Error::other(format!("stderr drain join: {e}"))))??;
    let status = wait_result.map_err(RunError::Io)?;
    let exit_code = status.code().unwrap_or(-1);
    Ok(SubprocessOutcome {
        success: status.success(),
        exit_code,
        stderr_tail,
    })
}

/// Forward every line from a child stream into an mpsc channel. Drops
/// the channel on EOF so the consumer's `recv()` returns `None`. Errors
/// reading the stream are swallowed — they are vanishingly rare and the
/// parent's `wait()` will still surface a non-zero exit code if the
/// child died unhealthily mid-output.
async fn forward_lines<R>(stream: R, tx: tokio::sync::mpsc::UnboundedSender<String>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        if tx.send(line).is_err() {
            break; // receiver dropped; nothing to do
        }
    }
}

/// Drain stderr into a log file *and* an mpsc channel for sink replay.
/// Returns the in-memory tail used in error messages.
async fn drain_stderr_to_disk_and_channel(
    stderr: tokio::process::ChildStderr,
    log_path: PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<String, RunError> {
    let mut log = tokio::fs::File::create(&log_path)
        .await
        .map_err(RunError::Io)?;
    let mut tail = String::new();

    let mut reader = BufReader::new(stderr).lines();
    while let Some(line) = reader.next_line().await.map_err(RunError::Io)? {
        log.write_all(line.as_bytes()).await.map_err(RunError::Io)?;
        log.write_all(b"\n").await.map_err(RunError::Io)?;

        push_with_cap(&mut tail, &line);

        if tx.send(line).is_err() {
            // Receiver dropped; keep draining to disk + tail for the
            // error path, but stop forwarding.
            break;
        }
    }

    log.flush().await.map_err(RunError::Io)?;
    Ok(tail)
}

/// Drain both stream channels into the progress sink. We poll both
/// receivers in a `select!` so events arrive in roughly the order they
/// were written by the child (best-effort interleaving — the kernel
/// already buffers each pipe independently, so strict ordering across
/// the two streams is impossible without timestamps in the child).
async fn replay_streams(
    mut stdout_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    mut stderr_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    stage: Stage,
    language: Option<&str>,
    progress: &mut (dyn ProgressSink + Send),
) {
    loop {
        tokio::select! {
            line = stdout_rx.recv() => match line {
                Some(line) => forward_line(progress, stage, language, line),
                None => {
                    // stdout closed; drain stderr to completion.
                    while let Some(line) = stderr_rx.recv().await {
                        forward_line(progress, stage, language, line);
                    }
                    return;
                }
            },
            line = stderr_rx.recv() => match line {
                Some(line) => forward_line(progress, stage, language, line),
                None => {
                    // stderr closed; drain stdout to completion.
                    while let Some(line) = stdout_rx.recv().await {
                        forward_line(progress, stage, language, line);
                    }
                    return;
                }
            },
        }
    }
}

/// Decide whether `line` is a structured engine event or unstructured
/// noise, then emit the matching [`ProgressEvent`].
///
/// This is the single place where the wire format gets parsed. Structured
/// events become [`ProgressEvent::Engine`] so a consumer can match on
/// the typed `EngineEvent` payload and render real percentages /
/// file-by-file progress. Anything else (tracing logs, panics, stack
/// traces, the analyzer's legacy `[ge-analyze] Running ...` lines)
/// falls through to [`ProgressEvent::Raw`].
///
/// Both branches preserve `stage` and `language` from the runner's call
/// site — the engine itself doesn't always know which language pass it's
/// running (the parser does; the analyzer doesn't, by design), so this
/// metadata is supplied by the runner.
fn forward_line(
    progress: &mut (dyn ProgressSink + Send),
    stage: Stage,
    language: Option<&str>,
    line: String,
) {
    if let Some(event) = try_parse_line(&line) {
        progress.on_event(ProgressEvent::Engine {
            stage,
            language: language.map(|s| s.to_string()),
            event,
        });
    } else {
        progress.on_event(ProgressEvent::Raw {
            stage,
            language: language.map(|s| s.to_string()),
            line,
        });
    }
}

/// Append `line` (plus newline) to `tail`, dropping characters from the
/// front when capacity is exceeded. Operates on `char` boundaries so
/// the resulting `String` stays valid UTF-8 even when the trim point
/// would split a multi-byte sequence.
fn push_with_cap(tail: &mut String, line: &str) {
    tail.push_str(line);
    tail.push('\n');
    while tail.len() > STDERR_TAIL_CAP {
        // Drop the first character (a few bytes for ASCII, up to 4 for
        // multi-byte). This is O(n) per drop but cap-bounded; in
        // practice it converges in a single pass after a long line.
        let next = tail
            .char_indices()
            .nth(1)
            .map(|(idx, _)| idx)
            .unwrap_or(tail.len());
        tail.drain(..next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_cap_keeps_string_under_limit() {
        let mut tail = String::new();
        for _ in 0..1000 {
            push_with_cap(&mut tail, &"x".repeat(100));
        }
        assert!(tail.len() <= STDERR_TAIL_CAP);
    }

    #[test]
    fn tail_cap_handles_multibyte_chars() {
        let mut tail = String::new();
        // Each "🦀" is 4 bytes in UTF-8.
        for _ in 0..3000 {
            push_with_cap(&mut tail, "🦀");
        }
        assert!(tail.len() <= STDERR_TAIL_CAP);
        // Must still be valid UTF-8 (would panic otherwise).
        let _ = tail.chars().count();
    }
}
