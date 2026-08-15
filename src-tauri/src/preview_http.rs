use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use tauri::{AppHandle, Manager};

struct Server {
    port: u16,
    cache_root: PathBuf,
    current: Mutex<Option<PathBuf>>,
}

static SERVER: OnceLock<Server> = OnceLock::new();
static START: Mutex<()> = Mutex::new(());

/// Serve the preview over loopback HTTP with byte ranges.
///
/// WKWebView's `asset://` handler has gone fully black on large MP4s in this
/// app (the HTML chrome disappears with the video). Native HTTP Range requests
/// keep the decoder in the media stack instead of the custom-protocol path.
pub fn url_for(app: &AppHandle, file: &Path) -> Result<String, String> {
    let server = ensure_started(app)?;
    let canonical = file
        .canonicalize()
        .map_err(|e| format!("preview path: {e}"))?;
    if !canonical.starts_with(&server.cache_root) {
        return Err("preview file is outside the app cache".into());
    }
    *server
        .current
        .lock()
        .map_err(|e| e.to_string())? = Some(canonical);
    Ok(format!("http://127.0.0.1:{}/preview.mp4", server.port))
}

fn ensure_started(app: &AppHandle) -> Result<&'static Server, String> {
    if let Some(server) = SERVER.get() {
        return Ok(server);
    }
    let _lock = START.lock().map_err(|e| e.to_string())?;
    if let Some(server) = SERVER.get() {
        return Ok(server);
    }
    let cache_root = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    let cache_root = cache_root.canonicalize().unwrap_or(cache_root);
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("preview http bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    SERVER
        .set(Server {
            port,
            cache_root,
            current: Mutex::new(None),
        })
        .map_err(|_| "preview server already running".to_string())?;
    thread::Builder::new()
        .name("preview-http".into())
        .spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { continue };
                let _ = thread::Builder::new()
                    .name("preview-http-conn".into())
                    .spawn(move || serve(stream));
            }
        })
        .map_err(|e| e.to_string())?;
    SERVER
        .get()
        .ok_or_else(|| "preview server missing after start".into())
}

fn serve(mut stream: TcpStream) {
    let mut buf = [0u8; 8192];
    let n = match stream.read(&mut buf) {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let req = String::from_utf8_lossy(&buf[..n]);
    let Some(first) = req.lines().next() else { return };
    if first.starts_with("OPTIONS ") {
        let _ = stream.write_all(
            b"HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Range\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    let is_head = first.starts_with("HEAD ");
    if !is_head && !first.starts_with("GET ") {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n");
        return;
    }

    let Some(server) = SERVER.get() else { return };
    let file_path = server.current.lock().ok().and_then(|g| g.clone());
    let Some(file_path) = file_path else {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
        return;
    };

    let mut file = match File::open(&file_path) {
        Ok(f) => f,
        Err(_) => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
            return;
        }
    };
    let total = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if total == 0 {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
        return;
    }

    let range_header = req.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("range") {
            Some(value.trim().to_string())
        } else {
            None
        }
    });
    let range = range_header
        .as_deref()
        .and_then(|value| parse_range(value, total));

    let (status, start, end) = match range {
        Some((start, end)) if start <= end && start < total => ("206 Partial Content", start, end),
        Some(_) => {
            let body = format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(body.as_bytes());
            return;
        }
        None => ("200 OK", 0, total - 1),
    };
    let length = end - start + 1;
    if file.seek(SeekFrom::Start(start)).is_err() {
        return;
    }

    let mut headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nAccept-Ranges: bytes\r\nContent-Length: {length}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n"
    );
    if status.starts_with("206") {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    headers.push_str("\r\n");
    if stream.write_all(headers.as_bytes()).is_err() {
        return;
    }
    if is_head {
        return;
    }

    let mut remaining = length;
    let mut chunk = [0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(chunk.len() as u64) as usize;
        let read = match file.read(&mut chunk[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if stream.write_all(&chunk[..read]).is_err() {
            break;
        }
        remaining -= read as u64;
    }
}

fn parse_range(header: &str, file_len: u64) -> Option<(u64, u64)> {
    let spec = header.trim();
    let spec = spec.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if file_len == 0 {
        return None;
    }
    if start_s.is_empty() {
        let suffix: u64 = end_s.parse().ok()?;
        let start = file_len.saturating_sub(suffix);
        return Some((start, file_len - 1));
    }
    let start: u64 = start_s.parse().ok()?;
    let end = if end_s.is_empty() {
        file_len - 1
    } else {
        end_s.parse().ok()?
    };
    Some((start, end.min(file_len - 1)))
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn range_open_end() {
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
    }

    #[test]
    fn range_closed() {
        assert_eq!(parse_range("bytes=0-1023", 5000), Some((0, 1023)));
    }

    #[test]
    fn range_suffix() {
        assert_eq!(parse_range("bytes=-50", 200), Some((150, 199)));
    }
}
