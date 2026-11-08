use dioxus::prelude::*;

use crate::logic::layout::DrawerTab;

fn glyph(children: Element) -> Element {
    rsx! {
        svg {
            class: "tab-icon",
            width: "16",
            height: "16",
            view_box: "0 0 16 16",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.5",
            stroke_linecap: "square",
            stroke_linejoin: "miter",
            "aria-hidden": "true",
            "focusable": "false",
            {children}
        }
    }
}

#[component]
pub fn TabIcon(tab: DrawerTab) -> Element {
    match tab {
        DrawerTab::Files => glyph(rsx! { path { d: "M2.5 5H6L7.4 6.5H13.5V12.5H2.5V5Z" } }),
        DrawerTab::Lead => glyph(rsx! { path { d: "M2.5 3.5H13.5V9H8L6 12V9H2.5V3.5Z" } }),
        DrawerTab::Jobs => glyph(rsx! {
            line { x1: "2.5", y1: "4", x2: "4", y2: "4" }
            line { x1: "6", y1: "4", x2: "13.5", y2: "4" }
            line { x1: "2.5", y1: "8", x2: "4", y2: "8" }
            line { x1: "6", y1: "8", x2: "13.5", y2: "8" }
            line { x1: "2.5", y1: "12", x2: "4", y2: "12" }
            line { x1: "6", y1: "12", x2: "13.5", y2: "12" }
        }),
        DrawerTab::Changes => glyph(rsx! {
            line { x1: "3.5", y1: "6", x2: "12.5", y2: "6" }
            line { x1: "8", y1: "2.5", x2: "8", y2: "6" }
            line { x1: "3.5", y1: "10.5", x2: "12.5", y2: "10.5" }
        }),
        DrawerTab::Routines => glyph(rsx! {
            path { d: "M13.3 8a5.3 5.3 0 1 1-5.3-5.3c1.5 0 2.9.6 3.9 1.6L13.3 5.7" }
            polyline { points: "13.3 2.8 13.3 5.7 10.3 5.7" }
        }),
        DrawerTab::Usage => glyph(rsx! {
            path { d: "M3 12.2a5 5 0 0 1 10 0" }
            line { x1: "8", y1: "12.2", x2: "11.1", y2: "7.2" }
        }),
        DrawerTab::Activity => {
            glyph(rsx! { polyline { points: "1.5 8 4 8 6 3.5 8.5 12.5 11 8 14.5 8" } })
        }
        DrawerTab::History => glyph(rsx! {
            line { x1: "4", y1: "3.5", x2: "4", y2: "12.5" }
            line { x1: "4", y1: "6.5", x2: "11", y2: "6.5" }
            circle { cx: "4", cy: "3.5", r: "1.4", fill: "currentColor", stroke: "none" }
            circle { cx: "4", cy: "12.5", r: "1.4", fill: "currentColor", stroke: "none" }
            circle { cx: "12", cy: "6.5", r: "1.4", fill: "currentColor", stroke: "none" }
        }),
        DrawerTab::Goals => glyph(rsx! {
            circle { cx: "8", cy: "8", r: "5" }
            circle { cx: "8", cy: "8", r: "2" }
            line { x1: "8", y1: "1.5", x2: "8", y2: "3.2" }
            line { x1: "8", y1: "12.8", x2: "8", y2: "14.5" }
            line { x1: "1.5", y1: "8", x2: "3.2", y2: "8" }
            line { x1: "12.8", y1: "8", x2: "14.5", y2: "8" }
        }),
    }
}
