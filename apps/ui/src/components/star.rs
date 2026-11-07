use dioxus::prelude::*;

#[component]
pub fn Star(#[props(default = 12.0)] size: f64) -> Element {
    rsx! {
        svg {
            class: "star-mark",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 16 16",
            path { fill: "currentColor", d: "M8 0 9.1 6.9 16 8 9.1 9.1 8 16 6.9 9.1 0 8 6.9 6.9Z" }
        }
    }
}

#[component]
pub fn RayBurst() -> Element {
    rsx! { span { class: "ray-burst", aria_hidden: "true" } }
}
