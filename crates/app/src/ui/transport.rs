//! Top toolbar: transport, seek, volume, shuffle and repeat.

use gpui_pre::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use mkit::core::theme::Theme;
use mkit::icon_button::{IconButton, Size, Variant};
use related_core::RepeatMode;

use super::{MONO, format_ms};
use crate::icons::{self, icon};
use crate::player::Player;

impl Player {
    pub(super) fn render_transport(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let playback = self.playback.as_ref();
        let playing = playback.is_some_and(|p| p.is_playing);
        let has_track = playback.is_some_and(|p| p.track.is_some());
        let shuffle = playback.is_some_and(|p| p.shuffle);
        let repeat = playback.map_or(RepeatMode::Off, |p| p.repeat);
        let duration = playback.and_then(|p| p.track.as_ref()).map_or(0, |t| t.duration_ms);
        let position = self.progress_ms();
        let volume = playback.and_then(|p| p.device.as_ref()?.volume_percent);

        let divider = || div().w(px(1.)).h(px(24.)).mx(px(t.spacing.xsmall)).bg(t.colors.border);
        let time = |text: String| {
            div()
                .w(px(44.))
                .flex_none()
                .font_family(MONO)
                .text_size(px(t.typography.caption))
                .text_color(t.colors.text_muted)
                .child(text)
        };

        let (play_glyph, play_label) =
            if playing { (&icons::PAUSE, "Pause") } else { (&icons::PLAY, "Play") };
        let repeat_label = match repeat {
            RepeatMode::Off => "Repeat off",
            RepeatMode::Context => "Repeat all",
            RepeatMode::Track => "Repeat one",
        };
        let toggled = |on: bool| if on { Variant::Secondary } else { Variant::Ghost };
        let tint = |on: bool| if on { t.colors.text } else { t.colors.text_muted };

        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(t.spacing.small))
            .px(px(t.spacing.medium))
            .bg(t.colors.surface)
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.xsmall))
                    .child(
                        IconButton::new(
                            "Previous track",
                            icon(&icons::SKIP_BACK, 16., t.colors.text),
                        )
                        .id(1001)
                        .variant(Variant::Ghost)
                        .size(Size::Small)
                        .disabled(!has_track)
                        .on_click(cx.listener(|this, _, _, cx| this.previous(cx))),
                    )
                    .child(
                        IconButton::new(play_label, icon(play_glyph, 16., t.colors.accent_text))
                            .id(1002)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
                    )
                    .child(
                        IconButton::new(
                            "Next track",
                            icon(&icons::SKIP_FORWARD, 16., t.colors.text),
                        )
                        .id(1003)
                        .variant(Variant::Ghost)
                        .size(Size::Small)
                        .disabled(!has_track)
                        .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
                    ),
            )
            .child(divider())
            .child(time(format_ms(position)).flex().justify_end())
            .child(div().flex_1().min_w(px(120.)).child(self.seek.clone()))
            .child(time(format_ms(duration)))
            .child(divider())
            .child(
                IconButton::new("Shuffle", icon(&icons::SHUFFLE, 16., tint(shuffle)))
                    .id(1004)
                    .variant(toggled(shuffle))
                    .size(Size::Small)
                    .disabled(!has_track)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_shuffle(cx))),
            )
            .child(
                IconButton::new(
                    repeat_label,
                    icon(
                        if repeat == RepeatMode::Track {
                            &icons::REPEAT_ONE
                        } else {
                            &icons::REPEAT
                        },
                        16.,
                        tint(repeat != RepeatMode::Off),
                    ),
                )
                .id(1005)
                .variant(toggled(repeat != RepeatMode::Off))
                .size(Size::Small)
                .disabled(!has_track)
                .on_click(cx.listener(|this, _, _, cx| this.cycle_repeat(cx))),
            )
            .child(divider())
            .child(icon(&icons::VOLUME, 16., t.colors.text_muted))
            .child(div().w(px(96.)).flex_none().child(self.volume.clone()))
            .child(
                div()
                    .w(px(36.))
                    .flex_none()
                    .font_family(MONO)
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .child(volume.map_or("–".to_owned(), |v| format!("{v}%"))),
            )
    }
}
