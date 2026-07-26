use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::Manager;

/// GUI apps launched from Finder don't inherit the shell PATH, so fall back to
/// the common Homebrew/MacPorts install locations.
fn find_tool(name: &str) -> String {
    if let Ok(path) = which(name) {
        return path;
    }
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
        let candidate = format!("{dir}/{name}");
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    name.to_string()
}

fn which(name: &str) -> Result<String, ()> {
    let out = Command::new("which").arg(name).output().map_err(|_| ())?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Ok(s);
        }
    }
    Err(())
}

pub fn ffmpeg_path() -> String {
    find_tool("ffmpeg")
}

pub fn ffprobe_path() -> String {
    find_tool("ffprobe")
}

/// Per-source cache directory keyed by path + size + mtime, so proxies and
/// analysis survive app restarts but invalidate if the file changes.
pub fn cache_dir_for(app: &tauri::AppHandle, input: &str) -> Result<PathBuf, String> {
    let meta = std::fs::metadata(input).map_err(|e| format!("cannot stat {input}: {e}"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut h = sha1_smol::Sha1::new();
    h.update(format!("{input}|{}|{mtime}", meta.len()).as_bytes());
    let digest = h.digest().to_string();
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("media")
        .join(&digest[..16]);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn child_pids() -> &'static Mutex<HashSet<u32>> {
    static PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Kill any ffmpeg/ffprobe children still running (called on app exit so a
/// half-finished proxy or export doesn't linger as an orphan).
pub fn kill_all_children() {
    let pids: Vec<u32> = child_pids().lock().map(|s| s.iter().copied().collect()).unwrap_or_default();
    for pid in pids {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

/// Tracks which cache dirs have a job running, so a duplicate import can't
/// start a second ffmpeg racing on the same output files.
fn jobs_in_flight() -> &'static Mutex<HashSet<String>> {
    static JOBS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub struct JobGuard(String);

impl JobGuard {
    pub fn acquire(key: String) -> Result<Self, String> {
        let mut jobs = jobs_in_flight().lock().map_err(|e| e.to_string())?;
        if !jobs.insert(key.clone()) {
            return Err("This movie is already being processed.".into());
        }
        Ok(JobGuard(key))
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut jobs) = jobs_in_flight().lock() {
            jobs.remove(&self.0);
        }
    }
}

pub fn spawn(cmd: &str, args: &[&str]) -> Result<Child, String> {
    let child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start {cmd}: {e}"))?;
    if let Ok(mut pids) = child_pids().lock() {
        pids.insert(child.id());
    }
    Ok(child)
}

/// Drains stderr on its own thread while the caller reads stdout. Without
/// this, a chatty ffmpeg fills the 64KB stderr pipe, blocks on write, and the
/// whole job deadlocks. Returns the last few lines for error reporting.
pub fn drain_stderr(child: &mut Child) -> Option<std::thread::JoinHandle<String>> {
    let stderr = child.stderr.take()?;
    Some(std::thread::spawn(move || {
        use std::io::BufRead;
        let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if tail.len() >= 10 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        tail.into_iter().collect::<Vec<_>>().join("\n")
    }))
}

/// Reads `-progress pipe:1` key=value output from ffmpeg stdout and reports
/// the current output timestamp in seconds.
pub fn read_progress<R: std::io::BufRead>(reader: R, mut on_time: impl FnMut(f64)) {
    for line in reader.lines().map_while(Result::ok) {
        if let Some(v) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = v.trim().parse::<i64>() {
                if us >= 0 {
                    on_time(us as f64 / 1_000_000.0);
                }
            }
        }
    }
}

pub fn wait_checked(
    mut child: Child,
    context: &str,
    stderr_drain: Option<std::thread::JoinHandle<String>>,
) -> Result<(), String> {
    let pid = child.id();
    let status = child.wait().map_err(|e| e.to_string());
    if let Ok(mut pids) = child_pids().lock() {
        pids.remove(&pid);
    }
    let tail = stderr_drain
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{context} failed:\n{tail}"))
    }
}
