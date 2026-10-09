//! Left pane: search, recently explored seeds, and Spotify Connect devices.

use gpui_pre::{
    Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*,
    px,
};
use mkit::button::{Button, Size, Variant};
use mkit::core::theme::Theme;
use related_core::Track;

use super::{muted, section_label};
use crate::icons::{self, icon};
use crate::local::LocalState;
use crate::player::Player;

impl Player {
    pub(super) fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let searching = !self.search_query.is_empty();
        let seed_id = self.seed.as_ref().map(|s| s.id.clone());

        let mut list = div().flex().flex_col().gap(px(1.));
        if searching {
            list = list.child(section_label("Results", &t));
            if self.searching && self.search_results.is_empty() {
                list = list.child(hint("Searching", &t));
            } else if self.search_results.is_empty() {
                list = list.child(hint("No results found.", &t));
            }
            for (i, track) in self.search_results.clone().into_iter().enumerate() {
                list =
                    list.child(self.track_row(("search", i), track, seed_id.as_deref(), true, cx));
            }
        } else {
            list = list.child(section_label("Explored", &t));
            if self.history.is_empty() {
                list = list.child(hint("Songs you play or explore appear here.", &t));
            }
            for (i, track) in self.history.clone().into_iter().enumerate() {
                list = list.child(self.track_row(
                    ("history", i),
                    track,
                    seed_id.as_deref(),
                    false,
                    cx,
                ));
            }
        }

        let mut devices = div().flex().flex_col().gap(px(1.)).child(section_label("Devices", &t));
        if let Some(card) = self.render_local_card(cx) {
            devices = devices.child(card);
        }
        if self.devices.is_empty() && self.local.is_none() {
            devices = devices.child(hint("Open Spotify on a computer, phone or speaker.", &t));
        }
        for (i, device) in self.devices.clone().into_iter().enumerate() {
            let active = device.is_active;
            let caption = if active {
                "Active".to_owned()
            } else if self.is_local_device(&device) {
                "This computer".to_owned()
            } else {
                device.kind.clone()
            };
            devices = devices.child(
                div()
                    .id(("device", i))
                    .h(px(t.controls.small))
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.small))
                    .px(px(t.spacing.small))
                    .rounded(px(t.radii.small))
                    .cursor_pointer()
                    .hover(|s| s.bg(muted(&t)))
                    .when(active, |el| el.bg(muted(&t)))
                    .child(icon(
                        &icons::SPEAKER,
                        14.,
                        if active { t.colors.text } else { t.colors.text_muted },
                    ))
                    .child(div().flex_1().min_w_0().truncate().child(device.name.clone()))
                    .child(
                        div()
                            .text_size(px(t.typography.caption))
                            .text_color(if active { t.colors.success } else { t.colors.text_muted })
                            .child(caption),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.transfer_to(&device, cx))),
            );
        }

        div()
            .w(px(272.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(t.spacing.medium))
            .p(px(t.spacing.medium))
            .bg(t.colors.surface)
            .border_r_1()
            .border_color(t.colors.border)
            .child(self.search.clone())
            .child(div().id("sidebar-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .child(
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(t.colors.border)
                    .pt(px(t.spacing.small))
                    .child(devices),
            )
    }

    /// Status of the built-in player while it is not simply online: connecting,
    /// waiting for permission, or failed. Once online it is an ordinary device row.
    fn render_local_card(&self, cx: &mut Context<Self>) -> Option<gpui_pre::AnyElement> {
        let t = *cx.global::<Theme>();
        let name = self.config.local_playback.device_name.clone();
        let online = self.local_state == LocalState::Ready
            && self.devices.iter().any(|d| self.is_local_device(d));
        let (dot, title, detail, action): (
            _,
            String,
            Option<String>,
            Option<gpui_pre::AnyElement>,
        ) = match &self.local_state {
            LocalState::Off => return None,
            _ if online => return None,
            LocalState::Starting | LocalState::Connecting | LocalState::Ready => {
                (t.colors.warning, "Connecting".to_owned(), None, None)
            }
            LocalState::Reconnecting => (t.colors.warning, "Reconnecting".to_owned(), None, None),
            LocalState::NeedsAuthorization(reason) => {
                let action = if self.authorizing_local.is_some() {
                    div()
                        .flex()
                        .gap(px(t.spacing.small))
                        .child(
                            Button::new("Waiting for browser")
                                .id(4001)
                                .size(Size::Small)
                                .loading(true)
                                .disabled(true),
                        )
                        .child(
                            Button::new("Cancel")
                                .id(4002)
                                .variant(Variant::Ghost)
                                .size(Size::Small)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_authorize_local(cx)),
                                ),
                        )
                        .into_any_element()
                } else {
                    Button::new("Allow playback here")
                        .id(4003)
                        .size(Size::Small)
                        .on_click(cx.listener(|this, _, _, cx| this.authorize_local(cx)))
                        .into_any_element()
                };
                let detail = match reason {
                    Some(r) => format!(
                        "Spotify did not accept the app's sign-in for audio ({r}). Approve playback once in your browser."
                    ),
                    None => {
                        "Approve playback once in your browser to play audio in this app.".into()
                    }
                };
                (t.colors.warning, "Needs permission".to_owned(), Some(detail), Some(action))
            }
            LocalState::Failed(e) => (
                t.colors.danger,
                "Unavailable".to_owned(),
                Some(e.clone()),
                Some(
                    Button::new("Retry")
                        .id(4004)
                        .variant(Variant::Outline)
                        .size(Size::Small)
                        .on_click(cx.listener(|this, _, _, cx| this.restart_local(cx)))
                        .into_any_element(),
                ),
            ),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(t.spacing.small))
                .p(px(t.spacing.small))
                .rounded(px(t.radii.small))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(t.spacing.small))
                        .child(icon(&icons::SPEAKER, 14., t.colors.text_muted))
                        .child(div().flex_1().min_w_0().truncate().child(name))
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(t.typography.caption))
                                .text_color(dot)
                                .child(title),
                        ),
                )
                .when_some(detail, |el, d| {
                    el.child(
                        div()
                            .text_size(px(t.typography.caption))
                            .text_color(t.colors.text_muted)
                            .child(d),
                    )
                })
                .when_some(action, |el, a| el.child(a))
                .into_any_element(),
        )
    }

    /// A compact track row. Click explores its related tracks; the buttons
    /// play it or add it to the queue.
    fn track_row(
        &self,
        id: (&'static str, usize),
        track: Track,
        seed_id: Option<&str>,
        show_queue: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let is_seed = seed_id == Some(track.id.as_str());
        let group: SharedString = format!("{}-{}", id.0, id.1).into();
        let play_track = track.clone();
        let queue_track = track.clone();
        let explore_track = track.clone();
        div()
            .id(id)
            .group(group.clone())
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(t.spacing.small))
            .px(px(t.spacing.small))
            .rounded(px(t.radii.small))
            .cursor_pointer()
            .hover(|s| s.bg(muted(&t)))
            .when(is_seed, |el| el.bg(muted(&t)))
            .child(self.cover(track.album_art.as_deref(), 32., t.radii.small, &t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .when(is_seed, |el| el.font_weight(FontWeight::MEDIUM))
                            .child(track.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(t.typography.caption))
                            .text_color(t.colors.text_muted)
                            .child(track.artist_line()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.))
                    .opacity(0.)
                    .group_hover(group, |s| s.opacity(1.))
                    .child(mini_button(("play", id.1), &icons::PLAY, "Play", &t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.play_track(&play_track, cx)
                        }),
                    ))
                    .when(show_queue, |el| {
                        el.child(
                            mini_button(("queue", id.1), &icons::LIST_PLUS, "Add to queue", &t)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.queue_track(&queue_track, cx)
                                })),
                        )
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.explore(explore_track.clone(), cx)))
    }
}

fn hint(text: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .px(px(t.spacing.small))
        .py(px(t.spacing.xsmall))
        .text_size(px(t.typography.caption))
        .text_color(t.colors.text_muted)
        .child(text)
}

/// A 24px ghost icon button for dense rows, where mkit's smallest
/// IconButton (32px) would not fit. Same tokens, one size down.
pub(super) fn mini_button(
    id: (&'static str, usize),
    glyph: &'static icons::Glyph,
    label: &'static str,
    t: &Theme,
) -> gpui_pre::Stateful<gpui_pre::Div> {
    let hover = muted(t);
    let border = t.colors.border;
    div()
        .id(id)
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(t.radii.small))
        .cursor_pointer()
        .hover(move |s| s.bg(hover).border_1().border_color(border))
        .tooltip(move |_, cx| cx.new(|_| TooltipText(label)).into())
        .child(icon(glyph, 14., t.colors.text))
}

struct TooltipText(&'static str);

impl gpui_pre::Render for TooltipText {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        // mkit tooltip look: accent fill, caption text.
        div()
            .px(px(t.spacing.medium))
            .py(px(6.))
            .rounded(px(t.radii.medium))
            .bg(t.colors.accent)
            .text_color(t.colors.accent_text)
            .text_size(px(t.typography.caption))
            .font_family(super::FONT)
            .child(self.0)
    }
}
