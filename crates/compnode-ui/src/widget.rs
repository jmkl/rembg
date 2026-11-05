//! The widget interface. Widgets are headless state machines: they measure
//! themselves, react to [`InputEvent`]s (positions already in graph space),
//! edit a [`Value`], and draw into a [`DrawList`]. Nothing here needs a window.
use crate::draw::DrawList;
use crate::input::InputEvent;
use crate::schema::{SocketDef, WidgetKind};
use crate::theme::Theme;
use crate::value::Value;
use compnode_core::{Lod, Rect};

/// Ask the host to show a real text field (IME, clipboard and selection come
/// from the host toolkit). The host answers with
/// [`Widget::commit_text`] or [`Widget::cancel_edit`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextEditRequest {
    pub text: String,
    pub multiline: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Response {
    /// The widget used the event; do not process it further.
    pub consumed: bool,
    /// The value was modified.
    pub changed: bool,
    /// An editing gesture started (drag, popup interaction): group the
    /// following changes into one undo step.
    pub begin_edit: bool,
    /// The gesture ended.
    pub end_edit: bool,
    /// A button was clicked.
    pub clicked: bool,
    pub text_edit: Option<TextEditRequest>,
}

impl Response {
    pub fn consumed() -> Response {
        Response {
            consumed: true,
            ..Response::default()
        }
    }
}

pub trait Widget {
    /// Height for the given width. Never less than one row for visible widgets.
    fn measure(&self, width: f32, value: &Value) -> f32;
    /// Handle an event. `rect` is where the widget is placed this frame.
    fn event(&mut self, ev: &InputEvent, rect: Rect, value: &mut Value) -> Response;
    /// Draw into the list (fields on `Layer::Nodes`, popups on `Layer::Popup`).
    fn draw(&self, dl: &mut DrawList, rect: Rect, value: &Value, label: &str, theme: &Theme);
    /// Host finished a text edit started by a [`TextEditRequest`].
    fn commit_text(&mut self, _text: &str, _value: &mut Value) -> Response {
        Response::default()
    }
    /// Host abandoned the text edit.
    fn cancel_edit(&mut self) {}
    /// While true the widget wants every event (drag in progress, popup open).
    fn captures(&self) -> bool {
        false
    }
    /// Area of an open popup, for hosts that route events or clip.
    fn popup_rect(&self, _rect: Rect) -> Option<Rect> {
        None
    }
    fn set_focused(&mut self, _focused: bool) {}
    /// Height/width ratio of the shown image (image previews only).
    fn set_aspect(&mut self, _aspect: f32) {}
    /// Fraction of the row used by the label (widgets that show a label on the
    /// left and a compact control on the right). Updated from the theme.
    fn set_label_frac(&mut self, _frac: f32) {}
}

/// Builds the widget for a socket. A host installs a factory to add its own
/// widgets; the built-in set is used for every socket the factory declines.
///
/// The factory is asked first for sockets that carry a
/// [`SocketDef::widget_id`], then for every other socket, so a host can
/// replace a built-in widget as well as add new ones.
pub type WidgetFactory = Box<dyn Fn(&SocketDef) -> Option<Box<dyn Widget>>>;

/// Widgets are hidden when their socket is connected (the wire provides the
/// value) and when the level of detail is below [`Lod::Full`].
pub fn widget_visible(connected: bool, lod: Lod) -> bool {
    !connected && lod == Lod::Full
}

/// Build the widget a socket asks for. `None` for [`WidgetKind::None`].
pub fn make_widget(def: &SocketDef) -> Option<Box<dyn Widget>> {
    use crate::widgets::*;
    let w: Box<dyn Widget> = match def.widget {
        WidgetKind::None => return None,
        WidgetKind::Float => Box::new(NumberField::float(def.min, def.max, def.step)),
        WidgetKind::Int => Box::new(NumberField::int(def.min, def.max, def.step)),
        WidgetKind::Slider => {
            let (lo, hi) = def.slider_range();
            Box::new(Slider::new(lo, hi))
        }
        WidgetKind::Toggle => Box::new(Toggle::new(ToggleStyle::Checkbox)),
        WidgetKind::Button => Box::new(Button::new()),
        WidgetKind::Dropdown => Box::new(Dropdown::new(def.options.clone())),
        WidgetKind::TextArea => Box::new(TextArea::new(2, 8)),
        WidgetKind::ColorPicker => Box::new(ColorPicker::new()),
        WidgetKind::ImagePreview => Box::new(ImagePreview::new()),
        WidgetKind::Label => Box::new(Label::new()),
    };
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::SocketType;

    #[test]
    fn visibility_rule() {
        assert!(widget_visible(false, Lod::Full));
        assert!(!widget_visible(true, Lod::Full));
        assert!(!widget_visible(false, Lod::Compact));
        assert!(!widget_visible(false, Lod::Dot));
    }

    #[test]
    fn factory_builds_every_widget_kind() {
        let kinds = [
            WidgetKind::Float,
            WidgetKind::Int,
            WidgetKind::Slider,
            WidgetKind::Toggle,
            WidgetKind::Button,
            WidgetKind::Dropdown,
            WidgetKind::TextArea,
            WidgetKind::ColorPicker,
            WidgetKind::ImagePreview,
            WidgetKind::Label,
        ];
        for k in kinds {
            let def = SocketDef::new("x", SocketType::Float)
                .widget(k)
                .options(&["a", "b"]);
            let w = make_widget(&def).expect("widget");
            assert!(w.measure(120.0, &Value::Float(0.0)) > 0.0, "{k:?}");
        }
        let none = SocketDef::new("x", SocketType::Image);
        assert!(make_widget(&none).is_none());
    }
}
