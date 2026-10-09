//! spotify-related: a Spotify player built around a better related-tracks list.

mod icons;
mod images;
mod player;
mod ui;

use std::borrow::Cow;

use gpui_platform::application;
use gpui_pre::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use mkit::core::theme::{SHADCN_DARK, set_theme};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    application().run(|cx: &mut App| {
        gpui_kit::init(cx);
        install_fonts(cx);
        set_theme(cx, SHADCN_DARK);
        cx.bind_keys(mkit::button::default_key_bindings());
        cx.bind_keys(mkit::icon_button::default_key_bindings());
        cx.bind_keys(mkit::slider::default_key_bindings());
        cx.bind_keys(mkit::text_field::default_key_bindings());
        cx.bind_keys(mkit::search_field::default_key_bindings());
        cx.bind_keys(mkit::segmented_control::default_key_bindings());
        cx.bind_keys(ui::key_bindings());

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.), px(800.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("spotify-related".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(960.), px(600.))),
            app_id: Some("spotify-related".into()),
            ..WindowOptions::default()
        };
        cx.open_window(options, |window, cx| cx.new(|cx| player::Player::new(window, cx)))
            .expect("open player window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// Geist and Geist Mono, the mkit type families (SIL Open Font License).
fn install_fonts(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::warn!("bundled fonts failed to load, using the system font: {e:#}");
    }
}
