//! OpenAI Responses API with live streaming, for commit messages: a ChatGPT
//! subscription (OAuth, the same flow and client as `codex login`) or an API
//! key. This is mira's only network access, and it only runs on request.
//!
//! Requests go through `curl`, so mira links no HTTP or TLS stack. Secrets
//! reach curl on stdin (`--config -`), never on its command line, where other
//! processes could read them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const ISSUER: &str = "https://auth.openai.com";
/// Codex CLI's public OAuth client, as used by `codex login` and opencode.
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const SCOPE: &str = "openid profile email offline_access api.connectors.read api.connectors.invoke";
const CALLBACK_PORTS: [u16; 2] = [1455, 1457];
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const CHATGPT_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const API_URL: &str = "https://api.openai.com/v1/responses";
const STREAM_MAX_SECS: u32 = 120;
const TOKEN_MAX_SECS: u32 = 30;
const REFRESH_MARGIN_SECS: u64 = 60;
const SUCCESS_HTML: &str = "<!doctype html><html><body style=\"font-family:sans-serif\"><p>Signed in to ChatGPT. \
                            You can close this window and go back to mira.</p><script>window.close()</script></body></html>";

// ------------------------------------------------------------------ tokens

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub account_id: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

fn tokens_path() -> Option<PathBuf> {
    crate::persist::state_dir().map(|d| d.join("chatgpt.json"))
}

pub fn load_tokens() -> Option<Tokens> {
    let on_disk = || -> Option<Tokens> { serde_json::from_str(&std::fs::read_to_string(tokens_path()?).ok()?).ok() };
    // Rotated but not written yet: newer than the file (which the server no longer
    // takes), unless another mira refreshed and wrote a newer pair since.
    let kept = unsaved().clone();
    if let Some(t) = kept {
        if let Some(file) = on_disk().filter(|f| f.expires_at > t.expires_at) {
            *unsaved() = None;
            return Some(file);
        }
        let _ = save_tokens(&t);
        return Some(t);
    }
    on_disk()
}

/// Tokens a refresh got but couldn't write (disk full, permissions). The
/// server already retired the old refresh token, so losing these would end
/// the session: they're kept in memory and written when the disk allows.
static UNSAVED: std::sync::Mutex<Option<Tokens>> = std::sync::Mutex::new(None);

fn unsaved() -> std::sync::MutexGuard<'static, Option<Tokens>> {
    UNSAVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn save_tokens(tokens: &Tokens) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(tokens).map_err(|e| e.to_string())?;
    crate::persist::write_private(&tokens_path().ok_or("no state directory")?, &json)?;
    *unsaved() = None;
    Ok(())
}

pub fn sign_out() -> Result<(), String> {
    *unsaved() = None;
    match tokens_path().map(std::fs::remove_file) {
        Some(Err(e)) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// The signed-in account (its email, or empty when the token has none).
pub fn account() -> Option<String> {
    load_tokens().map(|t| t.email.unwrap_or_default())
}

fn jwt_claims(jwt: &str) -> Option<Value> {
    serde_json::from_slice(&b64url_decode(jwt.split('.').nth(1)?)?).ok()
}

/// Tokens from a token-endpoint reply. A refresh may omit fields that did not
/// rotate; those are kept from `previous`.
fn tokens_from(v: &Value, previous: Option<&Tokens>) -> Result<Tokens, String> {
    let field = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let access_token = field("access_token").ok_or("the token reply has no access_token")?;
    let refresh_token = field("refresh_token")
        .or_else(|| previous.map(|p| p.refresh_token.clone()))
        .ok_or("the token reply has no refresh_token")?;
    let id_token =
        field("id_token").or_else(|| previous.map(|p| p.id_token.clone())).ok_or("the token reply has no id_token")?;
    let claims = jwt_claims(&id_token);
    let auth = claims.as_ref().and_then(|c| c.get("https://api.openai.com/auth"));
    let account_id = auth
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| previous.map(|p| p.account_id.clone()))
        .ok_or("could not find the ChatGPT account id in the sign-in reply")?;
    let email = claims
        .as_ref()
        .and_then(|c| c.get("email"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| previous.and_then(|p| p.email.clone()));
    let expires_at = jwt_claims(&access_token)
        .and_then(|c| c.get("exp").and_then(Value::as_u64))
        .unwrap_or_else(|| crate::persist::now() + 3600);
    Ok(Tokens { access_token, refresh_token, id_token, account_id, email, expires_at })
}

fn token_request(body: Vec<u8>, content_type: &str, cancel: &AtomicBool) -> Result<(u16, Value), String> {
    let url = format!("{ISSUER}/oauth/token");
    let reply = post(&url, &[("Content-Type", content_type)], &body, TOKEN_MAX_SECS, cancel, &mut |_| {})?;
    let v: Value = serde_json::from_str(&reply.body)
        .map_err(|_| format!("the sign-in server returned {}: {}", reply.status, snippet(&reply.body)))?;
    Ok((reply.status, v))
}

/// Exchange the refresh token. When the server rejects it the session is over:
/// the stored tokens are removed so the next generation signs in again.
pub fn refresh(tokens: &Tokens, cancel: &AtomicBool) -> Result<Tokens, String> {
    let body = json!({ "grant_type": "refresh_token", "client_id": CLIENT_ID, "refresh_token": tokens.refresh_token });
    let (status, v) = token_request(body.to_string().into_bytes(), "application/json", cancel)?;
    if matches!(status, 400 | 401 | 403) {
        let _ = sign_out();
        return Err("your ChatGPT session expired — click ✦ to sign in again".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("refreshing the ChatGPT session failed ({status}): {}", error_message(&v)));
    }
    let fresh = tokens_from(&v, Some(tokens))?;
    if fresh.account_id != tokens.account_id {
        let _ = sign_out();
        return Err("the refreshed session belongs to another ChatGPT account — sign in again".into());
    }
    // A failed write mustn't lose the rotated refresh token (see `UNSAVED`).
    if save_tokens(&fresh).is_err() {
        *unsaved() = Some(fresh.clone());
    }
    Ok(fresh)
}

/// `tokens`, refreshed first when the access token is about to expire.
pub fn fresh(tokens: Tokens, cancel: &AtomicBool) -> Result<Tokens, String> {
    if crate::persist::now() + REFRESH_MARGIN_SECS < tokens.expires_at { Ok(tokens) } else { refresh(&tokens, cancel) }
}

// ------------------------------------------------------------------ login

/// Browser sign-in (OAuth PKCE with a localhost callback, like `codex login`).
/// Blocks until the browser comes back, `cancel` is set or 5 minutes pass.
/// `on_url` gets the sign-in link and whether a browser was opened for it, so
/// the user can open it by hand.
pub fn login(cancel: &AtomicBool, on_url: &mut dyn FnMut(&str, bool)) -> Result<Tokens, String> {
    let listener = CALLBACK_PORTS
        .iter()
        .find_map(|port| TcpListener::bind(("127.0.0.1", *port)).ok())
        .ok_or("ports 1455 and 1457 are busy — close `codex login` (or whatever uses them) and try again")?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let verifier = b64url_encode(&random_bytes(32)?);
    let challenge = b64url_encode(&sha256(verifier.as_bytes()));
    let state = b64url_encode(&random_bytes(24)?);
    let redirect = format!("http://localhost:{port}/auth/callback");
    let url = format!(
        "{ISSUER}/oauth/authorize?{}",
        form_encode(&[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", &redirect),
            ("scope", SCOPE),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
        ])
    );
    on_url(&url, open_browser(&url).is_ok());

    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let code = loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        if Instant::now() > deadline {
            return Err("sign-in timed out after 5 minutes".into());
        }
        match listener.accept() {
            Ok((stream, _)) => match callback(stream, &state) {
                Some(result) => break result?,
                None => continue,
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    drop(listener);

    let body = form_encode(&[
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("code_verifier", &verifier),
    ]);
    let (status, v) = token_request(body.into_bytes(), "application/x-www-form-urlencoded", cancel)?;
    if !(200..300).contains(&status) {
        return Err(format!("sign-in failed ({status}): {}", error_message(&v)));
    }
    let tokens = tokens_from(&v, None)?;
    save_tokens(&tokens)?;
    Ok(tokens)
}

fn open_browser(url: &str) -> Result<(), String> {
    let mut child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not open the browser (xdg-open: {e})"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Handle one request on the callback port. `None` for anything that is not a
/// valid callback (favicon, a stale tab), so the wait goes on.
fn callback(mut stream: TcpStream, state: &str) -> Option<Result<String, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while buf.len() < 16 * 1024 && !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let target = head.lines().next().and_then(|l| l.split(' ').nth(1)).unwrap_or("/");
    let (route, query) = target.split_once('?').unwrap_or((target, ""));
    let mut respond = |status: &str, body: &str| {
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    };
    if route != "/auth/callback" {
        respond("404 Not Found", "Not found");
        return None;
    }
    let params = parse_query(query);
    let get = |k: &str| params.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    // Only this sign-in's redirect may end it (an error carries `state` too,
    // RFC 6749 §4.1.2.1): another page or process can't abort the wait.
    if get("state") != Some(state) {
        respond("400 Bad Request", "Invalid sign-in callback.");
        return None;
    }
    if let Some(error) = get("error") {
        respond("200 OK", "Sign-in failed. You can close this window.");
        return Some(Err(format!("sign-in failed: {}", get("error_description").unwrap_or(error))));
    }
    match get("code") {
        Some(code) if !code.is_empty() => {
            respond("200 OK", SUCCESS_HTML);
            Some(Ok(code.to_string()))
        }
        _ => {
            respond("400 Bad Request", "Invalid sign-in callback.");
            None
        }
    }
}

// ------------------------------------------------------------------ streaming

pub enum Auth<'a> {
    ChatGpt(&'a Tokens),
    ApiKey(&'a str),
}

#[derive(Debug, PartialEq)]
pub enum StreamError {
    /// 401: the caller may refresh the session and retry.
    Unauthorized,
    Other(String),
}

/// Stream a reply from the Responses API. Every text delta goes to `on_delta`;
/// returns the full text.
pub fn stream(
    auth: &Auth,
    model: &str,
    instructions: &str,
    input: &str,
    cancel: &AtomicBool,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String, StreamError> {
    let url = match auth {
        Auth::ChatGpt(_) => CHATGPT_URL,
        Auth::ApiKey(_) => API_URL,
    };
    stream_to(url, auth, model, instructions, input, cancel, on_delta)
}

fn stream_to(
    url: &str,
    auth: &Auth,
    model: &str,
    instructions: &str,
    input: &str,
    cancel: &AtomicBool,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String, StreamError> {
    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": [{ "role": "user", "content": [{ "type": "input_text", "text": input }] }],
        "store": false,
        "stream": true,
    });
    let session = session_id().map_err(StreamError::Other)?;
    let (bearer, mut headers) = match auth {
        Auth::ChatGpt(t) => {
            body["include"] = json!([]);
            (
                format!("Bearer {}", t.access_token),
                vec![
                    ("ChatGPT-Account-Id", t.account_id.clone()),
                    ("originator", "codex_cli_rs".to_string()),
                    ("session-id", session),
                    ("OpenAI-Beta", "responses=experimental".to_string()),
                ],
            )
        }
        Auth::ApiKey(key) => (format!("Bearer {key}"), Vec::new()),
    };
    headers.push(("Authorization", bearer));
    headers.push(("Content-Type", "application/json".into()));
    headers.push(("Accept", "text/event-stream".into()));
    let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let mut sse = Sse::default();
    let mut out = Output::default();
    let reply = post(url, &headers, body.to_string().as_bytes(), STREAM_MAX_SECS, cancel, &mut |line| {
        if let Some(delta) = sse.line(line).and_then(|data| out.event(&data)) {
            on_delta(&delta);
        }
    })
    .map_err(StreamError::Other)?;
    if let Some(data) = sse.finish() {
        out.event(&data);
    }
    let service = match auth {
        Auth::ChatGpt(_) => "ChatGPT",
        Auth::ApiKey(_) => "OpenAI",
    };
    if reply.status == 401 {
        return Err(StreamError::Unauthorized);
    }
    if !(200..300).contains(&reply.status) {
        let detail = serde_json::from_str::<Value>(&reply.body)
            .map(|v| error_message(&v))
            .unwrap_or_else(|_| snippet(&reply.body));
        return Err(StreamError::Other(format!("{service} request failed ({}): {detail}", reply.status)));
    }
    if let Some(failure) = out.failure {
        return Err(StreamError::Other(format!("{service} request failed: {failure}")));
    }
    Ok(out.completed.filter(|t| !t.trim().is_empty()).unwrap_or(out.deltas))
}

/// Server-sent events: collects `data:` lines until a blank line ends the event.
#[derive(Default)]
struct Sse {
    data: String,
    open: bool,
}

impl Sse {
    fn line(&mut self, line: &str) -> Option<String> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.finish();
        }
        if let Some(d) = line.strip_prefix("data:") {
            if self.open {
                self.data.push('\n');
            }
            self.data.push_str(d.strip_prefix(' ').unwrap_or(d));
            self.open = true;
        }
        None
    }

    fn finish(&mut self) -> Option<String> {
        self.open = false;
        let data = std::mem::take(&mut self.data);
        (!data.is_empty() && data != "[DONE]").then_some(data)
    }
}

#[derive(Default)]
struct Output {
    deltas: String,
    completed: Option<String>,
    failure: Option<String>,
}

impl Output {
    /// Apply one event; returns its text delta, if any.
    fn event(&mut self, data: &str) -> Option<String> {
        let v: Value = serde_json::from_str(data).ok()?;
        match v.get("type").and_then(Value::as_str)? {
            "response.output_text.delta" => {
                let d = v.get("delta").and_then(Value::as_str).filter(|d| !d.is_empty())?;
                self.deltas.push_str(d);
                Some(d.to_string())
            }
            "response.completed" => {
                self.completed = completed_text(&v);
                None
            }
            "response.failed" | "error" => {
                self.failure.get_or_insert_with(|| error_message(&v));
                None
            }
            _ => None,
        }
    }
}

fn completed_text(event: &Value) -> Option<String> {
    event.pointer("/response/output")?.as_array()?.iter().find_map(|item| {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            return None;
        }
        let text: String = item
            .get("content")?
            .as_array()?
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("output_text"))
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect();
        (!text.trim().is_empty()).then_some(text)
    })
}

fn error_message(v: &Value) -> String {
    ["/response/error/message", "/error/message", "/error_description", "/error", "/message", "/detail"]
        .iter()
        .find_map(|p| v.pointer(p).and_then(Value::as_str))
        .unwrap_or("unknown error")
        .to_string()
}

fn snippet(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &body[..i]),
        None if body.is_empty() => "(empty reply)".into(),
        None => body.to_string(),
    }
}

// ------------------------------------------------------------------ API key

/// `OPENAI_API_KEY`, or the first line of `~/.config/mira/openai_key`, which
/// must not be readable by others.
pub fn api_key() -> Result<Option<String>, String> {
    if let Some(key) = std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.trim().is_empty()) {
        return Ok(Some(key.trim().to_string()));
    }
    let Some(path) = crate::persist::config_dir().map(|d| d.join("openai_key")) else {
        return Ok(None);
    };
    key_from_file(&path)
}

fn key_from_file(path: &Path) -> Result<Option<String>, String> {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(None);
    };
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("{} holds your API key: run chmod 600 on it", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    Ok(text.lines().next().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string))
}

// ------------------------------------------------------------------ curl

struct Reply {
    status: u16,
    body: String,
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r"))
}

/// The curl config (read from stdin) for a POST of `body_file`.
fn curl_config(url: &str, headers: &[(&str, &str)], body_file: &Path) -> String {
    let mut config = format!(
        "url = {}\nrequest = \"POST\"\ndata-binary = {}\n",
        quote(url),
        quote(&format!("@{}", body_file.display()))
    );
    for (k, v) in headers {
        config.push_str(&format!("header = {}\n", quote(&format!("{k}: {v}"))));
    }
    config
}

fn scratch_dir() -> Result<PathBuf, String> {
    use std::os::unix::fs::DirBuilderExt;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir =
        std::env::temp_dir().join(format!("mira-http-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// POST through curl. Each line of the reply goes to `on_line` as it arrives
/// (for streaming); the whole body is returned with the HTTP status.
fn post(
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    max_secs: u32,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
) -> Result<Reply, String> {
    let dir = scratch_dir()?;
    let result = post_in(&dir, url, headers, body, max_secs, cancel, on_line);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn post_in(
    dir: &Path,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    max_secs: u32,
    cancel: &AtomicBool,
    on_line: &mut dyn FnMut(&str),
) -> Result<Reply, String> {
    let body_file = dir.join("body");
    crate::persist::write_private(&body_file, body)?;
    let child = Command::new("curl")
        .args(["-sS", "-N", "--connect-timeout", "15", "--max-time", &max_secs.to_string()])
        .args(["-w", "%{stderr}\n%{http_code}", "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run curl ({e}); mira needs it for network requests"))?;
    let config = curl_config(url, headers, &body_file).into_bytes();
    // curl's own --max-time ends the request; this limit only guards against a hung curl.
    let limit = Duration::from_secs(u64::from(max_secs) + 15);
    let mut text = String::new();
    let mut pending = Vec::new();
    let mut line_out = |line: &[u8]| {
        let line = String::from_utf8_lossy(line);
        on_line(&line);
        text.push_str(&line);
        text.push('\n');
    };
    let done = crate::proc::stream(child, config, cancel, limit, &mut |bytes| {
        pending.extend_from_slice(bytes);
        while let Some(i) = pending.iter().position(|&b| b == b'\n') {
            line_out(&pending[..i]);
            pending.drain(..=i);
        }
    })?;
    if !pending.is_empty() {
        line_out(&pending);
    }
    let err = done.stderr.trim_end();
    let (message, code) = err.rsplit_once('\n').unwrap_or(("", err));
    match code.trim().parse::<u16>() {
        Ok(status) if status > 0 => Ok(Reply { status, body: text }),
        _ => {
            let message = message.trim().trim_start_matches("curl: ");
            Err(match done.stdin_error {
                _ if !message.is_empty() => message.to_string(),
                Some(e) => format!("could not pass the request to curl: {e}"),
                None => "the request failed (no network?)".into(),
            })
        }
    }
}

// ------------------------------------------------------------------ encoding

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn session_id() -> Result<String, String> {
    let b = random_bytes(16)?;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding (RFC 4648 §5).
fn b64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// Base64url (also accepts standard base64 and padding).
fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v))).collect::<Vec<_>>().join("&")
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| char::from(b).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => match (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
                (Some(hi), Some(lo)) => {
                    out.push(hi << 4 | lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

/// SHA-256 (FIPS 180-4), for the PKCE challenge.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (slot, word) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *slot = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for (k, wi) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(*k).wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        *chunk = v.to_be_bytes();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha256(long)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url_encode(&sha256(verifier.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn base64url_round_trips() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0xff, 0xfe, 0x00]] {
            assert_eq!(b64url_decode(&b64url_encode(data)).as_deref(), Some(data));
        }
        assert_eq!(b64url_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64url_decode("Zm8="), Some(b"fo".to_vec()));
        assert_eq!(b64url_decode("not base64!"), None);
    }

    #[test]
    fn query_and_form_encoding() {
        assert_eq!(form_encode(&[("a b", "x/y&z=é")]), "a%20b=x%2Fy%26z%3D%C3%A9");
        let q = parse_query("code=ab%2Fc&state=x+y&bad=%zz&trail=%4");
        assert_eq!(q[0], ("code".into(), "ab/c".into()));
        assert_eq!(q[1], ("state".into(), "x y".into()));
        assert_eq!(q[2].1, "%zz");
        assert_eq!(q[3].1, "%4");
    }

    #[test]
    fn curl_config_quotes_values() {
        let c = curl_config("https://x/y", &[("Authorization", "Bearer a\"b\\c")], Path::new("/tmp/d/body"));
        assert!(c.contains("url = \"https://x/y\"\n"));
        assert!(c.contains("data-binary = \"@/tmp/d/body\"\n"));
        assert!(c.contains("header = \"Authorization: Bearer a\\\"b\\\\c\"\n"));
    }

    fn jwt(claims: &Value) -> String {
        format!("e30.{}.sig", b64url_encode(claims.to_string().as_bytes()))
    }

    #[test]
    fn tokens_come_from_the_id_token_and_keep_unrotated_fields() {
        let id = jwt(&json!({ "email": "a@b.c", "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1" } }));
        let access = jwt(&json!({ "exp": 2_000_000_000u64 }));
        let t = tokens_from(&json!({ "access_token": access, "refresh_token": "r1", "id_token": id }), None).unwrap();
        assert_eq!((t.account_id.as_str(), t.email.as_deref(), t.expires_at), ("acc-1", Some("a@b.c"), 2_000_000_000));
        let r = tokens_from(&json!({ "access_token": "opaque" }), Some(&t)).unwrap();
        assert_eq!((r.refresh_token.as_str(), r.account_id.as_str()), ("r1", "acc-1"));
        assert!(r.expires_at > crate::persist::now());
        assert!(tokens_from(&json!({ "access_token": "x", "refresh_token": "y", "id_token": "z" }), None).is_err());
    }

    #[test]
    fn sse_events_become_deltas_completion_and_failures() {
        let mut sse = Sse::default();
        let mut out = Output::default();
        let mut deltas = Vec::new();
        let stream = "event: response.output_text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"fix\"}\r\n\r\n\
                      data: {\"type\":\"response.output_text.delta\",\n\
                      data: \"delta\":\": a\"}\n\n\
                      : keep-alive\n\n\
                      data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"reasoning\"},\
                      {\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"fix: a\"}]}]}}\n\ndata: [DONE]";
        for line in stream.split('\n') {
            if let Some(d) = sse.line(line).and_then(|data| out.event(&data)) {
                deltas.push(d);
            }
        }
        assert!(sse.finish().is_none(), "[DONE] is not an event");
        assert_eq!(deltas, ["fix", ": a"]);
        assert_eq!(out.completed.as_deref(), Some("fix: a"));
        out.event(r#"{"type":"response.failed","response":{"error":{"message":"quota"}}}"#);
        out.event(r#"{"type":"error","message":"second"}"#);
        assert_eq!(out.failure.as_deref(), Some("quota"));
    }

    #[test]
    fn api_key_file_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch_dir().unwrap();
        let path = dir.join("openai_key");
        assert_eq!(key_from_file(&path), Ok(None));
        std::fs::write(&path, "sk-test\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(key_from_file(&path).unwrap_err().contains("chmod 600"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(key_from_file(&path), Ok(Some("sk-test".into())));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A one-shot HTTP server on localhost: returns its URL and the request it got.
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1/responses", listener.local_addr().unwrap().port());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = s.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&req).to_string()
        });
        (url, handle)
    }

    fn test_tokens() -> Tokens {
        Tokens {
            access_token: "tok".into(),
            refresh_token: "r".into(),
            id_token: "i".into(),
            account_id: "acc-9".into(),
            email: None,
            expires_at: u64::MAX,
        }
    }

    #[test]
    fn streams_through_curl_with_headers_off_the_command_line() {
        let (url, server) = serve(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"feat: \"}\n\n\
             data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n",
        );
        let mut got = String::new();
        let t = test_tokens();
        let text =
            stream_to(&url, &Auth::ChatGpt(&t), "m", "sys", "diff", &AtomicBool::new(false), &mut |d| got.push_str(d))
                .unwrap();
        assert_eq!((text.as_str(), got.as_str()), ("feat: x", "feat: x"));
        let req = server.join().unwrap();
        assert!(req.contains("Authorization: Bearer tok"));
        assert!(req.contains("ChatGPT-Account-Id: acc-9"));
        assert!(req.contains(r#""instructions":"sys""#) && req.contains(r#""include":[]"#));
    }

    #[test]
    fn reports_401_and_api_errors() {
        let (url, server) = serve("HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
        let none = &mut |_: &str| {};
        let r = stream_to(&url, &Auth::ApiKey("sk"), "m", "s", "i", &AtomicBool::new(false), none);
        assert_eq!(r, Err(StreamError::Unauthorized));
        let req = server.join().unwrap();
        assert!(req.contains("Authorization: Bearer sk") && !req.contains("include"));

        let body = r#"{"error":{"message":"model not found"}}"#;
        let reply: &'static str = Box::leak(
            format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .into_boxed_str(),
        );
        let (url, server) = serve(reply);
        let r = stream_to(&url, &Auth::ApiKey("sk"), "m", "s", "i", &AtomicBool::new(false), none);
        assert_eq!(r, Err(StreamError::Other("OpenAI request failed (404): model not found".into())));
        server.join().unwrap();

        let r = stream_to("http://127.0.0.1:9/x", &Auth::ApiKey("sk"), "m", "s", "i", &AtomicBool::new(false), none);
        assert!(matches!(r, Err(StreamError::Other(_))));
    }

    #[test]
    fn callback_accepts_only_the_matching_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hit = |path: &'static str| {
            let client = std::thread::spawn(move || {
                let mut c = TcpStream::connect(addr).unwrap();
                write!(c, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut reply = String::new();
                let _ = c.read_to_string(&mut reply);
                reply
            });
            let (stream, _) = listener.accept().unwrap();
            let result = callback(stream, "s1");
            (result, client.join().unwrap())
        };
        let (r, reply) = hit("/favicon.ico");
        assert!(r.is_none() && reply.starts_with("HTTP/1.1 404"));
        let (r, _) = hit("/auth/callback?code=c&state=other");
        assert!(r.is_none());
        let (r, _) = hit("/auth/callback?error=access_denied&error_description=Nope");
        assert!(r.is_none(), "an error without this sign-in's state doesn't end it");
        let (r, _) = hit("/auth/callback?error=access_denied&error_description=Nope&state=s1");
        assert_eq!(r, Some(Err("sign-in failed: Nope".into())));
        let (r, reply) = hit("/auth/callback?code=c%2D1&state=s1");
        assert_eq!(r, Some(Ok("c-1".into())));
        assert!(reply.contains("Signed in to ChatGPT"));
    }
}
