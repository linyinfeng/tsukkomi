use std::sync::Arc;

use clap::Parser;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tsukkomi::chat::{ChatInput, DefaultChatManager, ImageData};
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

    let manager = Arc::new(DefaultChatManager::new(
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
    manager: Arc<DefaultChatManager>,
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

    let input = if let Some(photos) = msg.photo() {
        let largest = match photos.last() {
            Some(p) => p,
            None => return Ok(()),
        };
        let file = bot.get_file(largest.file.id.clone()).await?;
        let mut buf = Vec::new();
        bot.download_file(&file.path, &mut buf).await?;
        ChatInput {
            text: msg.caption().map(|c| c.to_string()),
            images: vec![ImageData {
                media_type: infer::get(&buf)
                    .map(|k| k.mime_type())
                    .map(ToString::to_string),
                data: buf,
            }],
            user_id,
            display_name,
            sent_at: msg.date,
            reply_to_user_id,
        }
    } else if let Some(text) = msg.text() {
        ChatInput {
            text: Some(text.to_string()),
            images: Vec::new(),
            user_id,
            display_name,
            sent_at: msg.date,
            reply_to_user_id,
        }
    } else {
        return Ok(());
    };

    match manager.reply(&msg.chat.id.0.to_string(), input).await {
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
