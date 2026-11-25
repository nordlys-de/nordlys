//! A close button for the active window. Apps like terminals draw no
//! titlebar, so this is a small layer surface that appears inside the
//! window's top-right corner when the pointer goes near it, and is a
//! single clear pixel the rest of the time. Hyprland only.

use crate::icons::{self, icon};
use crate::sources::{Compositor, Frame};
use iced::alignment::{Horizontal, Vertical};
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

pub struct Closer {
    compositor: Compositor,
    shown: Option<Frame>,
}

#[to_layer_message]
#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Close,
}

/// Whether the pointer is in the window's top-right corner.
fn near_corner(w: &Frame, (x, y): (i32, i32)) -> bool {
    let right = w.x + w.width;
    x >= right - REACH_X.min(w.width) && x <= right && y >= w.y && y <= w.y + REACH_Y.min(w.height)
}

/// Layer margins (top, right, bottom, left) that put the button there.
fn margins(w: &Frame) -> (i32, i32, i32, i32) {
    let left = w.x + w.width - SIZE as i32 - INSET;
    ((w.y + INSET).max(0), 0, 0, left.max(0))
}

impl Closer {
    pub fn new(compositor: Compositor) -> Self {
        Self {
            compositor,
            shown: None,
        }
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
            Message::Tick => {
                let next = self
                    .compositor
                    .active_frame()
                    .filter(|(w, pointer)| near_corner(w, *pointer))
                    .map(|(w, _)| w);
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
        iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick)
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
