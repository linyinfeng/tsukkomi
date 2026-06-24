use clap::Args;

#[derive(Clone, Args)]
pub struct TsukkomiOptions {
    #[arg(
        long,
        env = "TSUKKOMI_SYSTEM_PROMPT",
        default_value_t = crate::chat::system_prompt().to_string()
    )]
    pub system_prompt: String,

    #[arg(long, env = "TSUKKOMI_MAX_RETRIES", default_value_t = 3)]
    pub max_retries: u32,
}
