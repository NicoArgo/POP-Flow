use crate::app::iced::event::listen_raw;
use crate::subscriptions::launcher;
use crate::{components, fl};
use clap::Parser;
use cosmic::app::{Core, CosmicFlags, Settings, Task};
use cosmic::cctk::sctk;
use cosmic::cctk::sctk::shell::wlr_layer;
use cosmic::dbus_activation::Details;
use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::core::text::{Ellipsize, EllipsizeHeightLimit};
use cosmic::iced::event::Status;
use cosmic::iced::event::wayland::OverlapNotifyEvent;
use cosmic::iced::id::Id;
use cosmic::iced::keyboard::key::Named;
use cosmic::iced::platform_specific::runtime::wayland::{
    layer_surface::SctkLayerSurfaceSettings,
    popup::{SctkPopupSettings, SctkPositioner},
};
use cosmic::iced::platform_specific::shell::commands::layer_surface::set_padding;
use cosmic::iced::platform_specific::shell::commands::{self};
use cosmic::iced::platform_specific::shell::commands::{
    activation::request_token,
    layer_surface::{Anchor, KeyboardInteractivity, destroy_layer_surface, get_layer_surface},
};
use cosmic::iced::platform_specific::shell::wayland::commands::overlap_notify::overlap_notify;
use cosmic::iced::runtime::core::event::wayland::{LayerEvent, OutputEvent};
use cosmic::iced::runtime::core::event::{PlatformSpecific, wayland};
use cosmic::iced::runtime::core::layout::Limits;
use cosmic::iced::runtime::core::window::{Event as WindowEvent, Id as SurfaceId};
use cosmic::iced::runtime::platform_specific::wayland::CornerRadius;
use cosmic::iced::runtime::platform_specific::wayland::layer_surface::IcedMargin;
use cosmic::iced::runtime::{Action, platform_specific, task};
use cosmic::iced::widget::operation;
use cosmic::iced::widget::row;
use cosmic::iced::widget::scrollable::RelativeOffset;
// POP Flow: `Row` for the thumbnail grid's rows.
use cosmic::iced::widget::{Column, Row, column, container};
use cosmic::iced::{
    self, Border, Length, Padding, Point, Rectangle, Shadow, Size, Subscription, window,
};
use cosmic::surface::action::{LiveSettings, app_layer_shell, simple_layer_shell};
use cosmic::theme::{self, Button, Container};
use cosmic::widget::icon::IconFallback;
use cosmic::widget::space::{horizontal as horizontal_space, vertical as vertical_space};
use cosmic::widget::text_input::{self, StyleSheet as TextInputStyleSheet};
use cosmic::widget::{autosize, button, divider, icon, id_container, mouse_area, scrollable, text};
use cosmic::{Element, keyboard_nav, surface};
use iced::keyboard::{Key, Modifiers};
use iced::{Alignment, Color};
use pop_launcher::{GpuPreference, IconSource, SearchResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
// POP Flow: `PathBuf` for the context menu's file-path extraction.
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::LazyLock;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
// POP Flow: the thumbnail grid truncates window titles by display width, so a
// CJK or emoji title doesn't get cut mid-glyph. Upstream dropped both crates
// when it rewrote the result list; they are ours now (see Cargo.toml).
use unicode_truncate::UnicodeTruncateStr;
use unicode_width::UnicodeWidthStr;

static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("autosize"));
static MAIN_ID: LazyLock<Id> = LazyLock::new(|| Id::new("main"));
static INPUT_ID: LazyLock<Id> = LazyLock::new(|| Id::new("input_id"));
static SCROLLABLE: LazyLock<Id> = LazyLock::new(|| Id::new("scrollable"));

pub(crate) static MENU_ID: LazyLock<SurfaceId> = LazyLock::new(SurfaceId::unique);
const SCROLL_MIN: usize = 8;

#[derive(Parser, Debug, Serialize, Deserialize, Clone)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Args {
    #[clap(subcommand)]
    pub subcommand: Option<LauncherTasks>,
}

#[derive(Debug, Serialize, Deserialize, Clone, clap::Subcommand)]
pub enum LauncherTasks {
    #[clap(about = "Toggle the launcher and switch to the alt-tab view")]
    AltTab,
    #[clap(about = "Toggle the launcher and switch to the alt-tab view")]
    ShiftAltTab,
    #[clap(about = "Start the launcher with an input")]
    Input { input: Option<String> },
    #[clap(about = "Close the launcher if open")]
    Close,
}

impl Display for LauncherTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::ser::to_string(self).unwrap())
    }
}

impl FromStr for LauncherTasks {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::de::from_str(s)
    }
}

impl CosmicFlags for Args {
    type SubCommand = LauncherTasks;
    type Args = Vec<String>;

    fn action(&self) -> Option<&LauncherTasks> {
        self.subcommand.as_ref()
    }
}

pub fn run() -> cosmic::iced::Result {
    let args = Args::parse();
    cosmic::app::run_single_instance::<CosmicLauncher>(
        Settings::default()
            .antialiasing(true)
            .client_decorations(true)
            .debug(false)
            .default_text_size(16.0)
            .scale_factor(1.0)
            .no_main_window(true)
            .exit_on_close(false),
        args,
    )
}

pub fn menu_button<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
) -> cosmic::widget::Button<'a, Message> {
    button::custom(content)
        .class(Button::AppletMenu)
        .padding(menu_control_padding())
        .width(Length::Fill)
}

pub fn menu_control_padding() -> Padding {
    let theme = cosmic::theme::active();
    let cosmic = theme.cosmic();
    [cosmic.space_xxs(), cosmic.space_m()].into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Visible,
    Hidden,
    WaitingToBeShown,
}

/// A live window thumbnail captured for the alt-tab switcher.
#[derive(Clone)]
pub struct ThumbEntry {
    identifier: String,
    title: String,
    app_id: String,
    image: cosmic::widget::image::Handle,
}

/// A single row in the right-click context menu.
#[derive(Debug, Clone)]
struct MenuEntry {
    name: String,
    action: MenuAction,
}

/// What activating a context-menu row does.
#[derive(Debug, Clone)]
pub enum MenuAction {
    /// Activate a pop-launcher-provided context option: (result id, context id).
    Context(u32, u32),
    /// Open a terminal working in this directory.
    OpenTerminal(String),
    /// Reveal this path in the file manager.
    OpenFolder(String),
    /// Copy this path to the clipboard.
    CopyPath(String),
}

#[derive(Clone)]
pub struct CosmicLauncher {
    core: Core,
    input_value: String,
    surface_state: SurfaceState,
    launcher_items: Vec<SearchResult>,
    launcher_item_icon_handles: Vec<Option<cosmic::widget::icon::Handle>>,
    /// Live window thumbnails (from the wayland capture backend), keyed by
    /// toplevel identifier; correlated to launcher items by title/app_id.
    thumbnails: Vec<ThumbEntry>,
    /// Channel to tell the capture thread to start/stop capturing.
    thumb_tx: Option<calloop::channel::Sender<crate::wayland::Cmd>>,
    /// Index of the alt-tab cell currently hovered by the pointer, if any.
    hovered: Option<usize>,
    tx: Option<mpsc::Sender<launcher::Request>>,
    /// Open right-click context menu: the rows to show. Mixes pop-launcher's own
    /// context options with launcher-side actions (open in terminal, etc.).
    menu: Option<Vec<MenuEntry>>,
    cursor_position: Option<Point<f32>>,
    focused: usize,
    last_hide: Instant,
    alt_tab: bool,
    alt_tab_released: bool,
    window_id: window::Id,
    queue: VecDeque<Message>,
    result_ids: Vec<Id>,
    overlap: HashMap<String, Rectangle>,
    margin: f32,
    height: f32,
    /// Logical size of the output the launcher sits on, learned from wayland
    /// output events. Bounds the alt-tab grid so it fills the screen and shrinks
    /// its thumbnails to fit rather than scrolling. `None` until the first output
    /// event arrives, in which case the grid falls back to a sane default size.
    screen_size: Option<(f32, f32)>,
    needs_clear: bool,
    hand_over: String,
    dummy_id: Option<window::Id>,
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    Backspace,
    TabPress,
    CompleteFocusedId(Id),
    Activate(Option<usize>),
    Context(usize),
    MenuActivate(MenuAction),
    CloseContextMenu,
    CursorMoved(Point<f32>),
    Hide,
    LauncherEvent(launcher::Event),
    Layer(LayerEvent, window::Id),
    Output(OutputEvent),
    KeyboardNav(keyboard_nav::Action),
    ActivationToken(Option<String>, String, String, GpuPreference, bool),
    AltTab,
    ShiftAltTab,
    Opened(Size, window::Id),
    AltRelease,
    Overlap(OverlapNotifyEvent),
    Surface(surface::Action),
    Thumb(crate::wayland::Event),
    Hover(usize),
    Unhover,
    CloseWindow(usize),
}

impl CosmicLauncher {
    fn request(&self, r: launcher::Request) {
        debug!("request: {:?}", r);
        if let Some(tx) = &self.tx {
            if let Err(e) = tx.blocking_send(r) {
                error!("tx: {e}");
            }
        } else {
            info!("tx not found");
        }
    }

    fn create_dummy_layer_surface(&mut self) -> Task<Message> {
        self.needs_clear = true;
        let id = window::Id::unique();
        self.dummy_id = Some(id);
        Task::batch(vec![
            cosmic::surface::surface_task(simple_layer_shell::<Message>(
                || LiveSettings {
                    padding: Some(IcedMargin::default()),
                    corners: Some(CornerRadius::default()),
                    blur: Some(false),
                },
                move || {
                    SctkLayerSurfaceSettings {
                        id,
                        layer: wlr_layer::Layer::Bottom,
                        keyboard_interactivity: wlr_layer::KeyboardInteractivity::None,
                        input_zone: Some(Vec::new()),
                        anchor: wlr_layer::Anchor::TOP,
                        output:
                            cosmic::iced::runtime::platform_specific::wayland::layer_surface::IcedOutput::Active,
                        namespace: "cosmic_launcher_dummy".into(),
                        margin: IcedMargin::default(),
                        size: Some((Some(200), Some(200))),
                        exclusive_zone: -1,
                        size_limits: Limits::NONE,
                    }
                },
                None::<fn() -> Element<'static, cosmic::Action<Message>>>,
            )),
            self.handle_overlap(),
            overlap_notify(id, true),
        ])
    }

    fn show(&mut self) -> Task<Message> {
        self.surface_state = SurfaceState::Visible;
        cosmic::surface::surface_task(app_layer_shell(
            |app: &CosmicLauncher| LiveSettings {
                padding: Some(app.layer_padding()),
                corners: None,
                blur: None,
            },
            move |app: &mut CosmicLauncher| SctkLayerSurfaceSettings {
                id: app.window_id,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                anchor: Anchor::TOP,
                namespace: "launcher".into(),
                size: None,
                // Wide cap so the multi-column alt-tab grid can grow to fill the
                // screen; the grid math in `alt_tab_view` keeps its actual size
                // within the output, and the compositor clamps to the output too.
                size_limits: Limits::NONE.min_width(1.0).min_height(1.0).max_width(8000.0),
                exclusive_zone: -1,
                ..Default::default()
            },
            None,
        ))
        .chain(self.handle_overlap())
    }

    fn hide(&mut self) -> Task<Message> {
        self.input_value.clear();
        self.focused = 0;
        self.alt_tab = false;
        self.alt_tab_released = false;
        self.queue.clear();
        self.hand_over.clear();
        self.set_thumbs_active(false);
        self.thumbnails.clear();

        self.request(launcher::Request::Close);

        let mut tasks = Vec::new();

        if self.surface_state == SurfaceState::Visible {
            tasks.push(destroy_layer_surface(self.window_id));
            if self.menu.take().is_some() {
                tasks.push(commands::popup::destroy_popup(*MENU_ID));
            }
        }

        self.surface_state = SurfaceState::Hidden;

        Task::batch(tasks)
    }

    fn focus_next(&mut self) {
        if self.launcher_items.is_empty() {
            return;
        }
        self.focused = (self.focused + 1) % self.launcher_items.len();
    }

    fn focus_previous(&mut self) {
        if self.launcher_items.is_empty() {
            return;
        }
        self.focused = (self.focused + self.launcher_items.len() - 1) % self.launcher_items.len();
    }

    fn handle_overlap(&mut self) -> Task<Message> {
        let mid_height = self.height / 2.;
        self.margin = 0.;

        for o in self.overlap.values() {
            if self.margin + mid_height < o.y
                || self.margin > o.y + o.height
                || mid_height < o.y + o.height / 2.0
            {
                continue;
            }
            self.margin = o.y + o.height;
        }
        let mut cmds = Vec::with_capacity(2);
        cmds.push(set_padding::<()>(self.window_id, self.layer_padding()).discard());
        cmds.push(
            if self.core.system_theme().cosmic().frosted_system_interface {
                task::effect(Action::PlatformSpecific(
                    platform_specific::Action::Wayland(
                        cosmic::iced::runtime::platform_specific::wayland::Action::BlurSurface(
                            self.window_id,
                            Some(vec![Rectangle {
                                x: 0.,
                                y: 0.,
                                width: f32::MAX,
                                height: f32::MAX,
                            }]),
                        ),
                    ),
                ))
            } else {
                task::effect(Action::PlatformSpecific(
                    platform_specific::Action::Wayland(
                        cosmic::iced::runtime::platform_specific::wayland::Action::BlurSurface(
                            self.window_id,
                            None,
                        ),
                    ),
                ))
            },
        );
        Task::batch(cmds)
    }

    /// Tell the capture thread to start/stop capturing window thumbnails.
    fn set_thumbs_active(&self, active: bool) {
        if let Some(tx) = &self.thumb_tx {
            let _ = tx.send(crate::wayland::Cmd::SetActive(active));
        }
    }

    /// Find a captured thumbnail matching a launcher item (a window result).
    /// Correlates by window title. pop-launcher exposes the real window title
    /// as either the item name or description, and terminal titles carry an
    /// animated spinner glyph, so titles are normalized before comparing.
    /// All captured thumbnails whose normalized window title matches this
    /// launcher item. When several windows share a title this returns them all;
    /// callers decide whether that is safe to act on.
    fn matching_thumbs(&self, item: &SearchResult) -> Vec<&ThumbEntry> {
        if item.window.is_none() {
            return Vec::new();
        }
        let a = norm_title(&item.description);
        let b = norm_title(&item.name);
        self.thumbnails
            .iter()
            .filter(|t| {
                let tt = norm_title(&t.title);
                !tt.is_empty() && (tt == a || tt == b)
            })
            .collect()
    }

    /// A representative thumbnail image to show for an item (first title match).
    /// Used by the plain launcher list; the alt-tab grid uses the stricter 1:1
    /// [`Self::assign_thumbnails`] instead.
    fn thumbnail_for(&self, item: &SearchResult) -> Option<&cosmic::widget::image::Handle> {
        self.matching_thumbs(item)
            .into_iter()
            .next()
            .map(|t| &t.image)
    }

    /// Assign at most one captured thumbnail to each launcher item, 1:1, so two
    /// windows that share a title never render the *same* image. Greedy in list
    /// order: each window item claims the first still-unclaimed thumbnail whose
    /// normalized title matches, preferring one whose app id also matches.
    /// Returns a per-item vector of indices into `self.thumbnails`.
    ///
    /// This only decides which image is drawn — closing/activating a window is
    /// done by pop-launcher id, never by this correlation, so a mismatch here is
    /// cosmetic and can never act on the wrong window.
    fn assign_thumbnails(&self) -> Vec<Option<usize>> {
        assign_thumbs(&self.launcher_items, &self.thumbnails)
    }

    /// The real window title for a launcher item (for the hover label).
    fn window_title(item: &SearchResult) -> &str {
        let raw = if item.window.is_some() {
            &item.description
        } else {
            &item.name
        };
        raw.lines().next().unwrap_or("")
    }

    /// The Windows-style thumbnail grid shown while alt-tab is active.
    fn alt_tab_view(&self) -> Element<'_, Message> {
        // Base thumbnail size; the grid scales this down (never up past 1.0) so
        // every window fits on screen at once, no scrolling.
        const THUMB_W: f32 = 264.0;
        const THUMB_H: f32 = 156.0;
        // Reserve one caption line under every thumbnail so revealing the title
        // on focus/hover doesn't grow the cell and reflow the whole grid. Kept at
        // a fixed, readable height even as the thumbnails shrink.
        const LABEL_H: f32 = 20.0;

        // Fit the whole grid inside the current output: pick a column count and a
        // thumbnail scale so nothing overflows and no scrollbar is needed. Falls
        // back to a sane default screen size until the first output event lands.
        let (screen_w, screen_h) = self.screen_size.unwrap_or((1920.0, 1080.0));
        let avail_w = (screen_w - 64.0).max(THUMB_W + 24.0);
        let avail_h = (screen_h - self.margin - 112.0).max(THUMB_H + 60.0);
        let (cols, scale) = grid_layout(self.launcher_items.len(), avail_w, avail_h);

        let thumb_w = THUMB_W * scale;
        let thumb_h = THUMB_H * scale;
        let icon_sz = 56.0 * scale;
        let close_icon = (16.0 * scale).round().max(11.0) as u16;
        // Caption chars that fit the (possibly narrowed) thumbnail width.
        let title_cap = ((thumb_w / 7.5) as usize).max(6);

        // 1:1 image assignment so two same-title windows never share a picture.
        let thumb_assign = self.assign_thumbnails();

        let cells: Vec<Element<Message>> = self
            .launcher_items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let is_focused = i == self.focused;
                let revealed = is_focused || self.hovered == Some(i);

                // Live thumbnail, or icon fallback while it loads / if unmatched.
                let media: Element<Message> = if let Some(handle) = thumb_assign
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|ti| &self.thumbnails[ti].image)
                {
                    cosmic::widget::image::Image::new(handle.clone())
                        .width(Length::Fixed(thumb_w))
                        .height(Length::Fixed(thumb_h))
                        .content_fit(cosmic::iced::ContentFit::Contain)
                        .into()
                } else if let Some(Some(icon_handle)) = self.launcher_item_icon_handles.get(i) {
                    container(
                        icon(icon_handle.clone())
                            .width(Length::Fixed(icon_sz))
                            .height(Length::Fixed(icon_sz)),
                    )
                    .center_x(Length::Fixed(thumb_w))
                    .center_y(Length::Fixed(thumb_h))
                    .into()
                } else {
                    container(vertical_space().height(Length::Fixed(thumb_h)))
                        .width(Length::Fixed(thumb_w))
                        .height(Length::Fixed(thumb_h))
                        .into()
                };

                // Windows-style close (X) in the thumbnail's top-right corner,
                // revealed on hover/focus. Shown for every window item: closing
                // goes through pop-launcher by id, so it always hits this exact
                // window — never a look-alike.
                let media: Element<Message> = if revealed && item.window.is_some() {
                    let x_button = button::custom(
                        icon::from_name("window-close-symbolic").size(close_icon),
                    )
                    .class(Button::Destructive)
                    .padding(4)
                    .on_press(Message::CloseWindow(i));
                    let close = container(cosmic::widget::tooltip(
                        x_button,
                        text::body(fl!("close-window")),
                        cosmic::widget::tooltip::Position::Top,
                    ))
                    .width(Length::Fixed(thumb_w))
                    .height(Length::Fixed(thumb_h))
                    .align_x(Horizontal::Right)
                    .align_y(Vertical::Top)
                    .padding(6);
                    cosmic::iced::widget::Stack::new()
                        .width(Length::Fixed(thumb_w))
                        .height(Length::Fixed(thumb_h))
                        .push(media)
                        .push(close)
                        .into()
                } else {
                    media
                };

                // Window name — hidden until this cell is focused or hovered, but
                // its row is always reserved (LABEL_H) so the grid never reflows.
                let title = Self::window_title(item);
                let title = if title.width() > title_cap {
                    format!("{}…", title.unicode_truncate(title_cap).0)
                } else {
                    title.to_string()
                };
                let label: Element<Message> = container(if revealed {
                    Element::from(text::caption(title).class(theme::Text::Custom(|t| {
                        cosmic::iced::widget::text::Style {
                            color: Some(t.cosmic().on_bg_color().into()),
                            ..Default::default()
                        }
                    })))
                } else {
                    horizontal_space().width(Length::Fixed(0.0)).into()
                })
                .width(Length::Fixed(thumb_w))
                .height(Length::Fixed(LABEL_H))
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .into();

                let cell = column![media, label].spacing(6).align_x(Alignment::Center);

                let btn = cosmic::widget::button::custom(cell)
                    .id(self.result_ids[i].clone())
                    .on_press(Message::Activate(Some(i)))
                    .padding(6)
                    .class(Button::Custom {
                        active: Box::new(move |focused, theme| {
                            let focused = is_focused || focused;
                            let rad = theme.cosmic().corner_radii.radius_m;
                            let a = if focused {
                                button::Catalog::hovered(theme, focused, focused, &Button::Text)
                            } else {
                                button::Catalog::active(theme, focused, focused, &Button::Text)
                            };
                            button::Style {
                                border_radius: rad.into(),
                                outline_width: 0.0,
                                ..a
                            }
                        }),
                        hovered: Box::new(move |_focused, theme| {
                            let rad = theme.cosmic().corner_radii.radius_m;
                            let a = button::Catalog::hovered(theme, true, true, &Button::Text);
                            button::Style {
                                border_radius: rad.into(),
                                outline_width: 0.0,
                                ..a
                            }
                        }),
                        disabled: Box::new(|theme| {
                            let rad = theme.cosmic().corner_radii.radius_m;
                            let a = button::Catalog::disabled(theme, &Button::Text);
                            button::Style {
                                border_radius: rad.into(),
                                outline_width: 0.0,
                                ..a
                            }
                        }),
                        pressed: Box::new(move |_focused, theme| {
                            let rad = theme.cosmic().corner_radii.radius_m;
                            let a = button::Catalog::pressed(theme, true, true, &Button::Text);
                            button::Style {
                                border_radius: rad.into(),
                                outline_width: 0.0,
                                ..a
                            }
                        }),
                    });

                mouse_area(btn)
                    .on_enter(Message::Hover(i))
                    .on_exit(Message::Unhover)
                    .on_right_release(Message::Context(i))
                    .on_middle_release(Message::CloseWindow(i))
                    .into()
            })
            .collect();

        // Lay cells out `cols` per row, padding the final row with spacers so the
        // grid stays aligned. No scrollbar — `grid_layout` already sized every
        // thumbnail so the whole grid fits the screen.
        let mut grid = Column::new().spacing(8);
        let mut iter = cells.into_iter().peekable();
        while iter.peek().is_some() {
            let mut children: Vec<Element<Message>> = Vec::with_capacity(cols);
            for _ in 0..cols {
                match iter.next() {
                    Some(cell) => children.push(cell),
                    None => children
                        .push(horizontal_space().width(Length::Fixed(thumb_w + 12.0)).into()),
                }
            }
            grid = grid.push(Row::with_children(children).spacing(8));
        }

        let body: Element<Message> = grid.into();

        let window = Column::new()
            .push(vertical_space().height(Length::Fixed(self.margin + 16.)))
            .push(
                container(id_container(body, MAIN_ID.clone()))
                    .width(Length::Shrink)
                    .height(Length::Shrink)
                    .class(Container::Custom(Box::new(|theme| {
                        let t = theme.cosmic();
                        let radii = t.radius_s().map(|x| if x < 4.0 { x } else { x + 4.0 });
                        container::Style {
                            text_color: Some(t.on_bg_color().into()),
                            icon_color: Some(t.on_bg_color().into()),
                            // Transparent-aware background: with blur active an
                            // opaque fill would hide the frosted desktop behind
                            // the layer surface.
                            background: Some(
                                Color::from(t.background(theme.transparent).base).into(),
                            ),
                            border: Border {
                                radius: radii.into(),
                                width: 1.0,
                                color: t.bg_divider().into(),
                            },
                            shadow: Shadow::default(),
                            ..Default::default()
                        }
                    })))
                    .padding(16),
            );

        Element::from(autosize::autosize(window, AUTOSIZE_ID.clone()))
    }

    fn layer_padding(&self) -> IcedMargin {
        IcedMargin {
            #[allow(clippy::cast_possible_truncation)]
            top: self.margin as i32 + 16,
            ..Default::default()
        }
    }
}

/// Normalize a window title for correlation: drop leading non-alphanumeric
/// glyphs (e.g. the terminal's animated braille spinner) and surrounding
/// whitespace so that titles from different sources/instants still match.
fn norm_title(s: &str) -> String {
    s.trim()
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

/// Best-effort app identity for a launcher item, taken from its icon name
/// (usually the desktop/app id): lowercased basename without a `.desktop` suffix.
fn item_app_hint(item: &SearchResult) -> Option<String> {
    let IconSource::Name(name) = item.icon.as_ref()? else {
        return None;
    };
    let base = name.rsplit('/').next().unwrap_or(name);
    let base = base.strip_suffix(".desktop").unwrap_or(base).trim();
    (!base.is_empty()).then(|| base.to_lowercase())
}

/// Loose match between a toplevel `app_id` and an item's app hint — they often
/// differ in casing / reverse-DNS form (e.g. `org.mozilla.firefox` vs `firefox`).
fn app_id_matches(app_id: &str, hint: &str) -> bool {
    let a = app_id.to_lowercase();
    let last = a.rsplit('.').next().unwrap_or(a.as_str());
    a == hint || last == hint || a.contains(hint) || hint.contains(last)
}

/// Greedy 1:1 assignment of captured thumbnails to launcher items (see
/// [`CosmicLauncher::assign_thumbnails`]). Pulled out as a free function over
/// slices so it can be unit-tested without a full app instance.
fn assign_thumbs(items: &[SearchResult], thumbs: &[ThumbEntry]) -> Vec<Option<usize>> {
    let mut used = vec![false; thumbs.len()];
    items
        .iter()
        .map(|item| {
            item.window?;
            let a = norm_title(&item.description);
            let b = norm_title(&item.name);
            let hint = item_app_hint(item);
            let mut fallback: Option<usize> = None;
            for (ti, t) in thumbs.iter().enumerate() {
                if used[ti] {
                    continue;
                }
                let tt = norm_title(&t.title);
                if tt.is_empty() || (tt != a && tt != b) {
                    continue;
                }
                // A title + app_id match is unambiguous — take it at once.
                if hint
                    .as_deref()
                    .is_some_and(|h| app_id_matches(&t.app_id, h))
                {
                    used[ti] = true;
                    return Some(ti);
                }
                // Otherwise keep the first title-only match as a fallback.
                if fallback.is_none() {
                    fallback = Some(ti);
                }
            }
            if let Some(ti) = fallback {
                used[ti] = true;
            }
            fallback
        })
        .collect()
}

/// Grouping key for clustering windows of the same application: the item's app
/// hint (from its icon / app id) when available, else its lowercased display
/// name. Windows of one app share a key and are drawn next to each other.
fn group_key(item: &SearchResult) -> String {
    item_app_hint(item).unwrap_or_else(|| item.name.to_lowercase())
}

/// Reorder results so windows of the same application sit next to each other,
/// disturbing the existing (MRU) order as little as possible: applications
/// appear in the order their most-recent window already appears, and windows
/// within one application keep their relative order. Window items are kept
/// ahead of non-window items, preserving the previous partition. Stable.
fn group_by_app(items: &mut [SearchResult]) {
    // First-occurrence rank of each app group in the current order, so the app
    // owning the most-recent window stays first.
    let mut rank: HashMap<String, usize> = HashMap::new();
    for item in items.iter() {
        let next = rank.len();
        rank.entry(group_key(item)).or_insert(next);
    }
    items.sort_by_key(|item| {
        (
            i32::from(item.window.is_none()),
            rank.get(&group_key(item)).copied().unwrap_or(usize::MAX),
        )
    });
}

/// Choose how many columns the alt-tab grid uses and how much to scale its
/// thumbnails so all `n` cells fit inside `avail_w` x `avail_h` without
/// scrolling. Tries every column count and keeps the one that yields the
/// largest thumbnails; the scale is capped at 1.0 (never upscaled past the
/// captured size) and floored at `MIN_SCALE` (past which the grid may exceed
/// the screen rather than render unreadably tiny thumbnails). The caption line
/// and per-cell padding are treated as fixed (unscaled) chrome so labels stay
/// legible as the images shrink. Returns `(columns, scale)`.
fn grid_layout(n: usize, avail_w: f32, avail_h: f32) -> (usize, f32) {
    const THUMB_W: f32 = 264.0;
    const THUMB_H: f32 = 156.0;
    // Fixed chrome per cell: height = caption + inner spacing + button padding;
    // width = button padding only.
    const CELL_FIXED_H: f32 = 38.0;
    const CELL_FIXED_W: f32 = 12.0;
    const GAP: f32 = 8.0;
    const MIN_SCALE: f32 = 0.30;

    if n == 0 {
        return (1, 1.0);
    }

    let (mut best_cols, mut best_scale) = (1usize, 0.0f32);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let (colsf, rowsf) = (cols as f32, rows as f32);
        let row_fixed = colsf * CELL_FIXED_W + (colsf - 1.0) * GAP;
        let col_fixed = rowsf * CELL_FIXED_H + (rowsf - 1.0) * GAP;
        let s_w = (avail_w - row_fixed) / (colsf * THUMB_W);
        let s_h = (avail_h - col_fixed) / (rowsf * THUMB_H);
        let s = s_w.min(s_h).min(1.0);
        if s > best_scale {
            best_scale = s;
            best_cols = cols;
        }
    }
    (best_cols, best_scale.max(MIN_SCALE))
}

fn alt_tab_modifier_is_released(modifiers: Modifiers) -> bool {
    !modifiers.alt() && !modifiers.logo() && !modifiers.control()
}

async fn launch(
    token: Option<String>,
    app_id: String,
    exec: String,
    gpu: GpuPreference,
    terminal: bool,
) {
    let mut envs = Vec::new();
    if let Some(token) = token {
        envs.push(("XDG_ACTIVATION_TOKEN".to_string(), token.clone()));
        envs.push(("DESKTOP_STARTUP_ID".to_string(), token));
    }

    if let Some(gpu_envs) = try_get_gpu_envs(gpu).await {
        envs.extend(gpu_envs);
    }

    cosmic::desktop::spawn_desktop_exec(exec, envs, Some(&app_id), terminal).await;
}

/// Spawn a shell-style exec line detached from the launcher, used by the
/// context-menu actions (e.g. "open in terminal").
async fn spawn_exec(exec: String) {
    cosmic::desktop::spawn_desktop_exec(exec, Vec::<(String, String)>::new(), None, false).await;
}

/// POSIX single-quote a string so a path with spaces or shell metacharacters
/// survives the exec-line parsing in `spawn_desktop_exec`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Extract an existing filesystem path from a search result, if it looks like
/// one. File/folder results carry the absolute path in the description (or the
/// name); a leading `~` is expanded. Returns `None` for windows and for text
/// that doesn't resolve to something on disk, so apps don't get file actions.
fn result_path(item: &SearchResult) -> Option<PathBuf> {
    if item.window.is_some() {
        return None;
    }
    for raw in [item.description.as_str(), item.name.as_str()] {
        let raw = raw.trim();
        let candidate = if let Some(rest) = raw.strip_prefix("~/") {
            match std::env::var_os("HOME") {
                Some(home) => Path::new(&home).join(rest),
                None => continue,
            }
        } else if raw.starts_with('/') {
            PathBuf::from(raw)
        } else {
            continue;
        };
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Launcher-side context-menu rows for a result, shown above pop-launcher's own
/// options. Keys off a filesystem path in the result (file/folder results
/// expose one): "Open in terminal" at that location, "Open folder", "Copy path".
/// Returns empty for results without a resolvable path (apps, windows).
fn launcher_actions(item: &SearchResult) -> Vec<MenuEntry> {
    let Some(path) = result_path(item) else {
        return Vec::new();
    };
    // The directory to act on: the path itself when it's a folder, else its parent.
    let dir = if path.is_dir() {
        path.clone()
    } else {
        path.parent().map_or_else(|| path.clone(), Path::to_path_buf)
    };
    let dir = dir.to_string_lossy().into_owned();
    let full = path.to_string_lossy().into_owned();
    vec![
        MenuEntry {
            name: fl!("open-in-terminal"),
            action: MenuAction::OpenTerminal(dir.clone()),
        },
        MenuEntry {
            name: fl!("open-folder"),
            action: MenuAction::OpenFolder(dir),
        },
        MenuEntry {
            name: fl!("copy-path"),
            action: MenuAction::CopyPath(full),
        },
    ]
}

async fn try_get_gpu_envs(gpu: GpuPreference) -> Option<HashMap<String, String>> {
    let connection = zbus::Connection::system().await.ok()?;
    let proxy = switcheroo_control::SwitcherooControlProxy::new(&connection)
        .await
        .ok()?;
    let gpus = proxy.get_gpus().await.ok()?;
    match gpu {
        GpuPreference::Default => gpus.into_iter().find(|gpu| gpu.default),
        GpuPreference::NonDefault => gpus.into_iter().find(|gpu| !gpu.default),
        GpuPreference::SpecificIdx(idx) => gpus.into_iter().nth(idx as usize),
    }
    .map(|gpu| gpu.environment)
}

impl cosmic::Application for CosmicLauncher {
    type Message = Message;
    type Executor = cosmic::executor::single::Executor;
    type Flags = Args;
    const APP_ID: &'static str = "com.system76.CosmicLauncher";

    fn init(mut core: Core, _flags: Args) -> (Self, Task<Message>) {
        core.set_app_type(cosmic::core::AppType::System);

        core.set_keyboard_nav(false);

        let mut app = CosmicLauncher {
            core,
            input_value: String::new(),
            surface_state: SurfaceState::Hidden,
            launcher_items: Vec::new(),
            launcher_item_icon_handles: Vec::new(),
            thumbnails: Vec::new(),
            thumb_tx: None,
            hovered: None,
            tx: None,
            menu: None,
            cursor_position: None,
            focused: 0,
            last_hide: Instant::now(),
            alt_tab: false,
            alt_tab_released: false,
            window_id: SurfaceId::unique(),
            queue: VecDeque::new(),
            result_ids: (0..10)
                .map(|id| Id::new(id.to_string()))
                .collect::<Vec<_>>(),
            margin: 0.,
            overlap: HashMap::new(),
            height: 800.,
            screen_size: None,
            needs_clear: false,
            hand_over: String::default(),
            dummy_id: None,
        };
        let task = app.create_dummy_layer_surface();
        app.needs_clear = false;
        (app, task)
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    #[allow(clippy::too_many_lines)]
    fn update(&mut self, message: Message) -> Task<Self::Message> {
        match message {
            Message::InputChanged(value) => {
                self.input_value.clone_from(&value);
                self.focused = 0;
                self.request(launcher::Request::Search(value));
                return operation::snap_to(SCROLLABLE.clone(), RelativeOffset::START);
            }
            Message::Backspace => {
                self.input_value.pop();
                self.focused = 0;
                self.request(launcher::Request::Search(self.input_value.clone()));
                return operation::snap_to(SCROLLABLE.clone(), RelativeOffset::START);
            }
            Message::TabPress if !self.alt_tab => {
                let focused = self.focused;
                self.focused = 0;
                return cosmic::task::message(cosmic::Action::App(
                    Self::Message::CompleteFocusedId(self.result_ids[focused].clone()),
                ));
            }
            Message::TabPress => {}
            Message::CompleteFocusedId(id) => {
                let i = self
                    .result_ids
                    .iter()
                    .position(|res_id| res_id == &id)
                    .unwrap_or_default();

                if let Some(id) = self.launcher_items.get(i).map(|res| res.id) {
                    self.request(launcher::Request::Complete(id));
                }
            }
            Message::Activate(i) => {
                let alt_tab = self.alt_tab;
                if let Some(item) = self.launcher_items.get(i.unwrap_or(self.focused)) {
                    self.request(launcher::Request::Activate(item.id));
                    if alt_tab {
                        return self.hide();
                    }
                } else {
                    return self.hide();
                }
            }
            Message::Context(i) => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }

                if let Some(item) = self.launcher_items.get(i) {
                    self.request(launcher::Request::Context(item.id));
                }
            }
            Message::CursorMoved(pos) => {
                self.cursor_position = Some(pos);
            }
            Message::MenuActivate(action) => {
                let close = if self.menu.take().is_some() {
                    commands::popup::destroy_popup(*MENU_ID)
                } else {
                    Task::none()
                };
                match action {
                    MenuAction::Context(id, context) => {
                        self.request(launcher::Request::ActivateContext(id, context));
                        return close;
                    }
                    MenuAction::OpenTerminal(dir) => {
                        // Launch the COSMIC terminal in that directory, then hide.
                        let exec =
                            format!("cosmic-term --working-directory {}", sh_quote(&dir));
                        return Task::batch([
                            close,
                            Task::perform(spawn_exec(exec), |()| {
                                cosmic::action::app(Message::Hide)
                            }),
                        ]);
                    }
                    MenuAction::OpenFolder(dir) => {
                        let exec = format!("xdg-open {}", sh_quote(&dir));
                        return Task::batch([
                            close,
                            Task::perform(spawn_exec(exec), |()| {
                                cosmic::action::app(Message::Hide)
                            }),
                        ]);
                    }
                    MenuAction::CopyPath(path) => {
                        return Task::batch([close, cosmic::iced::clipboard::write(path)]);
                    }
                }
            }
            Message::Opened(size, window_id) => {
                let mut tasks = Vec::with_capacity(3);
                if let Some(dummy) = self.dummy_id
                    && dummy == window_id
                {
                    tasks.push(overlap_notify(window_id, true));
                } else if self.dummy_id.is_none() {
                    tasks.push(overlap_notify(self.window_id, true));
                }

                if !self.hand_over.is_empty() {
                    let input = self.hand_over.clone();
                    self.hand_over.clear();
                    tasks.push(self.update(Message::InputChanged(input)));
                }
                return Task::batch(tasks);
            }
            Message::LauncherEvent(e) => match e {
                launcher::Event::Started(tx) => {
                    self.tx.replace(tx);
                    self.request(launcher::Request::Search(self.input_value.clone()));
                }
                launcher::Event::ServiceIsClosed => {
                    self.request(launcher::Request::ServiceIsClosed);
                }
                launcher::Event::Response(response) => match response {
                    pop_launcher::Response::Close => {
                        return self.hide();
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    pop_launcher::Response::Context { id, options } => {
                        // Launcher-side actions (open in terminal, …) first, then
                        // whatever context options pop-launcher offers for this item.
                        let mut entries: Vec<MenuEntry> = self
                            .launcher_items
                            .iter()
                            .find(|it| it.id == id)
                            .map(launcher_actions)
                            .unwrap_or_default();
                        entries.extend(options.into_iter().map(|o| MenuEntry {
                            name: o.name,
                            action: MenuAction::Context(id, o.id),
                        }));
                        if entries.is_empty() {
                            return Task::none();
                        }

                        self.menu = Some(entries);
                        let Some(pos) = self.cursor_position.as_ref() else {
                            return Task::none();
                        };
                        let rect = Rectangle {
                            x: pos.x.round() as i32,
                            y: pos.y.round() as i32,
                            width: 1,
                            height: 1,
                        };
                        return commands::popup::get_popup(SctkPopupSettings {
                                    parent: self.window_id,
                                    id: *MENU_ID,
                                    positioner: SctkPositioner {
                                        size: None,
                                        size_limits: Limits::NONE.min_width(1.0).min_height(1.0).max_width(300.0).max_height(800.0),
                                        anchor_rect: rect,
                                        anchor:
                                            sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Anchor::Right,
                                        gravity: sctk::reexports::protocols::xdg::shell::client::xdg_positioner::Gravity::Right,
                                        reactive: true,
                                        ..Default::default()
                                    },
                                    grab: true,
                                    parent_size: None,
                                    close_with_children: false,
                                    input_zone: None,
                                });
                    }
                    pop_launcher::Response::DesktopEntry {
                        path,
                        gpu_preference,
                        action_name,
                    } => {
                        if let Some(entry) = cosmic::desktop::load_desktop_file(&[], path) {
                            let exec = if let Some(action_name) = action_name {
                                entry
                                    .desktop_actions
                                    .into_iter()
                                    .find(|action| action.name == action_name)
                                    .map(|action| action.exec)
                            } else {
                                entry.exec
                            };

                            let Some(exec) = exec else {
                                return Task::none();
                            };
                            return request_token(
                                Some(String::from(Self::APP_ID)),
                                Some(self.window_id),
                            )
                            .map(move |token| {
                                cosmic::Action::App(Message::ActivationToken(
                                    token,
                                    entry.id.to_string(),
                                    exec.clone(),
                                    gpu_preference,
                                    entry.terminal,
                                ))
                            });
                        }
                    }
                    pop_launcher::Response::Update(mut list) => {
                        if self.alt_tab && list.is_empty() {
                            return self.hide();
                        }
                        if self.alt_tab || self.input_value.is_empty() {
                            // Window-list views (alt-tab, empty launcher): most
                            // recent first, then cluster same-app windows.
                            list.reverse();
                            group_by_app(&mut list);
                        } else {
                            // Search view: keep pop-launcher's relevance order,
                            // only floating window matches above the rest.
                            list.sort_by_key(|item| i32::from(item.window.is_none()));
                        }
                        self.launcher_items.splice(.., list);
                        // Keep the focused index in range when the list shrinks
                        // (e.g. after a window closes) so nothing points past the
                        // end.
                        if self.focused >= self.launcher_items.len() {
                            self.focused = self.launcher_items.len().saturating_sub(1);
                        }
                        if self.result_ids.len() < self.launcher_items.len() {
                            self.result_ids.extend(
                                (self.result_ids.len()..self.launcher_items.len())
                                    .map(|id| Id::new((id).to_string()))
                                    .collect::<Vec<_>>(),
                            );
                        }

                        self.launcher_item_icon_handles.clear();
                        self.launcher_item_icon_handles = self
                            .launcher_items
                            .iter()
                            .map(|item| {
                                item.icon.as_ref().map(|icon_source| match icon_source {
                                    // Check if the name is actually a path
                                    IconSource::Name(name) if name.contains('/') => {
                                        let path = Path::new(&**name);
                                        if path.exists() {
                                            icon::from_path(path.into())
                                        } else {
                                            icon::from_name("application-default")
                                                .prefer_svg(true)
                                                .size(64)
                                                .fallback(Some(IconFallback::Names(vec![
                                                    "application-x-executable".into(),
                                                ])))
                                                .handle()
                                        }
                                    }
                                    // Fetch icon by name
                                    IconSource::Name(name) => icon::from_name(&**name)
                                        .prefer_svg(true)
                                        .size(64)
                                        .fallback(Some(IconFallback::Names(vec![
                                            "application-default".into(),
                                            "application-x-executable".into(),
                                        ])))
                                        .handle(),
                                    // By mime
                                    IconSource::Mime(mime) => {
                                        icon::from_name(mime.as_ref().replace('/', "-"))
                                            .prefer_svg(true)
                                            .size(64)
                                            .fallback(Some(IconFallback::Names(vec![
                                                "application-default".into(),
                                                "application-x-executable".into(),
                                            ])))
                                            .handle()
                                    }
                                })
                            })
                            .collect();

                        let mut cmds = Vec::new();

                        while let Some(element) = self.queue.pop_front() {
                            let updated = self.update(element);
                            cmds.push(updated);
                        }

                        if self.alt_tab_released {
                            cmds.push(self.update(Message::Activate(None)));
                        } else if self.surface_state == SurfaceState::WaitingToBeShown {
                            cmds.push(self.show());
                        }
                        return Task::batch(cmds);
                    }
                    pop_launcher::Response::Fill(s) => {
                        self.input_value = s;
                        self.request(launcher::Request::Search(self.input_value.clone()));
                    }
                },
            },
            Message::Layer(LayerEvent::Done, id) if self.dummy_id == Some(id) => {
                self.dummy_id = None;
            }
            Message::Output(event) => {
                // Track the output the launcher sits on so the alt-tab grid can
                // size itself to the screen. Single-monitor setups report one
                // output; on multi-monitor we keep the most recent, which is a
                // heuristic — good enough to pick a grid that fits.
                if let OutputEvent::Created(Some(info)) | OutputEvent::InfoUpdate(info) = &event
                    && let Some((w, h)) = info.logical_size
                    && w > 1
                    && h > 1
                {
                    self.screen_size = Some((w as f32, h as f32));
                }

                if matches!(event, OutputEvent::Created(_) | OutputEvent::InfoUpdate(_))
                    && self.dummy_id.is_none()
                {
                    return self.create_dummy_layer_surface();
                }
            }
            Message::Layer(_, id) if id != self.window_id => {}
            Message::Layer(e, _) => match e {
                LayerEvent::Focused | LayerEvent::Done => {}
                LayerEvent::Unfocused => {
                    self.last_hide = Instant::now();
                    return self.hide();
                }
            },
            Message::Overlap(overlap_notify_event) => match overlap_notify_event {
                OverlapNotifyEvent::OverlapLayerAdd {
                    identifier,
                    namespace,
                    logical_rect,
                    exclusive,
                    ..
                } => {
                    if self.needs_clear {
                        self.needs_clear = false;
                        self.overlap.clear();
                    }
                    if exclusive > 0 || namespace == "Dock" || namespace == "Panel" {
                        self.overlap.insert(identifier, logical_rect);
                    }
                    return self.handle_overlap();
                }
                OverlapNotifyEvent::OverlapLayerRemove { identifier } => {
                    self.overlap.remove(&identifier);
                    return self.handle_overlap();
                }
                _ => {}
            },
            Message::CloseContextMenu => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }
            }
            Message::Hide => {
                if self.menu.take().is_some() {
                    return commands::popup::destroy_popup(*MENU_ID);
                }
                return self.hide();
            }
            Message::KeyboardNav(e) => {
                match e {
                    keyboard_nav::Action::FocusNext => {
                        self.focus_next();
                        // TODO ideally we could use an operation to scroll exactly to a specific widget.
                        return operation::snap_to(
                            SCROLLABLE.clone(),
                            RelativeOffset {
                                x: None,
                                y: Some(
                                    (self.focused as f32
                                        / (self.launcher_items.len() as f32 - 1.).max(1.))
                                    .max(0.0),
                                ),
                            },
                        );
                    }
                    keyboard_nav::Action::FocusPrevious => {
                        self.focus_previous();
                        return operation::snap_to(
                            SCROLLABLE.clone(),
                            RelativeOffset {
                                x: None,
                                y: Some(
                                    (self.focused as f32
                                        / (self.launcher_items.len() as f32 - 1.).max(1.))
                                    .max(0.0),
                                ),
                            },
                        );
                    }
                    keyboard_nav::Action::Escape => {
                        self.input_value.clear();
                        self.request(launcher::Request::Search(String::new()));
                    }
                    _ => {}
                };
            }
            Message::ActivationToken(token, app_id, exec, dgpu, terminal) => {
                return Task::perform(launch(token, app_id, exec, dgpu, terminal), |()| {
                    cosmic::action::app(Message::Hide)
                });
            }
            Message::AltTab => {
                self.focus_next();
                return operation::snap_to(
                    SCROLLABLE.clone(),
                    RelativeOffset {
                        x: None,
                        y: Some(
                            (self.focused as f32 / (self.launcher_items.len() as f32 - 1.).max(1.))
                                .max(0.0),
                        ),
                    },
                );
            }
            Message::ShiftAltTab => {
                self.focus_previous();
                return operation::snap_to(
                    SCROLLABLE.clone(),
                    RelativeOffset {
                        x: None,
                        y: Some(
                            (self.focused as f32 / (self.launcher_items.len() as f32 - 1.).max(1.))
                                .max(0.0),
                        ),
                    },
                );
            }
            Message::AltRelease => {
                if self.alt_tab {
                    if self.surface_state == SurfaceState::Visible {
                        debug!("alt-tab modifier released; activating focused item");
                        return self.update(Message::Activate(None));
                    }

                    debug!(
                        "alt-tab modifier released before launcher was visible; deferring activation"
                    );
                    self.alt_tab_released = true;
                }
            }
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(a),
                ));
            }
            Message::Thumb(event) => match event {
                crate::wayland::Event::Ready(tx) => {
                    self.thumb_tx = Some(tx);
                }
                crate::wayland::Event::Thumbnail {
                    identifier,
                    title,
                    app_id,
                    image,
                } => {
                    if let Some(entry) =
                        self.thumbnails.iter_mut().find(|t| t.identifier == identifier)
                    {
                        entry.title = title;
                        entry.app_id = app_id;
                        entry.image = image;
                    } else {
                        self.thumbnails.push(ThumbEntry {
                            identifier,
                            title,
                            app_id,
                            image,
                        });
                    }
                }
                crate::wayland::Event::Closed(identifier) => {
                    self.thumbnails.retain(|t| t.identifier != identifier);
                }
            },
            Message::Hover(i) => {
                self.hovered = Some(i);
            }
            Message::Unhover => {
                self.hovered = None;
            }
            Message::CloseWindow(i) => {
                // Close the exact window by its pop-launcher id (never by title),
                // so a look-alike window is never at risk.
                let Some(id) = self
                    .launcher_items
                    .get(i)
                    .filter(|item| item.window.is_some())
                    .map(|item| item.id)
                else {
                    return Task::none();
                };
                self.request(launcher::Request::Quit(id));
                // Optimistically drop the entry so the grid updates immediately;
                // pop-launcher refreshes the list on its next search.
                self.launcher_items.remove(i);
                if i < self.launcher_item_icon_handles.len() {
                    self.launcher_item_icon_handles.remove(i);
                }
                if self.launcher_items.is_empty() {
                    if self.alt_tab {
                        return self.hide();
                    }
                } else if self.focused >= self.launcher_items.len() {
                    self.focused = self.launcher_items.len() - 1;
                }
            }
        }
        Task::none()
    }

    fn dbus_activation(
        &mut self,
        msg: cosmic::dbus_activation::Message,
    ) -> iced::Task<cosmic::Action<Self::Message>> {
        match msg.msg {
            Details::Activate => {
                if self.surface_state != SurfaceState::Hidden {
                    return self.hide();
                }
                // hack: allow to close the launcher from the panel button
                if self.last_hide.elapsed().as_millis() > 100 {
                    self.request(launcher::Request::Search(String::new()));

                    self.surface_state = SurfaceState::WaitingToBeShown;
                    return Task::none();
                }
            }
            Details::ActivateAction { action, .. } => {
                debug!("ActivateAction {}", action);

                let Ok(cmd) = LauncherTasks::from_str(&action) else {
                    return Task::none();
                };

                if self.surface_state == SurfaceState::Hidden {
                    self.surface_state = SurfaceState::WaitingToBeShown;
                }

                match cmd {
                    LauncherTasks::AltTab => {
                        if self.alt_tab {
                            if self.surface_state == SurfaceState::WaitingToBeShown
                                || self.launcher_items.is_empty()
                            {
                                self.queue.push_back(Message::AltTab);
                                return Task::none();
                            }
                            return self.update(Message::AltTab);
                        }

                        self.alt_tab = true;
                        self.set_thumbs_active(true);
                        self.alt_tab_released = false;
                        self.request(launcher::Request::Search(String::new()));
                        self.queue.push_back(Message::AltTab);
                    }
                    LauncherTasks::ShiftAltTab => {
                        if self.alt_tab {
                            if self.surface_state == SurfaceState::WaitingToBeShown
                                || self.launcher_items.is_empty()
                            {
                                self.queue.push_back(Message::ShiftAltTab);
                                return Task::none();
                            }
                            return self.update(Message::ShiftAltTab);
                        }

                        self.alt_tab = true;
                        self.set_thumbs_active(true);
                        self.alt_tab_released = false;
                        self.request(launcher::Request::Search(String::new()));
                        self.queue.push_back(Message::ShiftAltTab);
                    }
                    LauncherTasks::Input { input } => {
                        self.request(launcher::Request::Search(String::new()));
                        if let Some(input) = input {
                            self.hand_over.push_str(&input);
                        };
                    }
                    LauncherTasks::Close => {
                        return self.update(Message::Hide);
                    }
                }
            }
            Details::Open { .. } => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        unreachable!("No main window")
    }

    #[allow(clippy::too_many_lines)]
    fn view_window(&self, id: SurfaceId) -> Element<'_, Self::Message> {
        if id == self.window_id {
            if self.alt_tab {
                return self.alt_tab_view();
            }
            let launcher_entry = text_input::search_input(fl!("type-to-search"), &self.input_value)
                .on_input(Message::InputChanged)
                .on_paste(Message::InputChanged)
                .on_submit(|_| Message::Activate(None))
                .on_tab(Message::TabPress)
                .style(cosmic::theme::TextInput::Custom {
                    active: Box::new(|theme| theme.focused(&cosmic::theme::TextInput::Search)),
                    error: Box::new(|theme| theme.focused(&cosmic::theme::TextInput::Search)),
                    hovered: Box::new(|theme| theme.focused(&cosmic::theme::TextInput::Search)),
                    focused: Box::new(|theme| theme.focused(&cosmic::theme::TextInput::Search)),
                    disabled: Box::new(|theme| theme.disabled(&cosmic::theme::TextInput::Search)),
                })
                .width(600.)
                .id(INPUT_ID.clone())
                .always_active();

            let buttons: Vec<_> = self
                .launcher_items
                .iter()
                .enumerate()
                .flat_map(|(i, item)| {
                    let (name, desc) = if item.window.is_some() {
                        (&item.description, &item.name)
                    } else {
                        (&item.name, &item.description)
                    };

                    let name = Column::with_children(name.lines().map(|line| {
                        text::body(line.to_string())
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                            .align_x(Horizontal::Left)
                            .align_y(Vertical::Center)
                            .class(cosmic::theme::Text::Custom(|t| {
                                let theme = t.cosmic();
                                cosmic::iced::widget::text::Style {
                                    color: Some(theme.on_bg_color().into()),
                                    selected_fill: theme.accent_color().into(),
                                }
                            }))
                            .into()
                    }));

                    let desc = Column::with_children(desc.lines().map(|line| {
                        text::caption(line.to_string())
                            .ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(1)))
                            .align_x(Horizontal::Left)
                            .align_y(Vertical::Center)
                            .class(theme::Text::Custom(|t| {
                                let theme = t.cosmic();
                                cosmic::iced::widget::text::Style {
                                    color: Some(theme.on_bg_color().into()),
                                    selected_fill: theme.accent_color().into(),
                                }
                            }))
                            .into()
                    }));

                    let mut button_content = Vec::new();
                    if !self.alt_tab
                        && let Some(source) = item.category_icon.as_ref()
                    {
                        let icon_handle = match source {
                            IconSource::Name(name) => {
                                if Path::new(name.as_ref()).exists() {
                                    icon::from_path(Path::new(name.as_ref()).into())
                                } else {
                                    icon::from_name(name.as_ref()).handle()
                                }
                            }

                            IconSource::Mime(mime) => {
                                icon::from_name(mime.as_ref().replace('/', "-")).handle()
                            }
                        };

                        button_content.push(
                            icon(icon_handle)
                                .width(Length::Fixed(16.0))
                                .height(Length::Fixed(16.0))
                                .class(cosmic::theme::Svg::Custom(Rc::new(|theme| {
                                    cosmic::iced::widget::svg::Style {
                                        color: Some(theme.cosmic().on_bg_color().into()),
                                    }
                                })))
                                .into(),
                        );
                    }
                    if let Some(thumb) = self.thumbnail_for(item) {
                        button_content.push(
                            cosmic::widget::image::Image::new(thumb.clone())
                                .width(Length::Fixed(160.0))
                                .height(Length::Fixed(100.0))
                                .content_fit(cosmic::iced::ContentFit::Contain)
                                .into(),
                        );
                    } else if let Some(Some(icon_handle)) = self.launcher_item_icon_handles.get(i) {
                        button_content.push(
                            icon(icon_handle.clone())
                                .width(Length::Fixed(32.0))
                                .height(Length::Fixed(32.0))
                                .into(),
                        );
                    }

                    button_content.push(column![name, desc].width(Length::FillPortion(5)).into());
                    if i < 10 {
                        button_content.push(
                            container(
                                text::body(format!("Ctrl + {}", (i + 1) % 10))
                                    .align_y(Vertical::Center)
                                    .align_x(Horizontal::Right)
                                    .class(theme::Text::Custom(|t| {
                                        let theme = t.cosmic();
                                        cosmic::iced::widget::text::Style {
                                            color: Some(theme.on_bg_color().into()),
                                            selected_fill: theme.accent_color().into(),
                                        }
                                    })),
                            )
                            .width(Length::FillPortion(1))
                            .center_y(Length::Shrink)
                            .align_y(Vertical::Center)
                            .align_x(Horizontal::Right)
                            .into(),
                        );
                    }
                    let is_focused = i == self.focused;
                    let btn = mouse_area(
                        cosmic::widget::button::custom(
                            row(button_content).spacing(8).align_y(Alignment::Center),
                        )
                        .id(self.result_ids[i].clone())
                        .width(Length::Fill)
                        .on_press(Message::Activate(Some(i)))
                        .padding([8, 24])
                        .class(Button::Custom {
                            active: Box::new(move |focused, theme| {
                                let focused = is_focused || focused;
                                let rad_s = theme.cosmic().corner_radii.radius_s;
                                let a = if focused {
                                    button::Catalog::hovered(theme, focused, focused, &Button::Text)
                                } else {
                                    button::Catalog::active(theme, focused, focused, &Button::Text)
                                };
                                button::Style {
                                    border_radius: rad_s.into(),
                                    outline_width: 0.0,
                                    ..a
                                }
                            }),
                            hovered: Box::new(move |focused, theme| {
                                let focused = is_focused || focused;
                                let rad_s = theme.cosmic().corner_radii.radius_s;

                                let text = button::Catalog::hovered(
                                    theme,
                                    focused,
                                    focused,
                                    &Button::Text,
                                );
                                button::Style {
                                    border_radius: rad_s.into(),
                                    outline_width: 0.0,
                                    ..text
                                }
                            }),
                            disabled: Box::new(|theme| {
                                let rad_s = theme.cosmic().corner_radii.radius_s;

                                let text = button::Catalog::disabled(theme, &Button::Text);
                                button::Style {
                                    border_radius: rad_s.into(),
                                    outline_width: 0.0,
                                    ..text
                                }
                            }),
                            pressed: Box::new(move |focused, theme| {
                                let focused = is_focused || focused;
                                let rad_s = theme.cosmic().corner_radii.radius_s;

                                let text = button::Catalog::pressed(
                                    theme,
                                    focused,
                                    focused,
                                    &Button::Text,
                                );
                                button::Style {
                                    border_radius: rad_s.into(),
                                    outline_width: 0.0,
                                    ..text
                                }
                            }),
                        }),
                    )
                    .on_right_release(Message::Context(i));
                    if i == self.launcher_items.len() - 1 {
                        vec![btn.into()]
                    } else {
                        vec![btn.into(), divider::horizontal::light().into()]
                    }
                })
                .collect();

            let mut content = if self.alt_tab {
                Column::new()
                    .max_width(600)
                    .spacing(16)
                    .width(Length::Fixed(600.))
                    .height(Length::Shrink)
            } else {
                column![launcher_entry]
                    .max_width(600)
                    .width(Length::Shrink)
                    .height(Length::Shrink)
                    .spacing(16)
            };

            if buttons.len() > SCROLL_MIN {
                content = content.push(
                    container(scrollable(components::list::column(buttons)).id(SCROLLABLE.clone()))
                        .max_height(504),
                );
            } else if !buttons.is_empty() {
                content = content.push(components::list::column(buttons));
            }

            let window = Column::new()
                .push(vertical_space().height(Length::Fixed(self.margin + 16.)))
                .push(
                    container(id_container(content, MAIN_ID.clone()))
                        .width(Length::Shrink)
                        .height(Length::Shrink)
                        .class(Container::Custom(Box::new(|theme| {
                            let t = theme.cosmic();
                            let radii = t.radius_s().map(|x| if x < 4.0 { x } else { x + 4.0 });

                            container::Style {
                                text_color: Some(t.on_bg_color().into()),
                                icon_color: Some(t.on_bg_color().into()),
                                background: Some(
                                    Color::from(t.background(theme.transparent).base).into(),
                                ),
                                border: Border {
                                    radius: radii.into(),
                                    width: 1.0,
                                    color: t.bg_divider().into(),
                                },
                                shadow: Shadow::default(),
                                snap: true,
                            }
                        })))
                        .padding([24, 32]),
                );

            let autosize = autosize::autosize(
                if self.menu.is_some() {
                    Element::from(
                        mouse_area(window)
                            .on_release(Message::CloseContextMenu)
                            .on_right_release(Message::CloseContextMenu),
                    )
                } else {
                    window.into()
                },
                AUTOSIZE_ID.clone(),
            );
            return Element::from(autosize);
        }
        if id == *MENU_ID {
            let Some(entries) = self.menu.as_ref() else {
                return container(horizontal_space().width(Length::Fixed(1.0)))
                    .width(Length::Fixed(1.0))
                    .height(Length::Fixed(1.0))
                    .into();
            };
            let list_column = Column::with_children(entries.iter().map(|entry| {
                menu_button(text::body(&entry.name))
                    .on_press(Message::MenuActivate(entry.action.clone()))
                    .into()
            }))
            .padding([8, 0]);

            return container(
                container(scrollable(list_column)).class(theme::Container::custom(|theme| {
                    let cosmic = theme.cosmic();
                    let corners = cosmic.corner_radii;
                    container::Style {
                        text_color: Some(cosmic.background(theme.transparent).on.into()),
                        background: Some(
                            Color::from(cosmic.background(theme.transparent).base).into(),
                        ),
                        border: Border {
                            radius: corners.radius_m.into(),
                            width: 1.0,
                            color: cosmic.background(theme.transparent).divider.into(),
                        },
                        shadow: Shadow::default(),
                        icon_color: Some(cosmic.background(theme.transparent).on.into()),
                        snap: true,
                    }
                })),
            )
            .width(Length::Shrink)
            .height(Length::Shrink)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Top)
            .into();
        }

        vertical_space().height(Length::Fixed(1.0)).into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch(vec![
            launcher::subscription(0).map(Message::LauncherEvent),
            crate::wayland::subscription().map(Message::Thumb),
            listen_raw(|e, status, id| match e {
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Layer(e, _, layer_id),
                )) => Some(Message::Layer(e, layer_id)),
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::OverlapNotify(event, ..),
                )) => Some(Message::Overlap(event)),
                cosmic::iced::Event::PlatformSpecific(PlatformSpecific::Wayland(
                    wayland::Event::Output(event, _),
                )) => Some(Message::Output(event)),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                    key: Key::Named(Named::Alt | Named::Super | Named::Control),
                    ..
                }) => Some(Message::AltRelease),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
                    modifiers,
                    ..
                }) if alt_tab_modifier_is_released(modifiers) => Some(Message::AltRelease),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
                    modifiers,
                )) if alt_tab_modifier_is_released(modifiers) => Some(Message::AltRelease),
                cosmic::iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key,
                    text: _,
                    modifiers,
                    ..
                }) => match key {
                    Key::Character(c) if modifiers.control() && (c == "p" || c == "k") => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious))
                    }
                    Key::Character(c) if modifiers.control() && (c == "n" || c == "j") => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext))
                    }
                    Key::Character(c) if modifiers.control() => {
                        let nums = (1..10)
                            .map(|n| (n.to_string(), ((n + 10) % 10) - 1))
                            .collect::<Vec<_>>();
                        nums.iter()
                            .find_map(|n| (n.0 == c).then(|| Message::Activate(Some(n.1))))
                    }
                    Key::Named(Named::ArrowUp) => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusPrevious))
                    }
                    Key::Named(Named::ArrowDown) => {
                        Some(Message::KeyboardNav(keyboard_nav::Action::FocusNext))
                    }
                    Key::Named(Named::Escape) => Some(Message::Hide),
                    Key::Named(Named::Tab) => Some(Message::TabPress),
                    Key::Named(Named::Backspace)
                        if matches!(status, Status::Ignored) && modifiers.is_empty() =>
                    {
                        Some(Message::Backspace)
                    }
                    _ => None,
                },
                cosmic::iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::CursorMoved(position))
                }
                cosmic::iced::Event::Window(WindowEvent::Opened { position: _, size }) => {
                    Some(Message::Opened(size, id))
                }
                cosmic::iced::Event::Window(WindowEvent::Resized(s)) => {
                    Some(Message::Opened(s, id))
                }
                _ => None,
            }),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pop_launcher::IconSource;

    fn item(id: u32, name: &str, desc: &str, icon: Option<&str>, is_window: bool) -> SearchResult {
        SearchResult {
            id,
            name: name.into(),
            description: desc.into(),
            icon: icon.map(|s| IconSource::Name(s.to_string().into())),
            category_icon: None,
            window: is_window.then_some((0, id)),
        }
    }

    fn thumb(identifier: &str, title: &str, app_id: &str) -> ThumbEntry {
        ThumbEntry {
            identifier: identifier.into(),
            title: title.into(),
            app_id: app_id.into(),
            image: cosmic::widget::image::Handle::from_rgba(1, 1, vec![0u8; 4]),
        }
    }

    fn ids(items: &[SearchResult]) -> Vec<u32> {
        items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn norm_title_strips_leading_spinner_and_ws() {
        // The terminal's animated braille glyph must not defeat correlation.
        assert_eq!(norm_title("⠋ user@host: ~"), norm_title("user@host: ~"));
        assert_eq!(norm_title("  Firefox  "), "Firefox");
        assert_eq!(norm_title("***"), "");
    }

    #[test]
    fn app_id_matches_reverse_dns_and_case() {
        assert!(app_id_matches("org.mozilla.firefox", "firefox"));
        assert!(app_id_matches("Firefox", "firefox"));
        assert!(!app_id_matches("org.gnome.gedit", "firefox"));
    }

    #[test]
    fn item_app_hint_normalizes_icon_name() {
        assert_eq!(
            item_app_hint(&item(1, "Firefox", "t", Some("org.mozilla.Firefox"), true)).as_deref(),
            Some("org.mozilla.firefox")
        );
        assert_eq!(
            item_app_hint(&item(1, "X", "t", Some("/a/b/foo.desktop"), true)).as_deref(),
            Some("foo")
        );
        assert_eq!(item_app_hint(&item(1, "X", "t", None, true)), None);
    }

    #[test]
    fn group_by_app_clusters_same_app_preserving_mru() {
        // MRU-first order: firefox, code, firefox, gimp.
        let mut list = vec![
            item(1, "Firefox", "A1", Some("firefox"), true),
            item(2, "Code", "B1", Some("code"), true),
            item(3, "Firefox", "A2", Some("firefox"), true),
            item(4, "GIMP", "C1", Some("gimp"), true),
        ];
        group_by_app(&mut list);
        // The two firefox windows become adjacent; firefox stays first because
        // it owns the most-recent window; app order is otherwise preserved.
        assert_eq!(ids(&list), vec![1, 3, 2, 4]);
    }

    #[test]
    fn group_by_app_keeps_non_windows_last() {
        let mut list = vec![
            item(1, "Firefox", "A1", Some("firefox"), true),
            item(2, "Calculator", "run", Some("calc"), false),
            item(3, "Firefox", "A2", Some("firefox"), true),
        ];
        group_by_app(&mut list);
        assert_eq!(ids(&list), vec![1, 3, 2]);
    }

    #[test]
    fn assign_thumbs_gives_same_title_windows_distinct_images() {
        // Two identical-title Firefox windows must not share one picture.
        let items = vec![
            item(1, "Firefox", "Mozilla Firefox", Some("firefox"), true),
            item(2, "Firefox", "Mozilla Firefox", Some("firefox"), true),
        ];
        let thumbs = vec![
            thumb("id-a", "Mozilla Firefox", "org.mozilla.firefox"),
            thumb("id-b", "Mozilla Firefox", "org.mozilla.firefox"),
        ];
        let got = assign_thumbs(&items, &thumbs);
        assert!(got[0].is_some() && got[1].is_some());
        assert_ne!(got[0], got[1], "each window must claim its own thumbnail");
    }

    #[test]
    fn assign_thumbs_prefers_app_id_over_title_only() {
        let items = vec![item(1, "Firefox", "Doc", Some("firefox"), true)];
        let thumbs = vec![
            thumb("wrong", "Doc", "org.gnome.gedit"),
            thumb("right", "Doc", "org.mozilla.firefox"),
        ];
        // Both share the title; the app-id match wins.
        assert_eq!(assign_thumbs(&items, &thumbs), vec![Some(1)]);
    }

    #[test]
    fn assign_thumbs_skips_non_windows_and_unmatched() {
        let items = vec![
            item(1, "Calc", "calculator", Some("calc"), false),
            item(2, "Editor", "Untitled", Some("gedit"), true),
        ];
        let thumbs = vec![thumb("id", "Something Else", "org.x.y")];
        assert_eq!(assign_thumbs(&items, &thumbs), vec![None, None]);
    }

    #[test]
    fn grid_layout_keeps_full_size_for_few_windows() {
        // A handful of windows on a big screen never upscale past 1.0.
        let (_cols, scale) = grid_layout(4, 1920.0 - 64.0, 1080.0 - 112.0);
        assert_eq!(scale, 1.0);
    }

    #[test]
    fn grid_layout_scales_down_when_crowded() {
        // 30 windows on a small laptop screen can't fit at full size, so the
        // thumbnails must shrink instead of scrolling.
        let (cols, scale) = grid_layout(30, 1366.0 - 64.0, 768.0 - 112.0);
        assert!(cols >= 3, "expected a multi-column grid, got {cols}");
        assert!(scale < 1.0, "thumbnails should shrink, got {scale}");
        assert!(scale >= 0.30, "scale must not drop below the floor, got {scale}");
    }

    #[test]
    fn grid_layout_handles_zero() {
        assert_eq!(grid_layout(0, 1000.0, 1000.0), (1, 1.0));
    }

    #[test]
    fn grid_layout_result_fits_the_budget() {
        // For any count, the chosen layout must actually fit within the budget
        // (except when the scale hits its floor, where slight overflow is
        // accepted over unreadable thumbnails).
        const THUMB_W: f32 = 264.0;
        const THUMB_H: f32 = 156.0;
        let (aw, ah) = (1600.0f32, 900.0f32);
        for n in 1..=30usize {
            let (cols, scale) = grid_layout(n, aw, ah);
            let rows = n.div_ceil(cols);
            let w = cols as f32 * (THUMB_W * scale + 12.0) + (cols as f32 - 1.0) * 8.0;
            let h = rows as f32 * (THUMB_H * scale + 38.0) + (rows as f32 - 1.0) * 8.0;
            if scale > 0.30 {
                assert!(w <= aw + 0.5, "n={n} cols={cols} width {w} > {aw}");
                assert!(h <= ah + 0.5, "n={n} rows={rows} height {h} > {ah}");
            }
        }
    }

    #[test]
    fn sh_quote_wraps_and_escapes() {
        assert_eq!(sh_quote("/home/user"), "'/home/user'");
        assert_eq!(sh_quote("/a b/c"), "'/a b/c'"); // spaces survive
        assert_eq!(sh_quote("it's"), "'it'\\''s'"); // embedded single quote
    }

    #[test]
    fn result_path_resolves_existing_abs_path() {
        // Root always exists and is a directory.
        assert_eq!(
            result_path(&item(1, "root", "/", Some("folder"), false)),
            Some(PathBuf::from("/"))
        );
    }

    #[test]
    fn result_path_rejects_windows_and_non_paths() {
        // Plain app result: no filesystem path -> no file actions.
        assert_eq!(
            result_path(&item(1, "Firefox", "Web Browser", Some("firefox"), false)),
            None
        );
        // A window result never gets file actions, even if its text looks pathy.
        assert_eq!(result_path(&item(2, "Term", "/", Some("term"), true)), None);
    }

    #[test]
    fn launcher_actions_offers_terminal_for_folders() {
        let acts = launcher_actions(&item(1, "root", "/", Some("folder"), false));
        assert_eq!(acts.len(), 3);
        assert!(matches!(acts[0].action, MenuAction::OpenTerminal(_)));
        // Non-path results get no launcher-side actions.
        assert!(launcher_actions(&item(2, "Firefox", "Web Browser", Some("ff"), false)).is_empty());
    }
}
