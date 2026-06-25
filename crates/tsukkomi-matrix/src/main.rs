use std::sync::Arc;

use chrono::Utc;
use clap::Parser;
use matrix_sdk::{
    Client,
    config::SyncSettings,
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

    #[command(flatten)]
    tsukkomi: TsukkomiOptions,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tsukkomi::utils::init_tracing();

    let opts = Arc::new(Options::parse());
    tracing::debug!(?opts, "Parsed options");

    // TODO: For proper encrypted room support, persist the session:
    // 1. Add .sqlite_store() to persist sync state and crypto keys
    // 2. Restore session via client.restore_session() using a stored
    //    access_token + device_id instead of logging in every time
    // 3. Login with .device_id("tsukkomi-bot") to keep a consistent
    //    device identity so the crypto store doesn't break on restart
    // Currently we log in fresh every time, which creates a new device
    // and loses sync state. This works for unencrypted rooms only.
    let client = Client::builder()
        .homeserver_url(&opts.homeserver)
        .build()
        .await?;

    client
        .matrix_auth()
        .login_username(&opts.username, &opts.password)
        .send()
        .await?;

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
