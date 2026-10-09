//! The center pane: related tracks for the seed, with where each pick came from.

use gpui_pre::{
    Context, FontWeight, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*,
    px, relative, uniform_list,
};
use mkit::badge::{Badge, Status};
use mkit::button::{Button, Variant};
use mkit::core::theme::Theme;
use mkit::empty_state::EmptyState;
use mkit::icon_button::{self, IconButton};
use mkit::progress::Progress;
use mkit::tooltip::Tooltip;
use related_core::related::{Resolution, SourceHit};
use related_core::{RelatedTrack, SourceId};

use super::sidebar::mini_button;
use super::{
    ExploreSelected, LIST_CONTEXT, MONO, PlaySelected, QueueSelected, SelectNext, SelectPrevious,
    TogglePlay, format_ms, heading, muted,
};
use crate::icons::{self, icon};
use crate::player::{Player, art_url, seed_track};

const ROW_HEIGHT: f32 = 44.;
/// Column widths: number, cover, (title flexible), artist share, sources, match, length, actions.
const NUM_W: f32 = 28.;
const SOURCES_W: f32 = 92.;
const MATCH_W: f32 = 64.;
const LENGTH_W: f32 = 48.;
const ACTIONS_W: f32 = 80.;

impl Player {
    pub(super) fn render_related(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_related_header(cx))
            .child(self.render_related_body(cx))
    }

    fn render_related_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let title: SharedString = match &self.seed {
            Some(seed) => format!("{} · {}", seed.name, seed.artist_line()).into(),
            None => "Nothing to relate yet".into(),
        };
        let playable = self
            .report
            .as_ref()
            .map_or(0, |r| r.tracks.iter().filter(|t| t.track().is_some()).count());

        let mut sources = div().flex().items_center().gap(px(t.spacing.xsmall)).flex_wrap();
        if let Some(report) = &self.report {
            for status in &report.sources {
                let (badge_status, text) = match &status.outcome {
                    Ok(0) => (Status::Warning, format!("{} · none", status.source.label())),
                    Ok(n) => (Status::Success, format!("{} · {n}", status.source.label())),
                    Err(_) => (Status::Danger, format!("{} · failed", status.source.label())),
                };
                let tip = match &status.outcome {
                    Ok(_) => format!("{} ({} ms)", status.source.description(), status.elapsed_ms),
                    Err(e) => e.clone(),
                };
                sources = sources.child(Tooltip::new(tip, Badge::status(badge_status, text)));
            }
        }
        if !self.has_lastfm() {
            sources = sources.child(Tooltip::new(
                "Add a Last.fm API key in config.toml for track-level similarity",
                Badge::status(Status::Neutral, "Last.fm · no key"),
            ));
        }

        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(t.spacing.medium))
            .px(px(t.spacing.large))
            .pt(px(t.spacing.large))
            .pb(px(t.spacing.medium))
            .border_b_1()
            .border_color(t.colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.medium))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(t.typography.caption))
                                    .text_color(t.colors.text_muted)
                                    .child(if self.follow {
                                        "Related to now playing"
                                    } else {
                                        "Related to pinned song"
                                    }),
                            )
                            .child(heading(title, t.typography.heading).truncate()),
                    )
                    .child(self.seed_mode.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.small))
                    .child(
                        Button::new("Play all")
                            .id(2001)
                            .leading(icon(&icons::PLAY, 14., t.colors.accent_text))
                            .disabled(playable == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.play_all_related(cx))),
                    )
                    .child(
                        Button::new(if self.radio { "Radio on" } else { "Radio" })
                            .id(2002)
                            .variant(if self.radio { Variant::Secondary } else { Variant::Outline })
                            .leading(icon(
                                &icons::RADIO,
                                14.,
                                if self.radio { t.colors.success } else { t.colors.text },
                            ))
                            .aria_label("Radio mode: keep queueing related tracks")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let on = !this.radio;
                                this.set_radio(on, cx)
                            })),
                    )
                    .child(
                        IconButton::new(
                            "Refresh related tracks",
                            icon(&icons::REFRESH, 16., t.colors.text),
                        )
                        .id(2003)
                        .variant(icon_button::Variant::Ghost)
                        .size(icon_button::Size::Small)
                        .disabled(self.seed.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_related(cx))),
                    )
                    .child(div().flex_1())
                    .child(sources),
            )
    }

    fn render_related_body(&mut self, cx: &mut Context<Self>) -> gpui_pre::AnyElement {
        let t = *cx.global::<Theme>();
        let centered = |el: gpui_pre::AnyElement| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p(px(t.spacing.xxlarge))
                .child(el)
                .into_any_element()
        };
        if self.seed.is_none() {
            return centered(
                EmptyState::new("Nothing playing")
                    .icon(icon(&icons::MUSIC, 20., t.colors.text_muted))
                    .description("Play something in Spotify, or search for a song and pick it to see related tracks.")
                    .into_any_element(),
            );
        }
        if self.loading_related {
            return centered(
                div()
                    .w(px(280.))
                    .flex()
                    .flex_col()
                    .gap(px(t.spacing.medium))
                    .child(
                        div()
                            .text_color(t.colors.text_muted)
                            .child("Asking Last.fm, ListenBrainz and Deezer"),
                    )
                    .child(Progress::new("Loading related tracks", 0.).indeterminate())
                    .into_any_element(),
            );
        }
        let Some(report) = &self.report else { return div().into_any_element() };
        if report.tracks.is_empty() {
            let failed = report.sources.iter().all(|s| s.outcome.is_err());
            return centered(
                EmptyState::new(if failed { "Sources are unreachable" } else { "No related tracks found" })
                    .icon(icon(&icons::ALERT, 20., t.colors.text_muted))
                    .description(if failed {
                        "Check your connection, then refresh."
                    } else {
                        "This song is too new or rare for the sources. Add a Last.fm key for wider coverage, or explore another song."
                    })
                    .action(
                        Button::new("Refresh")
                            .id(2004)
                            .variant(Variant::Outline)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_related(cx))),
                    )
                    .into_any_element(),
            );
        }

        let count = report.tracks.len();
        let header = div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(t.spacing.small))
            .text_size(px(t.typography.caption))
            .text_color(t.colors.text_muted)
            .border_b_1()
            .border_color(t.colors.border)
            .px(px(t.spacing.large))
            .child(div().w(px(2.)))
            .child(div().w(px(NUM_W - 10.)).flex_none().child("#"))
            .child(div().w(px(32.)))
            .child(div().flex_1().min_w_0().child("Title"))
            .child(div().w(relative(0.22)).min_w_0().child("Artist"))
            .child(div().w(px(SOURCES_W)).flex_none().child("Sources"))
            .child(div().w(px(MATCH_W)).flex_none().child("Match"))
            .child(div().w(px(LENGTH_W)).flex_none().flex().justify_end().child("Length"))
            .child(div().w(px(ACTIONS_W)).flex_none());

        let list = uniform_list(
            "related-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                range.map(|i| this.render_related_row(i, cx).into_any_element()).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.list_scroll)
        .flex_1()
        .px(px(t.spacing.small))
        .py(px(t.spacing.xsmall));

        div()
            .id("related-body")
            .key_context(LIST_CONTEXT)
            .track_focus(&self.list_focus)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| this.toggle_play(cx)))
            .on_action(cx.listener(|this, _: &PlaySelected, _, cx| {
                if let Some(track) = this.selected_track() {
                    this.play_track(&track, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &QueueSelected, _, cx| {
                if let Some(track) = this.selected_track() {
                    this.queue_track(&track, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ExploreSelected, _, cx| {
                if let Some(track) = this.selected_seed() {
                    this.explore(track, cx);
                }
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .into_any_element()
    }

    fn render_related_row(&mut self, i: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let Some(row) = self.report.as_ref().and_then(|r| r.tracks.get(i)).cloned() else {
            return div().id(("row", i)).h(px(ROW_HEIGHT));
        };
        let selected = self.selected == Some(i);
        let now_playing = self
            .playback
            .as_ref()
            .and_then(|p| p.track.as_ref())
            .zip(row.track())
            .is_some_and(|(a, b)| a.id == b.id);
        let spotify = row.track().cloned();
        let available = spotify.is_some();
        let group: SharedString = format!("related-row-{i}").into();
        let dim = |c| if row.spotify == Resolution::NotOnSpotify { t.colors.disabled } else { c };

        let caption = match &row.spotify {
            Resolution::Found(track) => track.album.clone(),
            Resolution::Pending if self.demo => row.album.clone().unwrap_or_default(),
            Resolution::Pending => "Finding on Spotify".into(),
            Resolution::NotOnSpotify => "Not on Spotify".into(),
        };
        let length = spotify.as_ref().map_or(String::new(), |s| format_ms(s.duration_ms));

        let actions = {
            let (play, queue, explore) = (spotify.clone(), spotify.clone(), seed_track(&row));
            div()
                .w(px(ACTIONS_W))
                .flex_none()
                .flex()
                .justify_end()
                .gap(px(2.))
                .when(!selected, |el| el.opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)))
                .when(available, |el| {
                    el.child(mini_button(("rplay", i), &icons::PLAY, "Play", &t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(track) = &play {
                                this.play_track(track, cx);
                            }
                        }),
                    ))
                    .child(
                        mini_button(("rqueue", i), &icons::LIST_PLUS, "Add to queue", &t).on_click(
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(track) = &queue {
                                    this.queue_track(track, cx);
                                }
                            }),
                        ),
                    )
                })
                .child(
                    mini_button(("rexplore", i), &icons::COMPASS, "Explore related", &t).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.explore(explore.clone(), cx);
                        }),
                    ),
                )
        };

        div()
            .id(("row", i))
            .group(group)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .gap(px(t.spacing.small))
            .px(px(t.spacing.small))
            .rounded(px(t.radii.small))
            .when(available, |el| el.cursor_pointer())
            .hover(|s| s.bg(muted(&t)))
            .when(selected, |el| el.bg(muted(&t)))
            // A selection bar as well as the fill, so selection never relies on colour alone.
            .child(
                div()
                    .w(px(2.))
                    .h(px(20.))
                    .rounded(px(1.))
                    .when(selected, |el| el.bg(t.colors.text)),
            )
            .child(
                div()
                    .w(px(NUM_W - 10.))
                    .flex_none()
                    .flex()
                    .font_family(MONO)
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .when(now_playing, |el| el.child(icon(&icons::VOLUME, 14., t.colors.success)))
                    .when(!now_playing, |el| el.child((i + 1).to_string())),
            )
            .child(self.cover(art_url(&row).as_deref(), 32., t.radii.small, &t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_color(dim(t.colors.text))
                            .when(now_playing, |el| el.font_weight(FontWeight::SEMIBOLD))
                            .child(row.title.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(t.typography.caption))
                            .text_color(t.colors.text_muted)
                            .child(caption),
                    ),
            )
            .child(
                div()
                    .w(relative(0.22))
                    .flex_none()
                    .min_w_0()
                    .truncate()
                    .text_color(dim(t.colors.text_muted))
                    .child(row.artist.clone()),
            )
            .child(source_pills(&row, &t))
            .child(match_meter(row.score, &t))
            .child(
                div()
                    .w(px(LENGTH_W))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .font_family(MONO)
                    .text_size(px(t.typography.caption))
                    .text_color(t.colors.text_muted)
                    .child(length),
            )
            .child(actions)
            .on_click(cx.listener(move |this, event: &gpui_pre::ClickEvent, window, cx| {
                window.focus(&this.list_focus, cx);
                this.select(Some(i), cx);
                if event.click_count() >= 2
                    && let Some(track) = this.selected_track()
                {
                    this.play_track(&track, cx);
                }
            }))
    }
}

/// One pill per source that suggested the track, in a fixed order so the
/// columns line up. The tooltip gives the source's own rank.
fn source_pills(row: &RelatedTrack, t: &Theme) -> impl IntoElement {
    let mut pills = div().w(px(SOURCES_W)).flex_none().flex().gap(px(3.));
    for source in SourceId::ALL {
        let hit = row.hits.iter().find(|h| h.source == source).copied();
        let (short, full) = match source {
            SourceId::LastFm => ("LFM", "Last.fm"),
            SourceId::ListenBrainz => ("LB", "ListenBrainz"),
            SourceId::Deezer => ("DZ", "Deezer"),
        };
        let pill = div()
            .h(px(18.))
            .px(px(5.))
            .flex()
            .items_center()
            .rounded(px(t.radii.small))
            .font_family(MONO)
            .text_size(px(10.))
            .child(short);
        let pill = match hit {
            Some(_) => pill.bg(muted(t)).text_color(t.colors.text),
            None => pill.text_color(t.colors.disabled).opacity(0.45),
        };
        let label = match hit {
            Some(SourceHit { rank, similarity: Some(s), .. }) => {
                format!("{full}: #{} · similarity {:.0}%", rank + 1, s * 100.)
            }
            Some(SourceHit { rank, .. }) => format!("{full}: #{}", rank + 1),
            None => format!("Not suggested by {full}"),
        };
        pills = pills.child(Tooltip::new(label, pill));
    }
    pills
}

/// Fused score as a short bar; the best track fills it.
fn match_meter(score: f64, t: &Theme) -> impl IntoElement {
    div().w(px(MATCH_W)).flex_none().flex().items_center().child(
        div().w(px(MATCH_W - 12.)).h(px(4.)).rounded(px(t.radii.pill)).bg(t.colors.border).child(
            div()
                .h_full()
                .w(relative(score.clamp(0.04, 1.) as f32))
                .rounded(px(t.radii.pill))
                .bg(t.colors.accent),
        ),
    )
}
