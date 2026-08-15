use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::Manager;

struct ToolPaths {
    ffmpeg: String,
    ffprobe: String,
}

static TOOLS: OnceLock<ToolPaths> = OnceLock::new();

/// Prefer the ffmpeg copied into the .app, then Homebrew. Finder-launched
/// apps do not inherit a shell PATH, so the bundled copy is what makes
/// double-click work on a machine that never ran `brew install`.
pub fn init_tools(app: &tauri::AppHandle) {
    let _ = TOOLS.get_or_init(|| resolve_tools(Some(app)));
}

fn resolve_tools(app: Option<&tauri::AppHandle>) -> ToolPaths {
    let bundled = app.and_then(bundled_ffbin);
    ToolPaths {
        ffmpeg: pick_tool("ffmpeg", bundled.as_deref()),
        ffprobe: pick_tool("ffprobe", bundled.as_deref()),
    }
}

fn bundled_ffbin(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(resource) = app.path().resource_dir() {
        dirs.push(resource.join("ffbin"));
        dirs.push(resource.join("resources").join("ffbin"));
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/ffbin"));
    dirs.into_iter().find(|dir| dir.join("ffmpeg").is_file())
}

fn pick_tool(name: &str, bundled_dir: Option<&Path>) -> String {
    if let Some(dir) = bundled_dir {
        let candidate = dir.join(name);
        if tool_works(&candidate.to_string_lossy()) {
            return candidate.to_string_lossy().into_owned();
        }
    }
    find_tool(name)
}

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
    TOOLS
        .get()
        .map(|tools| tools.ffmpeg.clone())
        .unwrap_or_else(|| find_tool("ffmpeg"))
}

pub fn ffprobe_path() -> String {
    TOOLS
        .get()
        .map(|tools| tools.ffprobe.clone())
        .unwrap_or_else(|| find_tool("ffprobe"))
}

fn tool_works(path: &str) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaToolsStatus {
    pub ffmpeg: bool,
    pub ffprobe: bool,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
}

/// Used on the welcome screen so a missing ffmpeg is a clear message
/// instead of a spawn error after the user drops a movie.
#[tauri::command]
pub fn check_media_tools(app: tauri::AppHandle) -> MediaToolsStatus {
    init_tools(&app);
    let ffmpeg_path = ffmpeg_path();
    let ffprobe_path = ffprobe_path();
    MediaToolsStatus {
        ffmpeg: tool_works(&ffmpeg_path),
        ffprobe: tool_works(&ffprobe_path),
        ffmpeg_path,
        ffprobe_path,
    }
}

/// Stop in-flight ffmpeg/ffprobe work when the user closes a movie.
#[tauri::command]
pub fn cancel_jobs() {
    kill_all_children();
    clear_jobs();
}

/// Everything the analysis passes need from their environment: somewhere to
/// cache per-movie results, somewhere to keep downloaded models, and a way to
/// report progress. Implemented by the Tauri app at runtime and by
/// [`HeadlessHost`] so the same detectors can run from `scan_report` without a
/// GUI — which is what makes threshold calibration a terminal loop.
pub trait ScanHost: Send + Sync {
    fn cache_root(&self) -> Result<PathBuf, String>;
    fn models_dir(&self) -> Result<PathBuf, String>;
    fn emit(&self, event: &str, payload: serde_json::Value);
}

impl ScanHost for tauri::AppHandle {
    fn cache_root(&self) -> Result<PathBuf, String> {
        self.path().app_cache_dir().map_err(|e| e.to_string())
    }
    fn models_dir(&self) -> Result<PathBuf, String> {
        Ok(self
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("models"))
    }
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        let _ = Emitter::emit(self, event, payload);
    }
}

/// Bundle identifier, and therefore the name of the app's cache and data
/// directories. Must match `identifier` in tauri.conf.json — when these drifted
/// apart the harness silently used a parallel directory and the app
/// re-downloaded every model.
pub const BUNDLE_IDENTIFIER: &str = "com.jaco.videofy";

/// Host for command-line runs. Reuses the app's real cache and model
/// directories so a headless scan warms the same caches the GUI reads.
pub struct HeadlessHost {
    cache: PathBuf,
    models: PathBuf,
    pub verbose: bool,
    /// Last whole percent reported, so piping the output does not produce one
    /// line per progress tick.
    last_pct: std::sync::atomic::AtomicU8,
}

impl HeadlessHost {
    pub fn new(verbose: bool) -> Result<Self, String> {
        let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
        let base = PathBuf::from(home).join("Library");
        Ok(HeadlessHost {
            cache: base.join("Caches").join(BUNDLE_IDENTIFIER),
            models: base
                .join("Application Support")
                .join(BUNDLE_IDENTIFIER)
                .join("models"),
            verbose,
            last_pct: std::sync::atomic::AtomicU8::new(u8::MAX),
        })
    }
}

impl ScanHost for HeadlessHost {
    fn cache_root(&self) -> Result<PathBuf, String> {
        Ok(self.cache.clone())
    }
    fn models_dir(&self) -> Result<PathBuf, String> {
        Ok(self.models.clone())
    }
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if !self.verbose {
            return;
        }
        let Some(pct) = payload.get("pct").and_then(|v| v.as_f64()) else {
            return;
        };
        let whole = pct.clamp(0.0, 100.0) as u8;
        if self
            .last_pct
            .swap(whole, std::sync::atomic::Ordering::Relaxed)
            == whole
        {
            return;
        }
        eprint!("\r  {event}: {whole:3}%    ");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }
}

/// Per-source cache directory keyed by path + size + mtime, so proxies and
/// analysis survive app restarts but invalidate if the file changes.
pub fn cache_dir_for(host: &dyn ScanHost, input: &str) -> Result<PathBuf, String> {
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
    let dir = host.cache_root()?.join("media").join(&digest[..16]);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn child_pids() -> &'static Mutex<HashSet<u32>> {
    static PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Kill any ffmpeg/ffprobe children still running (called on app exit and
/// when the user hits New movie, so a half-finished proxy or Whisper pass
/// does not linger into the next film).
pub fn kill_all_children() {
    let pids: Vec<u32> = child_pids()
        .lock()
        .map(|s| s.iter().copied().collect())
        .unwrap_or_default();
    for pid in pids {
        #[cfg(unix)]
        {
            // spawn() puts each child in its own process group so -PID
            // reaps ffmpeg helpers too.
            let _ = Command::new("kill")
                .args(["-9", &format!("-{pid}")])
                .status();
        }
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    if let Ok(mut set) = child_pids().lock() {
        set.clear();
    }
}

/// Tracks which cache dirs have a job running, so a duplicate import can't
/// start a second ffmpeg racing on the same output files. Values are
/// generation ids so a cancelled JobGuard cannot clear a newer run of the
/// same movie.
fn jobs_in_flight() -> &'static Mutex<HashMap<String, u64>> {
    static JOBS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_job_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub struct JobGuard {
    key: String,
    id: u64,
}

impl JobGuard {
    pub fn acquire(key: String) -> Result<Self, String> {
        let mut jobs = jobs_in_flight().lock().map_err(|e| e.to_string())?;
        if jobs.contains_key(&key) {
            return Err("This movie is already being processed.".into());
        }
        let id = next_job_id();
        jobs.insert(key.clone(), id);
        Ok(JobGuard { key, id })
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        if let Ok(mut jobs) = jobs_in_flight().lock() {
            if jobs.get(&self.key) == Some(&self.id) {
                jobs.remove(&self.key);
            }
        }
    }
}

fn clear_jobs() {
    if let Ok(mut jobs) = jobs_in_flight().lock() {
        jobs.clear();
    }
}

pub fn spawn(cmd: &str, args: &[&str]) -> Result<Child, String> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("failed to start {cmd}: {e}"))?;
    if let Ok(mut pids) = child_pids().lock() {
        pids.insert(child.id());
    }
    Ok(child)
}

pub fn run_output(cmd: &str, args: &[&str]) -> Result<Output, String> {
    let child = spawn(cmd, args)?;
    let pid = child.id();
    let output = child.wait_with_output().map_err(|e| e.to_string());
    if let Ok(mut pids) = child_pids().lock() {
        pids.remove(&pid);
    }
    output
}

/// Drains stderr on its own thread while the caller reads stdout. Without
/// this, a chatty ffmpeg fills the 64KB stderr pipe, blocks on write, and the
/// whole job deadlocks. Returns the last few lines for error reporting.
pub fn drain_stderr(child: &mut Child) -> Option<std::thread::JoinHandle<String>> {
    let stderr = child.stderr.take()?;
    Some(std::thread::spawn(move || {
        use std::io::BufRead;
        let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
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
    let tail = stderr_drain.and_then(|h| h.join().ok()).unwrap_or_default();
    let status = status?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{context} failed:\n{tail}"))
    }
}

#[cfg(test)]
mod tests {
    /// The harness and the app must agree on where caches and models live.
    /// They did not, and the app silently re-downloaded 156 MB of models.
    #[test]
    fn identifier_matches_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid config");
        assert_eq!(
            config["identifier"].as_str(),
            Some(super::BUNDLE_IDENTIFIER),
            "BUNDLE_IDENTIFIER must match tauri.conf.json"
        );
    }
}
