use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{Path, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{stream::StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::{RwLock, Mutex};
use anyhow::{Context, Result};


// Define shared state for the app
#[derive(Debug, Clone)]
struct AppState {
    connections: Arc<RwLock<HashMap<String, WebSocketSender>>>, // Online connections
    pending_messages: Arc<RwLock<HashMap<String, Vec<MessagePayload>>>>, // Offline messages
}

// Type for WebSocket sender
type WebSocketSender = Mutex<futures::stream::SplitSink<WebSocket, Message>>;


// Enum to represent client actions
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action_type", rename_all = "snake_case")]
enum ClientAction {
    SendMessage(MessagePayload),
    PresenceUpdate(PresencePayload),
    Disconnect(DisconnectPayload),
}

// Struct to represent a chat message
#[derive(Debug, Serialize, Deserialize)]
struct MessagePayload {
    sender: String,
    recipient: String,
    content: String,
}

// Struct for presence updates
#[derive(Debug, Serialize, Deserialize)]
struct PresencePayload {
    user: String,
    status: String, // e.g., "online", "offline"
}

// Struct for disconnection requests
#[derive(Debug, Serialize, Deserialize)]
struct DisconnectPayload {
    user: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let app_state = AppState {
        connections: Arc::new(RwLock::new(HashMap::new())),
        pending_messages: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/ws/:username", get(handle_websocket))
        .with_state(app_state);

        let addr = "127.0.0.1:3000".parse::<SocketAddr>().context("Failed to parse the address")?;
        let listener = tokio::net::TcpListener::bind(addr).await.context("Failed to bind the address")?;
        println!("Listning on port {}", addr);
            
        axum::serve(listener, app).await.context("Server failed to run").unwrap();
        Ok(())
}

async fn handle_websocket(
    ws: WebSocketUpgrade,
    Path(username): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| websocket_handler(socket, state, username))
}

async fn websocket_handler(socket: WebSocket, state: AppState, username: String) {
    let (sender, mut receiver) = socket.split();
    let sender = Mutex::new(sender);

    state.connections.write().await.insert(username.clone(), sender);

    while let Some(Ok(message)) = receiver.next().await {
        if let Message::Text(payload) = message {
            if let Ok(action) = serde_json::from_str::<ClientAction>(&payload) {
                match action {
                    ClientAction::SendMessage(msg) => handle_send_message(msg, &state).await,
                    ClientAction::PresenceUpdate(presence) => handle_presence_update(presence, &state).await,
                    ClientAction::Disconnect(disconnect) => {
                        handle_disconnect(disconnect, &state).await;
                        break;
                    }
                }
            }
        }
    }

    state.connections.write().await.remove(&username);
}

async fn handle_send_message(payload: MessagePayload, state: &AppState) {
    let mut connections = state.connections.write().await;

    if let Some(recipient_sender) = connections.get_mut(&payload.recipient) {
        if recipient_sender
            .lock()
            .await
            .send(Message::Text(payload.content))
            .await
            .is_err()
        {
            eprintln!("Failed to send message to {}", payload.recipient);
        }
    } else {
        state
            .pending_messages
            .write()
            .await
            .entry(payload.recipient.clone())
            .or_default()
            .push(payload);
    }
}

async fn handle_presence_update(payload: PresencePayload, state: &AppState) {
    let mut connections = state.connections.write().await;

    match payload.status.as_str() {
        "online" => {
            println!("{} is online", payload.user);
        }
        "offline" => {
            connections.remove(&payload.user);
        }
        _ => {
            eprintln!("Invalid presence status: {}", payload.status);
        }
    }
}

async fn handle_disconnect(payload: DisconnectPayload, state: &AppState) {
    state.connections.write().await.remove(&payload.user);
}
