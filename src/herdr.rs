//! Client for the herdr socket API (newline-delimited JSON over a unix
//! socket). mira runs `herdr` in its terminal and uses this client to learn
//! which agents are working where, so it can snapshot the repository around
//! each agent run, and to type review comments into the right pane.
//!
//! Protocol facts are in `.agents/skills/herdr-client/SKILL.md`. Every call is
//! one short blocking connection: run it through `app::blocking`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);
/// Upper bound for one response line (a snapshot of a big session is a few KB).
const MAX_LINE: u64 = 8 * 1024 * 1024;

/// Pane ids look like `w1:p2`. Opaque; never derived, only read from herdr.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaneId(pub String);

impl fmt::Display for PaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    /// Finished and not yet seen; for mira the same as `Idle`.
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    /// Running a turn: producing work, or waiting on an approval mid-turn.
    pub fn busy(self) -> bool {
        matches!(self, Self::Working | Self::Blocked)
    }

    /// Ready for the next prompt.
    pub fn ready(self) -> bool {
        matches!(self, Self::Idle | Self::Done)
    }
}

/// One entry of `agent.list`. Unknown fields are ignored, as the protocol asks.
#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub pane_id: PaneId,
    /// Agent kind: `codex`, `kiro`, `claude`, …
    #[serde(default)]
    pub agent: Option<String>,
    /// User-assigned agent name, if any.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_agent: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub foreground_cwd: Option<PathBuf>,
    /// Changes every time a working period completes.
    #[serde(default)]
    pub completion_seq: Option<u64>,
    /// herdr's counter of agent state changes, as of this pane's last one
    /// (unchanged = nothing happened in the pane since).
    #[serde(default)]
    pub state_change_seq: Option<u64>,
}

impl Agent {
    /// Directory the agent works in: its own process cwd when known.
    pub fn dir(&self) -> Option<&Path> {
        self.foreground_cwd.as_deref().or(self.cwd.as_deref())
    }

    /// Short human name: the user's name for it, else its kind.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.display_agent.clone())
            .or_else(|| self.agent.clone())
            .unwrap_or_else(|| "agent".into())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    /// The pane the user is in (one per server: in the focused workspace).
    #[serde(default)]
    pub focused: bool,
}

/// The focused pane, i.e. where the user is working in herdr.
pub fn focused(panes: &[Pane]) -> Option<&Pane> {
    panes.iter().find(|p| p.focused)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No server is listening (no socket, or a stale one).
    NotRunning,
    Io(String),
    /// The reply was not what the protocol promises.
    Protocol(String),
    /// herdr answered with an error object.
    Server {
        code: String,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => f.write_str("herdr is not running"),
            Self::Io(e) => write!(f, "herdr socket: {e}"),
            Self::Protocol(e) => write!(f, "herdr protocol: {e}"),
            Self::Server { code, message } => write!(f, "herdr: {message} ({code})"),
        }
    }
}

impl std::error::Error for Error {}

fn io_error(e: &std::io::Error) -> Error {
    use std::io::ErrorKind::{ConnectionRefused, NotFound};
    match e.kind() {
        NotFound | ConnectionRefused => Error::NotRunning,
        _ => Error::Io(e.to_string()),
    }
}

/// herdr's config directory: `$XDG_CONFIG_HOME/herdr` or `~/.config/herdr`.
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("herdr"))
}

/// herdr's rule for session names: 1–64 bytes of `[A-Za-z0-9._-]`, not `.`/`..`.
pub fn valid_session(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name != "."
        && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Socket of `session` (`None` or `default` = the default session).
pub fn socket_path(session: Option<&str>) -> Option<PathBuf> {
    let dir = config_dir()?;
    Some(match session.filter(|s| *s != "default") {
        Some(name) => dir.join("sessions").join(name).join("herdr.sock"),
        None => dir.join("herdr.sock"),
    })
}

/// Whether the `herdr` binary is on `$PATH`.
pub fn installed() -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("herdr").is_file()))
}

/// herdr's version (`herdr --version`, e.g. `0.9.5`), if it runs.
pub fn version() -> Option<String> {
    let out = std::process::Command::new("herdr")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.trim().trim_start_matches("herdr").trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Program and argv that attach a herdr client to `session`. Goes through
/// `env -u` so a mira started from inside a herdr pane does not hand its pane
/// identity to the new client.
pub fn launch_command(session: Option<&str>) -> (String, Vec<String>) {
    let mut args: Vec<String> = Vec::new();
    for var in
        ["HERDR_ENV", "HERDR_SOCKET_PATH", "HERDR_SESSION", "HERDR_WORKSPACE_ID", "HERDR_TAB_ID", "HERDR_PANE_ID"]
    {
        args.push("-u".into());
        args.push(var.into());
    }
    args.push("herdr".into());
    if let Some(name) = session.filter(|s| *s != "default" && valid_session(s)) {
        args.push("--session".into());
        args.push(name.into());
    }
    ("env".into(), args)
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    pub fn for_session(session: Option<&str>) -> Option<Self> {
        socket_path(session).map(Self::new)
    }

    /// One request/response round trip on a fresh connection.
    pub fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| io_error(&e))?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(|e| io_error(&e))?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(|e| io_error(&e))?;
        let id = format!("mira-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let mut line = serde_json::to_vec(&json!({ "id": id, "method": method, "params": params }))
            .map_err(|e| Error::Protocol(e.to_string()))?;
        line.push(b'\n');
        stream.write_all(&line).map_err(|e| io_error(&e))?;

        let mut reply = String::new();
        BufReader::new(stream.take(MAX_LINE)).read_line(&mut reply).map_err(|e| io_error(&e))?;
        if reply.trim().is_empty() {
            return Err(Error::Protocol("connection closed without a reply".into()));
        }
        let mut v: Value = serde_json::from_str(&reply).map_err(|e| Error::Protocol(e.to_string()))?;
        if let Some(err) = v.get("error") {
            return Err(Error::Server {
                code: err["code"].as_str().unwrap_or("unknown").to_string(),
                message: err["message"].as_str().unwrap_or("request failed").to_string(),
            });
        }
        if v["id"].as_str() != Some(id.as_str()) {
            return Err(Error::Protocol(format!("reply for another request: {}", v["id"])));
        }
        match v.get_mut("result") {
            Some(result) => Ok(result.take()),
            None => Err(Error::Protocol("reply without result".into())),
        }
    }

    /// `result[field]` decoded as `T`.
    fn list<T: for<'de> Deserialize<'de>>(&self, method: &str, field: &str) -> Result<Vec<T>, Error> {
        let mut result = self.call(method, json!({}))?;
        serde_json::from_value(result[field].take()).map_err(|e| Error::Protocol(format!("{method}: {e}")))
    }

    pub fn agents(&self) -> Result<Vec<Agent>, Error> {
        self.list("agent.list", "agents")
    }

    pub fn panes(&self) -> Result<Vec<Pane>, Error> {
        self.list("pane.list", "panes")
    }

    /// Focus the workspace for `root`, creating it when none has a pane there.
    /// Returns its id.
    pub fn ensure_workspace(&self, root: &Path, label: &str) -> Result<String, Error> {
        // A pane in the folder itself, else one below it (the user `cd`ed).
        let panes = self.panes()?;
        let existing = panes
            .iter()
            .find(|p| p.cwd.as_deref() == Some(root))
            .or_else(|| panes.iter().find(|p| p.cwd.as_deref().is_some_and(|c| c.starts_with(root))))
            .map(|p| p.workspace_id.clone());
        match existing {
            Some(ws) => {
                self.call("workspace.focus", json!({ "workspace_id": ws }))?;
                Ok(ws)
            }
            None => {
                let created = self.call(
                    "workspace.create",
                    json!({ "cwd": root.to_string_lossy(), "label": label, "focus": true }),
                )?;
                created["workspace"]["workspace_id"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| Error::Protocol("workspace.create without workspace id".into()))
            }
        }
    }

    /// Type `text` into a pane without pressing Enter.
    pub fn send_text(&self, pane: &PaneId, text: &str) -> Result<(), Error> {
        self.call("pane.send_text", json!({ "pane_id": pane.0, "text": text })).map(|_| ())
    }

    pub fn focus_pane(&self, pane: &PaneId) -> Result<(), Error> {
        self.call("pane.focus", json!({ "pane_id": pane.0 })).map(|_| ())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    /// A fake herdr: answers each request line with `respond(method, params)`
    /// and records the requests.
    pub(crate) struct FakeServer {
        pub socket: PathBuf,
        pub requests: Arc<Mutex<Vec<(String, Value)>>>,
        dir: PathBuf,
    }

    impl Drop for FakeServer {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    pub(crate) fn fake_server(respond: impl Fn(&str, &Value) -> Value + Send + 'static) -> FakeServer {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mira-herdr-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let socket = dir.join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut line = String::new();
                let mut reader = BufReader::new(&stream);
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let req: Value = serde_json::from_str(&line).expect("request is JSON");
                let method = req["method"].as_str().unwrap_or_default().to_string();
                log.lock().expect("lock").push((method.clone(), req["params"].clone()));
                let mut reply = respond(&method, &req["params"]);
                reply["id"] = req["id"].clone();
                let mut out = &stream;
                let _ = out.write_all(format!("{reply}\n").as_bytes());
            }
        });
        FakeServer { socket, requests, dir }
    }

    #[test]
    fn decodes_agents_ignoring_unknown_fields_and_statuses() {
        let server = fake_server(|_, _| {
            json!({"result": {"type": "agent_list", "agents": [
                {"pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": "t", "focused": true,
                 "revision": 3, "agent": "codex", "agent_status": "working", "cwd": "/repo",
                 "foreground_cwd": "/repo/sub", "completion_seq": 4, "brand_new_field": {"x": 1}},
                {"pane_id": "w1:p2", "workspace_id": "w1", "agent_status": "thinking-hard"}
            ]}})
        });
        let agents = Client::new(server.socket.clone()).agents().expect("agents");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].pane_id, PaneId("w1:p1".into()));
        assert_eq!(agents[0].agent_status, AgentStatus::Working);
        assert_eq!(agents[0].dir(), Some(Path::new("/repo/sub")));
        assert_eq!(agents[0].completion_seq, Some(4));
        assert_eq!(agents[0].label(), "codex");
        assert_eq!(agents[1].agent_status, AgentStatus::Unknown);
        assert_eq!(agents[1].label(), "agent");
    }

    #[test]
    fn server_errors_and_missing_server_are_typed() {
        let server =
            fake_server(|_, _| json!({"error": {"code": "pane_not_found", "message": "pane w9:p9 not found"}}));
        let err = Client::new(server.socket.clone()).focus_pane(&PaneId("w9:p9".into())).unwrap_err();
        assert_eq!(err, Error::Server { code: "pane_not_found".into(), message: "pane w9:p9 not found".into() });

        let gone = Client::new(std::env::temp_dir().join("mira-no-such-herdr.sock"));
        assert_eq!(gone.agents().unwrap_err(), Error::NotRunning);
    }

    #[test]
    fn ensure_workspace_focuses_existing_or_creates() {
        let server = fake_server(|method, _| match method {
            "pane.list" => json!({"result": {"type": "pane_list", "panes": [
                {"pane_id": "w1:p1", "workspace_id": "w1", "cwd": "/other"},
                {"pane_id": "w2:p1", "workspace_id": "w2", "cwd": "/repo"}
            ]}}),
            "workspace.create" => json!({"result": {"type": "workspace_created", "workspace": {"workspace_id": "w3"}}}),
            _ => json!({"result": {"type": "ok"}}),
        });
        let client = Client::new(server.socket.clone());
        assert_eq!(client.ensure_workspace(Path::new("/repo"), "repo").expect("existing"), "w2");
        assert_eq!(client.ensure_workspace(Path::new("/new"), "new").expect("created"), "w3");
        let reqs = server.requests.lock().expect("lock").clone();
        let methods: Vec<&str> = reqs.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(methods, ["pane.list", "workspace.focus", "pane.list", "workspace.create"]);
        assert_eq!(reqs[1].1, json!({"workspace_id": "w2"}));
        assert_eq!(reqs[3].1, json!({"cwd": "/new", "label": "new", "focus": true}));
    }

    #[test]
    fn focused_pane_is_where_the_user_is() {
        let server = fake_server(|_, _| {
            json!({"result": {"type": "pane_list", "panes": [
                {"pane_id": "w1:p1", "workspace_id": "w1", "cwd": "/a", "focused": false},
                {"pane_id": "w2:p1", "workspace_id": "w2", "cwd": "/b", "focused": true},
                {"pane_id": "w3:p1", "workspace_id": "w3"}
            ]}})
        });
        let panes = Client::new(server.socket.clone()).panes().expect("panes");
        let f = focused(&panes).expect("focused");
        assert_eq!((f.workspace_id.as_str(), f.cwd.as_deref()), ("w2", Some(Path::new("/b"))));
        assert!(focused(&panes[..1]).is_none());
    }

    #[test]
    fn launch_strips_pane_identity_and_validates_session() {
        let (prog, args) = launch_command(Some("work"));
        assert_eq!(prog, "env");
        assert!(args.windows(2).any(|w| w == ["-u", "HERDR_SOCKET_PATH"]));
        assert_eq!(&args[args.len() - 3..], ["herdr", "--session", "work"]);
        let (_, args) = launch_command(Some("../evil"));
        assert_eq!(args.last().map(String::as_str), Some("herdr"));
        assert!(valid_session("mira-1.0_x") && !valid_session("..") && !valid_session("a/b"));
    }
}
