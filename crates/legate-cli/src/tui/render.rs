//! Ratatui drawing of the screen model. Colour mode matches the Ink screen:
//! headings are upper case and the selection mark is ▌.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

use super::state::TuiState;
use super::view::{LineTone, RowModel, ScreenModel, screen_model};

const ACCENT: Color = Color::Rgb(224, 52, 60);
const QUIET: Color = Color::DarkGray;

pub fn draw(frame: &mut Frame<'_>, state: &TuiState) {
    let model = screen_model(state);
    let area = frame.area();
    let error_row = u16::from(model.error.is_some());
    let body_top = 1u16;
    let footer_y = area.height.saturating_sub(1);
    let prompt_y = footer_y.saturating_sub(1 + error_row);
    let error_y = if error_row == 1 {
        footer_y.saturating_sub(1)
    } else {
        0
    };
    let body_height = prompt_y.saturating_sub(body_top);
    put(frame, 0, 0, &heading_top(&model), Style::default());
    let left = model.left.min(area.width.saturating_sub(1));
    let body = Rect {
        x: 0,
        y: body_top,
        width: area.width,
        height: body_height,
    };
    let left_rect = Rect {
        x: body.x,
        y: body.y,
        width: left,
        height: body.height,
    };
    let right_rect = Rect {
        x: body.x + left,
        y: body.y,
        width: body.width.saturating_sub(left),
        height: body.height,
    };
    let trees_h = model.worktrees_height.min(left_rect.height);
    let jobs_h = left_rect.height.saturating_sub(trees_h);
    draw_box(
        frame,
        Rect {
            x: left_rect.x,
            y: left_rect.y,
            width: left_rect.width,
            height: trees_h,
        },
        &paint_heading(&model.worktrees_title, model.color),
        model.focus == super::state::Focus::Worktrees,
        model.color,
        &model.worktrees,
        None,
    );
    draw_box(
        frame,
        Rect {
            x: left_rect.x,
            y: left_rect.y + trees_h,
            width: left_rect.width,
            height: jobs_h,
        },
        &paint_heading(&model.jobs_title, model.color),
        model.focus == super::state::Focus::Jobs,
        model.color,
        &model.jobs,
        if model.jobs.is_empty() {
            Some(" none yet")
        } else {
            None
        },
    );
    draw_detail(frame, right_rect, &model);
    let prompt = prompt_line(&model, &state.draft);
    put(frame, 0, prompt_y, &prompt, Style::default());
    if let Some(error) = &model.error {
        let style = if model.color {
            Style::default().fg(ACCENT)
        } else {
            Style::default()
        };
        put(frame, 0, error_y, error, style);
    }
    put(frame, 0, footer_y, &model.footer, Style::default());
}

#[allow(dead_code)]
pub fn frame_text(state: &TuiState) -> String {
    let backend = ratatui::backend::TestBackend::new(state.columns, state.rows);
    let mut terminal = ratatui::Terminal::new(backend).expect("test backend");
    terminal.draw(|frame| draw(frame, state)).expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            line.push_str(buffer[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

fn paint_heading(text: &str, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    match text.split_once("  ") {
        Some((word, rest)) => format!("{}  {rest}", word.to_uppercase()),
        None => text.to_uppercase(),
    }
}

fn row_symbols(row: &RowModel, color: bool) -> String {
    if color && row.selected {
        let rest: String = row.text.chars().skip(2).collect();
        format!("▌ {rest}")
    } else {
        row.text.clone()
    }
}

fn heading_top(model: &ScreenModel) -> String {
    model.top.clone()
}

fn prompt_line(model: &ScreenModel, draft: &str) -> String {
    if !model.confirm.is_empty() {
        return model.confirm.clone();
    }
    let mark = if model.color { "› " } else { "> " };
    let body = if model.editing {
        if draft.is_empty() {
            model.placeholder.clone()
        } else {
            draft.to_string()
        }
    } else {
        model.placeholder.clone()
    };
    if model.aside.is_empty() {
        format!("{mark}{body}")
    } else {
        format!("{mark}{body}  {}", model.aside)
    }
}

fn draw_box(
    frame: &mut Frame<'_>,
    rect: Rect,
    title: &str,
    focused: bool,
    color: bool,
    rows: &[RowModel],
    empty: Option<&str>,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let border = if color && focused {
        Style::default().fg(ACCENT)
    } else if color {
        Style::default().fg(QUIET)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }
    put(
        frame,
        inner.x,
        inner.y,
        title,
        Style::default().add_modifier(Modifier::BOLD),
    );
    if let Some(empty) = empty
        && inner.height > 1
    {
        put(
            frame,
            inner.x,
            inner.y + 1,
            empty,
            Style::default().fg(QUIET),
        );
    }
    for (index, row) in rows.iter().enumerate() {
        let y = inner.y + 1 + index as u16;
        if y >= inner.y + inner.height {
            break;
        }
        let style = if row.focused && !color {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if color && row.focused {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        put(frame, inner.x, y, &row_symbols(row, color), style);
    }
}

fn draw_detail(frame: &mut Frame<'_>, rect: Rect, model: &ScreenModel) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let focused = model.focus == super::state::Focus::Detail;
    let border = if model.color && focused {
        Style::default().fg(ACCENT)
    } else if model.color {
        Style::default().fg(QUIET)
    } else {
        Style::default()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height == 0 {
        return;
    }
    put(
        frame,
        inner.x,
        inner.y,
        &model.title,
        Style::default().add_modifier(Modifier::BOLD),
    );
    for (index, line) in model.detail.iter().enumerate() {
        let y = inner.y + 1 + index as u16;
        if y >= inner.y + inner.height {
            break;
        }
        put(
            frame,
            inner.x,
            y,
            &line.text,
            tone_style(line.tone, model.color),
        );
    }
}

fn tone_style(tone: LineTone, color: bool) -> Style {
    if !color {
        return Style::default();
    }
    match tone {
        LineTone::Dim => Style::default().fg(QUIET),
        LineTone::Add => Style::default().fg(Color::Green),
        LineTone::Remove | LineTone::Error | LineTone::Accent => Style::default().fg(ACCENT),
        LineTone::Plain => Style::default(),
    }
}

fn put(frame: &mut Frame<'_>, x: u16, y: u16, text: &str, style: Style) {
    let area = frame.area();
    if y >= area.height || x >= area.width {
        return;
    }
    let width = (area.width - x) as usize;
    frame.buffer_mut().set_stringn(x, y, text, width, style);
}
