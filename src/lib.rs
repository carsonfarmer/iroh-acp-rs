//! The Agent Client Protocol over iroh. Each ACP connection is one bidirectional
//! QUIC stream between two iroh endpoints, framed exactly like ACP over stdio
//! (newline-delimited JSON-RPC), so any stdio agent or client works unchanged at
//! either end. Peers find each other with endpoint tickets and connect directly
//! or through the n0 relays.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::{env, path::Path, path::PathBuf, time::Duration};

use agent_client_protocol::{ByteStreams, Client, ConnectTo};
use iroh::endpoint::{Builder, QuicTransportConfig, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// ALPN identifies ACP streams on an iroh connection.
pub const ALPN: &[u8] = b"acp/1";

/// The error of [`dial`] and [`connect_agent`].
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Returns an endpoint builder that uses the n0 relays. Its connections close
/// after 10s without word from the peer (iroh's default is 30s), so an agent
/// whose client was killed, as editors stop agents, does not outlive it by long.
pub fn builder() -> Builder {
    let idle = Duration::from_secs(10).try_into().expect("10s fits");
    let config = QuicTransportConfig::builder().max_idle_timeout(Some(idle));
    Endpoint::builder(presets::N0).transport_config(config.build())
}

/// Returns the path of the key file `name` in the user's config directory, the
/// same directory Go's `os.UserConfigDir` returns.
pub fn key_path(name: &str) -> PathBuf {
    let home = env::home_dir().unwrap_or_default();
    let dir = match env::var_os("XDG_CONFIG_HOME") {
        _ if cfg!(target_os = "macos") => home.join("Library/Application Support"),
        Some(dir) if !dir.is_empty() => dir.into(),
        _ => home.join(".config"),
    };
    dir.join("iroh-acp").join(name)
}

/// Returns the secret key saved at `path`, creating it first if needed, so an
/// endpoint that uses it keeps its ID, and a server its ticket, across restarts.
pub fn load_key(path: impl AsRef<Path>) -> io::Result<SecretKey> {
    let path = path.as_ref();
    if !path.exists() {
        DirBuilder::new().recursive(true).mode(0o700).create(path.parent().unwrap_or(path))?;
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        file.write_all(&SecretKey::generate().to_bytes())?;
    }
    SecretKey::try_from(&std::fs::read(path)?[..]).map_err(io::Error::other)
}

/// Opens an ACP stream to the endpoint in `ticket`. Finishing the send stream
/// closes only that side, like closing an agent's stdin.
pub async fn dial(ep: &Endpoint, ticket: &str) -> Result<(SendStream, RecvStream), Error> {
    let ticket: EndpointTicket = ticket.parse()?;
    Ok(ep.connect(ticket, ALPN).await?.open_bi().await?)
}

/// Returns an allow func for [`serve`] that accepts only `ids`.
pub fn allow_ids(ids: Vec<EndpointId>) -> impl Fn(EndpointId) -> bool + Clone + Send + 'static {
    move |id| ids.contains(&id)
}

/// Serves ACP on `ep` until it closes, running `handle` in a new task for each
/// stream. It disconnects peers whose ID `allow` rejects, with the reason
/// "not allowed", and logs them.
pub async fn serve<A, H, F>(ep: &Endpoint, allow: A, handle: H)
where
    A: Fn(EndpointId) -> bool + Clone + Send + 'static,
    H: Fn(SendStream, RecvStream) -> F + Clone + Send + 'static,
    F: Future<Output: Send> + Send + 'static,
{
    ep.set_alpns(vec![ALPN.to_vec()]);
    while let Some(incoming) = ep.accept().await {
        let (allow, handle) = (allow.clone(), handle.clone());
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            if !allow(conn.remote_id()) {
                conn.close(1u32.into(), b"not allowed");
                // Return, as a closed connection still yields the streams it has.
                return eprintln!("rejected {}", conn.remote_id());
            }
            while let Ok((send, recv)) = conn.accept_bi().await {
                tokio::spawn(handle(send, recv));
            }
        });
    }
}

/// Serves a new agent from `new_agent` on each ACP stream, as [`serve`] does.
pub async fn serve_agent<A: ConnectTo<Client>>(
    ep: &Endpoint,
    allow: impl Fn(EndpointId) -> bool + Clone + Send + 'static,
    new_agent: impl Fn() -> A + Clone + Send + 'static,
) {
    serve(ep, allow, move |send, recv| {
        new_agent().connect_to(ByteStreams::new(send.compat_write(), recv.compat()))
    })
    .await;
}

/// Connects to the agent served at `ticket`. Pass the result to
/// `Client.builder().connect_with`.
pub async fn connect_agent(ep: &Endpoint, ticket: &str) -> Result<impl ConnectTo<Client>, Error> {
    let (send, recv) = dial(ep, ticket).await?;
    Ok(ByteStreams::new(send.compat_write(), recv.compat()))
}
