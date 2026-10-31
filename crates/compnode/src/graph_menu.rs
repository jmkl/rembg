//! Filterable "add node" menu used by right-clicking the canvas.
//!
//! The menu is rendered by `app` with `PopupCloseBehavior::CloseOnClickOutside`
//! so clicking the filter input does not dismiss it.
//!
//! Without a filter the menu mirrors Blender's context menus: each category is
//! a row that opens a submenu with its nodes. While filtering, all matching
//! nodes are listed flat so a search never hides results inside a category.
//!
//! Keyboard navigation follows the same model: Up/Down move the highlight,
//! Right opens a category, Left returns to the category list and Enter adds the
//! highlighted node (or opens the highlighted category). The filter field keeps
//! Left/Right for text editing until the keyboard highlight becomes active.

use egui::containers::menu::{MenuState, SubMenu};
use egui::{Align, Id, Key, Modifiers, Ui};
use egui_snarl::Snarl;

use crate::catalog::{self, NodeCategory, NodeSpec};
use crate::model::{Kind, Node};
use crate::style::controls::{MenuItem, muted};
use crate::style::{font, icons};

/// Keyboard highlight state for the add-node menu. It lives in `CompNodeApp`
/// so it survives between frames and resets when the menu closes.
#[derive(Default)]
pub(crate) struct MenuNav {
    /// `Some(category)` while a category submenu is the active level.
    level: Option<NodeCategory>,
    /// Highlighted row within the active level.
    index: usize,
    /// Only draw the keyboard highlight after the keyboard was used.
    active: bool,
    /// Query the highlight was built for; a change resets navigation.
    last_query: String,
    /// The `(level, index)` the view was last scrolled to. `scroll_to_me` must
    /// only run when the selection moves: repeating it every frame keeps the
    /// scroll animation (and its repaints) alive forever, which shows up as
    /// high CPU, navigation lag and a flickering list edge.
    scrolled_to: Option<(Option<NodeCategory>, usize)>,
    /// The filter field has been given keyboard focus for this menu opening.
    /// Cleared by `reset` when the menu closes, so every opening focuses the
    /// field once and the user can type immediately.
    filter_focused_once: bool,
}

impl MenuNav {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

enum RootRow {
    Recent(&'static NodeSpec),
    Category(usize),
}

pub(crate) fn show(
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    filter: &mut String,
    recent: &mut Vec<Kind>,
    pos: egui::Pos2,
    nav: &mut MenuNav,
) {
    ui.set_min_width(220.0);

    let specs = catalog::all_specs();
    let categories: Vec<(NodeCategory, Vec<&'static NodeSpec>)> = NodeCategory::ALL
        .into_iter()
        .filter_map(|category| {
            let rows: Vec<_> = specs
                .iter()
                .copied()
                .filter(|spec| !spec.hidden && spec.category() == category)
                .collect();
            (!rows.is_empty()).then_some((category, rows))
        })
        .collect();
    let recent_specs: Vec<&'static NodeSpec> = recent
        .iter()
        .copied()
        .map(catalog::spec)
        .filter(|spec| !spec.hidden)
        .collect();

    // Handle the state left over from the previous frame before consuming key
    // presses, so a changed query cannot leave the highlight pointing at a row
    // that no longer exists.
    let query_before = filter.trim().to_owned();
    if nav.last_query != query_before {
        reset_level(ui, nav);
        nav.last_query = query_before.clone();
    }
    if let Some(category) = nav.level
        && !categories.iter().any(|(known, _)| *known == category)
    {
        nav.level = None;
        nav.index = 0;
    }

    let filter_id = ui.make_persistent_id("graph_node_filter");
    let filter_focused = ui.memory(|memory| memory.has_focus(filter_id));

    // Once the pointer moves, prefer hover highlighting over the keyboard
    // highlight so the two do not compete.
    if ui.input(|input| input.pointer.delta().length_sq() > 0.0) {
        nav.active = false;
    }

    let root_rows_before = if query_before.is_empty() {
        root_rows(&recent_specs, categories.len())
    } else {
        Vec::new()
    };
    let current_len = match nav.level {
        Some(category) => category_rows(&categories, category)
            .map(|rows| rows.len())
            .unwrap_or(0),
        None if query_before.is_empty() => root_rows_before.len(),
        None => matching_specs(&specs, &query_before).len(),
    };

    let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    // Left/Right edit the filter text while it is focused, unless the keyboard
    // highlight is active: then they open and leave submenus like in Blender.
    let (left, right) = if filter_focused && !nav.active {
        (false, false)
    } else {
        (
            ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowLeft)),
            ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowRight)),
        )
    };

    if down {
        if !nav.active {
            nav.active = true;
            nav.index = 0;
        } else if current_len > 0 {
            nav.index = (nav.index + 1).min(current_len - 1);
        }
    }
    if up {
        if !nav.active {
            nav.active = true;
            nav.index = current_len.saturating_sub(1);
        } else {
            nav.index = nav.index.saturating_sub(1);
        }
    }

    if right
        && nav.level.is_none()
        && query_before.is_empty()
        && let Some(RootRow::Category(category)) = root_rows_before.get(nav.index)
    {
        nav.active = true;
        nav.level = Some(categories[*category].0);
        nav.index = 0;
    }

    if left && let Some(category) = nav.level {
        nav.active = true;
        nav.level = None;
        nav.index = root_rows_before
            .iter()
            .position(
                |row| matches!(row, RootRow::Category(index) if categories[*index].0 == category),
            )
            .unwrap_or(0);
        MenuState::from_ui(ui, |state, _| state.open_item = None);
    }

    let mut enter_node = false;
    if enter {
        nav.active = true;
        match nav.level {
            Some(_) => enter_node = true,
            None if query_before.is_empty() => match root_rows_before.get(nav.index) {
                Some(RootRow::Recent(_)) => enter_node = true,
                Some(RootRow::Category(category)) => {
                    nav.level = Some(categories[*category].0);
                    nav.index = 0;
                }
                None => {}
            },
            None => enter_node = true,
        }
    }

    let filter_response = ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Filter nodes…")
            .desired_width(220.0)
            .id(filter_id),
    );
    if !nav.filter_focused_once {
        nav.filter_focused_once = true;
        filter_response.request_focus();
    }
    ui.add_space(3.0);

    let query = filter.trim();
    if nav.last_query != query {
        reset_level(ui, nav);
        nav.last_query = query.to_owned();
        // The rows the key press referred to may have just disappeared.
        enter_node = false;
    }

    let query_empty = query.is_empty();
    let root_rows = if query_empty {
        root_rows(&recent_specs, categories.len())
    } else {
        Vec::new()
    };
    let matches = if query_empty {
        Vec::new()
    } else {
        matching_specs(&specs, query)
    };

    let current_len = match nav.level {
        Some(category) => category_rows(&categories, category)
            .map(|rows| rows.len())
            .unwrap_or(0),
        None if query_empty => root_rows.len(),
        None => matches.len(),
    };
    if current_len == 0 {
        nav.index = 0;
    } else {
        nav.index = nav.index.min(current_len - 1);
    }

    let mut clicked = false;
    let mut open_categories: Vec<(Id, NodeCategory)> = Vec::new();

    if query_empty {
        let recent_count = recent_specs.len();
        // The root list scrolls too, for the same reason as the node lists:
        // long category lists must stay on screen, and a scroll target set by
        // a root row must be consumed here instead of leaking into whatever
        // ScrollArea happens to run later in the same frame (e.g. a submenu's).
        egui::ScrollArea::vertical()
            .max_height(menu_max_height(ui))
            .show(ui, |ui| {
                for (index, row) in root_rows.iter().enumerate() {
                    if clicked {
                        break;
                    }
                    if index == 0 && recent_count > 0 {
                        ui.label(muted("Recently used", font::SMALL));
                    }
                    if index == recent_count && recent_count > 0 {
                        ui.separator();
                    }
                    match *row {
                        RootRow::Recent(spec) => {
                            let selected =
                                nav.active && nav.level.is_none() && nav.index == index;
                            let response = ui.add(
                                MenuItem::new(spec.title)
                                    .icon(spec.icon)
                                    .highlighted(selected),
                            );
                            scroll_selected(nav, selected, &response);
                            if response.clicked() || (selected && enter_node) {
                                create_node(spec, snarl, pos, recent);
                                clicked = true;
                            }
                        }
                        RootRow::Category(category) => {
                            let (category, category_specs) = &categories[category];
                            let selected =
                                nav.active && nav.level.is_none() && nav.index == index;
                            let row_id = ui.next_auto_id();
                            let open = MenuState::from_ui(ui, |state, _| {
                                state.open_item == Some(SubMenu::id_from_widget_id(row_id))
                            });
                            let response = ui.add(
                                MenuItem::new(category.label())
                                    .icon(icons::FOLDER_OPEN)
                                    .trailing_icon(icons::CHEVRON_RIGHT)
                                    .highlighted(open || selected),
                            );
                            scroll_selected(nav, selected, &response);

                            let submenu_id = SubMenu::id_from_widget_id(response.id);
                            open_categories.push((submenu_id, *category));
                            if nav.active && nav.level == Some(*category) {
                                // `MenuState::from_ui` drops an open item whose
                                // submenu was not shown in the previous frame. A
                                // keyboard-open request happens before
                                // `SubMenu::show` runs, so mark it as shown first
                                // or the request is cancelled.
                                MenuState::mark_shown(ui.ctx(), submenu_id);
                                MenuState::from_ui(ui, |state, _| {
                                    state.open_item = Some(submenu_id)
                                });
                            }
                            let sub_selected =
                                (nav.active && nav.level == Some(*category)).then_some(nav.index);

                            SubMenu::new().show(ui, &response, |ui| {
                                ui.set_min_width(190.0);
                                clicked = node_list(
                                    ui,
                                    category_specs.iter().copied(),
                                    snarl,
                                    pos,
                                    recent,
                                    nav,
                                    sub_selected,
                                    enter_node,
                                );
                            });
                        }
                    }
                }
            });
    } else if matches.is_empty() {
        ui.add(egui::Label::new(muted("No matching nodes", font::SMALL)).selectable(false));
    } else {
        let selected = (nav.active && nav.level.is_none()).then_some(nav.index);
        clicked = node_list(
            ui,
            matches.iter().copied(),
            snarl,
            pos,
            recent,
            nav,
            selected,
            enter_node,
        );
    }

    // Keep the navigation level in sync when a submenu is opened or closed by
    // the pointer, so the arrow keys continue from whatever is on screen.
    let open_item = MenuState::from_ui(ui, |state, _| state.open_item);
    if let Some((_, category)) = open_categories
        .iter()
        .find(|(id, _)| Some(*id) == open_item)
    {
        if nav.level != Some(*category) {
            nav.level = Some(*category);
            nav.index = 0;
        }
    } else if nav.level.is_some() && !nav.active {
        nav.level = None;
    }

    if clicked {
        // Close the context menu from its own UI. Closing from a submenu only
        // closes that submenu, so this is done on the root menu's `Ui`.
        ui.close();
    }
}

fn reset_level(ui: &Ui, nav: &mut MenuNav) {
    nav.level = None;
    nav.index = 0;
    nav.active = false;
    MenuState::from_ui(ui, |state, _| state.open_item = None);
}

fn root_rows(recent: &[&'static NodeSpec], category_count: usize) -> Vec<RootRow> {
    recent
        .iter()
        .copied()
        .map(RootRow::Recent)
        .chain((0..category_count).map(RootRow::Category))
        .collect()
}

fn category_rows<'a>(
    categories: &'a [(NodeCategory, Vec<&'static NodeSpec>)],
    category: NodeCategory,
) -> Option<&'a [&'static NodeSpec]> {
    categories
        .iter()
        .find(|(known, _)| *known == category)
        .map(|(_, rows)| rows.as_slice())
}

fn matching_specs(specs: &[&'static NodeSpec], query: &str) -> Vec<&'static NodeSpec> {
    let mut matches: Vec<_> = specs
        .iter()
        .copied()
        .filter(|spec| !spec.hidden)
        .filter_map(|spec| spec.match_score(query).map(|score| (score, spec)))
        .collect();
    matches.sort_by_key(|(score, spec)| (*score, spec.title));
    matches.into_iter().map(|(_, spec)| spec).collect()
}

/// Long menus scroll instead of covering the screen.
fn menu_max_height(ui: &Ui) -> f32 {
    (ui.ctx().content_rect().height() * 0.55).clamp(160.0, 440.0)
}

/// Scroll the highlighted row into view — but only when the selection moved
/// to a new row. Asking to scroll the same row every frame restarts the
/// scroll animation indefinitely; rows that cannot be perfectly centred
/// (list edges) then oscillate against the scroll clamp, so egui repaints
/// the whole app every frame: high CPU, navigation lag and a flickering
/// list edge.
fn scroll_selected(nav: &mut MenuNav, selected: bool, response: &egui::Response) {
    if selected && nav.scrolled_to != Some((nav.level, nav.index)) {
        response.scroll_to_me(Some(Align::Center));
        nav.scrolled_to = Some((nav.level, nav.index));
    }
}

/// Show rows in a scrollable list, keeping long categories and search results
/// inside the screen. `selected` is the keyboard highlight (if any).
fn node_list(
    ui: &mut Ui,
    specs: impl IntoIterator<Item = &'static NodeSpec>,
    snarl: &mut Snarl<Node>,
    pos: egui::Pos2,
    recent: &mut Vec<Kind>,
    nav: &mut MenuNav,
    selected: Option<usize>,
    enter: bool,
) -> bool {
    let max_height = menu_max_height(ui);
    let mut clicked = false;
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            for (index, spec) in specs.into_iter().enumerate() {
                let is_selected = selected == Some(index);
                let response = ui.add(
                    MenuItem::new(spec.title)
                        .icon(spec.icon)
                        .highlighted(is_selected),
                );
                scroll_selected(nav, is_selected, &response);
                if response.clicked() || (is_selected && enter) {
                    create_node(spec, snarl, pos, recent);
                    clicked = true;
                    break;
                }
            }
        });
    clicked
}

fn create_node(
    spec: &'static NodeSpec,
    snarl: &mut Snarl<Node>,
    pos: egui::Pos2,
    recent: &mut Vec<Kind>,
) {
    (spec.create)(snarl, pos, spec.kind);
    recent.retain(|kind| *kind != spec.kind);
    recent.insert(0, spec.kind);
    recent.truncate(5);
}
