use crate::net::models::{
    ChannelItem, HavenEvent, MessageHistoryEvent, NewMessageEvent, OnlineUsersDeltaEvent,
    OnlineUsersEvent, UserInfo,
};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

#[derive(Debug, Clone)]
pub enum OutgoingCommand {
    #[allow(dead_code)]
    GetChannels,
    EnterChannel { code: String },
    SendMessage { code: String, content: String, client_nonce: Option<String> },
    JoinVoice { code: String },
    LeaveVoice { code: String },
    VoiceSpeaking { speaking: bool },
    VoiceMuteState { code: String, muted: bool },
}

pub struct HavenSocketClient {
    cmd_tx: mpsc::UnboundedSender<OutgoingCommand>,
}

async fn establish_socket(
    ws_url: &str,
    token: &str,
) -> Result<(WsWrite, WsRead, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Connecting to WebSocket: {}", ws_url);
    let handshake_timeout = Duration::from_secs(10);
    let (ws_stream, _) = tokio::time::timeout(handshake_timeout, connect_async(ws_url))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "WebSocket connect timed out"))??;
    let (mut write, mut read) = ws_stream.split();

    match tokio::time::timeout(handshake_timeout, read.next()).await {
        Ok(Some(Ok(Message::Text(msg)))) if msg.starts_with('0') => {
            log::info!("Engine.IO Open: {}", msg);
        }
        Ok(Some(Ok(other))) => {
            return Err(format!("Unexpected Engine.IO handshake frame: {:?}", other).into());
        }
        Ok(Some(Err(e))) => return Err(Box::new(e)),
        Ok(None) => return Err("WebSocket closed before Engine.IO open".into()),
        Err(_) => return Err("Timed out waiting for Engine.IO open".into()),
    }

    let connect_pkt = format!(
        "40{}",
        json!({
            "token": token,
            "presenceDeltas": 1
        })
    );
    write.send(Message::Text(connect_pkt)).await?;

    let ack_sid = match tokio::time::timeout(handshake_timeout, read.next()).await {
        Ok(Some(Ok(Message::Text(ack)))) if ack.starts_with("40") => {
            log::info!("Socket.IO Ack: {}", ack);
            Some(ack)
        }
        Ok(Some(Ok(Message::Text(err_pkt)))) if err_pkt.starts_with("44") => {
            return Err(format!("Socket.IO authentication rejected: {}", err_pkt).into());
        }
        Ok(Some(Ok(other))) => {
            return Err(format!("Unexpected Socket.IO connect response: {:?}", other).into());
        }
        Ok(Some(Err(e))) => return Err(Box::new(e)),
        Ok(None) => return Err("WebSocket closed before Socket.IO ack".into()),
        Err(_) => return Err("Timed out waiting for Socket.IO ack".into()),
    };

    let init_pkt = format!("42{}", json!(["get-channels", {}]));
    write.send(Message::Text(init_pkt)).await?;

    Ok((write, read, ack_sid))
}

fn dispatch_socket_event(
    event_name: &str,
    payload: Option<&serde_json::Value>,
    ping_origin: Instant,
    event_tx: &mpsc::UnboundedSender<HavenEvent>,
) {
    match event_name {
        "session-info" => {
            if let Some(p) = payload {
                match serde_json::from_value::<UserInfo>(p.clone()) {
                    Ok(user) => {
                        let _ = event_tx.send(HavenEvent::SessionInfo(user));
                    }
                    Err(e) => log::error!("Deserialization error on session-info: {:?}", e),
                }
            }
        }
        "channels-list" => {
            if let Some(p) = payload {
                if let Ok(channels) = serde_json::from_value::<Vec<ChannelItem>>(p.clone()) {
                    let _ = event_tx.send(HavenEvent::ChannelsList(channels));
                }
            }
        }
        "message-history" => {
            if let Some(p) = payload {
                match serde_json::from_value::<MessageHistoryEvent>(p.clone()) {
                    Ok(hist) => {
                        let _ = event_tx.send(HavenEvent::MessageHistory(hist));
                    }
                    Err(e) => log::error!("Deserialization error on message-history: {:?}", e),
                }
            }
        }
        "new-message" => {
            if let Some(p) = payload {
                match serde_json::from_value::<NewMessageEvent>(p.clone()) {
                    Ok(new_msg) => {
                        let _ = event_tx.send(HavenEvent::NewMessage(new_msg));
                    }
                    Err(e) => log::error!("Deserialization error on new-message: {:?}", e),
                }
            }
        }
        "online-users" => {
            if let Some(p) = payload {
                match serde_json::from_value::<OnlineUsersEvent>(p.clone()) {
                    Ok(online_evt) => {
                        let _ = event_tx.send(HavenEvent::OnlineUsers(online_evt));
                    }
                    Err(e) => log::error!("Deserialization error on online-users: {:?}", e),
                }
            }
        }
        "online-users-delta" => {
            if let Some(p) = payload {
                match serde_json::from_value::<OnlineUsersDeltaEvent>(p.clone()) {
                    Ok(delta_evt) => {
                        let _ = event_tx.send(HavenEvent::OnlineUsersDelta(delta_evt));
                    }
                    Err(e) => log::error!("Deserialization error on online-users-delta: {:?}", e),
                }
            }
        }
        "pong-check" => {
            if let Some(sent_t) = payload.and_then(|p| p.get("t")).and_then(|v| v.as_u64()) {
                let now_ms = ping_origin.elapsed().as_millis() as u64;
                let rtt = now_ms.saturating_sub(sent_t).max(1) as u32;
                let _ = event_tx.send(HavenEvent::PingRtt(rtt));
            }
        }
        "voice-users-update" | "voice-count-update" => {
            if let Some(p) = payload {
                let code = p
                    .get("channelCode")
                    .or_else(|| p.get("code"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let users = p.get("users").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let _ = event_tx.send(HavenEvent::VoiceUsersUpdate {
                    channel_code: code,
                    users,
                });
            }
        }
        "voice-existing-users" => {
            if let Some(p) = payload {
                let code = p.get("channelCode").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let users = p.get("users").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let _ = event_tx.send(HavenEvent::VoiceExistingUsers {
                    channel_code: code,
                    users,
                });
            }
        }
        "voice-user-joined" => {
            if let Some(p) = payload {
                let code = p.get("channelCode").and_then(|v| v.as_str()).map(|s| s.to_string());
                let user = p.get("user").cloned().unwrap_or(p.clone());
                let _ = event_tx.send(HavenEvent::VoiceUserJoined {
                    channel_code: code,
                    user,
                });
            }
        }
        "voice-user-left" => {
            if let Some(p) = payload {
                let uid = p.get("userId").and_then(|v| v.as_i64());
                let _ = event_tx.send(HavenEvent::VoiceUserLeft { user_id: uid });
            }
        }
        "voice-speaking" => {
            if let Some(p) = payload {
                let uid = p.get("userId").and_then(|v| v.as_i64());
                let spk = p.get("speaking").and_then(|v| v.as_bool()).unwrap_or(false);
                let _ = event_tx.send(HavenEvent::VoiceSpeaking {
                    user_id: uid,
                    speaking: spk,
                });
            }
        }
        "error-msg" => {
            let msg = payload
                .and_then(|p| p.as_str())
                .unwrap_or("Server rejected request")
                .to_string();
            let _ = event_tx.send(HavenEvent::Error(msg));
        }
        _ => {
            log::debug!("Ignored event: {}", event_name);
        }
    }
}

fn backoff_delay(attempt: u32) -> Duration {
    let base_secs = 1u64.checked_shl(attempt.min(5)).unwrap_or(32).min(30);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() % 500) as u64)
        .unwrap_or(100);
    Duration::from_secs(base_secs) + Duration::from_millis(jitter_ms)
}

async fn write_command(
    write: &mut WsWrite,
    cmd: OutgoingCommand,
    last_channel: &mut Option<String>,
    last_voice: &mut Option<String>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    match cmd {
        OutgoingCommand::GetChannels => {
            write.send(Message::Text(format!("42{}", json!(["get-channels", {}])))).await
        }
        OutgoingCommand::EnterChannel { code } => {
            *last_channel = Some(code.clone());
            let enter = format!("42{}", json!(["enter-channel", { "code": code }]));
            write.send(Message::Text(enter)).await?;
            let hist = format!("42{}", json!(["get-messages", { "code": code }]));
            write.send(Message::Text(hist)).await
        }
        OutgoingCommand::SendMessage { code, content, client_nonce } => {
            let body = if let Some(nonce) = client_nonce {
                json!({ "code": code, "content": content, "clientNonce": nonce })
            } else {
                json!({ "code": code, "content": content })
            };
            let pkt = format!("42{}", json!(["send-message", body]));
            write.send(Message::Text(pkt)).await
        }
        OutgoingCommand::JoinVoice { code } => {
            *last_voice = Some(code.clone());
            let pkt = format!("42{}", json!(["voice-join", { "code": code }]));
            write.send(Message::Text(pkt)).await
        }
        OutgoingCommand::LeaveVoice { code } => {
            *last_voice = None;
            let pkt = format!("42{}", json!(["voice-leave", { "code": code }]));
            write.send(Message::Text(pkt)).await
        }
        OutgoingCommand::VoiceSpeaking { speaking } => {
            let pkt = format!("42{}", json!(["voice-speaking", { "speaking": speaking }]));
            write.send(Message::Text(pkt)).await
        }
        OutgoingCommand::VoiceMuteState { code, muted } => {
            let pkt = format!("42{}", json!(["voice-mute-state", { "code": code, "muted": muted }]));
            write.send(Message::Text(pkt)).await
        }
    }
}

impl HavenSocketClient {
    pub async fn connect(
        ws_url: &str,
        token: &str,
        event_tx: mpsc::UnboundedSender<HavenEvent>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let (mut write, mut read, initial_ack) = establish_socket(ws_url, token).await?;
        if let Some(ack) = initial_ack {
            let _ = event_tx.send(HavenEvent::Connected(ack));
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<OutgoingCommand>();
        let ws_url_owned = ws_url.to_string();
        let token_owned = token.to_string();

        tokio::spawn(async move {
            let ping_origin = Instant::now();
            let mut last_channel: Option<String> = None;
            let mut last_voice: Option<String> = None;

            'supervisor: loop {
                let mut ping_ticker = tokio::time::interval(Duration::from_secs(10));
                ping_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let disconnect_reason: String;

                loop {
                    tokio::select! {
                        _ = ping_ticker.tick() => {
                            let t_ms = ping_origin.elapsed().as_millis() as u64;
                            let pkt = format!("42{}", json!(["ping-check", { "t": t_ms }]));
                            if let Err(e) = write.send(Message::Text(pkt)).await {
                                disconnect_reason = format!("Ping write failed: {e}");
                                break;
                            }
                        }
                        cmd_opt = cmd_rx.recv() => {
                            let Some(cmd) = cmd_opt else {
                                break 'supervisor;
                            };
                            if let Err(e) = write_command(&mut write, cmd, &mut last_channel, &mut last_voice).await {
                                log::error!("Failed to send outgoing packet: {:?}", e);
                                disconnect_reason = format!("Write error: {e}");
                                break;
                            }
                        }
                        msg_res = read.next() => {
                            match msg_res {
                                Some(Ok(Message::Text(text))) => {
                                    if text == "2" {
                                        if let Err(e) = write.send(Message::Text("3".to_string())).await {
                                            disconnect_reason = format!("Pong send failed: {e}");
                                            break;
                                        }
                                        continue;
                                    }
                                    if let Some(stripped) = text.strip_prefix("42") {
                                        if let Ok(event_val) = serde_json::from_str::<serde_json::Value>(stripped) {
                                            if let Some(arr) = event_val.as_array() {
                                                if let Some(event_name) = arr.first().and_then(|v| v.as_str()) {
                                                    dispatch_socket_event(event_name, arr.get(1), ping_origin, &event_tx);
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(_))) | None => {
                                    log::warn!("WebSocket connection closed.");
                                    disconnect_reason = "Server closed connection".to_string();
                                    break;
                                }
                                Some(Err(e)) => {
                                    log::error!("WebSocket error: {:?}", e);
                                    disconnect_reason = e.to_string();
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                }

                let _ = event_tx.send(HavenEvent::Disconnected(disconnect_reason));

                // Reconnect loop with exponential backoff (1s..30s + jitter)
                let mut attempt = 0u32;
                loop {
                    let delay = backoff_delay(attempt);
                    log::info!("Reconnecting WebSocket in {:?} (attempt {})...", delay, attempt + 1);
                    let sleep = tokio::time::sleep(delay);
                    tokio::pin!(sleep);
                    loop {
                        tokio::select! {
                            _ = &mut sleep => break,
                            cmd_opt = cmd_rx.recv() => {
                                match cmd_opt {
                                    None => break 'supervisor,
                                    Some(OutgoingCommand::EnterChannel { code }) => last_channel = Some(code),
                                    Some(OutgoingCommand::JoinVoice { code }) => last_voice = Some(code),
                                    Some(OutgoingCommand::LeaveVoice { .. }) => last_voice = None,
                                    _ => {}
                                }
                            }
                        }
                    }

                    match establish_socket(&ws_url_owned, &token_owned).await {
                        Ok((mut new_w, new_r, ack)) => {
                            if let Some(ack_str) = ack {
                                let _ = event_tx.send(HavenEvent::Connected(ack_str));
                            }
                            if let Some(ch) = &last_channel {
                                let _ = write_command(
                                    &mut new_w,
                                    OutgoingCommand::EnterChannel { code: ch.clone() },
                                    &mut last_channel,
                                    &mut last_voice,
                                )
                                .await;
                            }
                            if let Some(vc) = &last_voice {
                                let _ = write_command(
                                    &mut new_w,
                                    OutgoingCommand::JoinVoice { code: vc.clone() },
                                    &mut last_channel,
                                    &mut last_voice,
                                )
                                .await;
                            }
                            write = new_w;
                            read = new_r;
                            break;
                        }
                        Err(e) => {
                            log::warn!("WebSocket reconnect failed: {:?}", e);
                            attempt = attempt.saturating_add(1);
                        }
                    }
                }
            }
        });

        Ok(Self { cmd_tx })
    }

    pub fn send_message(&self, code: &str, content: &str) {
        self.send_message_with_nonce(code, content, None);
    }

    pub fn send_message_with_nonce(&self, code: &str, content: &str, client_nonce: Option<&str>) {
        let _ = self.cmd_tx.send(OutgoingCommand::SendMessage {
            code: code.to_string(),
            content: content.to_string(),
            client_nonce: client_nonce.map(str::to_string),
        });
    }

    pub fn enter_channel(&self, code: &str) {
        let _ = self.cmd_tx.send(OutgoingCommand::EnterChannel {
            code: code.to_string(),
        });
    }

    pub fn join_voice(&self, code: &str) {
        let _ = self.cmd_tx.send(OutgoingCommand::JoinVoice {
            code: code.to_string(),
        });
    }

    pub fn leave_voice(&self, code: &str) {
        let _ = self.cmd_tx.send(OutgoingCommand::LeaveVoice {
            code: code.to_string(),
        });
    }

    pub fn send_voice_speaking(&self, speaking: bool) {
        let _ = self.cmd_tx.send(OutgoingCommand::VoiceSpeaking { speaking });
    }

    pub fn send_voice_mute_state(&self, code: &str, muted: bool) {
        let _ = self.cmd_tx.send(OutgoingCommand::VoiceMuteState {
            code: code.to_string(),
            muted,
        });
    }
}
