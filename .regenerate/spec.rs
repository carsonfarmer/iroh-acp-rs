//! The held-out tests for a rebuild of iroh-acp-rs, checked against SPEC.md.
//! They drive only the exported API, the built binaries and plain iroh, and
//! compare the exported declarations with the spec, so they
//! can judge the code in this repository or a rebuild of it.
//!
//! Tests that dial through the n0 relays are ignored by default. Run them with
//! `cargo test -- --include-ignored`. The binaries test needs the spec-echo
//! example, which `cargo test` builds unless it is given `--test`.

#[path = "echo.rs"]
mod echo;

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use agent_client_protocol::schema::{ProtocolVersion, v1::InitializeRequest};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectTo, ConnectionTo};
use iroh::endpoint::{ApplicationClose, ConnectError, ConnectingError, ConnectionError};
use iroh::endpoint::{RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointId, RelayMode, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

const SERVER: &str = env!("CARGO_BIN_EXE_acp-server");
const CLIENT: &str = env!("CARGO_BIN_EXE_acp-client");
const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// Returns section `n` of SPEC.md, from its heading to the next one.
fn section(n: u32) -> String {
    let spec = std::fs::read_to_string(Path::new(ROOT).join(".regenerate/SPEC.md")).unwrap();
    let start = spec.find(&format!("\n## {n}. ")).unwrap_or_else(|| panic!("no section {n}")) + 1;
    let end = spec[start..].find("\n## ").map_or(spec.len(), |i| start + i);
    spec[start..end].to_owned()
}

/// Returns the first block in `text` fenced as `lang`.
fn fenced(text: &str, lang: &str) -> String {
    let open = format!("```{lang}\n");
    let start = text.find(&open).unwrap_or_else(|| panic!("no {lang} block")) + open.len();
    let end = text[start..].find("```").unwrap();
    text[start..start + end].to_owned()
}

/// Returns, sorted and normalized, each declaration in `src` that starts with
/// `pub ` at the start of a line, up to the `{` or `;` that ends its signature.
fn declarations(src: &str) -> Vec<String> {
    let (mut decls, mut decl) = (Vec::new(), None::<String>);
    for line in src.lines() {
        let line = line.split("//").next().unwrap().trim_end();
        if decl.is_none() && line.starts_with("pub ") {
            decl = Some(String::new());
        }
        let Some(text) = decl.as_mut() else { continue };
        text.push_str(line);
        text.push('\n');
        if line.ends_with('{') || line.ends_with(';') {
            decls.push(normalize(&decl.take().unwrap()));
        }
    }
    decls.sort();
    decls
}

/// Splits a declaration into words, lifetimes, string literals and punctuation,
/// with `::` and `->` as single tokens.
fn tokens(decl: &str) -> Vec<String> {
    let (mut tokens, mut chars) = (Vec::new(), decl.chars().peekable());
    while let Some(c) = chars.next() {
        let mut token = c.to_string();
        match c {
            _ if c.is_whitespace() => continue,
            '"' => {
                while let Some(c) = chars.next() {
                    token.push(c);
                    match c {
                        '\\' => token.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '\'' | '_' | 'a'..='z' | 'A'..='Z' | '0'..='9' => {
                while let Some(c) = chars.next_if(|c| c.is_alphanumeric() || *c == '_') {
                    token.push(c);
                }
            }
            ':' => token.extend(chars.next_if(|c| *c == ':')),
            '-' => token.extend(chars.next_if(|c| *c == '>')),
            _ => {}
        }
        tokens.push(token);
    }
    tokens
}

fn is_word(token: &str) -> bool {
    token.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '\'')
}

fn is_lowercase_word(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
}

/// Drops path prefixes such as `std::io::`, parameter names, trailing commas
/// and the final `{` or `;`, and keeps whitespace only between two words.
fn normalize(decl: &str) -> String {
    let mut tokens = tokens(decl);
    let mut i = 0;
    while i + 1 < tokens.len() {
        if is_lowercase_word(&tokens[i]) && tokens[i + 1] == "::" {
            tokens.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
    let mut kept: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if matches!(kept.last().map(String::as_str), Some("(" | ",")) {
            let name = i + usize::from(tokens[i] == "mut");
            if is_lowercase_word(&tokens[name]) && tokens.get(name + 1).is_some_and(|t| t == ":") {
                i = name + 2;
                continue;
            }
        }
        kept.push(tokens[i].clone());
        i += 1;
    }
    let ends = |next: Option<&String>| {
        next.is_none_or(|next| matches!(next.as_str(), ")" | ">" | "]" | "{" | ";"))
    };
    let mut tokens: Vec<&String> = kept
        .iter()
        .enumerate()
        .filter(|(i, t)| *t != "," || !ends(kept.get(i + 1)))
        .map(|(_, t)| t)
        .collect();
    if tokens.last().is_some_and(|t| *t == "{" || *t == ";") {
        tokens.pop();
    }
    let mut out = String::new();
    for (i, token) in tokens.iter().enumerate() {
        if i > 0 && is_word(tokens[i - 1]) && is_word(token) {
            out.push(' ');
        }
        out.push_str(token);
    }
    out
}

/// The crate exports exactly the declarations in SPEC.md section 5, and the
/// constant and `allow_ids` behave as the spec says.
#[test]
fn api() {
    let want = declarations(&fenced(&section(5), "rust"));
    let lib = std::fs::read_to_string(Path::new(ROOT).join("src/lib.rs")).unwrap();
    assert!(!want.is_empty(), "no declarations in SPEC.md section 5");
    assert_eq!(declarations(&lib), want, "src/lib.rs must export what SPEC.md section 5 declares");

    assert_eq!(iroh_acp::ALPN, b"acp/1");
    let [a, b, c] = [1, 2, 3].map(|n| SecretKey::from_bytes(&[n; 32]).public());
    let allow = iroh_acp::allow_ids(vec![a, b]);
    assert!(allow(a) && allow(b) && !allow(c), "allow_ids does not match its list");
    assert!(!iroh_acp::allow_ids(vec![])(a), "allow_ids with no IDs accepts a peer");
}

/// Runs the binaries with -h, with bad flags and without the arguments they
/// need, and checks the exit statuses in SPEC.md sections 6, 7 and 9. Every run
/// must write to stderr, and none may write to stdout or make a key file.
#[tokio::test]
async fn usage() {
    let path = temp_dir("usage").join("usage.key");
    let path = path.to_str().unwrap();
    let id = SecretKey::from_bytes(&[1; 32]).public().to_string();
    let allow = format!("-allow={id}");
    let cases: Vec<(&str, Vec<&str>, i32)> = vec![
        (SERVER, vec!["-h"], 0),
        (SERVER, vec!["-nope", "x", "-allow", &id, "sh"], 2),
        (SERVER, vec!["-allow"], 2),
        (SERVER, vec!["-allow=nope", "sh"], 2),
        (SERVER, vec!["sh"], 1),
        (SERVER, vec![&allow], 1),
        (SERVER, vec!["-allow", &id], 1),
        (SERVER, vec![], 1),
        (CLIENT, vec!["-h"], 0),
        (CLIENT, vec!["-nope"], 2),
        (CLIENT, vec!["-key"], 2),
    ];
    for (bin, args, status) in cases {
        // -key comes first, in each of its two forms, so no run can touch the
        // real key file.
        for key in [vec![format!("-key={path}")], vec!["-key".into(), path.into()]] {
            let args = [key, args.iter().map(|arg| arg.to_string()).collect()].concat();
            let run = Command::new(bin).args(&args).kill_on_drop(true).output();
            let out = timeout(Duration::from_secs(10), run).await.expect("still running").unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(out.status.code(), Some(status), "{bin} {args:?}: {stderr}");
            assert!(!stderr.is_empty(), "{bin} {args:?} wrote nothing to stderr");
            if status == 1 {
                assert!(stderr.contains("usage: acp-server"), "{bin} {args:?}: {stderr}");
            }
            assert!(out.stdout.is_empty(), "{bin} {args:?} wrote to stdout");
            assert!(!Path::new(path).exists(), "{bin} {args:?} made a key file");
        }
    }
}

/// Runs acp-client with `args` and `$HOME` set to `home`, and returns the ID it
/// prints.
fn client_id(home: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new(CLIENT)
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let id = stdout.strip_suffix('\n').unwrap_or_default();
    assert!(
        out.status.success()
            && id.len() == 64
            && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "acp-client {args:?}: {}, stdout {stdout:?}",
        out.status
    );
    id.to_owned()
}

/// Returns where `key_path` puts the key file `name` when `$HOME` is `home` and
/// `$XDG_CONFIG_HOME` is not set.
fn default_key(home: &Path, name: &str) -> PathBuf {
    let config = if cfg!(target_os = "macos") { "Library/Application Support" } else { ".config" };
    home.join(config).join("iroh-acp").join(name)
}

/// acp-client prints a stable ID for each key, and keeps its key in the user's
/// config directory unless `-key` says otherwise.
#[test]
fn client_ids() {
    let home = temp_dir("client-ids");
    let key = |name: &str| home.join(name).to_string_lossy().into_owned();
    let alice = client_id(&home, &["-key", &key("alice.key")]);
    assert_eq!(client_id(&home, &["-key", &key("alice.key")]), alice, "the ID changed");
    assert_ne!(client_id(&home, &["-key", &key("bob.key")]), alice, "two keys share an ID");

    let id = client_id(&home, &[]);
    let path = default_key(&home, "client.key");
    assert_eq!(mode(&path), 0o600, "{}", path.display());
    assert_eq!(mode(path.parent().unwrap()), 0o700, "{}", path.display());
    assert_eq!(client_id(&home, &["-key", path.to_str().unwrap()]), id);

    // Read the real path only. Nothing here writes to it.
    if cfg!(target_os = "macos") || std::env::var_os("XDG_CONFIG_HOME").is_none() {
        let real = default_key(&std::env::home_dir().unwrap(), "client.key");
        assert_eq!(iroh_acp::key_path("client.key"), real);
    }
}

/// Counts the lines in the Rust files under `path` that are neither blank nor
/// comments that start with `//`.
fn lines_of_code(path: &Path) -> usize {
    if path.is_dir() {
        return std::fs::read_dir(path).unwrap().map(|e| lines_of_code(&e.unwrap().path())).sum();
    }
    if path.extension().is_none_or(|ext| ext != "rs") {
        return 0;
    }
    let src = std::fs::read_to_string(path).unwrap();
    src.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//")).count()
}

/// The library stays under 100 lines of code and each binary under 50, as
/// SPEC.md section 2 says.
#[test]
fn size() {
    let (src, bin) = (Path::new(ROOT).join("src"), Path::new(ROOT).join("src/bin"));
    let mut counts = vec![("library".to_owned(), lines_of_code(&src) - lines_of_code(&bin), 100)];
    for entry in std::fs::read_dir(&bin).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        counts.push((name, lines_of_code(&path), 50));
    }
    let names: BTreeSet<&str> = counts[1..].iter().map(|(name, ..)| name.as_str()).collect();
    assert_eq!(names, BTreeSet::from(["acp-client", "acp-server"]), "the binaries in src/bin");
    for (name, count, budget) in counts {
        eprintln!("{name}: {count} lines of code, budget under {budget}");
        assert!(count < budget, "{name} has {count} lines of code, budget under {budget}");
    }
}

/// Bytes that are not ACP and do not end in a newline.
const BYTES: &[u8] = b"{\"jsonrpc\":\"2.0\"}\r\n\n\x00\xff no newline at the end";

/// Binds an endpoint from `iroh_acp::builder` on loopback without relays, or on
/// the relays alone.
async fn bind(direct: bool) -> Endpoint {
    let builder = iroh_acp::builder().clear_ip_transports();
    let builder = match direct {
        true => builder.relay_mode(RelayMode::Disabled).bind_addr("127.0.0.1:0").unwrap(),
        false => builder,
    };
    builder.bind().await.unwrap()
}

/// Binds an endpoint with plain iroh on loopback, to stand in for another
/// implementation. A server passes the ALPNs it accepts.
async fn plain(alpns: Vec<Vec<u8>>) -> Endpoint {
    let builder = Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled);
    let builder = builder.clear_ip_transports().bind_addr("127.0.0.1:0").unwrap();
    builder.alpns(alpns).bind().await.unwrap()
}

fn ticket(ep: &Endpoint) -> String {
    EndpointTicket::new(ep.addr()).to_string()
}

/// Serves on `ep` with a handle that copies each stream back to its sender.
fn serve_echo(ep: Endpoint, allow: impl Fn(EndpointId) -> bool + Clone + Send + 'static) {
    tokio::spawn(async move {
        let handle = |mut send: SendStream, mut recv: RecvStream| async move {
            let _ = tokio::io::copy(&mut recv, &mut send).await;
            let _ = send.finish();
        };
        iroh_acp::serve(&ep, allow, handle).await;
    });
}

/// Writes the test bytes and finishes the send stream, then checks that the
/// same bytes come back, followed by the end of the stream.
async fn echo(mut send: SendStream, mut recv: RecvStream) {
    send.write_all(BYTES).await.unwrap();
    send.finish().unwrap();
    let got = timeout(Duration::from_secs(30), recv.read_to_end(1 << 16)).await;
    assert_eq!(got.expect("echo timed out").unwrap(), BYTES);
}

// The wire tests check section 4 of SPEC.md. Plain iroh stands in for another
// implementation at one end, so the ALPN, the bytes and the close code must be
// the ones the spec names, not just ones this implementation agrees with itself
// on.

#[tokio::test(flavor = "multi_thread")]
async fn wire_dial_to_serve() {
    let (server, client) = (bind(true).await, bind(true).await);
    serve_echo(server.clone(), iroh_acp::allow_ids(vec![client.id()]));
    let (send, recv) = iroh_acp::dial(&client, &ticket(&server)).await.unwrap();
    echo(send, recv).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_plain_to_serve() {
    let (server, client) = (bind(true).await, plain(vec![]).await);
    serve_echo(server.clone(), iroh_acp::allow_ids(vec![client.id()]));
    let conn = client.connect(server.addr(), b"acp/1").await.unwrap();
    let (send, recv) = conn.open_bi().await.unwrap();
    echo(send, recv).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_dial_to_plain() {
    let (server, client) = (plain(vec![b"acp/1".to_vec()]).await, bind(true).await);
    let (accepted, conn) = oneshot::channel();
    let ep = server.clone();
    tokio::spawn(async move {
        let conn = ep.accept().await.unwrap().await.unwrap();
        // The test holds the connection, so it stays open until the echo is read.
        let _ = accepted.send(conn.clone());
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();
        let _ = tokio::io::copy(&mut recv, &mut send).await;
        let _ = send.finish();
    });
    let (send, recv) = iroh_acp::dial(&client, &ticket(&server)).await.unwrap();
    let conn = conn.await.unwrap();
    echo(send, recv).await;
    assert_eq!(conn.alpn(), b"acp/1");
    assert_eq!(conn.remote_id(), client.id());
}

/// A peer that allow rejects is closed with code 1 and "not allowed", and gets
/// no stream, even one that arrived before allow returned.
#[tokio::test(flavor = "multi_thread")]
async fn wire_plain_rejected() {
    let (server, stranger) = (bind(true).await, plain(vec![]).await);
    let (handled, mut served) = mpsc::unbounded_channel();
    let slow_reject = |_| {
        std::thread::sleep(Duration::from_millis(300)); // the stranger's stream arrives meanwhile
        false
    };
    let ep = server.clone();
    tokio::spawn(async move {
        iroh_acp::serve(&ep, slow_reject, move |_, _| {
            let handled = handled.clone();
            async move {
                let _ = handled.send(());
            }
        })
        .await;
    });
    let closed = match stranger.connect(server.addr(), b"acp/1").await {
        Ok(conn) => {
            let (mut send, _recv) = conn.open_bi().await.unwrap();
            let _ = send.write_all(b"{}\n").await;
            timeout(Duration::from_secs(30), conn.closed()).await.expect("still connected")
        }
        Err(
            ConnectError::Connection { source, .. }
            | ConnectError::Connecting {
                source: ConnectingError::ConnectionError { source, .. },
                ..
            },
        ) => source,
        Err(err) => panic!("connect: {err}"),
    };
    let reason = ApplicationClose { error_code: 1u32.into(), reason: "not allowed".into() };
    assert_eq!(closed, ConnectionError::ApplicationClosed(reason));
    let served = timeout(Duration::from_secs(1), served.recv()).await;
    assert!(served.is_err(), "the stranger's stream was served");
}

/// Runs one turn on the agent and returns the text it streamed back.
async fn turn(cx: &ConnectionTo<Agent>, text: &str) -> agent_client_protocol::Result<String> {
    cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await?;
    let mut session = cx.build_session("/").block_task().start_session().await?;
    session.send_prompt(text)?;
    session.read_to_string().await
}

/// Connects to the agent over `transport`, runs one turn, and disconnects.
async fn prompt(
    transport: impl ConnectTo<Client> + 'static,
    text: &str,
) -> agent_client_protocol::Result<String> {
    let session = Client.builder().connect_with(transport, async |cx| turn(&cx, text).await);
    timeout(Duration::from_secs(30), session).await.expect("turn timed out")
}

async fn serve_agent_connect_agent(direct: bool) {
    let server = bind(direct).await;
    if !direct {
        server.online().await;
    }
    let clients = [bind(direct).await, bind(direct).await];
    let allow = iroh_acp::allow_ids(clients.iter().map(Endpoint::id).collect());
    let ticket = ticket(&server);
    tokio::spawn(async move { iroh_acp::serve_agent(&server, allow, echo::agent).await });

    // A peer that is not allowed gets no agent, and the server carries on.
    let stranger = bind(direct).await;
    if let Ok(transport) = iroh_acp::connect_agent(&stranger, &ticket).await {
        assert!(prompt(transport, "let me in").await.is_err(), "the stranger was served");
    }
    // Two connections at once: each gets its own agent.
    let connect = async |i: usize, text: &str| {
        prompt(iroh_acp::connect_agent(&clients[i], &ticket).await.unwrap(), text).await
    };
    let (first, second) = tokio::join!(connect(0, "hello over iroh"), connect(1, "second"));
    assert_eq!(first.unwrap(), "hello over iroh");
    assert_eq!(second.unwrap(), "second");
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_agent_connect_agent_direct() {
    serve_agent_connect_agent(true).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the n0 relays"]
async fn serve_agent_connect_agent_relay_only() {
    serve_agent_connect_agent(false).await;
}

/// Returns a new empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("iroh-acp-spec-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// A key file is the 32 raw bytes of the secret key, made once with mode 0600
/// in a directory with mode 0700.
#[test]
fn load_key() {
    let path = temp_dir("load-key").join("iroh-acp").join("test.key");
    let created = iroh_acp::load_key(&path).unwrap();
    let loaded = iroh_acp::load_key(&path).unwrap();
    assert_eq!(loaded.public(), created.public());
    assert_eq!(std::fs::read(&path).unwrap(), created.to_bytes());
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);

    std::fs::write(&path, [7; 32]).unwrap();
    let loaded = iroh_acp::load_key(&path).unwrap();
    assert_eq!(loaded.public(), SecretKey::from_bytes(&[7; 32]).public());
    std::fs::write(&path, b"short").unwrap();
    assert!(iroh_acp::load_key(&path).is_err(), "loaded a key that is not 32 bytes");
}

/// A running acp-server, its ticket, and the lines it writes to stderr.
struct Server {
    process: Child,
    stdout: BufReader<ChildStdout>,
    ticket: String,
    lines: mpsc::UnboundedReceiver<String>,
}

impl Server {
    /// Starts acp-server with `args` and `$HOME` set to `home`, and reads its
    /// ticket.
    async fn start(home: &Path, args: &[String]) -> Server {
        let mut process = Command::new(SERVER)
            .args(args)
            .env("HOME", home)
            .env_remove("XDG_CONFIG_HOME")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let (tx, lines) = mpsc::unbounded_channel();
        let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = stderr.next_line().await {
                eprintln!("acp-server: {line}");
                let _ = tx.send(line);
            }
        });
        let mut stdout = BufReader::new(process.stdout.take().unwrap());
        let mut ticket = String::new();
        let read = timeout(Duration::from_secs(30), stdout.read_line(&mut ticket)).await;
        read.expect("no ticket within 30s").unwrap();
        let body = ticket.strip_prefix("endpoint").and_then(|t| t.strip_suffix('\n'));
        assert!(
            body.is_some_and(
                |b| !b.is_empty() && b.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
            ),
            "acp-server printed {ticket:?}, want a ticket"
        );
        Server { process, stdout, ticket: ticket.trim_end().to_owned(), lines }
    }

    /// Reads the server's log until a line contains `text`.
    async fn wait_for(&mut self, text: &str, within: Duration) {
        let found = timeout(within, async {
            while let Some(line) = self.lines.recv().await {
                if line.contains(text) {
                    return true;
                }
            }
            false
        });
        assert!(
            matches!(found.await, Ok(true)),
            "acp-server did not log {text:?} within {within:?}"
        );
    }

    /// Kills the server and checks that it wrote nothing to stdout after its
    /// ticket.
    async fn stop(mut self) {
        self.process.kill().await.unwrap();
        let mut rest = String::new();
        self.stdout.read_to_string(&mut rest).await.unwrap();
        assert!(rest.is_empty(), "acp-server printed {rest:?} after its ticket");
    }
}

/// Starts acp-client the way an editor starts an agent, and returns it and a
/// transport over its stdio.
fn connect(
    home: &Path,
    key: &str,
    ticket: &str,
    stderr: Stdio,
) -> (Child, impl ConnectTo<Client> + 'static) {
    let mut client = Command::new(CLIENT)
        .arg("-key")
        .arg(home.join(key))
        .arg(ticket)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (stdin, stdout) = (client.stdin.take().unwrap(), client.stdout.take().unwrap());
    (client, ByteStreams::new(stdin.compat_write(), stdout.compat()))
}

/// Returns the path of the spec-echo example, which `cargo test` builds.
fn echo_bin() -> String {
    let deps = std::env::current_exe().unwrap();
    let echo = deps.parent().unwrap().parent().unwrap().join("examples").join("spec-echo");
    assert!(echo.exists(), "{} is missing: run cargo test without --test", echo.display());
    echo.to_string_lossy().into_owned()
}

fn endpoint_id(ticket: &str) -> EndpointId {
    ticket.parse::<EndpointTicket>().unwrap().endpoint_addr().id
}

/// Runs the echo agent behind acp-server and drives acp-client the way an
/// editor would: as a stdio agent subprocess.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the n0 relays"]
async fn binaries() {
    let (home, echo) = (temp_dir("binaries"), echo_bin());
    let id = |name: &str| client_id(&home, &["-key", home.join(name).to_str().unwrap()]);
    let (alice, bob, stranger) = (id("alice.key"), id("bob.key"), id("stranger.key"));
    // The agent writes to stderr before it starts, which the server must pass
    // on. The server must also run the command with its arguments as given.
    let agent = ["sh", "-c", r#"echo hello from the agent >&2; exec "$0""#, &echo];
    let args: Vec<String> =
        ["-allow", &alice, "-allow", &bob].iter().chain(&agent).map(|s| s.to_string()).collect();
    // This server keeps its key in the default file, under the test's $HOME.
    let mut server = Server::start(&home, &args).await;
    let server_key = default_key(&home, "server.key");
    assert_eq!(mode(&server_key), 0o600, "{}", server_key.display());

    for key in ["alice.key", "bob.key"] {
        let want = format!("hello from {key}");
        let (mut client, transport) = connect(&home, key, &server.ticket, Stdio::inherit());
        assert_eq!(prompt(transport, &want).await.unwrap(), want);
        server.wait_for("hello from the agent", Duration::from_secs(1)).await;
        // Closing stdin must end the remote agent and then acp-client, within
        // half of acp-go's grace period.
        let start = Instant::now();
        let status = timeout(Duration::from_millis(2500), client.wait()).await;
        let status = status.expect("acp-client still running 2.5s after its stdin closed");
        assert!(status.unwrap().success(), "acp-client failed after {:?}", start.elapsed());
        server.wait_for("agent exited", Duration::from_secs(1)).await;
    }

    // A client that is not allowed gets no agent, and acp-client says why.
    let (mut client, transport) = connect(&home, "stranger.key", &server.ticket, Stdio::piped());
    assert!(prompt(transport, "let me in").await.is_err(), "the stranger was served");
    let mut stderr = String::new();
    client.stderr.take().unwrap().read_to_string(&mut stderr).await.unwrap();
    let status = client.wait().await.unwrap();
    assert!(
        !status.success() && stderr.contains("not allowed"),
        "acp-client exited {status}, stderr {stderr:?}"
    );
    server.wait_for(&format!("rejected {stranger}"), Duration::from_secs(10)).await;

    // Editors stop agents with SIGKILL, so acp-client cannot say goodbye.
    let (mut client, transport) = connect(&home, "alice.key", &server.ticket, Stdio::inherit());
    let session = Client.builder().connect_with(transport, async |cx| {
        let got = turn(&cx, "about to be killed").await;
        client.start_kill().unwrap();
        got
    });
    assert_eq!(session.await.unwrap(), "about to be killed");
    let start = Instant::now();
    server.wait_for("agent exited", Duration::from_secs(15)).await;
    eprintln!("the remote agent exited {:?} after its client was killed", start.elapsed());

    // A server restarted with the same key keeps its ID, so the old ticket works.
    let ticket = server.ticket.clone();
    server.stop().await;
    let args = [vec!["-key".to_owned(), server_key.to_string_lossy().into_owned()], args].concat();
    let mut server = Server::start(&home, &args).await;
    assert_eq!(endpoint_id(&server.ticket), endpoint_id(&ticket), "the server's ID changed");
    let (mut client, transport) = connect(&home, "alice.key", &ticket, Stdio::inherit());
    assert_eq!(prompt(transport, "same ticket").await.unwrap(), "same ticket");
    assert!(client.wait().await.unwrap().success());
    server.wait_for("agent exited", Duration::from_secs(1)).await;
    server.stop().await;
}
