use dioxus::prelude::*;

use crate::logic::rich_text::{Block, Inline, parse_rich_text};

#[component]
pub fn RichText(text: String) -> Element {
    let blocks = parse_rich_text(&text);
    rsx! {
        div { class: "rich-text",
            for (index, block) in blocks.into_iter().enumerate() {
                {
                    rsx! {
                        {render_block(index, block)}
                    }
                }
            }
        }
    }
}

fn render_block(index: usize, block: Block) -> Element {
    match block {
        Block::Code(text) => rsx! {
            pre { key: "{index}",
                code { "{text}" }
            }
        },
        Block::Paragraph(inlines) => rsx! {
            p { key: "{index}",
                for (inline_index, inline) in inlines.into_iter().enumerate() {
                    {render_inline(inline_index, inline)}
                }
            }
        },
    }
}

fn render_inline(index: usize, inline: Inline) -> Element {
    match inline {
        Inline::Break => rsx! { br { key: "{index}" } },
        Inline::Strong(text) => rsx! { strong { key: "{index}", "{text}" } },
        Inline::Code(text) => rsx! { code { key: "{index}", "{text}" } },
        Inline::Text(text) => rsx! { span { key: "{index}", "{text}" } },
    }
}
