use std::sync::Arc;

use clap::Parser;
use matrix_sdk::{
    config::SyncSettings,
    event_handler::Ctx,
    room::Room,
    ruma::events::room::message::{
        MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
    },
    Client,
};
use tracing::error;

#[derive(Clone, Parser)]
struct Args {
    #[arg(long, env = "MATRIX_HOMESERVER")]
    homeserver: String,
    #[arg(long, env = "MATRIX_USERNAME")]
    username: String,
    #[arg(long, env = "MATRIX_PASSWORD")]
    password: String,
    #[arg(long, required = true, value_delimiter = ',', env = "MATRIX_ROOMS")]
    rooms: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tsukkomi::utils::init_tracing();

    let args = Arc::new(Args::parse());

    let client = Client::builder()
        .homeserver_url(&args.homeserver)
        .build()
        .await?;

    client
        .matrix_auth()
        .login_username(&args.username, &args.password)
        .send()
        .await?;

    tracing::info!("Logged in as {}", client.user_id().unwrap());

    client.add_event_handler_context(args.clone());
    client.add_event_handler(on_room_message);

    tracing::info!("Starting sync loop");
    loop {
        if let Err(e) = client.sync(SyncSettings::default()).await {
            error!("Sync error: {e}");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}

async fn on_room_message(
    event: OriginalSyncRoomMessageEvent,
    room: Room,
    client: Client,
    args: Ctx<Arc<Args>>,
) {
    let own_user_id = match client.user_id() {
        Some(uid) => uid,
        None => return,
    };

    if event.sender == own_user_id {
        return;
    }

    if !args.rooms.contains(&room.room_id().to_string()) {
        return;
    }

    let body = match event.content.msgtype {
        MessageType::Text(text) => text.body,
        _ => return,
    };

    let content = RoomMessageEventContent::text_plain(tsukkomi::reply_to(&body));
    let _ = room.send(content).await;
}
