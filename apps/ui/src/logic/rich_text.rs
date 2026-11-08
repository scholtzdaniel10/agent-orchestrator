//! A small markdown subset for chat and job output. Port of `rich-text.ts`.
//!
//! The renderer draws these blocks as elements. It never injects the source as HTML.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(String),
    Code(String),
    Break,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Code(String),
}

pub fn parse_rich_text(text: &str) -> Vec<Block> {
    if text.is_empty() {
        return Vec::new();
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();

    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if let Some(block) = paragraph_from(paragraph) {
            blocks.push(block);
        }
        paragraph.clear();
    };

    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if is_fence(line) {
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            index += 1;
            while index < lines.len() && !is_fence(lines[index]) {
                code.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut paragraph, &mut blocks);
            index += 1;
            continue;
        }
        paragraph.push(line);
        index += 1;
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn is_fence(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("```") else {
        return false;
    };
    // A language tag may only use the TypeScript class `[\w.+#-]`, plus trailing spaces.
    let language = rest.trim_end_matches([' ', '\t']);
    language
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '+' | '#' | '-'))
}

fn paragraph_from(lines: &[&str]) -> Option<Block> {
    if lines.is_empty() {
        return None;
    }
    let mut inlines = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            inlines.push(Inline::Break);
        }
        inlines.extend(parse_line(line));
    }
    Some(Block::Paragraph(inlines))
}

fn parse_line(line: &str) -> Vec<Inline> {
    let chars: Vec<char> = line.chars().collect();
    let mut inlines = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '*' && chars.get(index + 1) == Some(&'*') {
            let close = chars[index + 2..]
                .windows(2)
                .position(|pair| pair == ['*', '*']);
            let Some(close) = close else {
                text.push_str("**");
                index += 2;
                continue;
            };
            flush_text(&mut text, &mut inlines);
            let start = index + 2;
            let end = start + close;
            inlines.push(Inline::Strong(chars[start..end].iter().collect()));
            index = end + 2;
            continue;
        }
        if chars[index] == '`' {
            let close = chars[index + 1..].iter().position(|ch| *ch == '`');
            let Some(close) = close else {
                text.push('`');
                index += 1;
                continue;
            };
            flush_text(&mut text, &mut inlines);
            let start = index + 1;
            let end = start + close;
            inlines.push(Inline::Code(chars[start..end].iter().collect()));
            index = end + 1;
            continue;
        }
        text.push(chars[index]);
        index += 1;
    }
    flush_text(&mut text, &mut inlines);
    inlines
}

fn flush_text(text: &mut String, inlines: &mut Vec<Inline>) {
    if text.is_empty() {
        return;
    }
    inlines.push(Inline::Text(std::mem::take(text)));
}

#[cfg(test)]
mod tests {
    use super::{Block, Inline, parse_rich_text};

    fn text(value: &str) -> Inline {
        Inline::Text(value.into())
    }

    fn paragraph(inlines: Vec<Inline>) -> Block {
        Block::Paragraph(inlines)
    }

    #[test]
    fn plain_text_is_one_paragraph() {
        assert_eq!(
            parse_rich_text("hello"),
            vec![paragraph(vec![text("hello")])]
        );
    }

    #[test]
    fn bold_becomes_a_strong_inline() {
        assert_eq!(
            parse_rich_text("say **hi** there"),
            vec![paragraph(vec![
                text("say "),
                Inline::Strong("hi".into()),
                text(" there"),
            ])]
        );
    }

    #[test]
    fn inline_code_becomes_a_code_inline() {
        assert_eq!(
            parse_rich_text("use `npm` here"),
            vec![paragraph(vec![
                text("use "),
                Inline::Code("npm".into()),
                text(" here"),
            ])]
        );
    }

    #[test]
    fn a_fenced_block_with_a_language_keeps_only_the_body() {
        assert_eq!(
            parse_rich_text("```ts\nconst n = 1\n```"),
            vec![Block::Code("const n = 1".into())]
        );
    }

    #[test]
    fn a_fenced_block_without_a_language_keeps_only_the_body() {
        assert_eq!(
            parse_rich_text("```\nconst n = 1\n```"),
            vec![Block::Code("const n = 1".into())]
        );
    }

    #[test]
    fn an_unclosed_fence_is_code_to_the_end() {
        assert_eq!(
            parse_rich_text("```js\nconst n = 1\nstill"),
            vec![Block::Code("const n = 1\nstill".into())]
        );
    }

    #[test]
    fn an_unclosed_bold_marker_stays_literal() {
        assert_eq!(
            parse_rich_text("**nope"),
            vec![paragraph(vec![text("**nope")])]
        );
    }

    #[test]
    fn script_markup_stays_plain_text() {
        assert_eq!(
            parse_rich_text("see <script>alert(1)</script> now"),
            vec![paragraph(vec![text("see <script>alert(1)</script> now")])]
        );
    }

    #[test]
    fn an_empty_string_has_no_blocks() {
        assert_eq!(parse_rich_text(""), Vec::<Block>::new());
    }

    #[test]
    fn blank_lines_split_paragraphs_and_single_newlines_stay_breaks() {
        assert_eq!(
            parse_rich_text("one\ntwo\n\nthree"),
            vec![
                paragraph(vec![text("one"), Inline::Break, text("two")]),
                paragraph(vec![text("three")]),
            ]
        );
    }

    #[test]
    fn asterisks_inside_inline_code_stay_literal() {
        assert_eq!(
            parse_rich_text("`**not bold**`"),
            vec![paragraph(vec![Inline::Code("**not bold**".into())])]
        );
    }
}
