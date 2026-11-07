//! Placeholder for `rich-text.ts`. The chat slice fills this in.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(String),
    Code(String),
}

pub fn parse_rich_text(text: &str) -> Vec<Block> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![Block::Paragraph(text.to_string())]
    }
}
