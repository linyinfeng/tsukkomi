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

    #[arg(long, env = "TSUKKOMI_MEMORY_DIRECTORY", default_value = "memory")]
    pub memory_directory: String,

    /// Number of recent messages kept in the active window for the agent.
    #[arg(long, env = "TSUKKOMI_SLIDING_WINDOW", default_value_t = 50)]
    pub sliding_window: u32,

    /// DeepSeek model used for generating conversation summaries.
    #[arg(
        long,
        env = "TSUKKOMI_SUMMARY_MODEL",
        default_value = "deepseek-v4-flash"
    )]
    pub summary_model: String,

    /// Maximum character length of the generated summary.
    #[arg(long, env = "TSUKKOMI_SUMMARY_MAX_CHARS", default_value_t = 2000)]
    pub summary_max_chars: u32,

    /// Header text prepended to the summary in the prompt.
    #[arg(long, env = "TSUKKOMI_SUMMARY_HEADER", default_value = "历史摘要")]
    pub summary_header: String,
}
