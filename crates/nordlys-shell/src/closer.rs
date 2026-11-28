//! A close button for the active window. Apps like terminals draw no
//! titlebar, so this is a small layer surface that appears inside the
//! window's top-right corner when the pointer goes near it, and is a
//! single clear pixel the rest of the time. Hyprland only.
//!
//! The window's place is read when Hyprland reports a change (and every
//! second for a floating window, which can move silently). Only the pointer
//! is polled, on a thread of its own and only while there is a window to
//! close; it says something only when the pointer goes into or out of the
//! corner, so an idle desktop costs no redraws.

use crate::events;
use crate::icons::{self, icon};
use crate::sources::{Active, Compositor, Frame};
use crate::system;
use iced::alignment::{Horizontal, Vertical};
use iced::futures::channel::mpsc;
use iced::widget::{Space, button, container, mouse_area};
use iced::{Background, Border, Color, Element, Length, Subscription, Task};
use iced_layershell::to_layer_message;
use std::time::Duration;

pub const SIZE: u32 = 30;
/// Gap between the button and the window's edges.
const INSET: i32 = 8;
/// The corner that reveals the button: this far left of and below it.
const REACH_X: i32 = 160;
const REACH_Y: i32 = 64;
/// How often the pointer is looked at near the corner, and elsewhere.
const CLOSE_BY: Duration = Duration::from_millis(150);
const FAR_OFF: Duration = Duration::from_millis(400);

pub struct Closer {
    compositor: Compositor,
    /// The screen this button lives on; it shows only for windows there.
    screen: Option<String>,
    active: Option<Active>,
    shown: Option<Frame>,
    window_reads: system::Reads,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    /// Hyprland reported a change, or a floating window may have moved.
    Changed,
    Window(Option<Option<Active>>),
    /// The pointer went into (true) or out of the active window's corner.
    Near(bool),
    Close,
}

/// Whether the pointer is in the window's top-right corner.
fn near_corner(w: &Frame, (x, y): (i32, i32)) -> bool {
    let right = w.x + w.width;
    x >= right - REACH_X.min(w.width) && x <= right && y >= w.y && y <= w.y + REACH_Y.min(w.height)
}

/// Whether the pointer is close enough to the corner to be looked at often.
fn close_by(w: &Frame, (x, y): (i32, i32)) -> bool {
    let right = w.x + w.width;
    x >= right - 3 * REACH_X && x <= right + REACH_X && y >= w.y - REACH_Y && y <= w.y + 3 * REACH_Y
}

/// Watch the pointer on a thread, saying only when it goes into or out of
/// `a`'s corner. Ends when the subscription goes.
fn watch_corner(compositor: Compositor, a: Active) -> mpsc::UnboundedReceiver<bool> {
    let (tx, rx) = mpsc::unbounded();
    std::thread::spawn(move || {
        let mut last = None;
        while !tx.is_closed() {
            let p = compositor.pointer().map(|p| a.local(p));
            let near = p.is_some_and(|p| near_corner(&a.frame, p));
            if last != Some(near) {
                if tx.unbounded_send(near).is_err() {
                    return;
                }
                last = Some(near);
            }
            let soon = p.is_some_and(|p| close_by(&a.frame, p));
            std::thread::sleep(if soon { CLOSE_BY } else { FAR_OFF });
        }
    });
    rx
}

/// Layer margins (top, right, bottom, left) that put the button there.
fn margins(w: &Frame) -> (i32, i32, i32, i32) {
    let left = w.x + w.width - SIZE as i32 - INSET;
    ((w.y + INSET).max(0), 0, 0, left.max(0))
}

impl Closer {
    pub fn new(compositor: Compositor, screen: Option<String>) -> (Self, Task<Message>) {
        let mut me = Self {
            compositor,
            screen,
            active: None,
            shown: None,
            window_reads: system::Reads::default(),
        };
        let first = me.read_window();
        (me, first)
    }

    fn read_window(&mut self) -> Task<Message> {
        if !self.window_reads.start() {
            return Task::none();
        }
        let c = self.compositor.clone();
        Task::perform(
            system::background(move || c.active_window()),
            Message::Window,
        )
    }

    fn show(&mut self, next: Option<Frame>) -> Task<Message> {
        if next == self.shown {
            return Task::none();
        }
        self.shown = next;
        match &self.shown {
            Some(w) => Task::batch([
                Task::done(Message::MarginChange(margins(w))),
                Task::done(Message::SizeChange((SIZE, SIZE))),
            ]),
            None => Task::batch([
                Task::done(Message::SizeChange((1, 1))),
                Task::done(Message::MarginChange((0, 0, 0, 0))),
            ]),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Changed => self.read_window(),
            Message::Window(read) => {
                let again = self.window_reads.done();
                if let Some(active) = read {
                    // A window on another screen is that screen's button's.
                    self.active =
                        active.filter(|a| self.screen.as_deref().is_none_or(|s| s == a.screen));
                }
                // The window moved or went: hide until the pointer says otherwise.
                let hide = match (&self.active, &self.shown) {
                    (Some(a), Some(s)) => a.frame != *s,
                    (None, Some(_)) => true,
                    _ => false,
                };
                let mut tasks = vec![];
                if hide {
                    tasks.push(self.show(None));
                }
                if again {
                    tasks.push(self.read_window());
                }
                Task::batch(tasks)
            }
            Message::Near(near) => {
                let next = self
                    .active
                    .as_ref()
                    .filter(|_| near)
                    .map(|a| a.frame.clone());
                self.show(next)
            }
            Message::Close => {
                if let Some(w) = &self.shown {
                    self.compositor.close(&w.id);
                }
                self.show(None)
            }
            _ => Task::none(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![events::changes(&self.compositor).map(|_| Message::Changed)];
        if let Some(a) = &self.active {
            subs.push(
                Subscription::run_with((self.compositor.clone(), a.clone()), |(c, a)| {
                    watch_corner(c.clone(), a.clone())
                })
                .map(Message::Near),
            );
            if a.floating {
                subs.push(iced::time::every(Duration::from_secs(1)).map(|_| Message::Changed));
            }
        }
        Subscription::batch(subs)
    }

    pub fn view(&self) -> Element<'_, Message> {
        if self.shown.is_none() {
            return Space::new().into();
        }
        let face = button(
            container(icon(icons::CLOSE, 16))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(0)
        .on_press(Message::Close)
        .style(|_, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered | button::Status::Pressed => {
                    Color::from_rgb8(0xE0, 0x5A, 0x6D)
                }
                _ => Color::from_rgba8(40, 46, 60, 0.92),
            })),
            text_color: Color::WHITE,
            border: Border {
                radius: 9.0.into(),
                width: 1.0,
                color: Color::from_rgba8(255, 255, 255, 0.18),
            },
            ..button::Style::default()
        });
        mouse_area(face)
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    }

    pub fn style(&self, _theme: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: Color::WHITE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame {
            id: "0x1".into(),
            x: 100,
            y: 60,
            width: 1000,
            height: 700,
        }
    }

    #[test]
    fn only_the_top_right_corner_reveals_it() {
        let w = frame();
        assert!(near_corner(&w, (1090, 70)));
        assert!(near_corner(&w, (950, 120)));
        assert!(!near_corner(&w, (500, 70)), "left of the corner");
        assert!(!near_corner(&w, (1090, 300)), "below the corner");
        assert!(!near_corner(&w, (1110, 70)), "outside the window");
    }

    #[test]
    fn the_button_sits_inside_the_corner() {
        assert_eq!(margins(&frame()), (68, 0, 0, 1062));
    }
}
