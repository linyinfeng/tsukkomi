use std::sync::Arc;

use anyhow::Context;
use backoff::ExponentialBackoff;
use backoff::future::retry;
use chrono::Utc;
use clap::Parser;
use matrix_sdk::{
    AuthSession, Client, SessionMeta,
    authentication::{SessionTokens, matrix::MatrixSession},
    config::SyncSettings,
    encryption::{BackupDownloadStrategy, EncryptionSettings},
    event_handler::Ctx,
    media::{MediaFormat, MediaRequestParameters},
    room::Room,
    ruma::events::room::member::{MembershipState, StrippedRoomMemberEvent},
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
    },
};
use tracing::error;
use tsukkomi::chat::{ChatInput, DefaultChatManager, ImageData};
use tsukkomi::cli::TsukkomiOptions;

#[derive(Clone)]
struct StartupTime(chrono::DateTime<Utc>);

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

    std::fs::create_dir_all(&opts.matrix_store_dir).with_context(|| {
        format!(
            "failed to create matrix store directory at {}",
            opts.matrix_store_dir
        )
    })?;
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
        .await
        .context("failed to build Matrix client")?;

    ensure_session(&client, &opts)
        .await
        .context("failed to ensure Matrix session")?;

    let bot_user_id = client
        .user_id()
        .expect("client must be authenticated after ensure_session");
    let bot_display_name = bot_user_id.localpart();
    tracing::info!("Logged in as {bot_user_id}");

    let startup = Utc::now();
    tracing::info!(startup = %startup, "Skipping messages before this time");

    let manager = Arc::new(
        DefaultChatManager::new(
            opts.tsukkomi.clone(),
            bot_user_id.as_str(),
            bot_display_name,
        )
        .context("failed to create ChatManager")?,
    );

    client.add_event_handler_context(opts.clone());
    client.add_event_handler_context(manager);
    client.add_event_handler_context(StartupTime(startup));
    client.add_event_handler(on_room_invite);
    client.add_event_handler(on_room_message);

    tracing::info!("Starting sync loop");
    loop {
        let backoff = ExponentialBackoff {
            max_elapsed_time: None,
            ..Default::default()
        };
        retry(backoff, || async {
            client.sync(SyncSettings::default()).await.map_err(|e| {
                error!("Sync error: {e}");
                backoff::Error::transient(e)
            })
        })
        .await
        .ok();
    }
}

async fn ensure_session(client: &Client, opts: &Options) -> anyhow::Result<()> {
    let json = match std::fs::read_to_string(&opts.matrix_session_file) {
        Ok(c) => c,
        Err(e) => {
            tracing::info!(
                "Session file {} not found, logging in: {e}",
                opts.matrix_session_file
            );
            return do_login(client, opts).await;
        }
    };
    let session: MatrixSession = match serde_json::from_str(&json) {
        Ok(s) => s,
        Err(e) => {
            error!(
                "Failed to parse session file {}: {e}",
                opts.matrix_session_file
            );
            return do_login(client, opts).await;
        }
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
        .await
        .with_context(|| format!("Matrix login failed for user {}", opts.username))?;

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

    let json =
        serde_json::to_string_pretty(&session).context("failed to serialize Matrix session")?;
    if let Some(parent) = std::path::Path::new(&opts.matrix_session_file).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&opts.matrix_session_file, json)
        .with_context(|| format!("failed to write session to {}", opts.matrix_session_file))?;
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
    Ctx(manager): Ctx<Arc<DefaultChatManager>>,
    Ctx(StartupTime(startup)): Ctx<StartupTime>,
) {
    let own_user_id = match client.user_id() {
        Some(uid) => uid,
        None => return,
    };

    if event.sender == own_user_id {
        return;
    }

    // Skip edits (m.replace) to avoid duplicate AI replies.
    if event.content.relates_to.is_some() {
        return;
    }

    if !opts.rooms.contains(&room.room_id().to_string()) {
        return;
    }

    // Skip messages sent before this bot instance started.
    // Without this, the initial sync feeds old history to the LLM.
    let Some(sent_at) =
        chrono::DateTime::from_timestamp_millis(i64::from(event.origin_server_ts.get()))
    else {
        tracing::warn!(
            ts = ?event.origin_server_ts,
            "Invalid event timestamp, skipping"
        );
        return;
    };
    if sent_at < startup {
        return;
    }

    let input = match &event.content.msgtype {
        MessageType::Text(text) => ChatInput {
            text: Some(text.body.clone()),
            images: Vec::new(),
            user_id: event.sender.to_string(),
            display_name: event.sender.localpart().to_string(),
            sent_at,
            reply_to_user_id: None,
        },
        MessageType::Image(image) => {
            let request = MediaRequestParameters {
                source: image.source.clone(),
                format: MediaFormat::File,
            };
            let data = match client.media().get_media_content(&request, true).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!("Failed to download image: {e}");
                    return;
                }
            };
            let mime = image
                .info
                .as_ref()
                .and_then(|i| i.mimetype.as_deref())
                .or_else(|| infer::get(&data).map(|k| k.mime_type()))
                .map(ToString::to_string);
            let caption = if image.body == image.filename.as_deref().unwrap_or("") {
                None
            } else {
                Some(image.body.clone())
            };
            ChatInput {
                text: caption,
                images: vec![ImageData {
                    data,
                    media_type: mime,
                }],
                user_id: event.sender.to_string(),
                display_name: event.sender.localpart().to_string(),
                sent_at,
                reply_to_user_id: None,
            }
        }
        _ => return,
    };

    match manager.reply(room.room_id().as_str(), input).await {
        Ok(Some(response)) => {
            let content = RoomMessageEventContent::text_plain(response.text);
            if let Err(e) = room.send(content).await {
                tracing::error!("Failed to send reply: {e}");
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("AI reply error: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parsing_minimal() {
        let opts = Options::try_parse_from([
            "tsukkomi-matrix",
            "--homeserver",
            "https://matrix.example.com",
            "--username",
            "bot",
            "--password",
            "secret",
            "--rooms",
            "#room1:example.com,#room2:example.com",
        ])
        .unwrap();
        assert_eq!(opts.homeserver, "https://matrix.example.com");
        assert_eq!(opts.username, "bot");
        assert_eq!(opts.password, "secret");
        assert_eq!(opts.rooms, vec!["#room1:example.com", "#room2:example.com"]);
    }

    #[test]
    fn cli_parsing_default_store_dir_and_session_file() {
        let opts = Options::try_parse_from([
            "tsukkomi-matrix",
            "--homeserver",
            "https://example.com",
            "--username",
            "u",
            "--password",
            "p",
            "--rooms",
            "r",
        ])
        .unwrap();
        assert_eq!(opts.matrix_store_dir, "./matrix-store");
        assert_eq!(opts.matrix_session_file, "matrix-session.json");
    }

    #[test]
    fn cli_parsing_custom_store_dir() {
        let opts = Options::try_parse_from([
            "tsukkomi-matrix",
            "--homeserver",
            "https://example.com",
            "--username",
            "u",
            "--password",
            "p",
            "--rooms",
            "r",
            "--matrix-store-dir",
            "/custom/store",
            "--matrix-session-file",
            "/custom/session.json",
        ])
        .unwrap();
        assert_eq!(opts.matrix_store_dir, "/custom/store");
        assert_eq!(opts.matrix_session_file, "/custom/session.json");
    }

    #[test]
    fn cli_parsing_flattened_tsukkomi_options() {
        let opts = Options::try_parse_from([
            "tsukkomi-matrix",
            "--homeserver",
            "https://example.com",
            "--username",
            "u",
            "--password",
            "p",
            "--rooms",
            "r",
            "--memory-directory",
            "/tmp/mem",
            "--max-retries",
            "5",
        ])
        .unwrap();
        assert_eq!(opts.tsukkomi.memory_directory, "/tmp/mem");
        assert_eq!(opts.tsukkomi.max_retries, 5);
    }
}
