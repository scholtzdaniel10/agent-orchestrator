use dioxus::prelude::*;

/// The React `ErrorBoundary`: a slot around children, and the same alert if that part throws.
#[component]
pub fn ErrorBoundary(class_name: Option<String>, children: Element) -> Element {
    let extra = class_name.unwrap_or_default();
    let slot = join_class("error-boundary-slot", &extra);
    let fail = join_class("part-error", &extra);
    let mut nonce = use_signal(|| 0i64);
    rsx! {
        dioxus::prelude::ErrorBoundary {
            handle_error: move |errors: dioxus::prelude::ErrorContext| {
                let message = errors
                    .error()
                    .map(|error| error.to_string())
                    .unwrap_or_default();
                let fail = fail.clone();
                rsx! {
                    div { class: "{fail}", role: "alert",
                        p { class: "part-error-title", "This part of the window hit an error." }
                        pre { class: "part-error-message", "{message}" }
                        button {
                            class: "btn btn-quiet",
                            r#type: "button",
                            onclick: move |_| {
                                errors.clear_errors();
                                nonce.set(nonce() + 1);
                            },
                            "Reload this part"
                        }
                    }
                }
            },
            div { key: "{nonce}", class: "{slot}", {children} }
        }
    }
}

fn join_class(base: &str, extra: &str) -> String {
    if extra.is_empty() {
        base.to_string()
    } else {
        format!("{base} {extra}")
    }
}
