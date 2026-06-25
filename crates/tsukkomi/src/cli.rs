use clap::Args;

#[derive(Debug, Clone, Args)]
pub struct TsukkomiOptions {
    /// Inline system prompt (overrides embedded default).
    #[arg(long, env = "TSUKKOMI_SYSTEM_PROMPT")]
    pub system_prompt: Option<String>,

    /// Path to a markdown file containing the system prompt.
    /// Takes precedence over --system-prompt if both are set.
    #[arg(long, env = "TSUKKOMI_SYSTEM_PROMPT_FILE")]
    pub system_prompt_file: Option<String>,

    #[arg(long, env = "TSUKKOMI_MAX_RETRIES", default_value_t = 3)]
    pub max_retries: u32,

    #[arg(long, env = "TSUKKOMI_MEMORY_DIRECTORY", default_value = "memory")]
    pub memory_directory: String,

    #[arg(long, env = "TSUKKOMI_SLIDING_WINDOW", default_value_t = 200)]
    pub sliding_window: u32,

    #[arg(long, env = "TSUKKOMI_SUMMARY_MAX_CHARS", default_value_t = 2000)]
    pub summary_max_chars: usize,

    #[arg(long, env = "TSUKKOMI_SUMMARY_HEADER", default_value = "历史摘要")]
    pub summary_header: String,

    #[arg(long, env = "TSUKKOMI_BATCH_SIZE", default_value_t = 100)]
    pub batch_size: u32,

    /// Minimum seconds between replies in the same room.
    /// Prevents the bot from responding to every single message.
    #[arg(long, env = "TSUKKOMI_DEBOUNCE_SECS", default_value_t = 30)]
    pub debounce_secs: u32,
}
