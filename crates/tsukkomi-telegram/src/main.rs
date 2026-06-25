use std::sync::Arc;

use base64::Engine as _;
use clap::Parser;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tsukkomi::chat::{ChatManager, MessageBody, MessagePayload};
use tsukkomi::cli::TsukkomiOptions;

#[derive(Debug, Parser)]
struct Options {
    #[arg(long, env = "TELOXIDE_TOKEN")]
    token: String,
    #[arg(long, required = true, value_delimiter = ',', env = "TELEGRAM_CHATS")]
    chats: Vec<i64>,

    #[command(flatten)]
    tsukkomi: TsukkomiOptions,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum Command {
    #[command(description = "Get the current chat ID")]
    ChatId,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tsukkomi::utils::init_tracing();

    let opts = Arc::new(Options::parse());
    tracing::debug!(?opts, "Parsed options");
    let bot = Bot::new(opts.token.clone());
    let bot_me = bot.get_me().await?;
    let bot_user_id = bot_me.id.0.to_string();
    let bot_display_name = bot_me.full_name();

    let manager = Arc::new(ChatManager::new(
        opts.tsukkomi.clone(),
        &bot_user_id,
        &bot_display_name,
    )?);

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(command_handler),
        )
        .branch(
            Update::filter_message()
                .filter({
                    let opts = opts.clone();
                    move |msg: Message| opts.chats.contains(&msg.chat.id.0)
                })
                .endpoint(msg_handler),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![opts, manager])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn command_handler(bot: Bot, msg: Message, cmd: Command) -> Result<(), Error> {
    match cmd {
        Command::ChatId => {
            bot.send_message(msg.chat.id, format!("Chat ID: {}", msg.chat.id.0))
                .await?;
        }
    }
    Ok(())
}

async fn msg_handler(
    _opts: Arc<Options>,
    manager: Arc<ChatManager>,
    bot: Bot,
    msg: Message,
) -> Result<(), Error> {
    let (user_id, display_name) = msg.from.as_ref().map_or_else(
        || ("unknown".into(), "Unknown".into()),
        |user| (user.id.0.to_string(), user.full_name()),
    );

    let reply_to_user_id = msg
        .reply_to_message()
        .and_then(|m| m.from.as_ref().map(|u| u.id.0.to_string()));

    let body = if let Some(photos) = msg.photo() {
        let largest = match photos.last() {
            Some(p) => p,
            None => return Ok(()),
        };
        let file = bot.get_file(largest.file.id.clone()).await?;
        let mut buf = Vec::new();
        bot.download_file(&file.path, &mut buf).await?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
        let Some(mime) = infer::get(&buf).map(|k| k.mime_type()) else {
            tracing::warn!("Cannot determine image MIME type, skipping");
            return Ok(());
        };
        MessageBody::Image {
            base64: b64,
            media_type: mime.to_string(),
            caption: msg.caption().map(|c| c.to_string()),
        }
    } else if let Some(text) = msg.text() {
        MessageBody::Text(text.to_string())
    } else {
        return Ok(());
    };

    let payload = MessagePayload {
        user_id,
        display_name,
        body,
        sent_at: msg.date,
        reply_to_user_id,
        debouncing: false,
    };

    match manager.reply(&msg.chat.id.0.to_string(), payload).await {
        Ok(Some(response)) => {
            bot.send_message(msg.chat.id, response.text).await?;
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("AI reply error: {e}");
        }
    }
    Ok(())
}
