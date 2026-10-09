//! Rendering for the player window, built from mkit components and tokens.

mod now_playing;
mod related;
mod setup;
mod sidebar;
mod transport;

use gpui_pre::{
    Context, FontWeight, ImageSource, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, Rgba, Styled, Window, actions, div, img, prelude::*, px,
};
use mkit::core::theme::Theme;

use crate::images::ArtState;
use crate::player::{NoticeKind, Player};

pub const FONT: &str = "Geist";
pub const MONO: &str = "Geist Mono";
const KEY_CONTEXT: &str = "Player";
const LIST_CONTEXT: &str = "RelatedList";

actions!(
    player,
    [
        TogglePlay,
        NextTrack,
        PreviousTrack,
        Refresh,
        SelectNext,
        SelectPrevious,
        PlaySelected,
        QueueSelected,
        ExploreSelected
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-right", NextTrack, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-left", PreviousTrack, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-r", Refresh, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-p", TogglePlay, Some(KEY_CONTEXT)),
        KeyBinding::new("space", TogglePlay, Some(LIST_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(LIST_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(LIST_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(LIST_CONTEXT)),
        KeyBinding::new("k", SelectPrevious, Some(LIST_CONTEXT)),
        KeyBinding::new("enter", PlaySelected, Some(LIST_CONTEXT)),
        KeyBinding::new("q", QueueSelected, Some(LIST_CONTEXT)),
        KeyBinding::new("e", ExploreSelected, Some(LIST_CONTEXT)),
    ]
}

impl Render for Player {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let root = div()
            .size_full()
            .bg(t.colors.background)
            .text_color(t.colors.text)
            .text_size(px(t.typography.body))
            .font_family(FONT);
        if !self.is_signed_in() || self.signing_in.is_some() {
            return root.child(self.render_setup(window, cx)).into_any_element();
        }
        root.id("player")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &TogglePlay, _, cx| this.toggle_play(cx)))
            .on_action(cx.listener(|this, _: &NextTrack, _, cx| this.next(cx)))
            .on_action(cx.listener(|this, _: &PreviousTrack, _, cx| this.previous(cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh_related(cx)))
            .flex()
            .flex_col()
            .child(self.render_transport(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_sidebar(window, cx))
                    .child(self.render_related(window, cx))
                    .child(self.render_now_playing(window, cx)),
            )
            .child(self.render_status(cx))
            .into_any_element()
    }
}

impl Player {
    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.global::<Theme>();
        let playback = self.playback.as_ref();
        let (dot, state) = match playback {
            Some(p) if p.is_playing => (t.colors.success, "Playing"),
            Some(p) if p.track.is_some() => (t.colors.warning, "Paused"),
            _ => (t.colors.disabled, "Idle"),
        };
        let device = playback
            .and_then(|p| p.device.as_ref())
            .map(|d| format!("on {}", d.name))
            .unwrap_or_else(|| "No active device".into());
        let resolving = self.report.as_ref().filter(|_| self.resolving).map(|r| {
            let done = r
                .tracks
                .iter()
                .filter(|t| t.spotify != related_core::related::Resolution::Pending)
                .count();
            format!("Finding on Spotify {done}/{}", r.tracks.len())
        });
        let notice = self.notice.clone();
        div()
            .h(px(t.controls.xsmall))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(t.spacing.large))
            .px(px(t.spacing.medium))
            .bg(t.colors.surface)
            .border_t_1()
            .border_color(t.colors.border)
            .text_size(px(t.typography.caption))
            .text_color(t.colors.text_muted)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_color(t.colors.text)
                    .child(div().size(px(6.)).rounded(px(t.radii.pill)).bg(dot))
                    .child(state),
            )
            .child(device)
            .when_some(resolving, |el, text| el.child(text))
            .when_some(notice, |el, n| {
                let color = match n.kind {
                    NoticeKind::Error => t.colors.danger,
                    NoticeKind::Info => t.colors.text_muted,
                };
                el.child(div().ml_auto().min_w_0().truncate().text_color(color).child(n.message))
            })
    }

    /// A square cover, or a quiet placeholder while it loads.
    pub(crate) fn cover(
        &self,
        url: Option<&str>,
        size: f32,
        radius: f32,
        t: &Theme,
    ) -> gpui_pre::AnyElement {
        let base = div().size(px(size)).flex_none().rounded(px(radius)).overflow_hidden();
        match url.and_then(|u| self.art.get(u)) {
            Some(ArtState::Ready(image)) => base
                .child(img(ImageSource::Image(image.clone())).size_full().rounded(px(radius)))
                .into_any_element(),
            _ => base
                .bg(t.colors.elevated_surface)
                .flex()
                .items_center()
                .justify_center()
                .child(crate::icons::icon(
                    &crate::icons::MUSIC,
                    (size * 0.4).clamp(10., 48.),
                    t.colors.disabled,
                ))
                .into_any_element(),
        }
    }
}

/// shadcn "muted": text mixed into the background, for hover and selected rows.
pub fn muted(t: &Theme) -> Rgba {
    mix(t.colors.text, t.colors.background, 0.12)
}

/// Outline for menus, popovers and outline buttons.
pub fn subtle_border(t: &Theme) -> Rgba {
    mix(t.colors.text, t.colors.surface, 0.10)
}

pub fn mix(a: Rgba, b: Rgba, amount: f32) -> Rgba {
    Rgba {
        r: a.r * amount + b.r * (1. - amount),
        g: a.g * amount + b.g * (1. - amount),
        b: a.b * amount + b.b * (1. - amount),
        a: 1.0,
    }
}

pub fn format_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Section label used above sidebar lists.
pub fn section_label(text: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .px(px(t.spacing.small))
        .py(px(t.spacing.xsmall))
        .text_size(px(t.typography.caption))
        .text_color(t.colors.text_muted)
        .child(text)
}

pub fn heading(text: impl Into<gpui_pre::SharedString>, size: f32) -> gpui_pre::Div {
    div().text_size(px(size)).font_weight(FontWeight::SEMIBOLD).child(text.into())
}

#[cfg(test)]
mod tests {
    use super::format_ms;

    #[test]
    fn formats_durations() {
        assert_eq!(format_ms(0), "0:00");
        assert_eq!(format_ms(200_040), "3:20");
        assert_eq!(format_ms(3_725_000), "1:02:05");
    }
}
