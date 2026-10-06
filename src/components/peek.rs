// SPDX-License-Identifier: GPL-3.0-only

//! POP Flow: "peek" — the focused alt-tab cell lifts out of the grid, drawn
//! scaled up on an overlay layer above its neighbours.
//!
//! The layout never changes: the wrapped content keeps exactly the slot the
//! grid gave it (cells stay fixed), and only the drawing is scaled about the
//! cell's centre, then nudged back inside the window if it would poke out at
//! the grid's edge. It redraws the same widgets — the thumbnail image handle
//! the cell already holds — so nothing is captured or copied for it.
//!
//! The scale eases in over [`DURATION`] when the cell becomes active and back
//! out when it stops being active, driven by redraw requests on the widget's
//! own state (like libcosmic's toggler), so the app needs no timer.
//!
//! Like iced's `Float`, which can't be used here because `cosmic::Theme`
//! doesn't implement its style catalog.

use std::time::{Duration, Instant};

use cosmic::iced::core::Renderer as _;
use cosmic::iced::core::widget::Operation;
use cosmic::iced::core::widget::tree::{self, Tree};
use cosmic::iced::core::{
    Border, Clipboard, Color, Event, Layout, Length, Rectangle, Shadow, Shell, Size,
    Transformation, Vector, Widget, layout, mouse, overlay, renderer, window,
};
use cosmic::{Element, Renderer, Theme};

/// How much the active cell grows.
pub const SCALE: f32 = 1.2;
/// Length of the grow / shrink animation.
pub const DURATION: Duration = Duration::from_millis(120);
/// Gap kept between the lifted cell and the window edge.
const EDGE: f32 = 2.0;

pub fn peek<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    active: bool,
) -> Peek<'a, Message> {
    Peek {
        content: content.into(),
        active,
    }
}

pub struct Peek<'a, Message> {
    content: Element<'a, Message>,
    active: bool,
}

#[derive(Debug, Clone, Copy)]
struct State {
    /// Target the current animation runs towards.
    active: bool,
    /// Scale when the current animation started.
    from: f32,
    /// Scale drawn right now.
    current: f32,
    /// Start of the running animation; set on its first frame.
    start: Option<Instant>,
    /// Whether an animation is pending or running.
    animating: bool,
}

impl State {
    fn floating(&self) -> bool {
        self.current > 1.001
    }
}

/// Scale `elapsed` into an animation from `from` to `to`, with an ease-out
/// curve. Returns the scale and whether the animation is finished.
pub fn eased(from: f32, to: f32, elapsed: Duration, duration: Duration) -> (f32, bool) {
    let t = if duration.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
    };
    let e = 1.0 - (1.0 - t).powi(3);
    (from + (to - from) * e, t >= 1.0)
}

/// Translation that brings `scaled` back inside `viewport` (less [`EDGE`]),
/// on each axis independently. Zero when it already fits; when it is larger
/// than the viewport on an axis, its top/left edge wins.
pub fn keep_inside(scaled: Rectangle, viewport: Rectangle) -> Vector {
    let axis = |pos: f32, len: f32, vpos: f32, vlen: f32| {
        let (lo, hi) = (vpos + EDGE, vpos + vlen - EDGE);
        if pos < lo {
            lo - pos
        } else if pos + len > hi {
            (hi - (pos + len)).max(lo - pos)
        } else {
            0.0
        }
    };
    Vector::new(
        axis(scaled.x, scaled.width, viewport.x, viewport.width),
        axis(scaled.y, scaled.height, viewport.y, viewport.height),
    )
}

/// `bounds` scaled by `scale` about its centre.
fn scaled_about_center(bounds: Rectangle, scale: f32) -> Rectangle {
    let (w, h) = (bounds.width * scale, bounds.height * scale);
    Rectangle {
        x: bounds.center_x() - w / 2.0,
        y: bounds.center_y() - h / 2.0,
        width: w,
        height: h,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Peek<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        let s = if self.active { SCALE } else { 1.0 };
        // A cell that is already active when first built (e.g. it got
        // recreated) still animates in from 1.0.
        tree::State::new(State {
            active: self.active,
            from: 1.0,
            current: 1.0,
            start: None,
            animating: s != 1.0,
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(child.size(), vec![child])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();

        // Focus moved onto / off this cell: (re)start from wherever it is now.
        if state.active != self.active {
            state.active = self.active;
            state.from = state.current;
            state.start = None;
            state.animating = true;
            shell.request_redraw();
        }

        if let Event::Window(window::Event::RedrawRequested(now)) = event
            && state.animating
        {
            let start = *state.start.get_or_insert(*now);
            let to = if state.active { SCALE } else { 1.0 };
            let (s, done) = eased(
                state.from,
                to,
                now.saturating_duration_since(start),
                DURATION,
            );
            state.current = s;
            if done {
                state.animating = false;
                state.start = None;
            } else {
                shell.request_redraw();
            }
        }

        // While lifted, the overlay handles input (in its scaled space).
        if state.floating() {
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if tree.state.downcast_ref::<State>().floating() {
            return; // drawn by the overlay instead
        }
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<State>().floating() {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let scale = tree.state.downcast_ref::<State>().current;
        let content_layout = layout.children().next().unwrap();
        if scale <= 1.001 {
            return self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                content_layout,
                renderer,
                viewport,
                translation,
            );
        }

        let bounds = content_layout.bounds();
        let nudge = keep_inside(scaled_about_center(bounds + translation, scale), *viewport);
        let shift = translation + nudge;
        let transformation =
            Transformation::translate(bounds.center_x() + shift.x, bounds.center_y() + shift.y)
                * Transformation::scale(scale)
                * Transformation::translate(-bounds.center_x(), -bounds.center_y());

        Some(overlay::Element::new(Box::new(PeekOverlay {
            content: &mut self.content,
            tree: &mut tree.children[0],
            layout: content_layout,
            viewport: *viewport,
            transformation,
            scale,
        })))
    }
}

impl<'a, Message: 'a> From<Peek<'a, Message>> for Element<'a, Message> {
    fn from(p: Peek<'a, Message>) -> Self {
        Element::new(p)
    }
}

struct PeekOverlay<'a, 'b, Message> {
    content: &'a mut Element<'b, Message>,
    tree: &'a mut Tree,
    layout: Layout<'a>,
    viewport: Rectangle,
    transformation: Transformation,
    scale: f32,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for PeekOverlay<'_, '_, Message> {
    fn layout(&mut self, _renderer: &Renderer, _bounds: Size) -> layout::Node {
        let bounds = self.layout.bounds() * self.transformation;
        layout::Node::new(bounds.size()).move_to(bounds.position())
    }

    fn update(
        &mut self,
        event: &Event,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let inverse = self.transformation.inverse();
        self.content.as_widget_mut().update(
            self.tree,
            event,
            self.layout,
            cursor * inverse,
            renderer,
            clipboard,
            shell,
            &(self.viewport * inverse),
        );
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let bounds = self.layout.bounds();
        let inverse = self.transformation.inverse();
        let cosmic = theme.cosmic();
        // Opaque card under the lifted cell: the cell's own highlight is
        // translucent, and the neighbours it now overlaps must not show
        // through it. Lift strength follows the animation.
        let lift = ((self.scale - 1.0) / (SCALE - 1.0)).clamp(0.0, 1.0);
        let mut card: Color = cosmic.background(false).base.into();
        card.a = 1.0;
        renderer.with_layer(self.viewport, |renderer| {
            renderer.with_transformation(self.transformation, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: Border {
                            radius: cosmic.corner_radii.radius_m.into(),
                            width: 0.0,
                            color: Color::TRANSPARENT,
                        },
                        shadow: Shadow {
                            color: Color {
                                a: 0.35 * lift,
                                ..Color::BLACK
                            },
                            offset: Vector::new(0.0, 4.0),
                            blur_radius: 16.0,
                        },
                        snap: false,
                    },
                    card,
                );
                self.content.as_widget().draw(
                    self.tree,
                    renderer,
                    theme,
                    style,
                    self.layout,
                    cursor * inverse,
                    &(self.viewport * inverse),
                );
            });
        });
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        let inverse = self.transformation.inverse();
        self.content.as_widget().mouse_interaction(
            self.tree,
            self.layout,
            cursor * inverse,
            &(self.viewport * inverse),
            renderer,
        )
    }

    fn index(&self) -> f32 {
        self.scale * 0.5
    }

    fn overlay<'c>(
        &'c mut self,
        _layout: Layout<'c>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'c, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            self.tree,
            self.layout,
            renderer,
            &(self.viewport * self.transformation.inverse()),
            self.transformation.translation(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eased_runs_from_start_to_end() {
        let d = DURATION;
        assert_eq!(eased(1.0, SCALE, Duration::ZERO, d), (1.0, false));
        let (mid, done) = eased(1.0, SCALE, d / 2, d);
        assert!(
            !done && mid > 1.1 && mid < SCALE,
            "ease-out is past halfway at t=0.5: {mid}"
        );
        assert_eq!(eased(1.0, SCALE, d, d), (SCALE, true));
        assert_eq!(eased(1.0, SCALE, d * 3, d), (SCALE, true));
        // Shrinking back works the same way.
        assert_eq!(eased(SCALE, 1.0, d, d), (1.0, true));
    }

    #[test]
    fn keep_inside_nudges_only_what_pokes_out() {
        let vp = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 600.0,
        };
        // Well inside: untouched.
        let r = Rectangle {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 200.0,
        };
        assert_eq!(keep_inside(r, vp), Vector::new(0.0, 0.0));
        // Off the left and top edges: pushed right / down to the edge gap.
        let r = Rectangle {
            x: -10.0,
            y: -5.0,
            width: 300.0,
            height: 200.0,
        };
        assert_eq!(keep_inside(r, vp), Vector::new(10.0 + EDGE, 5.0 + EDGE));
        // Off the right and bottom: pulled back.
        let r = Rectangle {
            x: 800.0,
            y: 450.0,
            width: 300.0,
            height: 200.0,
        };
        assert_eq!(keep_inside(r, vp), Vector::new(-100.0 - EDGE, -50.0 - EDGE));
    }

    #[test]
    fn scaled_cell_grows_about_its_centre() {
        let r = Rectangle {
            x: 100.0,
            y: 50.0,
            width: 276.0,
            height: 194.0,
        };
        let s = scaled_about_center(r, SCALE);
        assert!((s.center_x() - r.center_x()).abs() < 1e-3);
        assert!((s.center_y() - r.center_y()).abs() < 1e-3);
        assert!((s.width - 276.0 * SCALE).abs() < 1e-3);
    }
}
