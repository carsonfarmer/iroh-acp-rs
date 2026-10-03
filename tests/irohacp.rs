//! Tests of the library through its exported API, and of the built binaries.
//! Tests that dial through the n0 relays are ignored by default. Run them with
//! `cargo test -- --include-ignored`.

#[path = "../examples/echo.rs"]
mod echo;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use agent_client_protocol::schema::{ProtocolVersion, v1::InitializeRequest};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectTo, ConnectionTo};
use iroh::{Endpoint, RelayMode};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// Binds an endpoint on loopback without relays, or on the relays alone.
async fn bind(direct: bool) -> Endpoint {
    let builder = if direct {
        iroh_acp::builder()
            .relay_mode(RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .unwrap()
    } else {
        iroh_acp::builder().clear_ip_transports()
    };
    builder.bind().await.unwrap()
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
    let ticket = EndpointTicket::new(server.addr()).to_string();
    tokio::spawn(async move { iroh_acp::serve_agent(&server, allow, echo::agent).await });

    // A peer that is not allowed gets no agent, and the server carries on.
    let stranger = bind(direct).await;
    if let Ok(transport) = iroh_acp::connect_agent(&stranger, &ticket).await {
        assert!(prompt(transport, "let me in").await.is_err(), "stranger was served");
    }
    // Two connections at once: each gets its own agent.
    let (first, second) = tokio::join!(
        async {
            prompt(iroh_acp::connect_agent(&clients[0], &ticket).await.unwrap(), "hello over iroh")
                .await
        },
        async {
            prompt(
                iroh_acp::connect_agent(&clients[1], &ticket).await.unwrap(),
                "second connection",
            )
            .await
        },
    );
    assert_eq!(first.unwrap(), "hello over iroh");
    assert_eq!(second.unwrap(), "second connection");
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

/// A peer that allow rejects gets no stream, even one that arrived before allow
/// returned.
#[tokio::test(flavor = "multi_thread")]
async fn serve_rejects_queued_stream() {
    let server = bind(true).await;
    let ticket = EndpointTicket::new(server.addr()).to_string();
    let (handled, mut served) = mpsc::unbounded_channel();
    let slow_reject = |_| {
        std::thread::sleep(Duration::from_millis(300)); // the stranger's stream arrives meanwhile
        false
    };
    tokio::spawn(async move {
        iroh_acp::serve(&server, slow_reject, move |_, _| {
            let handled = handled.clone();
            async move { handled.send(()).unwrap() }
        })
        .await;
    });
    let stranger = bind(true).await;
    let (mut send, mut recv) = iroh_acp::dial(&stranger, &ticket).await.unwrap();
    send.write_all(b"{}\n").await.unwrap();
    let err = recv.read(&mut [0]).await.expect_err("read from a rejected stream");
    assert!(format!("{err:?}").contains("not allowed"), "read error {err:?}");
    assert!(
        timeout(Duration::from_secs(1), served.recv()).await.is_err(),
        "stranger's stream was served"
    );
}

/// Returns a new empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("iroh-acp-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn load_key() {
    let path = temp_dir("load-key").join("iroh-acp").join("test.key");
    let created = iroh_acp::load_key(&path).unwrap();
    let loaded = iroh_acp::load_key(&path).unwrap();
    assert_eq!(loaded.public(), created.public());
    assert_eq!(std::fs::read(&path).unwrap().len(), 32);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    std::fs::write(&path, b"short").unwrap();
    assert!(iroh_acp::load_key(&path).is_err(), "loaded a corrupt key");
}

#[test]
fn key_path() {
    let path = iroh_acp::key_path("client.key");
    assert!(path.ends_with("iroh-acp/client.key"), "{}", path.display());
    if cfg!(target_os = "macos") {
        assert!(
            path.to_string_lossy().contains("Library/Application Support"),
            "{}",
            path.display()
        );
    }
}

const SERVER: &str = env!("CARGO_BIN_EXE_acp-server");
const CLIENT: &str = env!("CARGO_BIN_EXE_acp-client");

/// Returns the path of the echo example, which `cargo test` builds.
fn echo_bin() -> PathBuf {
    let deps = std::env::current_exe().unwrap();
    let echo = deps.parent().unwrap().parent().unwrap().join("examples").join("echo");
    assert!(echo.exists(), "{} is missing: run cargo test without --test", echo.display());
    echo
}

fn key_flag(dir: &Path, name: &str) -> [String; 2] {
    ["-key".into(), dir.join(name).to_string_lossy().into()]
}

#[test]
fn client_prints_its_id() {
    let dir = temp_dir("client-id");
    let id = |name| {
        let out = std::process::Command::new(CLIENT).args(key_flag(&dir, name)).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    let first = id("client.key");
    assert_eq!(first.len(), 64, "{first}");
    assert!(first.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{first}");
    assert_eq!(id("client.key"), first, "the ID changed");
    assert_ne!(id("other.key"), first, "two keys share an ID");
    assert_eq!(mode(&dir.join("client.key")), 0o600);
}

/// -h and usage errors exit with the statuses Go's flag package uses, and
/// write nothing to stdout.
#[test]
fn usage_errors() {
    let dir = temp_dir("usage");
    let id = "0".repeat(64);
    let cases: [(&str, Vec<String>, i32, &str); 6] = [
        (SERVER, vec![], 1, "usage: acp-server"),
        (SERVER, vec![format!("-allow={id}")], 1, "usage: acp-server"),
        (SERVER, vec!["-allow".into(), id.clone(), "-h".into()], 0, "usage: acp-server"),
        (SERVER, vec!["--allow=nope".into(), "echo".into()], 2, ""),
        (CLIENT, vec!["-nope".into()], 2, "usage: acp-client [-key file] [ticket]"),
        (CLIENT, vec!["--help".into()], 0, "usage: acp-client"),
    ];
    for (bin, args, status, want) in cases {
        let args = [key_flag(&dir, "test.key").to_vec(), args].concat();
        let out = std::process::Command::new(bin).args(&args).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(status), "{bin} {args:?}: {stderr}");
        assert!(stderr.contains(want), "{bin} {args:?}: stderr {stderr:?}, want {want:?}");
        assert!(out.stdout.is_empty(), "{bin} {args:?} wrote to stdout");
    }
}

/// A running acp-server, its ticket, and the "agent exited" lines it logs.
struct Server {
    _process: Child,
    ticket: String,
    exited: mpsc::UnboundedReceiver<()>,
}

impl Server {
    async fn start(args: &[String]) -> Server {
        let mut process = Command::new(SERVER)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let (tx, exited) = mpsc::unbounded_channel();
        let mut lines = BufReader::new(process.stderr.take().unwrap()).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                eprintln!("acp-server: {line}");
                if line.contains("agent exited") {
                    let _ = tx.send(());
                }
            }
        });
        let mut ticket = String::new();
        BufReader::new(process.stdout.take().unwrap()).read_line(&mut ticket).await.unwrap();
        let ticket = ticket.trim().to_owned();
        let body = ticket.strip_prefix("endpoint").unwrap_or_default();
        assert!(
            !body.is_empty() && body.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')),
            "ticket {ticket:?}"
        );
        Server { _process: process, ticket, exited }
    }

    async fn agent_exits(&mut self, within: Duration) {
        let exited = timeout(within, self.exited.recv()).await;
        assert!(matches!(exited, Ok(Some(()))), "remote agent still running after {within:?}");
    }
}

/// Starts acp-client the way an editor starts an agent, and returns it and a
/// transport over its stdio.
fn connect(
    dir: &Path,
    key: &str,
    ticket: &str,
    stderr: Stdio,
) -> (Child, impl ConnectTo<Client> + 'static) {
    let mut client = Command::new(CLIENT)
        .args(key_flag(dir, key))
        .arg(ticket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let (stdin, stdout) = (client.stdin.take().unwrap(), client.stdout.take().unwrap());
    (client, ByteStreams::new(stdin.compat_write(), stdout.compat()))
}

/// Runs the echo example behind acp-server and drives acp-client as an editor
/// would: as a stdio agent subprocess.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the n0 relays"]
async fn binaries() {
    let (dir, echo) = (temp_dir("binaries"), echo_bin());
    let id =
        std::process::Command::new(CLIENT).args(key_flag(&dir, "client.key")).output().unwrap();
    let id = String::from_utf8(id.stdout).unwrap().trim().to_owned();
    let args = [
        key_flag(&dir, "server.key").to_vec(),
        vec!["-allow".into(), id, echo.to_string_lossy().into()],
    ]
    .concat();
    let mut server = Server::start(&args).await;

    for want in ["hello from acp-client", "the server keeps serving"] {
        let (mut client, transport) = connect(&dir, "client.key", &server.ticket, Stdio::inherit());
        assert_eq!(prompt(transport, want).await.unwrap(), want);
        // Closing stdin must end the remote agent and then acp-client.
        let start = Instant::now();
        let status =
            timeout(Duration::from_secs(5), client.wait()).await.expect("acp-client still running");
        assert!(status.unwrap().success(), "acp-client failed after {:?}", start.elapsed());
        server.agent_exits(Duration::from_secs(1)).await;
    }

    // A client that is not allowed gets no agent, and acp-client says why.
    let (mut stranger, transport) = connect(&dir, "stranger.key", &server.ticket, Stdio::piped());
    assert!(prompt(transport, "let me in").await.is_err(), "stranger was served");
    let mut stderr = String::new();
    stranger.stderr.take().unwrap().read_to_string(&mut stderr).await.unwrap();
    let status = stranger.wait().await.unwrap();
    assert!(
        !status.success() && stderr.contains("not allowed"),
        "acp-client exited {status}, stderr {stderr:?}"
    );

    // Zed stops agents with SIGKILL, so acp-client cannot say goodbye.
    let (mut client, transport) = connect(&dir, "client.key", &server.ticket, Stdio::inherit());
    let session = Client.builder().connect_with(transport, async |cx| {
        let got = turn(&cx, "about to be killed").await;
        client.start_kill().unwrap();
        got
    });
    assert_eq!(session.await.unwrap(), "about to be killed");
    let start = Instant::now();
    server.agent_exits(Duration::from_secs(15)).await;
    eprintln!("remote agent exited {:?} after its client was killed", start.elapsed());

    // A restarted server keeps its ticket.
    let ticket = server.ticket.clone();
    drop(server);
    let mut server = Server::start(&args).await;
    let (mut client, transport) = connect(&dir, "client.key", &ticket, Stdio::inherit());
    assert_eq!(prompt(transport, "same ticket").await.unwrap(), "same ticket");
    let _ = client.wait().await;
    server.agent_exits(Duration::from_secs(1)).await;
}
