use clap::Args;

#[derive(Clone, Args)]
pub struct TsukkomiOptions {
    #[arg(
        long,
        env = "TSUKKOMI_SYSTEM_PROMPT",
        default_value_t = crate::chat::system_prompt().to_string()
    )]
    pub system_prompt: String,
}
