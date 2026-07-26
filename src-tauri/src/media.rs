use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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

pub fn spawn(cmd: &str, args: &[&str]) -> Result<Child, String> {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start {cmd}: {e}"))
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

pub fn wait_checked(mut child: Child, context: &str) -> Result<(), String> {
    let mut stderr_tail = String::new();
    if let Some(err) = child.stderr.take() {
        use std::io::Read;
        let mut buf = String::new();
        let mut reader = std::io::BufReader::new(err);
        let _ = reader.read_to_string(&mut buf);
        let tail: Vec<&str> = buf.lines().rev().take(8).collect();
        stderr_tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{context} failed:\n{stderr_tail}"))
    }
}
