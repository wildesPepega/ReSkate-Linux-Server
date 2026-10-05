// The server's status as JSON over HTTP, on the game port's number: the game uses UDP, this TCP,
// so a hosting panel's one allocation (which opens both) is enough. GET /status answers with the
// last snapshot the main loop handed over (set()); this thread never touches the game itself.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Requests answered at once; more are turned away rather than queued.
const MAX_CLIENTS: usize = 16;
// The longest request read: a GET line and its headers.
const MAX_REQUEST: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(3);

pub struct StatusServer {
    body: Arc<Mutex<Arc<String>>>,
}

impl StatusServer {
    // Listens on TCP `port` on every address; the error says why it could not.
    pub fn start(port: u16) -> Result<StatusServer, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| e.to_string())?;
        let body = Arc::new(Mutex::new(Arc::new(String::from("{}"))));
        let shared = body.clone();
        let clients = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new()
            .name("status".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    if clients.fetch_add(1, Ordering::SeqCst) >= MAX_CLIENTS {
                        clients.fetch_sub(1, Ordering::SeqCst);
                        continue; // dropped: the connection closes
                    }
                    let body = shared.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let counter = clients.clone();
                    let spawned = std::thread::Builder::new().name("status-client".into()).spawn(move || {
                        answer(stream, &body);
                        counter.fetch_sub(1, Ordering::SeqCst);
                    });
                    if spawned.is_err() {
                        clients.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(StatusServer { body })
    }

    // What the next requests get.
    pub fn set(&self, json: String) {
        *self.body.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(json);
    }
}

fn answer(mut stream: TcpStream, body: &str) {
    let _ = stream.set_read_timeout(Some(TIMEOUT));
    let _ = stream.set_write_timeout(Some(TIMEOUT));
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < MAX_REQUEST {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buffer[..n]),
        }
    }
    let _ = stream.write_all(&response(&request, body));
    let _ = stream.flush();
}

// The whole HTTP response to `request` (raw bytes as read).
pub fn response(request: &[u8], body: &str) -> Vec<u8> {
    let line = request.split(|&c| c == b'\n').next().unwrap_or_default();
    let line = String::from_utf8_lossy(line);
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let path = target.split(['?', '#']).next().unwrap_or("");
    let (status, content, text) = if method != "GET" && method != "HEAD" {
        ("405 Method Not Allowed", "text/plain", "Only GET is supported.\n")
    } else if path == "/status" || path == "/status.json" {
        ("200 OK", "application/json", body)
    } else {
        ("404 Not Found", "text/plain", "Not found. The status is at /status.\n")
    };
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content}; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n",
        text.len()
    );
    if status.starts_with("405") {
        out.push_str("Allow: GET, HEAD\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    if method != "HEAD" {
        bytes.extend_from_slice(text.as_bytes());
    }
    bytes
}
