use dioxus::prelude::*;
use legate_core::ProviderId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BotName {
    Lead,
    Plan(ProviderId),
}

impl BotName {
    pub fn class(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Plan(id) => id.as_str(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BotState {
    Idle,
    Working,
    Resting,
    Error,
}

#[component]
pub fn BotAvatar(bot: BotName, state: BotState, size: f64, title: Option<String>) -> Element {
    let mut class = format!("bot-avatar bot-{}", bot.class());
    if state == BotState::Working {
        class.push_str(" is-working");
    }
    if state == BotState::Resting {
        class.push_str(" is-resting");
    }
    let label = title.unwrap_or_default();
    let eye = if state == BotState::Resting {
        "12.1"
    } else {
        "11.7"
    };
    rsx! {
        svg {
            class: "{class}",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 24 24",
            role: if label.is_empty() { "presentation" } else { "img" },
            rect { class: "bot-body", x: "0", y: "0", width: "24", height: "24" }
            rect { class: "bot-eye", x: "9.5", y: "{eye}", width: "1.4", height: "1" }
            rect { class: "bot-eye", x: "13.3", y: "{eye}", width: "1.4", height: "1" }
            if state == BotState::Error {
                rect { class: "bot-error", x: "19", y: "0", width: "5", height: "5" }
            }
        }
    }
}
