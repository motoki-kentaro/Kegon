//! Terminal surface widget integrating iced input method lifecycle (`shell.request_input_method`).

use iced::advanced::input_method::{InputMethod, Purpose};
use iced::advanced::layout::{Layout, Limits, Node};
use iced::advanced::renderer::Style;
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::mouse::Cursor;
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::font::TerminalCellMetrics;

/// Calculates relative cursor bounds inside the terminal surface for candidate window anchoring.
pub fn calculate_cursor_rectangle(
    col: usize,
    line: usize,
    metrics: &TerminalCellMetrics,
) -> Rectangle {
    Rectangle::new(
        Point::new(
            col as f32 * metrics.cell_width,
            line as f32 * metrics.cell_height,
        ),
        Size::new(metrics.cell_width, metrics.cell_height),
    )
}

/// A custom widget wrapping the terminal canvas to manage IME focus and candidate window placement.
pub struct TerminalSurface<'a, Message> {
    element: Element<'a, Message, Theme, Renderer>,
    is_focused: bool,
    cursor_rect: Option<Rectangle>,
}

impl<'a, Message> TerminalSurface<'a, Message> {
    /// Creates a new [`TerminalSurface`] wrapping an inner element with focus state and relative cursor bounds.
    pub fn new(
        element: impl Into<Element<'a, Message, Theme, Renderer>>,
        is_focused: bool,
        cursor_rect: Option<Rectangle>,
    ) -> Self {
        Self {
            element: element.into(),
            is_focused,
            cursor_rect,
        }
    }
}

impl<'a, Message> Widget<Message, Theme, Renderer> for TerminalSurface<'a, Message> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.element.as_widget().tag()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        self.element.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.element.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.element.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.element.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        self.element.as_widget_mut().layout(tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &Style,
        layout: Layout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.element
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.element.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );

        if self.is_focused {
            let bounds = layout.bounds();
            let absolute_cursor = match self.cursor_rect {
                Some(r) => Rectangle {
                    x: bounds.x + r.x,
                    y: bounds.y + r.y,
                    width: r.width,
                    height: r.height,
                },
                None => bounds,
            };
            shell.request_input_method(&InputMethod::<&str>::Enabled {
                cursor: absolute_cursor,
                purpose: Purpose::Normal,
                preedit: None,
            });
        } else {
            shell.request_input_method(&InputMethod::<&str>::Disabled);
        }
    }
}

impl<'a, Message: 'static> From<TerminalSurface<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
{
    fn from(surface: TerminalSurface<'a, Message>) -> Self {
        Element::new(surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_rectangle_calculation_matches_grid_geometry() {
        let metrics = TerminalCellMetrics {
            cell_width: 10.0,
            cell_height: 20.0,
            font_size: 14.0,
        };

        let rect = calculate_cursor_rectangle(5, 3, &metrics);
        assert_eq!(rect.x, 50.0);
        assert_eq!(rect.y, 60.0);
        assert_eq!(rect.width, 10.0);
        assert_eq!(rect.height, 20.0);
    }

    #[test]
    fn cursor_rectangle_at_origin() {
        let metrics = TerminalCellMetrics {
            cell_width: 8.5,
            cell_height: 18.0,
            font_size: 13.0,
        };

        let rect = calculate_cursor_rectangle(0, 0, &metrics);
        assert_eq!(rect.x, 0.0);
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.width, 8.5);
        assert_eq!(rect.height, 18.0);
    }
}
