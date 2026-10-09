//! Right pane: the playing track and how the related list is built.

use gpui_pre::{
    Context, FontWeight, IntoElement, ParentElement, Styled, Window, div, prelude::*, px,
};
use mkit::button::{Button, Variant};
use mkit::core::theme::Theme;
use related_core::SourceId;

use super::{heading, muted, subtle_border};
use crate::icons::{self, icon};
use crate::player::Player;

impl Player {
    pub(super) fn render_now_playing(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let playback = self.playback.as_ref();
        let track = playback.and_then(|p| p.track.clone());
        let art = playback.and_then(|p| p.large_art.clone());
        let following_other = !self.follow
            && track.as_ref().zip(self.seed.as_ref()).is_some_and(|(a, b)| a.id != b.id);

        let mut panel = div()
            .id("now-playing")
            .w(px(304.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(t.spacing.large))
            .p(px(t.spacing.large))
            .bg(t.colors.surface)
            .border_l_1()
            .border_color(t.colors.border)
            .overflow_y_scroll()
            .child(self.cover(art.as_deref(), 272., t.radii.large, &t));

        panel = match &track {
            Some(track) => {
                let explore = track.clone();
                panel
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(t.spacing.xsmall))
                            .child(
                                heading(track.name.clone(), t.typography.heading_small)
                                    .line_clamp(2),
                            )
                            .child(
                                div()
                                    .text_size(px(t.typography.caption))
                                    .text_color(t.colors.text_muted)
                                    .line_clamp(2)
                                    .child(format!("{} · {}", track.artist_line(), track.album)),
                            ),
                    )
                    .when(following_other, |el| {
                        el.child(
                            Button::new("Show related to this song")
                                .id(3001)
                                .variant(Variant::Outline)
                                .leading(icon(&icons::COMPASS, 14., t.colors.text))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_follow(true, cx);
                                    this.set_seed(explore.clone(), cx);
                                })),
                        )
                    })
            }
            None => panel.child(div().text_color(t.colors.text_muted).child(
                "Nothing is playing. Start Spotify on any device, or play a song from search.",
            )),
        };

        panel
            .child(div().h(px(1.)).bg(t.colors.border))
            .child(self.render_sources_legend(&t))
            .child(
                div()
                    .mt_auto()
                    .flex()
                    .justify_between()
                    .items_center()
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .child("Signed in to Spotify")
                    .child(
                        Button::new("Sign out")
                            .id(3002)
                            .variant(Variant::Ghost)
                            .size(mkit::button::Size::Small)
                            .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
                    ),
            )
    }

    fn render_sources_legend(&self, t: &Theme) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(t.spacing.small));
        for source in SourceId::ALL {
            let enabled = source != SourceId::LastFm || self.has_lastfm();
            let short = match source {
                SourceId::LastFm => "LFM",
                SourceId::ListenBrainz => "LB",
                SourceId::Deezer => "DZ",
            };
            list = list.child(
                div()
                    .flex()
                    .gap(px(t.spacing.small))
                    .child(
                        div()
                            .h(px(18.))
                            .px(px(5.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .rounded(px(t.radii.small))
                            .bg(muted(t))
                            .font_family(super::MONO)
                            .text_size(px(10.))
                            .child(short),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(if enabled {
                                        t.colors.text
                                    } else {
                                        t.colors.disabled
                                    })
                                    .child(if enabled {
                                        source.label().to_owned()
                                    } else {
                                        format!("{} (no key)", source.label())
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(t.typography.caption))
                                    .text_color(t.colors.text_muted)
                                    .child(source.description()),
                            ),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(t.spacing.medium))
            .child(div().font_weight(FontWeight::MEDIUM).child("How picks are ranked"))
            .child(
                div()
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .child("Each source ranks tracks its own way. Tracks several sources agree on rise to the top, and no artist fills more than two spots."),
            )
            .child(list)
            .child(
                div()
                    .p(px(t.spacing.medium))
                    .rounded(px(t.radii.medium))
                    .border_1()
                    .border_color(subtle_border(t))
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .child("Keys: ↑↓ select · Enter play · Q queue · E explore · Space play/pause"),
            )
    }
}
