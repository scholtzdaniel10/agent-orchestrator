pub(crate) fn fresh_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Split on `\n` or `\r\n`, keeping a trailing empty line the way `String.split` does.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find(['\n', '\r']) {
        out.push(&rest[..index]);
        if rest[index..].starts_with("\r\n") {
            rest = &rest[index + 2..];
        } else {
            rest = &rest[index + 1..];
        }
    }
    out.push(rest);
    out
}

pub(crate) fn first_line(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let line = split_lines(trimmed).into_iter().next().unwrap_or("");
    let count = line.chars().count();
    if count > max {
        let keep: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}\u{2026}")
    } else {
        line.to_string()
    }
}

/// Last `max` characters, with no ellipsis. `max` is counted in Unicode scalars.
pub(crate) fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

/// Last `max` characters, prefixed with `…` when the text was longer.
pub(crate) fn ellipsis_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count > max {
        format!("\u{2026}{}", tail_chars(text, max))
    } else {
        text.to_string()
    }
}

pub(crate) fn indent(text: &str) -> String {
    split_lines(text)
        .into_iter()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
