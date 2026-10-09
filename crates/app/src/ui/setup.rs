//! First-run screen: connect a Spotify app and, optionally, Last.fm.

use gpui_pre::{
    ClipboardItem, Context, FontWeight, IntoElement, ParentElement, Styled, Window, div,
    prelude::*, px,
};
use mkit::button::{Button, Variant};
use mkit::core::theme::Theme;
use mkit::inline_alert::Severity;

use super::{MONO, heading, subtle_border};
use crate::player::Player;

impl Player {
    pub(super) fn render_setup(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let redirect = self.config.redirect_uri();
        let waiting = self.signing_in.is_some();

        let step = |n: &'static str, title: &'static str, body: gpui_pre::AnyElement| {
            div()
                .flex()
                .gap(px(t.spacing.medium))
                .child(
                    div()
                        .size(px(24.))
                        .flex_none()
                        .rounded(px(t.radii.pill))
                        .bg(super::muted(&t))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(t.typography.caption))
                        .font_weight(FontWeight::MEDIUM)
                        .child(n),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(t.spacing.small))
                        .child(div().font_weight(FontWeight::MEDIUM).child(title))
                        .child(body),
                )
        };

        let redirect_copy = redirect.clone();
        let step_one = div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.small))
            .text_color(t.colors.text_muted)
            .child("Create an app in the Spotify developer dashboard. Add this redirect URI and tick Web API.")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.small))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(t.controls.small))
                            .px(px(t.spacing.medium))
                            .flex()
                            .items_center()
                            .rounded(px(t.radii.medium))
                            .border_1()
                            .border_color(subtle_border(&t))
                            .bg(t.colors.background)
                            .font_family(MONO)
                            .text_size(px(t.typography.caption))
                            .text_color(t.colors.text)
                            .child(redirect),
                    )
                    .child(
                        Button::new("Copy")
                            .id(9001)
                            .variant(Variant::Outline)
                            .size(mkit::button::Size::Small)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(redirect_copy.clone()))
                            }),
                    )
                    .child(
                        Button::new("Open dashboard")
                            .id(9002)
                            .variant(Variant::Outline)
                            .size(mkit::button::Size::Small)
                            .trailing(crate::icons::icon(&crate::icons::EXTERNAL, 14., t.colors.text))
                            .on_click(cx.listener(|this, _, _, cx| this.open_dashboard(cx))),
                    ),
            );

        let actions = if waiting {
            div()
                .flex()
                .items_center()
                .gap(px(t.spacing.medium))
                .child(Button::new("Waiting for Spotify").id(9003).loading(true).disabled(true))
                .child(
                    Button::new("Cancel")
                        .id(9004)
                        .variant(Variant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))),
                )
                .child(
                    div()
                        .text_size(px(t.typography.caption))
                        .text_color(t.colors.text_muted)
                        .child("Finish signing in in your browser."),
                )
        } else {
            div().flex().child(
                Button::new("Sign in with Spotify")
                    .id(9005)
                    .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
            )
        };

        let card = div()
            .w(px(560.))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(t.spacing.xlarge))
            .p(px(t.spacing.xlarge))
            .rounded(px(t.radii.large))
            .bg(t.colors.surface)
            .border_1()
            .border_color(subtle_border(&t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.xsmall))
                    .child(heading("Connect Spotify", t.typography.heading))
                    .child(div().text_color(t.colors.text_muted).child(
                        "spotify-related plays Spotify in this app, or on any of your devices, and finds related tracks from Last.fm, ListenBrainz and Deezer. Playback needs Spotify Premium.",
                    )),
            )
            .child(step("1", "Create a Spotify app", step_one.into_any_element()))
            .child(step(
                "2",
                "Paste its client ID",
                div().child(self.client_id_field.clone()).into_any_element(),
            ))
            .child(step(
                "3",
                "Add a Last.fm key for better picks",
                div().child(self.lastfm_field.clone()).into_any_element(),
            ))
            .when_some(self.setup_error.clone(), |el, message| {
                el.child(alert(Severity::Error, message, &t))
            })
            .child(actions);

        div()
            .id("setup")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .items_center()
            .justify_center()
            .p(px(t.spacing.xxlarge))
            .child(card)
    }
}

/// A static inline alert. mkit's `InlineAlert` is an entity with dismiss and
/// action events; a sign-in error needs neither, so this draws the same look
/// from the same tokens.
fn alert(severity: Severity, message: String, t: &Theme) -> impl IntoElement {
    let color = match severity {
        Severity::Error => t.colors.danger,
        Severity::Warning => t.colors.warning,
        Severity::Success => t.colors.success,
        Severity::Info => t.colors.text_muted,
    };
    div()
        .flex()
        .items_start()
        .gap(px(t.spacing.small))
        .p(px(t.spacing.medium))
        .rounded(px(t.radii.medium))
        .border_1()
        .border_color(color)
        .text_color(color)
        .child(crate::icons::icon(&crate::icons::ALERT, 16., color))
        .child(div().flex_1().min_w_0().child(message))
}
