use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicConfig {
    pub server_name: Option<String>,
    pub server_title: Option<String>,
    pub default_theme: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(rename = "statusText", default)]
    pub status_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelItem {
    pub id: i64,
    pub name: String,
    pub code: String,
    pub topic: Option<String>,
    pub is_dm: i64,
    pub voice_enabled: i64,
    pub text_enabled: i64,
    pub read_only: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageItem {
    pub id: i64,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub user_id: Option<i64>,
    #[serde(default)]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessageEvent {
    #[serde(rename = "channelCode")]
    pub channel_code: String,
    pub message: ChatMessageItem,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageHistoryEvent {
    #[serde(rename = "channelCode")]
    pub channel_code: String,
    pub messages: Vec<ChatMessageItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResponse {
    pub url: String,
    #[serde(rename = "originalName")]
    pub original_name: Option<String>,
    #[serde(rename = "fileSize")]
    pub file_size: Option<u64>,
    #[serde(rename = "isImage")]
    pub is_image: Option<bool>,
    pub mimetype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRole {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub level: Option<i64>,
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineUserItem {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "statusText", default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub role: Option<UserRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineUsersEvent {
    #[serde(rename = "channelCode")]
    pub channel_code: String,
    pub users: Vec<OnlineUserItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineUsersDeltaEvent {
    #[serde(rename = "channelCode")]
    pub channel_code: String,
    #[serde(default)]
    pub upsert: Vec<OnlineUserItem>,
    #[serde(default)]
    pub remove: Vec<i64>,
}

#[derive(Debug, Clone)]
pub enum HavenEvent {
    Connected(String),
    SessionInfo(UserInfo),
    ChannelsList(Vec<ChannelItem>),
    OnlineUsers(OnlineUsersEvent),
    OnlineUsersDelta(OnlineUsersDeltaEvent),
    MessageHistory(MessageHistoryEvent),
    NewMessage(NewMessageEvent),
    VoiceExistingUsers { channel_code: String, users: Vec<serde_json::Value> },
    VoiceUserJoined { channel_code: Option<String>, user: serde_json::Value },
    VoiceUserLeft { user_id: Option<i64> },
    VoiceSpeaking { user_id: Option<i64>, speaking: bool },
    VoiceUsersUpdate { channel_code: String, users: Vec<serde_json::Value> },
    PingRtt(u32),
    Disconnected(String),
}
