//! The main menu: match setup on the right, the game's pitch on the left,
//! and the online room flow (create a room, share its code, join by code).

use super::*;
use macroquad::models::{Mesh, Vertex, draw_mesh};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// State of the online room flow.
pub enum Lobby {
    Idle,
    Creating(Receiver<Result<net::Room, String>>),
    Hosting(net::Room),
    Joining(Receiver<Result<Peer, String>>, String),
    /// Connected as a guest, waiting for the host's settings.
    Joined(Peer),
    Failed(String),
}

/// Menu state that lives across frames.
pub struct MenuState {
    /// The code being typed into the join boxes.
    pub code: String,
    copied_at: f64,
}

impl MenuState {
    pub fn new() -> MenuState {
        MenuState { code: String::new(), copied_at: -100.0 }
    }
}

pub enum MenuAction {
    None,
    Play,
    Quit,
}

const GOLD_SOFT: Color = Color::new(0.91, 0.78, 0.52, 1.0);
const PALE: Color = Color::new(0.92, 0.93, 0.95, 1.0);
const MUTED: Color = Color::new(0.60, 0.65, 0.72, 1.0);
const GREEN: Color = Color::new(0.38, 0.86, 0.55, 1.0);
const RED: Color = Color::new(1.0, 0.50, 0.45, 1.0);

fn rgb(c: [f32; 3], a: f32) -> Color {
    Color::new(c[0], c[1], c[2], a)
}

fn fade(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * a)
}

/// A rectangle filled with a left-to-right colour ramp.
fn gradient(rect: Rect, left: Color, right: Color) {
    let v = |x: f32, y: f32, c: Color| Vertex::new(x, y, 0.0, 0.0, 0.0, c);
    let mesh = Mesh {
        vertices: vec![
            v(rect.x, rect.y, left),
            v(rect.x + rect.w, rect.y, right),
            v(rect.x + rect.w, rect.y + rect.h, right),
            v(rect.x, rect.y + rect.h, left),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        texture: None,
    };
    draw_mesh(&mesh);
}

fn label(r: &Renderer, text: &str, x: f32, y: f32, ui: f32) {
    r.text(text, x, y, 11.5 * ui, GOLD_SOFT, true);
}

/// A selectable card. Returns true when clicked.
fn card(r: &Renderer, sfx: &Sfx, rect: Rect, selected: bool, ui: f32) -> bool {
    let hover = rect.contains(mouse());
    let (fill, border) = if selected {
        (Color::new(GOLD.r, GOLD.g, GOLD.b, 0.13), Color::new(GOLD.r, GOLD.g, GOLD.b, 0.95))
    } else if hover {
        (Color::new(1.0, 1.0, 1.0, 0.085), Color::new(1.0, 1.0, 1.0, 0.28))
    } else {
        (Color::new(1.0, 1.0, 1.0, 0.035), Color::new(1.0, 1.0, 1.0, 0.10))
    };
    if selected {
        // Soft glow around the chosen card.
        let g = 5.0 * ui;
        r.panel(
            Rect::new(rect.x - g, rect.y - g, rect.w + 2.0 * g, rect.h + 2.0 * g),
            12.0 * ui + g,
            Color::new(GOLD.r, GOLD.g, GOLD.b, 0.06),
            Color::new(0.0, 0.0, 0.0, 0.0),
            0.0,
        );
    }
    r.panel(rect, 10.0 * ui, fill, border, 1.2 * ui);
    let clicked = hover && is_mouse_button_pressed(MouseButton::Left);
    if clicked {
        sfx.click();
    }
    clicked
}

/// A row of equal buttons where one is selected. Returns the clicked index.
fn segmented(r: &Renderer, sfx: &Sfx, rect: Rect, items: &[&str], selected: usize, ui: f32) -> Option<usize> {
    r.panel(rect, rect.h * 0.5, Color::new(0.0, 0.0, 0.0, 0.32), Color::new(1.0, 1.0, 1.0, 0.09), 1.0 * ui);
    let pad = 3.0 * ui;
    let w = (rect.w - 2.0 * pad) / items.len() as f32;
    let mut clicked = None;
    for (i, name) in items.iter().enumerate() {
        let cell = Rect::new(rect.x + pad + i as f32 * w, rect.y + pad, w, rect.h - 2.0 * pad);
        let hover = cell.contains(mouse());
        if i == selected {
            r.panel(cell, cell.h * 0.5, GOLD, Color::new(1.0, 1.0, 1.0, 0.25), 1.0 * ui);
        } else if hover {
            r.panel(cell, cell.h * 0.5, Color::new(1.0, 1.0, 1.0, 0.08), Color::new(0.0, 0.0, 0.0, 0.0), 0.0);
        }
        let color = if i == selected { Color::new(0.08, 0.06, 0.02, 1.0) } else if hover { PALE } else { MUTED };
        r.text_center(name, cell.x + cell.w / 2.0, cell.y + cell.h / 2.0, 12.5 * ui, color, true);
        if hover && is_mouse_button_pressed(MouseButton::Left) {
            sfx.click();
            clicked = Some(i);
        }
    }
    clicked
}

/// A labelled on/off switch. Returns true when clicked.
fn toggle(r: &Renderer, sfx: &Sfx, x: f32, y: f32, w: f32, text: &str, sub: &str, on: bool, ui: f32) -> bool {
    let row = Rect::new(x, y, w, 30.0 * ui);
    let hover = row.contains(mouse());
    r.text(text, x, y + 9.0 * ui, 13.5 * ui, if hover { PALE } else { Color::new(0.85, 0.87, 0.9, 1.0) }, true);
    r.text(sub, x, y + 25.0 * ui, 11.0 * ui, MUTED, false);
    let sw = Rect::new(x + w - 44.0 * ui, y + 3.0 * ui, 44.0 * ui, 24.0 * ui);
    let track = if on { GOLD } else { Color::new(1.0, 1.0, 1.0, 0.13) };
    r.panel(sw, 12.0 * ui, track, Color::new(1.0, 1.0, 1.0, 0.18), 1.0 * ui);
    let knob_x = if on { sw.x + sw.w - 12.0 * ui } else { sw.x + 12.0 * ui };
    draw_circle(knob_x, sw.y + 12.0 * ui, 9.0 * ui, if on { Color::new(0.10, 0.08, 0.03, 1.0) } else { PALE });
    let clicked = hover && is_mouse_button_pressed(MouseButton::Left);
    if clicked {
        sfx.click();
    }
    clicked
}

/// A ring of dots that chases itself, for "waiting" states.
fn spinner(x: f32, y: f32, radius: f32, color: Color) {
    let t = get_time() as f32 * 8.0;
    for i in 0..8 {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        let k = ((t - i as f32).rem_euclid(8.0)) / 8.0;
        draw_circle(x + a.cos() * radius, y + a.sin() * radius, radius * 0.22, fade(color, 0.2 + 0.8 * (1.0 - k)));
    }
}

/// Six letter tiles for a room code. `cursor` highlights the next empty tile.
fn code_tiles(r: &Renderer, x: f32, y: f32, w: f32, h: f32, code: &str, cursor: Option<usize>, ui: f32) {
    let gap = 7.0 * ui;
    let tw = (w - gap * (net::CODE_LEN as f32 - 1.0)) / net::CODE_LEN as f32;
    let blink = (get_time() * 2.0) as i64 % 2 == 0;
    for i in 0..net::CODE_LEN {
        let rect = Rect::new(x + i as f32 * (tw + gap), y, tw, h);
        let active = cursor == Some(i);
        let ch = code.chars().nth(i);
        let border = if active {
            GOLD
        } else if ch.is_some() {
            Color::new(1.0, 1.0, 1.0, 0.30)
        } else {
            Color::new(1.0, 1.0, 1.0, 0.12)
        };
        r.panel(rect, 8.0 * ui, Color::new(0.0, 0.0, 0.0, 0.40), border, if active { 1.8 * ui } else { 1.1 * ui });
        if let Some(c) = ch {
            r.text_center(&c.to_string(), rect.x + rect.w / 2.0, rect.y + rect.h / 2.0, h * 0.52, PALE, true);
        } else if active && blink {
            draw_rectangle(rect.x + rect.w / 2.0 - 1.0 * ui, rect.y + h * 0.28, 2.0 * ui, h * 0.44, GOLD);
        }
    }
}

/// A miniature top-down table in the colours of a theme.
fn mini_table(r: &Renderer, rect: Rect, th: &render::Theme, ui: f32) {
    let rail = 9.0 * ui;
    r.panel(rect, 11.0 * ui, rgb(th.wood1, 1.0), rgb(th.wood2, 1.0), 2.5 * ui);
    let felt = Rect::new(rect.x + rail, rect.y + rail, rect.w - 2.0 * rail, rect.h - 2.0 * rail);
    r.panel(felt, 3.0 * ui, rgb(th.felt, 1.0), Color::new(0.0, 0.0, 0.0, 0.35), 2.0 * ui);
    let glow = th.glow.iter().sum::<f32>() > 0.0;
    let accent = if glow { rgb(th.glow, 1.0) } else { rgb(th.accent, 1.0) };
    if glow {
        r.panel(
            Rect::new(rect.x - 2.0 * ui, rect.y - 2.0 * ui, rect.w + 4.0 * ui, rect.h + 4.0 * ui),
            13.0 * ui,
            Color::new(0.0, 0.0, 0.0, 0.0),
            fade(accent, 0.75),
            1.5 * ui,
        );
    }
    // Pockets.
    let pr = 4.6 * ui;
    for (px, py) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)] {
        let cx = felt.x + px * felt.w;
        let cy = felt.y + py * felt.h;
        draw_circle(cx, cy, pr, Color::new(0.02, 0.02, 0.02, 1.0));
        if glow {
            draw_circle_lines(cx, cy, pr + 1.0 * ui, 1.2 * ui, accent);
        }
    }
    // Sights along the rails.
    for k in 1..8 {
        if k == 4 {
            continue;
        }
        let sx = felt.x + felt.w * k as f32 / 8.0;
        draw_circle(sx, rect.y + rail * 0.5, 1.2 * ui, accent);
        draw_circle(sx, rect.y + rect.h - rail * 0.5, 1.2 * ui, accent);
    }
    // A little rack and the cue ball.
    let br = 3.0 * ui;
    let foot = vec2(felt.x + felt.w * 0.72, felt.y + felt.h * 0.5);
    let ids = [1u8, 9, 2, 10, 8, 3];
    let mut n = 0;
    for row in 0..3 {
        for k in 0..=row {
            let p = foot + vec2(row as f32 * br * 1.75, (k as f32 - row as f32 / 2.0) * br * 2.05);
            r.draw_icon_ball(p, br, ids[n % ids.len()], 1.0);
            n += 1;
        }
    }
    r.draw_icon_ball(vec2(felt.x + felt.w * 0.25, felt.y + felt.h * 0.5), br, 0, 1.0);
}

#[allow(clippy::too_many_arguments)]
pub fn menu(
    r: &mut Renderer,
    lay: &Layout,
    sfx: &mut Sfx,
    settings: &mut Settings,
    demo: &mut World,
    lobby: &mut Lobby,
    state: &mut MenuState,
) -> MenuAction {
    let ui = lay.ui;
    let now = get_time();

    // Background: the table slowly orbiting, darkened towards the left so the
    // headline reads clearly.
    let t = now as f32 * 0.05;
    let cam = Cam {
        pos: vec3(t.cos() * 3.0, t.sin() * 2.4, 1.45),
        target: vec3(0.0, 0.0, -0.05),
        fov: 42.0_f32.to_radians(),
    };
    r.draw_scene(&cam, demo, 0.0);
    gradient(
        Rect::new(0.0, 0.0, lay.sw, lay.sh),
        Color::new(0.010, 0.013, 0.020, 0.93),
        Color::new(0.010, 0.013, 0.020, 0.55),
    );

    let total_w = 1110.0 * ui;
    let x0 = ((lay.sw - total_w) * 0.5).max(0.0);
    let ch = 678.0 * ui;
    let y0 = ((lay.sh - ch) * 0.5).max(0.0);

    // ---------------- Hero column ----------------
    let hx = x0 + 8.0 * ui;
    r.draw_icon_ball(vec2(hx + 30.0 * ui, y0 + 74.0 * ui), 30.0 * ui, 8, 1.0);
    r.text("BREAKPOINT", hx + 74.0 * ui, y0 + 70.0 * ui, 44.0 * ui, PALE, true);
    r.text("B I L L I A R D S", hx + 77.0 * ui, y0 + 106.0 * ui, 14.0 * ui, GOLD, true);
    r.text("Real physics. Official rules.", hx, y0 + 158.0 * ui, 17.0 * ui, PALE, false);
    r.text("Play a friend anywhere with a room code.", hx, y0 + 182.0 * ui, 17.0 * ui, MUTED, false);

    let theme_count = THEMES.len().to_string();
    let features: [(u8, &str, String); 5] = [
        (1, "True-to-life ball physics", "Spin, throw and cushions at 960 steps a second".into()),
        (9, "Official WPA rules", "Called shots, push-outs and three-foul losses".into()),
        (3, "An elite computer", "Plans position, banks and plays safeties".into()),
        (11, "Online rooms", "Share a 6-letter code - no IP addresses".into()),
        (5, "Hand-finished tables", format!("{theme_count} finishes, from club wood to neon")),
    ];
    for (i, (ball, title, sub)) in features.iter().enumerate() {
        let y = y0 + (236.0 + i as f32 * 58.0) * ui;
        r.panel(
            Rect::new(hx, y - 20.0 * ui, 46.0 * ui, 46.0 * ui),
            12.0 * ui,
            Color::new(1.0, 1.0, 1.0, 0.045),
            Color::new(1.0, 1.0, 1.0, 0.10),
            1.0 * ui,
        );
        r.draw_icon_ball(vec2(hx + 23.0 * ui, y + 3.0 * ui), 12.0 * ui, *ball, 1.0);
        r.text(title, hx + 60.0 * ui, y - 5.0 * ui, 15.0 * ui, PALE, true);
        r.text(sub, hx + 60.0 * ui, y + 14.0 * ui, 12.0 * ui, MUTED, false);
    }
    r.text(
        &format!("v{VERSION}   |   F11 fullscreen   |   Esc pause"),
        hx,
        y0 + ch - 12.0 * ui,
        11.5 * ui,
        fade(MUTED, 0.8),
        false,
    );

    // ---------------- Setup card ----------------
    let cx = x0 + 390.0 * ui;
    let cw = total_w - 390.0 * ui;
    let card_rect = Rect::new(cx, y0, cw, ch);
    r.panel(
        Rect::new(cx - 10.0 * ui, y0 - 10.0 * ui, cw + 20.0 * ui, ch + 20.0 * ui),
        28.0 * ui,
        Color::new(GOLD.r, GOLD.g, GOLD.b, 0.035),
        Color::new(0.0, 0.0, 0.0, 0.0),
        0.0,
    );
    r.panel(card_rect, 20.0 * ui, Color::new(0.045, 0.051, 0.066, 0.95), Color::new(0.88, 0.73, 0.43, 0.38), 1.3 * ui);
    r.text("MATCH SETUP", cx + 28.0 * ui, y0 + 32.0 * ui, 14.0 * ui, PALE, true);
    r.text_right("Pick a game, an opponent and a table", cx + cw - 28.0 * ui, y0 + 32.0 * ui, 12.0 * ui, MUTED, false);
    draw_line(cx + 28.0 * ui, y0 + 54.0 * ui, cx + cw - 28.0 * ui, y0 + 54.0 * ui, 1.0 * ui, Color::new(1.0, 1.0, 1.0, 0.10));

    let col_w = (cw - 28.0 * 2.0 * ui - 32.0 * ui) / 2.0;
    let ax = cx + 28.0 * ui;
    let bx = ax + col_w + 32.0 * ui;

    // GAME
    label(r, "GAME", ax, y0 + 78.0 * ui, ui);
    let modes = [
        (Mode::EightBall, 8u8, "8-Ball", "Solids against stripes, then the 8"),
        (Mode::NineBall, 9, "9-Ball", "Lowest ball first - the 9 wins"),
        (Mode::Practice, 0, "Practice", "A free table, no rules, no clock"),
    ];
    for (i, (mode, ball, name, sub)) in modes.iter().enumerate() {
        let rect = Rect::new(ax, y0 + (92.0 + i as f32 * 58.0) * ui, col_w, 50.0 * ui);
        let sel = settings.mode == *mode;
        if card(r, sfx, rect, sel, ui) && !sel {
            settings.mode = *mode;
            rack(demo, *mode, get_time().to_bits());
            if *mode == Mode::Practice {
                *lobby = Lobby::Idle;
            }
        }
        r.draw_icon_ball(vec2(rect.x + 27.0 * ui, rect.y + rect.h / 2.0), 13.0 * ui, *ball, 1.0);
        r.text(name, rect.x + 52.0 * ui, rect.y + 17.0 * ui, 15.0 * ui, if sel { PALE } else { Color::new(0.82, 0.85, 0.88, 1.0) }, true);
        r.text(sub, rect.x + 52.0 * ui, rect.y + 35.0 * ui, 11.5 * ui, MUTED, false);
        if sel {
            draw_circle(rect.x + rect.w - 18.0 * ui, rect.y + rect.h / 2.0, 4.0 * ui, GOLD);
        }
    }

    // OPPONENT
    label(r, "OPPONENT", ax, y0 + 280.0 * ui, ui);
    let online = settings.opponent == Opponent::Online && settings.mode != Mode::Practice;
    let detail_y = y0 + 344.0 * ui;
    if settings.mode == Mode::Practice {
        let rect = Rect::new(ax, y0 + 294.0 * ui, col_w, 76.0 * ui);
        r.panel(rect, 10.0 * ui, Color::new(1.0, 1.0, 1.0, 0.035), Color::new(1.0, 1.0, 1.0, 0.10), 1.0 * ui);
        r.text("Solo table", rect.x + 16.0 * ui, rect.y + 24.0 * ui, 15.0 * ui, PALE, true);
        r.text("Work on position, spin and banks.", rect.x + 16.0 * ui, rect.y + 45.0 * ui, 12.0 * ui, MUTED, false);
        r.text("Clear the table and it racks again.", rect.x + 16.0 * ui, rect.y + 62.0 * ui, 12.0 * ui, MUTED, false);
    } else {
        let kind = match settings.opponent {
            Opponent::Human => 0,
            Opponent::Ai(_) => 1,
            Opponent::Online => 2,
        };
        if let Some(k) = segmented(r, sfx, Rect::new(ax, y0 + 294.0 * ui, col_w, 38.0 * ui), &["2 PLAYERS", "COMPUTER", "ONLINE"], kind, ui) {
            if k != kind {
                settings.opponent = match k {
                    0 => Opponent::Human,
                    1 => Opponent::Ai(settings.level),
                    _ => Opponent::Online,
                };
                if k != 2 {
                    *lobby = Lobby::Idle;
                }
            }
        }
        match settings.opponent {
            Opponent::Human => {
                let rect = Rect::new(ax, detail_y, col_w, 92.0 * ui);
                r.panel(rect, 10.0 * ui, Color::new(1.0, 1.0, 1.0, 0.035), Color::new(1.0, 1.0, 1.0, 0.10), 1.0 * ui);
                r.draw_icon_ball(vec2(rect.x + 24.0 * ui, rect.y + 30.0 * ui), 11.0 * ui, 2, 1.0);
                r.draw_icon_ball(vec2(rect.x + 40.0 * ui, rect.y + 30.0 * ui), 11.0 * ui, 13, 1.0);
                r.text("Pass and play", rect.x + 62.0 * ui, rect.y + 30.0 * ui, 15.0 * ui, PALE, true);
                r.text("Two players take turns on this computer.", rect.x + 16.0 * ui, rect.y + 58.0 * ui, 12.0 * ui, MUTED, false);
                r.text("Fouls and ball in hand are refereed for you.", rect.x + 16.0 * ui, rect.y + 76.0 * ui, 12.0 * ui, MUTED, false);
            }
            Opponent::Ai(current) => {
                let levels = [
                    (Difficulty::Easy, "Easy", "Learning the game"),
                    (Difficulty::Medium, "Medium", "A steady club player"),
                    (Difficulty::Hard, "Hard", "League shooter"),
                    (Difficulty::Elite, "Elite", "Plans every shot ahead"),
                ];
                let gw = (col_w - 10.0 * ui) / 2.0;
                for (i, (level, name, sub)) in levels.iter().enumerate() {
                    let rect = Rect::new(
                        ax + (i % 2) as f32 * (gw + 10.0 * ui),
                        detail_y + (i / 2) as f32 * 62.0 * ui,
                        gw,
                        54.0 * ui,
                    );
                    let sel = current == *level;
                    if card(r, sfx, rect, sel, ui) {
                        settings.level = *level;
                        settings.opponent = Opponent::Ai(*level);
                    }
                    r.text(name, rect.x + 14.0 * ui, rect.y + 18.0 * ui, 14.5 * ui, if sel { PALE } else { Color::new(0.82, 0.85, 0.88, 1.0) }, true);
                    r.text(sub, rect.x + 14.0 * ui, rect.y + 37.0 * ui, 11.0 * ui, MUTED, false);
                    for p in 0..4 {
                        let filled = p <= i;
                        let px = rect.x + rect.w - (14.0 + (3 - p) as f32 * 9.0) * ui;
                        let c = if filled { if i == 3 { RED } else { GOLD } } else { Color::new(1.0, 1.0, 1.0, 0.15) };
                        draw_circle(px, rect.y + 18.0 * ui, 3.0 * ui, c);
                    }
                }
                r.text(
                    "Elite looks two shots ahead and rarely misses.",
                    ax,
                    detail_y + 138.0 * ui,
                    11.5 * ui,
                    MUTED,
                    false,
                );
            }
            Opponent::Online => online_panel(r, sfx, ax, detail_y, col_w, lobby, state, ui),
        }
    }

    // TABLE FINISH
    label(r, "TABLE FINISH", bx, y0 + 78.0 * ui, ui);
    let n = THEMES.len();
    r.text_right(&format!("{:02} / {:02}", r.theme + 1, n), bx + col_w, y0 + 78.0 * ui, 11.0 * ui, MUTED, true);
    let tcard = Rect::new(bx, y0 + 92.0 * ui, col_w, 164.0 * ui);
    r.panel(tcard, 12.0 * ui, Color::new(0.0, 0.0, 0.0, 0.30), Color::new(1.0, 1.0, 1.0, 0.10), 1.0 * ui);
    let th = r.theme();
    mini_table(r, Rect::new(tcard.x + 46.0 * ui, tcard.y + 14.0 * ui, tcard.w - 92.0 * ui, 92.0 * ui), th, ui);
    r.text(th.name, tcard.x + 16.0 * ui, tcard.y + 126.0 * ui, 16.0 * ui, PALE, true);
    r.text(th.blurb, tcard.x + 16.0 * ui, tcard.y + 146.0 * ui, 11.5 * ui, MUTED, false);
    // Three swatches: cloth, rail, accent.
    let accent = if th.glow.iter().sum::<f32>() > 0.0 { th.glow } else { th.accent };
    for (k, c) in [th.felt, th.wood1, accent].iter().enumerate() {
        draw_circle(tcard.x + tcard.w - (20.0 + (2 - k) as f32 * 20.0) * ui, tcard.y + 126.0 * ui, 7.0 * ui, rgb(*c, 1.0));
        draw_circle_lines(tcard.x + tcard.w - (20.0 + (2 - k) as f32 * 20.0) * ui, tcard.y + 126.0 * ui, 7.0 * ui, 1.0 * ui, Color::new(1.0, 1.0, 1.0, 0.35));
    }
    let arrow = 30.0 * ui;
    let ay = tcard.y + 14.0 * ui + 46.0 * ui - arrow / 2.0;
    if button(r, sfx, Rect::new(tcard.x + 8.0 * ui, ay, arrow, arrow), "<", 16.0 * ui, PLAIN) || is_key_pressed(KeyCode::Left) {
        r.theme = (r.theme + n - 1) % n;
    }
    if button(r, sfx, Rect::new(tcard.x + tcard.w - 8.0 * ui - arrow, ay, arrow, arrow), ">", 16.0 * ui, PLAIN) || is_key_pressed(KeyCode::Right) {
        r.theme = (r.theme + 1) % n;
    }

    // CAMERA
    label(r, "CAMERA", bx, y0 + 280.0 * ui, ui);
    let views = [ViewMode::Cue, ViewMode::Free, ViewMode::Top];
    let vi = views.iter().position(|v| *v == settings.view).unwrap_or(0);
    if let Some(k) = segmented(r, sfx, Rect::new(bx, y0 + 294.0 * ui, col_w, 36.0 * ui), &["SHOOTER", "ORBIT", "TOP"], vi, ui) {
        settings.view = views[k];
    }

    // AIM ASSIST
    label(r, "AIM GUIDE", bx, y0 + 352.0 * ui, ui);
    if let Some(k) = segmented(r, sfx, Rect::new(bx, y0 + 366.0 * ui, col_w, 36.0 * ui), &["FULL", "SHORT", "OFF"], settings.guide as usize, ui) {
        settings.guide = k as u8;
    }

    // Switches
    let sy = y0 + 416.0 * ui;
    let step = 38.0 * ui;
    if toggle(r, sfx, bx, sy, col_w, "Pocket hints", "Show the pocket each ball is headed for", settings.pocket_hints, ui) {
        settings.pocket_hints = !settings.pocket_hints;
    }
    if toggle(r, sfx, bx, sy + step, col_w, "Call the pocket", "8-Ball: a ball only counts in the pocket you called", settings.call_shots, ui) {
        settings.call_shots = !settings.call_shots;
    }
    let sy = sy + step;
    let sound_on = sfx.0.as_ref().is_some_and(|s| s.enabled);
    let sound_sub = if sfx.0.is_none() { "No audio device found" } else { "Clacks, cushions and pockets" };
    if toggle(r, sfx, bx, sy + step, col_w, "Sound effects", sound_sub, sound_on, ui) {
        if let Some(s) = sfx.0.as_mut() {
            s.enabled = !s.enabled;
        }
    }
    // Music: a name on the left, a four-step level control on the right.
    let my = sy + 2.0 * step;
    let hover = Rect::new(bx, my, col_w, 30.0 * ui).contains(mouse());
    r.text("Music", bx, my + 9.0 * ui, 13.5 * ui, if hover { PALE } else { Color::new(0.85, 0.87, 0.9, 1.0) }, true);
    r.text("Late-night lounge jazz", bx, my + 25.0 * ui, 11.0 * ui, MUTED, false);
    let level = sfx.0.as_ref().map_or(0, |s| s.music_level as usize);
    let seg_w = 176.0 * ui;
    let seg = Rect::new(bx + col_w - seg_w, my + 2.0 * ui, seg_w, 26.0 * ui);
    if let Some(k) = segmented(r, sfx, seg, &["OFF", "LOW", "MID", "HIGH"], level, ui) {
        if let Some(s) = sfx.0.as_mut() {
            s.music_level = k as u8;
        }
    }
    if toggle(r, sfx, bx, sy + 3.0 * step, col_w, "Fullscreen", "Also F11 at any time", settings.fullscreen, ui) {
        settings.fullscreen = !settings.fullscreen;
        set_fullscreen(settings.fullscreen);
    }

    // Footer
    let fy = y0 + ch - 72.0 * ui;
    draw_line(cx + 28.0 * ui, fy, cx + cw - 28.0 * ui, fy, 1.0 * ui, Color::new(1.0, 1.0, 1.0, 0.10));
    let by = fy + 14.0 * ui;
    let quit_w = 120.0 * ui;
    let mut action = MenuAction::None;
    if online {
        r.text("The host picks the game. Your friend joins with the code.", ax, by + 23.0 * ui, 12.5 * ui, MUTED, false);
    } else {
        let play = Rect::new(ax, by, cw - 56.0 * ui - quit_w - 12.0 * ui, 46.0 * ui);
        if play.contains(mouse()) {
            let g = 4.0 * ui + 2.0 * ui * ((now * 5.0).sin() as f32 + 1.0);
            r.panel(
                Rect::new(play.x - g, play.y - g, play.w + 2.0 * g, play.h + 2.0 * g),
                14.0 * ui + g,
                Color::new(GOLD.r, GOLD.g, GOLD.b, 0.12),
                Color::new(0.0, 0.0, 0.0, 0.0),
                0.0,
            );
        }
        let text = if settings.mode == Mode::Practice { "START PRACTICE" } else { "PLAY MATCH" };
        if button(r, sfx, play, text, 19.0 * ui, PRIMARY) || is_key_pressed(KeyCode::Enter) {
            action = MenuAction::Play;
        }
        r.text_right("ENTER", play.x + play.w - 16.0 * ui, play.y + play.h / 2.0, 10.5 * ui, Color::new(0.08, 0.06, 0.02, 0.55), true);
    }
    if button(r, sfx, Rect::new(cx + cw - 28.0 * ui - quit_w, by, quit_w, 46.0 * ui), "QUIT", 14.0 * ui, PLAIN) {
        action = MenuAction::Quit;
    }
    action
}

#[allow(clippy::too_many_arguments)]
fn online_panel(r: &Renderer, sfx: &Sfx, x: f32, y: f32, w: f32, lobby: &mut Lobby, state: &mut MenuState, ui: f32) {
    let now = get_time();
    match lobby {
        Lobby::Creating(_) | Lobby::Hosting(_) => {
            while get_char_pressed().is_some() {}
            let code = match lobby {
                Lobby::Hosting(room) => room.code.clone(),
                _ => String::new(),
            };
            r.text("YOUR ROOM CODE", x, y + 8.0 * ui, 11.0 * ui, MUTED, true);
            code_tiles(r, x, y + 20.0 * ui, w, 52.0 * ui, &code, None, ui);
            if code.is_empty() {
                spinner(x + w / 2.0, y + 46.0 * ui, 10.0 * ui, GOLD);
            }
            let bw = (w - 10.0 * ui) / 2.0;
            let copied = now - state.copied_at < 1.6;
            let copy_label = if copied { "COPIED!" } else { "COPY CODE" };
            if button(r, sfx, Rect::new(x, y + 84.0 * ui, bw, 36.0 * ui), copy_label, 13.0 * ui, ButtonStyle { selected: copied, primary: !copied && !code.is_empty() })
                && !code.is_empty()
            {
                macroquad::miniquad::window::clipboard_set(&code);
                state.copied_at = now;
            }
            if button(r, sfx, Rect::new(x + bw + 10.0 * ui, y + 84.0 * ui, bw, 36.0 * ui), "CANCEL", 13.0 * ui, PLAIN) {
                *lobby = Lobby::Idle;
                return;
            }
            if let Lobby::Hosting(room) = lobby {
                spinner(x + 9.0 * ui, y + 146.0 * ui, 7.0 * ui, GREEN);
                r.text("Waiting for your friend to join...", x + 24.0 * ui, y + 146.0 * ui, 13.0 * ui, PALE, false);
                let badge = |r: &Renderer, bx: f32, on: bool, text: &str| {
                    draw_circle(bx + 5.0 * ui, y + 174.0 * ui, 4.0 * ui, if on { GREEN } else { Color::new(1.0, 1.0, 1.0, 0.2) });
                    r.text(text, bx + 15.0 * ui, y + 174.0 * ui, 11.5 * ui, if on { PALE } else { MUTED }, false);
                };
                badge(r, x, room.online, "Internet");
                badge(r, x + 90.0 * ui, room.lan, "Same Wi-Fi");
                if !room.online {
                    r.text("Internet relay unreachable: same network only", x, y + 196.0 * ui, 11.0 * ui, RED, false);
                }
            } else {
                r.text("Opening a room...", x, y + 146.0 * ui, 13.0 * ui, MUTED, false);
            }
        }
        _ => {
            // Create a room.
            let busy = matches!(lobby, Lobby::Joining(..) | Lobby::Joined(_));
            if button(r, sfx, Rect::new(x, y, w, 40.0 * ui), "CREATE ROOM", 15.0 * ui, PRIMARY) && !busy {
                *lobby = Lobby::Creating(net::create_room(net::Paths::all()));
                return;
            }
            r.text("You get a code to send to your friend", x, y + 54.0 * ui, 11.5 * ui, MUTED, false);
            // Divider.
            let dy = y + 78.0 * ui;
            draw_line(x, dy, x + w * 0.30, dy, 1.0 * ui, Color::new(1.0, 1.0, 1.0, 0.12));
            r.text_center("OR JOIN WITH A CODE", x + w / 2.0, dy, 10.5 * ui, MUTED, true);
            draw_line(x + w * 0.70, dy, x + w, dy, 1.0 * ui, Color::new(1.0, 1.0, 1.0, 0.12));
            // Typing and pasting.
            if !busy {
                while let Some(c) = get_char_pressed() {
                    let c = c.to_ascii_uppercase();
                    if net::CODE_ALPHABET.contains(&(c as u8)) && state.code.len() < net::CODE_LEN {
                        state.code.push(c);
                    }
                }
                if is_key_pressed(KeyCode::Backspace) {
                    state.code.pop();
                }
                let ctrl = is_key_down(KeyCode::LeftControl) || is_key_down(KeyCode::RightControl);
                if ctrl && is_key_pressed(KeyCode::V) {
                    if let Some(text) = macroquad::miniquad::window::clipboard_get() {
                        let cleaned: String = text
                            .chars()
                            .map(|c| c.to_ascii_uppercase())
                            .filter(|c| net::CODE_ALPHABET.contains(&(*c as u8)))
                            .take(net::CODE_LEN)
                            .collect();
                        state.code = cleaned;
                    }
                }
            } else {
                while get_char_pressed().is_some() {}
            }
            let cursor = (!busy && state.code.len() < net::CODE_LEN).then_some(state.code.len());
            code_tiles(r, x, dy + 16.0 * ui, w, 48.0 * ui, &state.code, cursor, ui);
            let ready = state.code.len() == net::CODE_LEN;
            let join_rect = Rect::new(x, dy + 74.0 * ui, w, 38.0 * ui);
            let style = ButtonStyle { selected: ready && !busy, primary: false };
            let label = if busy { "JOINING..." } else { "JOIN ROOM" };
            if (button(r, sfx, join_rect, label, 14.0 * ui, style) || is_key_pressed(KeyCode::Enter)) && ready && !busy {
                *lobby = Lobby::Joining(net::join_room(&state.code, net::Paths::all()), state.code.clone());
                return;
            }
            let sy = dy + 128.0 * ui;
            match lobby {
                Lobby::Joining(_, code) => {
                    spinner(x + 8.0 * ui, sy, 6.0 * ui, GOLD);
                    r.text(&format!("Looking for room {code}..."), x + 22.0 * ui, sy, 12.5 * ui, PALE, false);
                }
                Lobby::Joined(_) => {
                    spinner(x + 8.0 * ui, sy, 6.0 * ui, GREEN);
                    r.text("Connected - the host is starting the match", x + 22.0 * ui, sy, 12.5 * ui, PALE, false);
                }
                Lobby::Failed(e) => r.text(e, x, sy, 12.5 * ui, RED, false),
                _ => r.text("Type or paste (Ctrl+V) the code you were sent", x, sy, 11.5 * ui, MUTED, false),
            }
        }
    }
}
