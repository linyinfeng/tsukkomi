use std::sync::Arc;

use clap::Parser;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tsukkomi::chat::ChatManager;

#[derive(Parser)]
struct Args {
    #[arg(long, env = "TELOXIDE_TOKEN")]
    token: String,
    #[arg(long, required = true, value_delimiter = ',', env = "TELEGRAM_CHATS")]
    chats: Vec<i64>,
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

    let args = Arc::new(Args::parse());
    let manager = Arc::new(ChatManager::new()?);

    let bot = Bot::new(args.token.clone());

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(command_handler),
        )
        .branch(
            Update::filter_message()
                .filter({
                    let args = args.clone();
                    move |msg: Message| args.chats.contains(&msg.chat.id.0)
                })
                .endpoint(echo_handler),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![args, manager])
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

async fn echo_handler(
    _args: Arc<Args>,
    manager: Arc<ChatManager>,
    bot: Bot,
    msg: Message,
) -> Result<(), Error> {
    if let Some(text) = msg.text() {
        let reply = manager
            .reply(&msg.chat.id.0.to_string(), text)
            .await
            .map_err(|e| format!("AI reply error: {e}"))?;
        bot.send_message(msg.chat.id, reply).await?;
    }
    Ok(())
}
