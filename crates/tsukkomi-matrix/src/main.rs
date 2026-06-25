use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use matrix_sdk::{
    AuthSession, Client, SessionMeta,
    authentication::{SessionTokens, matrix::MatrixSession},
    config::SyncSettings,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    event_handler::Ctx,
    room::Room,
    ruma::events::room::member::{MembershipState, StrippedRoomMemberEvent},
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
    },
};
use tracing::error;
use tsukkomi::chat::{ChatManager, MessageBody, MessagePayload};
use tsukkomi::cli::TsukkomiOptions;

#[derive(Clone)]
struct StartupTime(i64);

#[derive(Clone, Debug, Parser)]
struct Options {
    #[arg(long, env = "MATRIX_HOMESERVER")]
    homeserver: String,
    #[arg(long, env = "MATRIX_USERNAME")]
    username: String,
    #[arg(long, env = "MATRIX_PASSWORD")]
    password: String,
    #[arg(long, required = true, value_delimiter = ',', env = "MATRIX_ROOMS")]
    rooms: Vec<String>,

    #[arg(long, default_value = "./matrix-store", env = "MATRIX_STORE_DIR")]
    matrix_store_dir: String,

    #[arg(
        long,
        default_value = "matrix-session.json",
        env = "MATRIX_SESSION_FILE"
    )]
    matrix_session_file: String,

    #[arg(env = "MATRIX_RECOVERY_KEY", hide = true)]
    matrix_recovery_key: Option<String>,

    #[command(flatten)]
    tsukkomi: TsukkomiOptions,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tsukkomi::utils::init_tracing();

    let opts = Arc::new(Options::parse());
    tracing::debug!(
        matrix_recovery_key = opts.matrix_recovery_key.is_some(),
        "Parsed options"
    );

    std::fs::create_dir_all(&opts.matrix_store_dir)?;
    tracing::debug!(store_dir = %opts.matrix_store_dir, "Matrix store directory ready");

    let client = Client::builder()
        .homeserver_url(&opts.homeserver)
        .sqlite_store(&opts.matrix_store_dir, None)
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: false,
            backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
            auto_enable_backups: false,
        })
        .build()
        .await?;

    ensure_session(&client, &opts).await?;

    let bot_user_id = client.user_id().unwrap();
    let bot_display_name = bot_user_id.localpart();
    tracing::info!("Logged in as {bot_user_id}");

    let startup_ms = Utc::now().timestamp_millis();
    tracing::info!(startup_ms, "Skipping messages before this timestamp");

    let manager = Arc::new(ChatManager::new(
        opts.tsukkomi.clone(),
        bot_user_id.as_str(),
        bot_display_name,
    )?);

    client.add_event_handler_context(opts.clone());
    client.add_event_handler_context(manager);
    client.add_event_handler_context(StartupTime(startup_ms));
    client.add_event_handler(on_room_invite);
    client.add_event_handler(on_room_message);

    tracing::info!("Starting sync loop");
    loop {
        if let Err(e) = client.sync(SyncSettings::default()).await {
            error!("Sync error: {e}");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}

async fn ensure_session(client: &Client, opts: &Options) -> anyhow::Result<()> {
    let Ok(json) = std::fs::read_to_string(&opts.matrix_session_file) else {
        return do_login(client, opts).await;
    };
    let Ok(session) = serde_json::from_str::<MatrixSession>(&json) else {
        return do_login(client, opts).await;
    };

    if let Err(e) = client.restore_session(AuthSession::Matrix(session)).await {
        tracing::warn!("Session restore failed: {e}");
        return do_login(client, opts).await;
    }

    tracing::info!("Session restored from {}", opts.matrix_session_file);

    try_import_recovery_key(client, opts).await;

    Ok(())
}

async fn do_login(client: &Client, opts: &Options) -> anyhow::Result<()> {
    tracing::info!("No valid session, logging in as new device");

    let login_response = client
        .matrix_auth()
        .login_username(&opts.username, &opts.password)
        .device_id("tsukkomi-bot")
        .initial_device_display_name("tsukkomi-bot")
        .send()
        .await?;

    let session = MatrixSession {
        meta: SessionMeta {
            user_id: login_response.user_id,
            device_id: login_response.device_id,
        },
        tokens: SessionTokens {
            access_token: login_response.access_token,
            refresh_token: login_response.refresh_token,
        },
    };

    let json = serde_json::to_string_pretty(&session)?;
    if let Some(parent) = std::path::Path::new(&opts.matrix_session_file).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&opts.matrix_session_file, json)?;
    tracing::debug!("Session saved to {}", opts.matrix_session_file);

    try_import_recovery_key(client, opts).await;

    Ok(())
}

async fn try_import_recovery_key(client: &Client, opts: &Options) {
    let Some(ref key) = opts.matrix_recovery_key else {
        tracing::debug!("No recovery key configured, skipping");
        return;
    };
    tracing::debug!("Attempting to import recovery key");
    match client.encryption().recovery().recover(key).await {
        Ok(_) => tracing::info!("Recovery key imported, backup download enabled"),
        Err(e) => tracing::warn!("Failed to import recovery key: {e}"),
    }
}

async fn on_room_invite(event: StrippedRoomMemberEvent, room: Room, client: Client) {
    let own_user_id = match client.user_id() {
        Some(uid) => uid,
        None => return,
    };

    if event.state_key != *own_user_id {
        return;
    }

    if event.content.membership != MembershipState::Invite {
        return;
    }

    tracing::info!("Joining room {}", room.room_id());
    if let Err(e) = room.join().await {
        tracing::error!("Failed to join room {}: {e}", room.room_id());
    }
}

async fn on_room_message(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    client: Client,
    Ctx(opts): Ctx<Arc<Options>>,
    Ctx(manager): Ctx<Arc<ChatManager>>,
    Ctx(StartupTime(startup_ms)): Ctx<StartupTime>,
) {
    let own_user_id = match client.user_id() {
        Some(uid) => uid,
        None => return,
    };

    if event.sender == own_user_id {
        return;
    }

    if !opts.rooms.contains(&room.room_id().to_string()) {
        return;
    }

    // Skip messages sent before this bot instance started.
    // Without this, the initial sync feeds old history to the LLM.
    if i64::from(event.origin_server_ts.get()) < startup_ms {
        return;
    }

    let body = match event.content.msgtype {
        MessageType::Text(ref text) => text.body.clone(),
        _ => return,
    };

    let msg = MessagePayload {
        user_id: event.sender.to_string(),
        display_name: event.sender.localpart().to_string(),
        body: MessageBody::Text(body),
        sent_at: chrono::DateTime::from_timestamp_millis(i64::from(event.origin_server_ts.get()))
            .unwrap_or_default(),
        reply_to_user_id: None,
        debouncing: false,
    };

    match manager.reply(room.room_id().as_str(), msg).await {
        Ok(Some(response)) => {
            let content = RoomMessageEventContent::text_plain(response.text);
            let _ = room.send(content).await;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("AI reply error: {e}");
        }
    }
}
