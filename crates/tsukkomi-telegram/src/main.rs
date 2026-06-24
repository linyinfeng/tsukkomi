use std::sync::Arc;

use clap::Parser;
use teloxide::prelude::*;

#[derive(Parser)]
struct Args {
    #[arg(long, env = "TELOXIDE_TOKEN")]
    token: String,
    #[arg(long, required = true)]
    chats: Vec<i64>,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tsukkomi::utils::init_tracing();

    let args = Arc::new(Args::parse());

    let bot = Bot::new(args.token.clone());

    let handler = Update::filter_message()
        .filter({
            let args = args.clone();
            move |msg: Message| args.chats.contains(&msg.chat.id.0)
        })
        .endpoint(handler);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![args])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handler(_args: Arc<Args>, bot: Bot, msg: Message) -> Result<(), Error> {
    if let Some(text) = msg.text() {
        bot.send_message(msg.chat.id, tsukkomi::reply_to(text))
            .await?;
    }
    Ok(())
}
