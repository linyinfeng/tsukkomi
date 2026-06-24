use std::sync::Arc;

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

#[derive(Clone, Parser)]
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

    let client = Client::builder()
        .homeserver_url(&opts.homeserver)
        .build()
        .await?;

    client
        .matrix_auth()
        .login_username(&opts.username, &opts.password)
        .send()
        .await?;

    tracing::info!("Logged in as {}", client.user_id().unwrap());

    let manager = Arc::new(ChatManager::new(opts.tsukkomi.clone())?);

    client.add_event_handler_context(opts.clone());
    client.add_event_handler_context(manager);
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
    opts: Ctx<Arc<Options>>,
    manager: Ctx<Arc<ChatManager>>,
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

    let body = match event.content.msgtype {
        MessageType::Text(text) => text.body,
        _ => return,
    };

    let msg = MessagePayload {
        user_id: event.sender.to_string(),
        display_name: event.sender.localpart().to_string(),
        body: MessageBody::Text(body),
    };

    match manager.reply(room.room_id().as_str(), msg).await {
        Ok(Some(reply)) => {
            let content = RoomMessageEventContent::text_plain(reply);
            let _ = room.send(content).await;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("AI reply error: {e}");
        }
    }
}
