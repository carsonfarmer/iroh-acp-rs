//! acp-server exposes a stdio ACP agent to the iroh peers it allows. It prints an
//! endpoint ticket, then runs one agent process per connection:
//!
//! ```text
//! acp-server -allow <client-id> npx -y @agentclientprotocol/claude-agent-acp
//! ```
//!
//! Its key is saved in -key, so tickets keep working across restarts.

use std::process::{ExitStatus, Stdio, exit};
use std::{fmt::Display, time::Duration};

use iroh::endpoint::{RecvStream, SendStream};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::{io, process::Command, time::timeout};
use tokio_util::task::AbortOnDropHandle;

const USAGE: &str =
    "usage: acp-server [-key file] -allow <client-id> [-allow ...] <agent-command> [args...]";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let (mut key, mut allow) = (iroh_acp::key_path("server.key"), Vec::new());
    // Read flags as Go's flag package does: -name value, -name=value or --name.
    while let Some(arg) = args.next_if(|arg| arg.starts_with('-')) {
        let (flag, value) = arg.split_once('=').map_or((&*arg, None), |(f, v)| (f, Some(v.into())));
        match (flag.trim_start_matches('-'), value.or_else(|| args.next())) {
            ("key", Some(file)) => key = file.into(),
            ("allow", Some(id)) => allow.push(id.parse().unwrap_or_else(|e| fail(e, 2))),
            ("h" | "help", _) => fail(USAGE, 0),
            _ => fail(USAGE, 2),
        }
    }
    let cmd: Vec<String> = args.collect();
    if cmd.is_empty() || allow.is_empty() {
        fail(USAGE, 1)
    }
    let key = iroh_acp::load_key(key).unwrap_or_else(|e| fail(e, 1));
    let ep = iroh_acp::builder().secret_key(key).bind().await.unwrap_or_else(|e| fail(e, 1));
    // Without a home relay the ticket has direct addresses only.
    let _ = timeout(Duration::from_secs(10), ep.online()).await;
    println!("{}", EndpointTicket::new(ep.addr()));
    iroh_acp::serve(&ep, iroh_acp::allow_ids(allow), move |tx, rx| run(cmd.clone(), tx, rx)).await;
}

// run runs the agent with the stream as its stdin and stdout, and logs its exit.
async fn run(cmd: Vec<String>, tx: SendStream, rx: RecvStream) {
    let status = agent(&cmd, tx, rx).await.map_or_else(|e| e.to_string(), |s| s.to_string());
    eprintln!("agent exited: {status}");
}

async fn agent(cmd: &[String], mut tx: SendStream, mut rx: RecvStream) -> io::Result<ExitStatus> {
    let mut agent = Command::new(&cmd[0]);
    let mut child = agent.args(&cmd[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
    let (mut stdin, mut stdout) = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
    tokio::spawn(async move { io::copy(&mut rx, &mut stdin).await });
    let output = tokio::spawn(async move { io::copy(&mut stdout, &mut tx).await });
    let status = child.wait().await;
    // Like Go's WaitDelay, give the output a second to drain once the agent exits.
    let _ = timeout(Duration::from_secs(1), AbortOnDropHandle::new(output)).await;
    status
}

fn fail(e: impl Display, code: i32) -> ! {
    eprintln!("{e}");
    exit(code)
}
