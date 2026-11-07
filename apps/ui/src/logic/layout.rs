//! Window layout. Port of `layout.ts`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const LAYOUT_KEY: &str = "orch.layout";
pub const LAYOUT_FIT_EVENT: &str = "layoutfit";
pub const LAYOUT_SHORTCUT_EVENT: &str = "layoutshortcut";

pub const MIN_SIDEBAR: i64 = 160;
pub const MAX_SIDEBAR: i64 = 480;
pub const MIN_DRAWER: i64 = 320;
pub const MAX_DRAWER: i64 = 720;
pub const MIN_WORKERS: i64 = 360;
pub const HEADER_HEIGHT: i64 = 40;
pub const STATUS_HEIGHT: i64 = 28;
pub const MIN_DOCK: i64 = 120;
pub const MAX_DOCK: i64 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaneLayoutId {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2-side")]
    TwoSide,
    #[serde(rename = "2-stack")]
    TwoStack,
    #[serde(rename = "3-side")]
    ThreeSide,
    #[serde(rename = "4-grid")]
    FourGrid,
    #[serde(rename = "6-grid")]
    SixGrid,
}

impl PaneLayoutId {
    pub const ALL: [Self; 7] = [
        Self::Auto,
        Self::One,
        Self::TwoSide,
        Self::TwoStack,
        Self::ThreeSide,
        Self::FourGrid,
        Self::SixGrid,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::One => "1",
            Self::TwoSide => "2-side",
            Self::TwoStack => "2-stack",
            Self::ThreeSide => "3-side",
            Self::FourGrid => "4-grid",
            Self::SixGrid => "6-grid",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_str() == value)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::One => "1",
            Self::TwoSide => "2 side by side",
            Self::TwoStack => "2 stacked",
            Self::ThreeSide => "3 side by side",
            Self::FourGrid => "4 grid",
            Self::SixGrid => "6 grid",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawerTab {
    Files,
    Lead,
    Jobs,
    Changes,
    Routines,
    Usage,
    Activity,
    History,
    Goals,
}

impl DrawerTab {
    pub const ALL: [Self; 9] = [
        Self::Files,
        Self::Lead,
        Self::Jobs,
        Self::Changes,
        Self::Routines,
        Self::Usage,
        Self::Activity,
        Self::History,
        Self::Goals,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Lead => "lead",
            Self::Jobs => "jobs",
            Self::Changes => "changes",
            Self::Routines => "routines",
            Self::Usage => "usage",
            Self::Activity => "activity",
            Self::History => "history",
            Self::Goals => "goals",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Lead => "Lead",
            Self::Jobs => "Jobs",
            Self::Changes => "Changes",
            Self::Routines => "Routines",
            Self::Usage => "Usage",
            Self::Activity => "Activity",
            Self::History => "History",
            Self::Goals => "Goals",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutShortcut {
    Drawer,
    Sidebar,
    Tab(DrawerTab),
}

impl LayoutShortcut {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drawer => "drawer",
            Self::Sidebar => "sidebar",
            Self::Tab(tab) => tab.as_str(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSizes {
    pub sidebar: i64,
    pub drawer: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutFlags {
    pub sidebar_shown: bool,
    pub drawer_open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockSizes {
    pub top: i64,
    pub bottom: i64,
    pub left: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideDocks {
    pub top: Vec<DrawerTab>,
    pub bottom: Vec<DrawerTab>,
    pub left: Vec<DrawerTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockActive {
    pub top: Option<DrawerTab>,
    pub bottom: Option<DrawerTab>,
    pub left: Option<DrawerTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutState {
    pub sizes: LayoutSizes,
    pub sidebar_shown: bool,
    pub drawer_open: bool,
    pub drawer_tab: DrawerTab,
    pub pane_layout: PaneLayoutId,
    pub tab_order: Vec<DrawerTab>,
    pub dock_tabs: SideDocks,
    pub dock_active: DockActive,
    pub dock_sizes: DockSizes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Docks {
    pub right: Vec<DrawerTab>,
    pub top: Vec<DrawerTab>,
    pub bottom: Vec<DrawerTab>,
    pub left: Vec<DrawerTab>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockId {
    Right,
    Top,
    Bottom,
    Left,
}

impl DockId {
    pub const ALL: [Self; 4] = [Self::Right, Self::Top, Self::Bottom, Self::Left];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeKey {
    Sidebar,
    Drawer,
}

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

pub const DEFAULT_SIZES: LayoutSizes = LayoutSizes {
    sidebar: 240,
    drawer: 420,
};
pub const DEFAULT_DOCK_SIZES: DockSizes = DockSizes {
    top: 240,
    bottom: 260,
    left: 300,
};

pub fn default_layout() -> LayoutState {
    LayoutState {
        sizes: DEFAULT_SIZES,
        sidebar_shown: true,
        drawer_open: true,
        drawer_tab: DrawerTab::Files,
        pane_layout: PaneLayoutId::Auto,
        tab_order: DrawerTab::ALL.to_vec(),
        dock_tabs: SideDocks {
            top: Vec::new(),
            bottom: Vec::new(),
            left: Vec::new(),
        },
        dock_active: DockActive {
            top: None,
            bottom: None,
            left: None,
        },
        dock_sizes: DEFAULT_DOCK_SIZES,
    }
}

pub fn is_pane_layout(value: &str) -> bool {
    PaneLayoutId::parse(value).is_some()
}

pub fn is_drawer_tab(value: &str) -> bool {
    DrawerTab::parse(value).is_some()
}

pub fn pane_count(layout: PaneLayoutId, terminal_count: usize) -> usize {
    match resolved_pane_layout(layout, terminal_count) {
        PaneLayoutId::One => 1,
        PaneLayoutId::TwoSide | PaneLayoutId::TwoStack => 2,
        PaneLayoutId::ThreeSide => 3,
        PaneLayoutId::FourGrid => 4,
        PaneLayoutId::SixGrid => 6,
        PaneLayoutId::Auto => 1,
    }
}

pub fn auto_pane_layout(terminal_count: usize) -> PaneLayoutId {
    match terminal_count {
        0 | 1 => PaneLayoutId::One,
        2 => PaneLayoutId::TwoSide,
        3 => PaneLayoutId::ThreeSide,
        4 => PaneLayoutId::FourGrid,
        _ => PaneLayoutId::SixGrid,
    }
}

pub fn resolved_pane_layout(layout: PaneLayoutId, terminal_count: usize) -> PaneLayoutId {
    if layout == PaneLayoutId::Auto {
        auto_pane_layout(terminal_count)
    } else {
        layout
    }
}

pub fn parse_layout(raw: &Value) -> LayoutState {
    let Some(root) = raw.as_object() else {
        return default_layout();
    };
    let sizes_raw = root.get("sizes").and_then(Value::as_object);
    let sizes = LayoutSizes {
        sidebar: as_number(
            sizes_raw.and_then(|map| map.get("sidebar")),
            DEFAULT_SIZES.sidebar,
            MIN_SIDEBAR,
            MAX_SIDEBAR,
        ),
        drawer: as_number(
            sizes_raw.and_then(|map| map.get("drawer")),
            DEFAULT_SIZES.drawer,
            MIN_DRAWER,
            MAX_DRAWER,
        ),
    };
    let shown = root.get("shown").and_then(Value::as_object);
    let fallback = default_layout();
    let sidebar_shown = as_bool(
        root.get("sidebarShown"),
        as_bool(
            shown.and_then(|map| map.get("projects")),
            fallback.sidebar_shown,
        ),
    );
    LayoutState {
        sizes,
        sidebar_shown,
        drawer_open: as_bool(root.get("drawerOpen"), fallback.drawer_open),
        drawer_tab: root
            .get("drawerTab")
            .and_then(Value::as_str)
            .and_then(DrawerTab::parse)
            .unwrap_or(fallback.drawer_tab),
        pane_layout: root
            .get("paneLayout")
            .and_then(Value::as_str)
            .and_then(PaneLayoutId::parse)
            .unwrap_or(fallback.pane_layout),
        ..docks_layout(root)
    }
}

fn docks_layout(root: &Map<String, Value>) -> LayoutState {
    let raw_sides = root.get("dockTabs").and_then(Value::as_object);
    let mut packed = Map::new();
    if let Some(order) = root.get("tabOrder") {
        packed.insert("right".into(), order.clone());
    }
    for key in ["top", "bottom", "left"] {
        if let Some(value) = raw_sides.and_then(|map| map.get(key)) {
            packed.insert(key.into(), value.clone());
        }
    }
    let docks = docks_from(&Value::Object(packed));
    let raw_active = root.get("dockActive").and_then(Value::as_object);
    let active = |tabs: &[DrawerTab], key: &str| {
        let chosen = raw_active
            .and_then(|map| map.get(key))
            .and_then(Value::as_str)
            .and_then(DrawerTab::parse);
        match chosen {
            Some(tab) if tabs.contains(&tab) => Some(tab),
            _ => tabs.first().copied(),
        }
    };
    let raw_sizes = root.get("dockSizes").and_then(Value::as_object);
    let mut state = default_layout();
    state.tab_order = docks.right;
    state.dock_tabs = SideDocks {
        top: docks.top.clone(),
        bottom: docks.bottom.clone(),
        left: docks.left.clone(),
    };
    state.dock_active = DockActive {
        top: active(&docks.top, "top"),
        bottom: active(&docks.bottom, "bottom"),
        left: active(&docks.left, "left"),
    };
    state.dock_sizes = DockSizes {
        top: as_number(
            raw_sizes.and_then(|map| map.get("top")),
            DEFAULT_DOCK_SIZES.top,
            MIN_DOCK,
            MAX_DOCK,
        ),
        bottom: as_number(
            raw_sizes.and_then(|map| map.get("bottom")),
            DEFAULT_DOCK_SIZES.bottom,
            MIN_DOCK,
            MAX_DOCK,
        ),
        left: as_number(
            raw_sizes.and_then(|map| map.get("left")),
            DEFAULT_DOCK_SIZES.left,
            MIN_DOCK,
            MAX_DOCK,
        ),
    };
    state
}

pub fn docks_from(raw: &Value) -> Docks {
    let mut seen = Vec::new();
    let take = |value: Option<&Value>, seen: &mut Vec<DrawerTab>| {
        let mut out = Vec::new();
        let Some(entries) = value.and_then(Value::as_array) else {
            return out;
        };
        for entry in entries {
            let Some(tab) = entry.as_str().and_then(DrawerTab::parse) else {
                continue;
            };
            if seen.contains(&tab) {
                continue;
            }
            seen.push(tab);
            out.push(tab);
        }
        out
    };
    let top = take(raw.get("top"), &mut seen);
    let bottom = take(raw.get("bottom"), &mut seen);
    let left = take(raw.get("left"), &mut seen);
    let mut right = take(raw.get("right"), &mut seen);
    for tab in tab_order_from(raw.get("right")) {
        if !seen.contains(&tab) {
            seen.push(tab);
            right.push(tab);
        }
    }
    Docks {
        right,
        top,
        bottom,
        left,
    }
}

pub fn dock_of(docks: &Docks, tab: DrawerTab) -> DockId {
    for id in DockId::ALL {
        if docks.of(id).contains(&tab) {
            return id;
        }
    }
    DockId::Right
}

impl Docks {
    fn of(&self, id: DockId) -> &Vec<DrawerTab> {
        match id {
            DockId::Right => &self.right,
            DockId::Top => &self.top,
            DockId::Bottom => &self.bottom,
            DockId::Left => &self.left,
        }
    }

    fn of_mut(&mut self, id: DockId) -> &mut Vec<DrawerTab> {
        match id {
            DockId::Right => &mut self.right,
            DockId::Top => &mut self.top,
            DockId::Bottom => &mut self.bottom,
            DockId::Left => &mut self.left,
        }
    }
}

pub fn move_tab_to(docks: &Docks, tab: DrawerTab, dock: DockId, index: f64) -> Docks {
    let mut next = Docks {
        right: Vec::new(),
        top: Vec::new(),
        bottom: Vec::new(),
        left: Vec::new(),
    };
    for id in DockId::ALL {
        *next.of_mut(id) = docks
            .of(id)
            .iter()
            .copied()
            .filter(|entry| *entry != tab)
            .collect();
    }
    let moved = move_tab(next.of(dock), tab, index);
    *next.of_mut(dock) = moved;
    next
}

pub fn tabs_in_order(docks: &Docks) -> Vec<DrawerTab> {
    DockId::ALL
        .into_iter()
        .flat_map(|id| docks.of(id).iter().copied())
        .collect()
}

pub fn tab_order_from(raw: Option<&Value>) -> Vec<DrawerTab> {
    let mut out = Vec::new();
    if let Some(entries) = raw.and_then(Value::as_array) {
        for entry in entries {
            let Some(tab) = entry.as_str().and_then(DrawerTab::parse) else {
                continue;
            };
            if !out.contains(&tab) {
                out.push(tab);
            }
        }
    }
    for tab in DrawerTab::ALL {
        if !out.contains(&tab) {
            out.push(tab);
        }
    }
    out
}

pub fn move_tab(order: &[DrawerTab], tab: DrawerTab, index: f64) -> Vec<DrawerTab> {
    let rest: Vec<DrawerTab> = order
        .iter()
        .copied()
        .filter(|entry| *entry != tab)
        .collect();
    let at = round_i64(index).clamp(0, rest.len() as i64) as usize;
    let mut next = rest;
    next.insert(at, tab);
    next
}

pub fn is_default_tab_order(order: &[DrawerTab]) -> bool {
    order.len() == DrawerTab::ALL.len()
        && order
            .iter()
            .zip(DrawerTab::ALL)
            .all(|(tab, expected)| *tab == expected)
}

#[allow(clippy::result_unit_err)]
pub trait LayoutStore {
    fn get_item(&self, key: &str) -> Result<Option<String>, ()>;
    fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()>;
}

pub fn load_layout(storage: &dyn LayoutStore) -> LayoutState {
    let raw = match storage.get_item(LAYOUT_KEY) {
        Ok(Some(raw)) => raw,
        Ok(None) => return default_layout(),
        Err(()) => return default_layout(),
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(value) => parse_layout(&value),
        Err(_) => default_layout(),
    }
}

pub fn save_layout(state: &LayoutState, storage: &mut dyn LayoutStore) {
    let Ok(raw) = serde_json::to_string(state) else {
        return;
    };
    let _ = storage.set_item(LAYOUT_KEY, &raw);
}

pub fn drawer_overlays(_sizes: LayoutSizes, viewport: Viewport, flags: LayoutFlags) -> bool {
    if !flags.drawer_open {
        return false;
    }
    let width = round_i64(viewport.width).max(1);
    let sidebar_part = if flags.sidebar_shown { MIN_SIDEBAR } else { 0 };
    sidebar_part + MIN_DRAWER + MIN_WORKERS > width
}

fn fit_sidebar(sidebar: i64, width: i64) -> i64 {
    if width - sidebar >= MIN_WORKERS {
        sidebar
    } else {
        (width - MIN_WORKERS).max(MIN_SIDEBAR)
    }
}

pub fn clamp_sizes(sizes: LayoutSizes, viewport: Viewport, flags: LayoutFlags) -> LayoutSizes {
    let width = round_i64(viewport.width).max(1);
    let mut sidebar = clamp_num(
        sizes.sidebar,
        DEFAULT_SIZES.sidebar,
        MIN_SIDEBAR,
        MAX_SIDEBAR,
    );
    let mut drawer = clamp_num(sizes.drawer, DEFAULT_SIZES.drawer, MIN_DRAWER, MAX_DRAWER);
    if !flags.drawer_open {
        if flags.sidebar_shown {
            sidebar = fit_sidebar(sidebar, width);
        }
        return LayoutSizes { sidebar, drawer };
    }
    if drawer_overlays(
        LayoutSizes { sidebar, drawer },
        Viewport {
            width: f64::from(width as u32),
            height: viewport.height,
        },
        flags,
    ) {
        drawer = MIN_DRAWER;
        if flags.sidebar_shown {
            sidebar = fit_sidebar(sidebar, width);
        }
        return LayoutSizes { sidebar, drawer };
    }
    let sidebar_part = if flags.sidebar_shown { sidebar } else { 0 };
    let overflow = sidebar_part + drawer + MIN_WORKERS - width;
    if overflow > 0 {
        let next_drawer = (drawer - overflow).max(MIN_DRAWER);
        let drawer_gave = drawer - next_drawer;
        drawer = next_drawer;
        if flags.sidebar_shown && drawer_gave < overflow {
            sidebar = (sidebar - (overflow - drawer_gave)).max(MIN_SIDEBAR);
        }
    }
    LayoutSizes { sidebar, drawer }
}

pub fn max_size(key: SizeKey, sizes: LayoutSizes, viewport: Viewport, flags: LayoutFlags) -> i64 {
    if key == SizeKey::Drawer && flags.drawer_open && flags.sidebar_shown {
        let width = round_i64(viewport.width).max(1);
        let sidebar = clamp_num(
            sizes.sidebar,
            DEFAULT_SIZES.sidebar,
            MIN_SIDEBAR,
            MAX_SIDEBAR,
        );
        return clamp_num(
            width - sidebar - MIN_WORKERS,
            DEFAULT_SIZES.drawer,
            MIN_DRAWER,
            MAX_DRAWER,
        );
    }
    let mut blown = sizes;
    match key {
        SizeKey::Sidebar => blown.sidebar = 10_000,
        SizeKey::Drawer => blown.drawer = 10_000,
    }
    match key {
        SizeKey::Sidebar => clamp_sizes(blown, viewport, flags).sidebar,
        SizeKey::Drawer => clamp_sizes(blown, viewport, flags).drawer,
    }
}

pub fn empty_slots(layout: PaneLayoutId, terminal_count: usize) -> Vec<Option<String>> {
    vec![None; pane_count(layout, terminal_count)]
}

fn clamp_index(index: i64, length: usize) -> usize {
    if length == 0 {
        return 0;
    }
    index.clamp(0, length as i64 - 1) as usize
}

pub fn fill_panes(
    layout: PaneLayoutId,
    terminal_ids: &[String],
    active_id: Option<&str>,
) -> Vec<Option<String>> {
    let mut slots = empty_slots(layout, terminal_ids.len());
    if terminal_ids.is_empty() {
        return slots;
    }
    let start = active_id
        .and_then(|id| terminal_ids.iter().position(|item| item == id))
        .unwrap_or(0);
    let count = slots.len().min(terminal_ids.len());
    for offset in 0..count {
        slots[offset] = Some(terminal_ids[(start + offset) % terminal_ids.len()].clone());
    }
    slots
}

pub fn assign_tab_to_pane(
    slots: &[Option<String>],
    focused_index: i64,
    terminal_id: &str,
) -> Vec<Option<String>> {
    if slots.is_empty() {
        return Vec::new();
    }
    let mut next = slots.to_vec();
    let focused = clamp_index(focused_index, next.len());
    let existing = next
        .iter()
        .position(|id| id.as_deref() == Some(terminal_id));
    if existing == Some(focused) {
        return next;
    }
    if let Some(existing) = existing {
        next.swap(focused, existing);
        return next;
    }
    next[focused] = Some(terminal_id.to_string());
    next
}

pub fn empty_pane_for_terminal(slots: &[Option<String>], terminal_id: &str) -> Vec<Option<String>> {
    slots
        .iter()
        .map(|id| {
            if id.as_deref() == Some(terminal_id) {
                None
            } else {
                id.clone()
            }
        })
        .collect()
}

pub fn place_new_terminal(
    slots: &[Option<String>],
    focused_index: i64,
    terminal_id: &str,
) -> Vec<Option<String>> {
    if slots.is_empty() {
        return Vec::new();
    }
    let mut next = slots.to_vec();
    if let Some(empty) = next.iter().position(Option::is_none) {
        next[empty] = Some(terminal_id.to_string());
        return next;
    }
    let index = clamp_index(focused_index, next.len());
    next[index] = Some(terminal_id.to_string());
    next
}

pub fn place_terminal_in_pane(
    slots: &[Option<String>],
    pane_index: i64,
    terminal_id: &str,
) -> Vec<Option<String>> {
    if slots.is_empty() {
        return Vec::new();
    }
    let mut next = slots.to_vec();
    if let Some(existing) = next
        .iter()
        .position(|id| id.as_deref() == Some(terminal_id))
    {
        next[existing] = None;
    }
    let index = clamp_index(pane_index, next.len());
    next[index] = Some(terminal_id.to_string());
    next
}

pub fn drop_missing_terminals(
    slots: &[Option<String>],
    terminal_ids: &[String],
) -> Vec<Option<String>> {
    slots
        .iter()
        .map(|id| match id {
            Some(id) if !terminal_ids.iter().any(|known| known == id) => None,
            other => other.clone(),
        })
        .collect()
}

pub fn seat_unplaced(slots: &[Option<String>], terminal_ids: &[String]) -> Vec<Option<String>> {
    let mut next = slots.to_vec();
    for id in terminal_ids {
        if next.iter().any(|slot| slot.as_deref() == Some(id)) {
            continue;
        }
        let Some(empty) = next.iter().position(Option::is_none) else {
            break;
        };
        next = place_terminal_in_pane(&next, empty as i64, id);
    }
    next
}

pub fn request_layout_fit() {
    #[cfg(target_arch = "wasm32")]
    dispatch_window(LAYOUT_FIT_EVENT);
}

pub fn dispatch_layout_shortcut(kind: LayoutShortcut) {
    #[cfg(target_arch = "wasm32")]
    dispatch_window(kind.as_str());
    #[cfg(not(target_arch = "wasm32"))]
    let _ = kind;
}

#[derive(Clone, Copy)]
pub struct KeyPress<'a> {
    pub key: &'a str,
    pub code: Option<&'a str>,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub alt_key: bool,
    pub meta_key: bool,
}

pub fn layout_shortcut(event: KeyPress<'_>) -> Option<LayoutShortcut> {
    if !(event.ctrl_key && !event.shift_key && !event.alt_key && !event.meta_key) {
        return None;
    }
    if event.key == "b" || event.key == "B" {
        return Some(LayoutShortcut::Drawer);
    }
    if event.key == "\\" || event.key == "Backslash" || event.code == Some("Backslash") {
        return Some(LayoutShortcut::Sidebar);
    }
    let tab = match event.key {
        "1" => DrawerTab::Files,
        "2" => DrawerTab::Lead,
        "3" => DrawerTab::Jobs,
        "4" => DrawerTab::Changes,
        "5" => DrawerTab::Routines,
        "6" => DrawerTab::Usage,
        "7" => DrawerTab::Activity,
        "8" => DrawerTab::History,
        "9" => DrawerTab::Goals,
        _ => return None,
    };
    Some(LayoutShortcut::Tab(tab))
}

#[cfg(target_arch = "wasm32")]
fn dispatch_window(name: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(event) = web_sys::Event::new(name) else {
        return;
    };
    let _ = window.dispatch_event(&event);
}

fn as_number(value: Option<&Value>, fallback: i64, min: i64, max: i64) -> i64 {
    let Some(number) = value.and_then(Value::as_f64) else {
        return fallback;
    };
    if !number.is_finite() {
        return fallback;
    }
    round_i64(number).clamp(min, max)
}

fn as_bool(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

fn clamp_num(value: i64, _fallback: i64, min: i64, max: i64) -> i64 {
    value.clamp(min, max)
}

fn round_i64(value: f64) -> i64 {
    value.round() as i64
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SIZES, DrawerTab, KeyPress, LAYOUT_KEY, LayoutFlags, LayoutShortcut, LayoutSizes,
        LayoutState, LayoutStore, MIN_DRAWER, MIN_SIDEBAR, MIN_WORKERS, PaneLayoutId, SizeKey,
        Viewport, assign_tab_to_pane, auto_pane_layout, clamp_sizes, default_layout, dock_of,
        docks_from, drawer_overlays, drop_missing_terminals, empty_pane_for_terminal, empty_slots,
        fill_panes, is_default_tab_order, layout_shortcut, load_layout, max_size, move_tab,
        move_tab_to, pane_count, parse_layout, place_new_terminal, place_terminal_in_pane,
        resolved_pane_layout, save_layout, seat_unplaced, tab_order_from, tabs_in_order,
    };
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    struct Memory {
        data: BTreeMap<String, String>,
        fail_get: bool,
    }

    impl LayoutStore for Memory {
        fn get_item(&self, key: &str) -> Result<Option<String>, ()> {
            if self.fail_get {
                return Err(());
            }
            Ok(self.data.get(key).cloned())
        }
        fn set_item(&mut self, key: &str, value: &str) -> Result<(), ()> {
            self.data.insert(key.to_string(), value.to_string());
            Ok(())
        }
    }

    const OPEN: LayoutFlags = LayoutFlags {
        sidebar_shown: true,
        drawer_open: true,
    };
    fn wide() -> Viewport {
        Viewport {
            width: 1600.0,
            height: 900.0,
        }
    }
    fn view(width: f64) -> Viewport {
        Viewport {
            width,
            height: 800.0,
        }
    }
    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }
    fn slots(values: &[Option<&str>]) -> Vec<Option<String>> {
        values
            .iter()
            .map(|value| value.map(str::to_string))
            .collect()
    }

    #[test]
    fn parse_layout_returns_defaults_for_junk() {
        assert_eq!(parse_layout(&Value::Null), default_layout());
        assert_eq!(parse_layout(&Value::String("old".into())), default_layout());
        assert_eq!(parse_layout(&json!(1)), default_layout());
    }

    #[test]
    fn parse_layout_keeps_good_values() {
        let parsed = parse_layout(&json!({
            "sizes": { "sidebar": 260, "drawer": 500, "lead": 300, "extra": 1 },
            "sidebarShown": false,
            "drawerOpen": false,
            "drawerTab": "jobs",
            "paneLayout": "4-grid",
            "other": true
        }));
        assert_eq!(parsed.sizes.sidebar, 260);
        assert_eq!(parsed.sizes.drawer, 500);
        assert!(!parsed.sidebar_shown);
        assert!(!parsed.drawer_open);
        assert_eq!(parsed.drawer_tab, DrawerTab::Jobs);
        assert_eq!(parsed.pane_layout, PaneLayoutId::FourGrid);
    }

    #[test]
    fn parse_layout_falls_back_for_unknown_names() {
        assert_eq!(
            parse_layout(&json!({ "paneLayout": "split" })).pane_layout,
            PaneLayoutId::Auto
        );
        assert_eq!(
            parse_layout(&json!({ "drawerTab": "flow" })).drawer_tab,
            DrawerTab::Files
        );
        assert_eq!(
            parse_layout(&json!({ "paneLayout": "auto" })).pane_layout,
            PaneLayoutId::Auto
        );
        assert_eq!(
            parse_layout(&json!({ "paneLayout": "1" })).pane_layout,
            PaneLayoutId::One
        );
    }

    #[test]
    fn parse_layout_reads_old_saves() {
        let parsed = parse_layout(&json!({
            "sizes": { "sidebar": 180, "lead": 300, "usage": 220, "flow": 100 },
            "shown": { "projects": false, "flow": true, "lead": true, "usage": true },
            "paneLayout": "2-side",
            "terminalsMode": true
        }));
        assert_eq!(parsed.sizes.sidebar, 180);
        assert_eq!(parsed.sizes.drawer, DEFAULT_SIZES.drawer);
        assert!(!parsed.sidebar_shown);
        assert!(parsed.drawer_open);
        assert_eq!(parsed.drawer_tab, DrawerTab::Files);
        assert_eq!(parsed.pane_layout, PaneLayoutId::TwoSide);
    }

    #[test]
    fn parse_layout_clamps_sizes() {
        let parsed = parse_layout(&json!({ "sizes": { "sidebar": 10, "drawer": 10 } }));
        assert_eq!(parsed.sizes.sidebar, MIN_SIDEBAR);
        assert_eq!(parsed.sizes.drawer, MIN_DRAWER);
        let wide = parse_layout(&json!({ "sizes": { "sidebar": 900, "drawer": 2000 } }));
        assert_eq!(wide.sizes.sidebar, 480);
        assert_eq!(wide.sizes.drawer, 720);
    }

    #[test]
    fn load_layout_reads_json_and_falls_back() {
        let storage = Memory {
            fail_get: false,
            data: BTreeMap::from([(
                LAYOUT_KEY.into(),
                json!({
                    "sizes": { "sidebar": 180, "drawer": 400 },
                    "sidebarShown": true,
                    "drawerOpen": false,
                    "drawerTab": "usage",
                    "paneLayout": "2-side"
                })
                .to_string(),
            )]),
        };
        assert_eq!(load_layout(&storage).pane_layout, PaneLayoutId::TwoSide);
        assert!(!load_layout(&storage).drawer_open);
        assert_eq!(load_layout(&storage).drawer_tab, DrawerTab::Usage);
        let blocked = Memory {
            data: BTreeMap::new(),
            fail_get: true,
        };
        assert_eq!(load_layout(&blocked), default_layout());
        let bad = Memory {
            fail_get: false,
            data: BTreeMap::from([(LAYOUT_KEY.into(), "{bad".into())]),
        };
        assert_eq!(load_layout(&bad), default_layout());
    }

    #[test]
    fn save_layout_writes_one_json_object() {
        let mut storage = Memory {
            data: BTreeMap::new(),
            fail_get: false,
        };
        let mut state = default_layout();
        state.sizes = LayoutSizes {
            sidebar: 180,
            drawer: 400,
        };
        state.sidebar_shown = false;
        state.drawer_tab = DrawerTab::Activity;
        state.pane_layout = PaneLayoutId::SixGrid;
        state.tab_order = vec![
            DrawerTab::Activity,
            DrawerTab::Files,
            DrawerTab::Lead,
            DrawerTab::Jobs,
            DrawerTab::Changes,
            DrawerTab::Routines,
            DrawerTab::Usage,
        ];
        state.dock_tabs.top = vec![DrawerTab::History];
        state.dock_active.top = Some(DrawerTab::History);
        state.dock_sizes.top = 200;
        save_layout(&state, &mut storage);
        let written: Value = serde_json::from_str(storage.data.get(LAYOUT_KEY).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({
                "sizes": { "sidebar": 180, "drawer": 400 },
                "sidebarShown": false,
                "drawerOpen": true,
                "drawerTab": "activity",
                "paneLayout": "6-grid",
                "tabOrder": ["activity", "files", "lead", "jobs", "changes", "routines", "usage"],
                "dockTabs": { "top": ["history"], "bottom": [], "left": [] },
                "dockActive": { "top": "history", "bottom": null, "left": null },
                "dockSizes": { "top": 200, "bottom": 260, "left": 300 }
            })
        );
    }

    #[test]
    fn clamp_sizes_matches_the_viewport() {
        assert_eq!(clamp_sizes(DEFAULT_SIZES, wide(), OPEN), DEFAULT_SIZES);
        let next = clamp_sizes(DEFAULT_SIZES, view(900.0), OPEN);
        assert!(!drawer_overlays(next, view(900.0), OPEN));
        assert!(next.sidebar + next.drawer + MIN_WORKERS <= 900);
        assert_eq!(next.drawer, MIN_DRAWER);
        assert_eq!(next.sidebar, 220);
        let defaults = clamp_sizes(DEFAULT_SIZES, view(1100.0), OPEN);
        assert_eq!(defaults.sidebar, 240);
        assert_eq!(defaults.drawer, DEFAULT_SIZES.drawer);
        let shrunk = clamp_sizes(
            LayoutSizes {
                sidebar: 240,
                drawer: 600,
            },
            view(1100.0),
            OPEN,
        );
        assert_eq!(shrunk.sidebar, 240);
        assert_eq!(shrunk.drawer, 1100 - 240 - MIN_WORKERS);
        assert!(shrunk.drawer > MIN_DRAWER);
        let narrow = clamp_sizes(DEFAULT_SIZES, view(800.0), OPEN);
        assert!(drawer_overlays(narrow, view(800.0), OPEN));
        assert_eq!(narrow.drawer, MIN_DRAWER);
        assert!(narrow.sidebar >= MIN_SIDEBAR);
        assert!(800 - narrow.sidebar >= MIN_WORKERS);
    }

    #[test]
    fn drawer_overlays_when_the_minimums_do_not_fit() {
        assert!(!drawer_overlays(DEFAULT_SIZES, wide(), OPEN));
        assert!(!drawer_overlays(DEFAULT_SIZES, view(900.0), OPEN));
        assert!(!drawer_overlays(
            LayoutSizes {
                sidebar: 480,
                drawer: 720
            },
            view(900.0),
            OPEN
        ));
        assert!(!drawer_overlays(
            DEFAULT_SIZES,
            view(900.0),
            LayoutFlags {
                sidebar_shown: true,
                drawer_open: false
            }
        ));
        assert!(drawer_overlays(DEFAULT_SIZES, view(800.0), OPEN));
        let hidden = clamp_sizes(
            DEFAULT_SIZES,
            Viewport {
                width: 700.0,
                height: 800.0,
            },
            LayoutFlags {
                sidebar_shown: false,
                drawer_open: false,
            },
        );
        assert_eq!(hidden.sidebar, DEFAULT_SIZES.sidebar);
        assert_eq!(hidden.drawer, DEFAULT_SIZES.drawer);
    }

    #[test]
    fn max_size_is_what_clamp_would_allow() {
        let viewport = Viewport {
            width: 1600.0,
            height: 800.0,
        };
        let max = max_size(SizeKey::Sidebar, DEFAULT_SIZES, viewport, OPEN);
        let fitted = clamp_sizes(
            LayoutSizes {
                sidebar: 10_000,
                drawer: DEFAULT_SIZES.drawer,
            },
            viewport,
            OPEN,
        );
        assert_eq!(max, fitted.sidebar);
        assert!(max >= MIN_SIDEBAR);
        assert_eq!(
            max_size(SizeKey::Drawer, DEFAULT_SIZES, view(1200.0), OPEN),
            1200 - 240 - MIN_WORKERS
        );
    }

    #[test]
    fn panes_fill_from_the_active_terminal() {
        assert_eq!(
            fill_panes(
                PaneLayoutId::FourGrid,
                &ids(&["a", "b", "c", "d", "e"]),
                Some("c")
            ),
            slots(&[Some("c"), Some("d"), Some("e"), Some("a")])
        );
        assert_eq!(
            fill_panes(PaneLayoutId::SixGrid, &ids(&["a", "b"]), Some("b")),
            slots(&[Some("b"), Some("a"), None, None, None, None])
        );
        assert_eq!(
            fill_panes(PaneLayoutId::TwoSide, &[], None),
            slots(&[None, None])
        );
        assert_eq!(empty_slots(PaneLayoutId::One, 0), slots(&[None]));
        assert_eq!(pane_count(PaneLayoutId::SixGrid, 0), 6);
        assert_eq!(
            fill_panes(PaneLayoutId::Auto, &ids(&["a", "b"]), Some("a")),
            slots(&[Some("a"), Some("b")])
        );
        assert_eq!(
            fill_panes(
                PaneLayoutId::Auto,
                &ids(&["a", "b", "c", "d", "e"]),
                Some("a")
            ),
            slots(&[Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), None])
        );
        assert_eq!(empty_slots(PaneLayoutId::Auto, 0), slots(&[None]));
        assert_eq!(pane_count(PaneLayoutId::Auto, 5), 6);
    }

    #[test]
    fn auto_pane_layout_caps_at_six() {
        assert_eq!(auto_pane_layout(0), PaneLayoutId::One);
        assert_eq!(auto_pane_layout(1), PaneLayoutId::One);
        assert_eq!(auto_pane_layout(2), PaneLayoutId::TwoSide);
        assert_eq!(auto_pane_layout(3), PaneLayoutId::ThreeSide);
        assert_eq!(auto_pane_layout(4), PaneLayoutId::FourGrid);
        assert_eq!(auto_pane_layout(5), PaneLayoutId::SixGrid);
        assert_eq!(auto_pane_layout(6), PaneLayoutId::SixGrid);
        assert_eq!(auto_pane_layout(9), PaneLayoutId::SixGrid);
        assert_eq!(
            resolved_pane_layout(PaneLayoutId::TwoStack, 5),
            PaneLayoutId::TwoStack
        );
        assert_eq!(
            resolved_pane_layout(PaneLayoutId::Auto, 3),
            PaneLayoutId::ThreeSide
        );
    }

    #[test]
    fn shortcuts_map_ctrl_chords() {
        let base = KeyPress {
            key: "b",
            code: None,
            ctrl_key: true,
            shift_key: false,
            alt_key: false,
            meta_key: false,
        };
        assert_eq!(
            layout_shortcut(base).map(LayoutShortcut::as_str),
            Some("drawer")
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "B", ..base }).map(LayoutShortcut::as_str),
            Some("drawer")
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "\\", ..base }).map(LayoutShortcut::as_str),
            Some("sidebar")
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "1", ..base }).map(LayoutShortcut::as_str),
            Some("files")
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "2", ..base }).map(LayoutShortcut::as_str),
            Some("lead")
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "7", ..base }).map(LayoutShortcut::as_str),
            Some("activity")
        );
        assert_eq!(
            layout_shortcut(KeyPress {
                key: "b",
                shift_key: true,
                ..base
            })
            .map(LayoutShortcut::as_str),
            None
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "t", ..base }).map(LayoutShortcut::as_str),
            None
        );
        assert_eq!(
            layout_shortcut(KeyPress { key: "0", ..base }).map(LayoutShortcut::as_str),
            None
        );
    }

    #[test]
    fn pane_assignment() {
        assert_eq!(
            assign_tab_to_pane(&slots(&[Some("a"), Some("b"), None]), 0, "b"),
            slots(&[Some("b"), Some("a"), None])
        );
        assert_eq!(
            assign_tab_to_pane(&slots(&[Some("a"), None]), 1, "b"),
            slots(&[Some("a"), Some("b")])
        );
        assert_eq!(
            assign_tab_to_pane(&slots(&[Some("a"), Some("c")]), 1, "b"),
            slots(&[Some("a"), Some("b")])
        );
        assert_eq!(
            empty_pane_for_terminal(&slots(&[Some("a"), Some("b"), Some("a")]), "a"),
            slots(&[None, Some("b"), None])
        );
        assert_eq!(
            place_new_terminal(&slots(&[Some("a"), None, None]), 2, "b"),
            slots(&[Some("a"), Some("b"), None])
        );
        assert_eq!(
            place_new_terminal(&slots(&[Some("a"), Some("c")]), 0, "b"),
            slots(&[Some("b"), Some("c")])
        );
        assert_eq!(
            place_terminal_in_pane(&slots(&[None, None]), 1, "a"),
            slots(&[None, Some("a")])
        );
        assert_eq!(
            place_terminal_in_pane(&slots(&[Some("a"), None]), 1, "a"),
            slots(&[None, Some("a")])
        );
        assert_eq!(
            drop_missing_terminals(&slots(&[Some("a"), Some("b")]), &ids(&["b"])),
            slots(&[None, Some("b")])
        );
        assert_eq!(
            seat_unplaced(
                &slots(&[Some("a"), None, None, None]),
                &ids(&["a", "b", "c"])
            ),
            slots(&[Some("a"), Some("b"), Some("c"), None])
        );
        assert_eq!(
            seat_unplaced(&slots(&[None, Some("b")]), &ids(&["a", "b", "c"])),
            slots(&[Some("a"), Some("b")])
        );
        assert_eq!(
            seat_unplaced(&slots(&[Some("a")]), &ids(&["a", "b"])),
            slots(&[Some("a")])
        );
        assert_eq!(
            seat_unplaced(&slots(&[Some("a"), None]), &ids(&["a"])),
            slots(&[Some("a"), None])
        );
    }

    #[test]
    fn tab_order_and_docks() {
        assert_eq!(
            tab_order_from(Some(&json!(["activity", "nope", "files", "activity"]))),
            vec![
                DrawerTab::Activity,
                DrawerTab::Files,
                DrawerTab::Lead,
                DrawerTab::Jobs,
                DrawerTab::Changes,
                DrawerTab::Routines,
                DrawerTab::Usage,
                DrawerTab::History,
                DrawerTab::Goals,
            ]
        );
        assert_eq!(tab_order_from(None), DrawerTab::ALL.to_vec());
        assert!(is_default_tab_order(&tab_order_from(Some(&Value::Null))));
        let order = DrawerTab::ALL.to_vec();
        assert_eq!(
            move_tab(&order, DrawerTab::Activity, 0.0)[0],
            DrawerTab::Activity
        );
        assert_eq!(
            *move_tab(&order, DrawerTab::Files, 99.0).last().unwrap(),
            DrawerTab::Files
        );
        assert_eq!(
            move_tab(&order, DrawerTab::Jobs, 1.0),
            vec![
                DrawerTab::Files,
                DrawerTab::Jobs,
                DrawerTab::Lead,
                DrawerTab::Changes,
                DrawerTab::Routines,
                DrawerTab::Usage,
                DrawerTab::Activity,
                DrawerTab::History,
                DrawerTab::Goals,
            ]
        );
        assert_eq!(move_tab(&order, DrawerTab::Lead, -5.0)[0], DrawerTab::Lead);

        let docks = docks_from(&json!({
            "right": ["files", "lead"],
            "top": ["activity", "activity", "nope"],
            "left": ["lead", "history"]
        }));
        assert_eq!(docks.top, vec![DrawerTab::Activity]);
        assert_eq!(docks.left, vec![DrawerTab::Lead, DrawerTab::History]);
        assert!(docks.bottom.is_empty());
        assert_eq!(
            docks.right,
            vec![
                DrawerTab::Files,
                DrawerTab::Jobs,
                DrawerTab::Changes,
                DrawerTab::Routines,
                DrawerTab::Usage,
                DrawerTab::Goals
            ]
        );
        let mut ordered = tabs_in_order(&docks);
        let mut expected = DrawerTab::ALL.to_vec();
        ordered.sort_by_key(|tab| tab.as_str());
        expected.sort_by_key(|tab| tab.as_str());
        assert_eq!(ordered, expected);
        assert_eq!(dock_of(&docks, DrawerTab::History), super::DockId::Left);

        let start = docks_from(
            &json!({ "right": ["files", "lead", "jobs", "changes", "routines", "usage", "activity", "history", "goals"] }),
        );
        let moved = move_tab_to(&start, DrawerTab::Activity, super::DockId::Top, 0.0);
        assert_eq!(moved.top, vec![DrawerTab::Activity]);
        assert!(!moved.right.contains(&DrawerTab::Activity));
        let back = move_tab_to(&moved, DrawerTab::Activity, super::DockId::Right, 0.0);
        assert_eq!(back.right[0], DrawerTab::Activity);
        assert!(back.top.is_empty());

        let saved = parse_layout(&json!({
            "tabOrder": ["files"],
            "dockTabs": { "top": ["activity"], "bottom": ["history"], "left": [] },
            "dockActive": { "top": "activity", "bottom": "lead" },
            "dockSizes": { "top": 50, "bottom": 9999 }
        }));
        assert_eq!(saved.dock_tabs.top, vec![DrawerTab::Activity]);
        assert_eq!(saved.dock_active.top, Some(DrawerTab::Activity));
        assert_eq!(saved.dock_active.bottom, Some(DrawerTab::History));
        assert_eq!(saved.dock_active.left, None);
        assert_eq!(saved.dock_sizes.top, 120);
        assert_eq!(saved.dock_sizes.bottom, 900);
        let old = parse_layout(&json!({ "tabOrder": ["activity", "files"] }));
        assert_eq!(old.tab_order[0], DrawerTab::Activity);
        assert_eq!(old.tab_order.len(), DrawerTab::ALL.len());
        assert!(
            old.dock_tabs.top.is_empty()
                && old.dock_tabs.bottom.is_empty()
                && old.dock_tabs.left.is_empty()
        );
    }

    #[test]
    fn default_layout_is_a_value() {
        let _state: LayoutState = default_layout();
    }
}
