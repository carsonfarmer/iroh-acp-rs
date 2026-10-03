//! An ACP agent on stdio that answers each prompt with its own text. The tests
//! run it behind acp-server and in process with `serve_agent`. This copy is part
//! of the held-out spec suite.

use agent_client_protocol::schema::v1::{
    ContentChunk, InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionNotification, SessionUpdate, StopReason,
};
use agent_client_protocol::{Agent, Client, ConnectTo, Stdio, on_receive_request};

/// Returns the echo agent, ready to connect to a client.
pub fn agent() -> impl ConnectTo<Client> {
    Agent
        .builder()
        .on_receive_request(
            async |req: InitializeRequest, responder, _| {
                responder.respond(InitializeResponse::new(req.protocol_version))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async |_: NewSessionRequest, responder, _| {
                responder.respond(NewSessionResponse::new("echo"))
            },
            on_receive_request!(),
        )
        .on_receive_request(
            async |req: PromptRequest, responder, cx| {
                for block in req.prompt {
                    let chunk = SessionUpdate::AgentMessageChunk(ContentChunk::new(block));
                    cx.send_notification(SessionNotification::new(req.session_id.clone(), chunk))?;
                }
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            on_receive_request!(),
        )
}

#[tokio::main]
#[allow(dead_code)]
async fn main() -> agent_client_protocol::Result<()> {
    agent().connect_to(Stdio::new()).await
}
