pub mod utils;

pub fn reply_to(text: &str) -> String {
    format!("你说: {}", text)
}
