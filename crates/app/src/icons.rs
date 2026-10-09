//! Lucide-style icons drawn as vector paths, the way mkit draws its own.
//!
//! mkit does not ship an icon set yet, so these follow its rules: a 24-unit
//! grid, 2-unit strokes, `currentColor`-style tinting by the caller, and
//! 16px in controls or 14px in dense rows.

use gpui_pre::{IntoElement, PathBuilder, Rgba, Styled, canvas, point, px};

/// One icon: stroked polylines and circles, plus filled polygons.
pub struct Glyph {
    lines: &'static [&'static [(f32, f32)]],
    circles: &'static [(f32, f32, f32)],
    fills: &'static [&'static [(f32, f32)]],
}

pub fn icon(glyph: &'static Glyph, size: f32, color: impl Into<Rgba>) -> impl IntoElement {
    let color = color.into();
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let unit = bounds.size.width / 24.0;
            let origin = bounds.origin;
            let at = |x: f32, y: f32| origin + point(unit * x, unit * y);

            if !glyph.lines.is_empty() || !glyph.circles.is_empty() {
                let mut path = PathBuilder::stroke(unit * 2.0);
                for &(cx, cy, r) in glyph.circles {
                    let radii = point(unit * r, unit * r);
                    path.move_to(at(cx - r, cy));
                    path.arc_to(radii, px(0.), false, true, at(cx + r, cy));
                    path.arc_to(radii, px(0.), false, true, at(cx - r, cy));
                    path.close();
                }
                for line in glyph.lines {
                    for (i, &(x, y)) in line.iter().enumerate() {
                        if i == 0 { path.move_to(at(x, y)) } else { path.line_to(at(x, y)) }
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
            for polygon in glyph.fills {
                let mut path = PathBuilder::fill();
                for (i, &(x, y)) in polygon.iter().enumerate() {
                    if i == 0 { path.move_to(at(x, y)) } else { path.line_to(at(x, y)) }
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size(px(size))
    .flex_none()
}

const NONE_LINES: &[&[(f32, f32)]] = &[];
const NONE_CIRCLES: &[(f32, f32, f32)] = &[];
const NONE_FILLS: &[&[(f32, f32)]] = &[];

pub static PLAY: Glyph = Glyph {
    lines: NONE_LINES,
    circles: NONE_CIRCLES,
    fills: &[&[(6., 3.), (20., 12.), (6., 21.)]],
};

pub static PAUSE: Glyph = Glyph {
    lines: NONE_LINES,
    circles: NONE_CIRCLES,
    fills: &[
        &[(6., 4.), (10., 4.), (10., 20.), (6., 20.)],
        &[(14., 4.), (18., 4.), (18., 20.), (14., 20.)],
    ],
};

pub static SKIP_BACK: Glyph = Glyph {
    lines: &[&[(19., 20.), (9., 12.), (19., 4.), (19., 20.)], &[(5., 19.), (5., 5.)]],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static SKIP_FORWARD: Glyph = Glyph {
    lines: &[&[(5., 4.), (15., 12.), (5., 20.), (5., 4.)], &[(19., 5.), (19., 19.)]],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static VOLUME: Glyph = Glyph {
    lines: &[
        &[(11., 5.), (6., 9.), (2., 9.), (2., 15.), (6., 15.), (11., 19.), (11., 5.)],
        &[(15.5, 8.5), (17., 10.), (17.5, 12.), (17., 14.), (15.5, 15.5)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static SHUFFLE: Glyph = Glyph {
    lines: &[
        &[(2., 18.), (6., 18.), (14., 6.), (22., 6.)],
        &[(2., 6.), (6., 6.), (8.5, 9.5)],
        &[(15.5, 14.5), (18., 18.), (22., 18.)],
        &[(18., 2.), (22., 6.), (18., 10.)],
        &[(18., 14.), (22., 18.), (18., 22.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static REPEAT: Glyph = Glyph {
    lines: &[
        &[(17., 2.), (21., 6.), (17., 10.)],
        &[(3., 11.), (3., 8.), (5., 6.), (21., 6.)],
        &[(7., 22.), (3., 18.), (7., 14.)],
        &[(21., 13.), (21., 16.), (19., 18.), (3., 18.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static REPEAT_ONE: Glyph = Glyph {
    lines: &[
        &[(17., 2.), (21., 6.), (17., 10.)],
        &[(3., 11.), (3., 8.), (5., 6.), (21., 6.)],
        &[(7., 22.), (3., 18.), (7., 14.)],
        &[(21., 13.), (21., 16.), (19., 18.), (3., 18.)],
        &[(11., 10.), (12., 9.), (12., 15.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

/// "Add to queue": a list with a plus.
pub static LIST_PLUS: Glyph = Glyph {
    lines: &[
        &[(3., 6.), (14., 6.)],
        &[(3., 12.), (14., 12.)],
        &[(3., 18.), (10., 18.)],
        &[(18., 13.), (18., 21.)],
        &[(14., 17.), (22., 17.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

/// "Explore related": a compass.
pub static COMPASS: Glyph = Glyph {
    lines: &[&[(16.2, 7.8), (14.1, 14.1), (7.8, 16.2), (9.9, 9.9), (16.2, 7.8)]],
    circles: &[(12., 12., 10.)],
    fills: NONE_FILLS,
};

/// Radio mode: broadcast waves around a dot.
pub static RADIO: Glyph = Glyph {
    lines: &[
        &[(4.9, 19.1), (2.9, 15.9), (2., 12.), (2.9, 8.1), (4.9, 4.9)],
        &[(7.8, 16.2), (6.6, 14.3), (6., 12.), (6.6, 9.7), (7.8, 7.8)],
        &[(16.2, 7.8), (17.4, 9.7), (18., 12.), (17.4, 14.3), (16.2, 16.2)],
        &[(19.1, 4.9), (21.1, 8.1), (22., 12.), (21.1, 15.9), (19.1, 19.1)],
    ],
    circles: &[(12., 12., 2.)],
    fills: NONE_FILLS,
};

pub static REFRESH: Glyph = Glyph {
    lines: &[
        &[(3., 12.), (4.2, 7.5), (7.5, 4.2), (12., 3.), (16.5, 4.2), (19.4, 6.8), (21., 8.)],
        &[(21., 3.), (21., 8.), (16., 8.)],
        &[(21., 12.), (19.8, 16.5), (16.5, 19.8), (12., 21.), (7.5, 19.8), (4.6, 17.2), (3., 16.)],
        &[(8., 16.), (3., 16.), (3., 21.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};

pub static SPEAKER: Glyph = Glyph {
    lines: &[
        &[
            (6., 2.),
            (18., 2.),
            (20., 4.),
            (20., 20.),
            (18., 22.),
            (6., 22.),
            (4., 20.),
            (4., 4.),
            (6., 2.),
        ],
        &[(12., 6.), (12., 6.01)],
    ],
    circles: &[(12., 14., 4.)],
    fills: NONE_FILLS,
};

pub static MUSIC: Glyph = Glyph {
    lines: &[&[(9., 18.), (9., 5.), (21., 3.), (21., 16.)]],
    circles: &[(6., 18., 3.), (18., 16., 3.)],
    fills: NONE_FILLS,
};

pub static ALERT: Glyph = Glyph {
    lines: &[&[(12., 8.), (12., 12.)], &[(12., 16.), (12.01, 16.)]],
    circles: &[(12., 12., 10.)],
    fills: NONE_FILLS,
};

pub static EXTERNAL: Glyph = Glyph {
    lines: &[
        &[(15., 3.), (21., 3.), (21., 9.)],
        &[(10., 14.), (21., 3.)],
        &[(18., 13.), (18., 19.), (16., 21.), (5., 21.), (3., 19.), (3., 8.), (5., 6.), (11., 6.)],
    ],
    circles: NONE_CIRCLES,
    fills: NONE_FILLS,
};
