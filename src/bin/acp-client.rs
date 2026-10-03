//! acp-client bridges stdio to an agent exposed by acp-server, so an ACP client
//! such as Zed can run the remote agent as if it were local:
//!
//! ```text
//! acp-client <ticket>
//! ```
//!
//! Without a ticket it prints its ID, for acp-server -allow. Its key is saved in
//! -key, so the ID stays the same.

use std::{error::Error, fmt::Display, process::exit, time::Duration};

use tokio::{io, time::timeout};

const USAGE: &str = "usage: acp-client [-key file] [ticket]";

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let mut key = iroh_acp::key_path("client.key");
    // Read flags as Go's flag package does: -name value, -name=value or --name.
    while let Some(arg) = args.next_if(|arg| arg.starts_with('-')) {
        let (flag, value) = arg.split_once('=').map_or((&*arg, None), |(f, v)| (f, Some(v.into())));
        match (flag.trim_start_matches('-'), value.or_else(|| args.next())) {
            ("key", Some(file)) => key = file.into(),
            ("h" | "help", _) => fail(USAGE, 0),
            _ => fail(USAGE, 2),
        }
    }
    let key = iroh_acp::load_key(key).unwrap_or_else(|e| fail(e, 1));
    let Some(ticket) = args.next() else { return println!("{}", key.public()) };
    let ep = iroh_acp::builder().secret_key(key).bind().await.unwrap_or_else(|e| fail(e, 1));
    let (mut send, mut recv) = iroh_acp::dial(&ep, &ticket).await.unwrap_or_else(|e| fail(e, 1));
    tokio::spawn(async move {
        let _ = io::copy(&mut io::stdin(), &mut send).await;
        let _ = send.finish();
    });
    let copied = io::copy(&mut recv, &mut io::stdout()).await;
    // close sends the close at once, then waits for the server to answer it. A
    // go-iroh server never does, which QUIC allows, so wait at most a second.
    let _ = timeout(Duration::from_secs(1), ep.close()).await;
    if let Err(e) = copied {
        // The source holds the reason the server gave for closing, if any.
        fail(Error::source(&e).unwrap_or(&e), 1)
    }
    // Exit here, as the runtime would otherwise wait for the blocked stdin read.
    exit(0)
}

fn fail(e: impl Display, code: i32) -> ! {
    eprintln!("{e}");
    exit(code)
}
