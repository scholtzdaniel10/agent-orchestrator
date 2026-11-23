//! One-click layouts for the Layout menu. Each preset puts every panel in one place; moving a
//! single panel is a drag from the command rail or a panel's Move menu.

use super::layout::{DockActive, DrawerTab, LayoutState, SideDocks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    /// Panels in the right inspector, projects on the left.
    Classic,
    /// Panels under the projects list, the terminals get the full width.
    Side,
    /// Panels below the terminals.
    Bottom,
    /// Terminals only: no projects list, no panel.
    Focus,
}

impl LayoutPreset {
    pub const ALL: [Self; 4] = [Self::Classic, Self::Side, Self::Bottom, Self::Focus];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Side => "side",
            Self::Bottom => "bottom",
            Self::Focus => "focus",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Side => "Side",
            Self::Bottom => "Bottom",
            Self::Focus => "Focus",
        }
    }

    pub const fn hint(self) -> &'static str {
        match self {
            Self::Classic => "Panels on the right",
            Self::Side => "Panels under projects",
            Self::Bottom => "Panels below terminals",
            Self::Focus => "Terminals only",
        }
    }
}

/// Every panel once, in the order the user sees them now (right, then the side docks), with
/// any panel missing from the saved layout at the end.
fn every_tab(state: &LayoutState) -> Vec<DrawerTab> {
    let mut out: Vec<DrawerTab> = Vec::new();
    let saved = state
        .tab_order
        .iter()
        .chain(&state.dock_tabs.top)
        .chain(&state.dock_tabs.bottom)
        .chain(&state.dock_tabs.left)
        .copied()
        .chain(DrawerTab::ALL);
    for tab in saved {
        if !out.contains(&tab) {
            out.push(tab);
        }
    }
    out
}

/// The panel to show after the change: the one the inspector last showed.
fn shown_tab(state: &LayoutState) -> DrawerTab {
    state.drawer_tab
}

pub fn apply_preset(state: &LayoutState, preset: LayoutPreset) -> LayoutState {
    let tabs = every_tab(state);
    let shown = shown_tab(state);
    let mut next = state.clone();
    next.tab_order = tabs.clone();
    next.dock_tabs = SideDocks {
        top: Vec::new(),
        bottom: Vec::new(),
        left: Vec::new(),
    };
    next.dock_active = DockActive {
        top: None,
        bottom: None,
        left: None,
    };
    next.drawer_tab = shown;
    match preset {
        LayoutPreset::Classic => {
            next.sidebar_shown = true;
            next.drawer_open = true;
        }
        LayoutPreset::Side => {
            next.sidebar_shown = true;
            next.drawer_open = false;
            next.dock_tabs.left = tabs;
            next.dock_active.left = Some(shown);
        }
        LayoutPreset::Bottom => {
            next.sidebar_shown = true;
            next.drawer_open = false;
            next.dock_tabs.bottom = tabs;
            next.dock_active.bottom = Some(shown);
        }
        LayoutPreset::Focus => {
            next.sidebar_shown = false;
            next.drawer_open = false;
        }
    }
    next
}

/// Which preset the layout matches, if any, so the menu can mark it.
pub fn preset_of(state: &LayoutState) -> Option<LayoutPreset> {
    let docks = &state.dock_tabs;
    let all = |list: &[DrawerTab]| DrawerTab::ALL.iter().all(|tab| list.contains(tab));
    let no_side = docks.top.is_empty() && docks.bottom.is_empty() && docks.left.is_empty();
    if no_side {
        return match (state.sidebar_shown, state.drawer_open) {
            (true, true) => Some(LayoutPreset::Classic),
            (false, false) => Some(LayoutPreset::Focus),
            _ => None,
        };
    }
    if all(&docks.left) && docks.top.is_empty() && docks.bottom.is_empty() && state.sidebar_shown {
        return Some(LayoutPreset::Side);
    }
    if all(&docks.bottom) && docks.top.is_empty() && docks.left.is_empty() && state.sidebar_shown {
        return Some(LayoutPreset::Bottom);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::layout::default_layout;

    #[test]
    fn the_default_layout_is_classic() {
        assert_eq!(preset_of(&default_layout()), Some(LayoutPreset::Classic));
    }

    #[test]
    fn each_preset_is_recognised_after_applying_it() {
        let start = default_layout();
        for preset in LayoutPreset::ALL {
            assert_eq!(
                preset_of(&apply_preset(&start, preset)),
                Some(preset),
                "{preset:?}"
            );
        }
    }

    #[test]
    fn side_and_bottom_hold_every_panel_and_show_the_last_one() {
        let mut start = default_layout();
        start.drawer_tab = DrawerTab::Goals;
        let side = apply_preset(&start, LayoutPreset::Side);
        assert_eq!(side.dock_tabs.left.len(), DrawerTab::ALL.len());
        assert_eq!(side.dock_active.left, Some(DrawerTab::Goals));
        assert!(!side.drawer_open);
        let bottom = apply_preset(&start, LayoutPreset::Bottom);
        assert_eq!(bottom.dock_tabs.bottom.len(), DrawerTab::ALL.len());
        assert_eq!(bottom.dock_active.bottom, Some(DrawerTab::Goals));
        assert!(bottom.dock_tabs.left.is_empty());
    }

    #[test]
    fn going_back_to_classic_keeps_the_users_tab_order() {
        let mut start = default_layout();
        start.tab_order = vec![DrawerTab::Goals, DrawerTab::Files];
        start.dock_tabs.bottom = vec![DrawerTab::Jobs];
        let classic = apply_preset(&start, LayoutPreset::Classic);
        assert_eq!(
            &classic.tab_order[..3],
            &[DrawerTab::Goals, DrawerTab::Files, DrawerTab::Jobs]
        );
        assert_eq!(classic.tab_order.len(), DrawerTab::ALL.len());
        assert!(classic.dock_tabs.bottom.is_empty());
        assert!(classic.drawer_open && classic.sidebar_shown);
    }

    #[test]
    fn focus_hides_projects_and_the_panel() {
        let focus = apply_preset(&default_layout(), LayoutPreset::Focus);
        assert!(!focus.sidebar_shown && !focus.drawer_open);
    }

    #[test]
    fn a_hand_made_layout_matches_no_preset() {
        let mut state = default_layout();
        state.dock_tabs.bottom = vec![DrawerTab::Jobs];
        assert_eq!(preset_of(&state), None);
    }
}
